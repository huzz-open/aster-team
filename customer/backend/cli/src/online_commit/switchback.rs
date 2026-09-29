//! A reverse operation starts before the forward commit point. Therefore the
//! installed active metadata must still be the old release; never repair drift.
use super::*;
use aster_upgrade_core::online::switchback::{SwitchbackJournal, SwitchbackPhase};

pub(crate) fn load_job(
    layout: &InstallLayout,
    journal: &SwitchbackJournal,
) -> Result<MaintenanceJob, CliFailure> {
    if !journal.valid() {
        return Err(failed("switchback journal is invalid"));
    }
    let complete = journal.phase() == SwitchbackPhase::Complete;
    super::load_job_for(
        layout,
        &journal.plan().original,
        complete,
        complete.then_some(MaintenanceStatus::Failed),
    )
}

fn old_metadata(
    layout: &InstallLayout,
    journal: &SwitchbackJournal,
) -> Result<PathBuf, CliFailure> {
    super::current_metadata(layout, &journal.plan().original, false)?;
    super::retained_release(layout, &journal.plan().original.plan().previous)
}

fn audit_failure(
    layout: &InstallLayout,
    journal: &SwitchbackJournal,
    release: &Path,
    deadline: Instant,
) -> Result<(), CliFailure> {
    super::failed_job_audit(layout, &load_job(layout, journal)?, release, deadline)
}

pub(crate) fn commit(
    layout: &InstallLayout,
    journal: &SwitchbackJournal,
    deadline: Instant,
) -> Result<(), CliFailure> {
    before(deadline)?;
    if journal.phase() != SwitchbackPhase::CommittingPrevious {
        return Err(failed("candidate exits are not durable"));
    }
    load_job(layout, journal)?;
    let release = old_metadata(layout, journal)?;
    // Exits are already proven. Disable only future candidate startup, never
    // stop either process or erase exit evidence while retirement is pending.
    let slot = journal.plan().candidate.slot.id();
    let mut disable = Command::new("systemctl");
    disable
        .args([
            "--system",
            "--no-ask-password",
            "--no-pager",
            "disable",
            "--",
        ])
        .arg(format!("aster-control@{slot}.service"))
        .arg(format!("aster-runner@{slot}.service"));
    command(disable, deadline)?;
    audit_failure(layout, journal, &release, deadline)?;
    old_metadata(layout, journal)?;
    before(deadline)
}

pub(crate) fn finish(
    layout: &InstallLayout,
    journal: &SwitchbackJournal,
) -> Result<(), CliFailure> {
    if journal.phase() != SwitchbackPhase::Complete {
        return Err(failed("switchback is not complete"));
    }
    let mut job = load_job(layout, journal)?;
    let release = old_metadata(layout, journal)?;
    audit_failure(
        layout,
        journal,
        &release,
        Instant::now() + Duration::from_secs(30),
    )?;
    let running = running_path(layout, &job.id);
    match fs::symlink_metadata(&running) {
        Ok(_) => {
            crate::maintenance_executor::set_job_status(
                &running,
                &mut job,
                MaintenanceStatus::Failed,
                "在线升级已回切到原版本",
            )?;
            // This also finalizes the signed logical Runner ownership using the
            // still-active old slot before removing the job's private material.
            crate::maintenance_executor::complete_job(layout, &running, &job)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if job.status != MaintenanceStatus::Failed {
                return Err(failed("archived switchback job is not failed"));
            }
            crate::slot_preparation::cleanup_completed_material(layout)?;
        }
        Err(error) => return Err(failed(error.to_string())),
    }
    Ok(())
}

#[cfg(test)]
mod tests;
