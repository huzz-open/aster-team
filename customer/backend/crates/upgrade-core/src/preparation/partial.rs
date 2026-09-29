//! Cancellation-only observations for startup that did not activate both services.
use super::*;

/// `None` means the adapter proved a loaded unit has no job, process or cgroup
/// tasks. It must never be inferred from a failed runtime request. Both missing
/// is permitted after a failed start; both present use CandidateActivation.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartialCandidateActivation {
    pub control: Option<RuntimeSnapshot>,
    pub runner: Option<ServiceInvocation>,
}

impl PartialCandidateActivation {
    pub(super) fn valid_for(
        &self,
        intent: &PreparationIntent,
        candidate: &ActiveReleaseSlot,
    ) -> bool {
        if self.control.is_some() && self.runner.is_some() {
            return false;
        }
        let Some(previous) = intent.previous_process.service.as_ref() else {
            return false;
        };
        let mut services = vec![previous, &intent.previous_runner_process];
        if let Some(control) = &self.control {
            if !startup::valid_control(control, intent, candidate) {
                return false;
            }
            let Some(service) = control.service.as_ref() else {
                return false;
            };
            services.push(service);
        }
        if let Some(runner) = &self.runner {
            services.push(runner);
        }
        startup::distinct_services(&services)
    }
}

impl PreparationJournal {
    pub(super) fn has_capture(&self) -> bool {
        self.activation.is_some() || self.partial_activation.is_some()
    }

    #[must_use]
    pub fn partial_activation(&self) -> Option<&PartialCandidateActivation> {
        self.partial_activation.as_ref()
    }

    /// Same immutable startup/material/boot contract as the complete activation,
    /// but exclusively owns cancellation. No synthetic Control/Runner identity.
    pub fn cancel_partial_startup(
        &self,
        observed: PartialCandidateActivation,
        material_sha256: &str,
        clock: &UpgradeClock,
    ) -> Option<Self> {
        let startup = self.startup.as_ref()?;
        if !self.valid()
            || self.abort.is_some()
            || self.has_capture()
            || startup.material_sha256 != material_sha256
            || clock.boot_id != startup.clock.boot_id
            || clock.uptime_ms < startup.clock.uptime_ms
        {
            return None;
        }
        let next = Self {
            revision: PreparationAbortPhase::CapturedForCancellation.revision(true)?,
            partial_activation: Some(observed),
            abort: Some(PreparationAbortPhase::CapturedForCancellation),
            ..self.clone()
        };
        next.valid().then_some(next)
    }

    /// Reobserve the same partial activation and every absent service before
    /// recording retirement permission. The adapter owns real absence proof.
    pub fn retiring_partial_candidate(
        &self,
        observed: &PartialCandidateActivation,
    ) -> Option<Self> {
        if !self.valid()
            || self.abort != Some(PreparationAbortPhase::Claimed)
            || self.partial_activation.as_ref() != Some(observed)
        {
            return None;
        }
        let next = Self {
            revision: PreparationAbortPhase::RetiringCandidate.revision(true)?,
            abort: Some(PreparationAbortPhase::RetiringCandidate),
            ..self.clone()
        };
        next.valid().then_some(next)
    }
}
