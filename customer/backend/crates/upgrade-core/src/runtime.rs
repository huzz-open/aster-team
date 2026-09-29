//! Executor-facing runtime contract. A transport error never proves a mutation
//! did not happen or that outstanding work finished.
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::ReleaseSlot;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct LifecycleSnapshot {
    pub accepting: bool,
    pub stopping: bool,
    pub revision: u64,
    pub in_flight: usize,
    pub oldest_request_age_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct RuntimeSnapshot {
    pub schema: String,
    pub installation_id: String,
    pub slot: ReleaseSlot,
    pub instance_id: String,
    #[serde(default)]
    pub service: Option<ServiceInvocation>,
    pub product_version: String,
    pub lifecycle: LifecycleSnapshot,
}

/// The running Control's HTTP request budget. Unresolved durable work may
/// outlive it: exhausting this wait budget must retain the old slot.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct RuntimeRequestBudget {
    #[serde(flatten)]
    pub runtime: RuntimeSnapshot,
    pub request_budget_ms: u64,
}

impl RuntimeRequestBudget {
    #[must_use]
    pub fn valid_for(&self, observed: &RuntimeSnapshot) -> bool {
        self.runtime.same_process(observed)
            && !self.runtime.lifecycle.stopping
            && self.runtime.lifecycle.revision == observed.lifecycle.revision
            && self.runtime.lifecycle.accepting == observed.lifecycle.accepting
            && self.request_budget_ms > 0
    }
}

impl RuntimeSnapshot {
    #[must_use]
    pub fn same_process(&self, other: &Self) -> bool {
        self.schema == "aster.control-runtime.v1"
            && other.schema == self.schema
            && !self.instance_id.is_empty()
            && self.installation_id == other.installation_id
            && self.slot == other.slot
            && self.instance_id == other.instance_id
            && self.service == other.service
            && self.product_version == other.product_version
    }

    /// The revision is the successful close-admission result, not a snapshot
    /// captured before closing or from a replacement process.
    #[must_use]
    pub fn observes_drain(&self, closed: &Self) -> bool {
        self.same_process(closed)
            && self.lifecycle.revision == closed.lifecycle.revision
            && !closed.lifecycle.accepting
            && !closed.lifecycle.stopping
            && !self.lifecycle.accepting
            && !self.lifecycle.stopping
    }

    #[must_use]
    pub fn observes_retirement(&self, drained: &Self) -> bool {
        self.same_process(drained)
            && !drained.lifecycle.accepting
            && !drained.lifecycle.stopping
            && drained.lifecycle.in_flight == 0
            && drained.lifecycle.revision.checked_add(1) == Some(self.lifecycle.revision)
            && !self.lifecycle.accepting
            && self.lifecycle.stopping
            && self.lifecycle.in_flight == 0
    }
}

/// systemd supplies a new invocation ID for each activation. PID alone can be
/// reused and must never identify an old process after executor recovery.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceInvocation {
    pub process_id: u32,
    pub invocation_id: String,
}

impl ServiceInvocation {
    #[must_use]
    pub fn valid(&self) -> bool {
        self.process_id > 0
            && self.invocation_id.len() == 32
            && self
                .invocation_id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            && self.invocation_id.bytes().any(|byte| byte != b'0')
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessProgress {
    Running,
    Stopping,
    Exited,
}

/// Only Exited proves normal termination of the pinned, drained Control.
/// Unknown, replaced, failed and missing units are errors, never exit evidence.
pub trait ControlProcessObserver {
    type Error;
    fn observe_control(
        &self,
        expected: &RuntimeSnapshot,
        deadline: Instant,
    ) -> Result<ProcessProgress, Self::Error>;
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessExpectation {
    pub manifest_sha256: String,
    pub models: Vec<String>,
    pub runner_ids: Vec<String>,
}

/// Complete configured model coverage, bound to the process and admission
/// revision that supplied it. Exceeding the bounded contract is an error;
/// callers must never turn a truncated inventory into readiness evidence.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessModelInventory {
    pub schema: String,
    pub installation_id: String,
    pub instance_id: String,
    pub slot: ReleaseSlot,
    pub revision: u64,
    pub models: Vec<String>,
}

impl ReadinessModelInventory {
    #[must_use]
    pub fn valid_models(models: &[String]) -> bool {
        valid_names(models, 32) && models.windows(2).all(|pair| pair[0] < pair[1])
    }

    #[must_use]
    pub fn valid_for(&self, observed: &RuntimeSnapshot) -> bool {
        self.schema == "aster.readiness-models.v1"
            && observed.schema == "aster.control-runtime.v1"
            && !self.installation_id.is_empty()
            && !self.instance_id.is_empty()
            && !observed.lifecycle.stopping
            && self.installation_id == observed.installation_id
            && self.instance_id == observed.instance_id
            && self.slot == observed.slot
            && self.revision == observed.lifecycle.revision
            && Self::valid_models(&self.models)
    }
}

impl ReadinessExpectation {
    #[must_use]
    pub fn valid(&self) -> bool {
        crate::valid_checksum(&self.manifest_sha256)
            && valid_names(&self.models, 32)
            && valid_names(&self.runner_ids, 8)
    }
}

fn valid_names(values: &[String], maximum: usize) -> bool {
    !values.is_empty()
        && values.len() <= maximum
        && values
            .iter()
            .all(|value| !value.is_empty() && value.len() <= 128)
        && values
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == values.len()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DrainProgress {
    Outstanding {
        count: usize,
        oldest_request_age_ms: u64,
    },
    Drained,
}

/// Implementations bind a fixed installation, slot and release. All operations
/// share the caller's monotonic deadline; a ready permit is consumed by value.
pub trait SlotRuntime {
    type Error;
    type ReadyPermit;

    fn status(&self, deadline: Instant) -> Result<RuntimeSnapshot, Self::Error>;
    fn readiness(
        &self,
        observed: &RuntimeSnapshot,
        expected: &ReadinessExpectation,
        deadline: Instant,
    ) -> Result<Self::ReadyPermit, Self::Error>;
    fn open_admission(
        &self,
        permit: Self::ReadyPermit,
        deadline: Instant,
    ) -> Result<RuntimeSnapshot, Self::Error>;
    fn close_admission(
        &self,
        observed: &RuntimeSnapshot,
        deadline: Instant,
    ) -> Result<RuntimeSnapshot, Self::Error>;
    fn observe_drain(
        &self,
        closed: &RuntimeSnapshot,
        deadline: Instant,
    ) -> Result<DrainProgress, Self::Error>;
}

/// Acknowledges a pinned, drained process entering irreversible shutdown. It
/// does not prove the OS process or its Runner exited. A lost reply requires
/// reconciliation; callers must not turn transport failure into exit evidence.
pub trait SlotRetirement: SlotRuntime {
    fn retire_drained(
        &self,
        drained: &RuntimeSnapshot,
        deadline: Instant,
    ) -> Result<RuntimeSnapshot, Self::Error>;
}

/// Public summary of live proxy state; configuration and concurrency evidence
/// remain owned by the transport. The deployment journal must pin the digest
/// before cutover and compare it again during recovery.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProxySnapshot {
    pub slot: ReleaseSlot,
    pub configuration_sha256: String,
    pub stream_close_delay_ms: u64,
}

/// Monotonic across executor restarts within one machine boot. Wall-clock time
/// is only for presentation; it cannot extend an online transition's budget.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpgradeClock {
    pub boot_id: String,
    pub uptime_ms: u64,
}

pub trait UpgradeClockSource {
    type Error;
    fn sample() -> Result<UpgradeClock, Self::Error>;
}

pub trait SlotProxy: Sized {
    type Error;
    type Observation;

    fn connect() -> Result<Self, Self::Error>;
    fn observe(&self, deadline: Instant) -> Result<Self::Observation, Self::Error>;
    fn snapshot(observed: &Self::Observation) -> &ProxySnapshot;
    /// Observe disk and live state without repairing either. The caller must
    /// hold the installation lock and verify this is the current durable journal.
    fn observe_transition(
        &self,
        journal: &crate::online::OnlineJournal,
        deadline: Instant,
    ) -> Result<Self::Observation, Self::Error>;
    /// Observe the reverse operation under its own durable phase, while the
    /// original forward journal remains frozen by the installation lock.
    fn observe_switchback(
        &self,
        journal: &crate::online::switchback::SwitchbackJournal,
        deadline: Instant,
    ) -> Result<Self::Observation, Self::Error>;
    /// Consume the original concurrency evidence once. A failed call may have
    /// applied the switch; do not replay it without another observation.
    fn switch_to(
        &self,
        observed: Self::Observation,
        target: ReleaseSlot,
        required_stream_delay_ms: u64,
        deadline: Instant,
    ) -> Result<Self::Observation, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed() -> RuntimeSnapshot {
        RuntimeSnapshot {
            schema: "aster.control-runtime.v1".into(),
            installation_id: "installation".into(),
            slot: ReleaseSlot::Blue,
            instance_id: "process-one".into(),
            service: None,
            product_version: "2.1.0".into(),
            lifecycle: LifecycleSnapshot {
                accepting: false,
                stopping: false,
                revision: 4,
                in_flight: 1,
                oldest_request_age_ms: 12,
            },
        }
    }

    #[test]
    fn drain_observation_rejects_restarts_reopens_and_shutdown() {
        let initial = closed();
        let mut next = initial.clone();
        next.lifecycle.in_flight = 0;
        assert!(next.observes_drain(&initial));
        next.instance_id = "process-two".into();
        assert!(!next.observes_drain(&initial));
        next = initial.clone();
        next.lifecycle.revision += 1;
        assert!(!next.observes_drain(&initial));
        next = initial.clone();
        next.lifecycle.accepting = true;
        assert!(!next.observes_drain(&initial));
        next = initial.clone();
        next.lifecycle.stopping = true;
        assert!(!next.observes_drain(&initial));
    }

    #[test]
    fn service_identity_requires_a_real_pid_and_nonzero_activation_and_is_part_of_process_binding()
    {
        let valid = ServiceInvocation {
            process_id: 42,
            invocation_id: "a".repeat(32),
        };
        assert!(valid.valid());
        for invalid in [
            ServiceInvocation {
                process_id: 0,
                ..valid.clone()
            },
            ServiceInvocation {
                invocation_id: "0".repeat(32),
                ..valid.clone()
            },
            ServiceInvocation {
                invocation_id: "a".repeat(31),
                ..valid.clone()
            },
            ServiceInvocation {
                invocation_id: "g".repeat(32),
                ..valid.clone()
            },
        ] {
            assert!(!invalid.valid());
        }
        let mut original = closed();
        original.service = Some(valid.clone());
        let mut replacement = original.clone();
        replacement.service.as_mut().unwrap().invocation_id = "b".repeat(32);
        assert!(!replacement.same_process(&original));
        replacement.service = Some(ServiceInvocation {
            process_id: 43,
            ..valid
        });
        assert!(!replacement.same_process(&original));
        replacement.service = None;
        assert!(!replacement.same_process(&original));
    }

    #[test]
    fn readiness_requires_a_bounded_nonempty_distinct_coverage_set() {
        let mut expected = ReadinessExpectation {
            manifest_sha256: "a".repeat(64),
            models: vec!["model".into()],
            runner_ids: vec!["runner".into()],
        };
        assert!(expected.valid());
        expected.models.push("model".into());
        assert!(!expected.valid());
        expected.models.clear();
        assert!(!expected.valid());
        expected.models = vec!["model".into()];
        expected.runner_ids = vec!["r".into(); 9];
        assert!(!expected.valid());
    }

    #[test]
    fn discovered_models_require_complete_canonical_coverage_and_the_original_process() {
        let observed = closed();
        let inventory = ReadinessModelInventory {
            schema: "aster.readiness-models.v1".into(),
            installation_id: observed.installation_id.clone(),
            instance_id: observed.instance_id.clone(),
            slot: observed.slot,
            revision: observed.lifecycle.revision,
            models: vec!["a".into(), "z".into()],
        };
        assert!(inventory.valid_for(&observed));
        for models in [
            vec![],
            vec!["a".into(), "a".into()],
            vec!["z".into(), "a".into()],
            (0..33).map(|n| format!("model-{n:02}")).collect(),
            vec!["x".repeat(129)],
        ] {
            let mut changed = inventory.clone();
            changed.models = models;
            assert!(!changed.valid_for(&observed));
        }
        let mut changed = inventory.clone();
        changed.revision += 1;
        assert!(!changed.valid_for(&observed));
        changed = inventory.clone();
        changed.installation_id.push_str("-other");
        assert!(!changed.valid_for(&observed));
        changed = inventory.clone();
        changed.instance_id.push_str("-replacement");
        assert!(!changed.valid_for(&observed));
        changed = inventory;
        changed.slot = observed.slot.other();
        assert!(!changed.valid_for(&observed));
    }
}
