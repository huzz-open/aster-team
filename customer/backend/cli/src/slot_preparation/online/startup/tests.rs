use super::*;

struct Fake {
    clock: UpgradeClock,
    material: String,
    activation: CandidateActivation,
    actions: Vec<(ReleaseSlot, Action)>,
    pending: usize,
    partial: Option<PartialCandidateActivation>,
    fail_start: bool,
    changed_after_start: bool,
    captures: usize,
    verifies: usize,
    replacement_on_second_capture: bool,
    material_change_on_verify: Option<usize>,
    capture_error_at: Option<usize>,
    clock_after_capture: Option<UpgradeClock>,
}
impl Host for Fake {
    fn clock(&mut self) -> Result<UpgradeClock, CliFailure> {
        Ok(self.clock.clone())
    }
    fn verify(&mut self, _: &PreparationJournal, _: Instant) -> Result<String, CliFailure> {
        self.verifies += 1;
        if self.material_change_on_verify == Some(self.verifies) {
            self.material = "e".repeat(64);
        }
        Ok(
            if self.changed_after_start
                && self
                    .actions
                    .iter()
                    .any(|(_, action)| *action == Action::Start)
            {
                "e".repeat(64)
            } else {
                self.material.clone()
            },
        )
    }
    fn dispatch(
        &mut self,
        slot: ReleaseSlot,
        action: Action,
        _: Instant,
    ) -> Result<(), CliFailure> {
        self.actions.push((slot, action));
        if self.fail_start && action == Action::Start {
            return Err(failed("lost start acknowledgement"));
        }
        Ok(())
    }
    fn capture(
        &mut self,
        _: &PreparationJournal,
        _: Instant,
    ) -> Result<Option<CandidateActivation>, CliFailure> {
        self.captures += 1;
        if let Some(clock) = self.clock_after_capture.take() {
            self.clock = clock;
        }
        if self.capture_error_at == Some(self.captures) {
            return Err(failed("candidate observation failed"));
        }
        if self.replacement_on_second_capture && self.captures == 2 {
            self.activation.runner.process_id += 1;
        }
        if self.pending > 0 {
            self.pending -= 1;
            Ok(None)
        } else {
            Ok(Some(self.activation.clone()))
        }
    }
    fn capture_partial(
        &mut self,
        _: &PreparationJournal,
        _: Instant,
    ) -> Result<PartialCandidateActivation, CliFailure> {
        self.partial
            .clone()
            .ok_or_else(|| failed("candidate quiescence unproven"))
    }
    fn pause(&mut self, _: Instant) {
        self.clock.uptime_ms += 45_000;
    }
}

fn fixture() -> (
    tempfile::TempDir,
    InstallLayout,
    JournalFile,
    PreparationJournal,
    Fake,
) {
    let (root, layout, mut store, initial, plan) = crate::online_journal::preparation_fixture();
    store.save_preparation(None, &initial).unwrap();
    let prepared = initial.provisioned(plan.candidate.clone()).unwrap();
    store.save_preparation(Some(&initial), &prepared).unwrap();
    let starting = prepared
        .starting(CandidateStartup {
            material_sha256: "d".repeat(64),
            clock: plan.clock.clone(),
        })
        .unwrap();
    store.save_preparation(Some(&prepared), &starting).unwrap();
    let host = Fake {
        clock: plan.clock,
        material: "d".repeat(64),
        activation: CandidateActivation {
            control: plan.candidate_process,
            runner: plan.candidate_runner_process,
        },
        actions: vec![],
        pending: 0,
        partial: None,
        fail_start: false,
        changed_after_start: false,
        captures: 0,
        verifies: 0,
        replacement_on_second_capture: false,
        material_change_on_verify: None,
        capture_error_at: None,
        clock_after_capture: None,
    };
    (root, layout, store, starting, host)
}

#[test]
fn startup_persists_actual_activation_and_reopen_never_dispatches_again() {
    let (_root, layout, mut store, starting, mut host) = fixture();
    host.pending = 1;
    let started = drive(&mut store, starting.clone(), &mut host).unwrap();
    assert_eq!(
        host.actions,
        [
            (ReleaseSlot::Green, Action::Enable),
            (ReleaseSlot::Green, Action::Start)
        ]
    );
    assert_eq!(started.startup(), starting.startup());
    assert_eq!(started.activation(), Some(&host.activation));
    drop(store);
    let mut store = JournalFile::open(&layout).unwrap();
    host.actions.clear();
    assert_eq!(
        drive(&mut store, started.clone(), &mut host).unwrap(),
        started
    );
    assert!(host.actions.is_empty());
}

#[test]
fn lost_start_acknowledgement_preserves_original_intent_for_retry() {
    let (_root, _layout, mut store, starting, mut host) = fixture();
    host.fail_start = true;
    assert!(drive(&mut store, starting.clone(), &mut host).is_err());
    assert_eq!(store.load_preparation().unwrap(), Some(starting.clone()));
    host.fail_start = false;
    host.clock.uptime_ms += 1000;
    let started = drive(&mut store, starting.clone(), &mut host).unwrap();
    assert_eq!(started.startup(), starting.startup());
}

#[test]
fn changed_material_boot_and_expired_budget_dispatch_nothing() {
    for variant in 0..3 {
        let (_root, _layout, mut store, starting, mut host) = fixture();
        match variant {
            0 => host.material = "e".repeat(64),
            1 => host.clock.boot_id = "different-boot".into(),
            _ => host.clock.uptime_ms += CandidateStartup::BUDGET_MS,
        }
        assert!(drive(&mut store, starting.clone(), &mut host).is_err());
        assert!(host.actions.is_empty());
        assert_eq!(store.load_preparation().unwrap(), Some(starting));
    }
}

#[test]
fn material_change_after_dispatch_and_deadline_during_wait_never_record_activation() {
    for variant in 0..2 {
        let (_root, _layout, mut store, starting, mut host) = fixture();
        if variant == 0 {
            host.changed_after_start = true;
        } else {
            host.clock.uptime_ms += CandidateStartup::BUDGET_MS - 30_000;
            host.pending = 1;
        }
        assert!(drive(&mut store, starting.clone(), &mut host).is_err());
        assert_eq!(
            host.actions,
            [
                (ReleaseSlot::Green, Action::Enable),
                (ReleaseSlot::Green, Action::Start)
            ]
        );
        assert_eq!(store.load_preparation().unwrap(), Some(starting));
    }
}

#[test]
fn replacement_or_missing_recorded_candidate_is_never_restarted() {
    for missing in [false, true] {
        let (_root, _layout, mut store, starting, mut host) = fixture();
        let started = drive(&mut store, starting, &mut host).unwrap();
        host.actions.clear();
        if missing {
            host.pending = 1;
        } else {
            host.activation.runner.process_id += 1;
        }
        assert!(drive(&mut store, started.clone(), &mut host).is_err());
        assert!(host.actions.is_empty());
        assert_eq!(store.load_preparation().unwrap(), Some(started));
    }
}

#[test]
fn open_candidate_and_stale_writer_cannot_publish_activation() {
    let (_root, _layout, mut store, starting, mut host) = fixture();
    host.activation.control.lifecycle.accepting = true;
    assert!(drive(&mut store, starting.clone(), &mut host).is_err());
    assert_eq!(store.load_preparation().unwrap(), Some(starting.clone()));
    host.activation.control.lifecycle.accepting = false;
    let started = drive(&mut store, starting.clone(), &mut host).unwrap();
    host.actions.clear();
    assert!(drive(&mut store, starting, &mut host).is_err());
    assert!(host.actions.is_empty());
    assert_eq!(store.load_preparation().unwrap(), Some(started));
}

#[test]
fn expired_startup_is_observed_only_for_cancellation_without_dispatch_or_budget_reset() {
    for elapsed in [
        0,
        CandidateStartup::BUDGET_MS,
        CandidateStartup::BUDGET_MS + 300_000,
    ] {
        let (_root, layout, mut store, starting, mut host) = fixture();
        host.clock.uptime_ms += elapsed;
        let cancelled = observe_cancellation(
            &store,
            &starting,
            &mut host,
            Instant::now() + Duration::from_secs(30),
        )
        .unwrap();
        assert!(host.actions.is_empty());
        assert_eq!(host.captures, 2);
        assert_eq!(host.verifies, 3);
        assert_eq!(cancelled.startup(), starting.startup());
        assert_eq!(cancelled.activation(), Some(&host.activation));
        assert_eq!(
            cancelled.abort_phase(),
            Some(aster_upgrade_core::preparation::PreparationAbortPhase::CapturedForCancellation)
        );
        // Observation cannot claim ownership or mutate the old checkpoint itself.
        assert_eq!(store.load_preparation().unwrap(), Some(starting.clone()));
        store.save_preparation(Some(&starting), &cancelled).unwrap();
        drop(store);
        let store = JournalFile::open(&layout).unwrap();
        assert_eq!(store.load_preparation().unwrap(), Some(cancelled.clone()));
        assert!(
            observe_cancellation(
                &store,
                &starting,
                &mut host,
                Instant::now() + Duration::from_secs(30)
            )
            .is_err()
        );
        assert!(host.actions.is_empty());
    }
}

#[test]
fn uncertain_or_changed_recovery_observations_never_write_or_dispatch() {
    for variant in 0..12 {
        let (_root, _layout, store, starting, mut host) = fixture();
        host.clock.uptime_ms += CandidateStartup::BUDGET_MS + 1;
        let mut deadline = Instant::now() + Duration::from_secs(30);
        match variant {
            0 => host.pending = 1,
            1 => host.replacement_on_second_capture = true,
            2 => host.material = "e".repeat(64),
            3 => host.material_change_on_verify = Some(2),
            4 => host.material_change_on_verify = Some(3),
            5 => host.capture_error_at = Some(2),
            6 => host.activation.control.lifecycle.in_flight = 1,
            7 => host.activation.control.lifecycle.accepting = true,
            8 => host.activation.control.lifecycle.stopping = true,
            9 => {
                let mut clock = host.clock.clone();
                clock.boot_id = "reboot".into();
                host.clock_after_capture = Some(clock);
            }
            10 => {
                let mut clock = host.clock.clone();
                clock.uptime_ms -= 1;
                host.clock_after_capture = Some(clock);
            }
            _ => deadline = Instant::now(),
        }
        assert!(
            observe_cancellation(&store, &starting, &mut host, deadline).is_err(),
            "case {variant}"
        );
        assert!(host.actions.is_empty());
        assert_eq!(store.load_preparation().unwrap(), Some(starting));
    }
}

#[test]
fn partial_startup_observation_is_cancel_only_and_rejects_late_completion_or_uncertainty() {
    for variant in 0..3 {
        for fault in 0..5 {
            let (_root, layout, mut store, starting, mut host) = fixture();
            let partial = PartialCandidateActivation {
                control: (variant == 0).then(|| host.activation.control.clone()),
                runner: (variant == 1).then(|| host.activation.runner.clone()),
            };
            host.partial = Some(partial.clone());
            host.pending = 2;
            host.clock.uptime_ms += CandidateStartup::BUDGET_MS + 1;
            match fault {
                0 => {}
                1 => host.pending = 1, // Second observation sees both services: preserve journal.
                2 => host.partial = None, // No quiescence evidence.
                3 => host.material_change_on_verify = Some(3),
                _ => host.capture_error_at = Some(2),
            }
            let result = observe_cancellation(
                &store,
                &starting,
                &mut host,
                Instant::now() + Duration::from_secs(30),
            );
            assert!(host.actions.is_empty());
            assert_eq!(store.load_preparation().unwrap(), Some(starting.clone()));
            if fault == 0 {
                let captured = result.unwrap();
                assert_eq!(captured.partial_activation(), Some(&partial));
                assert!(captured.activation().is_none());
                store.save_preparation(Some(&starting), &captured).unwrap();
                drop(store);
                let mut reopened = JournalFile::open(&layout).unwrap();
                assert_eq!(reopened.load_preparation().unwrap(), Some(captured.clone()));
                assert!(drive(&mut reopened, captured, &mut host).is_err());
                assert!(host.actions.is_empty());
            } else {
                assert!(result.is_err(), "variant {variant} fault {fault}");
            }
        }
    }
}
