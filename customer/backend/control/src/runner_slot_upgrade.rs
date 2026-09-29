use super::*;
use aster_storage::runner_quota::{
    RunnerQuotaMember, RunnerQuotaWriteOutcome, RunnerUpgradeBinding, RunnerUpgradePhase,
};
use aster_upgrade_core::{MaintenanceJob, MaintenanceOperation, MaintenanceStatus};

impl ControlState {
    /// Complete an installer attempt, including a crash after its private
    /// journal was created but before the registration transaction committed.
    pub async fn settle_local_runner_upgrade(
        &self,
        job_id: &str,
        candidate: &BootstrapLocalRunnerIdentity,
        survivor: &BootstrapLocalRunnerIdentity,
    ) -> Result<(), ControlError> {
        self.reconcile_local_runner_upgrade(job_id, Some(candidate), Some(survivor))
            .await
            .map(|_| ())
    }

    /// The signed database binding is authoritative even when a private
    /// provisioning journal was lost. Return whether a signed upgrade exists;
    /// callers must reject an active slot that claims an unrecorded candidate.
    pub async fn reconcile_local_runner_upgrade(
        &self,
        job_id: &str,
        expected_candidate: Option<&BootstrapLocalRunnerIdentity>,
        survivor: Option<&BootstrapLocalRunnerIdentity>,
    ) -> Result<bool, ControlError> {
        if let Some(candidate) = expected_candidate {
            candidate.validate()?;
        }
        if let Some(survivor) = survivor {
            survivor.validate()?;
        }
        let license = authorize_retained_feature(self, "runner")?;
        let quota = self.verified_runner_quota().await?;
        if let Some(binding) = quota.upgrade_binding(job_id) {
            if expected_candidate.is_some_and(|candidate| {
                binding.candidate.id != candidate.runner_id
                    || binding.candidate.credential_hash
                        != sha256_hex(candidate.credential.as_bytes())
            }) {
                return Err(ControlError::DataIntegrityInvalid);
            }
            let survivor = survivor.ok_or(ControlError::DataIntegrityInvalid)?;
            self.finish_local_runner_upgrade(job_id, survivor).await?;
            return Ok(true);
        }
        if let Some(survivor) = survivor {
            let member = RunnerQuotaMember {
                id: survivor.runner_id.clone(),
                credential_hash: sha256_hex(survivor.credential.as_bytes()),
            };
            quota
                .logical_id_for(&member)
                .map_err(|_| ControlError::DataIntegrityInvalid)?;
        }
        if let Some(candidate) = expected_candidate
            && self
                .credential_storage()?
                .list_runners()
                .await?
                .iter()
                .any(|runner| runner.id == candidate.runner_id)
        {
            return Err(ControlError::DataIntegrityInvalid);
        }
        let _guard = self.guard_retained_mutation(&license, "runner")?;
        Ok(false)
    }

    /// Retire only the other member of an authenticated upgrade binding. The
    /// executor must first stop it and durably select the surviving active slot.
    /// Taking the survivor identity prevents a generic 'skip quota' switch.
    pub async fn finish_local_runner_upgrade(
        &self,
        job_id: &str,
        survivor: &BootstrapLocalRunnerIdentity,
    ) -> Result<(), ControlError> {
        survivor.validate()?;
        let license = authorize_retained_feature(self, "runner")?;
        let survivor = RunnerQuotaMember {
            id: survivor.runner_id.clone(),
            credential_hash: sha256_hex(survivor.credential.as_bytes()),
        };
        for _ in 0..AUDIT_APPEND_RETRIES {
            let quota = self.verified_runner_quota().await?;
            let binding = quota
                .upgrade_binding(job_id)
                .ok_or(ControlError::DataIntegrityInvalid)?;
            let keep_candidate = if binding.candidate == survivor {
                true
            } else if binding.previous == survivor {
                false
            } else {
                return Err(ControlError::DataIntegrityInvalid);
            };
            let retired = if keep_candidate {
                &binding.previous.id
            } else {
                &binding.candidate.id
            };
            let Some(change) = quota
                .finish_upgrade(job_id, keep_candidate)
                .map_err(|_| ControlError::DataIntegrityInvalid)?
            else {
                let _guard = self.guard_retained_mutation(&license, "runner")?;
                return Ok(());
            };
            let next_state = self.sign_runner_quota_change(&change)?;
            let audit = self
                .prepare_audit_event(None, "runner.delete", "runner", Some(retired), "succeeded")
                .await?;
            let guard = self.guard_retained_mutation(&license, "runner")?;
            let storage = self.credential_storage()?.clone();
            let mutation = async move {
                storage
                    .apply_runner_quota_change(
                        runner_quota::QuotaMutation {
                            change,
                            next_state,
                            enrollment: None,
                            registration: None,
                        },
                        None,
                        audit,
                    )
                    .await
            };
            match finish_licensed_mutation(Arc::clone(&self.mutation_tasks), guard, mutation)
                .await?
            {
                RunnerQuotaWriteOutcome::Applied => {
                    self.runner_hub.disconnect(retired).await;
                    return Ok(());
                }
                RunnerQuotaWriteOutcome::QuotaChanged | RunnerQuotaWriteOutcome::AuditConflict => {
                    continue;
                }
                RunnerQuotaWriteOutcome::LimitReached | RunnerQuotaWriteOutcome::Conflict => {
                    return Err(ControlError::DataIntegrityInvalid);
                }
            }
        }
        Err(ControlError::DataIntegrityInvalid)
    }

    /// Private installer entry point. The caller must load the durable running
    /// job and active slot from the installation while holding its upgrade lock.
    /// No public route accepts a job or a logical-quota exemption from a client.
    pub async fn provision_local_runner_upgrade(
        &self,
        job: &MaintenanceJob,
        slot: ReleaseSlot,
        previous: &BootstrapLocalRunnerIdentity,
        identity: &BootstrapLocalRunnerIdentity,
    ) -> Result<(), ControlError> {
        job.validate()
            .map_err(|_| ControlError::DataIntegrityInvalid)?;
        if job.status != MaintenanceStatus::StartingCandidate
            || !matches!(job.operation, MaintenanceOperation::Upgrade { .. })
            || job.target_version.as_deref() != Some(self.product_version.to_string().as_str())
        {
            return Err(ControlError::DataIntegrityInvalid);
        }
        previous.validate()?;
        identity.validate()?;
        let license = authorize_non_consuming_features(self, ["runner"])?;
        let limit = resource_quota_limit(license.as_ref().quota(QuotaId::Runners));
        let storage = self.credential_storage()?;
        let actor = storage
            .identity_by_id(&job.requested_by)
            .await?
            .filter(|actor| {
                matches!(actor.role.as_str(), "owner" | "admin") && actor.status == "active"
            })
            .ok_or(ControlError::IdentityNotFound)?;
        verify_identity_integrity(self.auth_core()?, &actor)?;
        let previous = RunnerQuotaMember {
            id: previous.runner_id.clone(),
            credential_hash: sha256_hex(previous.credential.as_bytes()),
        };
        let candidate = RunnerQuotaMember {
            id: identity.runner_id.clone(),
            credential_hash: sha256_hex(identity.credential.as_bytes()),
        };
        let name = format!("local-runner-{}", slot.id());
        for _ in 0..AUDIT_APPEND_RETRIES {
            let quota = self.verified_runner_quota().await?;
            let binding = RunnerUpgradeBinding {
                phase: RunnerUpgradePhase::Prepared,
                installation_id: self.auth_core()?.installation_id().into(),
                job_id: job.id.clone(),
                logical_runner_id: quota
                    .logical_id_for(&previous)
                    .map_err(|_| ControlError::DataIntegrityInvalid)?
                    .into(),
                previous: previous.clone(),
                candidate: candidate.clone(),
            };
            // A lost DB reply must not allocate another identity or duplicate
            // an audit event. Matching only the Runner ID is insufficient.
            if let Some(existing) = quota.upgrade_binding(&job.id) {
                if existing != &binding {
                    return Err(ControlError::DataIntegrityInvalid);
                }
                let runner = storage
                    .runner_by_credential_hash(&candidate.credential_hash)
                    .await?
                    .ok_or(ControlError::DataIntegrityInvalid)?;
                let metadata = storage
                    .list_runners()
                    .await?
                    .into_iter()
                    .find(|entry| entry.id == runner.id)
                    .ok_or(ControlError::DataIntegrityInvalid)?;
                if runner.id != candidate.id
                    || metadata.name != name
                    || !runner.enabled
                    || metadata.platform != "linux"
                    || metadata.architecture != "x86_64"
                    || runner.protocol_version != RUNNER_PROTOCOL_VERSION
                {
                    return Err(ControlError::DataIntegrityInvalid);
                }
                let _guard = self.guard_licensed_mutation(Some(&license), "runner")?;
                return Ok(());
            }
            let prior = storage
                .runner_by_credential_hash(&previous.credential_hash)
                .await?
                .filter(|runner| runner.id == previous.id && runner.enabled)
                .ok_or(ControlError::DataIntegrityInvalid)?;
            let metadata = storage
                .list_runners()
                .await?
                .into_iter()
                .find(|entry| entry.id == prior.id)
                .ok_or(ControlError::DataIntegrityInvalid)?;
            if metadata.platform != "linux" || metadata.architecture != "x86_64" {
                return Err(ControlError::DataIntegrityInvalid);
            }
            let change = quota
                .begin_upgrade(binding)
                .map_err(|_| ControlError::DataIntegrityInvalid)?;
            let next_state = self.sign_runner_quota_change(&change)?;
            let now = format_database_time((self.now)())
                .map_err(|_| ControlError::DataIntegrityInvalid)?;
            let enrollment_id = format!(
                "enrollment_local_{}",
                identity.runner_id.trim_start_matches("runner_")
            );
            let enrollment = RunnerEnrollmentRecord {
                id: enrollment_id.clone(),
                token_hash: sha256_hex(format!("local-slot-enrollment:{enrollment_id}").as_bytes()),
                token_prefix: "local-slot".into(),
                runner_name: name.clone(),
                status: "pending".into(),
                expires_at: now.clone(),
                created_by: actor.id.clone(),
                created_at: now.clone(),
            };
            let registration = RunnerRegistrationRecord {
                id: candidate.id.clone(),
                credential_hash: candidate.credential_hash.clone(),
                version: self.product_version.to_string(),
                protocol_version: RUNNER_PROTOCOL_VERSION,
                platform: "linux".into(),
                architecture: "x86_64".into(),
                max_inflight: 4,
                created_at: now,
            };
            let audit = self
                .prepare_audit_event(
                    Some(&actor),
                    "runner.register",
                    "runner",
                    Some(&candidate.id),
                    "succeeded",
                )
                .await?;
            let guard = self.guard_licensed_mutation(Some(&license), "runner")?;
            let storage = storage.clone();
            let mutation = async move {
                storage
                    .apply_runner_quota_change(
                        runner_quota::QuotaMutation {
                            change,
                            next_state,
                            enrollment: Some(enrollment),
                            registration: Some(registration),
                        },
                        limit,
                        audit,
                    )
                    .await
            };
            match finish_licensed_mutation(Arc::clone(&self.mutation_tasks), guard, mutation)
                .await?
            {
                RunnerQuotaWriteOutcome::Applied => return Ok(()),
                RunnerQuotaWriteOutcome::QuotaChanged | RunnerQuotaWriteOutcome::AuditConflict => {
                    continue;
                }
                RunnerQuotaWriteOutcome::LimitReached => {
                    return Err(ControlError::RunnerLimitReached);
                }
                RunnerQuotaWriteOutcome::Conflict => {
                    return Err(ControlError::DataIntegrityInvalid);
                }
            }
        }
        Err(ControlError::DataIntegrityInvalid)
    }
}
