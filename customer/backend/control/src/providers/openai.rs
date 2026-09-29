use aster_runner_protocol::{UpstreamHttpRequest, UpstreamRequestHeader, encode_base64url};
use serde_json::{json, to_vec};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use zeroize::Zeroizing;

use crate::{
    ControlError, OPENAI_CODEX_RESPONSES_ENDPOINT,
    credential_broker::AuthLease,
    decode_openai_credential,
    gateway::responses::{CodexUpstreamAdapter, reduce_sse},
};

use super::{
    ProviderAdapter,
    types::{
        CANONICAL_SCHEMA_VERSION, CanonicalError, CanonicalFailureScope, CanonicalOperation,
        CanonicalRequestStage, CanonicalResult, CanonicalUsage, CapabilityDescriptor,
    },
};

pub struct OpenAiProvider;

impl OpenAiProvider {
    fn error(
        code: &str,
        scope: CanonicalFailureScope,
        stage: CanonicalRequestStage,
        retryable: bool,
        failover_eligible: bool,
        invalidate_credential: bool,
    ) -> CanonicalError {
        CanonicalError {
            schema_version: CANONICAL_SCHEMA_VERSION,
            code: code.to_owned(),
            public_cause: code.to_owned(),
            scope,
            stage,
            retryable,
            failover_eligible,
            invalidate_credential,
            billable: false,
        }
    }

    fn preparation_error(_: ControlError) -> CanonicalError {
        Self::error(
            "provider_request_invalid",
            CanonicalFailureScope::Provider,
            CanonicalRequestStage::BeforeUpstream,
            false,
            false,
            false,
        )
    }

    fn response_error(error: ControlError) -> CanonicalError {
        let code = match error {
            ControlError::UpstreamUsageInvalid => "provider_usage_invalid",
            ControlError::UpstreamResponseTooLarge => "provider_response_too_large",
            _ => "provider_response_invalid",
        };
        Self::error(
            code,
            CanonicalFailureScope::Provider,
            CanonicalRequestStage::ResponseStarted,
            false,
            false,
            false,
        )
    }
}

impl ProviderAdapter for OpenAiProvider {
    fn provider_id(&self) -> &'static str {
        "openai"
    }

    fn display_name(&self) -> &'static str {
        "OpenAI"
    }

    fn credential_kind(&self) -> &'static str {
        "oauth_refreshable"
    }

    fn enrollment_actions(&self) -> &'static [&'static str] {
        &["open_url", "submit_callback"]
    }

    fn supports_refresh(&self) -> bool {
        true
    }

    fn execution_host(&self) -> &'static str {
        "chatgpt.com"
    }

    fn credential_requires_refresh(
        &self,
        auth_lease: &AuthLease,
        now: OffsetDateTime,
    ) -> Result<bool, CanonicalError> {
        let credential =
            decode_openai_credential(auth_lease.material()).map_err(Self::preparation_error)?;
        if credential.account_id != auth_lease.upstream_subject_id() {
            return Err(Self::preparation_error(ControlError::CredentialInvalid));
        }
        let expires_at = OffsetDateTime::parse(&credential.expires_at, &Rfc3339)
            .map_err(|_| Self::preparation_error(ControlError::CredentialInvalid))?;
        Ok(expires_at <= now + time::Duration::minutes(2))
    }

    fn capabilities(&self, public_models: Vec<String>) -> CapabilityDescriptor {
        CapabilityDescriptor {
            schema_version: CANONICAL_SCHEMA_VERSION,
            provider_id: self.provider_id().to_owned(),
            source: "static_and_model_mapping".to_owned(),
            version: "1".to_owned(),
            ttl_seconds: 300,
            operations: vec![
                "text".to_owned(),
                "image_generation".to_owned(),
                "image_edit".to_owned(),
            ],
            public_models,
        }
    }

    fn prepare(
        &self,
        auth_lease: &AuthLease,
        operation: &CanonicalOperation,
        upstream_model: &str,
        request_id: &str,
    ) -> Result<UpstreamHttpRequest, CanonicalError> {
        if auth_lease.provider_id() != self.provider_id()
            || auth_lease.upstream_host() != "chatgpt.com"
            || operation.schema_version() != CANONICAL_SCHEMA_VERSION
        {
            return Err(Self::preparation_error(ControlError::CredentialInvalid));
        }
        let credential =
            decode_openai_credential(auth_lease.material()).map_err(Self::preparation_error)?;
        if credential.account_id != auth_lease.upstream_subject_id() {
            return Err(Self::preparation_error(ControlError::CredentialInvalid));
        }
        let provider_body =
            CodexUpstreamAdapter::encode(operation.upstream_request(), upstream_model)
                .map_err(Self::preparation_error)?;
        let body = Zeroizing::new(
            to_vec(&provider_body)
                .map_err(|_| Self::preparation_error(ControlError::GatewayRequestInvalid))?,
        );
        Ok(UpstreamHttpRequest {
            method: "POST".to_owned(),
            url: OPENAI_CODEX_RESPONSES_ENDPOINT.to_owned(),
            headers: vec![
                UpstreamRequestHeader {
                    name: "authorization".to_owned(),
                    value: format!("Bearer {}", credential.access_token),
                },
                UpstreamRequestHeader {
                    name: "chatgpt-account-id".to_owned(),
                    value: credential.account_id.clone(),
                },
                UpstreamRequestHeader {
                    name: "content-type".to_owned(),
                    value: "application/json".to_owned(),
                },
                UpstreamRequestHeader {
                    name: "accept".to_owned(),
                    value: "text/event-stream".to_owned(),
                },
                UpstreamRequestHeader {
                    name: "originator".to_owned(),
                    value: "Aster Team".to_owned(),
                },
                UpstreamRequestHeader {
                    name: "user-agent".to_owned(),
                    value: "Aster Team/0.1".to_owned(),
                },
                UpstreamRequestHeader {
                    name: "openai-beta".to_owned(),
                    value: "responses_websockets=2026-02-06".to_owned(),
                },
                UpstreamRequestHeader {
                    name: "x-client-request-id".to_owned(),
                    value: request_id.to_owned(),
                },
            ],
            body_base64url: encode_base64url(&body),
        })
    }

    fn parse_buffered_response(&self, body: &[u8]) -> Result<CanonicalResult, CanonicalError> {
        let (response, usage) = reduce_sse(body).map_err(Self::response_error)?;
        Ok(CanonicalResult {
            schema_version: CANONICAL_SCHEMA_VERSION,
            response,
            usage: CanonicalUsage {
                input_tokens: usage.uncached_input,
                cached_input_tokens: usage.cached_input,
                output_tokens: usage.output_tokens,
                provider_metadata: json!({
                    "cache_write_tokens": usage.cache_write,
                    "multiplier_micros": usage.multiplier_micros,
                }),
            },
        })
    }

    fn normalize_status(&self, status: u16) -> Option<CanonicalError> {
        match status {
            200..=299 => None,
            400 => Some(Self::error(
                "provider_request_rejected",
                CanonicalFailureScope::ClientRequest,
                CanonicalRequestStage::RejectedBeforeExecution,
                false,
                false,
                false,
            )),
            401 => Some(Self::error(
                "provider_credential_expired",
                CanonicalFailureScope::Credential,
                CanonicalRequestStage::RejectedBeforeExecution,
                true,
                true,
                true,
            )),
            403 => Some(Self::error(
                "provider_permission_denied",
                CanonicalFailureScope::Credential,
                CanonicalRequestStage::RejectedBeforeExecution,
                false,
                true,
                false,
            )),
            404 => Some(Self::error(
                "provider_model_not_found",
                CanonicalFailureScope::Model,
                CanonicalRequestStage::RejectedBeforeExecution,
                false,
                true,
                false,
            )),
            409 => Some(Self::error(
                "provider_conflict",
                CanonicalFailureScope::Provider,
                CanonicalRequestStage::RejectedBeforeExecution,
                false,
                false,
                false,
            )),
            429 => Some(Self::error(
                "provider_rate_limited",
                CanonicalFailureScope::Provider,
                CanonicalRequestStage::RejectedBeforeExecution,
                true,
                true,
                false,
            )),
            500..=599 => Some(Self::error(
                "provider_unavailable",
                CanonicalFailureScope::Provider,
                CanonicalRequestStage::ExecutionUnknown,
                false,
                false,
                false,
            )),
            _ => Some(Self::error(
                "provider_response_invalid",
                CanonicalFailureScope::Provider,
                CanonicalRequestStage::RejectedBeforeExecution,
                false,
                false,
                false,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_classes_do_not_collapse_credentials_permissions_and_limits() {
        let provider = OpenAiProvider;
        let unauthorized = provider.normalize_status(401).expect("401 classification");
        let forbidden = provider.normalize_status(403).expect("403 classification");
        let limited = provider.normalize_status(429).expect("429 classification");
        assert!(unauthorized.invalidate_credential);
        assert!(!forbidden.invalidate_credential);
        assert!(limited.retryable);
        let unavailable = provider.normalize_status(503).expect("503 classification");
        assert_eq!(unavailable.stage, CanonicalRequestStage::ExecutionUnknown);
        assert!(!unavailable.retryable);
        assert!(!unavailable.failover_eligible);
        assert_ne!(unauthorized.code, forbidden.code);
        assert_ne!(forbidden.code, limited.code);
    }

    #[test]
    fn response_parsing_preserves_untrusted_usage_as_a_started_response_failure() {
        let error = OpenAiProvider
            .parse_buffered_response(b"data: {\"type\":\"response.completed\",\"response\":{}}\n\n")
            .expect_err("missing usage must fail");
        assert_eq!(error.code, "provider_usage_invalid");
        assert_eq!(error.stage, CanonicalRequestStage::ResponseStarted);
        assert!(!error.retryable);
        assert!(!error.failover_eligible);
    }
}
