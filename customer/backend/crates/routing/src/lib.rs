#![forbid(unsafe_code)]

use aster_error_catalog::{ErrorDescriptor, routing as routing_errors};
use thiserror::Error;
use time::{Duration, OffsetDateTime};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CircuitState {
    Closed,
    Open { retry_at: OffsetDateTime },
    HalfOpen,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunnerSnapshot {
    pub runner_id: String,
    pub enabled: bool,
    pub protocol_version: u32,
    pub last_heartbeat_at: OffsetDateTime,
    pub inflight: u32,
    pub max_inflight: u32,
    pub recent_request_count: u32,
    pub recent_error_rate_ppm: u32,
    pub latency_ms: u32,
    pub circuit: CircuitState,
}

impl RunnerSnapshot {
    fn is_candidate(&self, required_protocol: u32, now: OffsetDateTime, timeout: Duration) -> bool {
        if !self.enabled
            || self.protocol_version != required_protocol
            || self.max_inflight == 0
            || self.inflight >= self.max_inflight
            || now - self.last_heartbeat_at > timeout
            || self.last_heartbeat_at > now + Duration::minutes(1)
        {
            return false;
        }
        match self.circuit {
            CircuitState::Closed => true,
            CircuitState::Open { retry_at } => now >= retry_at && self.inflight == 0,
            CircuitState::HalfOpen => self.inflight == 0,
        }
    }

    fn load_score(&self, now: OffsetDateTime) -> u128 {
        let utilization_ppm = u128::from(self.inflight) * 1_000_000 / u128::from(self.max_inflight);
        let heartbeat_age_ms = (now - self.last_heartbeat_at).whole_milliseconds().max(0) as u128;
        utilization_ppm * 1_000_000_000
            + u128::from(self.recent_request_count) * 10_000_000
            + u128::from(self.recent_error_rate_ppm) * 10_000
            + u128::from(self.latency_ms) * 100
            + heartbeat_age_ms.min(60_000)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionReason {
    LastSuccessfulAffinity,
    LowestRecentLoad,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunnerSelection {
    pub runner_id: String,
    pub reason: SelectionReason,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum RoutingError {
    #[error("no healthy Runner has compatible protocol and available capacity")]
    NoRunnerReady,
    #[error("the request can no longer switch Runner")]
    RetryForbidden,
}

impl RoutingError {
    pub const fn descriptor(&self) -> ErrorDescriptor {
        match self {
            Self::NoRunnerReady => routing_errors::RUNNER_NOT_READY,
            Self::RetryForbidden => routing_errors::RETRY_FORBIDDEN,
        }
    }
}

pub fn select_runner(
    runners: &[RunnerSnapshot],
    last_success_runner_id: Option<&str>,
    required_protocol: u32,
    now: OffsetDateTime,
    heartbeat_timeout: Duration,
) -> Result<RunnerSelection, RoutingError> {
    let candidates: Vec<_> = runners
        .iter()
        .filter(|runner| runner.is_candidate(required_protocol, now, heartbeat_timeout))
        .collect();
    if let Some(affinity) = last_success_runner_id
        && let Some(runner) = candidates
            .iter()
            .find(|runner| runner.runner_id == affinity)
    {
        return Ok(RunnerSelection {
            runner_id: runner.runner_id.clone(),
            reason: SelectionReason::LastSuccessfulAffinity,
        });
    }
    candidates
        .into_iter()
        .min_by_key(|runner| (runner.load_score(now), runner.runner_id.as_str()))
        .map(|runner| RunnerSelection {
            runner_id: runner.runner_id.clone(),
            reason: SelectionReason::LowestRecentLoad,
        })
        .ok_or(RoutingError::NoRunnerReady)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryPhase {
    Assigned,
    RunnerAccepted,
    UpstreamStarted,
    Streaming,
    Completed,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeliveryAttempt {
    phase: DeliveryPhase,
    switches_used: u8,
}

impl DeliveryAttempt {
    pub const fn new() -> Self {
        Self {
            phase: DeliveryPhase::Assigned,
            switches_used: 0,
        }
    }

    pub const fn phase(self) -> DeliveryPhase {
        self.phase
    }

    pub fn advance(&mut self, next: DeliveryPhase) -> Result<(), RoutingError> {
        let valid = matches!(
            (self.phase, next),
            (DeliveryPhase::Assigned, DeliveryPhase::RunnerAccepted)
                | (DeliveryPhase::Assigned, DeliveryPhase::Failed)
                | (
                    DeliveryPhase::RunnerAccepted,
                    DeliveryPhase::UpstreamStarted
                )
                | (DeliveryPhase::RunnerAccepted, DeliveryPhase::Failed)
                | (DeliveryPhase::UpstreamStarted, DeliveryPhase::Streaming)
                | (DeliveryPhase::UpstreamStarted, DeliveryPhase::Completed)
                | (DeliveryPhase::UpstreamStarted, DeliveryPhase::Failed)
                | (DeliveryPhase::Streaming, DeliveryPhase::Completed)
                | (DeliveryPhase::Streaming, DeliveryPhase::Failed)
        );
        if !valid {
            return Err(RoutingError::RetryForbidden);
        }
        self.phase = next;
        Ok(())
    }

    pub fn switch_runner(&mut self) -> Result<(), RoutingError> {
        if self.switches_used != 0
            || !matches!(
                self.phase,
                DeliveryPhase::Assigned | DeliveryPhase::RunnerAccepted
            )
        {
            return Err(RoutingError::RetryForbidden);
        }
        self.switches_used = 1;
        self.phase = DeliveryPhase::Assigned;
        Ok(())
    }
}

impl Default for DeliveryAttempt {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::*;

    fn runner(id: &str, inflight: u32, recent: u32) -> RunnerSnapshot {
        RunnerSnapshot {
            runner_id: id.to_owned(),
            enabled: true,
            protocol_version: 2,
            last_heartbeat_at: datetime!(2026-08-28 0:00 UTC),
            inflight,
            max_inflight: 10,
            recent_request_count: recent,
            recent_error_rate_ppm: 0,
            latency_ms: 20,
            circuit: CircuitState::Closed,
        }
    }

    #[test]
    fn keeps_last_successful_runner_when_it_is_still_healthy() {
        let now = datetime!(2026-08-28 0:00:10 UTC);
        let selected = select_runner(
            &[runner("runner-a", 8, 100), runner("runner-b", 0, 0)],
            Some("runner-a"),
            2,
            now,
            Duration::seconds(30),
        )
        .expect("select runner");
        assert_eq!(selected.runner_id, "runner-a");
        assert_eq!(selected.reason, SelectionReason::LastSuccessfulAffinity);
    }

    #[test]
    fn falls_back_to_the_lowest_recent_load_without_any_account_binding() {
        let now = datetime!(2026-08-28 0:00:10 UTC);
        let mut offline_affinity = runner("runner-a", 0, 0);
        offline_affinity.last_heartbeat_at = datetime!(2026-08-27 23:00 UTC);
        let selected = select_runner(
            &[
                offline_affinity,
                runner("runner-b", 2, 20),
                runner("runner-c", 0, 2),
            ],
            Some("runner-a"),
            2,
            now,
            Duration::seconds(30),
        )
        .expect("select runner");
        assert_eq!(selected.runner_id, "runner-c");
        assert_eq!(selected.reason, SelectionReason::LowestRecentLoad);
    }

    #[test]
    fn excludes_disabled_full_incompatible_and_open_circuit_runners() {
        let now = datetime!(2026-08-28 0:00:10 UTC);
        let mut disabled = runner("disabled", 0, 0);
        disabled.enabled = false;
        let mut full = runner("full", 10, 0);
        full.max_inflight = 10;
        let mut incompatible = runner("old", 0, 0);
        incompatible.protocol_version = 1;
        let mut open = runner("open", 0, 0);
        open.circuit = CircuitState::Open {
            retry_at: datetime!(2026-08-28 0:01 UTC),
        };
        assert_eq!(
            select_runner(
                &[disabled, full, incompatible, open],
                None,
                2,
                now,
                Duration::seconds(30),
            ),
            Err(RoutingError::NoRunnerReady)
        );
    }

    #[test]
    fn permits_one_switch_only_before_the_upstream_request_starts() {
        let mut attempt = DeliveryAttempt::new();
        attempt
            .advance(DeliveryPhase::RunnerAccepted)
            .expect("accept");
        attempt.switch_runner().expect("first switch");
        assert_eq!(attempt.switch_runner(), Err(RoutingError::RetryForbidden));

        let mut started = DeliveryAttempt::new();
        started
            .advance(DeliveryPhase::RunnerAccepted)
            .expect("accept");
        started
            .advance(DeliveryPhase::UpstreamStarted)
            .expect("start upstream");
        assert_eq!(started.switch_runner(), Err(RoutingError::RetryForbidden));
    }
}
