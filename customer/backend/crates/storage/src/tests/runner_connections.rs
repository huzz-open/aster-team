use super::*;

fn fixture() -> (tempfile::TempDir, SqlCipherStore, RunnerConnectionUpdate) {
    let directory = tempdir().unwrap();
    let mut store = initialize(&directory.path().join("customer.db"));
    let owner = identity("identity_owner", "owner@example.test", false);
    assert_eq!(
        store
            .initialize_first_owner(&owner, &seat_state(&[], 0), &initial_balance(&owner.id))
            .unwrap(),
        IdentityInitializeOutcome::Created
    );
    store
        .insert_runner_enrollment(&RunnerEnrollmentRecord {
            id: "enrollment_connection_test".into(),
            token_hash: "enrollment_token".into(),
            token_prefix: "test".into(),
            runner_name: "runner_test".into(),
            status: "pending".into(),
            expires_at: "2026-08-28T00:00:00.000Z".into(),
            created_by: owner.id,
            created_at: NOW.into(),
        })
        .unwrap();
    assert_eq!(
        store
            .consume_runner_enrollment_unchecked(
                "enrollment_token",
                NOW,
                &RunnerRegistrationRecord {
                    id: "runner_test".into(),
                    credential_hash: "credential_hash".into(),
                    version: "2.0.0".into(),
                    protocol_version: 2,
                    platform: "linux".into(),
                    architecture: "x86_64".into(),
                    max_inflight: 1,
                    created_at: NOW.into(),
                }
            )
            .unwrap(),
        RunnerEnrollmentConsumeOutcome::Registered
    );
    let update = RunnerConnectionUpdate {
        runner_id: "runner_test".into(),
        credential_hash: "credential_hash".into(),
        previous_protocol_version: 2,
        protocol_version: 3,
        runner_version: "2.1.0".into(),
        max_inflight: 2,
        heartbeat: RunnerHeartbeatUpdate {
            inflight: 0,
            recent_request_count: 0,
            recent_error_count: 0,
            latency_ms: 0,
        },
        observed_at: NOW.into(),
    };
    (directory, store, update)
}

#[test]
fn connection_upgrade_is_authenticated_idempotent_and_monotonic() {
    let (_directory, store, update) = fixture();
    let mut wrong = update.clone();
    wrong.credential_hash = "another_credential".into();
    assert!(!store.record_runner_connection(&wrong).unwrap());
    wrong = update.clone();
    wrong.previous_protocol_version = 1;
    assert!(!store.record_runner_connection(&wrong).unwrap());
    assert_eq!(store.list_runners().unwrap()[0].protocol_version, 2);
    assert!(store.record_runner_connection(&update).unwrap());
    assert!(store.record_runner_connection(&update).unwrap());
    let runners = store.list_runners().unwrap();
    assert_eq!(runners.len(), 1);
    assert_eq!(runners[0].id, update.runner_id);
    assert_eq!(runners[0].protocol_version, 3);
    assert_eq!(runners[0].version, update.runner_version);
    assert_eq!(runners[0].max_inflight, 2);
    let mut downgrade = update.clone();
    downgrade.previous_protocol_version = 3;
    downgrade.protocol_version = 2;
    assert!(!store.record_runner_connection(&downgrade).unwrap());
    assert_eq!(store.list_runners().unwrap()[0].protocol_version, 3);
}

#[test]
fn stale_connection_cannot_restore_rotated_credentials_or_disabled_runner() {
    let (_directory, store, update) = fixture();
    store
        .connection
        .execute(
            "UPDATE runners SET credential_hash='new_credential' WHERE id=?",
            params![update.runner_id],
        )
        .unwrap();
    assert!(!store.record_runner_connection(&update).unwrap());
    assert_eq!(store.list_runners().unwrap()[0].protocol_version, 2);
    let mut current = update.clone();
    current.credential_hash = "new_credential".into();
    assert!(store.record_runner_connection(&current).unwrap());
    store
        .update_runner_enabled(&update.runner_id, false, NOW)
        .unwrap();
    current.observed_at = "2026-08-27T00:00:05.000Z".into();
    assert!(!store.record_runner_connection(&current).unwrap());
    let runner = &store.list_runners().unwrap()[0];
    assert!(!runner.enabled);
    assert_eq!(runner.last_seen_at.as_deref(), Some(NOW));
}

#[test]
fn failed_connection_write_leaves_old_protocol_and_allows_retry() {
    let (_directory, store, update) = fixture();
    store.connection.execute_batch("CREATE TRIGGER fail_runner_connection BEFORE UPDATE OF last_seen_at ON runners BEGIN SELECT RAISE(ABORT, 'injected connection failure'); END;").unwrap();
    assert!(store.record_runner_connection(&update).is_err());
    let runners = store.list_runners().unwrap();
    assert_eq!(runners[0].protocol_version, 2);
    assert_eq!(runners[0].version, "2.0.0");
    store
        .connection
        .execute_batch("DROP TRIGGER fail_runner_connection;")
        .unwrap();
    assert!(store.record_runner_connection(&update).unwrap());
    assert_eq!(store.list_runners().unwrap()[0].protocol_version, 3);
}
