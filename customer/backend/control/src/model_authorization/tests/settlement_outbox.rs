use super::*;
use crate::gateway::durable_settlement::{Outcome, SettlementIntent, complete};
use aster_install_layout::InstallLayout;

fn configured(f: &Fixture) -> ControlState {
    f.state
        .clone()
        .with_settlement_outbox(&InstallLayout::new(f._directory.path().to_path_buf()).unwrap())
        .unwrap()
}
fn restored(f: &Fixture) -> ControlState {
    let store =
        aster_storage::SqlCipherStore::open(&f._directory.path().join("customer.db"), &[104; 32])
            .unwrap();
    ControlState::new("2.0.1", None)
        .with_storage(ControlStorage::SqlCipher(Arc::new(StdMutex::new(store))))
        .with_auth_core(AuthCore::new(&[105; 32], "installation_consumption_test").unwrap())
        .with_settlement_outbox(&InstallLayout::new(f._directory.path().to_path_buf()).unwrap())
        .unwrap()
        .with_now(datetime!(2026-09-07 1:00 UTC))
}
fn usage() -> ModelUsage {
    ModelUsage {
        uncached_input: 2,
        cached_input: 0,
        cache_write: 0,
        output_tokens: 3,
        multiplier_micros: 2_000_000,
        protocol: "openai_responses".into(),
        model: "test-model".into(),
        requested_model: Some("selected-model".into()),
        processing_tier: None,
        reasoning_effort: None,
        runner_id: "runner-test".into(),
    }
}
fn intent_path(f: &Fixture) -> PathBuf {
    let root = InstallLayout::new(f._directory.path().to_path_buf())
        .unwrap()
        .settlement_outbox();
    std::fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name().unwrap().to_string_lossy().len() == 69
                && p.extension().is_some_and(|e| e == "json")
        })
        .unwrap()
}

#[tokio::test]
async fn live_request_lock_prevents_ttl_and_disappears_after_last_owner_exits() {
    let f = Fixture::new().await;
    let state = configured(&f);
    let consumer = state.authorize_model_consumer(&f.token).await.unwrap();
    let reservation = state
        .reserve_model_quota(&consumer, "live-expired", 1)
        .await
        .unwrap();
    let later = state.clone().with_now(datetime!(2026-09-07 1:00 UTC));
    later.recover_expired_model_quota_page("").await.unwrap();
    assert_eq!(
        later
            .verified_quota_snapshot(&f.identity.id)
            .await
            .unwrap()
            .active_reservations
            .len(),
        1
    );
    drop(reservation);
    later.recover_expired_model_quota_page("").await.unwrap();
    let snapshot = later.verified_quota_snapshot(&f.identity.id).await.unwrap();
    assert!(snapshot.active_reservations.is_empty());
    assert_eq!(snapshot.balance.consumed_tokens, 0);
    assert!(
        snapshot.ledger_entries.iter().any(
            |e| e.reference_id == "live-expired" && e.protocol == "gateway_reservation_expired"
        )
    );
}

#[tokio::test]
async fn persisted_usage_recovers_after_reopen_without_current_key_or_license() {
    let f = Fixture::new().await;
    {
        let state = configured(&f);
        let consumer = state.authorize_model_consumer(&f.token).await.unwrap();
        let mut reservation = state
            .reserve_model_quota(&consumer, "durable-usage", 1)
            .await
            .unwrap();
        reservation.client_request_id = Some("original-client-reference".into());
        let intent = SettlementIntent::new(&reservation, Outcome::Usage(usage()));
        reservation
            .execution
            .as_ref()
            .unwrap()
            .publish(state.auth_core().unwrap(), &intent)
            .unwrap();
        assert_eq!(
            state
                .verified_quota_snapshot(&f.identity.id)
                .await
                .unwrap()
                .balance
                .consumed_tokens,
            0
        );
    }
    f.revoke_key();
    let recovered = restored(&f);
    recovered.recover_settlement_outbox_page("").await.unwrap();
    let snapshot = recovered
        .verified_quota_snapshot(&f.identity.id)
        .await
        .unwrap();
    assert!(snapshot.active_reservations.is_empty());
    assert_eq!(snapshot.balance.consumed_tokens, 10);
    let entry = snapshot
        .ledger_entries
        .iter()
        .find(|e| e.reference_id == "durable-usage")
        .unwrap();
    assert_eq!(
        entry.client_request_id.as_deref(),
        Some("original-client-reference")
    );
    assert_eq!(entry.multiplier_micros, 2_000_000);
    assert_eq!(entry.raw_tokens, 5);
    assert_eq!(entry.requested_model.as_deref(), Some("selected-model"));
    recovered.recover_settlement_outbox_page("").await.unwrap();
    assert_eq!(
        recovered
            .verified_quota_snapshot(&f.identity.id)
            .await
            .unwrap(),
        snapshot
    );
    assert!(
        recovered
            .outbox()
            .unwrap()
            .unwrap()
            .page("", 16)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn pending_unknown_audit_recovers_even_when_reserved_balance_is_zero() {
    let f = Fixture::new().await;
    {
        let state = configured(&f);
        let consumer = state.authorize_model_consumer(&f.token).await.unwrap();
        let reservation = state
            .reserve_model_quota(&consumer, "durable-unknown", 1)
            .await
            .unwrap();
        let failure = FailedModelRequest {
            protocol: "openai_responses".into(),
            model: "test-model".into(),
            requested_model: None,
            processing_tier: None,
            reasoning_effort: None,
            runner_id: Some("runner-test".into()),
            description: "execution unknown".into(),
        };
        let intent =
            SettlementIntent::new(&reservation, Outcome::FailureWithUnknown(failure.clone()));
        reservation
            .execution
            .as_ref()
            .unwrap()
            .publish(state.auth_core().unwrap(), &intent)
            .unwrap();
        state
            .fail_model_request(&reservation, &failure)
            .await
            .unwrap();
        assert_eq!(
            state
                .verified_quota_snapshot(&f.identity.id)
                .await
                .unwrap()
                .balance
                .reserved_tokens,
            0
        );
    }
    let a = restored(&f);
    let b = restored(&f);
    let (left, right) = tokio::join!(
        a.recover_settlement_outbox_page(""),
        b.recover_settlement_outbox_page("")
    );
    left.unwrap();
    right.unwrap();
    a.recover_settlement_outbox_page("").await.unwrap();
    let events = a.verified_audit_events().await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.action == "gateway.request.unbilled_upstream_unknown"
                && e.target_id.as_deref() == Some("durable-unknown"))
            .count(),
        1
    );
    assert!(
        a.outbox()
            .unwrap()
            .unwrap()
            .page("", 16)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn corrupted_intent_is_preserved_and_cannot_fall_back_to_unknown_ttl() {
    let f = Fixture::new().await;
    {
        let state = configured(&f);
        let consumer = state.authorize_model_consumer(&f.token).await.unwrap();
        let reservation = state
            .reserve_model_quota(&consumer, "corrupt-result", 1)
            .await
            .unwrap();
        let intent = SettlementIntent::new(&reservation, Outcome::Usage(usage()));
        reservation
            .execution
            .as_ref()
            .unwrap()
            .publish(state.auth_core().unwrap(), &intent)
            .unwrap();
    }
    let path = intent_path(&f);
    let mut envelope: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    envelope["claims"]["intent"]["outcome"]["value"]["output_tokens"] = 999.into();
    let changed = serde_json::to_vec(&envelope).unwrap();
    std::fs::write(&path, &changed).unwrap();
    let recovered = restored(&f);
    let before = recovered
        .verified_quota_snapshot(&f.identity.id)
        .await
        .unwrap();
    recovered.recover_settlement_outbox_page("").await.unwrap();
    recovered
        .recover_expired_model_quota_page("")
        .await
        .unwrap();
    assert_eq!(
        recovered
            .verified_quota_snapshot(&f.identity.id)
            .await
            .unwrap(),
        before
    );
    assert_eq!(std::fs::read(path).unwrap(), changed);
}

#[tokio::test]
async fn normal_completion_clears_persistent_intent_and_keeps_frozen_usage() {
    let f = Fixture::new().await;
    let state = configured(&f);
    let consumer = state.authorize_model_consumer(&f.token).await.unwrap();
    let reservation = state
        .reserve_model_quota(&consumer, "normal-persistent", 1)
        .await
        .unwrap();
    complete(&state, &reservation, Outcome::Usage(usage()))
        .await
        .unwrap();
    drop(reservation);
    assert!(
        state
            .outbox()
            .unwrap()
            .unwrap()
            .page("", 16)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        state
            .verified_quota_snapshot(&f.identity.id)
            .await
            .unwrap()
            .balance
            .consumed_tokens,
        10
    );
}

#[tokio::test]
async fn cancelling_the_http_owner_keeps_the_lock_until_database_completion() {
    use std::time::Duration;
    let f = Fixture::new().await;
    let state = configured(&f);
    let consumer = state.authorize_model_consumer(&f.token).await.unwrap();
    let reservation = state
        .reserve_model_quota(&consumer, "cancelled-owner", 1)
        .await
        .unwrap();
    let store = Arc::clone(&f.store);
    let (held, ready) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let blocker = std::thread::spawn(move || {
        let _guard = store.lock().unwrap();
        held.send(()).unwrap();
        let _ = wait.recv_timeout(Duration::from_secs(20));
    });
    ready.await.unwrap();
    let processing = state.clone();
    let http =
        tokio::spawn(
            async move { complete(&processing, &reservation, Outcome::Usage(usage())).await },
        );
    let directory = InstallLayout::new(f._directory.path())
        .unwrap()
        .settlement_outbox();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if std::fs::read_dir(&directory).unwrap().any(|entry| {
                let name = entry.unwrap().file_name();
                let name = name.to_string_lossy();
                name.len() == 69 && name.ends_with(".json")
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    http.abort();
    assert!(http.await.unwrap_err().is_cancelled());
    let outbox = state.outbox().unwrap().unwrap();
    assert!(outbox.acquire("cancelled-owner").unwrap().is_none());
    release.send(()).unwrap();
    blocker.join().unwrap();
    tokio::time::timeout(Duration::from_secs(10), state.drain_licensed_mutations())
        .await
        .unwrap();
    assert_eq!(
        state
            .verified_quota_snapshot(&f.identity.id)
            .await
            .unwrap()
            .balance
            .consumed_tokens,
        10
    );
    assert!(outbox.page("", 16).unwrap().is_empty());
    assert!(outbox.acquire("cancelled-owner").unwrap().is_some());
}
