use super::*;
use crate::runner_quota::{RunnerQuotaBindings, RunnerUpgradeBinding};

fn registration(id: &str, digit: &str) -> RunnerRegistrationRecord {
    RunnerRegistrationRecord {
        id: id.into(),
        credential_hash: digit.repeat(64),
        version: "2.1.0".into(),
        protocol_version: 3,
        platform: "linux".into(),
        architecture: "x86_64".into(),
        max_inflight: 4,
        created_at: NOW.into(),
    }
}

fn enroll(store: &SqlCipherStore, id: &str) {
    store
        .insert_runner_enrollment(&RunnerEnrollmentRecord {
            id: format!("enrollment_{id}"),
            token_hash: id.into(),
            token_prefix: id.into(),
            runner_name: id.into(),
            status: "pending".into(),
            expires_at: "2026-12-31T00:00:00.000Z".into(),
            created_by: "owner".into(),
            created_at: NOW.into(),
        })
        .unwrap();
}

#[test]
fn logical_quota_registration_compares_snapshot_inside_transaction_and_preserves_audit() {
    let directory = tempdir().unwrap();
    let mut store = initialize(&directory.path().join("quota.db"));
    let owner = identity("owner", "owner@example.test", false);
    store
        .initialize_first_owner(&owner, &seat_state(&[], 0), &initial_balance(&owner.id))
        .unwrap();
    for (id, digit) in [("runner_old", "a"), ("runner_new", "b")] {
        enroll(&store, id);
        assert_eq!(
            store
                .consume_runner_enrollment_unchecked(id, NOW, &registration(id, digit))
                .unwrap(),
            RunnerEnrollmentConsumeOutcome::Registered
        );
    }
    let bindings = RunnerQuotaBindings::new(vec![RunnerUpgradeBinding {
        phase: crate::runner_quota::RunnerUpgradePhase::Prepared,
        installation_id: "installation".into(),
        job_id: "job-one".into(),
        logical_runner_id: "runner_old".into(),
        previous: RunnerQuotaMember {
            id: "runner_old".into(),
            credential_hash: "a".repeat(64),
        },
        candidate: RunnerQuotaMember {
            id: "runner_new".into(),
            credential_hash: "b".repeat(64),
        },
    }])
    .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO security_state(key,value,revision,mac,updated_at) VALUES(?,?,1,?,?)",
            params![
                RUNNER_QUOTA_KEY,
                bindings.encode().unwrap(),
                b"fixture".as_slice(),
                NOW
            ],
        )
        .unwrap();
    store
        .update_runner_enabled("runner_old", false, NOW)
        .unwrap();
    let verify = |store: &SqlCipherStore| {
        store
            .runner_quota_snapshot()
            .unwrap()
            .verify("installation", |_| true)
            .unwrap()
    };
    let proof = verify(&store);
    assert_eq!(proof.occupied(), 1);
    enroll(&store, "runner_third");
    let event = audit_event(
        "runner-third-audit",
        1,
        "",
        'A',
        "runner.register",
        "runner_third",
    );
    let attempt = |store: &mut SqlCipherStore, verified, limit| {
        store
            .consume_runner_enrollment_with_quota_and_audit(
                "runner_third",
                NOW,
                &registration("runner_third", "c"),
                RunnerQuotaPolicy {
                    limit: Some(limit),
                    verified: Some(verified),
                },
                (0, "", &event),
            )
            .unwrap()
    };
    assert_eq!(
        attempt(&mut store, proof.clone(), 1),
        MutationWithAuditOutcome::Mutation(RunnerEnrollmentConsumeOutcome::LimitReached)
    );
    assert!(store.audit_events().unwrap().is_empty());
    // The same MAC/value but a different persisted revision invalidates a proof.
    store
        .connection
        .execute(
            "UPDATE security_state SET revision=2 WHERE key=?",
            [RUNNER_QUOTA_KEY],
        )
        .unwrap();
    assert_eq!(
        attempt(&mut store, proof, 2),
        MutationWithAuditOutcome::Mutation(RunnerEnrollmentConsumeOutcome::QuotaChanged)
    );
    assert_eq!(store.list_runners().unwrap().len(), 2);
    let proof = verify(&store);
    let stale = store
        .consume_runner_enrollment_with_quota_and_audit(
            "runner_third",
            NOW,
            &registration("runner_third", "c"),
            RunnerQuotaPolicy {
                limit: Some(2),
                verified: Some(proof.clone()),
            },
            (8, "wrong", &event),
        )
        .unwrap();
    assert_eq!(stale, MutationWithAuditOutcome::AuditConflict);
    assert_eq!(store.list_runners().unwrap().len(), 2);
    assert_eq!(
        attempt(&mut store, proof.clone(), 2),
        MutationWithAuditOutcome::Mutation(RunnerEnrollmentConsumeOutcome::Registered)
    );
    assert_eq!(store.audit_events().unwrap().len(), 1);
    assert_eq!(verify(&store).occupied(), 2);
    // A second registrant cannot commit with the earlier roster snapshot.
    enroll(&store, "runner_fourth");
    let next = audit_event(
        "runner-fourth-audit",
        2,
        &event.integrity_hmac,
        'B',
        "runner.register",
        "runner_fourth",
    );
    assert_eq!(
        store
            .consume_runner_enrollment_with_quota_and_audit(
                "runner_fourth",
                NOW,
                &registration("runner_fourth", "d"),
                RunnerQuotaPolicy {
                    limit: Some(2),
                    verified: Some(proof)
                },
                (1, &event.integrity_hmac, &next)
            )
            .unwrap(),
        MutationWithAuditOutcome::Mutation(RunnerEnrollmentConsumeOutcome::QuotaChanged)
    );
    assert_eq!(store.list_runners().unwrap().len(), 3);
    assert_eq!(store.audit_events().unwrap().len(), 1);
}

fn signed_fixture(change: &crate::runner_quota::RunnerQuotaChange) -> SecurityStateRecord {
    SecurityStateRecord {
        key: RUNNER_QUOTA_KEY.into(),
        value: change.value().to_vec(),
        revision: change.revision(),
        mac: b"fixture".to_vec(),
        updated_at: NOW.into(),
    }
}

#[test]
fn quota_upgrade_registration_commit_rollback_and_delete_are_atomic_and_recoverable() {
    use crate::runner_quota::{
        RunnerQuotaWrite, RunnerQuotaWriteOutcome as Outcome, RunnerUpgradePhase,
    };
    let directory = tempdir().unwrap();
    let mut store = initialize(&directory.path().join("lifecycle.db"));
    let owner = identity("owner", "owner@example.test", false);
    store
        .initialize_first_owner(&owner, &seat_state(&[], 0), &initial_balance(&owner.id))
        .unwrap();
    enroll(&store, "runner_old");
    store
        .consume_runner_enrollment_unchecked("runner_old", NOW, &registration("runner_old", "a"))
        .unwrap();
    let verify = |store: &SqlCipherStore| {
        store
            .runner_quota_snapshot()
            .unwrap()
            .verify("installation", |_| true)
            .unwrap()
    };
    let first = RunnerUpgradeBinding {
        phase: RunnerUpgradePhase::Prepared,
        installation_id: "installation".into(),
        job_id: "job-first".into(),
        logical_runner_id: "logical-local".into(),
        previous: RunnerQuotaMember {
            id: "runner_old".into(),
            credential_hash: "a".repeat(64),
        },
        candidate: RunnerQuotaMember {
            id: "runner_new".into(),
            credential_hash: "b".repeat(64),
        },
    };
    let change = verify(&store).begin_upgrade(first.clone()).unwrap();
    let next = signed_fixture(&change);
    let enrollment = RunnerEnrollmentRecord {
        id: "enrollment_new".into(),
        token_hash: "local-slot-new".into(),
        token_prefix: "local-slot".into(),
        runner_name: "local-blue".into(),
        status: "pending".into(),
        expires_at: NOW.into(),
        created_by: owner.id.clone(),
        created_at: NOW.into(),
    };
    let candidate = registration("runner_new", "b");
    let created = audit_event("audit-new", 1, "", 'A', "runner.register", &candidate.id);
    let write = || RunnerQuotaWrite {
        change: &change,
        next_state: &next,
        enrollment: Some(&enrollment),
        registration: Some(&candidate),
    };
    assert_eq!(
        store
            .apply_runner_quota_change(write(), Some(0), (0, "", &created))
            .unwrap(),
        Outcome::LimitReached
    );
    assert_eq!(
        store
            .apply_runner_quota_change(write(), Some(1), (8, "wrong", &created))
            .unwrap(),
        Outcome::AuditConflict
    );
    assert!(store.runner_quota_snapshot().unwrap().state.is_none());
    assert_eq!(store.list_runners().unwrap().len(), 1);
    assert_eq!(
        store
            .apply_runner_quota_change(write(), Some(1), (0, "", &created))
            .unwrap(),
        Outcome::Applied
    );
    assert_eq!(
        store
            .apply_runner_quota_change(write(), Some(1), (0, "", &created))
            .unwrap(),
        Outcome::QuotaChanged
    );
    let prepared = verify(&store);
    assert_eq!(prepared.occupied(), 1);
    assert_eq!(prepared.upgrade_binding("job-first"), Some(&first));
    assert!(prepared.begin_upgrade(first.clone()).is_err());
    assert!(prepared.delete_runner("runner_new").is_err());
    let finish = prepared.finish_upgrade("job-first", true).unwrap().unwrap();
    let next = signed_fixture(&finish);
    let retired = audit_event(
        "audit-retire",
        2,
        &created.integrity_hmac,
        'B',
        "runner.delete",
        "runner_old",
    );
    let write = || RunnerQuotaWrite {
        change: &finish,
        next_state: &next,
        enrollment: None,
        registration: None,
    };
    assert_eq!(
        store
            .apply_runner_quota_change(write(), None, (0, "", &retired))
            .unwrap(),
        Outcome::AuditConflict
    );
    assert_eq!(store.list_runners().unwrap().len(), 2);
    assert_eq!(
        store
            .apply_runner_quota_change(write(), None, (1, &created.integrity_hmac, &retired))
            .unwrap(),
        Outcome::Applied
    );
    let committed = verify(&store);
    assert_eq!(committed.occupied(), 1);
    assert!(
        committed
            .finish_upgrade("job-first", true)
            .unwrap()
            .is_none()
    );
    assert!(committed.finish_upgrade("job-first", false).is_err());
    assert!(committed.finish_upgrade("wrong-job", true).is_err());
    let second = RunnerUpgradeBinding {
        phase: RunnerUpgradePhase::Prepared,
        installation_id: "installation".into(),
        job_id: "job-second".into(),
        logical_runner_id: "logical-local".into(),
        previous: first.candidate,
        candidate: RunnerQuotaMember {
            id: "runner_next".into(),
            credential_hash: "c".repeat(64),
        },
    };
    let start = committed.begin_upgrade(second).unwrap();
    let next = signed_fixture(&start);
    let mut enrollment = enrollment.clone();
    enrollment.id = "enrollment_next".into();
    enrollment.token_hash = "local-slot-next".into();
    enrollment.runner_name = "runner_old".into();
    let candidate = registration("runner_next", "c");
    let created_again = audit_event(
        "audit-next",
        3,
        &retired.integrity_hmac,
        'C',
        "runner.register",
        "runner_next",
    );
    assert_eq!(
        store
            .apply_runner_quota_change(
                RunnerQuotaWrite {
                    change: &start,
                    next_state: &next,
                    enrollment: Some(&enrollment),
                    registration: Some(&candidate)
                },
                Some(1),
                (2, &retired.integrity_hmac, &created_again)
            )
            .unwrap(),
        Outcome::Applied
    );
    let rollback = verify(&store)
        .finish_upgrade("job-second", false)
        .unwrap()
        .unwrap();
    let next = signed_fixture(&rollback);
    let undone = audit_event(
        "audit-rollback",
        4,
        &created_again.integrity_hmac,
        'D',
        "runner.delete",
        "runner_next",
    );
    assert_eq!(
        store
            .apply_runner_quota_change(
                RunnerQuotaWrite {
                    change: &rollback,
                    next_state: &next,
                    enrollment: None,
                    registration: None
                },
                None,
                (3, &created_again.integrity_hmac, &undone)
            )
            .unwrap(),
        Outcome::Applied
    );
    let rolled_back = verify(&store);
    assert!(
        rolled_back
            .finish_upgrade("job-second", false)
            .unwrap()
            .is_none()
    );
    assert!(rolled_back.finish_upgrade("job-second", true).is_err());
    assert_eq!(rolled_back.occupied(), 1);
    let delete = rolled_back.delete_runner("runner_new").unwrap();
    let next = signed_fixture(&delete);
    let deleted = audit_event(
        "audit-delete",
        5,
        &undone.integrity_hmac,
        'E',
        "runner.delete",
        "runner_new",
    );
    assert_eq!(
        store
            .apply_runner_quota_change(
                RunnerQuotaWrite {
                    change: &delete,
                    next_state: &next,
                    enrollment: None,
                    registration: None
                },
                None,
                (4, &undone.integrity_hmac, &deleted)
            )
            .unwrap(),
        Outcome::Applied
    );
    assert_eq!(verify(&store).occupied(), 0);
    assert!(store.list_runners().unwrap().is_empty());
    assert_eq!(store.audit_events().unwrap().len(), 5);
}
