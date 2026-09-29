#![cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]

use super::*;
use tempfile::{TempDir, tempdir};
use time::macros::datetime;

const MODEL: &str = "gpt-request-identity-test";
const CLIENT_ID: &str = "01a06bea-7368-77b2-9503-677bc5b68235";

async fn provider_state() -> (TempDir, ControlState) {
    let directory = tempdir().expect("temporary database directory");
    let mut store = aster_storage::SqlCipherStore::initialize(
        &directory.path().join("customer.db"),
        &[101_u8; 32],
    )
    .expect("initialize provider test storage");
    store
        .insert_upstream_account_unchecked(
            "request-identity-account",
            "openai",
            "request-identity-subject",
            "request-identity@example.com",
            "2026-09-06T00:00:00.000Z",
        )
        .expect("insert OpenAI account");
    store
        .replace_account_models(
            "request-identity-account",
            &[DiscoveredModel {
                id: "request-identity-model".to_owned(),
                public_name: MODEL.to_owned(),
                display_name: "Request identity test".to_owned(),
                upstream_name: MODEL.to_owned(),
            }],
            "2026-09-06T00:00:00.000Z",
        )
        .expect("create routable model");
    let state = ControlState::new("test", None)
        .with_storage(ControlStorage::SqlCipher(Arc::new(StdMutex::new(store))))
        .with_credential_vault(
            CredentialVault::new(&[102_u8; 32], "request-identity-installation")
                .expect("credential vault"),
        )
        .with_now(datetime!(2026-09-06 0:00 UTC));
    let credential = serde_json::to_vec(&OpenAiCredentialPayload {
        schema: OPENAI_CREDENTIAL_SCHEMA.to_owned(),
        provider: "openai".to_owned(),
        access_token: "request-identity-access-token".to_owned(),
        refresh_token: "request-identity-refresh-token".to_owned(),
        id_token: String::new(),
        expires_at: "2026-09-06T01:00:00.000Z".to_owned(),
        account_id: "request-identity-subject".to_owned(),
        email: "request-identity@example.com".to_owned(),
        plan: "team".to_owned(),
    })
    .expect("encode OpenAI credential");
    state
        .create_credential_instance(
            "request-identity-account",
            b"request-identity-refresh-token",
            &credential,
            "2026-09-06T01:00:00.000Z",
        )
        .await
        .expect("store encrypted OpenAI credential");
    (directory, state)
}

async fn prepared_client_id(state: &ControlState, ids: &GatewayRequestIds, stream: bool) -> String {
    let operation = providers::CanonicalOperation::Text(providers::types::CanonicalTextOperation {
        schema_version: providers::CANONICAL_SCHEMA_VERSION,
        public_model: MODEL.to_owned(),
        stream,
        request: json!({"model": MODEL, "input": "Reply with OK.", "stream": stream}),
    });
    let (_, _, request) =
        prepare_provider_request(state, MODEL, ids, &operation, &HashSet::new(), None)
            .await
            .expect("prepare real OpenAI provider request");
    assert_eq!(request.url, OPENAI_CODEX_RESPONSES_ENDPOINT);
    let tracking_headers = request
        .headers
        .iter()
        .filter(|header| header.name.eq_ignore_ascii_case("x-client-request-id"))
        .collect::<Vec<_>>();
    assert_eq!(tracking_headers.len(), 1);
    tracking_headers[0].value.clone()
}

#[tokio::test]
async fn repeated_client_tracking_id_reaches_openai_with_independent_execution_ids() {
    let (_directory, state) = provider_state().await;
    let mut headers = HeaderMap::new();
    headers.insert("x-client-request-id", HeaderValue::from_static(CLIENT_ID));
    let first = gateway_request_id(&headers).expect("first request identity");
    let second = gateway_request_id(&headers).expect("second request identity");
    assert_ne!(first.execution_id, second.execution_id);
    assert_ne!(first.execution_id, CLIENT_ID);
    assert_ne!(second.execution_id, CLIENT_ID);
    for ids in [&first, &second] {
        for stream in [false, true] {
            assert_eq!(prepared_client_id(&state, ids, stream).await, CLIENT_ID);
        }
    }
}

#[tokio::test]
async fn client_tracking_header_takes_priority_over_proxy_tracking_header_at_openai() {
    let (_directory, state) = provider_state().await;
    let mut headers = HeaderMap::new();
    headers.insert("x-client-request-id", HeaderValue::from_static(CLIENT_ID));
    headers.insert(
        "x-request-id",
        HeaderValue::from_static("proxy-request-12345678"),
    );
    let ids = gateway_request_id(&headers).expect("request identity");
    assert_eq!(prepared_client_id(&state, &ids, true).await, CLIENT_ID);
}

#[tokio::test]
async fn openai_tracking_header_uses_safe_proxy_or_execution_id_fallback() {
    let (_directory, state) = provider_state().await;
    for (client, proxy, expected_client) in [
        (None, None, None),
        (
            None,
            Some("proxy-request-12345678"),
            Some("proxy-request-12345678"),
        ),
        (Some("unsafe/id"), None, None),
        (Some("short"), Some("unsafe proxy id"), None),
        (
            Some("unsafe/id"),
            Some("proxy-request-12345678"),
            Some("proxy-request-12345678"),
        ),
        (Some(CLIENT_ID), Some("unsafe proxy id"), Some(CLIENT_ID)),
    ] {
        let mut headers = HeaderMap::new();
        if let Some(value) = client {
            headers.insert("x-client-request-id", HeaderValue::from_static(value));
        }
        if let Some(value) = proxy {
            headers.insert("x-request-id", HeaderValue::from_static(value));
        }
        let ids = gateway_request_id(&headers).expect("request identity");
        let expected = expected_client.unwrap_or(ids.execution_id.as_str());
        assert_eq!(prepared_client_id(&state, &ids, false).await, expected);
    }

    for invalid in [
        HeaderValue::from_str(&"a".repeat(161)).expect("overlong header value"),
        HeaderValue::from_bytes(b"request-\xff-1234").expect("non-ASCII header value"),
    ] {
        let mut headers = HeaderMap::new();
        headers.insert("x-client-request-id", invalid);
        let ids = gateway_request_id(&headers).expect("request identity");
        assert_eq!(
            prepared_client_id(&state, &ids, true).await,
            ids.execution_id
        );
    }
}
