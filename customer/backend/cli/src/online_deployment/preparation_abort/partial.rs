//! Complete only the services captured during partial startup. Absent services
//! require repeated quiescence proof; runtime errors alone authorize nothing.
use super::*;
use aster_upgrade_core::retirement::{ControlRetirementProgress, retire_control};

pub(super) fn guard(
    layout: &InstallLayout,
    journal: &PreparationJournal,
    deadline: Instant,
) -> Result<(), CliFailure> {
    let partial = journal
        .partial_activation()
        .ok_or_else(|| failed("partial activation missing"))?;
    match journal.abort_phase() {
        Some(PreparationAbortPhase::CapturedForCancellation | PreparationAbortPhase::Claimed) => {
            if crate::slot_preparation::online::capture_partial_for_cancellation(
                layout, journal, deadline,
            )? != *partial
            {
                return Err(failed("partial startup activation changed"));
            }
        }
        Some(PreparationAbortPhase::RetiringCandidate) => assert_absent(journal, deadline)?,
        Some(PreparationAbortPhase::Finalizing | PreparationAbortPhase::Complete) => {
            assert_finalized(journal, |unit| service_state(unit, deadline))?;
        }
        None => return Err(failed("partial activation lacks cancellation ownership")),
    }
    Ok(())
}

fn assert_absent(journal: &PreparationJournal, deadline: Instant) -> Result<(), CliFailure> {
    let partial = journal
        .partial_activation()
        .ok_or_else(|| failed("partial activation missing"))?;
    let slot = journal
        .candidate()
        .ok_or_else(|| failed("candidate missing"))?
        .slot
        .id();
    for (service, captured) in [
        ("control", partial.control.is_some()),
        ("runner", partial.runner.is_some()),
    ] {
        if !captured {
            service_state(&format!("aster-{service}@{slot}.service"), deadline)?;
        }
    }
    Ok(())
}

fn assert_finalized(
    journal: &PreparationJournal,
    mut state: impl FnMut(&str) -> Result<String, CliFailure>,
) -> Result<(), CliFailure> {
    if !journal.valid()
        || !matches!(
            journal.abort_phase(),
            Some(PreparationAbortPhase::Finalizing | PreparationAbortPhase::Complete)
        )
    {
        return Err(failed("partial cancellation exits are not finalized"));
    }
    let partial = journal
        .partial_activation()
        .ok_or_else(|| failed("partial activation missing"))?;
    let slot = journal
        .candidate()
        .ok_or_else(|| failed("candidate missing"))?
        .slot
        .id();
    for (service, captured) in [
        ("control", partial.control.is_some()),
        ("runner", partial.runner.is_some()),
    ] {
        let observed = state(&format!("aster-{service}@{slot}.service"))?;
        if observed != "inactive" && (captured || observed != "failed") {
            return Err(failed("partial candidate exit or quiescence changed"));
        }
    }
    Ok(())
}

pub(super) fn retire(
    layout: &InstallLayout,
    journal: &PreparationJournal,
    deadline: Instant,
) -> Result<bool, CliFailure> {
    if !journal.valid() || journal.abort_phase() != Some(PreparationAbortPhase::RetiringCandidate) {
        return Err(failed("partial candidate lacks retirement permission"));
    }
    let partial = journal
        .partial_activation()
        .ok_or_else(|| failed("partial activation missing"))?;
    assert_absent(journal, deadline)?;
    if let Some(control) = &partial.control {
        let client = RuntimeClient::new(
            layout,
            journal
                .candidate()
                .ok_or_else(|| failed("candidate missing"))?,
        )?;
        if retire_control(&client, &SystemdControlObserver, control, deadline)
            .map_err(|e| failed(e.to_string()))?
            != ControlRetirementProgress::Exited
        {
            return Ok(false);
        }
    }
    if partial.runner.is_some()
        && SystemdRunnerHost::retire_preparation(journal, deadline)? != ProcessProgress::Exited
    {
        return Ok(false);
    }
    // Do not finalize an absent service that began activating during retirement.
    assert_absent(journal, deadline)?;
    Ok(true)
}

#[cfg(test)]
mod tests;
