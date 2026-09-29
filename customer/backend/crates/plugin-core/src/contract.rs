//! Host API v1 wire types. Every Lua result is untrusted until the Control
//! layer checks its authenticated model, connection and execution scope.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDescriptor {
    pub host_api: u32,
    pub canonical_schema: u32,
    pub rule_revision: String,
    pub channels: BTreeMap<String, ChannelDescriptor>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelDescriptor {
    pub provider: String,
    pub profile: String,
    pub wire_protocol: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityMode {
    Compatible,
    Strict,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, tag = "status", rename_all = "snake_case")]
pub enum PluginResult<T> {
    Ok { value: T },
    Error { error: AdapterError },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterError {
    pub code: String,
    pub reason_key: String,
    pub source_path: Option<String>,
    pub allowed_values: Option<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalOperation {
    pub schema_version: u32,
    pub kind: OperationKind,
    pub public_model: String,
    pub source_protocol: PublicProtocol,
    pub stream: bool,
    pub conversation: Vec<ConversationItem>,
    pub tools: Vec<FunctionDefinition>,
    pub parameters: Value,
    pub assets: Vec<AssetRef>,
    pub provenance: Vec<FieldProvenance>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Generate,
    GenerateImage,
    EditImage,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicProtocol {
    ChatCompletions,
    Responses,
    AnthropicMessages,
    Images,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationItem {
    pub role: String,
    pub content: Vec<ContentPart>,
    pub call_id: Option<String>,
    pub tool_name: Option<String>,
    pub arguments: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    Image { asset_handle: String },
    ReasoningText { text: String },
    OpaqueReasoning { source_channel: String, data: Value },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: Option<String>,
    pub parameters: Value,
    pub strict: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssetRef {
    pub handle: String,
    pub mime_type: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FieldProvenance {
    pub field: String,
    pub source_path: String,
    pub presence: FieldPresence,
    pub original_value: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalResultV2 {
    pub schema_version: u32,
    pub public_model: String,
    pub upstream_id: Option<String>,
    pub finish_reason: String,
    pub output: Vec<OutputItem>,
    pub usage: Option<CanonicalUsageV2>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "snake_case")]
pub enum OutputItem {
    Text {
        text: String,
    },
    ToolCall {
        call_id: String,
        name: String,
        arguments: Value,
    },
    ReasoningText {
        text: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalUsageV2 {
    pub input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_output_tokens: Option<u64>,
    pub raw: Value,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldPresence {
    Missing,
    Null,
    Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdaptationPlan {
    pub schema: u32,
    pub compatible: bool,
    pub rule_revision: String,
    pub target: AdaptationTarget,
    pub required_features: Vec<String>,
    pub changes: Vec<AdaptationChange>,
    pub rejected: Vec<AdapterError>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdaptationTarget {
    pub connection_id: String,
    pub connection_revision: u64,
    pub upstream_model: String,
    pub wire_protocol: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdaptationChange {
    pub source_path: String,
    pub target_path: String,
    pub kind: ChangeKind,
    pub requested: Value,
    pub effective: Value,
    pub rule_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Exact,
    Lossless,
    Mapped,
    OptionalNoop,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HttpIntent {
    pub action: HostAction,
    pub method: String,
    pub endpoint_id: String,
    pub relative_path: String,
    pub public_headers: Vec<(String, String)>,
    pub secret_bindings: Vec<SecretBinding>,
    pub body: Value,
    pub response_mode: ResponseMode,
    pub timeout_ms: u32,
    pub redirect_policy: RedirectPolicy,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostAction {
    Execute,
    DiscoverModels,
    AuthorizeCredential,
    RefreshCredential,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SecretBinding {
    pub slot: String,
    pub destination: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseMode {
    Json,
    Sse,
    Asset,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RedirectPolicy {
    Deny,
}

#[derive(Debug, Error)]
pub enum IntentError {
    #[error("plugin returned an invalid HTTP intent")]
    Invalid,
}

/// Structural validation only. Control must also resolve endpoint_id against
/// its connection profile, bind secrets, authorize the action, and enforce a
/// network target allowlist before dispatching this intent to Runner.
pub fn validate_http_intent(intent: &HttpIntent) -> Result<(), IntentError> {
    if !matches!(intent.method.as_str(), "GET" | "POST")
        || intent.endpoint_id.is_empty()
        || intent.endpoint_id.len() > 128
        || !intent
            .endpoint_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        || !intent.relative_path.starts_with('/')
        || intent.relative_path.starts_with("//")
        || intent.relative_path.len() > 2048
        || intent.relative_path.contains("..")
        || intent.relative_path.contains("://")
        || intent.relative_path.contains('#')
        || intent.relative_path.contains('\\')
        || intent.relative_path.chars().any(char::is_control)
        || intent.timeout_ms == 0
        || intent.timeout_ms > 120_000
        || intent.public_headers.len() > 32
        || intent.secret_bindings.len() > 8
    {
        return Err(IntentError::Invalid);
    }
    for (name, value) in &intent.public_headers {
        let lower = name.to_ascii_lowercase();
        if lower.is_empty()
            || lower.len() > 64
            || !lower
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || matches!(
                lower.as_str(),
                "authorization"
                    | "cookie"
                    | "host"
                    | "content-length"
                    | "transfer-encoding"
                    | "connection"
            )
            || value.len() > 8 * 1024
            || value.contains(&['\r', '\n'][..])
        {
            return Err(IntentError::Invalid);
        }
    }
    Ok(())
}
