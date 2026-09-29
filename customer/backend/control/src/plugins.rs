//! Control-owned signed plugin lifecycle. Lua never receives the installation
//! paths, publisher keys, upgrade state, or persistence handle.
use std::{
    collections::{BTreeMap, HashSet},
    convert::Infallible,
    fs,
    sync::{Arc, Mutex},
};

use aster_credential_vault::CredentialContext;
use aster_credential_vault::EncryptedCredentialMaterial;
use aster_install_layout::InstallLayout;
use aster_plugin_core::{
    AdaptationPlan, Bundle, CandidateOutcome, CanonicalOperation, CompatibilityMode,
    ExecutionLimits, HttpIntent, LuaRuntime, PluginDescriptor, PluginResult, PluginSlot,
    TrustedPublisher, validate_http_intent,
};
use aster_runner_protocol::{UpstreamHttpRequest, UpstreamRequestHeader};
use aster_storage::{
    AuditEventRecord, ConnectionCredential, ConnectionModelOrigin, ConnectionModelSpec,
    ConnectionModelSync, ConnectionModelSyncOutcome, ConnectionMutationOutcome, ConnectionRecord,
    ConnectionRoute, IdentityRecord, ModelRecord, MutationWithAuditOutcome,
};
use axum::{
    body::Body,
    http::{HeaderValue, StatusCode},
    response::IntoResponse,
};
use axum::{body::Bytes, http::HeaderMap, response::Response};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use zeroize::Zeroizing;

use crate::model_execution::{PluginImageAttempt, PluginImageProgress, PluginModelAttempt};
use crate::{
    ControlError, ControlState, ControlStorage, GatewayRequestIds,
    MAX_BUFFERED_UPSTREAM_RESPONSE_BYTES, ModelConsumer, ModelUsage, QuotaReservation,
    RUNNER_EXECUTE_TASK_TIMEOUT, RunnerTaskAuthorization, RunnerTaskBinding, StartedRunnerResponse,
    compiled_keys, decode_base64url, encode_base64url, failed_model_request, format_database_time,
    gateway_request_id, json_no_store, lifecycle, maintenance_atomic_write, random_identifier,
    take_sse_frame, with_aster_request_id,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginImageResult {
    kind: String,
    value: String,
    revised_prompt: Option<String>,
}

impl ControlStorage {
    pub async fn connection_model_origin(
        &self,
        model_id: &str,
    ) -> Result<Option<ConnectionModelOrigin>, ControlError> {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(store) => store
                .connection_model_origin(model_id)
                .await
                .map_err(ControlError::storage),
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            Self::SqlCipher(store) => {
                let store = Arc::clone(store);
                let model_id = model_id.to_owned();
                lifecycle::spawn_blocking(move || {
                    store
                        .lock()
                        .map_err(|_| ControlError::DataIntegrityInvalid)?
                        .connection_model_origin(&model_id)
                        .map_err(ControlError::storage)
                })
                .await
                .map_err(ControlError::storage)?
            }
            #[cfg(test)]
            Self::Fixed(_) | Self::Test { .. } => Ok(None),
        }
    }

    pub async fn insert_api_key_connection(
        &self,
        connection: ConnectionRecord,
        credential: ConnectionCredential,
        models: Vec<ConnectionModelSpec>,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: AuditEventRecord,
    ) -> Result<MutationWithAuditOutcome<()>, ControlError> {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(store) => store
                .insert_api_key_connection(
                    &connection,
                    &credential,
                    &models,
                    expected_audit_sequence,
                    expected_audit_hmac,
                    &audit_event,
                )
                .await
                .map_err(ControlError::storage),
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            Self::SqlCipher(store) => {
                let store = Arc::clone(store);
                let expected_audit_hmac = expected_audit_hmac.to_owned();
                lifecycle::spawn_blocking(move || {
                    store
                        .lock()
                        .map_err(|_| ControlError::DataIntegrityInvalid)?
                        .insert_api_key_connection(
                            &connection,
                            &credential,
                            &models,
                            expected_audit_sequence,
                            &expected_audit_hmac,
                            &audit_event,
                        )
                        .map_err(ControlError::storage)
                })
                .await
                .map_err(ControlError::storage)?
            }
            #[cfg(test)]
            Self::Fixed(_) | Self::Test { .. } => Err(ControlError::DataIntegrityInvalid),
        }
    }

    pub async fn list_api_key_connections(&self) -> Result<Vec<ConnectionRecord>, ControlError> {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(store) => store
                .list_api_key_connections()
                .await
                .map_err(ControlError::storage),
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            Self::SqlCipher(store) => {
                let store = Arc::clone(store);
                lifecycle::spawn_blocking(move || {
                    store
                        .lock()
                        .map_err(|_| ControlError::DataIntegrityInvalid)?
                        .list_api_key_connections()
                        .map_err(ControlError::storage)
                })
                .await
                .map_err(ControlError::storage)?
            }
            #[cfg(test)]
            Self::Fixed(_) | Self::Test { .. } => Ok(Vec::new()),
        }
    }

    pub async fn api_key_connection(
        &self,
        id: &str,
    ) -> Result<Option<(ConnectionRecord, ConnectionCredential)>, ControlError> {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(store) => store
                .api_key_connection(id)
                .await
                .map_err(ControlError::storage),
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            Self::SqlCipher(store) => {
                let store = Arc::clone(store);
                let id = id.to_owned();
                lifecycle::spawn_blocking(move || {
                    store
                        .lock()
                        .map_err(|_| ControlError::DataIntegrityInvalid)?
                        .api_key_connection(&id)
                        .map_err(ControlError::storage)
                })
                .await
                .map_err(ControlError::storage)?
            }
            #[cfg(test)]
            Self::Fixed(_) | Self::Test { .. } => Ok(None),
        }
    }

    pub async fn update_api_key_connection(
        &self,
        connection: ConnectionRecord,
        credential: Option<ConnectionCredential>,
        expected_revision: i64,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: AuditEventRecord,
    ) -> Result<MutationWithAuditOutcome<ConnectionMutationOutcome>, ControlError> {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(store) => store
                .update_api_key_connection(
                    &connection,
                    credential.as_ref(),
                    expected_revision,
                    expected_audit_sequence,
                    expected_audit_hmac,
                    &audit_event,
                )
                .await
                .map_err(ControlError::storage),
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            Self::SqlCipher(store) => {
                let store = Arc::clone(store);
                let expected_audit_hmac = expected_audit_hmac.to_owned();
                lifecycle::spawn_blocking(move || {
                    store
                        .lock()
                        .map_err(|_| ControlError::DataIntegrityInvalid)?
                        .update_api_key_connection(
                            &connection,
                            credential.as_ref(),
                            expected_revision,
                            expected_audit_sequence,
                            &expected_audit_hmac,
                            &audit_event,
                        )
                        .map_err(ControlError::storage)
                })
                .await
                .map_err(ControlError::storage)?
            }
            #[cfg(test)]
            Self::Fixed(_) | Self::Test { .. } => Err(ControlError::DataIntegrityInvalid),
        }
    }

    pub async fn sync_api_key_connection_models(
        &self,
        input: ConnectionModelSync,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: AuditEventRecord,
    ) -> Result<MutationWithAuditOutcome<ConnectionModelSyncOutcome>, ControlError> {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(store) => store
                .sync_api_key_connection_models(
                    &input,
                    expected_audit_sequence,
                    expected_audit_hmac,
                    &audit_event,
                )
                .await
                .map_err(ControlError::storage),
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            Self::SqlCipher(store) => {
                let store = Arc::clone(store);
                let expected_audit_hmac = expected_audit_hmac.to_owned();
                lifecycle::spawn_blocking(move || {
                    store
                        .lock()
                        .map_err(|_| ControlError::DataIntegrityInvalid)?
                        .sync_api_key_connection_models(
                            &input,
                            expected_audit_sequence,
                            &expected_audit_hmac,
                            &audit_event,
                        )
                        .map_err(ControlError::storage)
                })
                .await
                .map_err(ControlError::storage)?
            }
            #[cfg(test)]
            Self::Fixed(_) | Self::Test { .. } => Err(ControlError::DataIntegrityInvalid),
        }
    }

    pub async fn add_api_key_connection_models(
        &self,
        input: ConnectionModelSync,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: AuditEventRecord,
    ) -> Result<MutationWithAuditOutcome<ConnectionModelSyncOutcome>, ControlError> {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(store) => store
                .add_api_key_connection_models(
                    &input,
                    expected_audit_sequence,
                    expected_audit_hmac,
                    &audit_event,
                )
                .await
                .map_err(ControlError::storage),
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            Self::SqlCipher(store) => {
                let store = Arc::clone(store);
                let expected_audit_hmac = expected_audit_hmac.to_owned();
                lifecycle::spawn_blocking(move || {
                    store
                        .lock()
                        .map_err(|_| ControlError::DataIntegrityInvalid)?
                        .add_api_key_connection_models(
                            &input,
                            expected_audit_sequence,
                            &expected_audit_hmac,
                            &audit_event,
                        )
                        .map_err(ControlError::storage)
                })
                .await
                .map_err(ControlError::storage)?
            }
            #[cfg(test)]
            Self::Fixed(_) | Self::Test { .. } => Err(ControlError::DataIntegrityInvalid),
        }
    }

    pub async fn api_key_connection_route(
        &self,
        public_model: &str,
    ) -> Result<Option<ConnectionRoute>, ControlError> {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(store) => store
                .api_key_connection_route(public_model)
                .await
                .map_err(ControlError::storage),
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            Self::SqlCipher(store) => {
                let store = Arc::clone(store);
                let public_model = public_model.to_owned();
                lifecycle::spawn_blocking(move || {
                    store
                        .lock()
                        .map_err(|_| ControlError::DataIntegrityInvalid)?
                        .api_key_connection_route(&public_model)
                        .map_err(ControlError::storage)
                })
                .await
                .map_err(ControlError::storage)?
            }
            #[cfg(test)]
            Self::Fixed(_) | Self::Test { .. } => Ok(None),
        }
    }
}

const ACTIVE_SCHEMA: &str = "aster.plugin-active.v1";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompiledPluginKey {
    key_id: String,
    bundle_id: String,
    public_key_base64: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ActiveState {
    schema: String,
    bundle_id: String,
    digest: String,
    bundle_version: String,
    revision: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StreamMappingStep {
    state: Value,
    events: Vec<Value>,
    terminal: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginPreflightRequest {
    protocol: String,
    body: serde_json::Value,
    target: serde_json::Value,
    compatibility_mode: CompatibilityMode,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginVersionSwitchRequest {
    pub bundle_id: String,
    pub expected_activation_revision: u64,
    pub target_digest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateApiKeyConnectionRequest {
    pub channel_id: String,
    pub display_name: String,
    pub api_key: String,
    pub models: Vec<ConnectionModelRequest>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateApiKeyConnectionRequest {
    pub expected_revision: i64,
    pub enabled: Option<bool>,
    pub display_name: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RotateApiKeyConnectionRequest {
    pub expected_revision: i64,
    pub api_key: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncApiKeyModelsRequest {
    pub expected_revision: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddApiKeyModelsRequest {
    pub expected_revision: i64,
    pub models: Vec<ConnectionModelRequest>,
}

#[derive(Serialize)]
pub struct ApiKeyConnectionProbe {
    pub auth_observation: &'static str,
    pub discovery: &'static str,
    pub generation: &'static str,
    pub model_ids: Vec<String>,
    pub upstream_status: Option<u16>,
}

fn parse_api_key_models(body: &[u8]) -> Option<Vec<String>> {
    let root: serde_json::Value = serde_json::from_slice(body).ok()?;
    let items = root.get("data")?.as_array()?;
    if items.len() > 512 {
        return None;
    }
    let mut models = HashSet::new();
    for item in items {
        let id = item.get("id")?.as_str()?;
        if !valid_model_name(id) {
            return None;
        }
        models.insert(id.to_owned());
    }
    if models.len() > 256 {
        return None;
    }
    let mut models = models.into_iter().collect::<Vec<_>>();
    models.sort();
    Some(models)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionModelRequest {
    pub public_name: String,
    pub display_name: String,
    pub upstream_name: String,
    #[serde(default)]
    pub quota_unit: Option<String>,
}

struct ApiKeyChannelDefinition {
    id: &'static str,
    provider: &'static str,
    display_name: &'static str,
    billing_mode: &'static str,
    base_url: &'static str,
    host: &'static str,
    native_protocols: &'static [&'static str],
    supports_model_discovery: bool,
}

const API_KEY_CHANNELS: &[ApiKeyChannelDefinition] = &[
    ApiKeyChannelDefinition {
        id: "openai.api",
        provider: "openai",
        display_name: "OpenAI API Key",
        billing_mode: "usage",
        base_url: "https://api.openai.com/v1",
        host: "api.openai.com",
        native_protocols: &["responses", "chat", "images"],
        supports_model_discovery: true,
    },
    ApiKeyChannelDefinition {
        id: "deepseek.api",
        provider: "deepseek",
        display_name: "DeepSeek API Key",
        billing_mode: "usage",
        base_url: "https://api.deepseek.com",
        host: "api.deepseek.com",
        native_protocols: &["responses", "chat", "anthropic_messages"],
        supports_model_discovery: true,
    },
    ApiKeyChannelDefinition {
        id: "glm.bigmodel.general",
        provider: "glm",
        display_name: "GLM BigModel API Key",
        billing_mode: "usage",
        base_url: "https://open.bigmodel.cn/api/paas/v4",
        host: "open.bigmodel.cn",
        native_protocols: &["chat"],
        supports_model_discovery: false,
    },
    ApiKeyChannelDefinition {
        id: "glm.bigmodel.coding",
        provider: "glm",
        display_name: "GLM BigModel Coding Plan",
        billing_mode: "coding_plan",
        base_url: "https://open.bigmodel.cn/api/coding/paas/v4",
        host: "open.bigmodel.cn",
        native_protocols: &["chat"],
        supports_model_discovery: false,
    },
    ApiKeyChannelDefinition {
        id: "glm.zai.general",
        provider: "glm",
        display_name: "GLM Z.AI API Key",
        billing_mode: "usage",
        base_url: "https://api.z.ai/api/paas/v4",
        host: "api.z.ai",
        native_protocols: &["chat"],
        supports_model_discovery: false,
    },
    ApiKeyChannelDefinition {
        id: "glm.zai.coding",
        provider: "glm",
        display_name: "GLM Z.AI Coding Plan",
        billing_mode: "coding_plan",
        base_url: "https://api.z.ai/api/coding/paas/v4",
        host: "api.z.ai",
        native_protocols: &["chat"],
        supports_model_discovery: false,
    },
];

#[derive(Serialize)]
pub(crate) struct ApiKeyChannelView {
    id: &'static str,
    provider: &'static str,
    display_name: &'static str,
    endpoint_profile: &'static str,
    enrollment_kind: &'static str,
    billing_mode: &'static str,
    default_base_url: &'static str,
    native_protocols: &'static [&'static str],
    supports_model_discovery: bool,
}

pub(crate) fn api_key_channel_views() -> Vec<ApiKeyChannelView> {
    API_KEY_CHANNELS
        .iter()
        .map(|channel| ApiKeyChannelView {
            id: channel.id,
            provider: channel.provider,
            display_name: channel.display_name,
            endpoint_profile: channel.id,
            enrollment_kind: "api_key",
            billing_mode: channel.billing_mode,
            default_base_url: channel.base_url,
            native_protocols: channel.native_protocols,
            supports_model_discovery: channel.supports_model_discovery,
        })
        .collect()
}

fn official_api_key_channel(id: &str) -> Option<(&'static str, &'static str)> {
    API_KEY_CHANNELS
        .iter()
        .find(|channel| channel.id == id)
        .map(|channel| (channel.provider, channel.billing_mode))
}

fn valid_model_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 160
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':' | b'/')
        })
}

fn object_set_model(body: &mut serde_json::Value, model: &str) -> Result<(), ControlError> {
    body.as_object_mut()
        .ok_or(ControlError::GatewayRequestInvalid)?
        .insert(
            "model".to_owned(),
            serde_json::Value::String(model.to_owned()),
        );
    Ok(())
}

fn plugin_value<T>(result: PluginResult<T>) -> Result<T, ControlError> {
    match result {
        PluginResult::Ok { value } => Ok(value),
        PluginResult::Error { error } => {
            tracing::debug!(code=%error.code, reason=%error.reason_key, path=?error.source_path,
                "Lua adapter rejected request");
            Err(ControlError::GatewayRequestInvalid)
        }
    }
}

fn plugin_upstream_value<T>(result: PluginResult<T>) -> Result<T, ControlError> {
    match result {
        PluginResult::Ok { value } => Ok(value),
        PluginResult::Error { error } => {
            tracing::warn!(code=%error.code, reason=%error.reason_key, path=?error.source_path,
                "Lua adapter rejected upstream response");
            Err(ControlError::InvalidUpstreamResponse)
        }
    }
}

fn endpoint_base(id: &str) -> Option<(&'static str, &'static str)> {
    API_KEY_CHANNELS
        .iter()
        .find(|channel| channel.id == id)
        .map(|channel| (channel.base_url, channel.host))
}

pub(crate) fn connection_discovery_request(
    state: &ControlState,
    connection: &ConnectionRecord,
    credential: &ConnectionCredential,
) -> Result<Option<(RunnerTaskBinding, UpstreamHttpRequest)>, ControlError> {
    if !matches!(
        connection.channel_id.as_str(),
        "openai.api" | "deepseek.api"
    ) {
        return Ok(None);
    }
    if connection.status != "active"
        || credential.status != "active"
        || credential.connection_id != connection.id
    {
        return Err(ControlError::CredentialInvalid);
    }
    let (base, host) = endpoint_base(&connection.endpoint_profile)
        .ok_or(ControlError::UpstreamAccountInputInvalid)?;
    let material = EncryptedCredentialMaterial {
        encrypted_payload: credential.encrypted_payload.clone(),
        payload_nonce: credential.payload_nonce.clone(),
        wrapped_data_key: credential.wrapped_data_key.clone(),
        wrap_nonce: credential.wrap_nonce.clone(),
    };
    let secret = state
        .credential_vault()?
        .decrypt(
            &CredentialContext {
                credential_id: &credential.id,
                account_id: &connection.id,
                revision: credential.credential_revision,
            },
            &material,
        )
        .map_err(ControlError::CredentialVault)?;
    let secret = std::str::from_utf8(&secret).map_err(|_| ControlError::CredentialInvalid)?;
    if secret.len() < 8
        || secret.len() > 1024
        || secret
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(ControlError::CredentialInvalid);
    }
    Ok(Some((
        RunnerTaskBinding::credential(
            &connection.provider,
            &credential.id,
            credential.credential_revision,
            host,
        ),
        UpstreamHttpRequest {
            method: "GET".into(),
            url: format!("{base}/models"),
            headers: vec![
                UpstreamRequestHeader {
                    name: "authorization".into(),
                    value: format!("Bearer {secret}"),
                },
                UpstreamRequestHeader {
                    name: "accept".into(),
                    value: "application/json".into(),
                },
            ],
            body_base64url: String::new(),
        },
    )))
}

fn build_upstream_request(
    state: &ControlState,
    route: &ConnectionRoute,
    intent: &HttpIntent,
    wire_protocol: &str,
) -> Result<(RunnerTaskBinding, UpstreamHttpRequest), ControlError> {
    validate_http_intent(intent).map_err(|_| ControlError::GatewayRequestInvalid)?;
    let (base, host) = endpoint_base(&route.connection.endpoint_profile)
        .ok_or(ControlError::GatewayRequestInvalid)?;
    let expected_path = match wire_protocol {
        "responses" => "/responses",
        "chat" => "/chat/completions",
        "image_generate" => "/images/generations",
        "image_edit" => "/images/edits",
        _ => return Err(ControlError::GatewayRequestInvalid),
    };
    let multipart = wire_protocol == "image_edit";
    if intent.action != aster_plugin_core::contract::HostAction::Execute
        || intent.method != "POST"
        || intent.endpoint_id != route.connection.endpoint_profile
        || intent.relative_path != expected_path
        || !matches!(
            intent.response_mode,
            aster_plugin_core::contract::ResponseMode::Json
                | aster_plugin_core::contract::ResponseMode::Sse
        )
        || intent.public_headers.len() != 1
        || intent.public_headers[0].0 != "content-type"
        || intent.public_headers[0].1
            != if multipart {
                "multipart/form-data"
            } else {
                "application/json"
            }
        || intent.secret_bindings.len() != 1
        || intent.secret_bindings[0].slot != "api_key"
        || intent.secret_bindings[0].destination != "authorization_bearer"
        || intent.body.get("model").and_then(serde_json::Value::as_str)
            != Some(route.binding.upstream_name.as_str())
    {
        return Err(ControlError::GatewayRequestInvalid);
    }
    let credential = &route.credential;
    let context = CredentialContext {
        credential_id: &credential.id,
        account_id: &route.connection.id,
        revision: credential.credential_revision,
    };
    let material = EncryptedCredentialMaterial {
        encrypted_payload: credential.encrypted_payload.clone(),
        payload_nonce: credential.payload_nonce.clone(),
        wrapped_data_key: credential.wrapped_data_key.clone(),
        wrap_nonce: credential.wrap_nonce.clone(),
    };
    let key = state
        .credential_vault()?
        .decrypt(&context, &material)
        .map_err(ControlError::CredentialVault)?;
    let key = std::str::from_utf8(&key).map_err(|_| ControlError::CredentialInvalid)?;
    if key.is_empty() || key.chars().any(char::is_control) {
        return Err(ControlError::CredentialInvalid);
    }
    let (payload, content_type) = if multipart {
        serialize_image_multipart(&intent.body)?
    } else {
        (
            serde_json::to_vec(&intent.body).map_err(|_| ControlError::GatewayRequestInvalid)?,
            "application/json".to_owned(),
        )
    };
    let binding = RunnerTaskBinding::credential(
        &route.connection.provider,
        &credential.id,
        credential.credential_revision,
        host,
    );
    Ok((
        binding,
        UpstreamHttpRequest {
            method: "POST".into(),
            url: format!("{base}{expected_path}"),
            headers: vec![
                UpstreamRequestHeader {
                    name: "content-type".into(),
                    value: content_type,
                },
                UpstreamRequestHeader {
                    name: "authorization".into(),
                    value: format!("Bearer {key}"),
                },
            ],
            body_base64url: encode_base64url(&payload),
        },
    ))
}

fn serialize_image_multipart(body: &Value) -> Result<(Vec<u8>, String), ControlError> {
    let mut fields = body
        .as_object()
        .cloned()
        .ok_or(ControlError::GatewayRequestInvalid)?;
    let sources = fields
        .remove("__image_sources")
        .and_then(|value| value.as_array().cloned())
        .filter(|items| !items.is_empty() && items.len() <= 16)
        .ok_or(ControlError::GatewayRequestInvalid)?;
    let mask = fields.remove("__image_mask");
    let boundary = random_identifier("aster_image")?;
    let mut payload = Vec::new();
    for (name, value) in fields {
        if !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(ControlError::GatewayRequestInvalid);
        }
        let value = match value {
            Value::String(value) => value,
            Value::Number(value) => value.to_string(),
            _ => return Err(ControlError::GatewayRequestInvalid),
        };
        payload.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    for (index, source) in sources.iter().enumerate() {
        let source = source.as_str().ok_or(ControlError::GatewayRequestInvalid)?;
        append_image_part(&mut payload, &boundary, "image[]", index, source)?;
    }
    if let Some(mask) = mask {
        append_image_part(
            &mut payload,
            &boundary,
            "mask",
            0,
            mask.as_str().ok_or(ControlError::GatewayRequestInvalid)?,
        )?;
    }
    payload.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    if payload.len() > 18 * 1024 * 1024 {
        return Err(ControlError::GatewayRequestInvalid);
    }
    Ok((payload, format!("multipart/form-data; boundary={boundary}")))
}

fn append_image_part(
    payload: &mut Vec<u8>,
    boundary: &str,
    name: &str,
    index: usize,
    data_url: &str,
) -> Result<(), ControlError> {
    let (prefix, encoded) = data_url
        .split_once(',')
        .ok_or(ControlError::GatewayRequestInvalid)?;
    let mime = match prefix {
        "data:image/png;base64" => "image/png",
        "data:image/jpeg;base64" => "image/jpeg",
        "data:image/webp;base64" => "image/webp",
        _ => return Err(ControlError::GatewayRequestInvalid),
    };
    if name == "mask" && mime != "image/png" {
        return Err(ControlError::GatewayRequestInvalid);
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| ControlError::GatewayRequestInvalid)?;
    if bytes.len() > 10 * 1024 * 1024 || !valid_delivered_image(&bytes) {
        return Err(ControlError::GatewayRequestInvalid);
    }
    let extension = match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        _ => "webp",
    };
    payload.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"image-{index}.{extension}\"\r\nContent-Type: {mime}\r\n\r\n").as_bytes());
    payload.extend_from_slice(&bytes);
    payload.extend_from_slice(b"\r\n");
    Ok(())
}

fn image_asset_request(
    route: &ConnectionRoute,
    source: &str,
) -> Result<(RunnerTaskBinding, UpstreamHttpRequest), ControlError> {
    if source.len() > 8192 {
        return Err(ControlError::InvalidUpstreamResponse);
    }
    let url = url::Url::parse(source).map_err(|_| ControlError::InvalidUpstreamResponse)?;
    let host = url
        .host_str()
        .ok_or(ControlError::InvalidUpstreamResponse)?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
        || url.fragment().is_some()
        || host.parse::<std::net::IpAddr>().is_ok()
        || host
            .bytes()
            .any(|byte| !(byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.'))
    {
        return Err(ControlError::InvalidUpstreamResponse);
    }
    Ok((
        RunnerTaskBinding::credential(
            &route.connection.provider,
            &route.credential.id,
            route.credential.credential_revision,
            host,
        ),
        UpstreamHttpRequest {
            method: "GET".into(),
            url: source.into(),
            headers: Vec::new(),
            body_base64url: String::new(),
        },
    ))
}

struct PluginStreamContext {
    protocol: String,
    public_model: String,
    requested_model: String,
    wire_protocol: String,
    public_id: String,
    created: i64,
    multiplier_micros: i64,
}

async fn start_plugin_gateway_stream(
    state: ControlState,
    bundle: Arc<Bundle>,
    attempt: PluginModelAttempt<'_>,
    reservation: QuotaReservation,
    context: PluginStreamContext,
) -> Result<Response, ControlError> {
    let mut started = state
        .start_runner_http_request(
            None,
            &attempt.binding,
            RunnerTaskAuthorization::PluginModel(&attempt),
            &attempt.request,
        )
        .await?;
    if !(200..300).contains(&started.status) {
        let _ = crate::collect_streaming_error_body(&mut started).await;
        return Err(ControlError::UpstreamRequestFailed);
    }
    let (sender, receiver) = mpsc::channel::<Result<Bytes, Infallible>>(16);
    lifecycle::spawn(async move {
        forward_plugin_gateway_stream(state, bundle, started, reservation, context, sender).await;
    });
    let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|item| (item, receiver))
    });
    let mut response = (StatusCode::OK, Body::from_stream(stream)).into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    Ok(response)
}

type PluginStreamEvent = Option<(Vec<u8>, bool, Option<Value>, bool)>;

fn plugin_stream_event(
    frame: &[u8],
    wire_protocol: &str,
    model: &str,
) -> Result<PluginStreamEvent, ControlError> {
    let source = std::str::from_utf8(frame).map_err(|_| ControlError::InvalidUpstreamResponse)?;
    let mut data = None;
    for line in source.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(value) = line.strip_prefix("data:") {
            if data.replace(value.trim()).is_some() {
                return Err(ControlError::InvalidUpstreamResponse);
            }
        } else if !line.is_empty()
            && !line.starts_with(':')
            && !line.starts_with("event:")
            && !line.starts_with("id:")
        {
            return Err(ControlError::InvalidUpstreamResponse);
        }
    }
    let Some(data) = data else { return Ok(None) };
    if data == "[DONE]" {
        return if wire_protocol == "chat" {
            Ok(Some((b"data: [DONE]\n\n".to_vec(), true, None, false)))
        } else {
            Err(ControlError::InvalidUpstreamResponse)
        };
    }
    let mut event: Value =
        serde_json::from_str(data).map_err(|_| ControlError::InvalidUpstreamResponse)?;
    let object = event
        .as_object_mut()
        .ok_or(ControlError::InvalidUpstreamResponse)?;
    object.insert("model".into(), Value::String(model.to_owned()));
    let (kind, terminal, usage) = if wire_protocol == "responses" {
        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .ok_or(ControlError::InvalidUpstreamResponse)?
            .to_owned();
        if !matches!(
            kind.as_str(),
            "response.created"
                | "response.in_progress"
                | "response.completed"
                | "response.incomplete"
                | "response.failed"
                | "response.output_item.added"
                | "response.output_item.done"
                | "response.content_part.added"
                | "response.content_part.done"
                | "response.output_text.delta"
                | "response.output_text.done"
                | "response.function_call_arguments.delta"
                | "response.function_call_arguments.done"
                | "response.reasoning_text.delta"
                | "response.reasoning_text.done"
                | "response.reasoning_summary_part.added"
                | "response.reasoning_summary_part.done"
                | "response.reasoning_summary_text.delta"
                | "response.reasoning_summary_text.done"
        ) {
            return Err(ControlError::InvalidUpstreamResponse);
        }
        if let Some(response) = object.get_mut("response") {
            let response = response
                .as_object_mut()
                .ok_or(ControlError::InvalidUpstreamResponse)?;
            response.insert("model".into(), Value::String(model.to_owned()));
        }
        if kind == "response.failed" {
            return Err(ControlError::UpstreamRequestFailed);
        }
        let terminal = matches!(kind.as_str(), "response.completed" | "response.incomplete");
        let final_response = terminal.then(|| object.get("response").cloned()).flatten();
        if terminal && final_response.is_none() {
            return Err(ControlError::InvalidUpstreamResponse);
        }
        (Some(kind), terminal, final_response)
    } else if wire_protocol == "chat" {
        let choices = object
            .get("choices")
            .and_then(Value::as_array)
            .ok_or(ControlError::InvalidUpstreamResponse)?;
        if choices.len() > 1 {
            return Err(ControlError::InvalidUpstreamResponse);
        }
        (
            None,
            false,
            object
                .get("usage")
                .filter(|value| !value.is_null())
                .cloned(),
        )
    } else {
        return Err(ControlError::InvalidUpstreamResponse);
    };
    let json = serde_json::to_string(&event).map_err(|_| ControlError::InvalidUpstreamResponse)?;
    let encoded = if let Some(kind) = kind {
        format!("event: {kind}\ndata: {json}\n\n").into_bytes()
    } else {
        format!("data: {json}\n\n").into_bytes()
    };
    let finished = wire_protocol == "chat"
        && event["choices"].as_array().is_some_and(|choices| {
            choices.iter().any(|choice| {
                choice
                    .get("finish_reason")
                    .is_some_and(|reason| !reason.is_null())
            })
        });
    Ok(Some((encoded, terminal, usage, finished)))
}

async fn forward_plugin_gateway_stream(
    state: ControlState,
    bundle: Arc<Bundle>,
    mut started: StartedRunnerResponse,
    reservation: QuotaReservation,
    context: PluginStreamContext,
    sender: mpsc::Sender<Result<Bytes, Infallible>>,
) {
    let PluginStreamContext {
        protocol,
        public_model,
        requested_model,
        wire_protocol,
        public_id,
        created,
        multiplier_micros,
    } = context;
    let outcome = lifecycle::run_execution(RUNNER_EXECUTE_TASK_TIMEOUT, async {
        let mut expected_sequence = 0_u32;
        let mut pending = Vec::new();
        let mut total_bytes = 0_usize;
        let mut terminal = None;
        let mut final_data = None;
        let mut seen_chat_finish = false;
        let mut mapped_state = Value::Null;
        while let Some(frame) = started.events.recv().await {
            match frame {
                aster_runner_protocol::RunnerToControl::TaskChunk(frame)
                    if frame.task_id == started.task_id && frame.sequence == expected_sequence =>
                {
                    let chunk = decode_base64url(&frame.data_base64url)
                        .map_err(|_| ControlError::InvalidUpstreamResponse)?;
                    total_bytes = total_bytes
                        .checked_add(chunk.len())
                        .filter(|size| *size <= MAX_BUFFERED_UPSTREAM_RESPONSE_BYTES)
                        .ok_or(ControlError::UpstreamResponseTooLarge)?;
                    pending.extend_from_slice(&chunk);
                    while let Some(event) = take_sse_frame(&mut pending) {
                        if terminal.is_some() {
                            return Err(ControlError::InvalidUpstreamResponse);
                        }
                        if let Some((encoded, is_terminal, data, finished)) =
                            plugin_stream_event(&event, &wire_protocol, &requested_model)?
                        {
                            if wire_protocol == "chat" {
                                if is_terminal && !seen_chat_finish {
                                    return Err(ControlError::InvalidUpstreamResponse);
                                }
                                if finished {
                                    if seen_chat_finish {
                                        return Err(ControlError::InvalidUpstreamResponse);
                                    }
                                    seen_chat_finish = true;
                                }
                            }
                            if data.is_some() {
                                final_data = data;
                            }
                            if (wire_protocol == "chat"
                                && matches!(protocol.as_str(), "responses" | "anthropic_messages"))
                                || (wire_protocol == "responses" && protocol == "chat_completions")
                            {
                                let upstream_event = if is_terminal && wire_protocol == "chat" {
                                    Value::Null
                                } else {
                                    let json = std::str::from_utf8(&encoded)
                                        .map_err(|_| ControlError::InvalidUpstreamResponse)?;
                                    let data = json
                                        .lines()
                                        .find_map(|line| line.strip_prefix("data: "))
                                        .ok_or(ControlError::InvalidUpstreamResponse)?;
                                    serde_json::from_str::<Value>(data)
                                        .map_err(|_| ControlError::InvalidUpstreamResponse)?
                                };
                                let snapshot = Arc::clone(&bundle);
                                let current_state = std::mem::take(&mut mapped_state);
                                let id = public_id.clone();
                                let model = requested_model.clone();
                                let usage = final_data.clone();
                                let mapping_operation = if protocol == "responses" {
                                    "map_chat_to_responses_stream"
                                } else if protocol == "chat_completions" {
                                    "map_responses_to_chat_stream"
                                } else {
                                    "map_chat_to_messages_stream"
                                };
                                let step = lifecycle::spawn_blocking(move || {
                                    let runtime =
                                        LuaRuntime::new(snapshot, ExecutionLimits::default());
                                    let input = serde_json::json!({
                                        "state": current_state,
                                        "chunk":upstream_event, "event":upstream_event,
                                        "done":is_terminal, "usage":usage,
                                        "public_id":id, "public_model":model,
                                        "created":created,
                                    });
                                    runtime
                                        .invoke_typed::<StreamMappingStep>(
                                            "public",
                                            mapping_operation,
                                            &input,
                                        )
                                        .map_err(|_| ControlError::InvalidUpstreamResponse)
                                        .and_then(plugin_upstream_value)
                                })
                                .await
                                .map_err(|_| ControlError::InvalidUpstreamResponse)??;
                                if step.terminal != is_terminal || step.events.len() > 32 {
                                    return Err(ControlError::InvalidUpstreamResponse);
                                }
                                mapped_state = step.state;
                                let mut mapped = Vec::new();
                                for event in step.events {
                                    let kind = event
                                        .get("type")
                                        .and_then(Value::as_str)
                                        .ok_or(ControlError::InvalidUpstreamResponse)?;
                                    if protocol == "chat_completions" {
                                        if kind != "chat.chunk" {
                                            return Err(ControlError::InvalidUpstreamResponse);
                                        }
                                        let chunk = event
                                            .get("chunk")
                                            .ok_or(ControlError::InvalidUpstreamResponse)?;
                                        let json = serde_json::to_string(chunk)
                                            .map_err(|_| ControlError::InvalidUpstreamResponse)?;
                                        mapped.extend_from_slice(
                                            format!("data: {json}\n\n").as_bytes(),
                                        );
                                        continue;
                                    }
                                    let valid_kind = if protocol == "responses" {
                                        kind.starts_with("response.")
                                    } else {
                                        matches!(
                                            kind,
                                            "message_start"
                                                | "message_delta"
                                                | "message_stop"
                                                | "content_block_start"
                                                | "content_block_delta"
                                                | "content_block_stop"
                                        )
                                    };
                                    if !valid_kind
                                        || kind.bytes().any(|byte| {
                                            !(byte.is_ascii_lowercase()
                                                || byte == b'.'
                                                || byte == b'_')
                                        })
                                    {
                                        return Err(ControlError::InvalidUpstreamResponse);
                                    }
                                    let json = serde_json::to_string(&event)
                                        .map_err(|_| ControlError::InvalidUpstreamResponse)?;
                                    mapped.extend_from_slice(
                                        format!("event: {kind}\ndata: {json}\n\n").as_bytes(),
                                    );
                                }
                                if is_terminal && protocol == "chat_completions" {
                                    mapped.extend_from_slice(b"data: [DONE]\n\n");
                                }
                                if is_terminal {
                                    terminal = Some(mapped);
                                } else if !mapped.is_empty()
                                    && sender.send(Ok(Bytes::from(mapped))).await.is_err()
                                {
                                    return Err(ControlError::UpstreamRequestFailed);
                                }
                            } else if is_terminal {
                                terminal = Some(encoded);
                            } else if sender.send(Ok(Bytes::from(encoded))).await.is_err() {
                                return Err(ControlError::UpstreamRequestFailed);
                            }
                        }
                    }
                    expected_sequence = expected_sequence
                        .checked_add(1)
                        .ok_or(ControlError::InvalidUpstreamResponse)?;
                }
                aster_runner_protocol::RunnerToControl::TaskFinished(frame)
                    if frame.task_id == started.task_id && frame.status == started.status =>
                {
                    if !pending.is_empty() || terminal.is_none() {
                        return Err(ControlError::InvalidUpstreamResponse);
                    }
                    let data = final_data.ok_or(ControlError::UpstreamUsageInvalid)?;
                    let runtime = LuaRuntime::new(bundle, ExecutionLimits::default());
                    let usage = if wire_protocol == "chat" {
                        runtime
                            .invoke_typed::<aster_plugin_core::CanonicalUsageV2>(
                                "public",
                                "parse_stream_usage",
                                &serde_json::json!({"wire_protocol":"chat","usage":data}),
                            )
                            .map_err(|_| ControlError::UpstreamUsageInvalid)
                            .and_then(plugin_upstream_value)?
                    } else {
                        let result = runtime
                            .invoke_typed::<aster_plugin_core::CanonicalResultV2>(
                                "public",
                                "parse_buffered",
                                &serde_json::json!({"wire_protocol":"responses",
                                "public_model":public_model,"body":data}),
                            )
                            .map_err(|_| ControlError::InvalidUpstreamResponse)
                            .and_then(plugin_upstream_value)?;
                        result.usage.ok_or(ControlError::UpstreamUsageInvalid)?
                    };
                    let input = usage
                        .input_tokens
                        .ok_or(ControlError::UpstreamUsageInvalid)?;
                    let output = usage
                        .output_tokens
                        .ok_or(ControlError::UpstreamUsageInvalid)?;
                    let cached = usage.cached_input_tokens.unwrap_or(0);
                    if cached > input {
                        return Err(ControlError::UpstreamUsageInvalid);
                    }
                    let usage = ModelUsage {
                        uncached_input: i64::try_from(input - cached)
                            .map_err(|_| ControlError::UpstreamUsageInvalid)?,
                        cached_input: i64::try_from(cached)
                            .map_err(|_| ControlError::UpstreamUsageInvalid)?,
                        cache_write: 0,
                        output_tokens: i64::try_from(output)
                            .map_err(|_| ControlError::UpstreamUsageInvalid)?,
                        multiplier_micros,
                        protocol: protocol.clone(),
                        model: public_model.clone(),
                        requested_model: Some(requested_model.clone()),
                        processing_tier: None,
                        reasoning_effort: None,
                        runner_id: started.runner_id.clone(),
                    };
                    return Ok((usage, terminal.expect("checked terminal")));
                }
                aster_runner_protocol::RunnerToControl::TaskFailed(frame)
                    if frame.task_id == started.task_id =>
                {
                    return Err(ControlError::UpstreamRequestFailed);
                }
                _ => return Err(ControlError::InvalidUpstreamResponse),
            }
        }
        Err(ControlError::UpstreamRequestFailed)
    })
    .await
    .unwrap_or(Err(ControlError::UpstreamRequestFailed));
    drop(started._cancellation);
    match outcome {
        Ok((usage, terminal)) => {
            let settled = crate::gateway::durable_settlement::complete(
                &state,
                &reservation,
                crate::gateway::durable_settlement::Outcome::Usage(usage),
            )
            .await;
            if settled.is_ok() {
                let _ = sender.send(Ok(Bytes::from(terminal))).await;
            } else if let Err(error) = settled {
                let _ = sender
                    .send(Ok(Bytes::from(crate::gateway_stream_error_frame(&error))))
                    .await;
            }
        }
        Err(error) => {
            let failure = failed_model_request(
                &protocol,
                &public_model,
                Some(&requested_model),
                None,
                None,
                Some(&started.runner_id),
                &error,
            );
            let _ = crate::gateway::durable_settlement::complete(
                &state,
                &reservation,
                crate::gateway::durable_settlement::Outcome::FailureWithUnknown(failure),
            )
            .await;
            let _ = sender
                .send(Ok(Bytes::from(crate::gateway_stream_error_frame(&error))))
                .await;
        }
    }
}

pub fn compiled_plugin_publishers() -> Result<Vec<TrustedPublisher>, ControlError> {
    #[cfg(feature = "local-demo")]
    let runtime = std::env::var("ASTER_PLUGIN_TRUSTED_KEYS_JSON").ok();
    #[cfg(not(feature = "local-demo"))]
    let runtime: Option<String> = None;
    let source = runtime
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(compiled_keys::COMPILED_PLUGIN_KEYS_JSON);
    let entries: Vec<CompiledPluginKey> =
        serde_json::from_str(source).map_err(|_| ControlError::DataIntegrityInvalid)?;
    let mut seen = HashSet::new();
    if entries
        .iter()
        .any(|entry| !seen.insert(entry.key_id.as_str()))
    {
        return Err(ControlError::DataIntegrityInvalid);
    }
    entries
        .into_iter()
        .map(|entry| {
            let bytes = STANDARD
                .decode(entry.public_key_base64)
                .map_err(|_| ControlError::DataIntegrityInvalid)?;
            let bytes: [u8; 32] = bytes
                .try_into()
                .map_err(|_| ControlError::DataIntegrityInvalid)?;
            let public_key =
                VerifyingKey::from_bytes(&bytes).map_err(|_| ControlError::DataIntegrityInvalid)?;
            if entry.key_id.is_empty() || entry.bundle_id.is_empty() {
                return Err(ControlError::DataIntegrityInvalid);
            }
            Ok(TrustedPublisher {
                key_id: entry.key_id,
                bundle_id: entry.bundle_id,
                public_key,
            })
        })
        .collect()
}

const PLUGIN_PROVIDERS: &[(&str, &str)] = &[
    ("aster.openai", "openai"),
    ("aster.deepseek", "deepseek"),
    ("aster.glm", "glm"),
];

fn provider_bundle_id(provider: &str) -> Option<&'static str> {
    PLUGIN_PROVIDERS
        .iter()
        .find(|(_, name)| *name == provider)
        .map(|(id, _)| *id)
}

struct ProviderPlugin {
    provider: &'static str,
    slot: PluginSlot,
    layout: InstallLayout,
    update_lock: Mutex<()>,
    recovery_error: Mutex<Option<String>>,
}

impl ProviderPlugin {
    fn new(
        layout: InstallLayout,
        publishers: Vec<TrustedPublisher>,
        host_version: &str,
        provider: &'static str,
    ) -> Self {
        let manager = Self {
            provider,
            slot: PluginSlot::new(
                publishers,
                host_version.to_owned(),
                2,
                ExecutionLimits::default(),
            ),
            layout,
            update_lock: Mutex::new(()),
            recovery_error: Mutex::new(None),
        };
        if let Err(error) = manager.restore_active() {
            *manager
                .recovery_error
                .lock()
                .expect("plugin recovery lock poisoned") = Some(format!("{error:?}"));
            tracing::error!(?error, "active plugin needs administrative recovery");
        }
        manager
    }

    fn incoming_path(&self) -> std::path::PathBuf {
        self.layout
            .plugin_incoming_dir()
            .join(format!("{}.asterlua", self.provider))
    }

    fn state_path(&self) -> std::path::PathBuf {
        self.layout
            .plugin_state_dir()
            .join(format!("{}.json", self.provider))
    }

    fn versions_dir(&self) -> std::path::PathBuf {
        self.layout.plugin_versions_dir().join(self.provider)
    }

    pub fn active(&self) -> Option<Arc<Bundle>> {
        self.slot.active()
    }

    pub fn recovery_error(&self) -> Option<String> {
        self.recovery_error
            .lock()
            .expect("plugin recovery lock poisoned")
            .clone()
    }

    pub fn last_outcome(&self) -> CandidateOutcome {
        self.slot.last_outcome()
    }

    fn activation_state(&self) -> Result<Option<ActiveState>, ControlError> {
        match fs::read(self.state_path()) {
            Ok(bytes) => {
                let state: ActiveState = serde_json::from_slice(&bytes)
                    .map_err(|_| ControlError::DataIntegrityInvalid)?;
                if state.schema != ACTIVE_SCHEMA || !valid_digest(&state.digest) {
                    return Err(ControlError::DataIntegrityInvalid);
                }
                Ok(Some(state))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(ControlError::Io(error.to_string())),
        }
    }

    fn archived_digests(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(self.versions_dir()) else {
            return Vec::new();
        };
        let mut digests = entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter_map(|name| name.strip_suffix(".asterlua").map(str::to_owned))
            .filter(|digest| valid_digest(digest))
            .collect::<Vec<_>>();
        digests.sort();
        digests
    }

    fn archived_versions(&self) -> Vec<serde_json::Value> {
        self.archived_digests()
            .into_iter()
            .map(|digest| {
                let path = self.versions_dir().join(format!("{digest}.asterlua"));
                let bundle = fs::metadata(&path)
                    .ok()
                    .filter(|metadata| metadata.is_file() && metadata.len() <= 16 * 1024 * 1024)
                    .and_then(|_| fs::read(path).ok())
                    .and_then(|bytes| self.slot.inspect_archive(&bytes).ok())
                    .filter(|bundle| bundle.digest == digest);
                let available = bundle.is_some();
                serde_json::json!({
                    "digest": digest,
                    "version": bundle.as_ref().map(|bundle| &bundle.manifest.bundle_version),
                    "bundle_id": bundle.as_ref().map(|bundle| &bundle.manifest.bundle_id),
                    "display": bundle.as_ref().map(|bundle| &bundle.manifest.display),
                    "available": available,
                })
            })
            .collect()
    }

    fn restore_active(&self) -> Result<(), ControlError> {
        let path = self.state_path();
        let source = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(ControlError::Io(error.to_string())),
        };
        let state: ActiveState =
            serde_json::from_slice(&source).map_err(|_| ControlError::DataIntegrityInvalid)?;
        if state.schema != ACTIVE_SCHEMA || !valid_digest(&state.digest) {
            return Err(ControlError::DataIntegrityInvalid);
        }
        let archive_path = self
            .versions_dir()
            .join(format!("{}.asterlua", state.digest));
        let archive =
            fs::read(archive_path).map_err(|error| ControlError::Io(error.to_string()))?;
        let outcome = self.slot.submit(&archive, |bundle, _| {
            validate_description(bundle, self.provider)?;
            if bundle.digest != state.digest
                || bundle.manifest.bundle_id != state.bundle_id
                || bundle.manifest.bundle_version != state.bundle_version
            {
                return Err("active plugin record does not match signed bundle".into());
            }
            Ok(())
        });
        if !matches!(outcome, CandidateOutcome::Activated { .. }) {
            return Err(ControlError::DataIntegrityInvalid);
        }
        Ok(())
    }

    fn poll(&self) -> Option<CandidateOutcome> {
        let _update_guard = self
            .update_lock
            .lock()
            .expect("plugin update lock poisoned");
        if self.recovery_error().is_some() {
            return None;
        }
        let outcome = self.slot.poll(&self.incoming_path(), |bundle, archive| {
            validate_description(bundle, self.provider)?;
            self.persist(bundle, archive)
                .map_err(|error| format!("{error:?}"))
        });
        (!matches!(
            outcome,
            CandidateOutcome::Unchanged | CandidateOutcome::Absent
        ))
        .then_some(outcome)
    }

    fn submit_uploaded(&self, archive: &[u8]) -> Result<CandidateOutcome, ControlError> {
        let _update_guard = self
            .update_lock
            .lock()
            .expect("plugin update lock poisoned");
        let incoming = self.incoming_path();
        fs::create_dir_all(
            incoming
                .parent()
                .ok_or(ControlError::DataIntegrityInvalid)?,
        )
        .map_err(|error| ControlError::Io(error.to_string()))?;
        maintenance_atomic_write(&incoming, archive, false)?;
        let outcome = self.slot.submit(archive, |bundle, bytes| {
            validate_description(bundle, self.provider)?;
            self.persist(bundle, bytes)
                .map_err(|error| format!("{error:?}"))
        });
        if matches!(outcome, CandidateOutcome::Activated { .. }) {
            *self
                .recovery_error
                .lock()
                .expect("plugin recovery lock poisoned") = None;
        }
        Ok(outcome)
    }

    fn switch_version(
        &self,
        request: PluginVersionSwitchRequest,
    ) -> Result<CandidateOutcome, ControlError> {
        let _update_guard = self
            .update_lock
            .lock()
            .expect("plugin update lock poisoned");
        if !valid_digest(&request.target_digest) {
            return Err(ControlError::MaintenanceInvalid);
        }
        let archive_path = self
            .versions_dir()
            .join(format!("{}.asterlua", request.target_digest));
        let archive = fs::read(archive_path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ControlError::MaintenanceInvalid
            } else {
                ControlError::Io(error.to_string())
            }
        })?;
        if archive.len() > 16 * 1024 * 1024 {
            return Err(ControlError::MaintenanceInvalid);
        }
        let outcome = self.slot.submit(&archive, |bundle, bytes| {
            validate_description(bundle, self.provider)?;
            if bundle.digest != request.target_digest {
                return Err("archived bundle digest mismatch".into());
            }
            let active = self
                .activation_state()
                .map_err(|error| format!("{error:?}"))?;
            if active.as_ref().map(|state| state.revision)
                != Some(request.expected_activation_revision)
            {
                return Err("activation revision changed".into());
            }
            if active.as_ref().map(|state| state.digest.as_str())
                == Some(request.target_digest.as_str())
            {
                return Err("plugin version is already active".into());
            }
            self.persist(bundle, bytes)
                .map_err(|error| format!("{error:?}"))
        });
        if matches!(outcome, CandidateOutcome::Activated { .. }) {
            maintenance_atomic_write(&self.incoming_path(), &archive, false)?;
        }
        Ok(outcome)
    }

    fn persist(&self, bundle: &Bundle, archive: &[u8]) -> Result<(), ControlError> {
        let directory = self.versions_dir();
        fs::create_dir_all(&directory).map_err(|error| ControlError::Io(error.to_string()))?;
        let version = directory.join(format!("{}.asterlua", bundle.digest));
        match fs::read(&version) {
            Ok(existing) if existing == archive => {}
            Ok(_) => return Err(ControlError::DataIntegrityInvalid),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                maintenance_atomic_write(&version, archive, true)?;
            }
            Err(error) => return Err(ControlError::Io(error.to_string())),
        }
        let previous = match fs::read(self.state_path()) {
            Ok(bytes) => {
                let state: ActiveState = serde_json::from_slice(&bytes)
                    .map_err(|_| ControlError::DataIntegrityInvalid)?;
                if state.schema != ACTIVE_SCHEMA {
                    return Err(ControlError::DataIntegrityInvalid);
                }
                state.revision
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(ControlError::Io(error.to_string())),
        };
        let record = ActiveState {
            schema: ACTIVE_SCHEMA.into(),
            bundle_id: bundle.manifest.bundle_id.clone(),
            digest: bundle.digest.clone(),
            bundle_version: bundle.manifest.bundle_version.clone(),
            revision: previous
                .checked_add(1)
                .ok_or(ControlError::DataIntegrityInvalid)?,
        };
        let target = self.state_path();
        fs::create_dir_all(target.parent().ok_or(ControlError::DataIntegrityInvalid)?)
            .map_err(|error| ControlError::Io(error.to_string()))?;
        let encoded =
            serde_json::to_vec(&record).map_err(|_| ControlError::DataIntegrityInvalid)?;
        maintenance_atomic_write(&target, &encoded, false)
    }
}

pub struct PluginManager {
    providers: BTreeMap<&'static str, ProviderPlugin>,
}

impl PluginManager {
    fn new(layout: InstallLayout, publishers: Vec<TrustedPublisher>, host_version: &str) -> Self {
        let providers = PLUGIN_PROVIDERS
            .iter()
            .map(|(bundle_id, provider)| {
                (
                    *bundle_id,
                    ProviderPlugin::new(layout.clone(), publishers.clone(), host_version, provider),
                )
            })
            .collect();
        Self { providers }
    }

    fn for_bundle(&self, bundle_id: &str) -> Option<&ProviderPlugin> {
        self.providers.get(bundle_id)
    }

    pub fn active_for_provider(&self, provider: &str) -> Option<Arc<Bundle>> {
        self.for_bundle(provider_bundle_id(provider)?)
            .and_then(ProviderPlugin::active)
    }

    pub fn active_for_channel(&self, channel_id: &str) -> Option<Arc<Bundle>> {
        self.active_for_provider(channel_id.split('.').next()?)
    }

    fn poll(&self) -> Vec<(&'static str, CandidateOutcome)> {
        self.providers
            .iter()
            .filter_map(|(id, plugin)| plugin.poll().map(|outcome| (*id, outcome)))
            .collect()
    }

    fn submit_uploaded(&self, archive: &[u8]) -> Result<CandidateOutcome, ControlError> {
        let verifier = self
            .providers
            .values()
            .next()
            .ok_or(ControlError::MaintenanceInvalid)?;
        let bundle = verifier
            .slot
            .inspect_archive(archive)
            .map_err(|_| ControlError::MaintenanceInvalid)?;
        let plugin = self
            .for_bundle(&bundle.manifest.bundle_id)
            .ok_or(ControlError::MaintenanceInvalid)?;
        plugin.submit_uploaded(archive)
    }

    fn switch_version(
        &self,
        request: PluginVersionSwitchRequest,
    ) -> Result<CandidateOutcome, ControlError> {
        self.for_bundle(&request.bundle_id)
            .ok_or(ControlError::MaintenanceInvalid)?
            .switch_version(request)
    }
}

fn validate_description(bundle: &Bundle, provider: &str) -> Result<(), String> {
    if provider_bundle_id(provider) != Some(bundle.manifest.bundle_id.as_str()) {
        return Err("plugin bundle identity does not match provider".into());
    }
    let runtime = LuaRuntime::new(Arc::new(bundle.clone()), ExecutionLimits::default());
    let described = runtime
        .invoke_typed::<PluginDescriptor>("public", "describe", &serde_json::Value::Null)
        .map_err(|error| format!("invalid plugin description: {error}"))?;
    let PluginResult::Ok { value } = described else {
        return Err("plugin description rejected by Lua".into());
    };
    if value.host_api != 1
        || value.canonical_schema != 2
        || value.rule_revision.is_empty()
        || value.rule_revision.len() > 128
        || value.channels.is_empty()
        || value.channels.len() > 128
    {
        return Err("plugin description has unsupported version or size".into());
    }
    for (id, channel) in value.channels {
        if id != channel.profile
            || id.is_empty()
            || id.len() > 128
            || channel.provider != provider
            || !matches!(
                channel.wire_protocol.as_str(),
                "chat" | "responses" | "anthropic"
            )
        {
            return Err("plugin description has invalid channel".into());
        }
    }
    Ok(())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

impl ControlState {
    pub(crate) async fn prepare_codex_body(
        &self,
        body: serde_json::Value,
        upstream_model: &str,
    ) -> Result<Option<serde_json::Value>, ControlError> {
        let Some(manager) = self.plugins.as_ref() else {
            return Ok(None);
        };
        let bundle = manager
            .active_for_provider("openai")
            .ok_or(ControlError::UpstreamPluginUnavailable)?;
        let upstream_model = upstream_model.to_owned();
        lifecycle::spawn_blocking(move || {
            let runtime = LuaRuntime::new(bundle, ExecutionLimits::default());
            plugin_value(
                runtime
                    .invoke_typed::<serde_json::Value>(
                        "public",
                        "prepare_codex",
                        &serde_json::json!({"body":body,"upstream_model":upstream_model}),
                    )
                    .map_err(|_| ControlError::GatewayRequestInvalid)?,
            )
        })
        .await
        .map_err(|_| ControlError::DataIntegrityInvalid)?
        .map(Some)
    }

    pub(crate) async fn try_plugin_image(
        &self,
        consumer: &ModelConsumer,
        request_id: &GatewayRequestIds,
        body: Value,
        sources: Vec<String>,
        mask: Option<String>,
        edit: bool,
    ) -> Result<Option<Response>, ControlError> {
        let public_model = body
            .get("model")
            .and_then(Value::as_str)
            .ok_or(ControlError::GatewayRequestInvalid)?
            .to_owned();
        let Some(route) = self
            .credential_storage()?
            .api_key_connection_route(&public_model)
            .await?
        else {
            return Ok(None);
        };
        self.assert_model_access(consumer.identity_id(), &public_model)
            .await?;
        let count = body
            .get("n")
            .map_or(Some(1), Value::as_i64)
            .filter(|count| (1..=10).contains(count))
            .ok_or(ControlError::GatewayRequestInvalid)?;
        let bundle = self
            .plugins
            .as_ref()
            .and_then(|manager| manager.active_for_provider(&route.connection.provider))
            .ok_or(ControlError::UpstreamPluginUnavailable)?;
        let channel_id = route.connection.channel_id.clone();
        let upstream_model = route.binding.upstream_name.clone();
        let target = serde_json::json!({
            "channel_id":channel_id,
            "upstream_model":upstream_model,
        });
        let bundle_for_plan = Arc::clone(&bundle);
        let body_for_plan = body.clone();
        let source_handles = (0..sources.len())
            .map(|index| format!("asset_{index}"))
            .collect::<Vec<_>>();
        let mask_handle = mask.as_ref().map(|_| "asset_mask");
        let expected_handles = source_handles.clone();
        let mut intent = lifecycle::spawn_blocking(move || {
            let runtime = LuaRuntime::new(bundle_for_plan, ExecutionLimits::default());
            plugin_value(
                runtime
                    .invoke_typed::<HttpIntent>(
                        "public",
                        "prepare_image",
                        &serde_json::json!({"body":body_for_plan,"sources":source_handles,
                    "mask":mask_handle,"edit":edit,"target":target}),
                    )
                    .map_err(|_| ControlError::GatewayRequestInvalid)?,
            )
        })
        .await
        .map_err(|_| ControlError::DataIntegrityInvalid)??;
        if edit {
            let output = intent
                .body
                .as_object_mut()
                .ok_or(ControlError::GatewayRequestInvalid)?;
            let expected_mask = mask.as_ref().map(|_| Value::String("asset_mask".into()));
            if output.get("__image_sources") != Some(&serde_json::json!(expected_handles))
                || output.get("__image_mask") != expected_mask.as_ref()
            {
                return Err(ControlError::GatewayRequestInvalid);
            }
            output.insert("__image_sources".into(), serde_json::json!(sources));
            if let Some(mask) = mask {
                output.insert("__image_mask".into(), Value::String(mask));
            }
        }
        let wire = if edit { "image_edit" } else { "image_generate" };
        let (binding, request) = build_upstream_request(self, &route, &intent, wire)?;
        let orchestrator =
            crate::gateway::orchestrator::RequestOrchestrator::new(self, consumer, request_id);
        let spec = aster_policy_core::billing::plan::ImageSpec {
            size: body
                .get("size")
                .and_then(Value::as_str)
                .unwrap_or("1024x1024")
                .to_owned(),
            quality: body
                .get("quality")
                .and_then(Value::as_str)
                .unwrap_or("standard")
                .to_owned(),
        };
        let reservation = orchestrator
            .reserve(
                &public_model,
                "standard",
                &aster_policy_core::billing::plan::ReferenceRequest::Images {
                    requested_count: u32::try_from(count)
                        .map_err(|_| ControlError::GatewayRequestInvalid)?,
                    spec: spec.clone(),
                },
            )
            .await?;
        let mut data = Vec::new();
        let mut confirmed_count = 0_u32;
        let generation: Result<(), ControlError> = async {
            for _ in 0..count {
                let attempt = PluginImageAttempt::new(
                    consumer,
                    &reservation,
                    PluginImageProgress {
                        requested_count: count,
                        confirmed_images: false,
                    },
                    request_id,
                    &public_model,
                    route.clone(),
                    (binding.clone(), request.clone()),
                )?;
                let response = self
                    .execute_runner_http_request_with_command(
                        None,
                        &attempt.binding,
                        RunnerTaskAuthorization::PluginImage(&attempt),
                        &attempt.request,
                    )
                    .await?;
                if !(200..300).contains(&response.status) {
                    return Err(ControlError::UpstreamRequestFailed);
                }
                let upstream: Value = serde_json::from_slice(&response.body)
                    .map_err(|_| ControlError::InvalidUpstreamResponse)?;
                let items = upstream
                    .get("data")
                    .and_then(Value::as_array)
                    .filter(|items| items.len() == 1)
                    .ok_or(ControlError::InvalidUpstreamResponse)?;
                let item = items[0]
                    .as_object()
                    .ok_or(ControlError::InvalidUpstreamResponse)?;
                let sanitized = serde_json::json!({"data":[{
                    "b64_json":item.get("b64_json").map(|_| "image-payload"),
                    "url":item.get("url"),
                    "revised_prompt":item.get("revised_prompt"),
                }]});
                let runtime = LuaRuntime::new(Arc::clone(&bundle), ExecutionLimits::default());
                let provider = route.connection.provider.clone();
                let parsed = runtime
                    .invoke_typed::<PluginImageResult>(
                        "public",
                        "parse_image",
                        &serde_json::json!({"body":sanitized,"provider":provider}),
                    )
                    .map_err(|_| ControlError::InvalidUpstreamResponse)
                    .and_then(plugin_upstream_value)?;
                let encoded = if parsed.kind == "base64" && parsed.value == "image-payload" {
                    let raw = upstream
                        .pointer("/data/0/b64_json")
                        .and_then(Value::as_str)
                        .ok_or(ControlError::InvalidUpstreamResponse)?;
                    let bytes = STANDARD
                        .decode(raw)
                        .map_err(|_| ControlError::InvalidUpstreamResponse)?;
                    if !valid_delivered_image(&bytes) {
                        return Err(ControlError::InvalidUpstreamResponse);
                    }
                    confirmed_count += 1;
                    raw.to_owned()
                } else if parsed.kind == "url" && !parsed.value.is_empty() {
                    confirmed_count += 1;
                    let (asset_binding, asset_request) = image_asset_request(&route, &parsed.value)
                        .map_err(|_| ControlError::ImageDeliveryFailed)?;
                    let asset_attempt = PluginImageAttempt::new(
                        consumer,
                        &reservation,
                        PluginImageProgress {
                            requested_count: count,
                            confirmed_images: true,
                        },
                        request_id,
                        &public_model,
                        route.clone(),
                        (asset_binding, asset_request),
                    )?;
                    let asset = self
                        .execute_runner_http_request_with_command(
                            None,
                            &asset_attempt.binding,
                            RunnerTaskAuthorization::PluginImage(&asset_attempt),
                            &asset_attempt.request,
                        )
                        .await
                        .map_err(|_| ControlError::ImageDeliveryFailed)?;
                    if asset.status != 200 || !valid_delivered_image(&asset.body) {
                        return Err(ControlError::ImageDeliveryFailed);
                    }
                    STANDARD.encode(&asset.body)
                } else {
                    return Err(ControlError::InvalidUpstreamResponse);
                };
                data.push(serde_json::json!({
                    "b64_json": encoded,
                    "revised_prompt": parsed.revised_prompt,
                }));
            }
            Ok(())
        }
        .await;
        let usage = ModelUsage {
            uncached_input: 0,
            cached_input: 0,
            cache_write: 0,
            output_tokens: 0,
            multiplier_micros: 1_000_000,
            protocol: if edit {
                "images.edit"
            } else {
                "images.generate"
            }
            .to_owned(),
            model: public_model.clone(),
            requested_model: Some(public_model.clone()),
            processing_tier: None,
            reasoning_effort: None,
            runner_id: String::new(),
        };
        if confirmed_count > 0 {
            orchestrator
                .settle_images(&reservation, &usage, confirmed_count, spec)
                .await?;
        } else if let Err(error) = &generation {
            let failure = failed_model_request(
                &usage.protocol,
                &public_model,
                Some(&public_model),
                None,
                None,
                None,
                error,
            );
            orchestrator.fail(&reservation, &failure, true).await?;
        } else {
            orchestrator
                .settle_images(&reservation, &usage, 0, spec)
                .await?;
        }
        generation?;
        Ok(Some(with_aster_request_id(
            json_no_store(serde_json::json!({
                "created":(self.now)().unix_timestamp(), "data":data,
            })),
            request_id,
        )))
    }

    pub async fn public_model_capabilities(
        &self,
        model_id: &str,
        identity_id: Option<&str>,
    ) -> Result<Value, ControlError> {
        let storage = self.credential_storage()?;
        let model = storage
            .list_models()
            .await?
            .into_iter()
            .find(|model| model.id == model_id && model.enabled)
            .ok_or(ControlError::ModelNotFound)?;
        if let Some(identity_id) = identity_id {
            self.assert_model_access(identity_id, &model.public_name)
                .await?;
        }
        let quota_unit = storage
            .image_quota_unit(&model.id)
            .await?
            .unwrap_or_else(|| "token".into());
        let image_quota = if quota_unit == "image" {
            match identity_id {
                Some(identity_id) => self
                    .verified_image_balance(identity_id, &model.id)
                    .await?
                    .map(|balance| {
                        serde_json::json!({
                            "available_images":balance.available_images,
                            "reserved_images":balance.reserved_images,
                            "consumed_images":balance.consumed_images,
                        })
                    }),
                None => None,
            }
        } else {
            None
        };
        let Some(route) = storage.api_key_connection_route(&model.public_name).await? else {
            let codex = storage
                .gateway_route_candidates(&model.public_name)
                .await?
                .iter()
                .any(|candidate| candidate.provider == "openai");
            let active = self
                .plugins
                .as_ref()
                .and_then(|manager| manager.active_for_provider("openai"));
            return Ok(serde_json::json!({
                "model_id":model.id, "public_model":model.public_name,
                "source":if codex && active.is_some() { "codex_subscription" } else { "legacy" },
                "channel_id":if codex { Some("openai.codex.subscription") } else { None },
                "bundle_digest":active.as_ref().map(|bundle| bundle.digest.as_str()),
                "protocols":if codex { vec!["responses", "chat_completions", "anthropic_messages"] } else { Vec::new() },
                "rules":null,
                "quota_unit":quota_unit, "image_quota":image_quota,
            }));
        };
        let bundle = self
            .plugins
            .as_ref()
            .and_then(|manager| manager.active_for_provider(&route.connection.provider))
            .ok_or(ControlError::UpstreamPluginUnavailable)?;
        let source = bundle
            .files
            .get("public-rules.json")
            .ok_or(ControlError::DataIntegrityInvalid)?;
        let rules: Value =
            serde_json::from_slice(source).map_err(|_| ControlError::DataIntegrityInvalid)?;
        let model_rule = rules
            .get("models")
            .and_then(|models| models.get(route.binding.upstream_name.as_str()))
            .cloned();
        let image_rule = rules
            .get("image_models")
            .and_then(|models| models.get(route.binding.upstream_name.as_str()))
            .cloned();
        let protocols = if model_rule.is_some() {
            vec!["responses", "chat_completions", "anthropic_messages"]
        } else if let Some(rule) = &image_rule {
            if rule.get("edit").and_then(Value::as_bool) == Some(true) {
                vec!["images/generations", "images/edits"]
            } else {
                vec!["images/generations"]
            }
        } else {
            Vec::new()
        };
        let provider = &route.connection.provider;
        let channel_id = &route.connection.channel_id;
        let billing_mode = &route.connection.billing_mode;
        let upstream_model = &route.binding.upstream_name;
        Ok(serde_json::json!({
            "model_id":model.id,
            "public_model":model.public_name,
            "source":"signed_lua",
            "provider":provider,
            "channel_id":channel_id,
            "billing_mode":billing_mode,
            "quota_unit":quota_unit,
            "image_quota":image_quota,
            "upstream_model":upstream_model,
            "bundle_digest":bundle.digest,
            "rule_revision":rules.get("revision"),
            "compatibility_mode":"compatible",
            "protocols":protocols,
            "rules":model_rule.or(image_rule),
        }))
    }

    /// Returns None only when this model has no API-key connection, allowing
    /// the existing Codex OAuth path to continue during the migration.
    pub(crate) async fn try_plugin_gateway(
        &self,
        consumer: &ModelConsumer,
        headers: &HeaderMap,
        protocol: &str,
        body: &Bytes,
    ) -> Result<Option<Response>, ControlError> {
        let mut public_body: serde_json::Value =
            serde_json::from_slice(body).map_err(|_| ControlError::GatewayRequestInvalid)?;
        let requested_model = public_body
            .get("model")
            .and_then(serde_json::Value::as_str)
            .ok_or(ControlError::GatewayRequestInvalid)?
            .to_owned();
        let storage = self.credential_storage()?;
        let direct = storage.api_key_connection_route(&requested_model).await?;
        let (public_model, route) = if let Some(route) = direct {
            (requested_model.clone(), route)
        } else {
            let resolved = match self.resolve_gateway_model(&requested_model).await {
                Ok(resolved) => resolved,
                Err(ControlError::ModelNotFound) => return Ok(None),
                Err(error) => return Err(error),
            };
            let Some(route) = storage
                .api_key_connection_route(&resolved.base_model)
                .await?
            else {
                return Ok(None);
            };
            let mut options = serde_json::json!({"model": requested_model});
            let applied = resolved.apply_to_canonical(&mut options)?;
            if applied.processing_tier().is_some() {
                return Err(ControlError::GatewayRequestInvalid);
            }
            if let Some(effort) = applied.reasoning_effort() {
                let object = public_body
                    .as_object_mut()
                    .ok_or(ControlError::GatewayRequestInvalid)?;
                match protocol {
                    "chat_completions" => {
                        if object
                            .get("reasoning_effort")
                            .is_some_and(|value| value.as_str() != Some(effort))
                        {
                            return Err(ControlError::GatewayRequestInvalid);
                        }
                        object.insert("reasoning_effort".into(), effort.into());
                    }
                    "responses" => {
                        let reasoning = object
                            .entry("reasoning")
                            .or_insert_with(|| serde_json::json!({}));
                        let reasoning = reasoning
                            .as_object_mut()
                            .ok_or(ControlError::GatewayRequestInvalid)?;
                        if reasoning
                            .get("effort")
                            .is_some_and(|value| value.as_str() != Some(effort))
                        {
                            return Err(ControlError::GatewayRequestInvalid);
                        }
                        reasoning.insert("effort".into(), effort.into());
                    }
                    _ => return Err(ControlError::GatewayRequestInvalid),
                }
            }
            object_set_model(&mut public_body, &resolved.base_model)?;
            (resolved.base_model, route)
        };
        self.assert_model_access(consumer.identity_id(), &public_model)
            .await?;
        if self
            .credential_storage()?
            .image_quota_unit(&route.binding.model_id)
            .await?
            .as_deref()
            == Some("image")
        {
            return Err(ControlError::GatewayRequestInvalid);
        }
        let bundle = self
            .plugins
            .as_ref()
            .and_then(|manager| manager.active_for_provider(&route.connection.provider))
            .ok_or(ControlError::UpstreamPluginUnavailable)?;
        let channel_id = route.connection.channel_id.clone();
        let mut modes = headers.get_all("x-aster-compatibility").iter();
        let compatibility_mode = match (modes.next(), modes.next()) {
            (None, None) => CompatibilityMode::Compatible,
            (Some(value), None) => match value.to_str().map(str::trim).ok() {
                Some("compatible") => CompatibilityMode::Compatible,
                Some("strict") => CompatibilityMode::Strict,
                _ => return Err(ControlError::InvalidCompatibilityMode),
            },
            _ => return Err(ControlError::InvalidCompatibilityMode),
        };
        let connection_id = route.connection.id.clone();
        let connection_revision = route.connection.revision;
        let upstream_model = route.binding.upstream_name.clone();
        let target = serde_json::json!({
            "channel_id": channel_id,
            "connection_id": connection_id,
            "connection_revision": connection_revision,
            "upstream_model": upstream_model,
        });
        let bundle_for_plan = Arc::clone(&bundle);
        let protocol_owned = protocol.to_owned();
        let (operation, plan, intent) = lifecycle::spawn_blocking(move || {
            let runtime = LuaRuntime::new(bundle_for_plan, ExecutionLimits::default());
            let operation = plugin_value(
                runtime
                    .invoke_typed::<CanonicalOperation>(
                        "public",
                        "decode_request",
                        &serde_json::json!({"protocol": protocol_owned, "body": public_body}),
                    )
                    .map_err(|_| ControlError::GatewayRequestInvalid)?,
            )?;
            if operation.schema_version != 2
                || operation.kind != aster_plugin_core::contract::OperationKind::Generate
            {
                return Err(ControlError::GatewayRequestInvalid);
            }
            let plan = plugin_value(
                runtime
                    .invoke_typed::<AdaptationPlan>(
                        "public",
                        "assess",
                        &serde_json::json!({"operation":operation,"target":target,
                        "compatibility_mode":compatibility_mode}),
                    )
                    .map_err(|_| ControlError::GatewayRequestInvalid)?,
            )?;
            if !plan.compatible
                || !plan.rejected.is_empty()
                || plan.target.connection_id != target["connection_id"]
                || plan.target.connection_revision != target["connection_revision"]
                || plan.target.upstream_model != target["upstream_model"]
            {
                return Err(ControlError::GatewayRequestInvalid);
            }
            let intent = plugin_value(
                runtime
                    .invoke_typed::<HttpIntent>(
                        "public",
                        "prepare",
                        &serde_json::json!({"operation":operation,"target":target,"plan":plan}),
                    )
                    .map_err(|_| ControlError::GatewayRequestInvalid)?,
            )?;
            Ok::<_, ControlError>((operation, plan, intent))
        })
        .await
        .map_err(|_| ControlError::DataIntegrityInvalid)??;
        if operation.public_model != public_model
            || !matches!(
                operation.source_protocol,
                aster_plugin_core::contract::PublicProtocol::ChatCompletions
                    | aster_plugin_core::contract::PublicProtocol::Responses
                    | aster_plugin_core::contract::PublicProtocol::AnthropicMessages
            )
        {
            return Err(ControlError::GatewayRequestInvalid);
        }
        if operation.stream
            != matches!(
                intent.response_mode,
                aster_plugin_core::contract::ResponseMode::Sse
            )
        {
            return Err(ControlError::GatewayRequestInvalid);
        }
        if operation.stream
            && !matches!(
                (protocol, plan.target.wire_protocol.as_str()),
                ("chat_completions", "chat")
                    | ("chat_completions", "responses")
                    | ("responses", "responses")
                    | ("responses", "chat")
                    | ("anthropic_messages", "chat")
            )
        {
            return Err(ControlError::GatewayRequestInvalid);
        }
        let (binding, request) =
            build_upstream_request(self, &route, &intent, &plan.target.wire_protocol)?;
        let request_id: GatewayRequestIds = gateway_request_id(headers)?;
        let multiplier_micros = self.usage_multiplier_micros().await?;
        let orchestrator =
            crate::gateway::orchestrator::RequestOrchestrator::new(self, consumer, &request_id);
        let reservation = orchestrator
            .reserve(
                &public_model,
                "standard",
                &aster_policy_core::billing::plan::ReferenceRequest::TokenOneMillion,
            )
            .await?;
        let attempt = PluginModelAttempt::new(
            consumer,
            &reservation,
            &request_id,
            &public_model,
            route,
            binding,
            request,
        );
        let attempt = match attempt {
            Ok(attempt) => attempt,
            Err(error) => {
                let failure = failed_model_request(
                    protocol,
                    &public_model,
                    Some(&public_model),
                    None,
                    None,
                    None,
                    &error,
                );
                orchestrator.fail(&reservation, &failure, false).await?;
                return Err(error);
            }
        };
        if operation.stream {
            let response = start_plugin_gateway_stream(
                self.clone(),
                bundle,
                attempt,
                reservation.clone(),
                PluginStreamContext {
                    protocol: protocol.to_owned(),
                    public_model: public_model.clone(),
                    requested_model: requested_model.clone(),
                    wire_protocol: plan.target.wire_protocol.clone(),
                    public_id: random_identifier("response")?,
                    created: (self.now)().unix_timestamp(),
                    multiplier_micros,
                },
            )
            .await;
            return match response {
                Ok(response) => Ok(Some(with_aster_request_id(response, &request_id))),
                Err(error) => {
                    let stage =
                        crate::gateway::orchestrator::RequestOrchestrator::stage_for_control_error(
                            &error,
                        );
                    let failure = failed_model_request(
                        protocol,
                        &public_model,
                        Some(&requested_model),
                        None,
                        None,
                        None,
                        &error,
                    );
                    orchestrator
                        .fail(
                            &reservation,
                            &failure,
                            stage != crate::providers::CanonicalRequestStage::BeforeUpstream,
                        )
                        .await?;
                    Err(error)
                }
            };
        }
        let response = self
            .execute_runner_http_request_with_command(
                None,
                &attempt.binding,
                RunnerTaskAuthorization::PluginModel(&attempt),
                &attempt.request,
            )
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                let stage =
                    crate::gateway::orchestrator::RequestOrchestrator::stage_for_control_error(
                        &error,
                    );
                let failure = failed_model_request(
                    protocol,
                    &public_model,
                    Some(&public_model),
                    None,
                    None,
                    None,
                    &error,
                );
                orchestrator
                    .fail(
                        &reservation,
                        &failure,
                        stage != crate::providers::CanonicalRequestStage::BeforeUpstream,
                    )
                    .await?;
                return Err(error);
            }
        };
        if !(200..300).contains(&response.status) {
            let error = ControlError::UpstreamRequestFailed;
            let failure = failed_model_request(
                protocol,
                &public_model,
                Some(&public_model),
                None,
                None,
                Some(&response.runner_id),
                &error,
            );
            orchestrator.fail(&reservation, &failure, true).await?;
            return Err(error);
        }
        let encoded = (|| -> Result<(serde_json::Value, ModelUsage), ControlError> {
            let upstream_body = serde_json::from_slice::<serde_json::Value>(&response.body)
                .map_err(|_| ControlError::InvalidUpstreamResponse)?;
            let runtime = LuaRuntime::new(bundle, ExecutionLimits::default());
            let result = runtime.invoke_typed::<aster_plugin_core::CanonicalResultV2>(
                    "public", "parse_buffered",
                    &serde_json::json!({"body":upstream_body,"wire_protocol":plan.target.wire_protocol,
                        "public_model":public_model}),
                ).map_err(|_| ControlError::InvalidUpstreamResponse).and_then(plugin_upstream_value)?;
            let mut result = result;
            result.public_model = requested_model.clone();
            let output = runtime
                .invoke_typed::<serde_json::Value>(
                    "public",
                    "encode_public",
                    &serde_json::json!({"result":result,"protocol":protocol,
                        "public_id":random_identifier("response")?,
                        "created":(self.now)().unix_timestamp()}),
                )
                .map_err(|_| ControlError::InvalidUpstreamResponse)
                .and_then(plugin_upstream_value)?;
            let usage = result.usage.ok_or(ControlError::UpstreamUsageInvalid)?;
            let input = usage
                .input_tokens
                .ok_or(ControlError::UpstreamUsageInvalid)?;
            let output_tokens = usage
                .output_tokens
                .ok_or(ControlError::UpstreamUsageInvalid)?;
            let cached = usage.cached_input_tokens.unwrap_or(0);
            if cached > input {
                return Err(ControlError::UpstreamUsageInvalid);
            }
            let usage = ModelUsage {
                uncached_input: i64::try_from(input - cached)
                    .map_err(|_| ControlError::UpstreamUsageInvalid)?,
                cached_input: i64::try_from(cached)
                    .map_err(|_| ControlError::UpstreamUsageInvalid)?,
                cache_write: 0,
                output_tokens: i64::try_from(output_tokens)
                    .map_err(|_| ControlError::UpstreamUsageInvalid)?,
                multiplier_micros,
                protocol: protocol.to_owned(),
                model: public_model.to_owned(),
                requested_model: Some(requested_model.clone()),
                processing_tier: None,
                reasoning_effort: None,
                runner_id: response.runner_id.clone(),
            };
            Ok((output, usage))
        })();
        let (output, usage) = match encoded {
            Ok(value) => value,
            Err(error) => {
                let failure = failed_model_request(
                    protocol,
                    &public_model,
                    Some(&public_model),
                    None,
                    None,
                    Some(&response.runner_id),
                    &error,
                );
                orchestrator.fail(&reservation, &failure, true).await?;
                return Err(error);
            }
        };
        orchestrator.settle(&reservation, &usage).await?;
        Ok(Some(with_aster_request_id(
            json_no_store(output),
            &request_id,
        )))
    }

    pub async fn create_api_key_connection(
        &self,
        actor: &IdentityRecord,
        request: CreateApiKeyConnectionRequest,
    ) -> Result<serde_json::Value, ControlError> {
        let key = Zeroizing::new(request.api_key);
        let (provider, billing_mode) = official_api_key_channel(&request.channel_id)
            .ok_or(ControlError::UpstreamAccountInputInvalid)?;
        if request.display_name.trim().is_empty()
            || request.display_name.len() > 160
            || key.len() < 8
            || key.len() > 1024
            || key
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
            || request.models.len() > 64
        {
            return Err(ControlError::UpstreamAccountInputInvalid);
        }
        let bundle = self
            .plugins
            .as_ref()
            .and_then(|manager| manager.active_for_provider(provider))
            .ok_or(ControlError::UpstreamPluginUnavailable)?;
        let runtime = LuaRuntime::new(bundle, ExecutionLimits::default());
        let described = runtime
            .invoke_typed::<PluginDescriptor>("public", "describe", &serde_json::Value::Null)
            .map_err(|_| ControlError::DataIntegrityInvalid)?;
        let PluginResult::Ok { value: descriptor } = described else {
            return Err(ControlError::DataIntegrityInvalid);
        };
        let channel = descriptor
            .channels
            .get(&request.channel_id)
            .ok_or(ControlError::UpstreamAccountInputInvalid)?;
        if channel.provider != provider || channel.profile != request.channel_id {
            return Err(ControlError::DataIntegrityInvalid);
        }
        let mut seen = HashSet::new();
        let now =
            format_database_time((self.now)()).map_err(|_| ControlError::DataIntegrityInvalid)?;
        let mut models = Vec::with_capacity(request.models.len());
        for item in request.models {
            if !valid_model_name(&item.public_name)
                || !valid_model_name(&item.upstream_name)
                || item.display_name.is_empty()
                || item.display_name.len() > 160
                || !seen.insert(item.public_name.clone())
            {
                return Err(ControlError::UpstreamAccountInputInvalid);
            }
            let quota_unit = item.quota_unit.unwrap_or_else(|| "token".into());
            if !matches!(quota_unit.as_str(), "token" | "image")
                || (quota_unit == "image" && provider == "deepseek")
            {
                return Err(ControlError::UpstreamAccountInputInvalid);
            }
            models.push(ConnectionModelSpec {
                model: ModelRecord {
                    id: random_identifier("model")?,
                    public_name: item.public_name,
                    display_name: item.display_name,
                    enabled: false,
                    discovered_at: None,
                    created_at: now.clone(),
                },
                upstream_name: item.upstream_name,
                quota_unit,
            });
        }
        let connection_id = random_identifier("connection")?;
        let credential_id = random_identifier("credential")?;
        let vault = self.credential_vault()?;
        let material = vault
            .encrypt(
                &CredentialContext {
                    credential_id: &credential_id,
                    account_id: &connection_id,
                    revision: 0,
                },
                key.as_bytes(),
            )
            .map_err(ControlError::CredentialVault)?;
        let identity_hmac = vault
            .credential_identity_hmac(&request.channel_id, key.as_bytes())
            .map_err(ControlError::CredentialVault)?;
        let connection = ConnectionRecord {
            id: connection_id.clone(),
            provider: provider.into(),
            channel_id: request.channel_id.clone(),
            endpoint_profile: request.channel_id.clone(),
            auth_scheme: "api_key".into(),
            billing_mode: billing_mode.into(),
            display_name: request.display_name,
            legacy_account_id: None,
            status: "active".into(),
            revision: 1,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        let credential = ConnectionCredential {
            id: credential_id,
            connection_id: connection_id.clone(),
            credential_identity_hmac: identity_hmac,
            encrypted_payload: material.encrypted_payload,
            payload_nonce: material.payload_nonce,
            wrapped_data_key: material.wrapped_data_key,
            wrap_nonce: material.wrap_nonce,
            credential_revision: 0,
            status: "active".into(),
            created_at: now.clone(),
            updated_at: now,
        };
        let storage = self.credential_storage()?;
        let mut committed = false;
        for _ in 0..4 {
            let (sequence, hmac, audit_event) = self
                .prepare_audit_event(
                    Some(actor),
                    "upstream_connection.create",
                    "upstream_connection",
                    Some(&connection_id),
                    "succeeded",
                )
                .await?;
            crate::validate_admin_actor(self, actor).await?;
            crate::authorize_non_consuming_features(self, ["gateway"])?;
            match storage
                .insert_api_key_connection(
                    connection.clone(),
                    credential.clone(),
                    models.clone(),
                    sequence,
                    &hmac,
                    audit_event,
                )
                .await?
            {
                MutationWithAuditOutcome::Mutation(()) => {
                    committed = true;
                    break;
                }
                MutationWithAuditOutcome::AuditConflict => continue,
            }
        }
        if !committed {
            return Err(ControlError::DataIntegrityInvalid);
        }
        Ok(serde_json::json!({
            "id": connection_id,
            "provider": provider,
            "channel_id": request.channel_id,
            "billing_mode": billing_mode,
            "verification": "unverified",
            "revision": connection.revision,
        }))
    }

    pub async fn api_key_connections(&self) -> Result<serde_json::Value, ControlError> {
        let connections = self
            .credential_storage()?
            .list_api_key_connections()
            .await?;
        Ok(serde_json::json!({
            "items": connections.into_iter().map(|connection| serde_json::json!({
                "id": connection.id,
                "provider": connection.provider,
                "channel_id": connection.channel_id,
                "endpoint_profile": connection.endpoint_profile,
                "billing_mode": connection.billing_mode,
                "display_name": connection.display_name,
                "status": connection.status,
                "revision": connection.revision,
                "created_at": connection.created_at,
                "updated_at": connection.updated_at,
            })).collect::<Vec<_>>()
        }))
    }

    pub async fn probe_api_key_connection(
        &self,
        actor: &IdentityRecord,
        id: &str,
    ) -> Result<ApiKeyConnectionProbe, ControlError> {
        if !id.starts_with("connection_") || id.len() > 128 {
            return Err(ControlError::UpstreamAccountInputInvalid);
        }
        let (connection, credential) = self
            .credential_storage()?
            .api_key_connection(id)
            .await?
            .ok_or(ControlError::UpstreamAccountNotFound)?;
        let Some((binding, request)) =
            connection_discovery_request(self, &connection, &credential)?
        else {
            return Ok(ApiKeyConnectionProbe {
                auth_observation: "unknown",
                discovery: "unsupported",
                generation: "untested",
                model_ids: Vec::new(),
                upstream_status: None,
            });
        };
        let response = self
            .execute_runner_http_request_with_command(
                None,
                &binding,
                RunnerTaskAuthorization::PluginDiscover {
                    actor,
                    connection: &connection,
                    credential: &credential,
                },
                &request,
            )
            .await?;
        let parsed = if response.status == 200 {
            parse_api_key_models(&response.body)
        } else {
            None
        };
        let (auth_observation, discovery) = match response.status {
            200 if parsed.is_some() => ("passed", "passed"),
            200 => ("unknown", "invalid_response"),
            401 | 403 => ("failed", "unauthorized"),
            429 => ("unknown", "rate_limited"),
            _ => ("unknown", "upstream_error"),
        };
        Ok(ApiKeyConnectionProbe {
            auth_observation,
            discovery,
            generation: "untested",
            model_ids: parsed.unwrap_or_default(),
            upstream_status: Some(response.status),
        })
    }

    pub async fn sync_api_key_models_with_audit(
        &self,
        actor: &IdentityRecord,
        id: &str,
        expected_revision: i64,
    ) -> Result<serde_json::Value, ControlError> {
        if expected_revision < 1 {
            return Err(ControlError::UpstreamAccountInputInvalid);
        }
        let connection = self
            .credential_storage()?
            .api_key_connection(id)
            .await?
            .ok_or(ControlError::UpstreamAccountNotFound)?
            .0;
        if connection.revision != expected_revision {
            return Err(ControlError::UpstreamConnectionRevisionConflict);
        }
        let observation = self.probe_api_key_connection(actor, id).await?;
        if observation.discovery == "unsupported" {
            return Ok(serde_json::json!({
                "status": "unsupported", "added": 0,
                "reason": "channel_has_no_documented_model_list",
            }));
        }
        if observation.discovery != "passed" {
            return Err(ControlError::ModelSyncFailed);
        }
        let now =
            format_database_time((self.now)()).map_err(|_| ControlError::DataIntegrityInvalid)?;
        let mut models = Vec::with_capacity(observation.model_ids.len());
        for name in observation.model_ids {
            if !valid_model_name(&name) {
                continue;
            }
            models.push(ConnectionModelSpec {
                model: ModelRecord {
                    id: random_identifier("model")?,
                    public_name: name.clone(),
                    display_name: name.clone(),
                    enabled: false,
                    discovered_at: Some(now.clone()),
                    created_at: now.clone(),
                },
                upstream_name: name,
                quota_unit: "token".into(),
            });
        }
        let storage = self.credential_storage()?;
        for _ in 0..4 {
            let (sequence, hmac, audit_event) = self
                .prepare_audit_event(
                    Some(actor),
                    "upstream_connection.models.sync",
                    "upstream_connection",
                    Some(id),
                    "succeeded",
                )
                .await?;
            crate::validate_admin_actor(self, actor).await?;
            crate::authorize_non_consuming_operation(
                self,
                crate::BusinessOperationId::UpstreamSync,
            )?;
            match storage
                .sync_api_key_connection_models(
                    ConnectionModelSync {
                        connection_id: id.to_owned(),
                        expected_revision,
                        models: models.clone(),
                        now: now.clone(),
                    },
                    sequence,
                    &hmac,
                    audit_event,
                )
                .await?
            {
                MutationWithAuditOutcome::Mutation(ConnectionModelSyncOutcome::Applied {
                    added,
                }) => {
                    self.invalidate_gateway_model_variants()?;
                    return Ok(serde_json::json!({
                        "status": "synced", "added": added, "discovered": models.len(),
                        "revision": expected_revision.checked_add(1).ok_or(ControlError::DataIntegrityInvalid)?,
                    }));
                }
                MutationWithAuditOutcome::Mutation(ConnectionModelSyncOutcome::Conflict) => {
                    return Err(ControlError::UpstreamConnectionRevisionConflict);
                }
                MutationWithAuditOutcome::Mutation(ConnectionModelSyncOutcome::AlreadyBound) => {
                    return Err(ControlError::UpstreamAccountInputInvalid);
                }
                MutationWithAuditOutcome::AuditConflict => continue,
            }
        }
        Err(ControlError::DataIntegrityInvalid)
    }

    pub async fn add_api_key_models_with_audit(
        &self,
        actor: &IdentityRecord,
        id: &str,
        request: AddApiKeyModelsRequest,
    ) -> Result<serde_json::Value, ControlError> {
        if !id.starts_with("connection_")
            || id.len() > 128
            || request.expected_revision < 1
            || !(1..=64).contains(&request.models.len())
        {
            return Err(ControlError::UpstreamAccountInputInvalid);
        }
        let connection = self
            .credential_storage()?
            .api_key_connection(id)
            .await?
            .ok_or(ControlError::UpstreamAccountNotFound)?
            .0;
        if connection.revision != request.expected_revision {
            return Err(ControlError::UpstreamConnectionRevisionConflict);
        }
        if connection.status != "active" {
            return Err(ControlError::UpstreamAccountInputInvalid);
        }
        let now =
            format_database_time((self.now)()).map_err(|_| ControlError::DataIntegrityInvalid)?;
        let mut public_names = HashSet::new();
        let mut upstream_names = HashSet::new();
        let mut models = Vec::with_capacity(request.models.len());
        for item in request.models {
            if !valid_model_name(&item.public_name)
                || !valid_model_name(&item.upstream_name)
                || item.display_name.is_empty()
                || item.display_name.len() > 160
                || !public_names.insert(item.public_name.clone())
                || !upstream_names.insert(item.upstream_name.clone())
            {
                return Err(ControlError::UpstreamAccountInputInvalid);
            }
            let quota_unit = item.quota_unit.unwrap_or_else(|| "token".into());
            if !matches!(quota_unit.as_str(), "token" | "image")
                || (quota_unit == "image" && connection.provider == "deepseek")
            {
                return Err(ControlError::UpstreamAccountInputInvalid);
            }
            models.push(ConnectionModelSpec {
                model: ModelRecord {
                    id: random_identifier("model")?,
                    public_name: item.public_name,
                    display_name: item.display_name,
                    enabled: false,
                    discovered_at: None,
                    created_at: now.clone(),
                },
                upstream_name: item.upstream_name,
                quota_unit,
            });
        }
        let storage = self.credential_storage()?;
        for _ in 0..4 {
            let (sequence, hmac, audit_event) = self
                .prepare_audit_event(
                    Some(actor),
                    "upstream_connection.models.add",
                    "upstream_connection",
                    Some(id),
                    "succeeded",
                )
                .await?;
            crate::validate_admin_actor(self, actor).await?;
            crate::authorize_non_consuming_features(self, ["gateway"])?;
            match storage
                .add_api_key_connection_models(
                    ConnectionModelSync {
                        connection_id: id.to_owned(),
                        expected_revision: request.expected_revision,
                        models: models.clone(),
                        now: now.clone(),
                    },
                    sequence,
                    &hmac,
                    audit_event,
                )
                .await?
            {
                MutationWithAuditOutcome::Mutation(ConnectionModelSyncOutcome::Applied {
                    added,
                }) => {
                    self.invalidate_gateway_model_variants()?;
                    return Ok(serde_json::json!({
                        "added": added,
                        "revision": request.expected_revision.checked_add(1).ok_or(ControlError::DataIntegrityInvalid)?,
                    }));
                }
                MutationWithAuditOutcome::Mutation(ConnectionModelSyncOutcome::Conflict) => {
                    return Err(ControlError::UpstreamConnectionRevisionConflict);
                }
                MutationWithAuditOutcome::Mutation(ConnectionModelSyncOutcome::AlreadyBound) => {
                    return Err(ControlError::UpstreamAccountInputInvalid);
                }
                MutationWithAuditOutcome::AuditConflict => continue,
            }
        }
        Err(ControlError::DataIntegrityInvalid)
    }

    pub async fn api_key_connection_capabilities(
        &self,
        id: &str,
    ) -> Result<serde_json::Value, ControlError> {
        if !id.starts_with("connection_") || id.len() > 128 {
            return Err(ControlError::UpstreamAccountInputInvalid);
        }
        let connection = self
            .credential_storage()?
            .api_key_connection(id)
            .await?
            .ok_or(ControlError::UpstreamAccountNotFound)?
            .0;
        let bundle = self
            .plugins
            .as_ref()
            .and_then(|manager| manager.active_for_provider(&connection.provider))
            .ok_or(ControlError::UpstreamPluginUnavailable)?;
        let runtime = LuaRuntime::new(Arc::clone(&bundle), ExecutionLimits::default());
        let described = runtime
            .invoke_typed::<PluginDescriptor>("public", "describe", &Value::Null)
            .map_err(|_| ControlError::DataIntegrityInvalid)?;
        let PluginResult::Ok { value: descriptor } = described else {
            return Err(ControlError::DataIntegrityInvalid);
        };
        let channel = descriptor
            .channels
            .get(&connection.channel_id)
            .ok_or(ControlError::DataIntegrityInvalid)?;
        let rules: Value = bundle
            .files
            .get("public-rules.json")
            .map(|bytes| serde_json::from_slice(bytes))
            .transpose()
            .map_err(|_| ControlError::DataIntegrityInvalid)?
            .unwrap_or(Value::Null);
        let filtered = |section: &str| -> serde_json::Map<String, Value> {
            rules
                .get(section)
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|entries| entries.iter())
                .filter(|(_, rule)| {
                    rule.get("provider").and_then(Value::as_str)
                        == Some(connection.provider.as_str())
                })
                .map(|(name, rule)| (name.clone(), rule.clone()))
                .collect()
        };
        Ok(serde_json::json!({
            "connection_id": connection.id,
            "channel_id": connection.channel_id,
            "status": connection.status,
            "revision": connection.revision,
            "channel": channel,
            "rule_revision": descriptor.rule_revision,
            "bundle_digest": bundle.digest,
            "evidence": if rules.is_null() { "descriptor_only" } else { "published_plugin_rules" },
            "text_models": filtered("models"),
            "image_models": filtered("image_models"),
        }))
    }

    pub async fn update_api_key_connection_management(
        &self,
        actor: &IdentityRecord,
        id: &str,
        expected_revision: i64,
        enabled: Option<bool>,
        display_name: Option<String>,
        new_secret: Option<String>,
    ) -> Result<serde_json::Value, ControlError> {
        if !id.starts_with("connection_") || id.len() > 128 || expected_revision < 1 {
            return Err(ControlError::UpstreamAccountInputInvalid);
        }
        if enabled.is_none() && display_name.is_none() && new_secret.is_none() {
            return Err(ControlError::UpstreamAccountInputInvalid);
        }
        let storage = self.credential_storage()?;
        let (mut connection, mut credential) = storage
            .api_key_connection(id)
            .await?
            .ok_or(ControlError::UpstreamAccountNotFound)?;
        if connection.revision != expected_revision {
            return Err(ControlError::UpstreamConnectionRevisionConflict);
        }
        if let Some(name) = display_name {
            if name.trim().is_empty() || name.len() > 160 {
                return Err(ControlError::UpstreamAccountInputInvalid);
            }
            connection.display_name = name.trim().to_owned();
        }
        if let Some(enabled) = enabled {
            connection.status = if enabled { "active" } else { "disabled" }.into();
        }
        let now =
            format_database_time((self.now)()).map_err(|_| ControlError::DataIntegrityInvalid)?;
        connection.updated_at = now.clone();
        connection.revision = connection
            .revision
            .checked_add(1)
            .ok_or(ControlError::DataIntegrityInvalid)?;
        let rotating = new_secret.is_some();
        if let Some(secret) = new_secret {
            let secret = Zeroizing::new(secret);
            if secret.len() < 8
                || secret.len() > 1024
                || secret
                    .bytes()
                    .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
            {
                return Err(ControlError::UpstreamAccountInputInvalid);
            }
            credential.credential_revision = credential
                .credential_revision
                .checked_add(1)
                .ok_or(ControlError::DataIntegrityInvalid)?;
            let vault = self.credential_vault()?;
            let material = vault
                .encrypt(
                    &CredentialContext {
                        credential_id: &credential.id,
                        account_id: &connection.id,
                        revision: credential.credential_revision,
                    },
                    secret.as_bytes(),
                )
                .map_err(ControlError::CredentialVault)?;
            credential.credential_identity_hmac = vault
                .credential_identity_hmac(&connection.channel_id, secret.as_bytes())
                .map_err(ControlError::CredentialVault)?;
            credential.encrypted_payload = material.encrypted_payload;
            credential.payload_nonce = material.payload_nonce;
            credential.wrapped_data_key = material.wrapped_data_key;
            credential.wrap_nonce = material.wrap_nonce;
            credential.status = "active".into();
            credential.updated_at = now;
        }
        for _ in 0..4 {
            let (sequence, hmac, audit_event) = self
                .prepare_audit_event(
                    Some(actor),
                    if rotating {
                        "upstream_connection.rotate_key"
                    } else {
                        "upstream_connection.update"
                    },
                    "upstream_connection",
                    Some(id),
                    "succeeded",
                )
                .await?;
            crate::validate_admin_actor(self, actor).await?;
            crate::authorize_non_consuming_features(self, ["gateway"])?;
            match storage
                .update_api_key_connection(
                    connection.clone(),
                    rotating.then_some(credential.clone()),
                    expected_revision,
                    sequence,
                    &hmac,
                    audit_event,
                )
                .await?
            {
                MutationWithAuditOutcome::Mutation(ConnectionMutationOutcome::Updated) => {
                    return Ok(serde_json::json!({
                        "id": connection.id,
                        "revision": connection.revision,
                        "status": connection.status,
                        "display_name": connection.display_name,
                        "credential_revision": credential.credential_revision,
                    }));
                }
                MutationWithAuditOutcome::Mutation(ConnectionMutationOutcome::Conflict) => {
                    return Err(ControlError::UpstreamConnectionRevisionConflict);
                }
                MutationWithAuditOutcome::AuditConflict => continue,
            }
        }
        Err(ControlError::DataIntegrityInvalid)
    }

    pub fn with_plugin_bundle(mut self, layout: InstallLayout) -> Result<Self, ControlError> {
        let publishers = compiled_plugin_publishers()?;
        if !publishers.is_empty() {
            if PLUGIN_PROVIDERS
                .iter()
                .any(|(bundle_id, _)| !publishers.iter().any(|key| key.bundle_id == *bundle_id))
            {
                return Err(ControlError::DataIntegrityInvalid);
            }
            self.plugins = Some(Arc::new(PluginManager::new(
                layout,
                publishers,
                &self.product_version,
            )));
        }
        Ok(self)
    }

    pub fn plugins_configured(&self) -> bool {
        self.plugins.is_some()
    }

    pub fn plugin_status(&self) -> serde_json::Value {
        let Some(manager) = self.plugins.as_ref() else {
            return serde_json::json!({"configured": false, "plugins": []});
        };
        let plugins = PLUGIN_PROVIDERS.iter().filter_map(|(bundle_id, _)| {
            let plugin = manager.for_bundle(bundle_id)?;
            let active = plugin.active();
            let activation_revision = plugin.activation_state().ok().flatten().map(|state| state.revision);
            let submission = match plugin.last_outcome() {
                CandidateOutcome::Absent => serde_json::json!({"status": "absent"}),
                CandidateOutcome::Unchanged => serde_json::json!({"status": "unchanged"}),
                CandidateOutcome::Rejected { digest, reason } => {
                    serde_json::json!({"status": "rejected", "digest": digest, "reason": reason})
                }
                CandidateOutcome::Activated { digest } => {
                    serde_json::json!({"status": "activated", "digest": digest})
                }
            };
            Some(serde_json::json!({
                "bundle_id": bundle_id,
                "provider": plugin.provider,
                "active_digest": active.as_ref().map(|bundle| bundle.digest.as_str()),
                "active_version": active.as_ref().map(|bundle| bundle.manifest.bundle_version.as_str()),
                "active_display": active.as_ref().map(|bundle| &bundle.manifest.display),
                "activation_revision": activation_revision,
                "versions": plugin.archived_versions(),
                "recovery_error": plugin.recovery_error(),
                "last_submission": submission,
            }))
        }).collect::<Vec<_>>();
        serde_json::json!({"configured": true, "plugins": plugins})
    }

    pub async fn plugin_preflight(
        &self,
        request: PluginPreflightRequest,
    ) -> Result<serde_json::Value, ControlError> {
        let bundle = self
            .plugins
            .as_ref()
            .and_then(|manager| {
                manager.active_for_channel(request.target.get("channel_id")?.as_str()?)
            })
            .ok_or(ControlError::UpstreamPluginUnavailable)?;
        lifecycle::spawn_blocking(move || {
            let runtime = LuaRuntime::new(Arc::clone(&bundle), ExecutionLimits::default());
            let decoded = runtime
                .invoke_typed::<CanonicalOperation>(
                    "public",
                    "decode_request",
                    &serde_json::json!({"protocol":request.protocol,"body":request.body}),
                )
                .map_err(|_| ControlError::GatewayRequestInvalid)?;
            let operation = match decoded {
                PluginResult::Ok { value } => value,
                PluginResult::Error { error } => {
                    return Ok(
                        serde_json::json!({"status":"rejected","stage":"decode","error":error}),
                    );
                }
            };
            let plan = runtime
                .invoke_typed::<AdaptationPlan>(
                    "public",
                    "assess",
                    &serde_json::json!({
                        "operation":operation,
                        "target":request.target,
                        "compatibility_mode":request.compatibility_mode,
                    }),
                )
                .map_err(|_| ControlError::GatewayRequestInvalid)?;
            let plan = match plan {
                PluginResult::Ok { value } => value,
                PluginResult::Error { error } => {
                    return Ok(
                        serde_json::json!({"status":"rejected","stage":"assess","error":error}),
                    );
                }
            };
            if !plan.compatible {
                return Ok(serde_json::json!({
                    "status":"incompatible", "bundle_digest":bundle.digest,
                    "operation":operation, "plan":plan,
                }));
            }
            let intent = runtime
                .invoke_typed::<HttpIntent>(
                    "public",
                    "prepare",
                    &serde_json::json!({
                        "operation":operation,
                        "target":request.target,
                        "plan":plan,
                    }),
                )
                .map_err(|_| ControlError::GatewayRequestInvalid)?;
            let intent = match intent {
                PluginResult::Ok { value } => value,
                PluginResult::Error { error } => {
                    return Ok(
                        serde_json::json!({"status":"rejected","stage":"prepare","error":error}),
                    );
                }
            };
            validate_http_intent(&intent).map_err(|_| ControlError::GatewayRequestInvalid)?;
            Ok(serde_json::json!({
                "status":"compatible", "bundle_digest":bundle.digest,
                "operation":operation, "plan":plan, "intent":intent,
            }))
        })
        .await
        .map_err(|_| ControlError::DataIntegrityInvalid)?
    }

    pub async fn poll_plugin_updates(&self) -> Result<(), ControlError> {
        let Some(plugins) = self.plugins.as_ref() else {
            return Ok(());
        };
        let maintenance = self
            .maintenance
            .as_ref()
            .ok_or(ControlError::DataIntegrityInvalid)?;
        // Upgrade queueing uses the same mutex. It cannot start between this
        // check and the complete plugin validation/persistence/switch.
        let _update_permit = maintenance.submissions.lock().await;
        maintenance.prepare()?;
        if maintenance.has_active_job()? {
            return Ok(());
        }
        let plugins = Arc::clone(plugins);
        let outcomes = lifecycle::spawn_blocking(move || plugins.poll())
            .await
            .map_err(|_| ControlError::DataIntegrityInvalid)?;
        for (bundle_id, outcome) in outcomes {
            match outcome {
                CandidateOutcome::Activated { digest } => {
                    tracing::info!(%bundle_id, %digest, "signed Lua bundle activated");
                }
                CandidateOutcome::Rejected { digest, reason } => {
                    tracing::warn!(%bundle_id, %digest, %reason, "Lua bundle submission rejected");
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub async fn submit_plugin_bundle(
        &self,
        actor: &IdentityRecord,
        archive: &[u8],
    ) -> Result<CandidateOutcome, ControlError> {
        if archive.is_empty() || archive.len() > 16 * 1024 * 1024 {
            return Err(ControlError::MaintenanceInvalid);
        }
        let plugins = self
            .plugins
            .as_ref()
            .ok_or(ControlError::UpstreamPluginUnavailable)?;
        let maintenance = self
            .maintenance
            .as_ref()
            .ok_or(ControlError::DataIntegrityInvalid)?;
        let _update_permit = maintenance.submissions.lock().await;
        maintenance.prepare()?;
        if maintenance.has_active_job()? {
            return Err(ControlError::MaintenanceBusy);
        }
        crate::validate_admin_actor(self, actor).await?;
        crate::authorize_non_consuming_features(self, ["gateway"])?;
        let plugins = Arc::clone(plugins);
        let archive = archive.to_vec();
        lifecycle::spawn_blocking(move || plugins.submit_uploaded(&archive))
            .await
            .map_err(|_| ControlError::DataIntegrityInvalid)?
    }

    pub async fn switch_plugin_version(
        &self,
        actor: &IdentityRecord,
        request: PluginVersionSwitchRequest,
    ) -> Result<CandidateOutcome, ControlError> {
        let plugins = self
            .plugins
            .as_ref()
            .ok_or(ControlError::UpstreamPluginUnavailable)?;
        let maintenance = self
            .maintenance
            .as_ref()
            .ok_or(ControlError::DataIntegrityInvalid)?;
        let _update_permit = maintenance.submissions.lock().await;
        maintenance.prepare()?;
        if maintenance.has_active_job()? {
            return Err(ControlError::MaintenanceBusy);
        }
        crate::validate_admin_actor(self, actor).await?;
        crate::authorize_non_consuming_features(self, ["gateway"])?;
        let plugins = Arc::clone(plugins);
        lifecycle::spawn_blocking(move || plugins.switch_version(request))
            .await
            .map_err(|_| ControlError::DataIntegrityInvalid)?
    }
}

fn valid_delivered_image(bytes: &[u8]) -> bool {
    if bytes.is_empty() || bytes.len() > 32 * 1024 * 1024 {
        return false;
    }
    let dimensions = if bytes.len() >= 24
        && bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        && &bytes[12..16] == b"IHDR"
    {
        Some((
            u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
            u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
        ))
    } else if bytes.len() >= 30 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        match &bytes[12..16] {
            b"VP8X" => Some((
                1 + u32::from(bytes[24])
                    + (u32::from(bytes[25]) << 8)
                    + (u32::from(bytes[26]) << 16),
                1 + u32::from(bytes[27])
                    + (u32::from(bytes[28]) << 8)
                    + (u32::from(bytes[29]) << 16),
            )),
            b"VP8L" if bytes[20] == 0x2f => Some((
                1 + u32::from(bytes[21]) + (u32::from(bytes[22] & 0x3f) << 8),
                1 + u32::from(bytes[22] >> 6)
                    + (u32::from(bytes[23]) << 2)
                    + (u32::from(bytes[24] & 0x0f) << 10),
            )),
            b"VP8 " if &bytes[23..26] == b"\x9d\x01\x2a" => Some((
                u32::from(u16::from_le_bytes(bytes[26..28].try_into().unwrap()) & 0x3fff),
                u32::from(u16::from_le_bytes(bytes[28..30].try_into().unwrap()) & 0x3fff),
            )),
            _ => None,
        }
    } else if bytes.starts_with(b"\xff\xd8") {
        jpeg_dimensions(bytes)
    } else {
        None
    };
    dimensions.is_some_and(|(width, height)| {
        width > 0
            && height > 0
            && width <= 16_384
            && height <= 16_384
            && u64::from(width) * u64::from(height) <= 67_108_864
    })
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut offset = 2;
    while offset + 4 <= bytes.len() {
        if bytes[offset] != 0xff {
            return None;
        }
        while offset < bytes.len() && bytes[offset] == 0xff {
            offset += 1;
        }
        let marker = *bytes.get(offset)?;
        offset += 1;
        if marker == 0xd9 || marker == 0xda {
            return None;
        }
        let length = usize::from(u16::from_be_bytes(
            bytes.get(offset..offset + 2)?.try_into().ok()?,
        ));
        if length < 2 || offset.checked_add(length)? > bytes.len() {
            return None;
        }
        if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            if length < 7 {
                return None;
            }
            let height = u32::from(u16::from_be_bytes(
                bytes[offset + 3..offset + 5].try_into().ok()?,
            ));
            let width = u32::from(u16::from_be_bytes(
                bytes[offset + 5..offset + 7].try_into().ok()?,
            ));
            return Some((width, height));
        }
        offset += length;
    }
    None
}

#[cfg(test)]
mod image_header_tests {
    use super::valid_delivered_image;

    #[test]
    fn rejects_oversized_dimensions_and_non_images() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&1024_u32.to_be_bytes());
        png.extend_from_slice(&1024_u32.to_be_bytes());
        assert!(valid_delivered_image(&png));
        png[16..20].copy_from_slice(&100_000_u32.to_be_bytes());
        assert!(!valid_delivered_image(&png));
        assert!(!valid_delivered_image(b"<html>not an image</html>"));
        assert!(!valid_delivered_image(b"\xff\xd8\xff\xd9"));
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::parse_api_key_models;

    #[test]
    fn model_list_requires_bounded_valid_ids_and_deduplicates() {
        let names = parse_api_key_models(br#"{"object":"list","data":[{"id":"deepseek-flash"},{"id":"deepseek-flash"},{"id":"deepseek-v4-pro"}]}"#).unwrap();
        assert_eq!(names, ["deepseek-flash", "deepseek-v4-pro"]);
        assert!(parse_api_key_models(br#"{"data":[{"id":"bad model"}]}"#).is_none());
        assert!(parse_api_key_models(br#"{"data":"not-an-array"}"#).is_none());
    }
}
