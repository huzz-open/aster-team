//! Stable task identity makes a lost audit acknowledgement safe to retry.
use super::*;

impl ControlState {
    /// The private executor has already validated its durable job. This method
    /// binds the event ID to that job and rejects a changed version or outcome;
    /// it does not grant a feature, change quotas or replace License validation.
    pub async fn record_upgrade_audit_for_job(
        &self,
        job_id: &str,
        target_version: &str,
        succeeded: bool,
    ) -> Result<(), ControlError> {
        if !aster_upgrade_core::valid_identifier(job_id)
            || !valid_audit_filter_identifier(target_version)
        {
            return Err(ControlError::DataIntegrityInvalid);
        }
        let digest = hex_digest(&Sha256::digest(format!("aster.upgrade-audit.v1:{job_id}")));
        let event_id = format!("audit_{}", &digest[..32]);
        let outcome = if succeeded { "succeeded" } else { "failed" };
        for _ in 0..AUDIT_APPEND_RETRIES {
            let events = self.verified_audit_events().await?;
            if let Some(event) = events.iter().find(|event| event.id == event_id) {
                if event.actor_identity_id.is_some()
                    || event.actor_role != "system"
                    || event.action != "release.upgrade"
                    || event.target_type != "release"
                    || event.target_id.as_deref() != Some(target_version)
                    || event.outcome != outcome
                {
                    return Err(ControlError::DataIntegrityInvalid);
                }
                return Ok(());
            }
            let (sequence, previous_hmac, mut event) = self
                .prepare_audit_event(
                    None,
                    "release.upgrade",
                    "release",
                    Some(target_version),
                    outcome,
                )
                .await?;
            event.id.clone_from(&event_id);
            event.integrity_hmac = self
                .auth_core()?
                .audit_event_integrity_hmac(audit_event_integrity_input(&event))
                .map_err(ControlError::Auth)?;
            if self
                .credential_storage()?
                .append_audit_event(sequence, &previous_hmac, &event)
                .await?
                == AuditAppendOutcome::Applied
            {
                return Ok(());
            }
            // A concurrent append or duplicate ID must be resolved from a fresh,
            // fully verified chain, never by treating a DB conflict as success.
        }
        Err(ControlError::DataIntegrityInvalid)
    }
}

#[cfg(all(test, any(feature = "sqlcipher", feature = "sqlite-dev")))]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, ControlState) {
        let directory = tempfile::tempdir().unwrap();
        let store = aster_storage::SqlCipherStore::initialize(
            &directory.path().join("customer.db"),
            &[74_u8; 32],
        )
        .unwrap();
        let state = ControlState::new("2.0.1", None)
            .with_storage(ControlStorage::SqlCipher(Arc::new(StdMutex::new(store))))
            .with_auth_core(AuthCore::new(&[75_u8; 32], "installation-test-001").unwrap());
        (directory, state)
    }

    #[tokio::test]
    async fn repeated_and_concurrent_acknowledgements_append_one_signed_event() {
        let (_directory, state) = fixture();
        let (first, second) = tokio::join!(
            state.record_upgrade_audit_for_job("job-one", "2.1.0", true),
            state.record_upgrade_audit_for_job("job-one", "2.1.0", true),
        );
        first.unwrap();
        second.unwrap();
        let before = state.verified_audit_events().await.unwrap();
        assert_eq!(before.len(), 1);
        state
            .record_upgrade_audit_for_job("job-one", "2.1.0", true)
            .await
            .unwrap();
        assert_eq!(state.verified_audit_events().await.unwrap(), before);
        state
            .record_upgrade_audit_for_job("job-two", "2.1.0", true)
            .await
            .unwrap();
        assert_eq!(state.verified_audit_events().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_job_cannot_rewrite_its_version_or_outcome_and_replay_requires_valid_hmac() {
        let (_directory, state) = fixture();
        state
            .record_upgrade_audit_for_job("job-one", "2.1.0", true)
            .await
            .unwrap();
        for (version, succeeded) in [("2.2.0", true), ("2.1.0", false)] {
            assert!(matches!(
                state
                    .record_upgrade_audit_for_job("job-one", version, succeeded)
                    .await,
                Err(ControlError::DataIntegrityInvalid)
            ));
        }
        let wrong_key = state
            .clone()
            .with_auth_core(AuthCore::new(&[76_u8; 32], "installation-test-001").unwrap());
        assert!(
            wrong_key
                .record_upgrade_audit_for_job("job-one", "2.1.0", true)
                .await
                .is_err()
        );
        assert_eq!(state.verified_audit_events().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn invalid_task_identity_never_writes_an_event() {
        let (_directory, state) = fixture();
        for job in ["", "../job", "job with spaces"] {
            assert!(
                state
                    .record_upgrade_audit_for_job(job, "2.1.0", true)
                    .await
                    .is_err()
            );
        }
        assert!(state.verified_audit_events().await.unwrap().is_empty());
    }
}
