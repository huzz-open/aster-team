//! Retirement is tied to a service activation and a kernel process handle.
//! Never fall back to killing a PID or stopping a unit after an uncertain read.
use std::time::Instant;

use aster_error_catalog::delivery;
use aster_upgrade_core::{
    ReleaseSlot,
    online::{OnlineJournal, OnlinePhase},
    runtime::{ProcessProgress, RuntimeSnapshot, ServiceInvocation},
};

use super::CliFailure;

fn failed() -> CliFailure {
    CliFailure::new(
        delivery::UPGRADE_FAILED,
        "Runner retirement could not prove the pinned process state; preserve the online transition",
    )
}

fn before(deadline: Instant) -> Result<(), CliFailure> {
    if Instant::now() >= deadline {
        Err(failed())
    } else {
        Ok(())
    }
}

trait RunnerHost {
    type Handle;
    fn control(
        &mut self,
        drained: &RuntimeSnapshot,
        deadline: Instant,
    ) -> Result<ProcessProgress, CliFailure>;
    fn quiescent_control(&mut self, slot: ReleaseSlot, deadline: Instant)
    -> Result<(), CliFailure>;
    fn runner(
        &mut self,
        slot: ReleaseSlot,
        identity: &ServiceInvocation,
        deadline: Instant,
    ) -> Result<ProcessProgress, CliFailure>;
    fn open(&mut self, identity: &ServiceInvocation) -> Result<Self::Handle, CliFailure>;
    fn interrupt(&mut self, handle: &Self::Handle) -> Result<(), CliFailure>;
}

/// Only the old Control's completed exit authorizes retiring its local Runner.
/// A signal acknowledgement is not exit proof; callers must preserve the journal
/// and retry observation when this returns Running or Stopping.
fn retire_with<H: RunnerHost>(
    journal: &OnlineJournal,
    host: &mut H,
    deadline: Instant,
) -> Result<ProcessProgress, CliFailure> {
    before(deadline)?;
    if !journal.valid() || journal.phase() != OnlinePhase::RetiringPrevious {
        return Err(failed());
    }
    let plan = journal.plan();
    retire_pinned(
        host,
        ControlAbsence::Exited(journal.drained().ok_or_else(failed)?),
        (plan.candidate.slot, &plan.candidate_runner_process),
        (plan.previous.slot, &plan.previous_runner_process),
        deadline,
    )
}

enum ControlAbsence<'a> {
    Exited(&'a RuntimeSnapshot),
    NeverCaptured(ReleaseSlot),
}

fn retire_pinned<H: RunnerHost>(
    host: &mut H,
    control: ControlAbsence<'_>,
    survivor: (ReleaseSlot, &ServiceInvocation),
    retiring: (ReleaseSlot, &ServiceInvocation),
    deadline: Instant,
) -> Result<ProcessProgress, CliFailure> {
    before(deadline)?;
    match control {
        ControlAbsence::Exited(drained) => {
            if host.control(drained, deadline)? != ProcessProgress::Exited {
                return Err(failed());
            }
        }
        ControlAbsence::NeverCaptured(slot) => host.quiescent_control(slot, deadline)?,
    }
    before(deadline)?;
    if host.runner(survivor.0, survivor.1, deadline)? != ProcessProgress::Running {
        return Err(failed());
    }
    before(deadline)?;
    let observe = |host: &mut H| host.runner(retiring.0, retiring.1, deadline);
    let state = observe(host)?;
    before(deadline)?;
    if state != ProcessProgress::Running {
        return Ok(state);
    }
    // Opening can race exit/restart. Check the activation again *after* opening;
    // the kernel handle then prevents PID reuse between the check and signal.
    let handle = host.open(retiring.1)?;
    before(deadline)?;
    let state = observe(host)?;
    before(deadline)?;
    if state != ProcessProgress::Running {
        return Ok(state);
    }
    if let ControlAbsence::NeverCaptured(slot) = control {
        host.quiescent_control(slot, deadline)?;
        before(deadline)?;
    }
    host.interrupt(&handle)?;
    before(deadline)?;
    let state = observe(host)?;
    before(deadline)?;
    Ok(state)
}

fn retire_switchback_with<H: RunnerHost>(
    journal: &aster_upgrade_core::online::switchback::SwitchbackJournal,
    host: &mut H,
    deadline: Instant,
) -> Result<ProcessProgress, CliFailure> {
    use aster_upgrade_core::online::switchback::SwitchbackPhase;
    if !journal.valid() || journal.phase() != SwitchbackPhase::RetiringCandidate {
        return Err(failed());
    }
    let plan = journal.plan().original.plan();
    retire_pinned(
        host,
        ControlAbsence::Exited(journal.drained().ok_or_else(failed)?),
        (plan.previous.slot, &plan.previous_runner_process),
        (plan.candidate.slot, &plan.candidate_runner_process),
        deadline,
    )
}

fn retire_preparation_with<H: RunnerHost>(
    journal: &aster_upgrade_core::preparation::PreparationJournal,
    host: &mut H,
    deadline: Instant,
) -> Result<ProcessProgress, CliFailure> {
    use aster_upgrade_core::preparation::PreparationAbortPhase;
    if !journal.valid() || journal.abort_phase() != Some(PreparationAbortPhase::RetiringCandidate) {
        return Err(failed());
    }
    let activation = journal.activation().ok_or_else(failed)?;
    retire_pinned(
        host,
        ControlAbsence::Exited(journal.abort_drained().ok_or_else(failed)?),
        (
            journal.intent().previous.slot,
            &journal.intent().previous_runner_process,
        ),
        (activation.control.slot, &activation.runner),
        deadline,
    )
}

fn retire_partial_preparation_with<H: RunnerHost>(
    journal: &aster_upgrade_core::preparation::PreparationJournal,
    host: &mut H,
    deadline: Instant,
) -> Result<ProcessProgress, CliFailure> {
    use aster_upgrade_core::preparation::PreparationAbortPhase;
    if !journal.valid() || journal.abort_phase() != Some(PreparationAbortPhase::RetiringCandidate) {
        return Err(failed());
    }
    let partial = journal.partial_activation().ok_or_else(failed)?;
    if partial.control.is_some() {
        return Err(failed());
    }
    let runner = partial.runner.as_ref().ok_or_else(failed)?;
    let slot = journal.candidate().ok_or_else(failed)?.slot;
    retire_pinned(
        host,
        ControlAbsence::NeverCaptured(slot),
        (
            journal.intent().previous.slot,
            &journal.intent().previous_runner_process,
        ),
        (slot, runner),
        deadline,
    )
}

#[cfg(target_os = "linux")]
pub(super) struct SystemdRunnerHost;

#[cfg(target_os = "linux")]
impl SystemdRunnerHost {
    pub(super) fn retire_preparation(
        journal: &aster_upgrade_core::preparation::PreparationJournal,
        deadline: Instant,
    ) -> Result<ProcessProgress, CliFailure> {
        if journal.partial_activation().is_some() {
            retire_partial_preparation_with(journal, &mut Self, deadline)
        } else {
            retire_preparation_with(journal, &mut Self, deadline)
        }
    }

    pub(super) fn retire_switchback(
        journal: &aster_upgrade_core::online::switchback::SwitchbackJournal,
        deadline: Instant,
    ) -> Result<ProcessProgress, CliFailure> {
        retire_switchback_with(journal, &mut Self, deadline)
    }

    pub(super) fn capture(
        slot: ReleaseSlot,
        deadline: Instant,
    ) -> Result<ServiceInvocation, CliFailure> {
        let (identity, state) = Self::snapshot(slot, deadline)?;
        if state != ProcessProgress::Running {
            return Err(failed());
        }
        Ok(identity)
    }

    fn snapshot(
        slot: ReleaseSlot,
        deadline: Instant,
    ) -> Result<(ServiceInvocation, ProcessProgress), CliFailure> {
        before(deadline)?;
        let unit = format!("aster-runner@{}.service", slot.id());
        let source = super::control_process::show_service(&unit, deadline)?;
        let value = super::control_process::service_snapshot(
            std::str::from_utf8(&source).map_err(|_| failed())?,
            &unit,
        )?;
        before(deadline)?;
        Ok(value)
    }

    pub(super) fn retire(
        journal: &OnlineJournal,
        deadline: Instant,
    ) -> Result<ProcessProgress, CliFailure> {
        retire_with(journal, &mut Self, deadline)
    }
}

#[cfg(target_os = "linux")]
impl RunnerHost for SystemdRunnerHost {
    type Handle = rustix::fd::OwnedFd;

    fn control(
        &mut self,
        drained: &RuntimeSnapshot,
        deadline: Instant,
    ) -> Result<ProcessProgress, CliFailure> {
        use aster_upgrade_core::runtime::ControlProcessObserver as _;
        super::control_process::SystemdControlObserver.observe_control(drained, deadline)
    }

    fn quiescent_control(
        &mut self,
        slot: ReleaseSlot,
        deadline: Instant,
    ) -> Result<(), CliFailure> {
        super::control_process::quiescence::observe(
            &format!("aster-control@{}.service", slot.id()),
            deadline,
        )?;
        Ok(())
    }

    fn runner(
        &mut self,
        slot: ReleaseSlot,
        expected: &ServiceInvocation,
        deadline: Instant,
    ) -> Result<ProcessProgress, CliFailure> {
        let (identity, state) = Self::snapshot(slot, deadline)?;
        if &identity != expected {
            return Err(failed());
        }
        Ok(state)
    }

    fn open(&mut self, identity: &ServiceInvocation) -> Result<Self::Handle, CliFailure> {
        use rustix::process::{Pid, PidfdFlags, pidfd_open};
        let pid = i32::try_from(identity.process_id)
            .ok()
            .and_then(Pid::from_raw)
            .ok_or_else(failed)?;
        // Empty flags support the documented Linux 5.4 baseline; NONBLOCK was
        // added later. Unsupported kernels fail closed, with no PID fallback.
        pidfd_open(pid, PidfdFlags::empty()).map_err(|_| failed())
    }

    fn interrupt(&mut self, handle: &Self::Handle) -> Result<(), CliFailure> {
        rustix::process::pidfd_send_signal(handle, rustix::process::Signal::INT)
            .map_err(|_| failed())
    }
}

#[cfg(test)]
mod tests;
