use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CANONICAL_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, tag = "operation", rename_all = "snake_case")]
pub enum CanonicalOperation {
    Text(CanonicalTextOperation),
    ImageGeneration(CanonicalImageOperation),
    ImageEdit(CanonicalImageEditOperation),
}

impl CanonicalOperation {
    pub fn schema_version(&self) -> u32 {
        match self {
            Self::Text(operation) => operation.schema_version,
            Self::ImageGeneration(operation) => operation.schema_version,
            Self::ImageEdit(operation) => operation.schema_version,
        }
    }

    pub fn public_model(&self) -> &str {
        match self {
            Self::Text(operation) => &operation.public_model,
            Self::ImageGeneration(operation) => &operation.public_model,
            Self::ImageEdit(operation) => &operation.public_model,
        }
    }

    pub fn upstream_request(&self) -> &Value {
        match self {
            Self::Text(operation) => &operation.request,
            Self::ImageGeneration(operation) => &operation.request,
            Self::ImageEdit(operation) => &operation.request,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalTextOperation {
    pub schema_version: u32,
    pub public_model: String,
    pub stream: bool,
    /// A validated Responses-shaped document. Public protocol adapters are the
    /// only constructors in production; provider adapters may read but not
    /// reinterpret the originating public protocol.
    pub request: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalImageOperation {
    pub schema_version: u32,
    pub public_model: String,
    pub output_format: String,
    pub requested_count: u16,
    pub request: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalImageEditOperation {
    pub schema_version: u32,
    pub public_model: String,
    pub output_format: String,
    pub source_count: u16,
    pub has_mask: bool,
    pub request: Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalRequestStage {
    BeforeUpstream,
    RejectedBeforeExecution,
    ExecutionUnknown,
    ResponseStarted,
    Completed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalFailureScope {
    ClientRequest,
    Credential,
    Model,
    Provider,
    Runner,
    Transport,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalError {
    pub schema_version: u32,
    pub code: String,
    pub public_cause: String,
    pub scope: CanonicalFailureScope,
    pub stage: CanonicalRequestStage,
    pub retryable: bool,
    pub failover_eligible: bool,
    pub invalidate_credential: bool,
    pub billable: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalUsage {
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
    pub provider_metadata: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalResult {
    pub schema_version: u32,
    pub response: Value,
    pub usage: CanonicalUsage,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, tag = "event", rename_all = "snake_case")]
pub enum CanonicalStreamEvent {
    ResponseStarted { response_id: String },
    OutputDelta { index: u32, delta: String },
    Usage { usage: CanonicalUsage },
    Completed { result: CanonicalResult },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityDescriptor {
    pub schema_version: u32,
    pub provider_id: String,
    pub source: String,
    pub version: String,
    pub ttl_seconds: u32,
    pub operations: Vec<String>,
    pub public_models: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AvailabilitySnapshot {
    pub schema_version: u32,
    pub provider_id: String,
    pub credential_instance_id: String,
    pub available: bool,
    pub observed_at: String,
    pub expires_at: String,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulingConstraints {
    pub schema_version: u32,
    pub maximum_concurrency: Option<u32>,
    pub retry_after_seconds: Option<u32>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn canonical_operations_are_explicitly_versioned_and_tagged() {
        let operation = CanonicalOperation::ImageEdit(CanonicalImageEditOperation {
            schema_version: CANONICAL_SCHEMA_VERSION,
            public_model: "gpt-image-1.5".to_owned(),
            output_format: "png".to_owned(),
            source_count: 2,
            has_mask: true,
            request: json!({"model":"host-model"}),
        });
        let encoded = serde_json::to_value(&operation).expect("serialize canonical operation");
        assert_eq!(encoded["operation"], "image_edit");
        assert_eq!(operation.schema_version(), CANONICAL_SCHEMA_VERSION);
        assert_eq!(operation.public_model(), "gpt-image-1.5");
    }
}
