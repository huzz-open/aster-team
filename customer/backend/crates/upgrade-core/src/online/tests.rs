use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use super::*;
#[path = "switchback_tests.rs"]
mod switchback_cases;
use crate::{
    ActiveLocalRunner, ReleaseSlot,
    runtime::{DrainProgress, LifecycleSnapshot, SlotRuntime},
};

struct Runtime {
    state: RefCell<RuntimeSnapshot>,
    gone: Cell<bool>,
    lose_open: Cell<bool>,
    lose_close: Cell<bool>,
    opens: Cell<usize>,
    closes: Cell<usize>,
    readiness_checks: Cell<usize>,
    deny_ready: Cell<bool>,
    last_deadline: Cell<Option<Instant>>,
    retirement_calls: Cell<usize>,
    hold_exit: Cell<bool>,
}

impl Runtime {
    fn new(state: RuntimeSnapshot) -> Self {
        Self {
            state: RefCell::new(state),
            gone: Cell::new(false),
            lose_open: Cell::new(false),
            lose_close: Cell::new(false),
            opens: Cell::new(0),
            closes: Cell::new(0),
            readiness_checks: Cell::new(0),
            deny_ready: Cell::new(false),
            last_deadline: Cell::new(None),
            retirement_calls: Cell::new(0),
            hold_exit: Cell::new(false),
        }
    }
}

impl SlotRuntime for Runtime {
    type Error = &'static str;
    type ReadyPermit = RuntimeSnapshot;

    fn status(&self, _: Instant) -> Result<RuntimeSnapshot, Self::Error> {
        if self.gone.get() {
            Err("unreachable")
        } else {
            Ok(self.state.borrow().clone())
        }
    }

    fn readiness(
        &self,
        observed: &RuntimeSnapshot,
        _: &ReadinessExpectation,
        deadline: Instant,
    ) -> Result<Self::ReadyPermit, Self::Error> {
        self.last_deadline.set(Some(deadline));
        self.readiness_checks.set(self.readiness_checks.get() + 1);
        if self.deny_ready.get() {
            return Err("business readiness failed");
        }
        assert_eq!(observed, &*self.state.borrow());
        Ok(observed.clone())
    }

    fn open_admission(
        &self,
        permit: Self::ReadyPermit,
        _: Instant,
    ) -> Result<RuntimeSnapshot, Self::Error> {
        assert_eq!(permit, *self.state.borrow());
        let mut state = self.state.borrow_mut();
        state.lifecycle.accepting = true;
        state.lifecycle.revision += 1;
        self.opens.set(self.opens.get() + 1);
        if self.lose_open.replace(false) {
            Err("open reply lost")
        } else {
            Ok(state.clone())
        }
    }

    fn close_admission(
        &self,
        observed: &RuntimeSnapshot,
        _: Instant,
    ) -> Result<RuntimeSnapshot, Self::Error> {
        assert_eq!(observed, &*self.state.borrow());
        let mut state = self.state.borrow_mut();
        state.lifecycle.accepting = false;
        state.lifecycle.revision += 1;
        self.closes.set(self.closes.get() + 1);
        if self.lose_close.replace(false) {
            Err("close reply lost")
        } else {
            Ok(state.clone())
        }
    }

    fn observe_drain(
        &self,
        closed: &RuntimeSnapshot,
        deadline: Instant,
    ) -> Result<DrainProgress, Self::Error> {
        let state = self.status(deadline)?;
        if !state.observes_drain(closed) {
            return Err("drain changed");
        }
        Ok(if state.lifecycle.in_flight == 0 {
            DrainProgress::Drained
        } else {
            DrainProgress::Outstanding {
                count: state.lifecycle.in_flight,
                oldest_request_age_ms: state.lifecycle.oldest_request_age_ms,
            }
        })
    }
}

impl SlotRetirement for Runtime {
    fn retire_drained(
        &self,
        expected: &RuntimeSnapshot,
        _: Instant,
    ) -> Result<RuntimeSnapshot, Self::Error> {
        let mut state = self.state.borrow_mut();
        assert!(state.observes_drain(expected));
        assert_eq!(state.lifecycle.in_flight, 0);
        state.lifecycle.stopping = true;
        state.lifecycle.revision += 1;
        self.retirement_calls.set(self.retirement_calls.get() + 1);
        self.gone.set(!self.hold_exit.get());
        Ok(state.clone())
    }
}

struct Deployment {
    saved: Option<OnlineJournal>,
    traffic: RuntimeSnapshot,
    old: Rc<Runtime>,
    save_failure: Option<(OnlinePhase, bool)>,
    lose_switch: bool,
    lose_retire: bool,
    hold_runner_exit: bool,
    lose_commit: bool,
    switches: usize,
    stops: usize,
    commits: usize,
}

impl OnlineJournalStorage for Deployment {
    type Error = &'static str;

    fn load(&self) -> Result<Option<OnlineJournal>, Self::Error> {
        Ok(self.saved.clone())
    }

    fn assert_current(&mut self, journal: &OnlineJournal) -> Result<(), Self::Error> {
        if self.saved.as_ref() == Some(journal) {
            Ok(())
        } else {
            Err("stale journal")
        }
    }

    fn save(
        &mut self,
        previous: Option<&OnlineJournal>,
        next: &OnlineJournal,
    ) -> Result<(), Self::Error> {
        if previous != self.saved.as_ref() {
            return Err("CAS conflict");
        }
        assert!(next.follows(previous));
        let fault = self
            .save_failure
            .filter(|(phase, _)| *phase == next.phase());
        if fault.is_some() {
            self.save_failure = None;
        }
        if fault == Some((next.phase(), false)) {
            return Err("before durable save");
        }
        self.saved = Some(serde_json::from_slice(&serde_json::to_vec(next).unwrap()).unwrap());
        if fault.is_some() {
            return Err("after durable save");
        }
        Ok(())
    }
}

impl OnlineDeployment for Deployment {
    fn observe_control(
        &self,
        drained: &RuntimeSnapshot,
        _: Instant,
    ) -> Result<ProcessProgress, Self::Error> {
        let state = self.old.state.borrow();
        if !state.same_process(drained) {
            return Err("Control activation changed");
        }
        if self.old.gone.get() {
            if state.observes_retirement(drained) {
                Ok(ProcessProgress::Exited)
            } else {
                Err("Control did not exit normally")
            }
        } else {
            Ok(ProcessProgress::Running)
        }
    }
    fn traffic(&mut self, _: Instant) -> Result<RuntimeSnapshot, Self::Error> {
        Ok(self.traffic.clone())
    }

    fn switch_to(&mut self, candidate: &RuntimeSnapshot, _: Instant) -> Result<(), Self::Error> {
        assert_eq!(
            self.saved.as_ref().unwrap().phase(),
            OnlinePhase::SwitchingTraffic
        );
        assert!(self.old.state.borrow().lifecycle.accepting);
        self.traffic = candidate.clone();
        self.switches += 1;
        if std::mem::take(&mut self.lose_switch) {
            Err("switch reply lost")
        } else {
            Ok(())
        }
    }

    fn retire(
        &mut self,
        journal: &OnlineJournal,
        _: Instant,
    ) -> Result<ProcessProgress, Self::Error> {
        assert_eq!(self.saved.as_ref(), Some(journal));
        assert_eq!(journal.phase(), OnlinePhase::RetiringPrevious);
        let proof = journal.drained().unwrap();
        assert_eq!(proof.lifecycle.in_flight, 0);
        assert!(self.old.gone.get());
        assert!(self.old.state.borrow().observes_retirement(proof));
        self.stops = 1;
        if std::mem::take(&mut self.lose_retire) {
            Err("retirement reply lost")
        } else if self.hold_runner_exit {
            Ok(ProcessProgress::Stopping)
        } else {
            Ok(ProcessProgress::Exited)
        }
    }

    fn commit(&mut self, journal: &OnlineJournal, _: Instant) -> Result<(), Self::Error> {
        assert_eq!(self.saved.as_ref(), Some(journal));
        assert_eq!(journal.phase(), OnlinePhase::Committing);
        assert!(self.old.gone.get());
        self.commits = 1;
        if std::mem::take(&mut self.lose_commit) {
            Err("commit reply lost")
        } else {
            Ok(())
        }
    }
}

struct Rig {
    previous: Rc<Runtime>,
    candidate: Runtime,
    deployment: Deployment,
    journal: OnlineJournal,
}

fn release(slot: ReleaseSlot) -> ActiveReleaseSlot {
    let digit = if slot == ReleaseSlot::Blue { "a" } else { "b" };
    ActiveReleaseSlot {
        schema: ACTIVE_SLOT_RUNTIME_SCHEMA.into(),
        slot,
        version: if slot == ReleaseSlot::Blue {
            "2.1.0"
        } else {
            "2.1.1"
        }
        .into(),
        local_runner: Some(ActiveLocalRunner {
            runner_id: format!("runner_{}", digit.repeat(32)),
            manifest_sha256: digit.repeat(64),
        }),
    }
}

fn process(slot: ReleaseSlot) -> RuntimeSnapshot {
    RuntimeSnapshot {
        schema: "aster.control-runtime.v1".into(),
        installation_id: "install-one".into(),
        slot,
        instance_id: format!("process-{}", slot.id()),
        service: Some(crate::runtime::ServiceInvocation {
            process_id: if slot == ReleaseSlot::Blue { 101 } else { 102 },
            invocation_id: if slot == ReleaseSlot::Blue {
                "a".repeat(32)
            } else {
                "b".repeat(32)
            },
        }),
        product_version: release(slot).version,
        lifecycle: LifecycleSnapshot {
            accepting: slot == ReleaseSlot::Blue,
            stopping: false,
            revision: 3,
            in_flight: 0,
            oldest_request_age_ms: 0,
        },
    }
}

impl Rig {
    fn new() -> Self {
        let previous = Rc::new(Runtime::new(process(ReleaseSlot::Blue)));
        let candidate = Runtime::new(process(ReleaseSlot::Green));
        let mut deployment = Deployment {
            saved: None,
            traffic: previous.state.borrow().clone(),
            old: previous.clone(),
            save_failure: None,
            lose_switch: false,
            lose_retire: false,
            hold_runner_exit: false,
            lose_commit: false,
            switches: 0,
            stops: 0,
            commits: 0,
        };
        let candidate_release = release(ReleaseSlot::Green);
        let runner = candidate_release.local_runner.as_ref().unwrap();
        let plan = OnlinePlan {
            job_id: "upgrade-job".into(),
            previous: release(ReleaseSlot::Blue),
            readiness: ReadinessExpectation {
                manifest_sha256: runner.manifest_sha256.clone(),
                models: vec!["model".into()],
                runner_ids: vec![runner.runner_id.clone()],
            },
            candidate: candidate_release,
            previous_process: previous.state.borrow().clone(),
            candidate_process: candidate.state.borrow().clone(),
            previous_runner_process: crate::runtime::ServiceInvocation {
                process_id: 201,
                invocation_id: "c".repeat(32),
            },
            candidate_runner_process: crate::runtime::ServiceInvocation {
                process_id: 202,
                invocation_id: "d".repeat(32),
            },
            drain_budget_ms: 720_000,
            clock: UpgradeClock {
                boot_id: "test-boot".into(),
                uptime_ms: 1000,
            },
            proxy: ProxySnapshot {
                slot: ReleaseSlot::Blue,
                configuration_sha256: "c".repeat(64),
                stream_close_delay_ms: 900_000,
            },
        };
        let journal = OnlineJournal::create(plan, &mut deployment).unwrap();
        Self {
            previous,
            candidate,
            deployment,
            journal,
        }
    }

    fn step_at(&mut self, now: u64) -> OnlineResult<OnlineProgress, &'static str, &'static str> {
        self.journal.advance(
            self.previous.as_ref(),
            &self.candidate,
            &mut self.deployment,
            &UpgradeClock {
                boot_id: "test-boot".into(),
                uptime_ms: now,
            },
            Instant::now() + Duration::from_secs(30),
        )
    }

    fn step(&mut self) -> OnlineResult<OnlineProgress, &'static str, &'static str> {
        self.step_at(1000)
    }

    fn until(&mut self, phase: OnlinePhase) {
        for _ in 0..10 {
            if self.journal.phase() == phase {
                return;
            }
            self.step().unwrap();
        }
        panic!("transition did not reach {phase:?}");
    }

    fn recover_to_complete(&mut self) {
        for _ in 0..20 {
            // Simulate a fresh process, retaining only durable state and the
            // actual runtime/proxy state, never the old in-memory permit.
            self.journal = self.deployment.saved.clone().unwrap();
            if self.journal.phase() == OnlinePhase::Complete {
                return;
            }
            let _ = self.step();
        }
        panic!("recovery did not finish: {:?}", self.journal);
    }
}

#[test]
fn forward_cutover_preserves_old_tasks_and_commits_only_after_durable_zero() {
    let mut rig = Rig::new();
    rig.previous.state.borrow_mut().lifecycle.in_flight = 2;
    rig.until(OnlinePhase::DrainingPrevious);
    assert!(matches!(
        rig.step().unwrap(),
        OnlineProgress::Draining { count: 2, .. }
    ));
    assert_eq!(rig.deployment.stops, 0);
    assert_eq!(rig.deployment.commits, 0);
    rig.previous.state.borrow_mut().lifecycle.in_flight = 0;
    rig.step().unwrap();
    assert_eq!(rig.journal.phase(), OnlinePhase::RetiringPrevious);
    assert_eq!(rig.deployment.stops, 0);
    rig.until(OnlinePhase::Complete);
    assert_eq!(rig.deployment.stops, 1);
    assert_eq!(rig.deployment.commits, 1);
    assert_eq!(rig.step().unwrap(), OnlineProgress::Complete);
}

#[test]
fn every_checkpoint_survives_failure_before_and_after_durable_write() {
    for phase in [
        OnlinePhase::SwitchingTraffic,
        OnlinePhase::ClosingPrevious,
        OnlinePhase::DrainingPrevious,
        OnlinePhase::RetiringPrevious,
        OnlinePhase::Committing,
        OnlinePhase::Complete,
    ] {
        for after_write in [false, true] {
            let mut rig = Rig::new();
            rig.deployment.save_failure = Some((phase, after_write));
            rig.recover_to_complete();
            assert!(rig.deployment.save_failure.is_none());
            assert_eq!(rig.candidate.opens.get(), 1, "{phase:?}/{after_write}");
            assert_eq!(rig.previous.closes.get(), 1, "{phase:?}/{after_write}");
            assert_eq!(rig.deployment.switches, 1, "{phase:?}/{after_write}");
            assert_eq!(rig.deployment.stops, 1, "{phase:?}/{after_write}");
        }
    }
}

#[test]
fn lost_mutation_replies_are_reconciled_without_replaying_admission_or_switch() {
    let mut rig = Rig::new();
    rig.candidate.lose_open.set(true);
    rig.previous.lose_close.set(true);
    rig.deployment.lose_switch = true;
    rig.deployment.lose_retire = true;
    rig.deployment.lose_commit = true;
    rig.recover_to_complete();
    assert_eq!(rig.candidate.opens.get(), 1);
    assert_eq!(rig.previous.closes.get(), 1);
    assert_eq!(rig.deployment.switches, 1);
    assert_eq!(rig.deployment.stops, 1);
    // Initial opening, two switch observations and two retirement attempts.
    assert_eq!(rig.candidate.readiness_checks.get(), 5);
}

#[test]
fn timeout_is_durable_and_never_kills_or_silently_restarts_the_budget() {
    let mut rig = Rig::new();
    rig.previous.state.borrow_mut().lifecycle.in_flight = 1;
    rig.until(OnlinePhase::DrainingPrevious);
    assert!(matches!(rig.step_at(999), Err(OnlineError::Clock)));
    rig.step_at(721_000).unwrap();
    assert_eq!(rig.journal.phase(), OnlinePhase::DrainBudgetExhausted);
    rig.journal = rig.deployment.saved.clone().unwrap();
    assert_eq!(rig.journal.exhausted().unwrap().lifecycle.in_flight, 1);
    assert!(rig.journal.drained().is_none());
    rig.previous.state.borrow_mut().lifecycle.in_flight = 0;
    assert_eq!(
        rig.step_at(722_000).unwrap(),
        OnlineProgress::DrainBudgetExhausted
    );
    assert_eq!(rig.deployment.stops, 0);
    assert_eq!(rig.deployment.commits, 0);
}

#[test]
fn lost_switch_reply_cannot_reset_the_retention_anchor_after_restart() {
    let mut rig = Rig::new();
    rig.until(OnlinePhase::SwitchingTraffic);
    rig.deployment.lose_switch = true;
    assert!(rig.step_at(1010).is_err());
    rig.journal = rig.deployment.saved.clone().unwrap();
    assert!(matches!(
        rig.step_at(181_000),
        Err(OnlineError::ProxyWindowElapsed)
    ));
    assert_eq!(rig.journal.cutover_started_at_ms, Some(1000));
    assert_eq!(rig.deployment.switches, 1);
    assert_eq!(rig.previous.closes.get(), 0);
    assert_eq!(rig.deployment.stops, 0);
}

#[test]
fn close_reconciliation_keeps_the_original_drain_budget() {
    let mut rig = Rig::new();
    rig.until(OnlinePhase::ClosingPrevious);
    rig.previous.lose_close.set(true);
    assert!(rig.step().is_err());
    rig.journal = rig.deployment.saved.clone().unwrap();
    // Past the window for *issuing* a new close, but the original close is
    // positively observed. Reconciliation must not reissue it or reset time.
    rig.step_at(200_000).unwrap();
    assert_eq!(rig.journal.phase(), OnlinePhase::DrainingPrevious);
    assert_eq!(rig.journal.drain_started_at_ms, Some(1000));
    assert_eq!(rig.previous.closes.get(), 1);
    rig.step_at(721_000).unwrap();
    assert_eq!(rig.journal.phase(), OnlinePhase::DrainBudgetExhausted);
    assert_eq!(rig.deployment.stops, 0);
}

#[test]
fn cutover_calls_receive_only_the_remaining_retention_reserve() {
    let mut rig = Rig::new();
    rig.until(OnlinePhase::SwitchingTraffic);
    let before = Instant::now();
    rig.step_at(180_000).unwrap();
    assert!(rig.candidate.last_deadline.get().unwrap() <= before + Duration::from_secs(2));
    // A new close can no longer reserve the drain interval after 181 seconds.
    assert!(matches!(
        rig.step_at(181_000),
        Err(OnlineError::ProxyWindowElapsed)
    ));
    assert_eq!(rig.previous.closes.get(), 0);
}

#[test]
fn reboot_is_not_mistaken_for_an_executor_restart() {
    let mut rig = Rig::new();
    rig.until(OnlinePhase::DrainingPrevious);
    let error = rig.journal.advance(
        rig.previous.as_ref(),
        &rig.candidate,
        &mut rig.deployment,
        &UpgradeClock {
            boot_id: "different-boot".into(),
            uptime_ms: 2000,
        },
        Instant::now() + Duration::from_secs(30),
    );
    assert!(matches!(error, Err(OnlineError::BootChanged)));
    assert_eq!(rig.deployment.stops, 0);
    assert_eq!(rig.deployment.commits, 0);
}

#[test]
fn lost_old_process_reopened_admission_and_revision_drift_do_not_prove_drain() {
    for fault in 0..4 {
        let mut rig = Rig::new();
        rig.until(OnlinePhase::DrainingPrevious);
        match fault {
            0 => rig.previous.gone.set(true),
            1 => rig.previous.state.borrow_mut().instance_id = "replacement".into(),
            2 => rig.previous.state.borrow_mut().lifecycle.accepting = true,
            _ => rig.previous.state.borrow_mut().lifecycle.revision += 1,
        }
        assert!(rig.step().is_err());
        assert_eq!(rig.journal.phase(), OnlinePhase::DrainingPrevious);
        assert_eq!(rig.deployment.stops, 0);
    }
}

#[test]
fn stale_writer_unknown_traffic_and_failed_candidate_do_not_close_old_admission() {
    for fault in 0..3 {
        let mut rig = Rig::new();
        rig.until(OnlinePhase::SwitchingTraffic);
        match fault {
            0 => rig.journal.revision += 1,
            1 => rig.deployment.traffic.instance_id = "unrelated".into(),
            _ => rig.candidate.state.borrow_mut().lifecycle.stopping = true,
        }
        assert!(rig.step().is_err());
        assert_eq!(rig.previous.closes.get(), 0);
        assert_eq!(rig.deployment.switches, 0);
    }
}

#[test]
fn expired_call_and_invalid_plan_cannot_mutate_runtime() {
    let mut rig = Rig::new();
    assert!(matches!(
        rig.journal.advance(
            rig.previous.as_ref(),
            &rig.candidate,
            &mut rig.deployment,
            &UpgradeClock {
                boot_id: "test-boot".into(),
                uptime_ms: 1000
            },
            Instant::now()
        ),
        Err(OnlineError::Deadline)
    ));
    assert_eq!(rig.candidate.opens.get(), 0);
    let mut plan = rig.journal.plan().clone();
    plan.previous.local_runner = None;
    assert!(!plan.valid());
    plan = rig.journal.plan().clone();
    plan.candidate_process.lifecycle.accepting = true;
    assert!(!plan.valid());
    plan = rig.journal.plan().clone();
    plan.readiness.runner_ids.clear();
    assert!(!plan.valid());
    plan = rig.journal.plan().clone();
    plan.proxy.stream_close_delay_ms = plan.drain_budget_ms;
    assert!(!plan.valid());
    plan = rig.journal.plan().clone();
    plan.proxy.slot = ReleaseSlot::Green;
    assert!(!plan.valid());
    plan = rig.journal.plan().clone();
    plan.previous_process.service = None;
    assert!(!plan.valid());
    plan = rig.journal.plan().clone();
    plan.candidate_process
        .service
        .as_mut()
        .unwrap()
        .invocation_id = plan
        .previous_process
        .service
        .as_ref()
        .unwrap()
        .invocation_id
        .clone();
    assert!(!plan.valid());
    plan = rig.journal.plan().clone();
    plan.candidate_process.service.as_mut().unwrap().process_id =
        plan.previous_process.service.as_ref().unwrap().process_id;
    assert!(!plan.valid());
    let mut value = serde_json::to_value(&rig.journal).unwrap();
    value["phase"] = "complete".into();
    let forged: OnlineJournal = serde_json::from_value(value.clone()).unwrap();
    assert!(!forged.valid());
    value["unrecognized"] = true.into();
    assert!(serde_json::from_value::<OnlineJournal>(value).is_err());
}

#[test]
fn replacing_old_process_after_persisted_zero_is_rejected_by_retirement_adapter() {
    let mut rig = Rig::new();
    rig.until(OnlinePhase::RetiringPrevious);
    rig.previous.state.borrow_mut().instance_id = "replacement".into();
    assert!(matches!(rig.step(), Err(OnlineError::Retirement(_))));
    assert_eq!(rig.deployment.stops, 0);
    assert_eq!(rig.deployment.commits, 0);
}

#[test]
fn acknowledged_control_retirement_cannot_stop_runner_or_commit_before_process_exit() {
    let mut rig = Rig::new();
    rig.until(OnlinePhase::RetiringPrevious);
    rig.previous.hold_exit.set(true);
    for _ in 0..2 {
        assert_eq!(rig.step().unwrap(), OnlineProgress::RetiringPrevious);
        assert_eq!(rig.journal.phase(), OnlinePhase::RetiringPrevious);
        assert_eq!(rig.deployment.stops, 0);
        assert_eq!(rig.deployment.commits, 0);
        rig.journal = rig.deployment.saved.clone().unwrap();
    }
    assert_eq!(rig.previous.retirement_calls.get(), 1);
    rig.previous.gone.set(true);
    rig.until(OnlinePhase::Complete);
    assert_eq!(rig.previous.retirement_calls.get(), 1);
    assert_eq!(rig.deployment.stops, 1);
    assert_eq!(rig.deployment.commits, 1);
}

#[test]
fn runner_activations_must_be_valid_and_distinct_from_every_control_and_runner() {
    let rig = Rig::new();
    let original = rig.journal.plan().clone();
    for candidate_runner in [false, true] {
        for replacement in [
            original.previous_process.service.clone().unwrap(),
            original.candidate_process.service.clone().unwrap(),
            if candidate_runner {
                original.previous_runner_process.clone()
            } else {
                original.candidate_runner_process.clone()
            },
        ] {
            for collision in 0..3 {
                let mut plan = original.clone();
                let target = if candidate_runner {
                    &mut plan.candidate_runner_process
                } else {
                    &mut plan.previous_runner_process
                };
                match collision {
                    0 => *target = replacement.clone(),
                    1 => target.process_id = replacement.process_id,
                    _ => target.invocation_id = replacement.invocation_id.clone(),
                }
                assert!(!plan.valid());
            }
        }
        for invalid in [
            crate::runtime::ServiceInvocation {
                process_id: 0,
                invocation_id: "e".repeat(32),
            },
            crate::runtime::ServiceInvocation {
                process_id: 301,
                invocation_id: "0".repeat(32),
            },
        ] {
            let mut plan = original.clone();
            if candidate_runner {
                plan.candidate_runner_process = invalid;
            } else {
                plan.previous_runner_process = invalid;
            }
            assert!(!plan.valid());
        }
    }
}

#[test]
fn recovery_requires_persisted_runner_processes_and_rejects_old_journals() {
    let mut rig = Rig::new();
    let original = serde_json::to_value(&rig.journal).unwrap();
    for field in ["previous_runner_process", "candidate_runner_process"] {
        let mut value = original.clone();
        value["plan"].as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<OnlineJournal>(value).is_err());
    }
    let mut value = original;
    value["schema"] = "aster.online-transition.v1".into();
    let old: OnlineJournal = serde_json::from_value(value).unwrap();
    assert!(!old.valid());
    rig.deployment.saved = Some(old.clone());
    rig.journal = old;
    assert!(matches!(rig.step(), Err(OnlineError::InvalidJournal)));
    assert_eq!(rig.candidate.opens.get(), 0);
    assert_eq!(rig.deployment.stops, 0);
    assert_eq!(rig.deployment.commits, 0);
}

#[test]
fn pending_runner_exit_keeps_the_durable_retirement_phase_without_commit() {
    let mut rig = Rig::new();
    rig.until(OnlinePhase::RetiringPrevious);
    rig.deployment.hold_runner_exit = true;
    for _ in 0..2 {
        assert_eq!(rig.step().unwrap(), OnlineProgress::RetiringPrevious);
        assert_eq!(rig.journal.phase(), OnlinePhase::RetiringPrevious);
        assert_eq!(rig.deployment.commits, 0);
        rig.journal = rig.deployment.saved.clone().unwrap();
    }
    rig.deployment.hold_runner_exit = false;
    assert_eq!(
        rig.step().unwrap(),
        OnlineProgress::Advanced(OnlinePhase::Committing)
    );
}
