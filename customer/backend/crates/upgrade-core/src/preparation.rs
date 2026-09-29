//! Durable identity before candidate service startup. Persist the intent before
//! changing its slot, then persist the provisioned identity before starting it.
//! Cancellation retirement requires a durable capture and fresh process proof.
//! This record cannot grant admission or change License limits.
use crate::{
    ACTIVE_SLOT_RUNTIME_SCHEMA, ActiveReleaseSlot, MaintenanceJob, MaintenanceOperation,
    MaintenanceStatus, UpgradeMode,
    online::OnlinePlan,
    runtime::{ProxySnapshot, RuntimeSnapshot, ServiceInvocation, UpgradeClock},
};
use serde::{Deserialize, Serialize};

mod partial;
mod startup;
pub use partial::PartialCandidateActivation;
pub use startup::{CandidateActivation, CandidateStartup};

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreparationIntent {
    pub job: MaintenanceJob,
    pub previous: ActiveReleaseSlot,
    pub previous_process: RuntimeSnapshot,
    pub previous_runner_process: ServiceInvocation,
    pub candidate_manifest_sha256: String,
    pub migrations: crate::migration::MigrationPreflight,
    pub proxy: ProxySnapshot,
    pub clock: UpgradeClock,
}

impl PreparationIntent {
    #[must_use]
    pub fn valid(&self) -> bool {
        let previous = &self.previous_process;
        self.migrations.valid()
            && self.job.validate().is_ok()
            && self.job.status == MaintenanceStatus::StartingCandidate
            && self.job.upgrade_mode == Some(UpgradeMode::BlueGreen)
            && self.job.runner_was_running == Some(true)
            && matches!(self.job.operation, MaintenanceOperation::Upgrade { .. })
            && self
                .job
                .previous_release
                .as_ref()
                .is_some_and(|path| path.is_absolute())
            && self.job.candidate_release.as_ref().is_some_and(|path| {
                path.is_absolute() && Some(path) != self.job.previous_release.as_ref()
            })
            && self
                .job
                .target_version
                .as_ref()
                .is_some_and(|version| version != &self.job.current_version)
            && self.previous.validate().is_ok()
            && self.previous.schema == ACTIVE_SLOT_RUNTIME_SCHEMA
            && self.previous.version == self.job.current_version
            && previous.same_process(previous)
            && crate::valid_identifier(&previous.installation_id)
            && previous.slot == self.previous.slot
            && previous.product_version == self.previous.version
            && previous.lifecycle.accepting
            && !previous.lifecycle.stopping
            && previous.lifecycle.revision < u64::MAX
            && self.previous_runner_process.valid()
            && previous.service.as_ref().is_some_and(|control| {
                control.valid()
                    && control.process_id != self.previous_runner_process.process_id
                    && control.invocation_id != self.previous_runner_process.invocation_id
            })
            && crate::valid_checksum(&self.candidate_manifest_sha256)
            && self.proxy.slot == self.previous.slot
            && crate::valid_checksum(&self.proxy.configuration_sha256)
            && self.proxy.stream_close_delay_ms > 0
            && crate::valid_identifier(&self.clock.boot_id)
            && self
                .clock
                .uptime_ms
                .checked_add(self.proxy.stream_close_delay_ms)
                .is_some()
    }

    /// Mutable progress text/status does not change task ownership. Filesystem
    /// adapters additionally check the record's exact installation-scoped path.
    #[must_use]
    pub fn matches_job(&self, job: &MaintenanceJob) -> bool {
        self.valid()
            && job.validate().is_ok()
            && self.job.id == job.id
            && self.job.requested_by == job.requested_by
            && self.job.operation == job.operation
            && self.job.upgrade_mode == job.upgrade_mode
            && self.job.current_version == job.current_version
            && self.job.target_version == job.target_version
            && self.job.previous_release == job.previous_release
            && self.job.candidate_release == job.candidate_release
            && self.job.runner_was_running == job.runner_was_running
    }

    /// In-flight counts may naturally change while the old slot keeps serving.
    /// Reopening admission, rebooting or replacing either process is not resume.
    #[must_use]
    pub fn observes_previous(
        &self,
        live: &RuntimeSnapshot,
        runner: &ServiceInvocation,
        clock: &UpgradeClock,
    ) -> bool {
        self.valid()
            && live.same_process(&self.previous_process)
            && live.lifecycle.accepting
            && !live.lifecycle.stopping
            && live.lifecycle.revision == self.previous_process.lifecycle.revision
            && runner == &self.previous_runner_process
            && clock.boot_id == self.clock.boot_id
            && clock.uptime_ms >= self.clock.uptime_ms
    }
}

/// Cancellation owns either an unused provisioned candidate or an exactly
/// captured activation. Late observations can only enter cancellation; they
/// cannot restart the startup budget or authorize forward handoff.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreparationAbortPhase {
    CapturedForCancellation,
    Claimed,
    RetiringCandidate,
    Finalizing,
    Complete,
}

impl PreparationAbortPhase {
    fn revision(self, activated: bool) -> Option<u64> {
        match (activated, self) {
            (false, Self::CapturedForCancellation) => None,
            (true, Self::CapturedForCancellation) => Some(3),
            (false, Self::Claimed) => Some(2),
            (false, Self::RetiringCandidate) => None,
            (false, Self::Finalizing) => Some(3),
            (false, Self::Complete) => Some(4),
            (true, Self::Claimed) => Some(4),
            (true, Self::RetiringCandidate) => Some(5),
            (true, Self::Finalizing) => Some(6),
            (true, Self::Complete) => Some(7),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreparationJournal {
    schema: String,
    revision: u64,
    intent: PreparationIntent,
    candidate: Option<ActiveReleaseSlot>,
    startup: Option<CandidateStartup>,
    activation: Option<CandidateActivation>,
    partial_activation: Option<PartialCandidateActivation>,
    abort: Option<PreparationAbortPhase>,
}

impl PreparationJournal {
    pub fn create(intent: PreparationIntent) -> Option<Self> {
        intent.valid().then_some(Self {
            schema: "aster.online-preparation.v8".into(),
            revision: 0,
            intent,
            candidate: None,
            startup: None,
            activation: None,
            partial_activation: None,
            abort: None,
        })
    }

    #[must_use]
    pub fn intent(&self) -> &PreparationIntent {
        &self.intent
    }
    #[must_use]
    pub fn candidate(&self) -> Option<&ActiveReleaseSlot> {
        self.candidate.as_ref()
    }

    #[must_use]
    pub fn abort_phase(&self) -> Option<PreparationAbortPhase> {
        self.abort
    }

    fn abort_revision(&self, phase: PreparationAbortPhase) -> Option<u64> {
        phase
            .revision(self.has_capture())?
            .checked_sub(u64::from(self.candidate.is_none()))
    }

    /// Cancellation with no provisioned identity retains an unknown DB outcome;
    /// the adapter must still reconcile signed ownership before task archival.
    /// Persist before taking cancellation ownership; it cannot race startup.
    pub fn aborting(&self) -> Option<Self> {
        if !self.valid()
            || (self.startup.is_some() && self.activation.is_none())
            || self.abort.is_some()
        {
            return None;
        }
        let next = Self {
            revision: self.abort_revision(PreparationAbortPhase::Claimed)?,
            abort: Some(PreparationAbortPhase::Claimed),
            ..self.clone()
        };
        next.valid().then_some(next)
    }

    /// Capture a still-closed startup only for cancellation. Unlike `started`,
    /// this accepts an elapsed startup budget, but never a different boot,
    /// material or earlier clock. The adapter verifies actual signed inputs and
    /// observes both processes twice before persisting this irreversible choice.
    pub fn cancel_observed_startup(
        &self,
        activation: CandidateActivation,
        material_sha256: &str,
        clock: &UpgradeClock,
    ) -> Option<Self> {
        let startup = self.startup.as_ref()?;
        if !self.valid()
            || self.abort.is_some()
            || self.activation.is_some()
            || startup.material_sha256 != material_sha256
            || clock.boot_id != startup.clock.boot_id
            || clock.uptime_ms < startup.clock.uptime_ms
        {
            return None;
        }
        let next = Self {
            revision: PreparationAbortPhase::CapturedForCancellation.revision(true)?,
            activation: Some(activation),
            abort: Some(PreparationAbortPhase::CapturedForCancellation),
            ..self.clone()
        };
        next.valid().then_some(next)
    }

    /// The freshly read candidate must still be exactly the closed, empty
    /// activation. Persist this successor before issuing any retirement command.
    pub fn retiring_candidate(&self, observed: &RuntimeSnapshot) -> Option<Self> {
        if !self.valid()
            || self.abort != Some(PreparationAbortPhase::Claimed)
            || &self.activation.as_ref()?.control != observed
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

    /// The retirement phase durably attests a fresh exact observation of this
    /// closed activation. Finalizing/Complete retain it for repeated exit checks.
    #[must_use]
    pub fn abort_drained(&self) -> Option<&RuntimeSnapshot> {
        (self.valid()
            && matches!(
                self.abort,
                Some(
                    PreparationAbortPhase::RetiringCandidate
                        | PreparationAbortPhase::Finalizing
                        | PreparationAbortPhase::Complete
                )
            ))
        .then_some(())?;
        Some(&self.activation.as_ref()?.control)
    }

    /// Finalizing proves quiescent candidate and retained old service. Complete
    /// additionally proves the failure audit; only then may task archival run.
    pub fn advance_abort(&self) -> Option<Self> {
        if !self.valid() {
            return None;
        }
        let phase = match self.abort? {
            PreparationAbortPhase::CapturedForCancellation => PreparationAbortPhase::Claimed,
            PreparationAbortPhase::Claimed if !self.has_capture() => {
                PreparationAbortPhase::Finalizing
            }
            PreparationAbortPhase::RetiringCandidate => PreparationAbortPhase::Finalizing,
            PreparationAbortPhase::Claimed => return None,
            PreparationAbortPhase::Finalizing => PreparationAbortPhase::Complete,
            PreparationAbortPhase::Complete => return None,
        };
        let next = Self {
            revision: self.abort_revision(phase)?,
            abort: Some(phase),
            ..self.clone()
        };
        next.valid().then_some(next)
    }

    #[must_use]
    pub fn valid(&self) -> bool {
        let revision = if self.abort.is_some() {
            if self.has_capture() {
                3
            } else {
                u64::from(self.candidate.is_some())
            }
        } else {
            self.revision
        };
        self.schema == "aster.online-preparation.v8"
            && self.abort.is_none_or(|phase| {
                Some(self.revision) == self.abort_revision(phase)
                    && (self.startup.is_none() || self.has_capture())
            })
            && self.intent.valid()
            && self.partial_activation.as_ref().is_none_or(|partial| {
                self.abort.is_some()
                    && self.activation.is_none()
                    && self
                        .candidate
                        .as_ref()
                        .is_some_and(|candidate| partial.valid_for(&self.intent, candidate))
            })
            && match &self.candidate {
                None => revision == 0 && self.startup.is_none() && self.activation.is_none(),
                Some(candidate) => {
                    (1..=3).contains(&revision)
                        && candidate.validate().is_ok()
                        && candidate.schema == ACTIVE_SLOT_RUNTIME_SCHEMA
                        && candidate.slot == self.intent.previous.slot.other()
                        && Some(&candidate.version) == self.intent.job.target_version.as_ref()
                        && candidate.local_runner.as_ref().is_some_and(|runner| {
                            runner.manifest_sha256 == self.intent.candidate_manifest_sha256
                                && self
                                    .intent
                                    .previous
                                    .local_runner
                                    .as_ref()
                                    .is_some_and(|old| old.runner_id != runner.runner_id)
                        })
                }
            }
            && match (&self.startup, &self.activation) {
                (None, None) => revision <= 1,
                (Some(startup), None) => {
                    revision
                        == if self.partial_activation.is_some() {
                            3
                        } else {
                            2
                        }
                        && startup.valid_for(&self.intent)
                }
                (Some(startup), Some(activation)) => {
                    revision == 3
                        && startup.valid_for(&self.intent)
                        && self
                            .candidate
                            .as_ref()
                            .is_some_and(|candidate| activation.valid_for(&self.intent, candidate))
                }
                (None, Some(_)) => false,
            }
    }

    #[must_use]
    pub fn startup(&self) -> Option<&CandidateStartup> {
        self.startup.as_ref()
    }

    #[must_use]
    pub fn activation(&self) -> Option<&CandidateActivation> {
        self.activation.as_ref()
    }

    /// Callers persist this successor before issuing a service command.
    pub fn starting(&self, startup: CandidateStartup) -> Option<Self> {
        if !self.valid()
            || self.abort.is_some()
            || self.revision == 0
            || self.revision > 2
            || self.startup.as_ref().is_some_and(|saved| saved != &startup)
        {
            return None;
        }
        let next = Self {
            revision: 2,
            startup: Some(startup),
            ..self.clone()
        };
        next.valid().then_some(next)
    }

    /// A lost acknowledgement may repeat the exact activation, never replace it.
    pub fn started(
        &self,
        activation: CandidateActivation,
        material_sha256: &str,
        clock: &UpgradeClock,
    ) -> Option<Self> {
        if !self.valid()
            || self.abort.is_some()
            || self.revision < 2
            || !self
                .startup
                .as_ref()?
                .accepts_observation(material_sha256, clock)
            || self
                .activation
                .as_ref()
                .is_some_and(|saved| saved != &activation)
        {
            return None;
        }
        let next = Self {
            revision: 3,
            activation: Some(activation),
            ..self.clone()
        };
        next.valid().then_some(next)
    }

    /// Return the same record on a lost write acknowledgement. A different
    /// provisioned identity cannot silently replace a previously saved one.
    pub fn provisioned(&self, candidate: ActiveReleaseSlot) -> Option<Self> {
        if !self.valid()
            || self.abort.is_some()
            || self.startup.is_some()
            || self
                .candidate
                .as_ref()
                .is_some_and(|saved| saved != &candidate)
        {
            return None;
        }
        let next = Self {
            schema: self.schema.clone(),
            revision: 1,
            intent: self.intent.clone(),
            candidate: Some(candidate),
            startup: None,
            activation: None,
            partial_activation: None,
            abort: None,
        };
        next.valid().then_some(next)
    }

    #[must_use]
    pub fn follows(&self, previous: Option<&Self>) -> bool {
        self.valid()
            && match previous {
                None => self.revision == 0 && self.startup.is_none() && self.activation.is_none(),
                Some(previous) => {
                    previous.valid()
                        && self.intent == previous.intent
                        && match (previous.abort, self.abort) {
                            (None, _) => true,
                            (Some(old), Some(next)) => {
                                self.candidate == previous.candidate
                                    && self.startup == previous.startup
                                    && self.activation == previous.activation
                                    && self.partial_activation == previous.partial_activation
                                    && (old == next
                                        || previous
                                            .abort_revision(old)
                                            .and_then(|value| value.checked_add(1))
                                            == self.abort_revision(next))
                            }
                            (Some(_), None) => false,
                        }
                        && previous
                            .candidate
                            .as_ref()
                            .is_none_or(|saved| self.candidate.as_ref() == Some(saved))
                        && previous
                            .startup
                            .as_ref()
                            .is_none_or(|saved| self.startup.as_ref() == Some(saved))
                        && previous
                            .partial_activation
                            .as_ref()
                            .is_none_or(|saved| self.partial_activation.as_ref() == Some(saved))
                        && previous
                            .activation
                            .as_ref()
                            .is_none_or(|saved| self.activation.as_ref() == Some(saved))
                        && (self == previous
                            || previous.revision.checked_add(1) == Some(self.revision))
                }
            }
    }

    /// The caller must first persist OnlineJournal::create(plan). Only this
    /// exact handoff permits retiring the preparation record afterward.
    #[must_use]
    pub fn permits_handoff(&self, plan: &OnlinePlan) -> bool {
        self.valid()
            && self.abort.is_none()
            && plan.valid()
            && self.activation.as_ref().is_some_and(|activation| {
                activation.control == plan.candidate_process
                    && activation.runner == plan.candidate_runner_process
            })
            && self.startup.as_ref().is_some_and(|startup| {
                startup.accepts_observation(&startup.material_sha256, &plan.clock)
            })
            && self.candidate.as_ref() == Some(&plan.candidate)
            && plan.job_id == self.intent.job.id
            && plan.previous == self.intent.previous
            && self.intent.observes_previous(
                &plan.previous_process,
                &plan.previous_runner_process,
                &plan.clock,
            )
            && plan.readiness.manifest_sha256 == self.intent.candidate_manifest_sha256
            && plan.proxy == self.intent.proxy
    }
}

#[cfg(test)]
mod tests;
