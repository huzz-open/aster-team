use super::*;
use aster_license_core::v2;
use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use tempfile::tempdir;
use time::macros::datetime;
use tower::ServiceExt;

pub(super) async fn call(
    state: &ControlState,
    method: &str,
    path: &str,
    cookie: &str,
    body: Value,
) -> (StatusCode, HeaderMap, Value) {
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value =
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes)));
    (status, headers, value)
}

pub(super) async fn login_cookie(state: &ControlState, kind: &str, password: &str) -> String {
    let (status, headers, body) = call(
        state,
        "POST",
        &format!("/api/{kind}/auth/login"),
        "",
        json!({"email": format!("{kind}@example.test"), "password": password}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{kind}: {body}");
    headers["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn expired_v2_retains_authenticated_data_and_reduction_but_blocks_new_usage() {
    let directory = tempdir().unwrap();
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../contracts/test-vectors/license.v2.json"
    ))
    .unwrap();
    let document: v2::Document = serde_json::from_value(
        fixture["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == "commercial")
            .unwrap()["document"]
            .clone(),
    )
    .unwrap();
    let mut keys = TrustedLicenseKeys::new();
    keys.insert_scoped_spki_base64url(
        &document.claims.key_id,
        fixture["public_key_spki"].as_str().unwrap(),
        super::tests::v2_policy(&document),
    )
    .unwrap();
    let license = verify_product_license(&serde_json::to_vec(&document).unwrap(), &keys).unwrap();
    let history_path = directory.path().join("history.json");
    let history = Arc::new(LicenseStateStore::new(&history_path, &[82; 32]).unwrap());
    let before = datetime!(2026-09-02 0:00 UTC);
    history.initialize(&license, before).unwrap();
    let database =
        aster_storage::SqlCipherStore::initialize(&directory.path().join("customer.db"), &[83; 32])
            .unwrap();
    database
        .insert_upstream_account_unchecked(
            "account_0123456789abcdef0123456789abcdef",
            "openai",
            "retained-subject",
            "upstream@example.test",
            "2026-09-02T00:00:00.000Z",
        )
        .unwrap();
    let active = ControlState::new("2.1.0", Some(license))
        .with_license_state(history)
        .with_storage(ControlStorage::SqlCipher(Arc::new(StdMutex::new(database))))
        .with_auth_core(AuthCore::new(&[84; 32], "retention-test").unwrap())
        .with_now(before);
    active
        .initialize_owner_identity(
            "admin@example.test",
            "Owner",
            Zeroizing::new(b"admin-password-strong".to_vec()),
        )
        .await
        .unwrap();
    active
        .reset_admin_password(
            "admin@example.test",
            Zeroizing::new(b"admin-password-strong".to_vec()),
        )
        .await
        .unwrap();
    let actor = active
        .credential_storage()
        .unwrap()
        .identity_by_email("admin@example.test")
        .await
        .unwrap()
        .unwrap();
    let runner = active
        .bootstrap_local_runner(
            "admin@example.test",
            "retained-runner",
            "linux",
            "x86_64",
            4,
        )
        .await
        .unwrap();
    let member = active
        .create_member_identity(
            &actor,
            "member@example.test",
            "Member",
            Zeroizing::new(b"member-password-strong".to_vec()),
        )
        .await
        .unwrap();
    let initial_cookie = login_cookie(&active, "member", "member-password-strong").await;
    let changed = call(&active, "POST", "/api/member/auth/password", &initial_cookie,
        json!({"current_password": "member-password-strong", "new_password": "member-password-updated"})).await;
    assert_eq!(changed.0, StatusCode::OK, "{}", changed.2);
    let cookie = login_cookie(&active, "member", "member-password-updated").await;
    let created = call(
        &active,
        "POST",
        "/api/member/keys",
        &cookie,
        json!({"name": "before expiry"}),
    )
    .await;
    assert_eq!(created.0, StatusCode::CREATED, "{}", created.2);
    let key_id = created.2["api_key"]["id"].as_str().unwrap();

    let expired = active.clone().with_now(datetime!(2027-09-02 0:00 UTC));
    let admin_cookie = login_cookie(&expired, "admin", "admin-password-strong").await;
    let member_cookie = login_cookie(&expired, "member", "member-password-updated").await;
    for path in [
        format!("/api/admin/runners/{}", runner.runner_id),
        "/api/admin/upstream-accounts/account_0123456789abcdef0123456789abcdef".to_owned(),
    ] {
        let disabled = call(
            &expired,
            "PATCH",
            &path,
            &admin_cookie,
            json!({"enabled": false}),
        )
        .await;
        assert_eq!(disabled.0, StatusCode::OK, "{path}: {}", disabled.2);
        let enabled = call(
            &expired,
            "PATCH",
            &path,
            &admin_cookie,
            json!({"enabled": true}),
        )
        .await;
        assert_eq!(enabled.1["X-Aster-Error-Number"], "51002", "{path}");
        let deleted = call(&expired, "DELETE", &path, &admin_cookie, Value::Null).await;
        assert_eq!(deleted.0, StatusCode::OK, "{path}: {}", deleted.2);
    }
    for path in [
        "/api/member/me",
        "/api/member/keys",
        "/api/member/models",
        "/api/member/usage-summary",
        "/api/member/usage-logs",
        "/api/member/ledger",
        "/api/member/quota-requests",
        "/api/member/vouchers",
    ] {
        let response = call(&expired, "GET", path, &member_cookie, Value::Null).await;
        assert_eq!(response.0, StatusCode::OK, "{path}: {}", response.2);
    }
    for path in [
        "/api/admin/users",
        "/api/admin/consumption-logs",
        "/api/admin/consumption-logs/members",
        "/api/admin/runners",
        "/api/admin/upstream-accounts",
        "/api/admin/models",
        "/api/admin/quota-requests",
        "/api/admin/vouchers",
        "/api/admin/vouchers/recipients",
    ] {
        let response = call(&expired, "GET", path, &admin_cookie, Value::Null).await;
        assert_eq!(response.0, StatusCode::OK, "{path}: {}", response.2);
    }
    let blocked = call(
        &expired,
        "POST",
        "/api/member/keys",
        &member_cookie,
        json!({"name": "after expiry"}),
    )
    .await;
    assert_eq!(blocked.1["X-Aster-Error-Number"], "51002");
    assert!(matches!(
        authorize_model_consumption(&expired, "member").await,
        Err(ControlError::Policy(PolicyError::Expired))
    ));
    let revoked = call(
        &expired,
        "POST",
        &format!("/api/member/keys/{key_id}/revoke"),
        &member_cookie,
        Value::Null,
    )
    .await;
    assert_eq!(revoked.0, StatusCode::OK, "{}", revoked.2);
    expired
        .update_member_status(&actor, &member.id, "disabled", "member.disable")
        .await
        .unwrap();
    assert_eq!(expired.verified_occupied_seats().await.unwrap(), 0);
    assert!(matches!(
        expired
            .update_member_status(&actor, &member.id, "active", "member.enable")
            .await,
        Err(ControlError::Policy(PolicyError::Expired))
    ));
    // A newer signed term restores usage without clearing resource or audit data.
    let mut renewed_claims = document.claims.clone();
    renewed_claims.license_id = "renewed_retention_test".to_owned();
    renewed_claims.issued_at = "2027-09-02T00:00:00.000Z".to_owned();
    renewed_claims.validity.expiry = v2::Expiry::Fixed {
        expires_at: "2028-09-01T00:00:00.000Z".to_owned(),
    };
    let renewed = v2::sign(
        renewed_claims,
        &ed25519_dalek::SigningKey::from_bytes(&[42; 32]),
    )
    .unwrap();
    let renewed = verify_product_license(&serde_json::to_vec(&renewed).unwrap(), &keys).unwrap();
    expired
        .license_state
        .as_ref()
        .unwrap()
        .accept_replacement(&renewed, datetime!(2027-09-02 0:00 UTC))
        .unwrap();
    expired.replace_license(renewed).unwrap();
    expired
        .update_member_status(&actor, &member.id, "active", "member.enable")
        .await
        .unwrap();
    let member_cookie = login_cookie(&expired, "member", "member-password-updated").await;
    let records = call(
        &expired,
        "GET",
        "/api/member/keys",
        &member_cookie,
        Value::Null,
    )
    .await;
    assert_eq!(records.0, StatusCode::OK, "{}", records.2);
    assert_eq!(records.2["items"][0]["id"], key_id);
    assert_eq!(records.2["items"][0]["status"], "revoked");
    let new_key = call(
        &expired,
        "POST",
        "/api/member/keys",
        &member_cookie,
        json!({"name": "after renewal"}),
    )
    .await;
    assert_eq!(new_key.0, StatusCode::CREATED, "{}", new_key.2);
    assert!(!expired.verified_audit_events().await.unwrap().is_empty());
    // Retention still requires authenticated, intact installation history.
    let unauthenticated = call(&expired, "GET", "/api/admin/users", "", Value::Null).await;
    assert_eq!(unauthenticated.0, StatusCode::UNAUTHORIZED);
    std::fs::write(history_path, b"{}").unwrap();
    let corrupted = call(
        &expired,
        "GET",
        "/api/admin/users",
        &admin_cookie,
        Value::Null,
    )
    .await;
    assert_eq!(corrupted.0, StatusCode::FORBIDDEN, "{}", corrupted.2);
}
