//! Reconcile a drained Control's private retirement command with OS evidence.
//! The online journal must persist its drained snapshot before calling this.
use std::time::Instant;

use crate::runtime::{ControlProcessObserver, ProcessProgress, RuntimeSnapshot, SlotRetirement};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlRetirementProgress {
    Waiting,
    Exited,
}

#[derive(Debug, thiserror::Error)]
pub enum RetirementError<R, P> {
    #[error("Control retirement requires a pinned, closed, drained service")]
    InvalidDrain,
    #[error("Control retirement deadline elapsed")]
    Deadline,
    #[error("Control process or closed admission revision changed")]
    RuntimeChanged,
    #[error("Control retirement command is uncertain; reconcile before retrying: {0}")]
    Runtime(R),
    #[error("Control process exit could not be proved: {0}")]
    Process(P),
}

/// One bounded step, with at most one mutation and no sleep. An acknowledgement
/// alone returns Waiting; only the observer can prove Exited. Calling again with
/// the same durable drain snapshot reconciles lost replies and executor restarts.
pub fn retire_control<R: SlotRetirement, P: ControlProcessObserver>(
    runtime: &R,
    process: &P,
    drained: &RuntimeSnapshot,
    deadline: Instant,
) -> Result<ControlRetirementProgress, RetirementError<R::Error, P::Error>> {
    if !drained.same_process(drained)
        || drained.installation_id.is_empty()
        || drained.lifecycle.accepting
        || drained.lifecycle.stopping
        || drained.lifecycle.in_flight != 0
        || drained.lifecycle.revision == u64::MAX
        || !drained
            .service
            .as_ref()
            .is_some_and(|service| service.valid())
    {
        return Err(RetirementError::InvalidDrain);
    }
    let observe = || -> Result<ProcessProgress, RetirementError<R::Error, P::Error>> {
        if Instant::now() >= deadline {
            return Err(RetirementError::Deadline);
        }
        let state = process
            .observe_control(drained, deadline)
            .map_err(RetirementError::Process)?;
        if Instant::now() >= deadline {
            return Err(RetirementError::Deadline);
        }
        Ok(state)
    };
    match observe()? {
        ProcessProgress::Exited => return Ok(ControlRetirementProgress::Exited),
        ProcessProgress::Stopping => return Ok(ControlRetirementProgress::Waiting),
        ProcessProgress::Running => {}
    }
    let observed = match runtime.status(deadline) {
        Ok(observed) => observed,
        Err(error) => return reconcile_failure(observe()?, error),
    };
    if Instant::now() >= deadline {
        return Err(RetirementError::Deadline);
    }
    if !observed.observes_retirement(drained) {
        if !observed.observes_drain(drained) || observed.lifecycle.in_flight != 0 {
            return Err(RetirementError::RuntimeChanged);
        }
        match runtime.retire_drained(drained, deadline) {
            Ok(reply) if reply.observes_retirement(drained) => {}
            Ok(_) => return Err(RetirementError::RuntimeChanged),
            Err(error) => return reconcile_failure(observe()?, error),
        }
    }
    // A lost acknowledgement may leave the runtime in stopping while systemd
    // still reports running. Do not issue another retirement or infer exit.
    Ok(match observe()? {
        ProcessProgress::Exited => ControlRetirementProgress::Exited,
        ProcessProgress::Running | ProcessProgress::Stopping => ControlRetirementProgress::Waiting,
    })
}

fn reconcile_failure<R, P>(
    observed: ProcessProgress,
    error: R,
) -> Result<ControlRetirementProgress, RetirementError<R, P>> {
    match observed {
        ProcessProgress::Exited => Ok(ControlRetirementProgress::Exited),
        ProcessProgress::Stopping => Ok(ControlRetirementProgress::Waiting),
        ProcessProgress::Running => Err(RetirementError::Runtime(error)),
    }
}

#[cfg(test)]
mod tests;
