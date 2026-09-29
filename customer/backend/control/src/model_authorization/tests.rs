use super::*;
use crate::tests::{signed_v2_license, v2_policy};
use aster_license_core::{catalog::CapabilityId, v2};
use ed25519_dalek::SigningKey;
use tempfile::{TempDir, tempdir};
use time::macros::datetime;

mod durable_settlement;
mod execution;
mod quota_recovery;
mod settlement_outbox;

struct Fixture {
    _directory: TempDir,
    state: ControlState,
    store: Arc<StdMutex<aster_storage::SqlCipherStore>>,
    identity: IdentityRecord,
    key: ApiKeyRecord,
    token: String,
}

fn license(features: Vec<CapabilityId>, replacement: bool) -> VerifiedProductLicense {
    // An entitlement replacement uses a paid order. Updating a public free
    // document now requires the separate capacity-checked switch transaction.
    let (source, _) = signed_v2_license(if replacement { 2 } else { 0 }, None);
    let mut document: v2::Document = serde_json::from_slice(&source).unwrap();
    document.claims.entitlements.features = features;
    if replacement {
        document.claims.license_id = "test_replacement".to_owned();
        document.claims.issued_at = "2026-09-02T00:00:00.000Z".to_owned();
    }
    let key = SigningKey::from_bytes(&[42; 32]);
    let document = v2::sign(document.claims, &key).unwrap();
    let mut trusted = TrustedLicenseKeys::new();
    trusted
        .insert_scoped(
            document.claims.key_id.clone(),
            key.verifying_key(),
            v2_policy(&document),
        )
        .unwrap();
    verify_product_license(&serde_json::to_vec(&document).unwrap(), &trusted).unwrap()
}

impl Fixture {
    async fn new() -> Self {
        Self::with_features(vec![CapabilityId::Member]).await
    }

    async fn with_features(features: Vec<CapabilityId>) -> Self {
        let directory = tempdir().unwrap();
        let now = datetime!(2026-09-07 0:00 UTC);
        let timestamp = format_database_time(now).unwrap();
        let auth = AuthCore::new(&[105; 32], "installation_consumption_test").unwrap();
        let mut identity = IdentityRecord {
            id: "identity_consumer".to_owned(),
            email: "consumer@example.test".to_owned(),
            display_name: "Consumer".to_owned(),
            password_hash: auth.hash_password(b"test-password-strong").unwrap(),
            role: "member".to_owned(),
            status: "active".to_owned(),
            can_consume_model: true,
            password_change_required: false,
            revision: 0,
            integrity_hmac: String::new(),
            created_at: timestamp.clone(),
            updated_at: timestamp.clone(),
        };
        identity.integrity_hmac = auth
            .identity_integrity_hmac(identity_integrity_input(&identity))
            .unwrap();
        let issued = auth.issue_api_key().unwrap();
        let mut key = ApiKeyRecord {
            id: "key_consumer".to_owned(),
            identity_id: identity.id.clone(),
            name: "consumer".to_owned(),
            key_hash: issued.digest().to_owned(),
            key_prefix: issued.prefix().to_owned(),
            status: "active".to_owned(),
            revision: 0,
            integrity_hmac: String::new(),
            created_at: timestamp.clone(),
            last_used_at: None,
        };
        key.integrity_hmac = auth
            .api_key_integrity_hmac(api_key_integrity_input(&key))
            .unwrap();
        let setup = ControlState::new("2.0.1", None).with_auth_core(auth.clone());
        let registry = setup
            .security_state_record(std::slice::from_ref(&identity.id), 0, &timestamp)
            .unwrap();
        let balance = setup
            .initial_balance_record(&identity.id, &timestamp)
            .unwrap();
        let mut store = aster_storage::SqlCipherStore::initialize(
            &directory.path().join("customer.db"),
            &[104; 32],
        )
        .unwrap();
        store
            .initialize_first_owner(&identity, &registry, &balance)
            .unwrap();
        store.insert_api_key_unchecked(&key).unwrap();
        let store = Arc::new(StdMutex::new(store));
        let license = license(features, false);
        let history = Arc::new(
            LicenseStateStore::new(directory.path().join("license-state.json"), &[105; 32])
                .unwrap(),
        );
        history.initialize(&license, now).unwrap();
        let state = ControlState::new("2.0.1", Some(license))
            .with_storage(ControlStorage::SqlCipher(Arc::clone(&store)))
            .with_auth_core(auth)
            .with_license_state(history)
            .with_now(now);
        state
            .grant_model_quota(&identity.id, "test-grant", 1000, "test quota")
            .await
            .unwrap();
        Self {
            _directory: directory,
            state,
            store,
            identity,
            key,
            token: issued.plaintext().to_owned(),
        }
    }

    fn revoke_key(&self) {
        let mut next = self.key.clone();
        next.status = "revoked".to_owned();
        next.revision += 1;
        next.integrity_hmac = self
            .state
            .auth_core()
            .unwrap()
            .api_key_integrity_hmac(api_key_integrity_input(&next))
            .unwrap();
        assert!(
            self.store
                .lock()
                .unwrap()
                .update_api_key_status(
                    &next.id,
                    &next.identity_id,
                    self.key.revision,
                    &next.status,
                    &next.integrity_hmac,
                )
                .unwrap()
        );
    }

    fn reserved(&self) -> i64 {
        self.store
            .lock()
            .unwrap()
            .quota_state_snapshot(&self.identity.id)
            .unwrap()
            .unwrap()
            .balance
            .reserved_tokens
    }
}

#[tokio::test]
async fn real_consumer_can_reserve_but_cannot_cross_state_or_duplicate_request() {
    let f = Fixture::new().await;
    assert!(matches!(
        f.state.authorize_model_consumer("not-an-api-key").await,
        Err(ControlError::ExternalApiKeyInvalid)
    ));
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    let other_scope = f
        .state
        .clone()
        .with_storage(ControlStorage::SqlCipher(Arc::clone(&f.store)));
    assert!(matches!(
        other_scope
            .reserve_model_quota(&consumer, "cross-scope", 1)
            .await,
        Err(ControlError::ExternalApiKeyInvalid)
    ));
    assert_eq!(f.reserved(), 0);
    let reservation = f
        .state
        .clone()
        .reserve_model_quota(&consumer, "one-reservation", 100)
        .await
        .unwrap();
    assert_eq!(reservation.identity_id, f.identity.id);
    assert_eq!(f.reserved(), 100);
    assert!(matches!(
        f.state
            .reserve_model_quota(&consumer, "one-reservation", 100)
            .await,
        Err(ControlError::DuplicateGatewayRequest)
    ));
    assert_eq!(f.reserved(), 100);
}

#[tokio::test]
async fn revoked_key_and_changed_license_invalidate_existing_consumer() {
    let f = Fixture::new().await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    f.revoke_key();
    assert!(matches!(
        f.state.reserve_model_quota(&consumer, "revoked", 1).await,
        Err(ControlError::ExternalApiKeyInvalid)
    ));
    assert_eq!(f.reserved(), 0);

    let f = Fixture::new().await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    let replacement = license(vec![CapabilityId::Runner], true);
    f.state
        .license_state
        .as_ref()
        .unwrap()
        .accept_replacement(&replacement, (f.state.now)())
        .unwrap();
    f.state.replace_license(replacement).unwrap();
    assert!(matches!(
        f.state
            .reserve_model_quota(&consumer, "missing-feature", 1)
            .await,
        Err(ControlError::Policy(_))
    ));
    assert_eq!(f.reserved(), 0);
}

#[tokio::test]
async fn expired_license_and_shutdown_reject_reservations_without_mutation() {
    let f = Fixture::new().await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    let expired = f.state.clone().with_now(datetime!(2027-09-02 0:00 UTC));
    assert!(matches!(
        expired.reserve_model_quota(&consumer, "expired", 1).await,
        Err(ControlError::Policy(_))
    ));
    assert_eq!(f.reserved(), 0);
    let f = Fixture::new().await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    f.state.drain_licensed_mutations().await;
    assert!(
        f.state
            .reserve_model_quota(&consumer, "shutdown", 1)
            .await
            .is_err()
    );
    assert_eq!(f.reserved(), 0);
}

#[tokio::test]
async fn transaction_rejects_key_revoked_after_permit_and_preparation() {
    let f = Fixture::new().await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    let permit = ReservationPermit::authorize(&f.state, &consumer)
        .await
        .unwrap();
    let prepared = PreparedReservation::new(&f.state, &permit.subject, "racing-revocation", 100)
        .await
        .unwrap();
    f.revoke_key();
    let guard = f
        .state
        .guard_licensed_mutation(Some(&permit.license), "member")
        .unwrap();
    let result = finish_licensed_mutation(
        Arc::clone(&f.state.mutation_tasks),
        guard,
        prepared.commit(
            f.state.credential_storage().unwrap().clone(),
            permit.subject,
        ),
    )
    .await
    .unwrap();
    assert!(result.is_none());
    assert_eq!(f.reserved(), 0);
}

#[tokio::test]
async fn telemetry_changes_do_not_conflict_with_a_verified_reservation() {
    let f = Fixture::new().await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    let permit = ReservationPermit::authorize(&f.state, &consumer)
        .await
        .unwrap();
    let prepared = PreparedReservation::new(&f.state, &permit.subject, "telemetry", 100)
        .await
        .unwrap();
    f.state
        .credential_storage()
        .unwrap()
        .touch_api_key_last_used(&f.key.id, "2026-09-07T00:00:01.000Z")
        .await
        .unwrap();
    let guard = f
        .state
        .guard_licensed_mutation(Some(&permit.license), "member")
        .unwrap();
    let result = finish_licensed_mutation(
        Arc::clone(&f.state.mutation_tasks),
        guard,
        prepared.commit(
            f.state.credential_storage().unwrap().clone(),
            permit.subject,
        ),
    )
    .await
    .unwrap();
    assert!(result.is_some());
    assert_eq!(f.reserved(), 100);
}
