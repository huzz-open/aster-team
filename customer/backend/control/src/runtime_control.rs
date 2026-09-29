//! Private executor surface. Never merge this router into API/Admin/Member.
//! Admission status is not business readiness or proof of durable settlement.
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::{ControlError, ControlState, lifecycle::DrainSnapshot};
use aster_upgrade_core::ReleaseSlot;

#[derive(Clone)]
pub struct RuntimeControl(Arc<Inner>);

struct Inner {
    control: ControlState,
    installation_id: String,
    slot: ReleaseSlot,
    instance_id: String,
    service: Option<aster_upgrade_core::runtime::ServiceInvocation>,
    token_hash: [u8; 32],
    assets: Option<Arc<crate::web_assets::WebAssets>>,
    readiness: std::sync::Mutex<Option<ReadinessProof>>,
    checking: tokio::sync::Mutex<()>,
}

#[derive(Serialize)]
struct Snapshot<'a> {
    schema: &'static str,
    installation_id: &'a str,
    slot: ReleaseSlot,
    instance_id: &'a str,
    service: &'a Option<aster_upgrade_core::runtime::ServiceInvocation>,
    product_version: &'a str,
    lifecycle: DrainSnapshot,
    request_budget_ms: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AdmissionCommand {
    instance_id: String,
    expected_revision: u64,
    accepting: bool,
    readiness_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RetirementCommand {
    instance_id: String,
    expected_revision: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RunnerProbeCommand {
    instance_id: String,
    runner_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DependencyCommand {
    instance_id: String,
    models: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ModelInventoryCommand {
    instance_id: String,
    expected_revision: u64,
}

struct ReadinessProof {
    token: String,
    revision: u64,
    expires: std::time::Instant,
    runners: std::collections::BTreeMap<String, u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReadinessCommand {
    instance_id: String,
    expected_revision: u64,
    manifest_sha256: String,
    models: Vec<String>,
    runner_ids: Vec<String>,
}

impl RuntimeControl {
    pub fn new(
        control: ControlState,
        installation_id: String,
        slot: ReleaseSlot,
        token: &[u8; 32],
    ) -> Result<Self, ControlError> {
        // Hash the high-entropy bearer value rather than retaining it in state.
        let bearer = zeroize::Zeroizing::new(URL_SAFE_NO_PAD.encode(token));
        Ok(Self(Arc::new(Inner {
            control,
            installation_id,
            slot,
            instance_id: crate::random_identifier("instance")?,
            service: service_invocation(),
            token_hash: Sha256::digest(bearer.as_bytes()).into(),
            assets: None,
            readiness: std::sync::Mutex::new(None),
            checking: tokio::sync::Mutex::new(()),
        })))
    }

    pub fn with_web_assets(
        mut self,
        admin: std::path::PathBuf,
        member: std::path::PathBuf,
    ) -> Self {
        // Builder is only used before the instance is cloned into its servers.
        Arc::get_mut(&mut self.0)
            .expect("runtime builder used after cloning")
            .assets = Some(Arc::new(crate::web_assets::WebAssets::new(admin, member)));
        self
    }

    pub fn router(&self) -> Router {
        crate::entrypoints::runtime_router(self.clone())
            .layer(DefaultBodyLimit::max(8192))
            .layer(middleware::from_fn_with_state(self.clone(), protect))
    }

    fn snapshot(&self) -> Response {
        Json(Snapshot {
            schema: "aster.control-runtime.v1",
            installation_id: &self.0.installation_id,
            slot: self.0.slot,
            instance_id: &self.0.instance_id,
            service: &self.0.service,
            product_version: &self.0.control.product_version,
            lifecycle: self.0.control.lifecycle.snapshot(),
            request_budget_ms: self.0.control.lifecycle.request_budget_ms(),
        })
        .into_response()
    }
}

fn service_invocation() -> Option<aster_upgrade_core::runtime::ServiceInvocation> {
    let service = aster_upgrade_core::runtime::ServiceInvocation {
        process_id: std::process::id(),
        invocation_id: std::env::var("INVOCATION_ID").ok()?,
    };
    service.valid().then_some(service)
}

async fn protect(State(state): State<RuntimeControl>, request: Request, next: Next) -> Response {
    let mut credentials = request.headers().get_all(header::AUTHORIZATION).iter();
    let authorized = credentials
        .next()
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|token| token.len() == 43)
        .is_some_and(|token| {
            let hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
            hash == state.0.token_hash
        })
        && credentials.next().is_none();
    let timeout = if request.uri().path() == "/v1/readiness" {
        20
    } else {
        5
    };
    let mut response = if authorized {
        // Includes request-body parsing. Slow local callers cannot leave an
        // unbounded command handler alive while the process drains.
        tokio::time::timeout(std::time::Duration::from_secs(timeout), next.run(request))
            .await
            .unwrap_or_else(|_| StatusCode::REQUEST_TIMEOUT.into_response())
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub(super) async fn status(State(state): State<RuntimeControl>) -> Response {
    state.snapshot()
}

pub(super) async fn retire(
    State(state): State<RuntimeControl>,
    Json(command): Json<RetirementCommand>,
) -> Response {
    if command.instance_id != state.0.instance_id {
        return StatusCode::CONFLICT.into_response();
    }
    let mut proof = state.0.readiness.lock().expect("readiness proof poisoned");
    if !state
        .0
        .control
        .lifecycle
        .retire_drained(command.expected_revision)
    {
        return StatusCode::CONFLICT.into_response();
    }
    proof.take();
    // This acknowledges irreversible shutdown admission, not process exit.
    // The serving task observes the lifecycle and uses its graceful stop path.
    state.snapshot()
}

pub(super) async fn admission(
    State(state): State<RuntimeControl>,
    Json(command): Json<AdmissionCommand>,
) -> Response {
    if command.instance_id != state.0.instance_id {
        return StatusCode::CONFLICT.into_response();
    }
    if command.accepting {
        let connections = state.0.control.runner_hub.inner.connections.lock().await;
        let mut proof = state.0.readiness.lock().expect("readiness proof poisoned");
        let valid = proof.as_ref().is_some_and(|proof| {
            command.readiness_token.as_deref() == Some(proof.token.as_str())
                && proof.revision == command.expected_revision
                && std::time::Instant::now() < proof.expires
                && proof.runners.iter().all(|(id, generation)| {
                    connections
                        .get(id)
                        .is_some_and(|connection| connection.generation == *generation)
                })
        });
        if !valid {
            return StatusCode::CONFLICT.into_response();
        }
        proof.take();
        if !state
            .0
            .control
            .lifecycle
            .set_admission(command.expected_revision, true)
        {
            return StatusCode::CONFLICT.into_response();
        }
    } else {
        if !state
            .0
            .control
            .lifecycle
            .set_admission(command.expected_revision, false)
        {
            return StatusCode::CONFLICT.into_response();
        }
        state
            .0
            .readiness
            .lock()
            .expect("readiness proof poisoned")
            .take();
    }
    state.snapshot()
}

pub(super) async fn runner_probe(
    State(state): State<RuntimeControl>,
    Json(command): Json<RunnerProbeCommand>,
) -> Response {
    let before = state.0.control.lifecycle.snapshot();
    if command.instance_id != state.0.instance_id || before.stopping {
        return StatusCode::CONFLICT.into_response();
    }
    let passed = state
        .0
        .control
        .probe_runner(&command.runner_id)
        .await
        .is_ok();
    let after = state.0.control.lifecycle.snapshot();
    if after.stopping || after.revision != before.revision {
        return StatusCode::CONFLICT.into_response();
    }
    (
        if passed {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(serde_json::json!({
            "schema": "aster.runner-probe.v1", "instance_id": state.0.instance_id,
            "runner_id": command.runner_id, "passed": passed,
        })),
    )
        .into_response()
}

pub(super) async fn dependencies(
    State(state): State<RuntimeControl>,
    Json(command): Json<DependencyCommand>,
) -> Response {
    let before = state.0.control.lifecycle.snapshot();
    if command.instance_id != state.0.instance_id || before.stopping {
        return StatusCode::CONFLICT.into_response();
    }
    if command.models.is_empty()
        || command.models.len() > 32
        || command
            .models
            .iter()
            .any(|model| model.is_empty() || model.len() > 128)
        || command
            .models
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != command.models.len()
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let report = state.0.control.check_dependencies(&command.models).await;
    let after = state.0.control.lifecycle.snapshot();
    if after.stopping || before.revision != after.revision {
        return StatusCode::CONFLICT.into_response();
    }
    (
        if report.passed() {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(serde_json::json!({
            "schema": "aster.control-dependencies.v1", "instance_id": state.0.instance_id,
            "passed": report.passed(), "checks": report,
        })),
    )
        .into_response()
}

pub(super) async fn readiness(
    State(state): State<RuntimeControl>,
    Json(command): Json<ReadinessCommand>,
) -> Response {
    let before = state.0.control.lifecycle.snapshot();
    if command.instance_id != state.0.instance_id
        || command.expected_revision != before.revision
        || before.stopping
    {
        return StatusCode::CONFLICT.into_response();
    }
    if command.manifest_sha256.len() != 64
        || !command
            .manifest_sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || !valid_names(&command.models, 32)
        || !valid_names(&command.runner_ids, 8)
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Ok(_checking) = state.0.checking.try_lock() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    state
        .0
        .readiness
        .lock()
        .expect("readiness proof poisoned")
        .take();
    let configured_models = state.0.control.configured_readiness_models().await.ok();
    let mut requested_models = command.models.clone();
    requested_models.sort();
    let complete_coverage = configured_models.as_ref() == Some(&requested_models);
    if configured_models.is_some() && !complete_coverage {
        return StatusCode::CONFLICT.into_response();
    }
    let generations = {
        let connections = state.0.control.runner_hub.inner.connections.lock().await;
        command
            .runner_ids
            .iter()
            .filter_map(|id| connections.get(id).map(|c| (id.clone(), c.generation)))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let assets = async {
        match &state.0.assets {
            Some(assets) => {
                assets
                    .verify(
                        state.0.control.product_version.to_string(),
                        command.manifest_sha256.clone(),
                    )
                    .await
            }
            None => false,
        }
    };
    let runners = async {
        let results = futures_util::future::join_all(
            command
                .runner_ids
                .iter()
                .map(|id| state.0.control.probe_runner(id)),
        )
        .await;
        results.iter().all(Result::is_ok)
    };
    let (dependencies, runners, assets) = tokio::join!(
        state.0.control.check_dependencies(&command.models),
        runners,
        assets
    );
    if state
        .0
        .control
        .configured_readiness_models()
        .await
        .ok()
        .as_ref()
        != configured_models.as_ref()
    {
        return StatusCode::CONFLICT.into_response();
    }
    let connections = state.0.control.runner_hub.inner.connections.lock().await;
    let after = state.0.control.lifecycle.snapshot();
    if after.stopping
        || before.revision != after.revision
        || !generations.iter().all(|(id, generation)| {
            connections
                .get(id)
                .is_some_and(|c| c.generation == *generation)
        })
    {
        return StatusCode::CONFLICT.into_response();
    }
    let platform = cfg!(all(target_os = "linux", target_arch = "x86_64"));
    let ready = complete_coverage
        && platform
        && dependencies.passed()
        && runners
        && assets
        && generations.len() == command.runner_ids.len();
    let token = if ready {
        let Ok(token) = crate::random_identifier("ready") else {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        };
        *state.0.readiness.lock().expect("readiness proof poisoned") = Some(ReadinessProof {
            token: token.clone(),
            revision: before.revision,
            expires: std::time::Instant::now() + std::time::Duration::from_secs(10),
            runners: generations,
        });
        Some(token)
    } else {
        None
    };
    (if ready { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE }, Json(serde_json::json!({
        "schema": "aster.control-readiness.v1", "instance_id": state.0.instance_id, "slot": state.0.slot,
        "revision": before.revision, "manifest_sha256": command.manifest_sha256, "ready": ready,
        "checks": { "platform": platform, "dependencies": dependencies, "runners": runners, "assets": assets },
        "readiness_token": token, "valid_for_seconds": if ready { 10 } else { 0 },
    }))).into_response()
}

pub(super) async fn readiness_models(
    State(state): State<RuntimeControl>,
    Json(command): Json<ModelInventoryCommand>,
) -> Response {
    let before = state.0.control.lifecycle.snapshot();
    if command.instance_id != state.0.instance_id
        || command.expected_revision != before.revision
        || before.stopping
    {
        return StatusCode::CONFLICT.into_response();
    }
    let Ok(_checking) = state.0.checking.try_lock() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    // Discovery starts a new readiness evaluation. A failure here must not
    // leave a permit from an earlier evaluation available to open admission.
    state
        .0
        .readiness
        .lock()
        .expect("readiness proof poisoned")
        .take();
    let Ok(models) = state.0.control.configured_readiness_models().await else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let after = state.0.control.lifecycle.snapshot();
    if after.stopping || after.revision != before.revision {
        return StatusCode::CONFLICT.into_response();
    }
    Json(aster_upgrade_core::runtime::ReadinessModelInventory {
        schema: "aster.readiness-models.v1".into(),
        installation_id: state.0.installation_id.clone(),
        instance_id: state.0.instance_id.clone(),
        slot: state.0.slot,
        revision: before.revision,
        models,
    })
    .into_response()
}

fn valid_names(names: &[String], maximum: usize) -> bool {
    !names.is_empty()
        && names.len() <= maximum
        && names
            .iter()
            .all(|name| !name.is_empty() && name.len() <= 128)
        && names
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == names.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt as _;

    const TOKEN: [u8; 32] = [29; 32];
    fn runtime(control: ControlState) -> RuntimeControl {
        RuntimeControl::new(
            control,
            "installation-test".into(),
            ReleaseSlot::Blue,
            &TOKEN,
        )
        .unwrap()
    }
    async fn command(
        runtime: &RuntimeControl,
        instance: &str,
        revision: u64,
        accepting: bool,
    ) -> Response {
        runtime.router().oneshot(Request::builder().method("POST").uri("/v1/admission")
            .header(header::AUTHORIZATION, format!("Bearer {}", URL_SAFE_NO_PAD.encode(TOKEN)))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::json!({"instance_id": instance, "expected_revision": revision, "accepting": accepting, "readiness_token": if accepting { Some("test-ready") } else { None }}).to_string())).unwrap())
            .await.unwrap()
    }

    #[tokio::test]
    async fn every_registered_runtime_entry_requires_the_installation_token() {
        let runtime = runtime(ControlState::new("test", None));
        assert_eq!(crate::entrypoints::RUNTIME_ENTRIES.len(), 7);
        for entry in crate::entrypoints::RUNTIME_ENTRIES {
            let response = runtime
                .router()
                .oneshot(
                    Request::builder()
                        .method(entry.method.to_ascii_uppercase().as_str())
                        .uri(entry.path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{}",
                entry.operation
            );
            assert!(
                !crate::entrypoints::HTTP_ENTRIES
                    .iter()
                    .any(|public| public.path == entry.path)
            );
        }
    }

    #[tokio::test]
    async fn drain_preserves_response_ownership_and_rejects_stale_resume() {
        let control = ControlState::new("test", None);
        let runtime = runtime(control.clone());
        let app = crate::track_requests(
            Router::new().route("/work", get(|| async { "accepted" })),
            &control,
        );
        let response = app
            .clone()
            .oneshot(Request::builder().uri("/work").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let instance = &runtime.0.instance_id;
        let drained = command(&runtime, instance, 0, false).await;
        assert_eq!(drained.status(), StatusCode::OK);
        let snapshot: serde_json::Value =
            serde_json::from_slice(&to_bytes(drained.into_body(), 4096).await.unwrap()).unwrap();
        assert_eq!(snapshot["lifecycle"]["in_flight"], 1);
        assert_eq!(snapshot["lifecycle"]["revision"], 1);
        assert_eq!(
            app.oneshot(Request::builder().uri("/work").body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            to_bytes(response.into_body(), 100).await.unwrap(),
            "accepted"
        );
        control.lifecycle.wait_drained().await;
        assert_eq!(
            command(&runtime, instance, 0, true).await.status(),
            StatusCode::CONFLICT
        );
        assert!(!control.lifecycle.snapshot().accepting);
        *runtime.0.readiness.lock().unwrap() = Some(ReadinessProof {
            token: "test-ready".into(),
            revision: 1,
            expires: std::time::Instant::now() + std::time::Duration::from_secs(10),
            runners: Default::default(),
        });
        assert_eq!(
            command(&runtime, instance, 1, true).await.status(),
            StatusCode::OK
        );
        control.lifecycle.begin_shutdown();
        let snapshot = control.lifecycle.snapshot();
        assert!(snapshot.stopping);
        assert_eq!(
            command(&runtime, instance, snapshot.revision, true)
                .await
                .status(),
            StatusCode::CONFLICT
        );
        assert!(!control.lifecycle.snapshot().accepting);
    }

    #[tokio::test]
    async fn process_identity_prevents_old_commands_from_touching_restarted_slot() {
        let previous = runtime(ControlState::new("old", None));
        let current = runtime(ControlState::new("new", None));
        assert_ne!(previous.0.instance_id, current.0.instance_id);
        assert_eq!(
            command(&current, &previous.0.instance_id, 0, false)
                .await
                .status(),
            StatusCode::CONFLICT
        );
        assert_eq!(current.0.control.lifecycle.snapshot().revision, 0);
        assert!(current.0.control.lifecycle.snapshot().accepting);
    }

    async fn retire_command(
        runtime: &RuntimeControl,
        instance: &str,
        revision: u64,
        authorized: bool,
    ) -> Response {
        let mut request = Request::builder()
            .method("POST")
            .uri("/v1/retire")
            .header(header::CONTENT_TYPE, "application/json");
        if authorized {
            request = request.header(
                header::AUTHORIZATION,
                format!("Bearer {}", URL_SAFE_NO_PAD.encode(TOKEN)),
            );
        }
        runtime
            .router()
            .oneshot(
                request
                    .body(Body::from(
                        serde_json::json!({
                            "instance_id": instance, "expected_revision": revision,
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn retirement_cannot_drop_owned_response_or_stop_replacement_process() {
        let control = ControlState::new("test", None);
        let runtime = runtime(control.clone());
        let instance = &runtime.0.instance_id;
        let response = crate::track_requests(
            Router::new().route("/work", get(|| async { "completed" })),
            &control,
        )
        .oneshot(Request::builder().uri("/work").body(Body::empty()).unwrap())
        .await
        .unwrap();
        assert_eq!(
            command(&runtime, instance, 0, false).await.status(),
            StatusCode::OK
        );
        assert_eq!(
            retire_command(&runtime, instance, 1, true).await.status(),
            StatusCode::CONFLICT
        );
        assert!(!control.lifecycle.snapshot().stopping);
        assert_eq!(
            to_bytes(response.into_body(), 100).await.unwrap(),
            "completed"
        );
        assert_eq!(
            retire_command(&runtime, instance, 1, false).await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            retire_command(&runtime, "old-instance", 1, true)
                .await
                .status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            retire_command(&runtime, instance, 0, true).await.status(),
            StatusCode::CONFLICT
        );
        let reply = retire_command(&runtime, instance, 1, true).await;
        assert_eq!(reply.status(), StatusCode::OK);
        let snapshot: serde_json::Value =
            serde_json::from_slice(&to_bytes(reply.into_body(), 4096).await.unwrap()).unwrap();
        assert_eq!(snapshot["lifecycle"]["stopping"], true);
        assert_eq!(snapshot["lifecycle"]["revision"], 2);
        assert_eq!(snapshot["lifecycle"]["in_flight"], 0);
        assert_eq!(
            retire_command(&runtime, instance, 1, true).await.status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            command(&runtime, instance, 2, true).await.status(),
            StatusCode::CONFLICT
        );
        assert!(control.lifecycle.snapshot().stopping);
    }

    #[tokio::test]
    async fn private_http_requires_token_and_remains_queryable_during_drain() {
        let control = ControlState::new("test", None);
        let runtime = runtime(control.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = runtime.router();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap();
        let url = format!("http://{address}/v1/status");
        let wrong_token = URL_SAFE_NO_PAD.encode([28; 32]);
        for token in [None, Some("invalid"), Some(wrong_token.as_str())] {
            let request = client.get(&url);
            let request = if let Some(token) = token {
                request.bearer_auth(token)
            } else {
                request
            };
            let response = request.send().await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
        // Check the live public router before draining; a drain-generated 503
        // would not prove the command endpoint was absent.
        for (method, path) in [
            ("GET", "/v1/status"),
            ("POST", "/v1/admission"),
            ("POST", "/v1/runner-probe"),
            ("POST", "/v1/dependencies"),
            ("POST", "/v1/readiness"),
            ("POST", "/v1/readiness-models"),
        ] {
            assert_eq!(
                crate::router(control.clone())
                    .oneshot(
                        Request::builder()
                            .method(method)
                            .uri(path)
                            .body(Body::empty())
                            .unwrap()
                    )
                    .await
                    .unwrap()
                    .status(),
                StatusCode::NOT_FOUND
            );
        }
        control.lifecycle.begin_drain();
        let response = client
            .get(&url)
            .bearer_auth(URL_SAFE_NO_PAD.encode(TOKEN))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let snapshot: serde_json::Value = response.json().await.unwrap();
        assert_eq!(
            snapshot["request_budget_ms"],
            control.lifecycle.request_budget_ms()
        );
        assert!(snapshot["request_budget_ms"].as_u64().unwrap() > 0);
        assert_eq!(snapshot["installation_id"], "installation-test");
        assert_eq!(snapshot["slot"], "blue");
        assert_eq!(snapshot["lifecycle"]["in_flight"], 0);
        assert_eq!(snapshot["lifecycle"]["accepting"], false);
        stop.send(()).unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn malformed_commands_and_duplicate_authorization_do_not_mutate_state() {
        let runtime = runtime(ControlState::new("test", None));
        let token = format!("Bearer {}", URL_SAFE_NO_PAD.encode(TOKEN));
        let response = runtime
            .router()
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .header(header::AUTHORIZATION, &token)
                    .header(header::AUTHORIZATION, &token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let response = runtime.router().oneshot(Request::builder().method("POST").uri("/v1/admission")
            .header(header::AUTHORIZATION, &token).header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::json!({"instance_id": runtime.0.instance_id, "expected_revision": 0, "accepting": false, "extra": true}).to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(runtime.0.control.lifecycle.snapshot().revision, 0);
    }
    #[tokio::test]
    async fn runner_probe_endpoint_is_instance_bound_and_never_fakes_readiness() {
        let control = ControlState::new("test", None);
        let runtime = runtime(control.clone());
        for (instance, expected) in [
            ("previous-instance", StatusCode::CONFLICT),
            (
                runtime.0.instance_id.as_str(),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
        ] {
            let response = runtime.router().oneshot(Request::builder().method("POST").uri("/v1/runner-probe")
                .header(header::AUTHORIZATION, format!("Bearer {}", URL_SAFE_NO_PAD.encode(TOKEN)))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::json!({"instance_id": instance, "runner_id": "runner-test"}).to_string())).unwrap()).await.unwrap();
            assert_eq!(response.status(), expected);
            if expected == StatusCode::SERVICE_UNAVAILABLE {
                let result: serde_json::Value =
                    serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap())
                        .unwrap();
                assert_eq!(result["passed"], false);
                assert!(result.get("ready").is_none());
            }
        }
        assert_eq!(control.lifecycle.snapshot().in_flight, 0);
        assert_eq!(control.lifecycle.snapshot().revision, 0);
    }
    #[tokio::test]
    async fn dependencies_require_expected_models_and_do_not_fabricate_success() {
        let runtime = runtime(ControlState::new("test", None));
        for (models, expected) in [
            (vec![], StatusCode::BAD_REQUEST),
            (vec!["model", "model"], StatusCode::BAD_REQUEST),
            (vec!["model"], StatusCode::SERVICE_UNAVAILABLE),
        ] {
            let response = runtime.router().oneshot(Request::builder().method("POST").uri("/v1/dependencies")
                .header(header::AUTHORIZATION, format!("Bearer {}", URL_SAFE_NO_PAD.encode(TOKEN)))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::json!({"instance_id": runtime.0.instance_id, "models": models}).to_string())).unwrap()).await.unwrap();
            assert_eq!(response.status(), expected);
            if expected == StatusCode::SERVICE_UNAVAILABLE {
                let report: serde_json::Value =
                    serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap())
                        .unwrap();
                assert_eq!(report["passed"], false);
                assert_eq!(report["checks"]["database_read_write"], false);
                assert!(report.get("ready").is_none());
            }
        }
        assert_eq!(runtime.0.control.lifecycle.snapshot().in_flight, 0);
    }
    #[tokio::test]
    async fn readiness_proofs_expire_bind_runner_connections_and_are_single_use() {
        let control = ControlState::new("test", None);
        control.lifecycle.begin_drain();
        let runtime = runtime(control.clone());
        let instance = &runtime.0.instance_id;
        let (generation, _wire) = control.runner_hub.register("runner-test").await;
        assert_eq!(
            command(&runtime, instance, 1, true).await.status(),
            StatusCode::CONFLICT
        );
        let install = |generation, expires| {
            *runtime.0.readiness.lock().unwrap() = Some(ReadinessProof {
                token: "test-ready".into(),
                revision: 1,
                expires,
                runners: std::collections::BTreeMap::from([("runner-test".into(), generation)]),
            });
        };
        install(generation, std::time::Instant::now());
        assert_eq!(
            command(&runtime, instance, 1, true).await.status(),
            StatusCode::CONFLICT
        );
        install(
            generation,
            std::time::Instant::now() + std::time::Duration::from_secs(10),
        );
        let (replacement, _new_wire) = control.runner_hub.register("runner-test").await;
        assert_eq!(
            command(&runtime, instance, 1, true).await.status(),
            StatusCode::CONFLICT
        );
        assert!(!control.lifecycle.snapshot().accepting);
        install(
            replacement,
            std::time::Instant::now() + std::time::Duration::from_secs(10),
        );
        assert_eq!(
            command(&runtime, instance, 1, true).await.status(),
            StatusCode::OK
        );
        assert!(control.lifecycle.snapshot().accepting);
        assert!(runtime.0.readiness.lock().unwrap().is_none());
        assert_eq!(
            command(&runtime, instance, 2, true).await.status(),
            StatusCode::CONFLICT
        );
    }

    #[tokio::test]
    async fn failed_readiness_clears_previous_proof_and_cannot_open_admission() {
        let control = ControlState::new("test", None);
        control.lifecycle.begin_drain();
        let runtime = runtime(control.clone());
        *runtime.0.readiness.lock().unwrap() = Some(ReadinessProof {
            token: "test-ready".into(),
            revision: 1,
            expires: std::time::Instant::now() + std::time::Duration::from_secs(10),
            runners: Default::default(),
        });
        let response = runtime.router().oneshot(Request::builder().method("POST").uri("/v1/readiness")
            .header(header::AUTHORIZATION, format!("Bearer {}", URL_SAFE_NO_PAD.encode(TOKEN)))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::json!({"instance_id": runtime.0.instance_id, "expected_revision": 1, "manifest_sha256": "0".repeat(64), "models": ["gpt-test"], "runner_ids": ["runner-test"]}).to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let report: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
        assert_eq!(report["ready"], false);
        assert!(report["readiness_token"].is_null());
        assert!(runtime.0.readiness.lock().unwrap().is_none());
        assert_eq!(
            command(&runtime, &runtime.0.instance_id, 1, true)
                .await
                .status(),
            StatusCode::CONFLICT
        );
        assert!(!control.lifecycle.snapshot().accepting);
    }

    #[tokio::test]
    async fn failed_model_discovery_invalidates_an_earlier_ready_permit() {
        let control = ControlState::new("test", None);
        control.lifecycle.begin_drain();
        let runtime = runtime(control.clone());
        *runtime.0.readiness.lock().unwrap() = Some(ReadinessProof {
            token: "test-ready".into(),
            revision: 1,
            expires: std::time::Instant::now() + std::time::Duration::from_secs(10),
            runners: Default::default(),
        });
        let response = runtime
            .router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/readiness-models")
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", URL_SAFE_NO_PAD.encode(TOKEN)),
                    )
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({"instance_id":runtime.0.instance_id,
                "expected_revision":1})
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(runtime.0.readiness.lock().unwrap().is_none());
        assert_eq!(
            command(&runtime, &runtime.0.instance_id, 1, true)
                .await
                .status(),
            StatusCode::CONFLICT
        );
        assert!(!control.lifecycle.snapshot().accepting);
    }
}
