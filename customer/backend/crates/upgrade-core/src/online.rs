//! Recoverable forward cutover. This does not enable the product's online mode:
//! platform preparation, rollback and the durable deployment adapter are separate.
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::{
    ACTIVE_SLOT_RUNTIME_SCHEMA, ActiveReleaseSlot,
    runtime::{
        ControlProcessObserver, ProcessProgress, ProxySnapshot, ReadinessExpectation,
        RuntimeSnapshot, ServiceInvocation, SlotRetirement, UpgradeClock,
    },
};

const JOURNAL_SCHEMA: &str = "aster.online-transition.v2";

pub mod switchback;

/// Created only after signed staging, compatible migrations and closed candidate
/// startup. The caller derives the budget from the old runtime's request limits;
/// this module does not assume that all requests finish within five minutes.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OnlinePlan {
    pub job_id: String,
    pub previous: ActiveReleaseSlot,
    pub candidate: ActiveReleaseSlot,
    pub previous_process: RuntimeSnapshot,
    pub candidate_process: RuntimeSnapshot,
    /// Captured from the slot services before cutover and persisted for recovery.
    /// Runner IDs bind credentials; they cannot identify a process activation.
    pub previous_runner_process: ServiceInvocation,
    pub candidate_runner_process: ServiceInvocation,
    pub readiness: ReadinessExpectation,
    pub proxy: ProxySnapshot,
    pub clock: UpgradeClock,
    pub drain_budget_ms: u64,
}

impl OnlinePlan {
    fn distinct_service_invocations(&self) -> bool {
        let (Some(previous), Some(candidate)) = (
            self.previous_process.service.as_ref(),
            self.candidate_process.service.as_ref(),
        ) else {
            return false;
        };
        let services = [
            previous,
            candidate,
            &self.previous_runner_process,
            &self.candidate_runner_process,
        ];
        services.iter().enumerate().all(|(index, service)| {
            service.valid()
                && services[..index].iter().all(|other| {
                    service.process_id != other.process_id
                        && service.invocation_id != other.invocation_id
                })
        })
    }

    #[must_use]
    pub fn valid(&self) -> bool {
        let pair_valid = |release: &ActiveReleaseSlot, process: &RuntimeSnapshot| {
            release.validate().is_ok()
                && release.schema == ACTIVE_SLOT_RUNTIME_SCHEMA
                && process.same_process(process)
                && !process.installation_id.is_empty()
                && release.slot == process.slot
                && release.version == process.product_version
                && !process.lifecycle.stopping
                && process
                    .service
                    .as_ref()
                    .is_some_and(|service| service.valid())
                && process.lifecycle.revision < u64::MAX
        };
        crate::valid_identifier(&self.job_id)
            && pair_valid(&self.previous, &self.previous_process)
            && pair_valid(&self.candidate, &self.candidate_process)
            && self.previous.slot != self.candidate.slot
            && self.previous_process.installation_id == self.candidate_process.installation_id
            && self.previous_process.instance_id != self.candidate_process.instance_id
            && self.distinct_service_invocations()
            && self.previous_process.lifecycle.accepting
            && !self.candidate_process.lifecycle.accepting
            && self.candidate_process.lifecycle.in_flight == 0
            && self.drain_budget_ms > 0
            && self.proxy.slot == self.previous.slot
            && crate::valid_checksum(&self.proxy.configuration_sha256)
            && self.proxy.stream_close_delay_ms > self.drain_budget_ms
            && crate::valid_identifier(&self.clock.boot_id)
            && self
                .clock
                .uptime_ms
                .checked_add(self.proxy.stream_close_delay_ms)
                .is_some()
            && self.readiness.valid()
            && self.candidate.local_runner.as_ref().is_some_and(|runner| {
                self.readiness.manifest_sha256 == runner.manifest_sha256
                    && self.readiness.runner_ids.contains(&runner.runner_id)
                    && self
                        .previous
                        .local_runner
                        .as_ref()
                        .is_some_and(|previous| previous.runner_id != runner.runner_id)
            })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OnlinePhase {
    OpeningCandidate,
    SwitchingTraffic,
    ClosingPrevious,
    DrainingPrevious,
    DrainBudgetExhausted,
    RetiringPrevious,
    Committing,
    Complete,
}

/// No bearer token or readiness permit is serialized. Fields are private so a
/// caller cannot bypass the ordering rules by editing an in-memory phase.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OnlineJournal {
    schema: String,
    revision: u64,
    plan: OnlinePlan,
    phase: OnlinePhase,
    opened: Option<RuntimeSnapshot>,
    closed: Option<RuntimeSnapshot>,
    cutover_started_at_ms: Option<u64>,
    drain_started_at_ms: Option<u64>,
    drained: Option<RuntimeSnapshot>,
    exhausted: Option<RuntimeSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OnlineProgress {
    Advanced(OnlinePhase),
    Draining {
        count: usize,
        oldest_request_age_ms: u64,
    },
    DrainBudgetExhausted,
    RetiringPrevious,
    Complete,
}

#[derive(Debug, thiserror::Error)]
pub enum OnlineError<R, D> {
    #[error("online transition journal is invalid")]
    InvalidJournal,
    #[error("online transition deadline has elapsed")]
    Deadline,
    #[error("online transition clock moved backwards or overflowed")]
    Clock,
    #[error("machine boot changed; online continuity cannot be recovered as uninterrupted")]
    BootChanged,
    #[error(
        "proxy retention window cannot cover cutover and drain; preserve both slots for recovery"
    )]
    ProxyWindowElapsed,
    #[error("online runtime identity or admission state changed")]
    RuntimeChanged,
    #[error("traffic does not match either pinned runtime")]
    TrafficChanged,
    #[error("runtime operation failed; observe actual state before retrying: {0}")]
    Runtime(R),
    #[error("deployment operation failed; reload durable state before retrying: {0}")]
    Deployment(D),
    #[error("Control retirement could not finish: {0}")]
    Retirement(crate::retirement::RetirementError<R, D>),
}

pub type OnlineResult<T, R, D> = Result<T, OnlineError<R, D>>;

/// Durable storage is separate from proxy/process actions so file persistence
/// and recovery guards use exactly the same schema and CAS rules as cutover.
pub trait OnlineJournalStorage {
    type Error;

    fn load(&self) -> Result<Option<OnlineJournal>, Self::Error>;
    fn assert_current(&mut self, journal: &OnlineJournal) -> Result<(), Self::Error>;
    fn save(
        &mut self,
        previous: Option<&OnlineJournal>,
        next: &OnlineJournal,
    ) -> Result<(), Self::Error>;
}

/// Adapter requirements: hold the installation's exclusive upgrade lock for the
/// entire operation; check/replace the journal atomically with fsync; honor the
/// shared deadline. A returned error may mean an operation already happened.
///
/// `traffic` must observe every business entry in the live proxy, not merely the
/// intended config file, and fail if API, Member and Admin disagree.
/// Switches must preserve existing responses and Runner connections. Retirement
/// must target the pinned process and Runner only, recheck closed/zero state, and
/// reject a replacement process; an already stopped *pinned* pair is idempotent.
/// Commit must durably update all release pointers/audit before reporting success.
/// The CLI's maintenance recovery cannot be used as this adapter's recovery.
pub trait OnlineDeployment: OnlineJournalStorage {
    fn traffic(&mut self, deadline: Instant) -> Result<RuntimeSnapshot, Self::Error>;
    fn switch_to(
        &mut self,
        candidate: &RuntimeSnapshot,
        deadline: Instant,
    ) -> Result<(), Self::Error>;
    fn observe_control(
        &self,
        drained: &RuntimeSnapshot,
        deadline: Instant,
    ) -> Result<ProcessProgress, Self::Error>;
    /// Called only after the core proves Control exited. Observe/retire the
    /// pinned Runner without disabling either unit. Only Exited advances the
    /// durable journal; service disablement belongs to the Committing phase.
    fn retire(
        &mut self,
        journal: &OnlineJournal,
        deadline: Instant,
    ) -> Result<ProcessProgress, Self::Error>;
    fn commit(&mut self, journal: &OnlineJournal, deadline: Instant) -> Result<(), Self::Error>;
}

struct DeploymentProcess<'a, D>(&'a D);
impl<D: OnlineDeployment> ControlProcessObserver for DeploymentProcess<'_, D> {
    type Error = D::Error;
    fn observe_control(
        &self,
        drained: &RuntimeSnapshot,
        deadline: Instant,
    ) -> Result<ProcessProgress, Self::Error> {
        self.0.observe_control(drained, deadline)
    }
}

impl OnlineJournal {
    pub fn create<D: OnlineJournalStorage>(
        plan: OnlinePlan,
        deployment: &mut D,
    ) -> OnlineResult<Self, std::convert::Infallible, D::Error> {
        let journal = Self {
            schema: JOURNAL_SCHEMA.into(),
            revision: 0,
            plan,
            phase: OnlinePhase::OpeningCandidate,
            opened: None,
            closed: None,
            cutover_started_at_ms: None,
            drain_started_at_ms: None,
            drained: None,
            exhausted: None,
        };
        if !journal.valid() {
            return Err(OnlineError::InvalidJournal);
        }
        deployment
            .save(None, &journal)
            .map_err(OnlineError::Deployment)?;
        Ok(journal)
    }

    #[must_use]
    pub const fn phase(&self) -> OnlinePhase {
        self.phase
    }

    #[must_use]
    pub const fn plan(&self) -> &OnlinePlan {
        &self.plan
    }

    #[must_use]
    pub const fn drained(&self) -> Option<&RuntimeSnapshot> {
        self.drained.as_ref()
    }

    /// Last observed outstanding count at budget exhaustion, not a drain proof.
    #[must_use]
    pub const fn exhausted(&self) -> Option<&RuntimeSnapshot> {
        self.exhausted.as_ref()
    }

    /// Validate independent disk/live observations against durable phase intent.
    /// This is not a substitute for the caller's journal CAS, installation lock,
    /// boot/deadline checks or pinned Control identity checks.
    #[must_use]
    pub fn accepts_proxy(&self, live: &ProxySnapshot, disk: crate::ReleaseSlot) -> bool {
        if !self.valid()
            || live.configuration_sha256 != self.plan.proxy.configuration_sha256
            || live.stream_close_delay_ms != self.plan.proxy.stream_close_delay_ms
        {
            return false;
        }
        let previous = self.plan.previous.slot;
        let candidate = self.plan.candidate.slot;
        match self.phase {
            OnlinePhase::OpeningCandidate => live.slot == previous && disk == previous,
            OnlinePhase::SwitchingTraffic => {
                // Disk intent precedes the live CAS. Either acknowledgement can
                // be lost, but live-new/disk-old is never a legitimate outcome.
                (live.slot == previous && (disk == previous || disk == candidate))
                    || (live.slot == candidate && disk == candidate)
            }
            OnlinePhase::ClosingPrevious
            | OnlinePhase::DrainingPrevious
            | OnlinePhase::DrainBudgetExhausted
            | OnlinePhase::RetiringPrevious
            | OnlinePhase::Committing
            | OnlinePhase::Complete => live.slot == candidate && disk == candidate,
        }
    }

    /// A storage adapter must reject stale or rewritten history even if both
    /// individual records are structurally valid.
    #[must_use]
    pub fn follows(&self, previous: Option<&Self>) -> bool {
        if !self.valid() {
            return false;
        }
        let Some(previous) = previous else {
            return self.phase == OnlinePhase::OpeningCandidate;
        };
        previous.valid()
            && self.plan == previous.plan
            && previous.revision.checked_add(1) == Some(self.revision)
            && matches!(
                (previous.phase, self.phase),
                (OnlinePhase::OpeningCandidate, OnlinePhase::SwitchingTraffic)
                    | (OnlinePhase::SwitchingTraffic, OnlinePhase::ClosingPrevious)
                    | (OnlinePhase::ClosingPrevious, OnlinePhase::DrainingPrevious)
                    | (
                        OnlinePhase::DrainingPrevious,
                        OnlinePhase::RetiringPrevious | OnlinePhase::DrainBudgetExhausted
                    )
                    | (OnlinePhase::RetiringPrevious, OnlinePhase::Committing)
                    | (OnlinePhase::Committing, OnlinePhase::Complete)
            )
            && previous
                .opened
                .as_ref()
                .is_none_or(|value| self.opened.as_ref() == Some(value))
            && previous
                .closed
                .as_ref()
                .is_none_or(|value| self.closed.as_ref() == Some(value))
            && previous
                .drained
                .as_ref()
                .is_none_or(|value| self.drained.as_ref() == Some(value))
            && previous
                .drain_started_at_ms
                .is_none_or(|value| self.drain_started_at_ms == Some(value))
            && previous
                .cutover_started_at_ms
                .is_none_or(|value| self.cutover_started_at_ms == Some(value))
    }

    #[must_use]
    pub fn valid(&self) -> bool {
        let expected_revision = match self.phase {
            OnlinePhase::OpeningCandidate => 0,
            OnlinePhase::SwitchingTraffic => 1,
            OnlinePhase::ClosingPrevious => 2,
            OnlinePhase::DrainingPrevious => 3,
            OnlinePhase::DrainBudgetExhausted | OnlinePhase::RetiringPrevious => 4,
            OnlinePhase::Committing => 5,
            OnlinePhase::Complete => 6,
        };
        if self.schema != JOURNAL_SCHEMA || !self.plan.valid() || self.revision != expected_revision
        {
            return false;
        }
        let opened = self
            .opened
            .as_ref()
            .is_some_and(|value| changed_admission(value, &self.plan.candidate_process, true));
        let closed = self
            .closed
            .as_ref()
            .is_some_and(|value| changed_admission(value, &self.plan.previous_process, false));
        let drained = self.drained.as_ref().is_some_and(|value| {
            self.closed
                .as_ref()
                .is_some_and(|closed| value.observes_drain(closed))
                && value.lifecycle.in_flight == 0
        });
        let cutover_valid = self.cutover_started_at_ms.is_some_and(|start| {
            start >= self.plan.clock.uptime_ms
                && start
                    .checked_add(self.plan.proxy.stream_close_delay_ms)
                    .is_some()
        });
        let clock_valid = self.drain_started_at_ms.is_some_and(|start| {
            self.cutover_started_at_ms
                .is_some_and(|cutover| start >= cutover)
                && start
                    .checked_add(self.plan.drain_budget_ms)
                    .is_some_and(|drain_end| {
                        self.cutover_started_at_ms
                            .and_then(|cutover| {
                                cutover.checked_add(self.plan.proxy.stream_close_delay_ms)
                            })
                            .is_some_and(|proxy_end| drain_end <= proxy_end)
                    })
        });
        let exhaustion_valid = if self.phase == OnlinePhase::DrainBudgetExhausted {
            self.exhausted.as_ref().is_some_and(|value| {
                self.closed
                    .as_ref()
                    .is_some_and(|closed| value.observes_drain(closed))
            })
        } else {
            self.exhausted.is_none()
        };
        if !exhaustion_valid {
            return false;
        }
        match self.phase {
            OnlinePhase::OpeningCandidate => {
                self.revision == 0
                    && self.opened.is_none()
                    && self.closed.is_none()
                    && self.cutover_started_at_ms.is_none()
                    && self.drain_started_at_ms.is_none()
                    && self.drained.is_none()
            }
            OnlinePhase::SwitchingTraffic => {
                opened
                    && cutover_valid
                    && self.closed.is_none()
                    && self.drain_started_at_ms.is_none()
                    && self.drained.is_none()
            }
            OnlinePhase::ClosingPrevious => {
                opened
                    && cutover_valid
                    && self.closed.is_none()
                    && clock_valid
                    && self.drained.is_none()
            }
            OnlinePhase::DrainingPrevious | OnlinePhase::DrainBudgetExhausted => {
                opened && closed && cutover_valid && clock_valid && self.drained.is_none()
            }
            OnlinePhase::RetiringPrevious | OnlinePhase::Committing | OnlinePhase::Complete => {
                opened && closed && cutover_valid && clock_valid && drained
            }
        }
    }

    fn checkpoint<R, D: OnlineDeployment>(
        &mut self,
        deployment: &mut D,
        phase: OnlinePhase,
        update: impl FnOnce(&mut Self),
    ) -> OnlineResult<OnlineProgress, R, D::Error> {
        let mut next = self.clone();
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or(OnlineError::InvalidJournal)?;
        next.phase = phase;
        update(&mut next);
        if !next.follows(Some(self)) {
            return Err(OnlineError::InvalidJournal);
        }
        deployment
            .save(Some(self), &next)
            .map_err(OnlineError::Deployment)?;
        *self = next;
        Ok(OnlineProgress::Advanced(phase))
    }

    fn cutover_deadline<R, D>(
        &self,
        now_ms: u64,
        started: Instant,
        deadline: Instant,
    ) -> OnlineResult<Instant, R, D> {
        let cutover = self
            .cutover_started_at_ms
            .ok_or(OnlineError::InvalidJournal)?;
        let latest = cutover
            .checked_add(self.plan.proxy.stream_close_delay_ms - self.plan.drain_budget_ms)
            .ok_or(OnlineError::Clock)?;
        let remaining = latest
            .checked_sub(now_ms)
            .filter(|value| *value > 0)
            .ok_or(OnlineError::ProxyWindowElapsed)?;
        let limit = started
            .checked_add(std::time::Duration::from_millis(remaining))
            .ok_or(OnlineError::Clock)?;
        if Instant::now() >= limit {
            return Err(OnlineError::ProxyWindowElapsed);
        }
        Ok(deadline.min(limit))
    }

    /// One bounded transition, without sleeping or automatic mutation retries.
    /// On any uncertain write, reload the durable journal before calling again.
    /// A drain timeout is durable and preserves both slots, never a forced stop.
    pub fn advance<R: SlotRetirement, D: OnlineDeployment>(
        &mut self,
        previous: &R,
        candidate: &R,
        deployment: &mut D,
        clock: &UpgradeClock,
        deadline: Instant,
    ) -> OnlineResult<OnlineProgress, R::Error, D::Error> {
        let started = Instant::now();
        if !self.valid() {
            return Err(OnlineError::InvalidJournal);
        }
        if Instant::now() >= deadline {
            return Err(OnlineError::Deadline);
        }
        deployment
            .assert_current(self)
            .map_err(OnlineError::Deployment)?;
        if self.phase == OnlinePhase::Complete {
            return Ok(OnlineProgress::Complete);
        }
        if self.phase == OnlinePhase::DrainBudgetExhausted {
            return Ok(OnlineProgress::DrainBudgetExhausted);
        }
        if clock.boot_id != self.plan.clock.boot_id {
            return Err(OnlineError::BootChanged);
        }
        let now_ms = clock.uptime_ms;
        if now_ms < self.plan.clock.uptime_ms
            || self
                .cutover_started_at_ms
                .is_some_and(|start| now_ms < start)
        {
            return Err(OnlineError::Clock);
        }
        if self
            .drain_started_at_ms
            .is_some_and(|started| now_ms < started)
        {
            return Err(OnlineError::Clock);
        }
        // Reserve the full drain budget before any switch/close command. The
        // anchor was saved before the first possible proxy mutation and is not
        // reset when that mutation's reply is lost or the executor restarts.
        let deadline = if self.phase == OnlinePhase::SwitchingTraffic {
            self.cutover_deadline(now_ms, started, deadline)?
        } else {
            deadline
        };
        let traffic = deployment
            .traffic(deadline)
            .map_err(OnlineError::Deployment)?;
        let on_previous = traffic.same_process(&self.plan.previous_process);
        let on_candidate = traffic.same_process(&self.plan.candidate_process);
        if !on_previous && !on_candidate {
            return Err(OnlineError::TrafficChanged);
        }
        if self.phase != OnlinePhase::OpeningCandidate
            && self.phase != OnlinePhase::SwitchingTraffic
            && !on_candidate
        {
            return Err(OnlineError::TrafficChanged);
        }
        let candidate_status = candidate.status(deadline).map_err(OnlineError::Runtime)?;
        if !candidate_status.same_process(&self.plan.candidate_process)
            || candidate_status.lifecycle.stopping
        {
            return Err(OnlineError::RuntimeChanged);
        }
        if self.phase != OnlinePhase::OpeningCandidate
            && !changed_admission(&candidate_status, &self.plan.candidate_process, true)
        {
            return Err(OnlineError::RuntimeChanged);
        }
        match self.phase {
            OnlinePhase::OpeningCandidate => {
                if !on_previous {
                    return Err(OnlineError::TrafficChanged);
                }
                let old = previous.status(deadline).map_err(OnlineError::Runtime)?;
                if !unchanged_admission(&old, &self.plan.previous_process) {
                    return Err(OnlineError::RuntimeChanged);
                }
                let opened =
                    if changed_admission(&candidate_status, &self.plan.candidate_process, true) {
                        // A prior open may have succeeded despite a missing response.
                        candidate_status
                    } else {
                        if !unchanged_admission(&candidate_status, &self.plan.candidate_process) {
                            return Err(OnlineError::RuntimeChanged);
                        }
                        let permit = candidate
                            .readiness(&candidate_status, &self.plan.readiness, deadline)
                            .map_err(OnlineError::Runtime)?;
                        let result = candidate
                            .open_admission(permit, deadline)
                            .map_err(OnlineError::Runtime)?;
                        if !changed_admission(&result, &self.plan.candidate_process, true) {
                            return Err(OnlineError::RuntimeChanged);
                        }
                        result
                    };
                self.checkpoint(deployment, OnlinePhase::SwitchingTraffic, |next| {
                    next.opened = Some(opened);
                    next.cutover_started_at_ms = Some(now_ms);
                })
            }
            OnlinePhase::SwitchingTraffic => {
                // Recheck business readiness after recovery, including a lost
                // switch reply. Never replay an old in-memory permit.
                let _permit = candidate
                    .readiness(&candidate_status, &self.plan.readiness, deadline)
                    .map_err(OnlineError::Runtime)?;
                if on_previous {
                    let old = previous.status(deadline).map_err(OnlineError::Runtime)?;
                    if !unchanged_admission(&old, &self.plan.previous_process) {
                        return Err(OnlineError::RuntimeChanged);
                    }
                    deployment
                        .switch_to(&candidate_status, deadline)
                        .map_err(OnlineError::Deployment)?;
                    if !deployment
                        .traffic(deadline)
                        .map_err(OnlineError::Deployment)?
                        .same_process(&candidate_status)
                    {
                        return Err(OnlineError::TrafficChanged);
                    }
                }
                now_ms
                    .checked_add(self.plan.drain_budget_ms)
                    .ok_or(OnlineError::Clock)?;
                self.checkpoint(deployment, OnlinePhase::ClosingPrevious, |next| {
                    next.drain_started_at_ms = Some(now_ms)
                })
            }
            OnlinePhase::ClosingPrevious => {
                let old = previous.status(deadline).map_err(OnlineError::Runtime)?;
                let closed = if changed_admission(&old, &self.plan.previous_process, false) {
                    old
                } else {
                    if !unchanged_admission(&old, &self.plan.previous_process) {
                        return Err(OnlineError::RuntimeChanged);
                    }
                    let deadline = self.cutover_deadline(now_ms, started, deadline)?;
                    let result = previous
                        .close_admission(&old, deadline)
                        .map_err(OnlineError::Runtime)?;
                    if !changed_admission(&result, &self.plan.previous_process, false) {
                        return Err(OnlineError::RuntimeChanged);
                    }
                    result
                };
                self.checkpoint(deployment, OnlinePhase::DrainingPrevious, |next| {
                    next.closed = Some(closed)
                })
            }
            OnlinePhase::DrainingPrevious => {
                let start = self
                    .drain_started_at_ms
                    .ok_or(OnlineError::InvalidJournal)?;
                let old = previous.status(deadline).map_err(OnlineError::Runtime)?;
                if !self
                    .closed
                    .as_ref()
                    .is_some_and(|closed| old.observes_drain(closed))
                {
                    return Err(OnlineError::RuntimeChanged);
                }
                // Include this step's network time instead of accepting a zero
                // result observed after the persisted budget already expired.
                let step_ms =
                    u64::try_from(started.elapsed().as_millis()).map_err(|_| OnlineError::Clock)?;
                let observed_at_ms = now_ms.checked_add(step_ms).ok_or(OnlineError::Clock)?;
                let elapsed = now_ms
                    .checked_sub(start)
                    .and_then(|elapsed| elapsed.checked_add(step_ms))
                    .ok_or(OnlineError::Clock)?;
                let proxy_end = self
                    .cutover_started_at_ms
                    .and_then(|start| start.checked_add(self.plan.proxy.stream_close_delay_ms))
                    .ok_or(OnlineError::InvalidJournal)?;
                if elapsed >= self.plan.drain_budget_ms || observed_at_ms >= proxy_end {
                    return self.checkpoint(
                        deployment,
                        OnlinePhase::DrainBudgetExhausted,
                        |next| {
                            next.exhausted = Some(old);
                        },
                    );
                }
                if old.lifecycle.in_flight > 0 {
                    return Ok(OnlineProgress::Draining {
                        count: old.lifecycle.in_flight,
                        oldest_request_age_ms: old.lifecycle.oldest_request_age_ms,
                    });
                }
                self.checkpoint(deployment, OnlinePhase::RetiringPrevious, |next| {
                    next.drained = Some(old)
                })
            }
            OnlinePhase::RetiringPrevious => {
                let _permit = candidate
                    .readiness(&candidate_status, &self.plan.readiness, deadline)
                    .map_err(OnlineError::Runtime)?;
                let progress = crate::retirement::retire_control(
                    previous,
                    &DeploymentProcess(&*deployment),
                    self.drained.as_ref().ok_or(OnlineError::InvalidJournal)?,
                    deadline,
                )
                .map_err(OnlineError::Retirement)?;
                if progress == crate::retirement::ControlRetirementProgress::Waiting {
                    return Ok(OnlineProgress::RetiringPrevious);
                }
                if deployment
                    .retire(self, deadline)
                    .map_err(OnlineError::Deployment)?
                    != ProcessProgress::Exited
                {
                    return Ok(OnlineProgress::RetiringPrevious);
                }
                self.checkpoint(deployment, OnlinePhase::Committing, |_| {})
            }
            OnlinePhase::Committing => {
                deployment
                    .commit(self, deadline)
                    .map_err(OnlineError::Deployment)?;
                self.checkpoint(deployment, OnlinePhase::Complete, |_| {})
            }
            OnlinePhase::Complete | OnlinePhase::DrainBudgetExhausted => {
                unreachable!("terminal phases returned above")
            }
        }
    }
}

fn unchanged_admission(current: &RuntimeSnapshot, initial: &RuntimeSnapshot) -> bool {
    current.same_process(initial)
        && !current.lifecycle.stopping
        && current.lifecycle.accepting == initial.lifecycle.accepting
        && current.lifecycle.revision == initial.lifecycle.revision
}

fn changed_admission(
    current: &RuntimeSnapshot,
    initial: &RuntimeSnapshot,
    accepting: bool,
) -> bool {
    current.same_process(initial)
        && !current.lifecycle.stopping
        && current.lifecycle.accepting == accepting
        && initial.lifecycle.revision.checked_add(1) == Some(current.lifecycle.revision)
}

#[cfg(test)]
mod tests;
