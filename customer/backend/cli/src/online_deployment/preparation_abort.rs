//! Cancel an unused candidate or retire its durably captured closed activation.
//! Uncertain startup is never treated as an unused candidate or process exit.
use super::*;

mod partial;
use aster_upgrade_core::{
    MaintenanceJob, MaintenanceStatus,
    preparation::{PreparationAbortPhase, PreparationJournal},
    runtime::ReadinessExpectation,
};
use std::{fs, path::PathBuf};

fn job(layout: &InstallLayout, journal: &PreparationJournal) -> Result<MaintenanceJob, CliFailure> {
    let intent = journal.intent();
    let running = layout
        .upgrade_running()
        .join(format!("{}.json", intent.job.id));
    let completed = layout
        .upgrade_completed()
        .join(format!("{}.json", intent.job.id));
    let complete = journal.abort_phase() == Some(PreparationAbortPhase::Complete);
    let path = match fs::symlink_metadata(&running) {
        Ok(_) => {
            match fs::symlink_metadata(&completed) {
                Ok(_) => return Err(failed("cancelled task exists in two queues")),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(failed(error.to_string())),
            }
            running
        }
        Err(error) if complete && error.kind() == std::io::ErrorKind::NotFound => completed,
        Err(error) => return Err(failed(error.to_string())),
    };
    let job = crate::online_commit::persisted_job(layout, &path)?;
    let mut identity = job.clone();
    if complete && job.status == MaintenanceStatus::Failed && job.runner_was_running.is_none() {
        identity.runner_was_running = intent.job.runner_was_running;
    }
    if !intent.matches_job(&identity)
        || !(job.status == MaintenanceStatus::StartingCandidate
            || (complete && job.status == MaintenanceStatus::Failed))
    {
        return Err(failed("cancelled preparation no longer owns this task"));
    }
    Ok(job)
}

fn current(store: &JournalFile, journal: &PreparationJournal) -> Result<(), CliFailure> {
    if !journal.valid()
        || store.load()?.is_some()
        || store.load_preparation()?.as_ref() != Some(journal)
        || aster_upgrade_core::online::switchback::SwitchbackJournalStorage::load(store)?.is_some()
    {
        return Err(failed("preparation cancellation lost journal ownership"));
    }
    Ok(())
}

fn retained_guard(
    layout: &InstallLayout,
    store: &JournalFile,
    journal: &PreparationJournal,
    deadline: Instant,
) -> Result<PathBuf, CliFailure> {
    current(store, journal)?;
    job(layout, journal)?;
    if Instant::now() >= deadline {
        return Err(failed("preparation cancellation observation expired"));
    }
    if journal.candidate().is_none() {
        verify_unprovisioned_release(layout, journal)?;
    }
    let intent = journal.intent();
    let old = RuntimeClient::new(layout, &intent.previous)?;
    let observed = old.status(deadline)?;
    let runner = SystemdRunnerHost::capture(intent.previous.slot, deadline)?;
    if !intent.observes_previous(&observed, &runner, &LinuxUpgradeClock::sample()?) {
        return Err(failed(
            "old runtime changed before preparation cancellation",
        ));
    }
    let disk = crate::proxy_disk::InstalledUpstreams::open(layout)?;
    let disk_slot = disk.observe(deadline)?;
    let proxy = CaddyClient::connect()?.observe(deadline)?;
    disk.assert_target(disk_slot, deadline)?;
    if disk_slot != intent.previous.slot || CaddyClient::snapshot(&proxy) != &intent.proxy {
        return Err(failed("proxy no longer retains the old slot"));
    }
    let runner = intent
        .previous
        .local_runner
        .as_ref()
        .ok_or_else(|| failed("old Runner binding missing"))?;
    let expectation = ReadinessExpectation {
        manifest_sha256: runner.manifest_sha256.clone(),
        runner_ids: vec![runner.runner_id.clone()],
        models: old.configured_models(&observed, deadline)?,
    };
    let _permit = old.readiness(&observed, &expectation, deadline)?;
    let release = crate::online_commit::retained_release(layout, &intent.previous)?;
    current(store, journal)?;
    if Instant::now() >= deadline {
        return Err(failed("preparation cancellation observation expired"));
    }
    Ok(release)
}

/// The intent can predate slot material and the published Runner identity. The
/// signed candidate is still mandatory because its private finalizer reconciles
/// database ownership even when the provisioning process lost its response.
fn verify_unprovisioned_release(
    layout: &InstallLayout,
    journal: &PreparationJournal,
) -> Result<(), CliFailure> {
    let intent = journal.intent();
    let version = intent
        .job
        .target_version
        .as_ref()
        .ok_or_else(|| failed("candidate version missing"))?;
    let release = layout.release(version);
    if intent.job.candidate_release.as_ref() != Some(&release)
        || intent.job.previous_release.as_ref() != Some(&layout.release(&intent.previous.version))
    {
        return Err(failed("unprovisioned candidate path differs from its task"));
    }
    let signed = crate::verify_release_at(&release)?;
    if signed.claims().version != *version
        || signed.claims().platform != "linux"
        || signed.claims().architecture != "amd64"
        || crate::sha256_file(&release.join("RELEASE.json"))? != intent.candidate_manifest_sha256
    {
        return Err(failed("unprovisioned candidate signed release changed"));
    }
    crate::slot_preparation::require_recovery_support(&release)
}

fn guard(
    layout: &InstallLayout,
    store: &JournalFile,
    journal: &PreparationJournal,
    deadline: Instant,
) -> Result<PathBuf, CliFailure> {
    let release = retained_guard(layout, store, journal, deadline)?;
    candidate_guard(layout, journal, deadline)?;
    current(store, journal)?;
    if Instant::now() >= deadline {
        return Err(failed("cancellation candidate observation expired"));
    }
    Ok(release)
}

fn candidate_guard(
    layout: &InstallLayout,
    journal: &PreparationJournal,
    deadline: Instant,
) -> Result<(), CliFailure> {
    if journal.partial_activation().is_some() {
        return partial::guard(layout, journal, deadline);
    }
    if let Some(activation) = journal.activation() {
        match journal.abort_phase() {
            None
            | Some(
                PreparationAbortPhase::CapturedForCancellation | PreparationAbortPhase::Claimed,
            ) => {
                let candidate = RuntimeClient::new(
                    layout,
                    journal
                        .candidate()
                        .ok_or_else(|| failed("candidate binding missing"))?,
                )?;
                if candidate.status(deadline)? != activation.control
                    || SystemdRunnerHost::capture(activation.control.slot, deadline)?
                        != activation.runner
                {
                    return Err(failed("candidate activation changed before cancellation"));
                }
            }
            Some(PreparationAbortPhase::RetiringCandidate) => {
                // The retirement adapter reconciles Running/Stopping/Exited
                // against this exact activation before sending any command.
            }
            Some(PreparationAbortPhase::Finalizing | PreparationAbortPhase::Complete) => {
                // Finalizing was persisted only after both pinned exits. Unit
                // disable may lose its reply or let systemd GC those identities;
                // require continued inactivity, not the pre-disable exit schema.
                return assert_finalized_candidate_inactive(journal, |unit| {
                    service_state(unit, deadline)
                });
            }
        }
        return Ok(());
    }
    if journal.startup().is_some() {
        return Err(failed(
            "uncaptured startup cannot be treated as an unused candidate",
        ));
    }
    crate::slot_preparation::assert_candidate_stopped(
        journal.intent().previous.slot.other(),
        |unit| service_state(unit, deadline),
    )?;
    Ok(())
}

fn service_state(unit: &str, deadline: Instant) -> Result<String, CliFailure> {
    crate::control_process::quiescence::observe(unit, deadline)
}

fn assert_finalized_candidate_inactive(
    journal: &PreparationJournal,
    mut state: impl FnMut(&str) -> Result<String, CliFailure>,
) -> Result<(), CliFailure> {
    if !journal.valid()
        || journal.abort_drained().is_none()
        || !matches!(
            journal.abort_phase(),
            Some(PreparationAbortPhase::Finalizing | PreparationAbortPhase::Complete)
        )
    {
        return Err(failed("candidate exits are not durably finalized"));
    }
    let slot = journal
        .activation()
        .ok_or_else(|| failed("candidate activation missing"))?
        .control
        .slot
        .id();
    for unit in [
        format!("aster-control@{slot}.service"),
        format!("aster-runner@{slot}.service"),
    ] {
        if state(&unit)? != "inactive" {
            return Err(failed(
                "finalized candidate is active or its state is uncertain",
            ));
        }
    }
    Ok(())
}

fn retire_candidate(
    layout: &InstallLayout,
    journal: &PreparationJournal,
    deadline: Instant,
) -> Result<bool, CliFailure> {
    use aster_upgrade_core::retirement::{ControlRetirementProgress, retire_control};
    if journal.partial_activation().is_some() {
        return partial::retire(layout, journal, deadline);
    }
    let candidate = RuntimeClient::new(
        layout,
        journal
            .candidate()
            .ok_or_else(|| failed("candidate binding missing"))?,
    )?;
    let drained = journal
        .abort_drained()
        .ok_or_else(|| failed("candidate drain checkpoint missing"))?;
    if retire_control(&candidate, &SystemdControlObserver, drained, deadline)
        .map_err(|error| failed(error.to_string()))?
        != ControlRetirementProgress::Exited
    {
        return Ok(false);
    }
    Ok(SystemdRunnerHost::retire_preparation(journal, deadline)? == ProcessProgress::Exited)
}

fn candidate_disable_units(journal: &PreparationJournal) -> Result<[String; 2], CliFailure> {
    if !journal.valid() || journal.abort_phase() != Some(PreparationAbortPhase::Finalizing) {
        return Err(failed("candidate is not ready for startup disable"));
    }
    // Even a never-started slot may have retained enablement from an earlier
    // installation. Do not let a later boot start partially written material.
    let slot = journal.intent().previous.slot.other().id();
    Ok([
        format!("aster-control@{slot}.service"),
        format!("aster-runner@{slot}.service"),
    ])
}

fn disable_candidate(journal: &PreparationJournal, deadline: Instant) -> Result<(), CliFailure> {
    let units = candidate_disable_units(journal)?;
    let mut command = std::process::Command::new("systemctl");
    command
        .args([
            "--system",
            "--no-ask-password",
            "--no-pager",
            "disable",
            "--",
        ])
        .args(units);
    crate::control_process::bounded_output(command, deadline)?;
    Ok(())
}

pub(super) fn claim(layout: &InstallLayout) -> Result<bool, CliFailure> {
    let mut store = JournalFile::open(layout)?;
    let Some(journal) = store.load_preparation()? else {
        return Ok(false);
    };
    if journal.abort_phase().is_some() {
        return Ok(false);
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    retained_guard(layout, &store, &journal, deadline)?;
    let cancelled = match journal.aborting() {
        Some(cancelled) => cancelled,
        None => crate::slot_preparation::online::capture_startup_for_cancellation(
            layout, &store, &journal, deadline,
        )?,
    };
    candidate_guard(layout, &cancelled, deadline)?;
    retained_guard(layout, &store, &journal, deadline)?;
    store.save_preparation(Some(&journal), &cancelled)?;
    Ok(true)
}

pub(super) fn resume(layout: &InstallLayout) -> Result<bool, CliFailure> {
    let mut store = JournalFile::open(layout)?;
    let Some(mut journal) = store.load_preparation()? else {
        return Ok(false);
    };
    if journal.abort_phase().is_none() {
        return Ok(false);
    }
    loop {
        let deadline = Instant::now() + Duration::from_secs(30);
        let release = guard(layout, &store, &journal, deadline)?;
        match journal
            .abort_phase()
            .ok_or_else(|| failed("cancellation intent disappeared"))?
        {
            PreparationAbortPhase::CapturedForCancellation => {}
            PreparationAbortPhase::Claimed => {
                if let Some(partial) = journal.partial_activation() {
                    let observed =
                        crate::slot_preparation::online::capture_partial_for_cancellation(
                            layout, &journal, deadline,
                        )?;
                    if &observed != partial {
                        return Err(failed("partial candidate changed before retirement"));
                    }
                    let next = journal
                        .retiring_partial_candidate(&observed)
                        .ok_or_else(|| failed("partial candidate cannot enter retirement"))?;
                    current(&store, &journal)?;
                    store.save_preparation(Some(&journal), &next)?;
                    journal = next;
                    continue;
                }
                if journal.activation().is_some() {
                    let candidate = RuntimeClient::new(
                        layout,
                        journal
                            .candidate()
                            .ok_or_else(|| failed("candidate binding missing"))?,
                    )?;
                    let next = journal
                        .retiring_candidate(&candidate.status(deadline)?)
                        .ok_or_else(|| failed("candidate is no longer closed and empty"))?;
                    current(&store, &journal)?;
                    store.save_preparation(Some(&journal), &next)?;
                    journal = next;
                    continue;
                }
            }
            PreparationAbortPhase::RetiringCandidate => {
                if !retire_candidate(layout, &journal, deadline)? {
                    thread::sleep(Duration::from_millis(250));
                    continue;
                }
            }
            PreparationAbortPhase::Finalizing => {
                disable_candidate(&journal, deadline)?;
                crate::online_commit::failed_job_audit(
                    layout,
                    &job(layout, &journal)?,
                    &release,
                    deadline,
                )?;
            }
            PreparationAbortPhase::Complete => {
                let mut task = job(layout, &journal)?;
                crate::online_commit::failed_job_audit(layout, &task, &release, deadline)?;
                let running = layout.upgrade_running().join(format!("{}.json", task.id));
                match fs::symlink_metadata(&running) {
                    Ok(_) => {
                        crate::maintenance_executor::set_job_status(
                            &running,
                            &mut task,
                            MaintenanceStatus::Failed,
                            "候选版本准备失败，已保留原版本",
                        )?;
                        crate::maintenance_executor::complete_job(layout, &running, &task)?;
                    }
                    Err(error)
                        if error.kind() == std::io::ErrorKind::NotFound
                            && task.status == MaintenanceStatus::Failed =>
                    {
                        crate::slot_preparation::cleanup_completed_material(layout)?;
                    }
                    Err(error) => return Err(failed(error.to_string())),
                }
                store.archive_cancelled_preparation(&journal)?;
                return Ok(true);
            }
        }
        let next = journal
            .advance_abort()
            .ok_or_else(|| failed("cancellation successor is invalid"))?;
        store.save_preparation(Some(&journal), &next)?;
        journal = next;
    }
}

#[cfg(test)]
mod tests;
