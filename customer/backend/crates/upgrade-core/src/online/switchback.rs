//! Reverse routing is a separate durable operation bound to the original
//! forward journal. The losing candidate may still own requests throughout.
use super::*;
use crate::runtime::RuntimeRequestBudget;
use std::time::Duration;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SwitchbackPlan {
    pub original: OnlineJournal,
    pub previous: RuntimeSnapshot,
    pub candidate: RuntimeSnapshot,
    pub candidate_budget: RuntimeRequestBudget,
    pub proxy: ProxySnapshot,
    pub clock: UpgradeClock,
}

impl SwitchbackPlan {
    #[must_use]
    pub fn valid(&self) -> bool {
        let original = &self.original;
        original.valid()
            && matches!(
                original.phase,
                OnlinePhase::SwitchingTraffic
                    | OnlinePhase::ClosingPrevious
                    | OnlinePhase::DrainingPrevious
                    | OnlinePhase::DrainBudgetExhausted
            )
            && original
                .opened
                .as_ref()
                .is_some_and(|opened| unchanged_admission(&self.candidate, opened))
            && original.closed.as_ref().map_or_else(
                || {
                    unchanged_admission(&self.previous, &original.plan.previous_process)
                        || changed_admission(&self.previous, &original.plan.previous_process, false)
                },
                |closed| unchanged_admission(&self.previous, closed),
            )
            && self.previous.lifecycle.revision < u64::MAX - 1
            && self.candidate.lifecycle.revision < u64::MAX - 1
            && self.candidate_budget.valid_for(&self.candidate)
            && self.proxy.slot == original.plan.candidate.slot
            && self.proxy.configuration_sha256 == original.plan.proxy.configuration_sha256
            && self.proxy.stream_close_delay_ms == original.plan.proxy.stream_close_delay_ms
            && self.proxy.stream_close_delay_ms > self.candidate_budget.request_budget_ms
            && self.clock.boot_id == original.plan.clock.boot_id
            && self.clock.uptime_ms >= original.plan.clock.uptime_ms
            && original
                .cutover_started_at_ms
                .is_none_or(|start| self.clock.uptime_ms >= start)
            && original
                .drain_started_at_ms
                .is_none_or(|start| self.clock.uptime_ms >= start)
            && self
                .clock
                .uptime_ms
                .checked_add(self.proxy.stream_close_delay_ms)
                .is_some()
    }

    #[must_use]
    pub fn previous_readiness(&self) -> ReadinessExpectation {
        let runner = self.original.plan.previous.local_runner.as_ref();
        ReadinessExpectation {
            manifest_sha256: runner.map_or_else(String::new, |r| r.manifest_sha256.clone()),
            runner_ids: runner.map_or_else(Vec::new, |r| vec![r.runner_id.clone()]),
            models: self.original.plan.readiness.models.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SwitchbackPhase {
    PreparingPrevious,
    SwitchingBack,
    ClosingCandidate,
    DrainingCandidate,
    RetiringCandidate,
    CommittingPrevious,
    Complete,
    DrainBudgetExhausted,
}

impl SwitchbackPhase {
    fn revision(self) -> u64 {
        match self {
            Self::PreparingPrevious => 0,
            Self::SwitchingBack => 1,
            Self::ClosingCandidate => 2,
            Self::DrainingCandidate => 3,
            Self::RetiringCandidate | Self::DrainBudgetExhausted => 4,
            Self::CommittingPrevious => 5,
            Self::Complete => 6,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SwitchbackJournal {
    schema: String,
    revision: u64,
    plan: SwitchbackPlan,
    phase: SwitchbackPhase,
    opened: Option<RuntimeSnapshot>,
    closed: Option<RuntimeSnapshot>,
    drained: Option<RuntimeSnapshot>,
    switch_started_ms: Option<u64>,
    drain_started_ms: Option<u64>,
}

/// The adapter must atomically claim the original forward journal before the
/// first save, and reject any subsequent forward advancement. Hold the same
/// installation lock through observations, CAS writes and process operations.
pub trait SwitchbackJournalStorage {
    type Error;
    fn load(&self) -> Result<Option<SwitchbackJournal>, Self::Error>;
    fn assert_current(&mut self, journal: &SwitchbackJournal) -> Result<(), Self::Error>;
    fn save(
        &mut self,
        previous: Option<&SwitchbackJournal>,
        next: &SwitchbackJournal,
    ) -> Result<(), Self::Error>;
}

pub trait SwitchbackDeployment: SwitchbackJournalStorage {
    fn observe_control(
        &self,
        drained: &RuntimeSnapshot,
        deadline: Instant,
    ) -> Result<ProcessProgress, Self::Error>;
    /// Observe routing independently of the losing candidate's reachability.
    fn traffic(
        &mut self,
        journal: &SwitchbackJournal,
        deadline: Instant,
    ) -> Result<crate::ReleaseSlot, Self::Error>;
    fn switch_back(
        &mut self,
        journal: &SwitchbackJournal,
        deadline: Instant,
    ) -> Result<(), Self::Error>;
    /// Only called after the candidate Control's normal exit is proved.
    fn retire_candidate_runner(
        &mut self,
        journal: &SwitchbackJournal,
        deadline: Instant,
    ) -> Result<ProcessProgress, Self::Error>;
    /// Commit the old slot and failed upgrade outcome without database rollback.
    fn commit_previous(
        &mut self,
        journal: &SwitchbackJournal,
        deadline: Instant,
    ) -> Result<(), Self::Error>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SwitchbackProgress {
    Advanced(SwitchbackPhase),
    Draining {
        count: usize,
        oldest_request_age_ms: u64,
    },
    Retiring,
    DrainBudgetExhausted,
    Complete,
}

impl SwitchbackJournal {
    pub fn create<D: SwitchbackJournalStorage>(
        plan: SwitchbackPlan,
        deployment: &mut D,
    ) -> OnlineResult<Self, std::convert::Infallible, D::Error> {
        let next = Self {
            schema: "aster.online-switchback.v1".into(),
            revision: 0,
            plan,
            phase: SwitchbackPhase::PreparingPrevious,
            opened: None,
            closed: None,
            drained: None,
            switch_started_ms: None,
            drain_started_ms: None,
        };
        if !next.valid() {
            return Err(OnlineError::InvalidJournal);
        }
        deployment
            .save(None, &next)
            .map_err(OnlineError::Deployment)?;
        Ok(next)
    }

    #[must_use]
    pub fn plan(&self) -> &SwitchbackPlan {
        &self.plan
    }
    #[must_use]
    pub fn phase(&self) -> SwitchbackPhase {
        self.phase
    }
    #[must_use]
    pub fn drained(&self) -> Option<&RuntimeSnapshot> {
        self.drained.as_ref()
    }

    /// Accept only disk/live combinations reachable by the durable reverse
    /// phase. Observation never repairs a mismatch or advances the journal.
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
            SwitchbackPhase::PreparingPrevious => live.slot == candidate && disk == candidate,
            SwitchbackPhase::SwitchingBack => {
                // Persist the old-slot boot target before the live CAS. A lost
                // reply may leave either step applied, never live-old/disk-new.
                (live.slot == candidate && (disk == candidate || disk == previous))
                    || (live.slot == previous && disk == previous)
            }
            SwitchbackPhase::ClosingCandidate
            | SwitchbackPhase::DrainingCandidate
            | SwitchbackPhase::DrainBudgetExhausted
            | SwitchbackPhase::RetiringCandidate
            | SwitchbackPhase::CommittingPrevious
            | SwitchbackPhase::Complete => live.slot == previous && disk == previous,
        }
    }

    #[must_use]
    pub fn valid(&self) -> bool {
        if self.schema != "aster.online-switchback.v1"
            || !self.plan.valid()
            || self.revision != self.phase.revision()
        {
            return false;
        }
        let opened = self.opened.as_ref().is_some_and(|opened| {
            if self.plan.previous.lifecycle.accepting {
                unchanged_admission(opened, &self.plan.previous)
            } else {
                changed_admission(opened, &self.plan.previous, true)
            }
        });
        let switch = self.switch_started_ms.is_some_and(|start| {
            start >= self.plan.clock.uptime_ms
                && start
                    .checked_add(self.plan.proxy.stream_close_delay_ms)
                    .is_some()
        });
        let drain = self.drain_started_ms.is_some_and(|start| {
            self.switch_started_ms.is_some_and(|switch| {
                start >= switch
                    && start
                        .checked_add(self.plan.candidate_budget.request_budget_ms)
                        .is_some_and(|end| {
                            switch
                                .checked_add(self.plan.proxy.stream_close_delay_ms)
                                .is_some_and(|limit| end <= limit)
                        })
            })
        });
        let closed = self
            .closed
            .as_ref()
            .is_some_and(|closed| changed_admission(closed, &self.plan.candidate, false));
        let drained = self.drained.as_ref().is_some_and(|value| {
            value.lifecycle.in_flight == 0
                && self
                    .closed
                    .as_ref()
                    .is_some_and(|closed| value.observes_drain(closed))
        });
        match self.phase {
            SwitchbackPhase::PreparingPrevious => {
                self.opened.is_none()
                    && self.closed.is_none()
                    && self.drained.is_none()
                    && self.switch_started_ms.is_none()
                    && self.drain_started_ms.is_none()
            }
            SwitchbackPhase::SwitchingBack => {
                opened
                    && switch
                    && self.closed.is_none()
                    && self.drained.is_none()
                    && self.drain_started_ms.is_none()
            }
            SwitchbackPhase::ClosingCandidate => {
                opened && switch && drain && self.closed.is_none() && self.drained.is_none()
            }
            SwitchbackPhase::DrainingCandidate | SwitchbackPhase::DrainBudgetExhausted => {
                opened && switch && drain && closed && self.drained.is_none()
            }
            SwitchbackPhase::RetiringCandidate
            | SwitchbackPhase::CommittingPrevious
            | SwitchbackPhase::Complete => opened && switch && drain && closed && drained,
        }
    }

    #[must_use]
    pub fn follows(&self, previous: Option<&Self>) -> bool {
        if !self.valid() {
            return false;
        }
        let Some(previous) = previous else {
            return self.revision == 0;
        };
        previous.valid()
            && self.plan == previous.plan
            && previous.revision.checked_add(1) == Some(self.revision)
            && !matches!(
                previous.phase,
                SwitchbackPhase::Complete | SwitchbackPhase::DrainBudgetExhausted
            )
            && previous
                .opened
                .as_ref()
                .is_none_or(|v| self.opened.as_ref() == Some(v))
            && previous
                .closed
                .as_ref()
                .is_none_or(|v| self.closed.as_ref() == Some(v))
            && previous
                .drained
                .as_ref()
                .is_none_or(|v| self.drained.as_ref() == Some(v))
            && previous
                .switch_started_ms
                .is_none_or(|v| self.switch_started_ms == Some(v))
            && previous
                .drain_started_ms
                .is_none_or(|v| self.drain_started_ms == Some(v))
    }

    fn checkpoint<R, D: SwitchbackDeployment>(
        &mut self,
        deployment: &mut D,
        deadline: Instant,
        phase: SwitchbackPhase,
        update: impl FnOnce(&mut Self),
    ) -> OnlineResult<SwitchbackProgress, R, D::Error> {
        before(deadline)?;
        let mut next = self.clone();
        next.revision += 1;
        next.phase = phase;
        update(&mut next);
        if !next.follows(Some(self)) {
            return Err(OnlineError::InvalidJournal);
        }
        deployment
            .save(Some(self), &next)
            .map_err(OnlineError::Deployment)?;
        *self = next;
        Ok(SwitchbackProgress::Advanced(phase))
    }

    fn switch_deadline<R, D>(
        &self,
        now: u64,
        started: Instant,
        deadline: Instant,
    ) -> OnlineResult<Instant, R, D> {
        let remaining = self
            .switch_started_ms
            .and_then(|start| {
                start.checked_add(
                    self.plan.proxy.stream_close_delay_ms
                        - self.plan.candidate_budget.request_budget_ms,
                )
            })
            .and_then(|end| end.checked_sub(now))
            .filter(|v| *v > 0)
            .ok_or(OnlineError::ProxyWindowElapsed)?;
        let bounded = deadline.min(
            started
                .checked_add(Duration::from_millis(remaining))
                .ok_or(OnlineError::Clock)?,
        );
        before(bounded)?;
        Ok(bounded)
    }

    pub fn advance<R: SlotRetirement, D: SwitchbackDeployment>(
        &mut self,
        previous: &R,
        candidate: &R,
        deployment: &mut D,
        clock: &UpgradeClock,
        deadline: Instant,
    ) -> OnlineResult<SwitchbackProgress, R::Error, D::Error> {
        let started = Instant::now();
        if !self.valid() {
            return Err(OnlineError::InvalidJournal);
        }
        if started >= deadline {
            return Err(OnlineError::Deadline);
        }
        deployment
            .assert_current(self)
            .map_err(OnlineError::Deployment)?;
        if self.phase == SwitchbackPhase::Complete {
            return Ok(SwitchbackProgress::Complete);
        }
        if self.phase == SwitchbackPhase::DrainBudgetExhausted {
            return Ok(SwitchbackProgress::DrainBudgetExhausted);
        }
        if clock.boot_id != self.plan.clock.boot_id {
            return Err(OnlineError::BootChanged);
        }
        if clock.uptime_ms < self.plan.clock.uptime_ms
            || self
                .switch_started_ms
                .is_some_and(|start| clock.uptime_ms < start)
            || self
                .drain_started_ms
                .is_some_and(|start| clock.uptime_ms < start)
        {
            return Err(OnlineError::Clock);
        }
        let deadline = if self.phase == SwitchbackPhase::SwitchingBack {
            self.switch_deadline(clock.uptime_ms, started, deadline)?
        } else {
            deadline
        };
        let traffic = deployment
            .traffic(self, deadline)
            .map_err(OnlineError::Deployment)?;
        let on_previous = traffic == self.plan.original.plan.previous.slot;
        if traffic != self.plan.original.plan.candidate.slot && !on_previous {
            return Err(OnlineError::TrafficChanged);
        }
        if !matches!(
            self.phase,
            SwitchbackPhase::PreparingPrevious | SwitchbackPhase::SwitchingBack
        ) && !on_previous
        {
            return Err(OnlineError::TrafficChanged);
        }
        let old = previous.status(deadline).map_err(OnlineError::Runtime)?;
        if self.phase != SwitchbackPhase::PreparingPrevious
            && self
                .opened
                .as_ref()
                .is_none_or(|opened| !unchanged_admission(&old, opened))
        {
            return Err(OnlineError::RuntimeChanged);
        }
        match self.phase {
            SwitchbackPhase::PreparingPrevious => {
                if on_previous {
                    return Err(OnlineError::TrafficChanged);
                }
                let was_opened = if self.plan.previous.lifecycle.accepting {
                    unchanged_admission(&old, &self.plan.previous)
                } else {
                    changed_admission(&old, &self.plan.previous, true)
                };
                if !was_opened && !unchanged_admission(&old, &self.plan.previous) {
                    return Err(OnlineError::RuntimeChanged);
                }
                let permit = previous
                    .readiness(&old, &self.plan.previous_readiness(), deadline)
                    .map_err(OnlineError::Runtime)?;
                let opened = if was_opened {
                    old
                } else {
                    before(deadline)?;
                    previous
                        .open_admission(permit, deadline)
                        .map_err(OnlineError::Runtime)?
                };
                self.checkpoint(
                    deployment,
                    deadline,
                    SwitchbackPhase::SwitchingBack,
                    |next| {
                        next.opened = Some(opened);
                        next.switch_started_ms = Some(clock.uptime_ms);
                    },
                )
            }
            SwitchbackPhase::SwitchingBack => {
                let _permit = previous
                    .readiness(&old, &self.plan.previous_readiness(), deadline)
                    .map_err(OnlineError::Runtime)?;
                if !on_previous {
                    before(deadline)?;
                    deployment
                        .switch_back(self, deadline)
                        .map_err(OnlineError::Deployment)?;
                }
                if Instant::now() >= deadline {
                    return Err(OnlineError::Deadline);
                }
                if deployment
                    .traffic(self, deadline)
                    .map_err(OnlineError::Deployment)?
                    != self.plan.original.plan.previous.slot
                {
                    return Err(OnlineError::TrafficChanged);
                }
                let elapsed =
                    u64::try_from(started.elapsed().as_millis()).map_err(|_| OnlineError::Clock)?;
                let now = clock
                    .uptime_ms
                    .checked_add(elapsed)
                    .ok_or(OnlineError::Clock)?;
                self.checkpoint(
                    deployment,
                    deadline,
                    SwitchbackPhase::ClosingCandidate,
                    |next| next.drain_started_ms = Some(now),
                )
            }
            SwitchbackPhase::ClosingCandidate => {
                let current = candidate.status(deadline).map_err(OnlineError::Runtime)?;
                let closed = if changed_admission(&current, &self.plan.candidate, false) {
                    current
                } else {
                    if !unchanged_admission(&current, &self.plan.candidate) {
                        return Err(OnlineError::RuntimeChanged);
                    }
                    let deadline = self.switch_deadline(clock.uptime_ms, started, deadline)?;
                    before(deadline)?;
                    candidate
                        .close_admission(&current, deadline)
                        .map_err(OnlineError::Runtime)?
                };
                self.checkpoint(
                    deployment,
                    deadline,
                    SwitchbackPhase::DrainingCandidate,
                    |next| next.closed = Some(closed),
                )
            }
            SwitchbackPhase::DrainingCandidate => {
                let current = candidate.status(deadline).map_err(OnlineError::Runtime)?;
                if self
                    .closed
                    .as_ref()
                    .is_none_or(|closed| !current.observes_drain(closed))
                {
                    return Err(OnlineError::RuntimeChanged);
                }
                let elapsed =
                    u64::try_from(started.elapsed().as_millis()).map_err(|_| OnlineError::Clock)?;
                let now = clock
                    .uptime_ms
                    .checked_add(elapsed)
                    .ok_or(OnlineError::Clock)?;
                let end = self
                    .drain_started_ms
                    .and_then(|start| {
                        start.checked_add(self.plan.candidate_budget.request_budget_ms)
                    })
                    .ok_or(OnlineError::Clock)?;
                if now >= end {
                    return self.checkpoint(
                        deployment,
                        deadline,
                        SwitchbackPhase::DrainBudgetExhausted,
                        |_| {},
                    );
                }
                if current.lifecycle.in_flight == 0 {
                    return self.checkpoint(
                        deployment,
                        deadline,
                        SwitchbackPhase::RetiringCandidate,
                        |next| next.drained = Some(current),
                    );
                }
                Ok(SwitchbackProgress::Draining {
                    count: current.lifecycle.in_flight,
                    oldest_request_age_ms: current.lifecycle.oldest_request_age_ms,
                })
            }
            SwitchbackPhase::RetiringCandidate => {
                let _permit = previous
                    .readiness(&old, &self.plan.previous_readiness(), deadline)
                    .map_err(OnlineError::Runtime)?;
                let drained = self.drained.as_ref().ok_or(OnlineError::InvalidJournal)?;
                if crate::retirement::retire_control(
                    candidate,
                    &SwitchbackProcess(&*deployment),
                    drained,
                    deadline,
                )
                .map_err(OnlineError::Retirement)?
                    != crate::retirement::ControlRetirementProgress::Exited
                {
                    return Ok(SwitchbackProgress::Retiring);
                }
                if deployment
                    .retire_candidate_runner(self, deadline)
                    .map_err(OnlineError::Deployment)?
                    != ProcessProgress::Exited
                {
                    return Ok(SwitchbackProgress::Retiring);
                }
                self.checkpoint(
                    deployment,
                    deadline,
                    SwitchbackPhase::CommittingPrevious,
                    |_| {},
                )
            }
            SwitchbackPhase::CommittingPrevious => {
                before(deadline)?;
                deployment
                    .commit_previous(self, deadline)
                    .map_err(OnlineError::Deployment)?;
                self.checkpoint(deployment, deadline, SwitchbackPhase::Complete, |_| {})
            }
            SwitchbackPhase::Complete | SwitchbackPhase::DrainBudgetExhausted => {
                unreachable!("terminal phases returned above")
            }
        }
    }
}

fn before<R, D>(deadline: Instant) -> OnlineResult<(), R, D> {
    if Instant::now() >= deadline {
        Err(OnlineError::Deadline)
    } else {
        Ok(())
    }
}

struct SwitchbackProcess<'a, D>(&'a D);
impl<D: SwitchbackDeployment> ControlProcessObserver for SwitchbackProcess<'_, D> {
    type Error = D::Error;
    fn observe_control(
        &self,
        drained: &RuntimeSnapshot,
        deadline: Instant,
    ) -> Result<ProcessProgress, Self::Error> {
        self.0.observe_control(drained, deadline)
    }
}
