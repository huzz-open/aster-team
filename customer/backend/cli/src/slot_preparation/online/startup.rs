//! Start only the prepared candidate. A lost acknowledgement is reconciled
//! against the same material, boot budget and durable activation.
use super::*;
use aster_upgrade_core::{
    preparation::{CandidateActivation, PartialCandidateActivation},
    runtime::{ControlProcessObserver, ProcessProgress, UpgradeClock},
};

mod installed;
#[cfg(test)]
mod tests;

trait Host {
    fn clock(&mut self) -> Result<UpgradeClock, CliFailure>;
    fn verify(
        &mut self,
        journal: &PreparationJournal,
        deadline: Instant,
    ) -> Result<String, CliFailure>;
    fn dispatch(
        &mut self,
        slot: ReleaseSlot,
        action: Action,
        deadline: Instant,
    ) -> Result<(), CliFailure>;
    fn capture(
        &mut self,
        journal: &PreparationJournal,
        deadline: Instant,
    ) -> Result<Option<CandidateActivation>, CliFailure>;
    fn capture_partial(
        &mut self,
        journal: &PreparationJournal,
        deadline: Instant,
    ) -> Result<PartialCandidateActivation, CliFailure>;
    fn pause(&mut self, deadline: Instant);
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    Enable,
    Start,
}

pub(super) fn run(
    layout: &InstallLayout,
    store: &mut JournalFile,
    journal: PreparationJournal,
) -> Result<PreparationJournal, CliFailure> {
    drive(
        store,
        journal,
        &mut installed::Installed {
            layout,
            forward: true,
        },
    )
}

/// Observe without enabling, starting, or extending the original startup. The
/// caller performs the cancellation CAS after checking old-slot readiness.
pub(super) fn capture_for_cancellation(
    layout: &InstallLayout,
    store: &JournalFile,
    journal: &PreparationJournal,
    deadline: Instant,
) -> Result<PreparationJournal, CliFailure> {
    observe_cancellation(
        store,
        journal,
        &mut installed::Installed {
            layout,
            forward: false,
        },
        deadline,
    )
}

fn observe_cancellation(
    store: &JournalFile,
    journal: &PreparationJournal,
    host: &mut impl Host,
    deadline: Instant,
) -> Result<PreparationJournal, CliFailure> {
    if !journal.valid()
        || journal.startup().is_none()
        || journal.activation().is_some()
        || journal.abort_phase().is_some()
    {
        return Err(failed("startup has no uncaptured candidate to reconcile"));
    }
    let verify = |host: &mut dyn Host| -> Result<(String, UpgradeClock), CliFailure> {
        current(store, journal)?;
        if Instant::now() >= deadline {
            return Err(failed("cancellation observation expired"));
        }
        let material = host.verify(journal, deadline)?;
        let clock = host.clock()?;
        let startup = journal
            .startup()
            .ok_or_else(|| failed("startup intent missing"))?;
        if material != startup.material_sha256
            || clock.boot_id != startup.clock.boot_id
            || clock.uptime_ms < startup.clock.uptime_ms
            || Instant::now() >= deadline
        {
            return Err(failed(
                "cancellation startup material, boot or observation changed",
            ));
        }
        current(store, journal)?;
        Ok((material, clock))
    };
    let (_, first_clock) = verify(host)?;
    let activation = capture_cancellation(host, journal, deadline)?;
    let (_, second_clock) = verify(host)?;
    if second_clock.uptime_ms < first_clock.uptime_ms
        || capture_cancellation(host, journal, deadline)? != activation
    {
        return Err(failed("candidate changed during cancellation observation"));
    }
    let (material, clock) = verify(host)?;
    if clock.uptime_ms < second_clock.uptime_ms {
        return Err(failed("cancellation clock moved backwards"));
    }
    match activation {
        CancellationCapture::Complete(activation) => {
            journal.cancel_observed_startup(activation, &material, &clock)
        }
        CancellationCapture::Partial(partial) => {
            journal.cancel_partial_startup(partial, &material, &clock)
        }
    }
    .ok_or_else(|| failed("observed startup cannot authorize cancellation"))
}

#[derive(PartialEq)]
enum CancellationCapture {
    Complete(CandidateActivation),
    Partial(PartialCandidateActivation),
}

fn capture_cancellation(
    host: &mut impl Host,
    journal: &PreparationJournal,
    deadline: Instant,
) -> Result<CancellationCapture, CliFailure> {
    match host.capture(journal, deadline)? {
        Some(activation) => Ok(CancellationCapture::Complete(activation)),
        None => host
            .capture_partial(journal, deadline)
            .map(CancellationCapture::Partial),
    }
}

pub(super) fn capture_partial(
    layout: &InstallLayout,
    journal: &PreparationJournal,
    deadline: Instant,
) -> Result<PartialCandidateActivation, CliFailure> {
    installed::Installed {
        layout,
        forward: false,
    }
    .capture_partial(journal, deadline)
}

fn current(store: &JournalFile, journal: &PreparationJournal) -> Result<(), CliFailure> {
    if journal.abort_phase().is_some()
        || store.load()?.is_some()
        || store.load_preparation()?.as_ref() != Some(journal)
    {
        return Err(failed(
            "candidate startup no longer owns the preparation record",
        ));
    }
    Ok(())
}

fn guard(
    store: &JournalFile,
    journal: &PreparationJournal,
    host: &mut impl Host,
    deadline: Instant,
) -> Result<(String, UpgradeClock), CliFailure> {
    current(store, journal)?;
    if Instant::now() >= deadline {
        return Err(failed("candidate startup budget exhausted"));
    }
    let material = host.verify(journal, deadline)?;
    let clock = host.clock()?;
    if !journal
        .startup()
        .is_some_and(|startup| startup.accepts_observation(&material, &clock))
        || Instant::now() >= deadline
    {
        return Err(failed(
            "candidate startup material, boot or original deadline changed",
        ));
    }
    current(store, journal)?;
    Ok((material, clock))
}

fn drive(
    store: &mut JournalFile,
    journal: PreparationJournal,
    host: &mut impl Host,
) -> Result<PreparationJournal, CliFailure> {
    if !journal.valid() {
        return Err(failed("invalid candidate startup journal"));
    }
    let startup = journal
        .startup()
        .ok_or_else(|| failed("candidate startup intent is missing"))?;
    let clock = host.clock()?;
    if !startup.accepts_observation(&startup.material_sha256, &clock) {
        return Err(failed("candidate startup original budget is unavailable"));
    }
    let remaining = startup.clock.uptime_ms + CandidateStartup::BUDGET_MS - clock.uptime_ms;
    let deadline = Instant::now() + Duration::from_millis(remaining);
    let slot = journal
        .candidate()
        .ok_or_else(|| failed("candidate binding is missing"))?
        .slot;
    guard(store, &journal, host, deadline)?;
    if journal.activation().is_none() {
        for action in [Action::Enable, Action::Start] {
            guard(store, &journal, host, deadline)?;
            host.dispatch(slot, action, deadline)?;
        }
    }
    loop {
        guard(store, &journal, host, deadline)?;
        if let Some(activation) = host.capture(&journal, deadline)? {
            let (material, clock) = guard(store, &journal, host, deadline)?;
            if host.capture(&journal, deadline)?.as_ref() != Some(&activation) {
                return Err(failed(
                    "candidate activation changed during startup observation",
                ));
            }
            // Re-sample after the final process observation, not before it.
            let observed_at = host.clock()?;
            if observed_at.uptime_ms < clock.uptime_ms || Instant::now() >= deadline {
                return Err(failed(
                    "candidate startup observation exceeded its deadline",
                ));
            }
            let next = journal
                .started(activation, &material, &observed_at)
                .ok_or_else(|| {
                    failed("candidate is not the closed, empty activation prepared by this task")
                })?;
            current(store, &journal)?;
            if next != journal {
                store.save_preparation(Some(&journal), &next)?;
            }
            return Ok(next);
        }
        if journal.activation().is_some() {
            return Err(failed(
                "recorded candidate activation is unavailable; preserve recovery evidence",
            ));
        }
        host.pause(deadline);
    }
}
