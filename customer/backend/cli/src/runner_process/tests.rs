use super::*;
use aster_upgrade_core::{
    online::{OnlineJournalStorage, OnlinePlan},
    runtime::LifecycleSnapshot,
};
use std::{collections::VecDeque, time::Duration};

struct Host {
    survivor: ReleaseSlot,
    control: ProcessProgress,
    candidate: bool,
    old: VecDeque<Result<ProcessProgress, ()>>,
    events: Vec<&'static str>,
    signal_fails: bool,
    open_fails: bool,
    quiescence_checks: usize,
    fail_quiescence_at: Option<usize>,
}
impl Host {
    fn new(old: impl IntoIterator<Item = Result<ProcessProgress, ()>>) -> Self {
        Self {
            survivor: ReleaseSlot::Green,
            control: ProcessProgress::Exited,
            candidate: true,
            old: old.into_iter().collect(),
            events: vec![],
            signal_fails: false,
            open_fails: false,
            quiescence_checks: 0,
            fail_quiescence_at: None,
        }
    }
}
impl RunnerHost for Host {
    type Handle = u32;
    fn control(&mut self, _: &RuntimeSnapshot, _: Instant) -> Result<ProcessProgress, CliFailure> {
        self.events.push("control");
        Ok(self.control)
    }
    fn quiescent_control(&mut self, slot: ReleaseSlot, _: Instant) -> Result<(), CliFailure> {
        assert_eq!(slot, self.survivor.other());
        self.events.push("quiescent-control");
        self.quiescence_checks += 1;
        if self.control == ProcessProgress::Exited
            && self.fail_quiescence_at != Some(self.quiescence_checks)
        {
            Ok(())
        } else {
            Err(failed())
        }
    }
    fn runner(
        &mut self,
        slot: ReleaseSlot,
        identity: &ServiceInvocation,
        _: Instant,
    ) -> Result<ProcessProgress, CliFailure> {
        if slot == self.survivor {
            assert_eq!(
                identity.process_id,
                if slot == ReleaseSlot::Green { 202 } else { 201 }
            );
            self.events.push("candidate");
            if self.candidate {
                Ok(ProcessProgress::Running)
            } else {
                Err(failed())
            }
        } else {
            assert_eq!(
                identity.process_id,
                if slot == ReleaseSlot::Green { 202 } else { 201 }
            );
            self.events.push("old");
            self.old
                .pop_front()
                .expect("unexpected observation")
                .map_err(|()| failed())
        }
    }
    fn open(&mut self, identity: &ServiceInvocation) -> Result<Self::Handle, CliFailure> {
        self.events.push("open");
        if self.open_fails {
            Err(failed())
        } else {
            Ok(identity.process_id)
        }
    }
    fn interrupt(&mut self, handle: &Self::Handle) -> Result<(), CliFailure> {
        assert_eq!(
            *handle,
            if self.survivor == ReleaseSlot::Green {
                201
            } else {
                202
            }
        );
        self.events.push("signal");
        if self.signal_fails {
            Err(failed())
        } else {
            Ok(())
        }
    }
}

struct Store;
impl OnlineJournalStorage for Store {
    type Error = ();
    fn load(&self) -> Result<Option<OnlineJournal>, ()> {
        unreachable!()
    }
    fn assert_current(&mut self, _: &OnlineJournal) -> Result<(), ()> {
        unreachable!()
    }
    fn save(&mut self, _: Option<&OnlineJournal>, _: &OnlineJournal) -> Result<(), ()> {
        Ok(())
    }
}
fn journal() -> OnlineJournal {
    let plan: OnlinePlan = crate::online_journal::tests::plan();
    let journal = OnlineJournal::create(plan.clone(), &mut Store).unwrap();
    let mut value = serde_json::to_value(journal).unwrap();
    let lifecycle = |accepting| LifecycleSnapshot {
        accepting,
        stopping: false,
        revision: 1,
        in_flight: 0,
        oldest_request_age_ms: 0,
    };
    let mut opened = plan.candidate_process.clone();
    opened.lifecycle = lifecycle(true);
    let mut closed = plan.previous_process.clone();
    closed.lifecycle = lifecycle(false);
    value["phase"] = "retiring_previous".into();
    value["revision"] = 4.into();
    value["opened"] = serde_json::to_value(opened).unwrap();
    value["closed"] = serde_json::to_value(&closed).unwrap();
    value["drained"] = serde_json::to_value(closed).unwrap();
    value["cutover_started_at_ms"] = 1001.into();
    value["drain_started_at_ms"] = 1002.into();
    let result: OnlineJournal = serde_json::from_value(value).unwrap();
    assert!(result.valid());
    result
}
fn run(host: &mut Host) -> Result<ProcessProgress, CliFailure> {
    retire_with(&journal(), host, Instant::now() + Duration::from_secs(30))
}
#[test]
fn signals_only_the_pinned_handle_after_control_exit_and_second_activation_check() {
    let mut host = Host::new([
        Ok(ProcessProgress::Running),
        Ok(ProcessProgress::Running),
        Ok(ProcessProgress::Exited),
    ]);
    assert_eq!(run(&mut host).unwrap(), ProcessProgress::Exited);
    assert_eq!(
        host.events,
        [
            "control",
            "candidate",
            "old",
            "open",
            "old",
            "signal",
            "old"
        ]
    );
}
#[test]
fn control_or_candidate_failure_cannot_open_a_process_handle() {
    for control in [ProcessProgress::Running, ProcessProgress::Stopping] {
        let mut host = Host::new([]);
        host.control = control;
        assert!(run(&mut host).is_err());
        assert_eq!(host.events, ["control"]);
    }
    let mut host = Host::new([]);
    host.candidate = false;
    assert!(run(&mut host).is_err());
    assert_eq!(host.events, ["control", "candidate"]);
}
#[test]
fn replacement_before_or_after_open_never_receives_a_signal() {
    for states in [vec![Err(())], vec![Ok(ProcessProgress::Running), Err(())]] {
        let mut host = Host::new(states);
        assert!(run(&mut host).is_err());
        assert!(!host.events.contains(&"signal"));
    }
}
#[test]
fn already_exited_and_exit_during_open_are_safe_idempotent_observations() {
    for states in [
        vec![Ok(ProcessProgress::Exited)],
        vec![Ok(ProcessProgress::Running), Ok(ProcessProgress::Exited)],
    ] {
        let mut host = Host::new(states);
        assert_eq!(run(&mut host).unwrap(), ProcessProgress::Exited);
        assert!(!host.events.contains(&"signal"));
    }
}
#[test]
fn pending_or_lost_signal_reply_is_not_reported_as_completed() {
    let mut host = Host::new([
        Ok(ProcessProgress::Running),
        Ok(ProcessProgress::Running),
        Ok(ProcessProgress::Running),
    ]);
    assert_eq!(run(&mut host).unwrap(), ProcessProgress::Running);
    let mut host = Host::new([Ok(ProcessProgress::Running), Ok(ProcessProgress::Running)]);
    host.signal_fails = true;
    assert!(run(&mut host).is_err());
    host.old.push_back(Ok(ProcessProgress::Exited));
    assert_eq!(run(&mut host).unwrap(), ProcessProgress::Exited);
    assert_eq!(
        host.events
            .iter()
            .filter(|event| **event == "signal")
            .count(),
        1
    );
}
#[test]
fn unsupported_process_handles_expired_calls_and_wrong_phase_have_no_fallback() {
    let mut host = Host::new([Ok(ProcessProgress::Running)]);
    host.open_fails = true;
    assert!(run(&mut host).is_err());
    assert!(!host.events.contains(&"signal"));
    let mut host = Host::new([]);
    assert!(retire_with(&journal(), &mut host, Instant::now()).is_err());
    assert!(host.events.is_empty());
    let initial = OnlineJournal::create(crate::online_journal::tests::plan(), &mut Store).unwrap();
    assert!(
        retire_with(
            &initial,
            &mut host,
            Instant::now() + Duration::from_secs(30)
        )
        .is_err()
    );
    assert!(host.events.is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn linux_process_handle_cannot_signal_a_replacement_after_its_process_exits() {
    use std::process::{Child, Command, Stdio};
    struct OwnedChild(Child);
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            if !matches!(self.0.try_wait(), Ok(Some(_))) {
                let _ = self.0.kill();
            }
            let _ = self.0.wait();
        }
    }
    let spawn = || {
        OwnedChild(
            Command::new("sleep")
                .arg("30")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    };
    let mut old = spawn();
    let identity = ServiceInvocation {
        process_id: old.0.id(),
        invocation_id: "a".repeat(32),
    };
    let mut host = SystemdRunnerHost;
    let handle = host.open(&identity).unwrap();
    old.0.kill().unwrap();
    old.0.wait().unwrap();
    let mut replacement = spawn();
    assert!(host.interrupt(&handle).is_err());
    assert!(replacement.0.try_wait().unwrap().is_none());
    assert!(SystemdRunnerHost::capture(ReleaseSlot::Blue, Instant::now()).is_err());
    assert!(SystemdRunnerHost::retire(&journal(), Instant::now()).is_err());
}

#[test]
fn reverse_retirement_pins_candidate_and_requires_surviving_old_runner() {
    use aster_upgrade_core::online::switchback::SwitchbackPhase;
    for previous in [ReleaseSlot::Blue, ReleaseSlot::Green] {
        for fault in 0..5 {
            let (_directory, layout) = crate::proxy_disk::tests::fixture();
            let journal = crate::proxy_client::switchback_tests::journal(
                &layout,
                previous,
                SwitchbackPhase::RetiringCandidate,
            );
            let mut host = Host::new([
                Ok(ProcessProgress::Running),
                Ok(ProcessProgress::Running),
                Ok(ProcessProgress::Exited),
            ]);
            host.survivor = previous;
            match fault {
                0 => {}
                1 => host.control = ProcessProgress::Running,
                2 => host.candidate = false,
                3 => host.old = [Ok(ProcessProgress::Running), Err(())].into(),
                _ => host.old = [Ok(ProcessProgress::Exited)].into(),
            }
            // The fixture swaps slots but keeps the installation's pinned PID
            // identities, so adapt the fake host's role mapping consistently.
            let mut value = serde_json::to_value(journal).unwrap();
            if previous == ReleaseSlot::Green {
                value["plan"]["original"]["plan"]["previous_runner_process"]["process_id"] =
                    202.into();
                value["plan"]["original"]["plan"]["candidate_runner_process"]["process_id"] =
                    201.into();
            }
            let journal = serde_json::from_value(value).unwrap();
            let result = retire_switchback_with(
                &journal,
                &mut host,
                Instant::now() + Duration::from_secs(30),
            );
            assert_eq!(result.is_ok(), fault == 0 || fault == 4);
            if fault != 0 {
                assert!(!host.events.contains(&"signal"));
            }
        }
    }
}

#[test]
fn preparation_retirement_requires_durable_drain_and_pins_candidate_identity() {
    use aster_upgrade_core::preparation::{CandidateActivation, CandidateStartup};
    let (_directory, _layout, _store, initial, plan) = crate::online_journal::preparation_fixture();
    let started = initial
        .provisioned(plan.candidate.clone())
        .unwrap()
        .starting(CandidateStartup {
            material_sha256: "d".repeat(64),
            clock: plan.clock.clone(),
        })
        .unwrap()
        .started(
            CandidateActivation {
                control: plan.candidate_process.clone(),
                runner: plan.candidate_runner_process.clone(),
            },
            &"d".repeat(64),
            &plan.clock,
        )
        .unwrap();
    let claimed = started.aborting().unwrap();
    let retiring = claimed.retiring_candidate(&plan.candidate_process).unwrap();
    let deadline = || Instant::now() + Duration::from_secs(30);
    for record in [&started, &claimed, &retiring.advance_abort().unwrap()] {
        let mut host = Host::new([]);
        assert!(retire_preparation_with(record, &mut host, deadline()).is_err());
        assert!(host.events.is_empty());
    }
    for fault in 0..7 {
        let mut host = Host::new([
            Ok(ProcessProgress::Running),
            Ok(ProcessProgress::Running),
            Ok(ProcessProgress::Exited),
        ]);
        host.survivor = plan.previous.slot;
        match fault {
            0 => {}
            1 => host.control = ProcessProgress::Stopping,
            2 => host.candidate = false,
            3 => host.old = [Ok(ProcessProgress::Running), Err(())].into(),
            4 => host.old = [Ok(ProcessProgress::Exited)].into(),
            5 => {
                host.old = [
                    Ok(ProcessProgress::Running),
                    Ok(ProcessProgress::Running),
                    Ok(ProcessProgress::Stopping),
                ]
                .into()
            }
            _ => host.signal_fails = true,
        }
        let result = retire_preparation_with(&retiring, &mut host, deadline());
        assert_eq!(result.is_ok(), matches!(fault, 0 | 4 | 5));
        if matches!(fault, 1..=4) {
            assert!(!host.events.contains(&"signal"));
        }
        if fault == 5 {
            assert_eq!(result.unwrap(), ProcessProgress::Stopping);
        }
    }
}

#[test]
fn partial_runner_retirement_requires_durable_capture_and_rechecks_absent_control() {
    use aster_upgrade_core::preparation::{CandidateStartup, PartialCandidateActivation};
    let (_root, _layout, _store, initial, plan) = crate::online_journal::preparation_fixture();
    let partial = PartialCandidateActivation {
        control: None,
        runner: Some(plan.candidate_runner_process.clone()),
    };
    let captured = initial
        .provisioned(plan.candidate.clone())
        .unwrap()
        .starting(CandidateStartup {
            material_sha256: "d".repeat(64),
            clock: plan.clock.clone(),
        })
        .unwrap()
        .cancel_partial_startup(partial.clone(), &"d".repeat(64), &plan.clock)
        .unwrap();
    let claimed = captured.advance_abort().unwrap();
    let retiring = claimed.retiring_partial_candidate(&partial).unwrap();
    for record in [&captured, &claimed, &retiring] {
        let mut host = Host::new([
            Ok(ProcessProgress::Running),
            Ok(ProcessProgress::Running),
            Ok(ProcessProgress::Exited),
        ]);
        host.survivor = plan.previous.slot;
        let result = retire_partial_preparation_with(
            record,
            &mut host,
            Instant::now() + Duration::from_secs(30),
        );
        if record == &retiring {
            assert_eq!(result.unwrap(), ProcessProgress::Exited);
            assert_eq!(
                host.events,
                [
                    "quiescent-control",
                    "candidate",
                    "old",
                    "open",
                    "old",
                    "quiescent-control",
                    "signal",
                    "old"
                ]
            );
        } else {
            assert!(result.is_err());
            assert!(host.events.is_empty());
        }
    }
    for failure in 0..4 {
        let mut host = Host::new([Ok(ProcessProgress::Running), Err(())]);
        host.survivor = plan.previous.slot;
        match failure {
            0 => host.control = ProcessProgress::Running,
            1 => host.candidate = false,
            3 => {
                host.old = [Ok(ProcessProgress::Running), Ok(ProcessProgress::Running)].into();
                host.fail_quiescence_at = Some(2);
            }
            _ => {}
        }
        assert!(
            retire_partial_preparation_with(
                &retiring,
                &mut host,
                Instant::now() + Duration::from_secs(30)
            )
            .is_err()
        );
        assert!(!host.events.contains(&"signal"));
    }
}
