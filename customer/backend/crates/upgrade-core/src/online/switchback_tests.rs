use super::*;
use crate::online::switchback::*;
use crate::runtime::RuntimeRequestBudget;

struct BackDeployment {
    saved: Option<SwitchbackJournal>,
    original: OnlineJournal,
    candidate: Rc<Runtime>,
    traffic: ReleaseSlot,
    switches: usize,
    runner_stops: usize,
    commits: usize,
    lose_switch: bool,
    save_fault: Option<(SwitchbackPhase, bool)>,
    hold_runner: bool,
}

impl SwitchbackJournalStorage for BackDeployment {
    type Error = &'static str;
    fn load(&self) -> Result<Option<SwitchbackJournal>, Self::Error> {
        Ok(self.saved.clone())
    }

    fn assert_current(&mut self, journal: &SwitchbackJournal) -> Result<(), Self::Error> {
        if self.saved.as_ref() == Some(journal) && self.original == journal.plan().original {
            Ok(())
        } else {
            Err("stale switchback")
        }
    }
    fn save(
        &mut self,
        previous: Option<&SwitchbackJournal>,
        next: &SwitchbackJournal,
    ) -> Result<(), Self::Error> {
        if self.original != next.plan().original
            || previous != self.saved.as_ref()
            || !next.follows(previous)
        {
            return Err("lost journal ownership");
        }
        let fault = self.save_fault.filter(|(phase, _)| *phase == next.phase());
        if fault.is_some() {
            self.save_fault = None;
        }
        if fault == Some((next.phase(), false)) {
            return Err("write failed");
        }
        self.saved = Some(next.clone());
        if fault.is_some() {
            Err("write reply lost")
        } else {
            Ok(())
        }
    }
}

impl SwitchbackDeployment for BackDeployment {
    fn observe_control(
        &self,
        drained: &RuntimeSnapshot,
        _: Instant,
    ) -> Result<ProcessProgress, Self::Error> {
        let current = self.candidate.state.borrow();
        if !current.same_process(drained) {
            return Err("candidate was replaced");
        }
        Ok(if self.candidate.gone.get() {
            ProcessProgress::Exited
        } else if current.lifecycle.stopping {
            ProcessProgress::Stopping
        } else {
            ProcessProgress::Running
        })
    }

    fn traffic(&mut self, _: &SwitchbackJournal, _: Instant) -> Result<ReleaseSlot, Self::Error> {
        Ok(self.traffic)
    }
    fn switch_back(&mut self, journal: &SwitchbackJournal, _: Instant) -> Result<(), Self::Error> {
        self.assert_current(journal)?;
        assert_eq!(journal.phase(), SwitchbackPhase::SwitchingBack);
        self.traffic = journal.plan().original.plan().previous.slot;
        self.switches += 1;
        if std::mem::take(&mut self.lose_switch) {
            Err("switch reply lost")
        } else {
            Ok(())
        }
    }
    fn retire_candidate_runner(
        &mut self,
        journal: &SwitchbackJournal,
        _: Instant,
    ) -> Result<ProcessProgress, Self::Error> {
        self.assert_current(journal)?;
        assert_eq!(journal.phase(), SwitchbackPhase::RetiringCandidate);
        assert!(self.candidate.gone.get());
        assert_eq!(journal.drained().unwrap().lifecycle.in_flight, 0);
        self.runner_stops += 1;
        Ok(if self.hold_runner {
            ProcessProgress::Stopping
        } else {
            ProcessProgress::Exited
        })
    }
    fn commit_previous(
        &mut self,
        journal: &SwitchbackJournal,
        _: Instant,
    ) -> Result<(), Self::Error> {
        self.assert_current(journal)?;
        assert_eq!(journal.phase(), SwitchbackPhase::CommittingPrevious);
        assert!(self.candidate.gone.get());
        assert!(self.runner_stops > 0);
        assert!(!self.hold_runner);
        self.commits += 1;
        Ok(())
    }
}

struct BackRig {
    old: Rc<Runtime>,
    candidate: Rc<Runtime>,
    deployment: BackDeployment,
    journal: SwitchbackJournal,
}

impl BackRig {
    fn new() -> Self {
        let mut forward = Rig::new();
        forward.previous.state.borrow_mut().lifecycle.in_flight = 2;
        forward.until(OnlinePhase::DrainingPrevious);
        forward.candidate.state.borrow_mut().lifecycle.in_flight = 3;
        let candidate = Rc::new(forward.candidate);
        let candidate_state = candidate.state.borrow().clone();
        let mut proxy = forward.journal.plan().proxy.clone();
        proxy.slot = ReleaseSlot::Green;
        let plan = SwitchbackPlan {
            original: forward.journal.clone(),
            previous: forward.previous.state.borrow().clone(),
            candidate: candidate_state.clone(),
            candidate_budget: RuntimeRequestBudget {
                runtime: candidate_state,
                request_budget_ms: 600_000,
            },
            proxy,
            clock: UpgradeClock {
                boot_id: "test-boot".into(),
                uptime_ms: 2_000,
            },
        };
        let mut deployment = BackDeployment {
            saved: None,
            original: forward.journal,
            candidate: candidate.clone(),
            traffic: ReleaseSlot::Green,
            switches: 0,
            runner_stops: 0,
            commits: 0,
            lose_switch: false,
            save_fault: None,
            hold_runner: false,
        };
        let journal = SwitchbackJournal::create(plan, &mut deployment).unwrap();
        Self {
            old: forward.previous,
            candidate,
            deployment,
            journal,
        }
    }
    fn step_at(
        &mut self,
        now: u64,
    ) -> OnlineResult<SwitchbackProgress, &'static str, &'static str> {
        self.journal.advance(
            self.old.as_ref(),
            self.candidate.as_ref(),
            &mut self.deployment,
            &UpgradeClock {
                boot_id: "test-boot".into(),
                uptime_ms: now,
            },
            Instant::now() + Duration::from_secs(30),
        )
    }
    fn step(&mut self) -> OnlineResult<SwitchbackProgress, &'static str, &'static str> {
        self.step_at(2_001)
    }
    fn until(&mut self, phase: SwitchbackPhase) {
        for _ in 0..12 {
            self.journal = self.deployment.saved.clone().unwrap();
            if self.journal.phase() == phase {
                return;
            }
            let _ = self.step();
        }
        panic!("switchback did not reach {phase:?}: {:?}", self.journal);
    }
}

#[test]
fn reverse_route_keeps_both_work_owners_until_candidate_is_durably_drained() {
    let mut rig = BackRig::new();
    assert!(rig.journal.plan().valid());
    let expected = rig.journal.plan().previous_readiness();
    assert_eq!(expected.manifest_sha256, "a".repeat(64));
    assert_eq!(expected.models, vec!["model"]);
    rig.until(SwitchbackPhase::DrainingCandidate);
    assert_eq!(rig.deployment.traffic, ReleaseSlot::Blue);
    assert_eq!(rig.old.state.borrow().lifecycle.in_flight, 2);
    assert!(rig.old.state.borrow().lifecycle.accepting);
    assert!(matches!(
        rig.step().unwrap(),
        SwitchbackProgress::Draining { count: 3, .. }
    ));
    assert_eq!(rig.candidate.retirement_calls.get(), 0);
    assert_eq!(rig.deployment.commits, 0);
    rig.candidate.state.borrow_mut().lifecycle.in_flight = 0;
    rig.until(SwitchbackPhase::Complete);
    assert_eq!(rig.candidate.retirement_calls.get(), 1);
    assert_eq!(rig.old.retirement_calls.get(), 0);
    assert!(!rig.old.gone.get());
    assert_eq!(rig.deployment.commits, 1);
}

#[test]
fn lost_mutation_replies_reconcile_without_replaying_open_switch_or_close() {
    let mut rig = BackRig::new();
    rig.old.lose_open.set(true);
    rig.deployment.lose_switch = true;
    rig.candidate.lose_close.set(true);
    rig.until(SwitchbackPhase::DrainingCandidate);
    assert_eq!(rig.old.opens.get(), 1);
    assert_eq!(rig.candidate.closes.get(), 1);
    assert_eq!(rig.deployment.switches, 1);
    assert!(rig.old.readiness_checks.get() >= 4);
    rig.candidate.state.borrow_mut().lifecycle.in_flight = 0;
    rig.until(SwitchbackPhase::Complete);
}

#[test]
fn every_reverse_checkpoint_survives_before_and_after_write_failure() {
    for phase in [
        SwitchbackPhase::SwitchingBack,
        SwitchbackPhase::ClosingCandidate,
        SwitchbackPhase::DrainingCandidate,
        SwitchbackPhase::RetiringCandidate,
        SwitchbackPhase::CommittingPrevious,
        SwitchbackPhase::Complete,
    ] {
        for after in [false, true] {
            let mut rig = BackRig::new();
            rig.deployment.save_fault = Some((phase, after));
            rig.candidate.state.borrow_mut().lifecycle.in_flight = 0;
            rig.until(SwitchbackPhase::Complete);
            assert_eq!(rig.old.opens.get(), 1);
            assert_eq!(rig.candidate.closes.get(), 1);
            assert_eq!(rig.deployment.switches, 1);
            assert_eq!(rig.candidate.retirement_calls.get(), 1);
        }
    }
}

#[test]
fn failed_readiness_and_replacement_identity_cannot_change_routing() {
    for replace in [false, true] {
        let mut rig = BackRig::new();
        if replace {
            rig.old
                .state
                .borrow_mut()
                .instance_id
                .push_str("-replacement");
        } else {
            rig.old.deny_ready.set(true);
        }
        assert!(rig.step().is_err());
        assert_eq!(rig.deployment.switches, 0);
        assert_eq!(rig.old.opens.get(), 0);
        assert_eq!(rig.candidate.closes.get(), 0);
    }
    let mut rig = BackRig::new();
    rig.until(SwitchbackPhase::SwitchingBack);
    rig.old.deny_ready.set(true);
    assert!(rig.step().is_err());
    assert_eq!(rig.deployment.switches, 0);
}

#[test]
fn exhaustion_is_durable_and_does_not_reset_or_stop_pending_work() {
    let mut rig = BackRig::new();
    rig.until(SwitchbackPhase::DrainingCandidate);
    rig.step_at(602_100).unwrap();
    assert_eq!(rig.journal.phase(), SwitchbackPhase::DrainBudgetExhausted);
    rig.journal = serde_json::from_slice(&serde_json::to_vec(&rig.journal).unwrap()).unwrap();
    rig.candidate.state.borrow_mut().lifecycle.in_flight = 0;
    assert_eq!(
        rig.step_at(602_200).unwrap(),
        SwitchbackProgress::DrainBudgetExhausted
    );
    assert_eq!(rig.candidate.retirement_calls.get(), 0);
    assert_eq!(rig.deployment.runner_stops, 0);
    assert_eq!(rig.deployment.commits, 0);
}

#[test]
fn candidate_exit_acknowledgement_does_not_prove_runner_exit() {
    let mut rig = BackRig::new();
    rig.candidate.state.borrow_mut().lifecycle.in_flight = 0;
    rig.until(SwitchbackPhase::RetiringCandidate);
    rig.candidate.hold_exit.set(true);
    assert_eq!(rig.step().unwrap(), SwitchbackProgress::Retiring);
    assert_eq!(rig.deployment.runner_stops, 0);
    rig.candidate.gone.set(true);
    rig.deployment.hold_runner = true;
    assert_eq!(rig.step().unwrap(), SwitchbackProgress::Retiring);
    assert_eq!(rig.deployment.commits, 0);
    rig.deployment.hold_runner = false;
    rig.until(SwitchbackPhase::Complete);
}

#[test]
fn pause_cannot_extend_close_window_but_an_already_closed_reply_can_be_reconciled() {
    let mut rig = BackRig::new();
    rig.until(SwitchbackPhase::ClosingCandidate);
    assert!(matches!(
        rig.step_at(302_100),
        Err(OnlineError::ProxyWindowElapsed)
    ));
    assert_eq!(rig.candidate.closes.get(), 0);
    assert!(rig.candidate.state.borrow().lifecycle.accepting);

    let mut rig = BackRig::new();
    rig.until(SwitchbackPhase::ClosingCandidate);
    rig.candidate.lose_close.set(true);
    assert!(rig.step().is_err());
    rig.journal = rig.deployment.saved.clone().unwrap();
    assert_eq!(
        rig.step_at(302_100).unwrap(),
        SwitchbackProgress::Advanced(SwitchbackPhase::DrainingCandidate)
    );
    assert_eq!(rig.candidate.closes.get(), 1);
}

#[test]
fn old_business_failure_before_retirement_keeps_the_candidate_alive() {
    let mut rig = BackRig::new();
    rig.candidate.state.borrow_mut().lifecycle.in_flight = 0;
    rig.until(SwitchbackPhase::RetiringCandidate);
    rig.old.deny_ready.set(true);
    assert!(rig.step().is_err());
    assert_eq!(rig.candidate.retirement_calls.get(), 0);
    assert_eq!(rig.deployment.runner_stops, 0);
    assert_eq!(rig.deployment.commits, 0);
    assert!(!rig.candidate.gone.get());
}
