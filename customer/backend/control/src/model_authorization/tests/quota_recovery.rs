use super::*;

#[tokio::test]
async fn quota_recovery_requires_no_new_request_or_valid_key_or_current_license() {
    let f = Fixture::new().await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    let reservation = f
        .state
        .reserve_model_quota(&consumer, "orphan", 7)
        .await
        .unwrap();
    f.revoke_key();
    let recovered = f.state.clone().with_now(datetime!(2027-09-02 0:00 UTC));
    assert_eq!(
        recovered
            .recover_expired_model_quota_page("")
            .await
            .unwrap(),
        None
    );
    let snapshot = recovered
        .verified_quota_snapshot(&f.identity.id)
        .await
        .unwrap();
    assert!(snapshot.active_reservations.is_empty());
    assert_eq!(snapshot.balance.reserved_tokens, 0);
    assert_eq!(snapshot.balance.consumed_tokens, 0);
    let entries: Vec<_> = snapshot
        .ledger_entries
        .iter()
        .filter(|entry| entry.reference_id == reservation.request_id)
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].protocol, "gateway_reservation_expired");
    assert_eq!(entries[0].amount_tokens, 0);
    recovered
        .recover_expired_model_quota_page("")
        .await
        .unwrap();
    assert_eq!(
        recovered
            .verified_quota_snapshot(&f.identity.id)
            .await
            .unwrap(),
        snapshot
    );
}

#[tokio::test]
async fn quota_recovery_preserves_live_reservations_and_closed_candidate_does_no_work() {
    let f = Fixture::new().await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    f.state
        .reserve_model_quota(&consumer, "live", 5)
        .await
        .unwrap();
    let before = f
        .state
        .verified_quota_snapshot(&f.identity.id)
        .await
        .unwrap();
    f.state.recover_expired_model_quota_page("").await.unwrap();
    assert_eq!(
        f.state
            .verified_quota_snapshot(&f.identity.id)
            .await
            .unwrap(),
        before
    );
    let recovered = f.state.clone().with_now(datetime!(2026-09-07 0:31 UTC));
    let lifecycle = recovered.request_lifecycle();
    lifecycle.begin_drain();
    recovered
        .recover_expired_model_quota_page("")
        .await
        .unwrap();
    assert_eq!(lifecycle.snapshot().in_flight, 0);
    assert_eq!(
        recovered
            .verified_quota_snapshot(&f.identity.id)
            .await
            .unwrap(),
        before
    );
    assert!(lifecycle.set_admission(lifecycle.snapshot().revision, true));
    recovered
        .recover_expired_model_quota_page("")
        .await
        .unwrap();
    assert_eq!(f.reserved(), 0);
}

#[tokio::test]
async fn quota_recovery_leaves_corrupt_signed_state_untouched() {
    let f = Fixture::new().await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    let reservation = f
        .state
        .reserve_model_quota(&consumer, "tampered", 5)
        .await
        .unwrap();
    let connection = rusqlite::Connection::open(f._directory.path().join("customer.db")).unwrap();
    #[cfg(feature = "sqlcipher")]
    connection
        .pragma_update(None, "key", format!("x'{}'", "68".repeat(32)))
        .unwrap();
    connection
        .execute(
            "UPDATE quota_reservations SET integrity_hmac='tampered' WHERE id=?",
            [&reservation.id],
        )
        .unwrap();
    let recovered = f.state.clone().with_now(datetime!(2026-09-07 0:31 UTC));
    assert!(matches!(
        recovered.verified_quota_snapshot(&f.identity.id).await,
        Err(ControlError::DataIntegrityInvalid)
    ));
    // Discovery does not authorize settlement; even an expired row must verify.
    recovered
        .recover_expired_model_quota_page("")
        .await
        .unwrap();
    assert_eq!(f.reserved(), 5);
    let status: String = connection
        .query_row(
            "SELECT status FROM quota_reservations WHERE id=?",
            [&reservation.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "active");
    let terminal: u32 = connection
        .query_row(
            "SELECT COUNT(*) FROM ledger_entries WHERE reference_id=?",
            [&reservation.request_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(terminal, 0);
}

#[tokio::test]
async fn quota_recovery_limits_one_identity_and_finishes_it_on_the_next_pass() {
    let f = Fixture::new().await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    for index in 0..17 {
        f.state
            .reserve_model_quota(&consumer, &format!("orphan_{index}"), 1)
            .await
            .unwrap();
    }
    let recovered = f.state.clone().with_now(datetime!(2026-09-07 0:31 UTC));
    recovered
        .recover_expired_model_quota_page("")
        .await
        .unwrap();
    assert_eq!(f.reserved(), 1);
    recovered
        .recover_expired_model_quota_page("")
        .await
        .unwrap();
    assert_eq!(f.reserved(), 0);
}

#[tokio::test]
async fn quota_recovery_scheduler_runs_on_startup_and_exits_when_shutdown_begins() {
    let f = Fixture::new().await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    f.state
        .reserve_model_quota(&consumer, "scheduler_orphan", 1)
        .await
        .unwrap();
    let recovered = f.state.clone().with_now(datetime!(2026-09-07 0:31 UTC));
    let scheduler = tokio::spawn(async move { recovered.run_quota_recovery().await });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while f.reserved() != 0 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    f.state.request_lifecycle().begin_shutdown();
    tokio::time::timeout(std::time::Duration::from_secs(2), scheduler)
        .await
        .unwrap()
        .unwrap();
    f.state.request_lifecycle().wait_drained().await;
}

#[tokio::test]
async fn quota_recovery_corrupt_first_identity_does_not_starve_later_valid_work() {
    let f = Fixture::new().await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    f.state
        .reserve_model_quota(&consumer, "valid_later", 5)
        .await
        .unwrap();
    let connection = rusqlite::Connection::open(f._directory.path().join("customer.db")).unwrap();
    #[cfg(feature = "sqlcipher")]
    connection
        .pragma_update(None, "key", format!("x'{}'", "68".repeat(32)))
        .unwrap();
    connection.execute(
        "INSERT INTO identities(id,email,display_name,password_hash,role,status,can_consume_model,password_change_required,revision,integrity_hmac,created_at,updated_at)
         VALUES('identity_a_corrupt','corrupt@example.test','Corrupt','hash','member','active',1,0,0,'untrusted','2026-09-07T00:00:00.000Z','2026-09-07T00:00:00.000Z')", []
    ).unwrap();
    connection.execute(
        "INSERT INTO user_balances(identity_id,balance_tokens,reserved_tokens,granted_tokens,integrity_hmac,updated_at)
         VALUES('identity_a_corrupt',10,1,10,'untrusted','2026-09-07T00:00:00.000Z')", []
    ).unwrap();
    // Discovery can encounter a corrupted balance before a valid one. Its count
    // is only a hint; the failed signature must prevent this row's release.
    connection.execute(
        "INSERT INTO quota_reservations(id,identity_id,api_key_id,request_id,reserved_tokens,status,revision,integrity_hmac,created_at,expires_at)
         VALUES('corrupt_first','identity_a_corrupt',?,'corrupt_first',1,'active',0,'untrusted','2026-09-07T00:00:00.000Z','2026-09-07T00:30:00.000Z')", [&f.key.id]
    ).unwrap();
    let recovered = f.state.clone().with_now(datetime!(2026-09-07 0:31 UTC));
    recovered
        .recover_expired_model_quota_page("")
        .await
        .unwrap();
    assert_eq!(f.reserved(), 0);
    let status: String = connection
        .query_row(
            "SELECT status FROM quota_reservations WHERE id='corrupt_first'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "active");
    assert!(matches!(
        recovered
            .verified_quota_snapshot("identity_a_corrupt")
            .await,
        Err(ControlError::DataIntegrityInvalid)
    ));
}
