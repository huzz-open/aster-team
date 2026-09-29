mod openai;
pub mod types;

use std::{collections::BTreeMap, sync::Arc};

use aster_runner_protocol::UpstreamHttpRequest;
use time::OffsetDateTime;

use crate::credential_broker::AuthLease;

pub use openai::OpenAiProvider;
pub use types::{
    AvailabilitySnapshot, CANONICAL_SCHEMA_VERSION, CanonicalError, CanonicalFailureScope,
    CanonicalOperation, CanonicalRequestStage, CanonicalResult, CanonicalStreamEvent,
    CanonicalUsage, CapabilityDescriptor, SchedulingConstraints,
};

pub trait ProviderAdapter: Send + Sync {
    fn provider_id(&self) -> &'static str;

    fn display_name(&self) -> &'static str;

    fn credential_kind(&self) -> &'static str;

    fn enrollment_actions(&self) -> &'static [&'static str];

    fn supports_refresh(&self) -> bool;

    fn execution_host(&self) -> &'static str;

    fn credential_requires_refresh(
        &self,
        auth_lease: &AuthLease,
        now: OffsetDateTime,
    ) -> Result<bool, CanonicalError>;

    fn capabilities(&self, public_models: Vec<String>) -> CapabilityDescriptor;

    fn prepare(
        &self,
        auth_lease: &AuthLease,
        operation: &CanonicalOperation,
        upstream_model: &str,
        request_id: &str,
    ) -> Result<UpstreamHttpRequest, CanonicalError>;

    fn parse_buffered_response(&self, body: &[u8]) -> Result<CanonicalResult, CanonicalError>;

    fn normalize_status(&self, status: u16) -> Option<CanonicalError>;
}

#[derive(Clone)]
pub struct ProviderRegistry {
    adapters: BTreeMap<&'static str, Arc<dyn ProviderAdapter>>,
}

impl ProviderRegistry {
    pub fn production() -> Self {
        let mut adapters = BTreeMap::<&'static str, Arc<dyn ProviderAdapter>>::new();
        let openai = Arc::new(OpenAiProvider);
        adapters.insert(openai.provider_id(), openai);
        Self { adapters }
    }

    pub fn get(&self, provider_id: &str) -> Option<&dyn ProviderAdapter> {
        self.adapters.get(provider_id).map(AsRef::as_ref)
    }

    pub fn provider_ids(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.adapters.keys().copied()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    struct StaticApiKeyTestProvider;

    impl ProviderAdapter for StaticApiKeyTestProvider {
        fn provider_id(&self) -> &'static str {
            "static-test"
        }
        fn display_name(&self) -> &'static str {
            "Static API key test provider"
        }
        fn credential_kind(&self) -> &'static str {
            "api_key"
        }
        fn enrollment_actions(&self) -> &'static [&'static str] {
            &["request_secret"]
        }
        fn supports_refresh(&self) -> bool {
            false
        }
        fn execution_host(&self) -> &'static str {
            "static.example.test"
        }
        fn credential_requires_refresh(
            &self,
            _: &AuthLease,
            _: OffsetDateTime,
        ) -> Result<bool, CanonicalError> {
            Ok(false)
        }
        fn capabilities(&self, public_models: Vec<String>) -> CapabilityDescriptor {
            CapabilityDescriptor {
                schema_version: CANONICAL_SCHEMA_VERSION,
                provider_id: self.provider_id().to_owned(),
                source: "static".to_owned(),
                version: "test-1".to_owned(),
                ttl_seconds: 0,
                operations: vec!["text".to_owned()],
                public_models,
            }
        }
        fn prepare(
            &self,
            _: &AuthLease,
            _: &CanonicalOperation,
            _: &str,
            _: &str,
        ) -> Result<UpstreamHttpRequest, CanonicalError> {
            unreachable!("conformance metadata test does not persist a test credential")
        }
        fn parse_buffered_response(&self, _: &[u8]) -> Result<CanonicalResult, CanonicalError> {
            Ok(CanonicalResult {
                schema_version: CANONICAL_SCHEMA_VERSION,
                response: json!({"ok": true}),
                usage: CanonicalUsage {
                    input_tokens: 0,
                    cached_input_tokens: 0,
                    output_tokens: 0,
                    provider_metadata: json!({}),
                },
            })
        }
        fn normalize_status(&self, status: u16) -> Option<CanonicalError> {
            (status >= 400).then(|| CanonicalError {
                schema_version: CANONICAL_SCHEMA_VERSION,
                code: "static_test_error".to_owned(),
                public_cause: "static_test_error".to_owned(),
                scope: CanonicalFailureScope::Provider,
                stage: CanonicalRequestStage::RejectedBeforeExecution,
                retryable: false,
                failover_eligible: false,
                invalidate_credential: false,
                billable: false,
            })
        }
    }

    #[test]
    fn production_registry_contains_only_real_provider_adapters() {
        let registry = ProviderRegistry::production();
        assert_eq!(registry.provider_ids().collect::<Vec<_>>(), vec!["openai"]);
    }

    #[test]
    fn api_key_provider_with_static_capabilities_and_no_refresh_conforms() {
        let provider = StaticApiKeyTestProvider;
        assert_eq!(provider.credential_kind(), "api_key");
        assert_eq!(provider.enrollment_actions(), &["request_secret"]);
        assert!(!provider.supports_refresh());
        assert_eq!(
            provider.capabilities(vec!["test-model".to_owned()]).source,
            "static"
        );
        assert!(provider.normalize_status(200).is_none());
        assert!(provider.normalize_status(401).is_some());
    }
}
