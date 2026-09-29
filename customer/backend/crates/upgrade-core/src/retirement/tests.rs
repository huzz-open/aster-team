use super::*;
use crate::{
    ReleaseSlot,
    runtime::{
        DrainProgress, LifecycleSnapshot, ReadinessExpectation, ServiceInvocation, SlotRuntime,
    },
};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    time::Duration,
};

fn drained() -> RuntimeSnapshot {
    RuntimeSnapshot {
        schema: "aster.control-runtime.v1".into(),
        installation_id: "installation".into(),
        slot: ReleaseSlot::Blue,
        instance_id: "instance-old".into(),
        service: Some(ServiceInvocation {
            process_id: 123,
            invocation_id: "a".repeat(32),
        }),
        product_version: "2.1.0".into(),
        lifecycle: LifecycleSnapshot {
            accepting: false,
            stopping: false,
            revision: 4,
            in_flight: 0,
            oldest_request_age_ms: 0,
        },
    }
}

struct Runtime {
    state: RefCell<RuntimeSnapshot>,
    status_calls: Cell<usize>,
    retire_calls: Cell<usize>,
    unavailable: Cell<bool>,
    lose_reply: Cell<bool>,
    bad_reply: Cell<bool>,
}
impl Runtime {
    fn new() -> Self {
        Self {
            state: RefCell::new(drained()),
            status_calls: Cell::new(0),
            retire_calls: Cell::new(0),
            unavailable: Cell::new(false),
            lose_reply: Cell::new(false),
            bad_reply: Cell::new(false),
        }
    }
}
impl SlotRuntime for Runtime {
    type Error = &'static str;
    type ReadyPermit = ();
    fn status(&self, _: Instant) -> Result<RuntimeSnapshot, Self::Error> {
        self.status_calls.set(self.status_calls.get() + 1);
        if self.unavailable.get() {
            Err("unavailable")
        } else {
            Ok(self.state.borrow().clone())
        }
    }
    fn readiness(
        &self,
        _: &RuntimeSnapshot,
        _: &ReadinessExpectation,
        _: Instant,
    ) -> Result<(), Self::Error> {
        panic!("must not probe or reopen during retirement")
    }
    fn open_admission(&self, _: (), _: Instant) -> Result<RuntimeSnapshot, Self::Error> {
        panic!("must not reopen")
    }
    fn close_admission(
        &self,
        _: &RuntimeSnapshot,
        _: Instant,
    ) -> Result<RuntimeSnapshot, Self::Error> {
        panic!("must not close again")
    }
    fn observe_drain(&self, _: &RuntimeSnapshot, _: Instant) -> Result<DrainProgress, Self::Error> {
        panic!("must use durable drain revision")
    }
}
impl SlotRetirement for Runtime {
    fn retire_drained(
        &self,
        expected: &RuntimeSnapshot,
        _: Instant,
    ) -> Result<RuntimeSnapshot, Self::Error> {
        self.retire_calls.set(self.retire_calls.get() + 1);
        let mut state = self.state.borrow_mut();
        assert!(state.observes_drain(expected));
        state.lifecycle.stopping = true;
        state.lifecycle.revision += 1;
        if self.lose_reply.get() {
            return Err("lost acknowledgement");
        }
        let mut reply = state.clone();
        if self.bad_reply.get() {
            reply.instance_id = "replacement".into();
        }
        Ok(reply)
    }
}

struct Observer(RefCell<VecDeque<Result<ProcessProgress, &'static str>>>);
impl Observer {
    fn new(states: &[ProcessProgress]) -> Self {
        Self(RefCell::new(states.iter().copied().map(Ok).collect()))
    }
}
impl ControlProcessObserver for Observer {
    type Error = &'static str;
    fn observe_control(
        &self,
        expected: &RuntimeSnapshot,
        deadline: Instant,
    ) -> Result<ProcessProgress, Self::Error> {
        assert!(deadline > Instant::now());
        assert_eq!(expected, &drained());
        self.0
            .borrow_mut()
            .pop_front()
            .expect("unexpected extra observation")
    }
}
fn step(
    runtime: &Runtime,
    observer: &Observer,
) -> Result<ControlRetirementProgress, RetirementError<&'static str, &'static str>> {
    retire_control(
        runtime,
        observer,
        &drained(),
        Instant::now() + Duration::from_secs(2),
    )
}

#[test]
fn acknowledgement_waits_for_os_exit_and_recovery_does_not_resend() {
    let runtime = Runtime::new();
    let observer = Observer::new(&[
        ProcessProgress::Running,
        ProcessProgress::Running,
        ProcessProgress::Running,
        ProcessProgress::Stopping,
        ProcessProgress::Exited,
    ]);
    assert_eq!(
        step(&runtime, &observer).unwrap(),
        ControlRetirementProgress::Waiting
    );
    assert_eq!(
        step(&runtime, &observer).unwrap(),
        ControlRetirementProgress::Waiting
    );
    assert_eq!(
        step(&runtime, &observer).unwrap(),
        ControlRetirementProgress::Exited
    );
    assert_eq!(runtime.retire_calls.get(), 1);
    assert_eq!(runtime.status_calls.get(), 2);
    assert!(observer.0.borrow().is_empty());
}

#[test]
fn lost_acknowledgement_is_reconciled_without_a_second_mutation() {
    for outcome in [
        ProcessProgress::Running,
        ProcessProgress::Stopping,
        ProcessProgress::Exited,
    ] {
        let runtime = Runtime::new();
        runtime.lose_reply.set(true);
        let observer = Observer::new(&[ProcessProgress::Running, outcome]);
        let result = step(&runtime, &observer);
        match outcome {
            ProcessProgress::Running => assert!(matches!(result, Err(RetirementError::Runtime(_)))),
            ProcessProgress::Stopping => {
                assert_eq!(result.unwrap(), ControlRetirementProgress::Waiting)
            }
            ProcessProgress::Exited => {
                assert_eq!(result.unwrap(), ControlRetirementProgress::Exited)
            }
        }
        assert_eq!(runtime.retire_calls.get(), 1);
        let observer = Observer::new(&[ProcessProgress::Exited]);
        assert_eq!(
            step(&runtime, &observer).unwrap(),
            ControlRetirementProgress::Exited
        );
        assert_eq!(runtime.retire_calls.get(), 1);
    }
}

#[test]
fn disappearing_listener_requires_independent_process_evidence() {
    for outcome in [
        ProcessProgress::Running,
        ProcessProgress::Stopping,
        ProcessProgress::Exited,
    ] {
        let runtime = Runtime::new();
        runtime.unavailable.set(true);
        let result = step(
            &runtime,
            &Observer::new(&[ProcessProgress::Running, outcome]),
        );
        match outcome {
            ProcessProgress::Running => assert!(matches!(result, Err(RetirementError::Runtime(_)))),
            ProcessProgress::Stopping => {
                assert_eq!(result.unwrap(), ControlRetirementProgress::Waiting)
            }
            ProcessProgress::Exited => {
                assert_eq!(result.unwrap(), ControlRetirementProgress::Exited)
            }
        }
        assert_eq!(runtime.retire_calls.get(), 0);
    }
}

#[test]
fn stale_runtime_and_mismatched_acknowledgement_cannot_finish_retirement() {
    for fault in 0..5 {
        let runtime = Runtime::new();
        {
            let mut state = runtime.state.borrow_mut();
            match fault {
                0 => state.instance_id = "replacement".into(),
                1 => state.lifecycle.accepting = true,
                2 => state.lifecycle.in_flight = 1,
                3 => state.lifecycle.revision += 1,
                _ => runtime.bad_reply.set(true),
            }
        }
        assert!(matches!(
            step(&runtime, &Observer::new(&[ProcessProgress::Running])),
            Err(RetirementError::RuntimeChanged)
        ));
        assert_eq!(runtime.retire_calls.get(), usize::from(fault == 4));
    }
}

#[test]
fn invalid_drain_or_expired_step_never_queries_or_stops_a_process() {
    let runtime = Runtime::new();
    let observer = Observer::new(&[]);
    for fault in 0..5 {
        let mut invalid = drained();
        match fault {
            0 => invalid.lifecycle.accepting = true,
            1 => invalid.lifecycle.stopping = true,
            2 => invalid.lifecycle.in_flight = 1,
            3 => invalid.lifecycle.revision = u64::MAX,
            _ => invalid.service = None,
        }
        assert!(matches!(
            retire_control(
                &runtime,
                &observer,
                &invalid,
                Instant::now() + Duration::from_secs(1)
            ),
            Err(RetirementError::InvalidDrain)
        ));
    }
    assert!(matches!(
        retire_control(&runtime, &observer, &drained(), Instant::now()),
        Err(RetirementError::Deadline)
    ));
    assert_eq!(runtime.status_calls.get(), 0);
    assert_eq!(runtime.retire_calls.get(), 0);
}

#[test]
fn failed_process_observation_preserves_uncertainty() {
    let runtime = Runtime::new();
    let observer = Observer(RefCell::new(VecDeque::from([Err("service replaced")])));
    assert!(matches!(
        step(&runtime, &observer),
        Err(RetirementError::Process(_))
    ));
    assert_eq!(runtime.status_calls.get(), 0);
    assert_eq!(runtime.retire_calls.get(), 0);
}
