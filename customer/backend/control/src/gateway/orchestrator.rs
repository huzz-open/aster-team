use aster_policy_core::billing::plan::{ImageSpec, ReferenceRequest};
use aster_routing::RoutingError;
use aster_storage::GatewayRouteCandidate;

use crate::{
    BufferedRunnerResponse, ControlError, ControlState, FailedModelRequest, GatewayRequestIds,
    ModelConsumer, ModelExecution, ModelUsage, QuotaReservation, execute_provider_request,
    providers::{CanonicalError, CanonicalOperation, CanonicalRequestStage, ProviderRegistry},
};

/// Owns policies that span public protocols and upstream providers. Protocol
/// adapters validate/translate; provider adapters encode/decode; this type is
/// the single place that reserves and settles quota and interprets provider
/// retry/failover metadata.
pub(crate) struct RequestOrchestrator<'a> {
    state: &'a ControlState,
    authorized: &'a ModelConsumer,
    request_id: &'a GatewayRequestIds,
}

impl<'a> RequestOrchestrator<'a> {
    pub(crate) fn new(
        state: &'a ControlState,
        authorized: &'a ModelConsumer,
        request_id: &'a GatewayRequestIds,
    ) -> Self {
        Self {
            state,
            authorized,
            request_id,
        }
    }

    pub(crate) async fn reserve(
        &self,
        public_model: &str,
        tier: &str,
        reference: &ReferenceRequest,
    ) -> Result<QuotaReservation, ControlError> {
        let mut reservation = self
            .state
            .reserve_model_money(
                self.authorized,
                self.request_id,
                public_model,
                tier,
                reference,
            )
            .await?;
        reservation.client_request_id = self.request_id.client_request_id.clone();
        Ok(reservation)
    }

    pub(crate) async fn execute_provider(
        &self,
        reservation: &QuotaReservation,
        public_model: &str,
        operation: &CanonicalOperation,
    ) -> Result<(GatewayRouteCandidate, BufferedRunnerResponse), ControlError> {
        let execution = ModelExecution::new(
            self.authorized,
            reservation,
            self.request_id,
            public_model,
            operation,
        );
        execute_provider_request(self.state, &execution).await
    }

    pub(crate) async fn fail(
        &self,
        reservation: &QuotaReservation,
        failure: &FailedModelRequest,
        upstream_unknown: bool,
    ) -> Result<(), ControlError> {
        let outcome = if upstream_unknown {
            super::durable_settlement::Outcome::FailureWithUnknown(failure.clone())
        } else {
            super::durable_settlement::Outcome::Failure(failure.clone())
        };
        super::durable_settlement::complete(self.state, reservation, outcome).await
    }

    pub(crate) async fn settle(
        &self,
        reservation: &QuotaReservation,
        usage: &ModelUsage,
    ) -> Result<(), ControlError> {
        super::durable_settlement::complete(
            self.state,
            reservation,
            super::durable_settlement::Outcome::Usage(usage.clone()),
        )
        .await
    }

    pub(crate) async fn settle_images(
        &self,
        reservation: &QuotaReservation,
        usage: &ModelUsage,
        confirmed_count: u32,
        spec: ImageSpec,
    ) -> Result<(), ControlError> {
        super::durable_settlement::complete(
            self.state,
            reservation,
            super::durable_settlement::Outcome::ImageUsage {
                usage: usage.clone(),
                confirmed_count,
                spec,
            },
        )
        .await
    }

    pub(crate) fn normalize_provider_status(
        provider_id: &str,
        status: u16,
    ) -> Option<CanonicalError> {
        ProviderRegistry::production()
            .get(provider_id)
            .and_then(|provider| provider.normalize_status(status))
    }

    pub(crate) fn stage_for_control_error(error: &ControlError) -> CanonicalRequestStage {
        match error {
            ControlError::ModelAccountNotReady
            | ControlError::ModelAttemptStale
            | ControlError::ExternalApiKeyInvalid
            | ControlError::Policy(_)
            | ControlError::LicenseMissing
            | ControlError::LicenseState(_)
            // Within provider execution these errors originate in preparation,
            // authorization or ticket construction, before any model dispatch.
            // Transport ambiguity uses RetryForbidden instead.
            | ControlError::DataIntegrityInvalid
            | ControlError::Storage(_)
            | ControlError::StorageFailure(_, _)
            | ControlError::StorageNotConfigured
            | ControlError::TimeConversionFailed
            | ControlError::RandomnessUnavailable
            | ControlError::RunnerTaskIssuerNotConfigured
            | ControlError::TaskExecutionFailed(_)
            | ControlError::AuditReadConflict
            | ControlError::Auth(_)
            | ControlError::RunnerTicket(_)
            | ControlError::LicenseInvalid(_)
            | ControlError::MachineMismatch(_)
            | ControlError::QuotaSettlementConflict
            | ControlError::GatewayRequestInvalid
            | ControlError::ModelNotFound
            | ControlError::ModelAccessDenied
            | ControlError::CredentialNotFound
            | ControlError::CredentialInvalid
            | ControlError::UpstreamRequestFailed
            | ControlError::Routing(RoutingError::NoRunnerReady) => {
                CanonicalRequestStage::BeforeUpstream
            }
            ControlError::InvalidUpstreamResponse
            | ControlError::UpstreamResponseTooLarge
            | ControlError::UpstreamUsageInvalid => CanonicalRequestStage::ResponseStarted,
            ControlError::Routing(RoutingError::RetryForbidden) => {
                CanonicalRequestStage::ExecutionUnknown
            }
            _ => CanonicalRequestStage::ExecutionUnknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::providers::{CanonicalFailureScope, CanonicalRequestStage};

    use super::*;

    #[test]
    fn provider_status_policy_keeps_request_stage_and_failure_scope() {
        let error = RequestOrchestrator::normalize_provider_status("openai", 429)
            .expect("rate limit must be classified");
        assert_eq!(error.scope, CanonicalFailureScope::Provider);
        assert_eq!(error.stage, CanonicalRequestStage::RejectedBeforeExecution);
        assert!(error.retryable);
        assert!(error.failover_eligible);
    }

    #[test]
    fn transport_stage_distinguishes_safe_failover_from_unknown_execution() {
        assert_eq!(
            RequestOrchestrator::stage_for_control_error(&ControlError::UpstreamRequestFailed),
            CanonicalRequestStage::BeforeUpstream,
        );
        assert_eq!(
            RequestOrchestrator::stage_for_control_error(&ControlError::Routing(
                RoutingError::RetryForbidden,
            )),
            CanonicalRequestStage::ExecutionUnknown,
        );
        assert_eq!(
            RequestOrchestrator::stage_for_control_error(&ControlError::InvalidUpstreamResponse,),
            CanonicalRequestStage::ResponseStarted,
        );
    }
}
