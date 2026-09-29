use super::*;
use aster_license_core::catalog::CapabilityId;

const RECIPIENTS: &str = "/api/admin/vouchers/recipients";
const MEMBERS: &str = "/api/admin/consumption-logs/members";
const CONNECTION: &str = "/api/admin/runners/connection";

async fn call(
    state: &ControlState,
    method: &str,
    path: &str,
    cookie: &str,
    body: Value,
) -> Response {
    router(state.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .header(axum::http::header::COOKIE, cookie)
                .header("referer", "https://example.test/runners")
                .header("x-aster-business-context", "runner")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn value(response: Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

async fn login_cookie(state: &ControlState, kind: &str, email: &str, password: &str) -> String {
    let response = call(
        state,
        "POST",
        &format!("/api/{kind}/auth/login"),
        "",
        json!({"email": email, "password": password}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    response.headers()[axum::http::header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

// Real v2 signatures, persisted history, identity HMACs and HTTP sessions. These
// test-only values do not decide the commercial plans or D03 capability set.
async fn fixture(features: Vec<CapabilityId>) -> (tempfile::TempDir, ControlState, String) {
    let directory = tempdir().unwrap();
    let (document, keys) = signed_v2_license_with_features(features);
    let license = verify_product_license(&document, &keys).unwrap();
    let now = datetime!(2026-09-07 0:00 UTC);
    let history =
        Arc::new(LicenseStateStore::new(directory.path().join("state.json"), &[90; 32]).unwrap());
    history.initialize(&license, now).unwrap();
    let store =
        aster_storage::SqlCipherStore::initialize(&directory.path().join("customer.db"), &[91; 32])
            .unwrap();
    let state = ControlState::new("2.0.1", Some(license))
        .with_storage(ControlStorage::SqlCipher(Arc::new(StdMutex::new(store))))
        .with_license_state(history)
        .with_auth_core(AuthCore::new(&[90; 32], "installation_projection_test").unwrap())
        .with_now(now);
    state
        .initialize_owner_identity(
            "owner@example.test",
            "Owner",
            Zeroizing::new(b"owner-password-strong".to_vec()),
        )
        .await
        .unwrap();
    let first = login_cookie(
        &state,
        "admin",
        "owner@example.test",
        "owner-password-strong",
    )
    .await;
    for path in [RECIPIENTS, MEMBERS, CONNECTION] {
        let response = call(&state, "GET", path, &first, Value::Null).await;
        assert_eq!(response.headers()["X-Aster-Error-Number"], "11003");
    }
    let changed = call(
        &state,
        "POST",
        "/api/admin/auth/password",
        &first,
        json!({"current_password":"owner-password-strong","new_password":"owner-password-updated"}),
    )
    .await;
    assert_eq!(changed.status(), StatusCode::OK);
    let cookie = login_cookie(
        &state,
        "admin",
        "owner@example.test",
        "owner-password-updated",
    )
    .await;
    (directory, state, cookie)
}

async fn reject_direct_admin_mutations(
    state: &ControlState,
    actor: &IdentityRecord,
    member_id: &str,
    integrity_failure: bool,
) {
    let identities = state
        .credential_storage()
        .unwrap()
        .list_member_identities()
        .await
        .unwrap();
    let audit = state.verified_audit_events().await.unwrap();
    let balance = state
        .verified_quota_snapshot(member_id)
        .await
        .unwrap()
        .balance;
    let request = QuotaRequestRecord {
        id: "quota_request_00000000000000000000000000000001".to_owned(),
        identity_id: member_id.to_owned(),
        amount_nanos: 100,
        reason: "Service boundary".to_owned(),
        status: "pending".to_owned(),
        review_note: String::new(),
        reviewed_by: None,
        reviewed_at: None,
        revision: 0,
        integrity_hmac: String::new(),
        created_at: "2026-09-07T00:00:00.000Z".to_owned(),
    };
    let results = [
        state
            .create_member_identity(
                actor,
                "unauthorized@example.test",
                "Unauthorized",
                Zeroizing::new(b"member-password-strong".to_vec()),
            )
            .await
            .map(|_| ()),
        state
            .create_member_identities(
                actor,
                vec![MemberCreationInput {
                    email: "batch@example.test".to_owned(),
                    display_name: "Batch".to_owned(),
                    password: Zeroizing::new(b"member-password-strong".to_vec()),
                }],
            )
            .await
            .map(|_| ()),
        state
            .reset_member_password(
                actor,
                member_id,
                Zeroizing::new(b"reset-password-strong".to_vec()),
            )
            .await,
        state
            .update_member_status(actor, member_id, "disabled", "member.status.update")
            .await
            .map(|_| ()),
        state
            .grant_model_quota_with_audit(
                actor,
                member_id,
                "unauthorized-grant",
                100,
                "Unauthorized grant",
            )
            .await,
        state
            .review_quota_request_atomic(&request, "approved", "Unauthorized", actor)
            .await,
        state
            .refresh_openai_credential_via_runner_inner("credential_missing", Some(actor))
            .await,
        state
            .sync_openai_models_with_audit("account_missing", actor)
            .await
            .map(|_| ()),
    ];
    for (index, result) in results.into_iter().enumerate() {
        let rejected = if integrity_failure {
            matches!(result, Err(ControlError::DataIntegrityInvalid))
        } else {
            matches!(
                result,
                Err(ControlError::Unauthenticated(PrincipalKind::Admin))
            )
        };
        assert!(rejected, "service {index} returned {result:?}");
    }
    assert_eq!(
        state
            .credential_storage()
            .unwrap()
            .list_member_identities()
            .await
            .unwrap(),
        identities
    );
    assert_eq!(state.verified_audit_events().await.unwrap(), audit);
    assert_eq!(
        state
            .verified_quota_snapshot(member_id)
            .await
            .unwrap()
            .balance,
        balance
    );
}

#[tokio::test]
async fn direct_member_creation_requires_a_current_administrator() {
    let (_directory, state, cookie) = fixture(vec![CapabilityId::Member]).await;
    let mut headers = HeaderMap::new();
    headers.insert(axum::http::header::COOKIE, cookie.parse().unwrap());
    let owner = require_admin_ready(&state, &headers).await.unwrap();
    let member = state
        .create_member_identity(
            &owner,
            "actor@example.test",
            "Member actor",
            Zeroizing::new(b"member-password-strong".to_vec()),
        )
        .await
        .unwrap();
    let member_actor = state
        .credential_storage()
        .unwrap()
        .identity_by_id(&member.id)
        .await
        .unwrap()
        .unwrap();
    let before = state
        .credential_storage()
        .unwrap()
        .list_member_identities()
        .await
        .unwrap();
    let result = state
        .create_member_identity(
            &member_actor,
            "unauthorized@example.test",
            "Unauthorized creation",
            Zeroizing::new(b"member-password-strong".to_vec()),
        )
        .await;
    assert!(matches!(
        result,
        Err(ControlError::Unauthenticated(PrincipalKind::Admin))
    ));
    assert_eq!(
        state
            .credential_storage()
            .unwrap()
            .list_member_identities()
            .await
            .unwrap(),
        before
    );
    reject_direct_admin_mutations(&state, &member_actor, &member.id, false).await;
    let mut forged = owner.clone();
    forged.display_name = "Forged admin".to_owned();
    reject_direct_admin_mutations(&state, &forged, &member.id, true).await;
}

#[tokio::test]
async fn direct_admin_services_reject_signed_but_obsolete_or_disabled_actors() {
    let (directory, state, cookie) = fixture(vec![CapabilityId::Member]).await;
    let mut headers = HeaderMap::new();
    headers.insert(axum::http::header::COOKIE, cookie.parse().unwrap());
    let owner = require_admin_ready(&state, &headers).await.unwrap();
    let member = state
        .create_member_identity(
            &owner,
            "target@example.test",
            "Target",
            Zeroizing::new(b"member-password-strong".to_vec()),
        )
        .await
        .unwrap();
    let connection = rusqlite::Connection::open(directory.path().join("customer.db")).unwrap();
    #[cfg(feature = "sqlcipher")]
    connection
        .execute_batch(&format!("PRAGMA key=\"x'{}'\";", "5b".repeat(32)))
        .unwrap();
    for (role, status, password_change_required) in [
        ("owner", "disabled", false),
        ("member", "active", false),
        ("owner", "active", true),
        ("owner", "active", false),
    ] {
        let mut current = owner.clone();
        current.role = role.to_owned();
        current.status = status.to_owned();
        current.password_change_required = password_change_required;
        current.revision += 1;
        current.integrity_hmac = state
            .auth_core()
            .unwrap()
            .identity_integrity_hmac(identity_integrity_input(&current))
            .unwrap();
        connection.execute("UPDATE identities SET role=?1,status=?2,password_change_required=?3,revision=?4,integrity_hmac=?5 WHERE id=?6", rusqlite::params![current.role, current.status, current.password_change_required, current.revision, current.integrity_hmac, current.id]).unwrap();
        reject_direct_admin_mutations(&state, &owner, &member.id, false).await;
        if role != "owner" || status != "active" || password_change_required {
            reject_direct_admin_mutations(&state, &current, &member.id, false).await;
        } else {
            // A freshly read, ready administrator still succeeds; rejecting
            // an obsolete record must not lock out the current identity.
            state
                .grant_model_quota_with_audit(
                    &current,
                    &member.id,
                    "current-admin",
                    25,
                    "Current administrator",
                )
                .await
                .unwrap();
            assert_eq!(
                state
                    .verified_quota_snapshot(&member.id)
                    .await
                    .unwrap()
                    .balance
                    .balance_tokens,
                25
            );
            state
                .update_member_status(&current, &member.id, "disabled", "member.status.update")
                .await
                .unwrap();
            state
                .grant_model_quota_with_audit(
                    &current,
                    &member.id,
                    "disabled-member",
                    10,
                    "Disabled member remains manageable",
                )
                .await
                .unwrap();
            state
                .update_member_status(&current, &member.id, "deleted", "member.delete")
                .await
                .unwrap();
            let audit = state.verified_audit_events().await.unwrap();
            let rejected = state
                .grant_model_quota_with_audit(
                    &current,
                    &member.id,
                    "deleted-member",
                    100,
                    "Must not grant",
                )
                .await;
            assert!(matches!(rejected, Err(ControlError::IdentityNotFound)));
            assert_eq!(
                state
                    .verified_quota_snapshot(&member.id)
                    .await
                    .unwrap()
                    .balance
                    .balance_tokens,
                35
            );
            assert_eq!(state.verified_audit_events().await.unwrap(), audit);
        }
    }
}

#[tokio::test]
async fn direct_maintenance_task_rechecks_current_admin_before_authorization() {
    let (directory, state, cookie) =
        fixture(vec![CapabilityId::Gateway, CapabilityId::Runner]).await;
    let mut headers = HeaderMap::new();
    headers.insert(axum::http::header::COOKIE, cookie.parse().unwrap());
    let owner = require_admin_ready(&state, &headers).await.unwrap();
    let credential = transport_credential_fixture();
    let binding = RunnerTaskBinding::credential(
        "openai",
        &credential.id,
        credential.credential_revision,
        "chatgpt.com",
    );
    let payload = serde_json::to_vec(&UpstreamHttpRequest {
        method: "GET".to_owned(),
        url: OPENAI_CODEX_MODELS_ENDPOINT.to_owned(),
        headers: vec![],
        body_base64url: String::new(),
    })
    .unwrap();
    let operation = RunnerTaskAuthorization::DiscoverModels {
        actor: Some(&owner),
        credential: &credential,
    };
    assert!(
        operation
            .authorize(&state, &binding, &payload)
            .await
            .is_ok()
    );
    let mut disabled = owner.clone();
    disabled.status = "disabled".to_owned();
    disabled.revision += 1;
    disabled.integrity_hmac = state
        .auth_core()
        .unwrap()
        .identity_integrity_hmac(identity_integrity_input(&disabled))
        .unwrap();
    let connection = rusqlite::Connection::open(directory.path().join("customer.db")).unwrap();
    #[cfg(feature = "sqlcipher")]
    connection
        .execute_batch(&format!("PRAGMA key=\"x'{}'\";", "5b".repeat(32)))
        .unwrap();
    connection
        .execute(
            "UPDATE identities SET status=?1,revision=?2,integrity_hmac=?3 WHERE id=?4",
            rusqlite::params![
                disabled.status,
                disabled.revision,
                disabled.integrity_hmac,
                disabled.id
            ],
        )
        .unwrap();
    let rejected = operation.authorize(&state, &binding, &payload).await;
    assert!(matches!(
        rejected,
        Err(ControlError::Unauthenticated(PrincipalKind::Admin))
    ));
}

#[tokio::test]
async fn direct_self_password_change_checks_target_and_current_password_before_mutation() {
    let (_directory, state, cookie) = fixture(vec![CapabilityId::Member]).await;
    let mut headers = HeaderMap::new();
    headers.insert(axum::http::header::COOKIE, cookie.parse().unwrap());
    let owner = require_admin_ready(&state, &headers).await.unwrap();
    let mut members = Vec::new();
    for name in ["first", "second"] {
        let created = state
            .create_member_identity(
                &owner,
                &format!("{name}@example.test"),
                name,
                Zeroizing::new(b"member-password-strong".to_vec()),
            )
            .await
            .unwrap();
        members.push(
            state
                .credential_storage()
                .unwrap()
                .identity_by_id(&created.id)
                .await
                .unwrap()
                .unwrap(),
        );
    }
    let audit = state.verified_audit_events().await.unwrap();
    let crossed = state
        .apply_identity_password(
            PasswordMutationActor::SelfService {
                actor: &members[0],
                current_password: b"member-password-strong",
            },
            &members[1],
            Zeroizing::new(b"member-password-updated".to_vec()),
            false,
            "identity.password.update",
            PrincipalKind::Member,
        )
        .await;
    assert!(matches!(
        crossed,
        Err(ControlError::Unauthenticated(PrincipalKind::Member))
    ));
    let incorrect = state
        .apply_identity_password(
            PasswordMutationActor::SelfService {
                actor: &members[0],
                current_password: b"incorrect-current-password",
            },
            &members[0],
            Zeroizing::new(b"member-password-updated".to_vec()),
            false,
            "identity.password.update",
            PrincipalKind::Member,
        )
        .await;
    assert!(matches!(
        incorrect,
        Err(ControlError::CurrentPasswordInvalid(PrincipalKind::Member))
    ));
    assert_eq!(state.verified_audit_events().await.unwrap(), audit);
    for original in &members {
        assert_eq!(
            state
                .credential_storage()
                .unwrap()
                .identity_by_id(&original.id)
                .await
                .unwrap()
                .unwrap(),
            *original
        );
    }
    // First sign-in is an allowed self-service operation, even though the
    // ordinary member business endpoints require the password change first.
    assert!(members[0].password_change_required);
    state
        .apply_identity_password(
            PasswordMutationActor::SelfService {
                actor: &members[0],
                current_password: b"member-password-strong",
            },
            &members[0],
            Zeroizing::new(b"member-password-updated".to_vec()),
            false,
            "identity.password.update",
            PrincipalKind::Member,
        )
        .await
        .unwrap();
    let current = state
        .credential_storage()
        .unwrap()
        .identity_by_id(&members[0].id)
        .await
        .unwrap()
        .unwrap();
    assert!(!current.password_change_required);
    assert!(
        state
            .auth_core()
            .unwrap()
            .verify_password(&current.password_hash, b"member-password-updated")
            .unwrap()
    );
    reject_direct_admin_mutations(&state, &current, &members[1].id, false).await;

    let response = call(&state, "POST", "/api/admin/vouchers", &cookie, json!({"name":"Direct redemption", "quota_tokens":75,"max_redemptions":1,"valid_days":null,"recipient_user_ids":[current.id, members[1].id]})).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let member_cookie = login_cookie(
        &state,
        "member",
        "first@example.test",
        "member-password-updated",
    )
    .await;
    let deliveries = call(
        &state,
        "GET",
        "/api/member/vouchers",
        &member_cookie,
        Value::Null,
    )
    .await;
    assert_eq!(deliveries.status(), StatusCode::OK);
    let own = value(deliveries).await["items"][0]["delivery_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let before = state
        .verified_quota_snapshot(&current.id)
        .await
        .unwrap()
        .balance;
    for actor in [&owner, &members[0], &members[1]] {
        let denied = state.redeem_voucher_atomic(actor, None, Some(&own)).await;
        assert!(matches!(
            denied,
            Err(ControlError::Unauthenticated(PrincipalKind::Member))
        ));
    }
    assert_eq!(
        state
            .verified_quota_snapshot(&current.id)
            .await
            .unwrap()
            .balance,
        before
    );
    state
        .redeem_voucher_atomic(&current, None, Some(&own))
        .await
        .unwrap();
    assert_eq!(
        state
            .verified_quota_snapshot(&current.id)
            .await
            .unwrap()
            .balance
            .balance_tokens,
        75
    );
    assert_eq!(
        state
            .verified_quota_snapshot(&members[1].id)
            .await
            .unwrap()
            .balance
            .balance_tokens,
        0
    );
}

#[tokio::test]
async fn direct_member_creation_rechecks_administrator_after_password_preparation() {
    let (directory, mut state, cookie) = fixture(vec![CapabilityId::Member]).await;
    let mut headers = HeaderMap::new();
    headers.insert(axum::http::header::COOKIE, cookie.parse().unwrap());
    let owner = require_admin_ready(&state, &headers).await.unwrap();
    let mut revoked = owner.clone();
    revoked.status = "disabled".to_owned();
    revoked.revision += 1;
    revoked.integrity_hmac = state
        .auth_core()
        .unwrap()
        .identity_integrity_hmac(identity_integrity_input(&revoked))
        .unwrap();
    let audit = state.verified_audit_events().await.unwrap();
    let database = directory.path().join("customer.db");
    let observed = Arc::new(AtomicU64::new(0));
    let changed = Arc::new(AtomicBool::new(false));
    let hook_observed = Arc::clone(&observed);
    let hook_changed = Arc::clone(&changed);
    state.now = Arc::new(move || {
        // First observation is the initial License check. The next is the
        // creation timestamp after password preparation, before the insert.
        if hook_observed.fetch_add(1, Ordering::SeqCst) == 1 {
            let connection = rusqlite::Connection::open(&database).unwrap();
            #[cfg(feature = "sqlcipher")]
            connection
                .execute_batch(&format!("PRAGMA key=\"x'{}'\";", "5b".repeat(32)))
                .unwrap();
            connection
                .execute(
                    "UPDATE identities SET status=?1,revision=?2,integrity_hmac=?3 WHERE id=?4",
                    rusqlite::params![
                        revoked.status,
                        revoked.revision,
                        revoked.integrity_hmac,
                        revoked.id
                    ],
                )
                .unwrap();
            hook_changed.store(true, Ordering::SeqCst);
        }
        datetime!(2026-09-07 0:00 UTC)
    });
    let result = state
        .create_member_identity(
            &owner,
            "late@example.test",
            "Late",
            Zeroizing::new(b"member-password-strong".to_vec()),
        )
        .await;
    assert!(changed.load(Ordering::SeqCst));
    assert!(matches!(
        result,
        Err(ControlError::Unauthenticated(PrincipalKind::Admin))
    ));
    assert!(
        state
            .credential_storage()
            .unwrap()
            .list_member_identities()
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(state.verified_audit_events().await.unwrap(), audit);
}

#[tokio::test]
async fn minimal_member_projection_preserves_scope_and_omits_management_data() {
    let (directory, state, cookie) = fixture(vec![CapabilityId::Member]).await;
    let mut headers = HeaderMap::new();
    headers.insert(axum::http::header::COOKIE, cookie.parse().unwrap());
    let owner = require_admin_ready(&state, &headers).await.unwrap();
    let mut created = Vec::new();
    for name in ["active", "disabled", "deleted"] {
        let member = state
            .create_member_identity(
                &owner,
                &format!("{name}@example.test"),
                name,
                Zeroizing::new(b"member-password-strong".to_vec()),
            )
            .await
            .unwrap();
        if name != "active" {
            state
                .update_member_status(&owner, &member.id, name, "member.update")
                .await
                .unwrap();
        }
        created.push(member);
    }
    let active = json!({"id":created[0].id,"email":"active@example.test","display_name":"active"});
    let disabled =
        json!({"id":created[1].id,"email":"disabled@example.test","display_name":"disabled"});
    let recipients = call(&state, "GET", RECIPIENTS, &cookie, Value::Null).await;
    assert_eq!(recipients.status(), StatusCode::OK);
    assert_eq!(value(recipients).await, json!({"items":[active.clone()]}));
    let members = call(&state, "GET", MEMBERS, &cookie, Value::Null).await;
    assert_eq!(members.status(), StatusCode::OK);
    let members = value(members).await;
    let items = members["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert!(items.contains(&active) && items.contains(&disabled));

    // The selector and the actual write agree on eligible recipients.
    for (index, expected) in [(0, StatusCode::CREATED), (1, StatusCode::NOT_FOUND)] {
        let response = call(
            &state,
            "POST",
            "/api/admin/vouchers",
            &cookie,
            json!({"name":"Assigned test","quota_tokens":1000,"max_redemptions":1,
                "recipient_user_ids":[created[index].id],"valid_days":30}),
        )
        .await;
        assert_eq!(response.status(), expected);
        if index == 0 {
            assert_eq!(value(response).await["vouchers"][0]["recipient"], active);
        }
    }

    let member_cookie = login_cookie(
        &state,
        "member",
        "active@example.test",
        "member-password-strong",
    )
    .await;
    let impersonating_cookie = member_cookie.replace(
        cookie_name(PrincipalKind::Member),
        cookie_name(PrincipalKind::Admin),
    );
    for path in [RECIPIENTS, MEMBERS, CONNECTION] {
        for unauthorized_cookie in ["", member_cookie.as_str(), impersonating_cookie.as_str()] {
            let response = call(
                &state,
                "GET",
                &format!("{path}?context=member&fields=all"),
                unauthorized_cookie,
                Value::Null,
            )
            .await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
    }
    let response = call(&state, "GET", CONNECTION, &cookie, Value::Null).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // Valid identity data is necessary even though quota/account fields are not
    // loaded for a selector. Database edits cannot inject an unverified label.
    let connection = rusqlite::Connection::open(directory.path().join("customer.db")).unwrap();
    #[cfg(feature = "sqlcipher")]
    connection
        .execute_batch(&format!("PRAGMA key=\"x'{}'\";", "5b".repeat(32)))
        .unwrap();
    connection
        .execute(
            "UPDATE identities SET display_name='edited' WHERE id=?1",
            [&created[0].id],
        )
        .unwrap();
    for path in [RECIPIENTS, MEMBERS] {
        let response = call(&state, "GET", path, &cookie, Value::Null).await;
        assert!(!response.status().is_success());
        assert!(!value(response).await.to_string().contains("edited"));
    }
}

#[tokio::test]
async fn minimal_runner_projection_cannot_be_widened_by_context_or_fields() {
    let (_directory, state, cookie) = fixture(vec![CapabilityId::Runner]).await;
    let updated = call(
        &state,
        "PUT",
        "/api/admin/settings",
        &cookie,
        json!({"public_api_base_url":"https://api.example.test","usage_multiplier":1.5}),
    )
    .await;
    assert_eq!(updated.status(), StatusCode::OK);
    let response = call(
        &state,
        "GET",
        &format!("{CONNECTION}?context=settings&fields=all"),
        &cookie,
        Value::Null,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        value(response).await,
        json!({"public_api_base_url":"https://api.example.test"})
    );
    for path in [RECIPIENTS, MEMBERS] {
        let response = call(
            &state,
            "GET",
            &format!("{path}?context=runner"),
            &cookie,
            Value::Null,
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}

#[tokio::test]
async fn unrelated_license_does_not_grant_any_helper_projection() {
    let (_directory, state, cookie) = fixture(vec![CapabilityId::Gateway]).await;
    for path in [RECIPIENTS, MEMBERS, CONNECTION] {
        let response = call(
            &state,
            "GET",
            &format!("{path}?context=runner"),
            &cookie,
            Value::Null,
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}

#[tokio::test]
async fn upstream_external_operations_reject_incomplete_rights_before_preparation() {
    for features in [
        vec![CapabilityId::Gateway],
        vec![CapabilityId::Runner],
        vec![],
    ] {
        let (_directory, state, cookie) = fixture(features).await;
        for path in [
            "/api/admin/upstream-providers/openai/enrollments",
            "/api/admin/upstream-enrollments/enrollment_test/actions",
            "/api/admin/upstream-accounts/account_test/models/sync",
            "/api/admin/upstream-accounts/account_test/credentials/credential_test/refresh",
        ] {
            let response = call(&state, "POST", path, &cookie, json!({})).await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
            assert_eq!(
                response.headers()["X-Aster-Error-Number"],
                "51008",
                "{path}"
            );
        }
        assert!(state.oauth_sessions.lock().await.is_empty());
        // The same rule holds for direct service calls before record lookup or a refresh lease.
        assert!(matches!(
            state.sync_openai_models("account_test").await,
            Err(ControlError::Policy(_))
        ));
        let mut headers = HeaderMap::new();
        headers.insert(axum::http::header::COOKIE, cookie.parse().unwrap());
        let actor = require_admin_ready(&state, &headers).await.unwrap();
        assert!(matches!(
            state
                .refresh_openai_credential_via_runner_inner("credential_test", Some(&actor))
                .await,
            Err(ControlError::Policy(_))
        ));
    }
}

#[tokio::test]
async fn upstream_authorization_preparation_uses_gateway_and_runner_without_member() {
    let (_directory, state, cookie) =
        fixture(vec![CapabilityId::Gateway, CapabilityId::Runner]).await;
    let response = call(
        &state,
        "POST",
        "/api/admin/upstream-providers/openai/enrollments",
        &cookie,
        json!({}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let transition = value(response).await;
    assert_eq!(transition["state"], "action_required");
    let id = transition["enrollment_id"].as_str().unwrap();
    assert!(state.oauth_sessions.lock().await.contains_key(id));
    let canceled = call(
        &state,
        "DELETE",
        &format!("/api/admin/upstream-enrollments/{id}"),
        &cookie,
        Value::Null,
    )
    .await;
    assert_eq!(canceled.status(), StatusCode::OK);
    assert!(state.oauth_sessions.lock().await.is_empty());
}

#[tokio::test]
async fn minimal_gateway_manages_existing_accounts_credentials_and_models_without_runner() {
    let (_directory, state, cookie) = fixture(vec![CapabilityId::Gateway]).await;
    let state = state.with_credential_vault(
        CredentialVault::new(&[95; 32], "installation_projection_test").unwrap(),
    );
    let account_id = "account_00000000000000000000000000000001";
    let model_id = "model_00000000000000000000000000000001";
    // Seed pre-existing resources, with real encrypted credential material. All
    // reads and mutations below use authenticated production HTTP handlers.
    let ControlStorage::SqlCipher(store) = state.credential_storage().unwrap() else {
        panic!("expected isolated SQLite store");
    };
    store
        .lock()
        .unwrap()
        .insert_upstream_account_unchecked(
            account_id,
            "openai",
            "existing-subject",
            "existing@example.test",
            "2026-09-07T00:00:00.000Z",
        )
        .unwrap();
    let credential_id = state.create_credential_instance(
        account_id, b"existing-refresh-secret",
        br#"{"refresh_token":"existing-refresh-secret","access_token":"existing-access-secret"}"#,
        "2026-09-08T00:00:00.000Z",
    ).await.unwrap();
    state
        .credential_storage()
        .unwrap()
        .replace_account_models(
            account_id,
            &[DiscoveredModel {
                id: model_id.to_owned(),
                public_name: "existing-model".to_owned(),
                display_name: "Existing model".to_owned(),
                upstream_name: "existing-model".to_owned(),
            }],
            "2026-09-07T00:00:00.000Z",
        )
        .await
        .unwrap();
    assert_eq!(state.runner_hub.online_count().await, 0);

    let accounts = call(
        &state,
        "GET",
        "/api/admin/upstream-accounts",
        &cookie,
        Value::Null,
    )
    .await;
    assert_eq!(accounts.status(), StatusCode::OK);
    let accounts = value(accounts).await;
    assert_eq!(accounts["items"].as_array().unwrap().len(), 1);
    assert_eq!(accounts["items"][0]["id"], account_id);
    let credentials_path = format!("/api/admin/upstream-accounts/{account_id}/credentials");
    let credentials = call(&state, "GET", &credentials_path, &cookie, Value::Null).await;
    assert_eq!(credentials.status(), StatusCode::OK);
    let credentials = value(credentials).await;
    assert_eq!(credentials["items"][0]["id"], credential_id);
    assert!(!credentials.to_string().contains("existing-refresh-secret"));
    assert!(!credentials.to_string().contains("existing-access-secret"));
    let models = call(&state, "GET", "/api/admin/models", &cookie, Value::Null).await;
    assert_eq!(models.status(), StatusCode::OK);
    let models = value(models).await;
    assert_eq!(models["items"][0]["id"], model_id);
    assert_eq!(models["items"][0]["available"], false);

    for enabled in [false, true] {
        for path in [
            format!("/api/admin/upstream-accounts/{account_id}"),
            format!("/api/admin/models/{model_id}"),
        ] {
            let response = call(&state, "PATCH", &path, &cookie, json!({"enabled":enabled})).await;
            assert_eq!(response.status(), StatusCode::OK, "{path}");
        }
        let storage = state.credential_storage().unwrap();
        let saved_account = storage
            .upstream_account_by_id(account_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            saved_account.status,
            if enabled { "active" } else { "disabled" }
        );
        let saved_models = storage.list_models().await.unwrap();
        assert_eq!(saved_models[0].enabled, enabled);
        assert_eq!(
            storage
                .credential_instances_for_account(account_id)
                .await
                .unwrap()
                .len(),
            1
        );
    }
    // Existing IDs and valid credentials must not turn a missing capability
    // into a downstream error or begin a refresh lease.
    for path in [
        format!("/api/admin/upstream-accounts/{account_id}/models/sync"),
        format!("{credentials_path}/{credential_id}/refresh"),
    ] {
        let response = call(&state, "POST", &path, &cookie, Value::Null).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
        assert_eq!(response.headers()["X-Aster-Error-Number"], "51008");
    }
    let saved_credential = state
        .decrypt_credential_instance(&credential_id)
        .await
        .unwrap();
    assert_eq!(saved_credential.credential_revision, 0);
    let lease = state
        .begin_credential_refresh(&credential_id)
        .await
        .expect("denial left no refresh lease");
    state
        .cancel_credential_refresh(&credential_id, lease.lease_token.as_str())
        .await
        .unwrap();

    let deleted = call(
        &state,
        "DELETE",
        &format!("/api/admin/upstream-accounts/{account_id}"),
        &cookie,
        Value::Null,
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::OK);
    assert_eq!(value(deleted).await["deleted_credentials"], 1);
    let storage = state.credential_storage().unwrap();
    assert!(
        storage
            .upstream_account_by_id(account_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        storage
            .credential_instance_by_id(&credential_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        storage
            .gateway_route_candidates("existing-model")
            .await
            .unwrap()
            .is_empty()
    );
    let audits = state.verified_audit_events().await.unwrap();
    for (action, target, count) in [
        ("upstream_account.status.update", account_id, 2),
        ("model.update", model_id, 2),
        ("upstream_account.delete", account_id, 1),
    ] {
        let events = audits
            .iter()
            .filter(|event| event.action == action)
            .collect::<Vec<_>>();
        assert_eq!(events.len(), count, "{action}");
        assert!(
            events
                .iter()
                .all(|event| event.target_id.as_deref() == Some(target)
                    && event.actor_identity_id.is_some()
                    && event.outcome == "succeeded")
        );
    }
    assert!(!audits.iter().any(|event| matches!(
        event.action.as_str(),
        "upstream_model.sync" | "upstream_credential.refresh"
    )));
}
