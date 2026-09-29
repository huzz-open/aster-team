use super::*;
use aster_storage::runner_quota::{
    RUNNER_QUOTA_KEY, RunnerQuotaChange, RunnerQuotaSnapshot, RunnerQuotaWrite,
    RunnerQuotaWriteOutcome, VerifiedRunnerQuota,
};

pub(super) struct QuotaMutation {
    pub change: RunnerQuotaChange,
    pub next_state: SecurityStateRecord,
    pub enrollment: Option<RunnerEnrollmentRecord>,
    pub registration: Option<RunnerRegistrationRecord>,
}

impl QuotaMutation {
    fn write(&self) -> RunnerQuotaWrite<'_> {
        RunnerQuotaWrite {
            change: &self.change,
            next_state: &self.next_state,
            enrollment: self.enrollment.as_ref(),
            registration: self.registration.as_ref(),
        }
    }
}

impl ControlStorage {
    pub(super) async fn apply_runner_quota_change(
        &self,
        mutation: QuotaMutation,
        runner_limit: Option<u32>,
        audit: (u64, String, AuditEventRecord),
    ) -> Result<RunnerQuotaWriteOutcome, ControlError> {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(store) => store
                .apply_runner_quota_change(
                    mutation.write(),
                    runner_limit,
                    (audit.0, &audit.1, &audit.2),
                )
                .await
                .map_err(map_identity_storage_error),
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            Self::SqlCipher(store) => {
                let store = Arc::clone(store);
                lifecycle::spawn_blocking(move || {
                    store
                        .lock()
                        .map_err(|_| ControlError::DataIntegrityInvalid)?
                        .apply_runner_quota_change(
                            mutation.write(),
                            runner_limit,
                            (audit.0, &audit.1, &audit.2),
                        )
                        .map_err(map_identity_storage_error)
                })
                .await
                .map_err(ControlError::storage)?
            }
            #[cfg(test)]
            Self::Fixed(_) | Self::Test { .. } => Err(ControlError::DataIntegrityInvalid),
        }
    }

    async fn runner_quota_snapshot(&self) -> Result<RunnerQuotaSnapshot, ControlError> {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(store) => store
                .runner_quota_snapshot()
                .await
                .map_err(map_identity_storage_error),
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            Self::SqlCipher(store) => {
                let store = Arc::clone(store);
                lifecycle::spawn_blocking(move || {
                    store
                        .lock()
                        .map_err(|_| ControlError::DataIntegrityInvalid)?
                        .runner_quota_snapshot()
                        .map_err(map_identity_storage_error)
                })
                .await
                .map_err(ControlError::storage)?
            }
            #[cfg(test)]
            Self::Fixed(_) | Self::Test { .. } => Err(ControlError::DataIntegrityInvalid),
        }
    }
}

impl ControlState {
    pub(super) fn sign_runner_quota_change(
        &self,
        change: &RunnerQuotaChange,
    ) -> Result<SecurityStateRecord, ControlError> {
        let mac = self
            .auth_core()?
            .security_state_integrity_hmac(SecurityStateIntegrityInput {
                key: RUNNER_QUOTA_KEY,
                value: change.value(),
                revision: change.revision(),
            })
            .map_err(ControlError::Auth)?;
        Ok(SecurityStateRecord {
            key: RUNNER_QUOTA_KEY.into(),
            value: change.value().to_vec(),
            revision: change.revision(),
            mac: mac.into_bytes(),
            updated_at: format_database_time((self.now)())
                .map_err(|_| ControlError::DataIntegrityInvalid)?,
        })
    }

    pub(super) async fn verified_runner_quota(&self) -> Result<VerifiedRunnerQuota, ControlError> {
        let snapshot = self.credential_storage()?.runner_quota_snapshot().await?;
        let auth = self.auth_core()?;
        snapshot
            .verify(auth.installation_id(), |record| {
                std::str::from_utf8(&record.mac).ok().is_some_and(|mac| {
                    auth.verify_security_state_integrity_hmac(
                        SecurityStateIntegrityInput {
                            key: &record.key,
                            value: &record.value,
                            revision: record.revision,
                        },
                        mac,
                    )
                    .unwrap_or(false)
                })
            })
            .map_err(|_| ControlError::DataIntegrityInvalid)
    }
}
