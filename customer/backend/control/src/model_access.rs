#[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
use std::sync::Arc;

#[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
use crate::lifecycle;
use crate::{
    ControlError, ControlState, ControlStorage, constant_time_string_eq, format_database_time,
    random_identifier, sha256_hex,
};
use aster_auth_core::SecurityStateIntegrityInput;
use aster_storage::{
    AuditEventRecord, IdentityRecord, ModelAccessPolicyRecord, ModelAccessWriteOutcome,
    MutationWithAuditOutcome,
};
use serde::Deserialize;

impl ControlStorage {
    async fn admit_model_attempt(
        &self,
        policy: ModelAccessPolicyRecord,
        model_id: &str,
        attempt_id: &str,
        request_id: &str,
        admitted_at: &str,
    ) -> Result<bool, ControlError> {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(store) => store
                .admit_model_attempt(&policy, model_id, attempt_id, request_id, admitted_at)
                .await
                .map_err(ControlError::storage),
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            Self::SqlCipher(store) => {
                let store = Arc::clone(store);
                let model_id = model_id.to_owned();
                let attempt_id = attempt_id.to_owned();
                let request_id = request_id.to_owned();
                let admitted_at = admitted_at.to_owned();
                lifecycle::spawn_blocking(move || {
                    store
                        .lock()
                        .map_err(|_| ControlError::DataIntegrityInvalid)?
                        .admit_model_attempt(
                            &policy,
                            &model_id,
                            &attempt_id,
                            &request_id,
                            &admitted_at,
                        )
                        .map_err(ControlError::storage)
                })
                .await
                .map_err(ControlError::storage)?
            }
            #[cfg(test)]
            Self::Fixed(_) | Self::Test { .. } => Err(ControlError::DataIntegrityInvalid),
        }
    }

    async fn model_access_policy(
        &self,
        identity_id: &str,
    ) -> Result<Option<ModelAccessPolicyRecord>, ControlError> {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(store) => store
                .model_access_policy(identity_id)
                .await
                .map_err(ControlError::storage),
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            Self::SqlCipher(store) => {
                let store = Arc::clone(store);
                let identity_id = identity_id.to_owned();
                lifecycle::spawn_blocking(move || {
                    store
                        .lock()
                        .map_err(|_| ControlError::DataIntegrityInvalid)?
                        .model_access_policy(&identity_id)
                        .map_err(ControlError::storage)
                })
                .await
                .map_err(ControlError::storage)?
            }
            #[cfg(test)]
            Self::Fixed(_) | Self::Test { .. } => Ok(None),
        }
    }

    async fn pending_model_access_policies(
        &self,
    ) -> Result<Vec<ModelAccessPolicyRecord>, ControlError> {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(store) => store
                .pending_model_access_policies()
                .await
                .map_err(ControlError::storage),
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            Self::SqlCipher(store) => {
                let store = Arc::clone(store);
                lifecycle::spawn_blocking(move || {
                    store
                        .lock()
                        .map_err(|_| ControlError::DataIntegrityInvalid)?
                        .pending_model_access_policies()
                        .map_err(ControlError::storage)
                })
                .await
                .map_err(ControlError::storage)?
            }
            #[cfg(test)]
            Self::Fixed(_) | Self::Test { .. } => Ok(Vec::new()),
        }
    }

    async fn finalize_model_access_migration(
        &self,
        expected: ModelAccessPolicyRecord,
        digest: String,
        hmac: String,
    ) -> Result<bool, ControlError> {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(store) => store
                .finalize_model_access_migration(&expected, &digest, &hmac)
                .await
                .map_err(ControlError::storage),
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            Self::SqlCipher(store) => {
                let store = Arc::clone(store);
                lifecycle::spawn_blocking(move || {
                    store
                        .lock()
                        .map_err(|_| ControlError::DataIntegrityInvalid)?
                        .finalize_model_access_migration(&expected, &digest, &hmac)
                        .map_err(ControlError::storage)
                })
                .await
                .map_err(ControlError::storage)?
            }
            #[cfg(test)]
            Self::Fixed(_) | Self::Test { .. } => Ok(false),
        }
    }

    async fn model_id_by_public_name(&self, name: &str) -> Result<Option<String>, ControlError> {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(store) => store
                .model_id_by_public_name(name)
                .await
                .map_err(ControlError::storage),
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            Self::SqlCipher(store) => {
                let store = Arc::clone(store);
                let name = name.to_owned();
                lifecycle::spawn_blocking(move || {
                    store
                        .lock()
                        .map_err(|_| ControlError::DataIntegrityInvalid)?
                        .model_id_by_public_name(&name)
                        .map_err(ControlError::storage)
                })
                .await
                .map_err(ControlError::storage)?
            }
            #[cfg(test)]
            Self::Fixed(_) | Self::Test { .. } => Ok(None),
        }
    }

    async fn replace_model_access_policy(
        &self,
        policy: ModelAccessPolicyRecord,
        expected_revision: i64,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: AuditEventRecord,
    ) -> Result<MutationWithAuditOutcome<ModelAccessWriteOutcome>, ControlError> {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(store) => store
                .replace_model_access_policy(
                    &policy,
                    expected_revision,
                    expected_audit_sequence,
                    expected_audit_hmac,
                    &audit_event,
                )
                .await
                .map_err(ControlError::storage),
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            Self::SqlCipher(store) => {
                let store = Arc::clone(store);
                let expected_audit_hmac = expected_audit_hmac.to_owned();
                lifecycle::spawn_blocking(move || {
                    store
                        .lock()
                        .map_err(|_| ControlError::DataIntegrityInvalid)?
                        .replace_model_access_policy(
                            &policy,
                            expected_revision,
                            expected_audit_sequence,
                            &expected_audit_hmac,
                            &audit_event,
                        )
                        .map_err(ControlError::storage)
                })
                .await
                .map_err(ControlError::storage)?
            }
            #[cfg(test)]
            Self::Fixed(_) | Self::Test { .. } => Err(ControlError::DataIntegrityInvalid),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UpdateModelAccessRequest {
    pub expected_revision: i64,
    pub mode: String,
    pub model_ids: Vec<String>,
}

fn grant_digest(grants: &[String]) -> String {
    let mut encoded = Vec::new();
    for grant in grants {
        encoded.extend_from_slice(grant.as_bytes());
        encoded.push(0);
    }
    sha256_hex(&encoded)
}

impl ControlState {
    pub(super) fn model_policy_hmac(
        &self,
        policy: &ModelAccessPolicyRecord,
    ) -> Result<String, ControlError> {
        let value = serde_json::to_vec(&serde_json::json!({
            "identity_id":policy.identity_id,
            "mode":policy.mode,
            "revision":policy.revision,
            "grant_set_digest":policy.grant_set_digest,
            "updated_by":policy.updated_by,
            "updated_at":policy.updated_at,
        }))
        .map_err(|_| ControlError::DataIntegrityInvalid)?;
        self.auth_core()?
            .security_state_integrity_hmac(SecurityStateIntegrityInput {
                key: "identity_model_policy.v1",
                value: &value,
                revision: u64::try_from(policy.revision)
                    .map_err(|_| ControlError::DataIntegrityInvalid)?,
            })
            .map_err(ControlError::Auth)
    }

    fn verify_model_policy(&self, policy: &ModelAccessPolicyRecord) -> Result<(), ControlError> {
        if !matches!(policy.mode.as_str(), "selected" | "all_enabled")
            || policy.revision < 0
            || policy.grants.windows(2).any(|pair| pair[0] >= pair[1])
            || (policy.mode == "all_enabled" && !policy.grants.is_empty())
            || !constant_time_string_eq(&grant_digest(&policy.grants), &policy.grant_set_digest)
            || !constant_time_string_eq(&self.model_policy_hmac(policy)?, &policy.integrity_hmac)
        {
            return Err(ControlError::DataIntegrityInvalid);
        }
        Ok(())
    }

    pub async fn initialize_migrated_model_access(&self) -> Result<(), ControlError> {
        let storage = self.credential_storage()?;
        loop {
            let pending = storage.pending_model_access_policies().await?;
            if pending.is_empty() {
                return Ok(());
            }
            for mut policy in pending {
                if policy.mode != "selected"
                    || policy.revision != 0
                    || !policy.integrity_hmac.is_empty()
                    || !policy.grant_set_digest.is_empty()
                    || policy.grants.windows(2).any(|pair| pair[0] >= pair[1])
                {
                    return Err(ControlError::DataIntegrityInvalid);
                }
                policy.grant_set_digest = grant_digest(&policy.grants);
                policy.integrity_hmac = self.model_policy_hmac(&policy)?;
                if !storage
                    .finalize_model_access_migration(
                        policy.clone(),
                        policy.grant_set_digest.clone(),
                        policy.integrity_hmac.clone(),
                    )
                    .await?
                {
                    return Err(ControlError::DataIntegrityInvalid);
                }
            }
        }
    }

    pub async fn assert_model_access(
        &self,
        identity_id: &str,
        public_model: &str,
    ) -> Result<String, ControlError> {
        let storage = self.credential_storage()?;
        let model_id = storage
            .model_id_by_public_name(public_model)
            .await?
            .ok_or(ControlError::ModelNotFound)?;
        let policy = storage
            .model_access_policy(identity_id)
            .await?
            .ok_or(ControlError::ModelAccessDenied)?;
        self.verify_model_policy(&policy)?;
        if policy.mode == "all_enabled" || policy.grants.binary_search(&model_id).is_ok() {
            Ok(model_id)
        } else {
            Err(ControlError::ModelAccessDenied)
        }
    }

    pub async fn admit_model_attempt(
        &self,
        identity_id: &str,
        public_model: &str,
        request_id: &str,
    ) -> Result<(), ControlError> {
        let storage = self.credential_storage()?;
        let model_id = storage
            .model_id_by_public_name(public_model)
            .await?
            .ok_or(ControlError::ModelNotFound)?;
        let policy = storage
            .model_access_policy(identity_id)
            .await?
            .ok_or(ControlError::ModelAccessDenied)?;
        self.verify_model_policy(&policy)?;
        if policy.mode != "all_enabled" && policy.grants.binary_search(&model_id).is_err() {
            return Err(ControlError::ModelAccessDenied);
        }
        let attempt_id = random_identifier("model_attempt")?;
        let admitted_at =
            format_database_time((self.now)()).map_err(|_| ControlError::DataIntegrityInvalid)?;
        if !storage
            .admit_model_attempt(policy, &model_id, &attempt_id, request_id, &admitted_at)
            .await?
        {
            return Err(ControlError::ModelAttemptStale);
        }
        Ok(())
    }

    pub async fn permitted_model_ids(
        &self,
        identity_id: &str,
    ) -> Result<Option<std::collections::HashSet<String>>, ControlError> {
        let policy = self
            .credential_storage()?
            .model_access_policy(identity_id)
            .await?;
        let Some(policy) = policy else {
            return Ok(Some(std::collections::HashSet::new()));
        };
        self.verify_model_policy(&policy)?;
        if policy.mode == "all_enabled" {
            Ok(None)
        } else {
            Ok(Some(policy.grants.into_iter().collect()))
        }
    }

    pub async fn model_access_view(
        &self,
        identity_id: &str,
    ) -> Result<serde_json::Value, ControlError> {
        let storage = self.credential_storage()?;
        let identity = storage
            .identity_by_id(identity_id)
            .await?
            .filter(|identity| identity.role == "member" && identity.status != "deleted")
            .ok_or(ControlError::IdentityNotFound)?;
        crate::verify_identity_integrity(self.auth_core()?, &identity)?;
        let policy = storage.model_access_policy(identity_id).await?;
        if let Some(policy) = policy {
            self.verify_model_policy(&policy)?;
            return Ok(serde_json::json!({
                "mode":policy.mode,"revision":policy.revision,"model_ids":policy.grants,
            }));
        }
        Err(ControlError::DataIntegrityInvalid)
    }

    pub(crate) async fn update_model_access(
        &self,
        identity_id: &str,
        actor: &IdentityRecord,
        request: UpdateModelAccessRequest,
    ) -> Result<serde_json::Value, ControlError> {
        if request.expected_revision < 0
            || request.model_ids.len() > 512
            || !matches!(request.mode.as_str(), "selected" | "all_enabled")
            || (request.mode == "all_enabled" && !request.model_ids.is_empty())
        {
            return Err(ControlError::UpstreamModelInputInvalid);
        }
        let storage = self.credential_storage()?;
        if let Some(current) = storage.model_access_policy(identity_id).await? {
            self.verify_model_policy(&current)?;
        }
        let mut grants = request.model_ids;
        grants.sort();
        if grants.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(ControlError::UpstreamModelInputInvalid);
        }
        let now =
            format_database_time((self.now)()).map_err(|_| ControlError::DataIntegrityInvalid)?;
        let mut policy = ModelAccessPolicyRecord {
            identity_id: identity_id.to_owned(),
            mode: request.mode,
            revision: request
                .expected_revision
                .checked_add(1)
                .ok_or(ControlError::DataIntegrityInvalid)?,
            grant_set_digest: grant_digest(&grants),
            integrity_hmac: String::new(),
            updated_by: actor.id.clone(),
            updated_at: now,
            grants,
        };
        policy.integrity_hmac = self.model_policy_hmac(&policy)?;
        for _ in 0..4 {
            let (audit_sequence, audit_hmac, audit_event) = self
                .prepare_audit_event(
                    Some(actor),
                    "model_access.update",
                    "identity",
                    Some(identity_id),
                    "succeeded",
                )
                .await?;
            match storage
                .replace_model_access_policy(
                    policy.clone(),
                    request.expected_revision,
                    audit_sequence,
                    &audit_hmac,
                    audit_event,
                )
                .await?
            {
                MutationWithAuditOutcome::Mutation(ModelAccessWriteOutcome::Applied) => {
                    return Ok(serde_json::json!({
                        "mode":policy.mode,"revision":policy.revision,"model_ids":policy.grants,
                    }));
                }
                MutationWithAuditOutcome::Mutation(ModelAccessWriteOutcome::RevisionConflict) => {
                    return Err(ControlError::ModelAccessConflict);
                }
                MutationWithAuditOutcome::Mutation(ModelAccessWriteOutcome::IdentityNotFound) => {
                    return Err(ControlError::IdentityNotFound);
                }
                MutationWithAuditOutcome::Mutation(ModelAccessWriteOutcome::ModelNotFound) => {
                    return Err(ControlError::UpstreamModelNotFound);
                }
                MutationWithAuditOutcome::AuditConflict => continue,
            }
        }
        Err(ControlError::ModelAccessConflict)
    }
}
