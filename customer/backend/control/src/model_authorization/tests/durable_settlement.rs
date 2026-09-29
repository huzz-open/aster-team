use super::*;
use crate::gateway::durable_settlement::{Outcome, complete};

fn usage() -> ModelUsage {
    ModelUsage {
        uncached_input: 0,
        cached_input: 0,
        cache_write: 0,
        output_tokens: 1,
        multiplier_micros: 1_000_000,
        protocol: "openai_responses".into(),
        model: "test-model".into(),
        requested_model: None,
        processing_tier: None,
        reasoning_effort: None,
        runner_id: "runner-test".into(),
    }
}

#[tokio::test]
async fn expired_reservation_reaped_by_another_request_has_a_recoverable_unknown_outcome() {
    let f = Fixture::new().await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    let mut original = f
        .state
        .reserve_model_quota(&consumer, "before-outage", 1)
        .await
        .unwrap();
    original.client_request_id = Some("caller-reference".into());
    let recovered = f.state.clone().with_now(datetime!(2026-09-07 0:31 UTC));
    // A later request wins the race to reclaim the old TTL before its original
    // owner can retry. Use the real public admission path and storage transaction.
    let later = recovered
        .reserve_model_quota(&consumer, "after-outage", 1)
        .await
        .unwrap();
    let snapshot = recovered
        .verified_quota_snapshot(&f.identity.id)
        .await
        .unwrap();
    assert_eq!(snapshot.active_reservations.len(), 1);
    assert_eq!(snapshot.active_reservations[0].id, later.id);
    let terminal = snapshot
        .ledger_entries
        .iter()
        .find(|entry| entry.reference_id == original.request_id)
        .unwrap();
    assert_eq!(terminal.kind, "usage_failed");
    assert_eq!(terminal.protocol, "gateway_reservation_expired");
    assert_eq!(terminal.amount_tokens, 0);
    for _ in 0..2 {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            complete(&recovered, &original, Outcome::Usage(usage())),
        )
        .await
        .expect("TTL recovery must not loop forever");
        assert!(matches!(result, Err(ControlError::UsageSettlementInvalid)));
        assert_eq!(
            recovered
                .verified_quota_snapshot(&f.identity.id)
                .await
                .unwrap(),
            snapshot
        );
    }
}

#[tokio::test]
async fn unrepresentable_usage_becomes_durable_unbilled_failure_without_retrying_forever() {
    let f = Fixture::new().await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    for (request_id, output, multiplier) in [
        ("raw-overflow", 1, 1_000_000),
        ("billing-overflow", 0, 2_000_000),
    ] {
        let reservation = f
            .state
            .reserve_model_quota(&consumer, request_id, 1)
            .await
            .unwrap();
        let usage = ModelUsage {
            uncached_input: i64::MAX,
            output_tokens: output,
            multiplier_micros: multiplier,
            ..usage()
        };
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            complete(&f.state, &reservation, Outcome::Usage(usage)),
        )
        .await
        .expect("invalid immutable usage must finish failure settlement");
        assert!(matches!(result, Err(ControlError::UsageSettlementInvalid)));
        let snapshot = f
            .state
            .verified_quota_snapshot(&f.identity.id)
            .await
            .unwrap();
        assert!(snapshot.active_reservations.is_empty());
        assert_eq!(snapshot.balance.consumed_tokens, 0);
        let entries = snapshot
            .ledger_entries
            .iter()
            .filter(|entry| entry.reference_id == request_id)
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, "usage_failed");
        assert_eq!(entries[0].protocol, "gateway_usage_invalid");
        assert_eq!(entries[0].amount_tokens, 0);
        let audit = f.state.verified_audit_events().await.unwrap();
        assert!(audit.iter().any(|event| {
            event.action == "gateway.request.unbilled_upstream_unknown"
                && event.target_id.as_deref() == Some(request_id)
                && event.outcome == "failed"
        }));
    }
}

fn failed_request() -> FailedModelRequest {
    FailedModelRequest {
        protocol: "openai_responses".into(),
        model: "test-model".into(),
        requested_model: Some("requested-model".into()),
        processing_tier: None,
        reasoning_effort: None,
        runner_id: Some("runner-test".into()),
        description: "upstream disconnected after dispatch".into(),
    }
}

#[tokio::test]
async fn failure_and_unknown_audit_share_one_completion_and_replay_without_new_writes() {
    let f = Fixture::new().await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    let reservation = f
        .state
        .reserve_model_quota(&consumer, "unknown-once", 1)
        .await
        .unwrap();
    let outcome = Outcome::FailureWithUnknown(failed_request());
    complete(&f.state, &reservation, outcome.clone())
        .await
        .unwrap();
    let first = f.state.verified_audit_events().await.unwrap();
    let matching = first
        .iter()
        .filter(|event| {
            event.action == "gateway.request.unbilled_upstream_unknown"
                && event.target_id.as_deref() == Some("unknown-once")
        })
        .collect::<Vec<_>>();
    assert_eq!(matching.len(), 1);
    assert!(matching[0].id.starts_with("audit_gw_unknown_"));
    let quota = f
        .state
        .verified_quota_snapshot(&f.identity.id)
        .await
        .unwrap();
    assert!(quota.active_reservations.is_empty());
    assert_eq!(quota.balance.consumed_tokens, 0);
    assert_eq!(
        quota
            .ledger_entries
            .iter()
            .filter(|entry| entry.reference_id == "unknown-once")
            .count(),
        1
    );
    let later = f.state.clone().with_now(datetime!(2026-09-07 0:02 UTC));
    complete(&later, &reservation, outcome).await.unwrap();
    assert_eq!(later.verified_audit_events().await.unwrap(), first);
    assert_eq!(
        later.verified_quota_snapshot(&f.identity.id).await.unwrap(),
        quota
    );
}

#[tokio::test]
async fn separate_connections_complete_the_missing_audit_after_failure_ledger_committed() {
    let f = Fixture::new().await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    let reservation = f
        .state
        .reserve_model_quota(&consumer, "missing-unknown-audit", 1)
        .await
        .unwrap();
    let failure = failed_request();
    // Models an interrupted completion: its terminal quota transaction committed
    // but the required audit has not. Recovery must not settle a second time.
    f.state
        .fail_model_request(&reservation, &failure)
        .await
        .unwrap();
    let quota = f
        .state
        .verified_quota_snapshot(&f.identity.id)
        .await
        .unwrap();
    let reopened =
        aster_storage::SqlCipherStore::open(&f._directory.path().join("customer.db"), &[104; 32])
            .unwrap();
    let other = f
        .state
        .clone()
        .with_storage(ControlStorage::SqlCipher(Arc::new(StdMutex::new(reopened))));
    let left = complete(
        &f.state,
        &reservation,
        Outcome::FailureWithUnknown(failure.clone()),
    );
    let right = complete(&other, &reservation, Outcome::FailureWithUnknown(failure));
    let (left, right) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(left, right)
    })
    .await
    .unwrap();
    left.unwrap();
    right.unwrap();
    let events = other.verified_audit_events().await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(
                |event| event.action == "gateway.request.unbilled_upstream_unknown"
                    && event.target_id.as_deref() == Some("missing-unknown-audit")
            )
            .count(),
        1
    );
    assert_eq!(
        other.verified_quota_snapshot(&f.identity.id).await.unwrap(),
        quota
    );
}

#[tokio::test]
async fn known_failure_does_not_add_an_unknown_execution_audit() {
    let f = Fixture::new().await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    let reservation = f
        .state
        .reserve_model_quota(&consumer, "known-failure", 1)
        .await
        .unwrap();
    let events = f.state.verified_audit_events().await.unwrap();
    complete(&f.state, &reservation, Outcome::Failure(failed_request()))
        .await
        .unwrap();
    assert_eq!(f.state.verified_audit_events().await.unwrap(), events);
}

#[cfg(feature = "mariadb")]
#[tokio::test]
#[ignore = "requires disposable MariaDB; run npm run test:customer:external-db"]
async fn mariadb_settlement_recovers_failure_and_audit_across_two_controls() {
    use aster_storage::{MariaDbConfig, MariaDbStore};
    let config = MariaDbConfig {
        host: "127.0.0.1".into(),
        port: std::env::var("ASTER_CUSTOMER_TEST_DB_PORT")
            .unwrap()
            .parse()
            .unwrap(),
        database: "aster_control_test_settlement".into(),
        username: "aster_team".into(),
        password: std::env::var("ASTER_CUSTOMER_TEST_DB_PASSWORD").unwrap(),
        tls: true,
        ca_certificate: Some(std::env::var("ASTER_CUSTOMER_TEST_DB_CA").unwrap().into()),
        max_connections: 4,
    };
    let f = Fixture::new().await;
    let timestamp = format_database_time((f.state.now)()).unwrap();
    let registry = f
        .state
        .security_state_record(std::slice::from_ref(&f.identity.id), 0, &timestamp)
        .unwrap();
    let balance = f
        .state
        .initial_balance_record(&f.identity.id, &timestamp)
        .unwrap();
    let store = MariaDbStore::initialize(&config).await.unwrap();
    store
        .initialize_first_owner(&f.identity, &registry, &balance)
        .await
        .unwrap();
    store.insert_api_key_unchecked(&f.key).await.unwrap();
    let left = f.state.clone().with_storage(ControlStorage::MariaDb(store));
    let right = f.state.clone().with_storage(ControlStorage::MariaDb(
        MariaDbStore::open(&config).await.unwrap(),
    ));
    left.grant_model_quota(&f.identity.id, "mariadb-grant", 1000, "isolated test quota")
        .await
        .unwrap();
    let consumer = left.authorize_model_consumer(&f.token).await.unwrap();
    for (request, precommitted) in [
        ("concurrent-terminal", false),
        ("audit-after-restart", true),
    ] {
        let reservation = left
            .reserve_model_quota(&consumer, request, 1)
            .await
            .unwrap();
        let failure = failed_request();
        if precommitted {
            left.fail_model_request(&reservation, &failure)
                .await
                .unwrap();
        }
        let (a, b) = tokio::time::timeout(std::time::Duration::from_secs(20), async {
            tokio::join!(
                complete(
                    &left,
                    &reservation,
                    Outcome::FailureWithUnknown(failure.clone())
                ),
                complete(
                    &right,
                    &reservation,
                    Outcome::FailureWithUnknown(failure.clone())
                ),
            )
        })
        .await
        .unwrap();
        a.unwrap();
        b.unwrap();
        let quota = right.verified_quota_snapshot(&f.identity.id).await.unwrap();
        assert!(quota.active_reservations.is_empty());
        assert_eq!(quota.balance.consumed_tokens, 0);
        assert_eq!(
            quota
                .ledger_entries
                .iter()
                .filter(|entry| entry.reference_id == request)
                .count(),
            1
        );
        let audit = right.verified_audit_events().await.unwrap();
        assert_eq!(
            audit
                .iter()
                .filter(
                    |event| event.action == "gateway.request.unbilled_upstream_unknown"
                        && event.target_id.as_deref() == Some(request)
                )
                .count(),
            1
        );
        complete(&right, &reservation, Outcome::FailureWithUnknown(failure))
            .await
            .unwrap();
        assert_eq!(right.verified_audit_events().await.unwrap(), audit);
        assert_eq!(
            right.verified_quota_snapshot(&f.identity.id).await.unwrap(),
            quota
        );
    }
    // Persistent recovery discovers audit-only work even after the reservation
    // has left MariaDB. The two independent pools share installation-local locks.
    let layout = aster_install_layout::InstallLayout::new(f._directory.path()).unwrap();
    let left = left.with_settlement_outbox(&layout).unwrap();
    let right = right.with_settlement_outbox(&layout).unwrap();
    let reservation = left
        .reserve_model_quota(&consumer, "persistent-audit", 1)
        .await
        .unwrap();
    let failure = failed_request();
    let intent = crate::gateway::durable_settlement::SettlementIntent::new(
        &reservation,
        Outcome::FailureWithUnknown(failure.clone()),
    );
    reservation
        .execution
        .as_ref()
        .unwrap()
        .publish(left.auth_core().unwrap(), &intent)
        .unwrap();
    left.fail_model_request(&reservation, &failure)
        .await
        .unwrap();
    drop(intent);
    drop(reservation);
    let (a, b) = tokio::join!(
        left.recover_settlement_outbox_page(""),
        right.recover_settlement_outbox_page("")
    );
    a.unwrap();
    b.unwrap();
    let quota = right.verified_quota_snapshot(&f.identity.id).await.unwrap();
    assert_eq!(quota.balance.reserved_tokens, 0);
    assert_eq!(
        quota
            .ledger_entries
            .iter()
            .filter(|e| e.reference_id == "persistent-audit")
            .count(),
        1
    );
    let audit = right.verified_audit_events().await.unwrap();
    assert_eq!(
        audit
            .iter()
            .filter(|e| e.action == "gateway.request.unbilled_upstream_unknown"
                && e.target_id.as_deref() == Some("persistent-audit"))
            .count(),
        1
    );
    assert!(
        right
            .outbox()
            .unwrap()
            .unwrap()
            .page("", 16)
            .unwrap()
            .is_empty()
    );
    right.recover_settlement_outbox_page("").await.unwrap();
    assert_eq!(right.verified_audit_events().await.unwrap(), audit);
}
