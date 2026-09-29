use super::*;
use tempfile::tempdir;
use time::macros::datetime;

async fn state() -> (
    ControlState,
    Arc<StdMutex<aster_storage::SqlCipherStore>>,
    tempfile::TempDir,
    String,
) {
    let directory = tempdir().unwrap();
    let store = Arc::new(StdMutex::new(
        aster_storage::SqlCipherStore::initialize(&directory.path().join("slot.db"), &[71; 32])
            .unwrap(),
    ));
    let state = ControlState::new("2.0.1", None)
        .with_storage(ControlStorage::SqlCipher(Arc::clone(&store)))
        .with_auth_core(AuthCore::new(&[72; 32], "installation-slot-test").unwrap())
        .with_now(datetime!(2026-09-08 0:00 UTC));
    state
        .initialize_owner_identity(
            "owner@example.com",
            "Owner",
            Zeroizing::new(b"owner-password-strong".to_vec()),
        )
        .await
        .unwrap();
    let owner = state
        .credential_storage()
        .unwrap()
        .identity_by_email("owner@example.com")
        .await
        .unwrap()
        .unwrap();
    (state, store, directory, owner.id)
}

#[tokio::test]
async fn slot_registration_retries_without_new_runner_or_duplicate_audit() {
    let (state, store, _directory, owner) = state().await;
    let identity = BootstrapLocalRunnerIdentity::generate().unwrap();
    let before = store.lock().unwrap().audit_events().unwrap().len();
    state
        .provision_local_runner_slot(&owner, ReleaseSlot::Blue, &identity)
        .await
        .unwrap();
    let restarted = ControlState::new("2.1.0", None)
        .with_storage(ControlStorage::SqlCipher(Arc::clone(&store)))
        .with_auth_core(AuthCore::new(&[72; 32], "installation-slot-test").unwrap());
    restarted
        .provision_local_runner_slot(&owner, ReleaseSlot::Blue, &identity)
        .await
        .unwrap();
    assert_eq!(store.lock().unwrap().list_runners().unwrap().len(), 1);
    assert_eq!(
        store.lock().unwrap().audit_events().unwrap().len(),
        before + 1
    );
    let green = BootstrapLocalRunnerIdentity::generate().unwrap();
    state
        .provision_local_runner_slot(&owner, ReleaseSlot::Green, &green)
        .await
        .unwrap();
    assert_eq!(store.lock().unwrap().list_runners().unwrap().len(), 2);
    assert_eq!(
        store.lock().unwrap().audit_events().unwrap().len(),
        before + 2
    );
}

#[tokio::test]
async fn disabled_deleted_or_replaced_slot_material_is_not_silently_recreated() {
    let (state, store, _directory, owner) = state().await;
    let identity = BootstrapLocalRunnerIdentity::generate().unwrap();
    state
        .provision_local_runner_slot(&owner, ReleaseSlot::Blue, &identity)
        .await
        .unwrap();
    let wrong = BootstrapLocalRunnerIdentity {
        runner_id: identity.runner_id.clone(),
        credential: Zeroizing::new(format!("arr_{}", "z".repeat(64))),
    };
    assert!(
        state
            .provision_local_runner_slot(&owner, ReleaseSlot::Blue, &wrong)
            .await
            .is_err()
    );
    assert!(
        state
            .provision_local_runner_slot(&owner, ReleaseSlot::Green, &identity)
            .await
            .is_err()
    );
    let replacement = BootstrapLocalRunnerIdentity::generate().unwrap();
    assert!(
        state
            .provision_local_runner_slot(&owner, ReleaseSlot::Blue, &replacement)
            .await
            .is_err()
    );
    store
        .lock()
        .unwrap()
        .update_runner_enabled(&identity.runner_id, false, "2026-09-08T00:01:00.000Z")
        .unwrap();
    assert!(
        state
            .provision_local_runner_slot(&owner, ReleaseSlot::Blue, &identity)
            .await
            .is_err()
    );
    store
        .lock()
        .unwrap()
        .delete_runner(&identity.runner_id)
        .unwrap();
    let audit_count = store.lock().unwrap().audit_events().unwrap().len();
    assert!(
        state
            .provision_local_runner_slot(&owner, ReleaseSlot::Blue, &identity)
            .await
            .is_err()
    );
    assert!(
        state
            .provision_local_runner_slot(&owner, ReleaseSlot::Blue, &replacement)
            .await
            .is_err()
    );
    assert!(store.lock().unwrap().list_runners().unwrap().is_empty());
    assert_eq!(
        store.lock().unwrap().audit_events().unwrap().len(),
        audit_count
    );
}

#[tokio::test]
async fn slot_registration_requires_verified_actor_and_valid_material() {
    let (state, store, _directory, owner) = state().await;
    let mut identity = BootstrapLocalRunnerIdentity::generate().unwrap();
    assert!(
        state
            .provision_local_runner_slot("missing-owner", ReleaseSlot::Blue, &identity)
            .await
            .is_err()
    );
    identity.runner_id = "not-a-runner-id".to_owned();
    assert!(
        state
            .provision_local_runner_slot(&owner, ReleaseSlot::Blue, &identity)
            .await
            .is_err()
    );
    assert!(store.lock().unwrap().list_runners().unwrap().is_empty());
}

#[tokio::test]
async fn logical_runner_quota_authenticates_installation_binding_and_detects_tampering() {
    use aster_storage::runner_quota::{
        RUNNER_QUOTA_KEY, RunnerQuotaBindings, RunnerQuotaMember, RunnerUpgradeBinding,
    };
    let (state, store, directory, owner) = state().await;
    let previous = BootstrapLocalRunnerIdentity::generate().unwrap();
    let candidate = BootstrapLocalRunnerIdentity::generate().unwrap();
    state
        .provision_local_runner_slot(&owner, ReleaseSlot::Blue, &previous)
        .await
        .unwrap();
    state
        .provision_local_runner_slot(&owner, ReleaseSlot::Green, &candidate)
        .await
        .unwrap();
    assert_eq!(state.verified_runner_quota().await.unwrap().occupied(), 2);
    let value = RunnerQuotaBindings::new(vec![RunnerUpgradeBinding {
        phase: aster_storage::runner_quota::RunnerUpgradePhase::Prepared,
        installation_id: "installation-slot-test".into(),
        job_id: "upgrade-one".into(),
        logical_runner_id: previous.runner_id.clone(),
        previous: RunnerQuotaMember {
            id: previous.runner_id.clone(),
            credential_hash: sha256_hex(previous.credential.as_bytes()),
        },
        candidate: RunnerQuotaMember {
            id: candidate.runner_id.clone(),
            credential_hash: sha256_hex(candidate.credential.as_bytes()),
        },
    }])
    .unwrap()
    .encode()
    .unwrap();
    let mac = state
        .auth_core()
        .unwrap()
        .security_state_integrity_hmac(SecurityStateIntegrityInput {
            key: RUNNER_QUOTA_KEY,
            value: &value,
            revision: 1,
        })
        .unwrap();
    let connection = rusqlite::Connection::open(directory.path().join("slot.db")).unwrap();
    #[cfg(feature = "sqlcipher")]
    connection
        .execute_batch(&format!("PRAGMA key=\"x'{}'\";", "47".repeat(32)))
        .unwrap();
    connection
        .execute(
            "INSERT INTO security_state(key,value,revision,mac,updated_at) VALUES(?,?,1,?,?)",
            rusqlite::params![
                RUNNER_QUOTA_KEY,
                value,
                mac.as_bytes(),
                "2026-09-08T00:00:00.000Z"
            ],
        )
        .unwrap();
    let quota = state.verified_runner_quota().await.unwrap();
    assert_eq!(quota.occupied(), 1);
    assert!(quota.is_upgrade_member(&previous.runner_id));
    connection
        .execute(
            "UPDATE security_state SET revision=2 WHERE key=?",
            [RUNNER_QUOTA_KEY],
        )
        .unwrap();
    assert!(matches!(
        state.verified_runner_quota().await,
        Err(ControlError::DataIntegrityInvalid)
    ));
    connection
        .execute(
            "UPDATE security_state SET revision=1 WHERE key=?",
            [RUNNER_QUOTA_KEY],
        )
        .unwrap();
    store
        .lock()
        .unwrap()
        .delete_runner(&candidate.runner_id)
        .unwrap();
    assert!(matches!(
        state.verified_runner_quota().await,
        Err(ControlError::DataIntegrityInvalid)
    ));
}

fn upgrade_job(
    owner: &str,
    directory: &std::path::Path,
    id: &str,
) -> aster_upgrade_core::MaintenanceJob {
    use aster_upgrade_core::*;
    MaintenanceJob {
        schema: MAINTENANCE_JOB_SCHEMA.into(),
        id: id.into(),
        requested_by: owner.into(),
        operation: MaintenanceOperation::Upgrade {
            archive: directory.join("candidate.tar.gz"),
            archive_sha256: "a".repeat(64),
        },
        status: MaintenanceStatus::StartingCandidate,
        upgrade_mode: Some(UpgradeMode::Maintenance),
        runner_was_running: Some(true),
        current_version: "2.0.0".into(),
        target_version: Some("2.0.1".into()),
        previous_release: Some(directory.join("old")),
        candidate_release: Some(directory.join("new")),
        message: String::new(),
        created_at: "2026-09-08T00:00:00.000Z".into(),
        updated_at: "2026-09-08T00:00:00.000Z".into(),
    }
}

fn with_free_license(state: ControlState, directory: &std::path::Path) -> ControlState {
    let (bytes, keys) = crate::tests::signed_v2_license(1, None);
    let license = verify_product_license(&bytes, &keys).unwrap();
    let history = Arc::new(
        LicenseStateStore::new(directory.join("license-history.json"), &[90; 32]).unwrap(),
    );
    history
        .initialize(&license, datetime!(2026-09-08 0:00 UTC))
        .unwrap();
    state.replace_license(license).unwrap();
    state.with_license_state(history)
}

#[tokio::test]
async fn licensed_slot_upgrade_shares_one_quota_and_recovers_across_successive_jobs() {
    let (state, store, directory, owner) = state().await;
    let previous = BootstrapLocalRunnerIdentity::generate().unwrap();
    state
        .provision_local_runner_slot(&owner, ReleaseSlot::Blue, &previous)
        .await
        .unwrap();
    let state = with_free_license(state, directory.path());
    let job = upgrade_job(&owner, directory.path(), "upgrade_first");
    let candidate = BootstrapLocalRunnerIdentity::generate().unwrap();
    let before = store.lock().unwrap().audit_events().unwrap().len();
    state
        .provision_local_runner_upgrade(&job, ReleaseSlot::Green, &previous, &candidate)
        .await
        .unwrap();
    assert_eq!(state.verified_runner_quota().await.unwrap().occupied(), 1);
    assert_eq!(store.lock().unwrap().list_runners().unwrap().len(), 2);
    state
        .provision_local_runner_upgrade(&job, ReleaseSlot::Green, &previous, &candidate)
        .await
        .unwrap();
    assert_eq!(
        store.lock().unwrap().audit_events().unwrap().len(),
        before + 1
    );
    let replacement = BootstrapLocalRunnerIdentity::generate().unwrap();
    assert!(
        state
            .provision_local_runner_upgrade(&job, ReleaseSlot::Green, &previous, &replacement)
            .await
            .is_err()
    );
    let second = upgrade_job(&owner, directory.path(), "upgrade_second");
    assert!(
        state
            .provision_local_runner_upgrade(&second, ReleaseSlot::Blue, &candidate, &replacement)
            .await
            .is_err()
    );
    assert!(
        state
            .finish_local_runner_upgrade(&job.id, &replacement)
            .await
            .is_err()
    );
    state
        .finish_local_runner_upgrade(&job.id, &candidate)
        .await
        .unwrap();
    state
        .finish_local_runner_upgrade(&job.id, &candidate)
        .await
        .unwrap();
    assert!(
        state
            .finish_local_runner_upgrade(&job.id, &previous)
            .await
            .is_err()
    );
    assert_eq!(
        store.lock().unwrap().audit_events().unwrap().len(),
        before + 2
    );
    state
        .provision_local_runner_upgrade(&second, ReleaseSlot::Blue, &candidate, &replacement)
        .await
        .unwrap();
    let quota = state.verified_runner_quota().await.unwrap();
    assert_eq!(quota.occupied(), 1);
    assert_eq!(
        quota.upgrade_binding(&second.id).unwrap().logical_runner_id,
        previous.runner_id
    );
    state
        .finish_local_runner_upgrade(&second.id, &candidate)
        .await
        .unwrap();
    assert_eq!(store.lock().unwrap().list_runners().unwrap().len(), 1);
    assert_eq!(state.verified_runner_quota().await.unwrap().occupied(), 1);
    assert_eq!(
        store.lock().unwrap().audit_events().unwrap().len(),
        before + 4
    );
}

#[tokio::test]
async fn slot_upgrade_never_bypasses_license_or_accepts_a_different_job_context() {
    let (state, store, directory, owner) = state().await;
    let previous = BootstrapLocalRunnerIdentity::generate().unwrap();
    state
        .provision_local_runner_slot(&owner, ReleaseSlot::Blue, &previous)
        .await
        .unwrap();
    let candidate = BootstrapLocalRunnerIdentity::generate().unwrap();
    let job = upgrade_job(&owner, directory.path(), "upgrade_guarded");
    assert!(matches!(
        state
            .provision_local_runner_upgrade(&job, ReleaseSlot::Green, &previous, &candidate)
            .await,
        Err(ControlError::LicenseMissing)
    ));
    let state = with_free_license(state, directory.path());
    let before = store.lock().unwrap().audit_events().unwrap().len();
    let mut wrong = job.clone();
    wrong.status = aster_upgrade_core::MaintenanceStatus::Queued;
    assert!(
        state
            .provision_local_runner_upgrade(&wrong, ReleaseSlot::Green, &previous, &candidate)
            .await
            .is_err()
    );
    wrong = job.clone();
    wrong.target_version = Some("9.0.0".into());
    assert!(
        state
            .provision_local_runner_upgrade(&wrong, ReleaseSlot::Green, &previous, &candidate)
            .await
            .is_err()
    );
    wrong = job.clone();
    wrong.requested_by = "missing-owner".into();
    assert!(
        state
            .provision_local_runner_upgrade(&wrong, ReleaseSlot::Green, &previous, &candidate)
            .await
            .is_err()
    );
    let forged_previous = BootstrapLocalRunnerIdentity {
        runner_id: previous.runner_id.clone(),
        credential: candidate.credential.clone(),
    };
    assert!(
        state
            .provision_local_runner_upgrade(&job, ReleaseSlot::Green, &forged_previous, &candidate)
            .await
            .is_err()
    );
    assert_eq!(store.lock().unwrap().list_runners().unwrap().len(), 1);
    assert_eq!(store.lock().unwrap().audit_events().unwrap().len(), before);
}

#[tokio::test]
async fn slot_upgrade_rejects_existing_overquota_and_expired_license_without_writes() {
    let (state, store, directory, owner) = state().await;
    let previous = BootstrapLocalRunnerIdentity::generate().unwrap();
    let unrelated = BootstrapLocalRunnerIdentity::generate().unwrap();
    state
        .provision_local_runner_slot(&owner, ReleaseSlot::Blue, &previous)
        .await
        .unwrap();
    state
        .provision_local_runner_slot(&owner, ReleaseSlot::Green, &unrelated)
        .await
        .unwrap();
    let state = with_free_license(state, directory.path());
    let candidate = BootstrapLocalRunnerIdentity::generate().unwrap();
    let job = upgrade_job(&owner, directory.path(), "upgrade_overquota");
    let before = store.lock().unwrap().audit_events().unwrap().len();
    assert!(matches!(
        state
            .provision_local_runner_upgrade(&job, ReleaseSlot::Green, &previous, &candidate)
            .await,
        Err(ControlError::RunnerLimitReached)
    ));
    assert_eq!(store.lock().unwrap().list_runners().unwrap().len(), 2);
    assert_eq!(store.lock().unwrap().audit_events().unwrap().len(), before);
    assert!(
        state
            .verified_runner_quota()
            .await
            .unwrap()
            .upgrade_binding(&job.id)
            .is_none()
    );

    let (bytes, keys) = crate::tests::signed_v2_license(0, None);
    let license = verify_product_license(&bytes, &keys).unwrap();
    let history = Arc::new(
        LicenseStateStore::new(directory.path().join("fixed-history.json"), &[91; 32]).unwrap(),
    );
    history
        .initialize(&license, datetime!(2026-09-08 0:00 UTC))
        .unwrap();
    state.replace_license(license).unwrap();
    let expired = state
        .with_license_state(history)
        .with_now(datetime!(2028-09-08 0:00 UTC));
    assert!(matches!(
        expired
            .provision_local_runner_upgrade(&job, ReleaseSlot::Green, &previous, &candidate)
            .await,
        Err(ControlError::Policy(_))
    ));
    assert_eq!(store.lock().unwrap().list_runners().unwrap().len(), 2);
    assert_eq!(store.lock().unwrap().audit_events().unwrap().len(), before);
}

#[tokio::test]
async fn slot_settlement_distinguishes_unregistered_candidate_from_missing_signed_binding() {
    let (state, store, directory, owner) = state().await;
    let previous = BootstrapLocalRunnerIdentity::generate().unwrap();
    state
        .provision_local_runner_slot(&owner, ReleaseSlot::Blue, &previous)
        .await
        .unwrap();
    let state = with_free_license(state, directory.path());
    let candidate = BootstrapLocalRunnerIdentity::generate().unwrap();
    let job = upgrade_job(&owner, directory.path(), "upgrade_settlement");
    let before = store.lock().unwrap().audit_events().unwrap().len();
    state
        .settle_local_runner_upgrade(&job.id, &candidate, &previous)
        .await
        .unwrap();
    assert_eq!(store.lock().unwrap().audit_events().unwrap().len(), before);
    assert!(
        state
            .settle_local_runner_upgrade(&job.id, &candidate, &candidate)
            .await
            .is_err()
    );
    state
        .provision_local_runner_slot(&owner, ReleaseSlot::Green, &candidate)
        .await
        .unwrap();
    assert!(
        state
            .settle_local_runner_upgrade(&job.id, &candidate, &previous)
            .await
            .is_err()
    );
    assert_eq!(store.lock().unwrap().list_runners().unwrap().len(), 2);
}

#[tokio::test]
async fn slot_settlement_retries_committed_receipt_but_rejects_wrong_candidate() {
    let (state, store, directory, owner) = state().await;
    let previous = BootstrapLocalRunnerIdentity::generate().unwrap();
    state
        .provision_local_runner_slot(&owner, ReleaseSlot::Blue, &previous)
        .await
        .unwrap();
    let state = with_free_license(state, directory.path());
    let candidate = BootstrapLocalRunnerIdentity::generate().unwrap();
    let job = upgrade_job(&owner, directory.path(), "upgrade_settle_retry");
    state
        .provision_local_runner_upgrade(&job, ReleaseSlot::Green, &previous, &candidate)
        .await
        .unwrap();
    state
        .settle_local_runner_upgrade(&job.id, &candidate, &candidate)
        .await
        .unwrap();
    let audit_count = store.lock().unwrap().audit_events().unwrap().len();
    state
        .settle_local_runner_upgrade(&job.id, &candidate, &candidate)
        .await
        .unwrap();
    let wrong = BootstrapLocalRunnerIdentity::generate().unwrap();
    assert!(
        state
            .settle_local_runner_upgrade(&job.id, &wrong, &candidate)
            .await
            .is_err()
    );
    assert!(
        state
            .settle_local_runner_upgrade(&job.id, &candidate, &previous)
            .await
            .is_err()
    );
    assert_eq!(
        store.lock().unwrap().audit_events().unwrap().len(),
        audit_count
    );
    assert_eq!(store.lock().unwrap().list_runners().unwrap().len(), 1);
}

#[tokio::test]
async fn slot_reconciliation_uses_signed_binding_when_private_journal_was_lost() {
    let (state, store, directory, owner) = state().await;
    let previous = BootstrapLocalRunnerIdentity::generate().unwrap();
    state
        .provision_local_runner_slot(&owner, ReleaseSlot::Blue, &previous)
        .await
        .unwrap();
    let state = with_free_license(state, directory.path());
    let candidate = BootstrapLocalRunnerIdentity::generate().unwrap();
    let job = upgrade_job(&owner, directory.path(), "upgrade_missing_private_journal");
    assert!(
        !state
            .reconcile_local_runner_upgrade(&job.id, None, None)
            .await
            .unwrap()
    );
    state
        .provision_local_runner_upgrade(&job, ReleaseSlot::Green, &previous, &candidate)
        .await
        .unwrap();
    let before = store.lock().unwrap().audit_events().unwrap().len();
    assert!(
        state
            .reconcile_local_runner_upgrade(&job.id, None, None)
            .await
            .is_err()
    );
    let forged = BootstrapLocalRunnerIdentity::generate().unwrap();
    assert!(
        state
            .reconcile_local_runner_upgrade(&job.id, None, Some(&forged))
            .await
            .is_err()
    );
    assert_eq!(store.lock().unwrap().list_runners().unwrap().len(), 2);
    assert!(
        state
            .reconcile_local_runner_upgrade(&job.id, None, Some(&candidate))
            .await
            .unwrap()
    );
    assert!(
        state
            .reconcile_local_runner_upgrade(&job.id, None, Some(&candidate))
            .await
            .unwrap()
    );
    assert_eq!(
        store.lock().unwrap().audit_events().unwrap().len(),
        before + 1
    );
    assert_eq!(store.lock().unwrap().list_runners().unwrap().len(), 1);
}

#[tokio::test]
async fn unknown_preparation_reconciles_old_survivor_with_or_without_committed_registration() {
    for registered in [false, true] {
        for private_journal_lost in [false, true] {
            let (state, store, directory, owner) = state().await;
            let previous = BootstrapLocalRunnerIdentity::generate().unwrap();
            state
                .provision_local_runner_slot(&owner, ReleaseSlot::Blue, &previous)
                .await
                .unwrap();
            let state = with_free_license(state, directory.path());
            let candidate = BootstrapLocalRunnerIdentity::generate().unwrap();
            let job = upgrade_job(&owner, directory.path(), "unknown_prepare_cancel");
            if registered {
                state
                    .provision_local_runner_upgrade(&job, ReleaseSlot::Green, &previous, &candidate)
                    .await
                    .unwrap();
            }
            let expected = (!private_journal_lost).then_some(&candidate);
            let before = store.lock().unwrap().audit_events().unwrap().len();
            assert_eq!(
                state
                    .reconcile_local_runner_upgrade(&job.id, expected, Some(&previous))
                    .await
                    .unwrap(),
                registered
            );
            let audit_count = store.lock().unwrap().audit_events().unwrap().len();
            assert_eq!(audit_count, before + usize::from(registered));
            // A lost finalization reply repeats the same signed receipt and does
            // not create another delete audit or switch the selected survivor.
            assert_eq!(
                state
                    .reconcile_local_runner_upgrade(&job.id, expected, Some(&previous))
                    .await
                    .unwrap(),
                registered
            );
            assert_eq!(
                store.lock().unwrap().audit_events().unwrap().len(),
                audit_count
            );
            let runners = store.lock().unwrap().list_runners().unwrap();
            assert_eq!(runners.len(), 1);
            assert_eq!(runners[0].id, previous.runner_id);
            if registered {
                assert!(
                    state
                        .provision_local_runner_upgrade(
                            &job,
                            ReleaseSlot::Green,
                            &previous,
                            &candidate
                        )
                        .await
                        .is_err()
                );
                assert!(
                    state
                        .reconcile_local_runner_upgrade(&job.id, expected, Some(&candidate))
                        .await
                        .is_err()
                );
            }
        }
    }
}
