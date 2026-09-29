//! Durable installed-release commit after the journal proves both old processes
//! exited. This code never switches traffic or stops a running service.
use super::{CliFailure, SelectedRelease};
use aster_install_layout::InstallLayout;
use aster_upgrade_core::{
    ActiveReleaseSlot, MaintenanceJob, MaintenanceOperation, MaintenanceStatus, UpgradeMode,
    online::{OnlineJournal, OnlinePhase},
};
use serde::Serialize;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _, symlink};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

pub(super) mod switchback;

fn failed(message: impl Into<String>) -> CliFailure {
    CliFailure::new(aster_error_catalog::delivery::UPGRADE_FAILED, message)
}
fn before(deadline: Instant) -> Result<(), CliFailure> {
    if Instant::now() >= deadline {
        Err(failed(
            "online commit deadline elapsed; reload durable state",
        ))
    } else {
        Ok(())
    }
}
fn safe_parent(layout: &InstallLayout, path: &Path) -> Result<(), CliFailure> {
    if !path.starts_with(layout.root()) {
        return Err(failed("online commit path escaped the installation"));
    }
    for parent in path
        .parent()
        .ok_or_else(|| failed("path has no parent"))?
        .ancestors()
    {
        let metadata = fs::symlink_metadata(parent).map_err(|error| failed(error.to_string()))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata.mode() & 0o022 != 0 {
            return Err(failed(
                "online commit parent is not a protected plain directory",
            ));
        }
        if parent == layout.root() {
            return Ok(());
        }
    }
    Err(failed("online commit path is outside the installation"))
}
fn read(layout: &InstallLayout, path: &Path, maximum: u64) -> Result<Vec<u8>, CliFailure> {
    safe_parent(layout, path)?;
    let metadata = fs::symlink_metadata(path).map_err(|error| failed(error.to_string()))?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.nlink() != 1
        || metadata.mode() & 0o022 != 0
        || metadata.len() > maximum
    {
        return Err(failed("online commit file is unsafe or oversized"));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(maximum + 1).read_to_end(&mut bytes))
        .map_err(|error| failed(error.to_string()))?;
    if bytes.len() as u64 > maximum {
        return Err(failed("online commit file grew while reading"));
    }
    Ok(bytes)
}
fn json<T: serde::de::DeserializeOwned>(
    layout: &InstallLayout,
    path: &Path,
) -> Result<T, CliFailure> {
    serde_json::from_slice(&read(layout, path, 256 * 1024)?)
        .map_err(|error| failed(error.to_string()))
}
fn running_path(layout: &InstallLayout, job: &str) -> PathBuf {
    layout.upgrade_running().join(format!("{job}.json"))
}
fn completed_path(layout: &InstallLayout, job: &str) -> PathBuf {
    layout.upgrade_completed().join(format!("{job}.json"))
}

pub(super) fn load_job(
    layout: &InstallLayout,
    journal: &OnlineJournal,
    allow_completed: bool,
) -> Result<MaintenanceJob, CliFailure> {
    load_job_for(
        layout,
        journal,
        allow_completed,
        (journal.phase() == OnlinePhase::Complete).then_some(MaintenanceStatus::Succeeded),
    )
}

fn load_job_for(
    layout: &InstallLayout,
    journal: &OnlineJournal,
    allow_completed: bool,
    terminal: Option<MaintenanceStatus>,
) -> Result<MaintenanceJob, CliFailure> {
    if !journal.valid() {
        return Err(failed("online journal is invalid"));
    }
    let plan = journal.plan();
    let running = running_path(layout, &plan.job_id);
    let completed = completed_path(layout, &plan.job_id);
    let path = match fs::symlink_metadata(&running) {
        Ok(_) => {
            if fs::symlink_metadata(&completed).is_ok() {
                return Err(failed("online job exists in both running and completed"));
            }
            running
        }
        Err(error) if allow_completed && error.kind() == std::io::ErrorKind::NotFound => completed,
        Err(error) => return Err(failed(error.to_string())),
    };
    let job: MaintenanceJob = json(layout, &path)?;
    job.validate().map_err(|error| failed(error.to_string()))?;
    if job.id != plan.job_id
        || job.upgrade_mode != Some(UpgradeMode::BlueGreen)
        || job.current_version != plan.previous.version
        || job.target_version.as_deref() != Some(plan.candidate.version.as_str())
        || job.previous_release.as_ref() != Some(&layout.release(&plan.previous.version))
        || job.candidate_release.as_ref() != Some(&layout.release(&plan.candidate.version))
        || !(matches!(
            job.status,
            MaintenanceStatus::StartingCandidate
                | MaintenanceStatus::Migrating
                | MaintenanceStatus::SwitchingTraffic
                | MaintenanceStatus::DrainingPrevious
        ) || terminal == Some(job.status))
    {
        return Err(failed("durable job does not match the online plan"));
    }
    let MaintenanceOperation::Upgrade { archive, .. } = &job.operation else {
        return Err(failed("online task is not an upgrade"));
    };
    if archive != &layout.upgrade_uploads().join(format!("{}.tar.gz", job.id)) {
        return Err(failed("online upload belongs to another job"));
    }
    Ok(job)
}

fn candidate_release(
    layout: &InstallLayout,
    journal: &OnlineJournal,
) -> Result<PathBuf, CliFailure> {
    let plan = journal.plan();
    let release = layout.release(&plan.candidate.version);
    safe_parent(layout, &release.join("RELEASE.json"))?;
    let verified = super::verify_release_at(&release)?;
    if verified.claims().version != plan.candidate.version
        || verified.claims().platform != "linux"
        || verified.claims().architecture != "amd64"
        || super::sha256_file(&release.join("RELEASE.json"))? != plan.readiness.manifest_sha256
    {
        return Err(failed(
            "candidate signed release no longer matches the online plan",
        ));
    }
    Ok(release)
}

fn current_metadata(
    layout: &InstallLayout,
    journal: &OnlineJournal,
    require_candidate: bool,
) -> Result<(), CliFailure> {
    let plan = journal.plan();
    let active: ActiveReleaseSlot = json(layout, &layout.active_slot())?;
    active
        .validate()
        .map_err(|error| failed(error.to_string()))?;
    if active != plan.candidate && (require_candidate || active != plan.previous) {
        return Err(failed("active release is unrelated to the online plan"));
    }
    let current = layout
        .current()
        .canonicalize()
        .map_err(|error| failed(error.to_string()))?;
    let candidate = layout
        .release(&plan.candidate.version)
        .canonicalize()
        .map_err(|error| failed(error.to_string()))?;
    if current != candidate
        && (require_candidate
            || current
                != layout
                    .release(&plan.previous.version)
                    .canonicalize()
                    .map_err(|error| failed(error.to_string()))?)
    {
        return Err(failed(
            "current release pointer changed outside the online task",
        ));
    }
    let selected: SelectedRelease = json(layout, &layout.selected_release())?;
    let version = if selected.version == plan.candidate.version {
        &plan.candidate.version
    } else if !require_candidate && selected.version == plan.previous.version {
        &plan.previous.version
    } else {
        return Err(failed("selected release changed outside the online task"));
    };
    if selected.schema != super::SELECTED_RELEASE_SCHEMA
        || selected.release_root
            != layout
                .release(version)
                .canonicalize()
                .map_err(|error| failed(error.to_string()))?
        || selected.manifest_sha256
            != super::sha256_file(&selected.release_root.join("RELEASE.json"))?
    {
        return Err(failed("selected release metadata is inconsistent"));
    }
    Ok(())
}

fn command(command: Command, deadline: Instant) -> Result<(), CliFailure> {
    before(deadline)?;
    super::control_process::bounded_output(command, deadline)?;
    before(deadline)
}
fn temporary(path: &Path) -> Result<PathBuf, CliFailure> {
    let mut nonce = [0_u8; 16];
    getrandom::fill(&mut nonce)
        .map_err(|_| failed("cannot allocate online commit temporary file"))?;
    Ok(path
        .parent()
        .ok_or_else(|| failed("path has no parent"))?
        .join(format!(".online-{}.tmp", super::lowercase_hex(&nonce))))
}
fn replace(
    layout: &InstallLayout,
    path: &Path,
    bytes: &[u8],
    mode: u32,
    deadline: Instant,
) -> Result<(), CliFailure> {
    before(deadline)?;
    let old = read(layout, path, 512 * 1024 * 1024)?;
    if old == bytes {
        return before(deadline);
    }
    let temporary = temporary(path)?;
    let result = (|| {
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| failed(error.to_string()))?;
        output
            .set_permissions(fs::Permissions::from_mode(mode))
            .map_err(|error| failed(error.to_string()))?;
        let mut ownership = Command::new("chown");
        ownership
            .arg(format!("--reference={}", path.display()))
            .arg("--")
            .arg(&temporary);
        command(ownership, deadline)?;
        output
            .write_all(bytes)
            .and_then(|()| output.sync_all())
            .map_err(|error| failed(error.to_string()))?;
        before(deadline)?;
        fs::rename(&temporary, path).map_err(|error| failed(error.to_string()))?;
        super::sync_parent(path.parent().ok_or_else(|| failed("path has no parent"))?)?;
        before(deadline)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
fn replace_json(
    layout: &InstallLayout,
    path: &Path,
    value: &impl Serialize,
    deadline: Instant,
) -> Result<(), CliFailure> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| failed(error.to_string()))?;
    bytes.push(b'\n');
    replace(layout, path, &bytes, 0o640, deadline)
}
fn select_current(
    layout: &InstallLayout,
    release: &Path,
    deadline: Instant,
) -> Result<(), CliFailure> {
    before(deadline)?;
    safe_parent(layout, &layout.current())?;
    if layout
        .current()
        .canonicalize()
        .map_err(|error| failed(error.to_string()))?
        == release
            .canonicalize()
            .map_err(|error| failed(error.to_string()))?
    {
        return Ok(());
    }
    let temporary = temporary(&layout.current())?;
    let result = (|| {
        symlink(release, &temporary).map_err(|error| failed(error.to_string()))?;
        fs::rename(&temporary, layout.current()).map_err(|error| failed(error.to_string()))?;
        super::sync_parent(layout.root())?;
        before(deadline)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
fn audit(
    layout: &InstallLayout,
    journal: &OnlineJournal,
    release: &Path,
    deadline: Instant,
) -> Result<(), CliFailure> {
    let mut audit = Command::new("runuser");
    audit
        .args(["-u", "aster-team", "--"])
        .arg(layout.release_binary(release, "aster-control"))
        .args([
            "record-upgrade-audit",
            "--outcome",
            "succeeded",
            "--job-id",
            &journal.plan().job_id,
            "--target-version",
            &journal.plan().candidate.version,
        ]);
    command(audit, deadline)
}

fn promote_startup(
    layout: &InstallLayout,
    journal: &OnlineJournal,
    deadline: Instant,
) -> Result<(), CliFailure> {
    if journal.phase() != OnlinePhase::Committing {
        return Err(failed(
            "startup admission cannot change before durable process retirement",
        ));
    }
    let path = layout.control_slot_environment(journal.plan().candidate.slot.id());
    let current = read(layout, &path, 16 * 1024)?;
    let current =
        std::str::from_utf8(&current).map_err(|_| failed("candidate environment is not UTF-8"))?;
    let committed = super::slot_preparation::committed_control_environment(
        current,
        journal.plan().candidate.slot,
    )?;
    replace(layout, &path, committed.as_bytes(), 0o640, deadline)
}

fn verify_startup(layout: &InstallLayout, journal: &OnlineJournal) -> Result<(), CliFailure> {
    let path = layout.control_slot_environment(journal.plan().candidate.slot.id());
    let current = read(layout, &path, 16 * 1024)?;
    let current =
        std::str::from_utf8(&current).map_err(|_| failed("candidate environment is not UTF-8"))?;
    let committed = super::slot_preparation::committed_control_environment(
        current,
        journal.plan().candidate.slot,
    )?;
    if committed != current {
        return Err(failed(
            "committed Control would restart with candidate admission",
        ));
    }
    Ok(())
}

pub(super) fn commit(
    layout: &InstallLayout,
    journal: &OnlineJournal,
    deadline: Instant,
) -> Result<(), CliFailure> {
    before(deadline)?;
    if journal.phase() != OnlinePhase::Committing {
        return Err(failed(
            "both process exits must be durable before release commit",
        ));
    }
    load_job(layout, journal, false)?;
    let release = candidate_release(layout, journal)?;
    current_metadata(layout, journal, false)?;
    let mut disable = Command::new("systemctl");
    disable
        .args([
            "--system",
            "--no-ask-password",
            "--no-pager",
            "disable",
            "--",
        ])
        .arg(format!(
            "aster-control@{}.service",
            journal.plan().previous.slot.id()
        ))
        .arg(format!(
            "aster-runner@{}.service",
            journal.plan().previous.slot.id()
        ));
    command(disable, deadline)?;
    select_current(layout, &release, deadline)?;
    replace_json(
        layout,
        &layout.selected_release(),
        &SelectedRelease {
            schema: super::SELECTED_RELEASE_SCHEMA.into(),
            version: journal.plan().candidate.version.clone(),
            release_root: release
                .canonicalize()
                .map_err(|error| failed(error.to_string()))?,
            manifest_sha256: journal.plan().readiness.manifest_sha256.clone(),
        },
        deadline,
    )?;
    let cli = read(
        layout,
        &layout.release_binary(&release, "aster-team-cli"),
        512 * 1024 * 1024,
    )?;
    replace(layout, &layout.stable_cli(), &cli, 0o755, deadline)?;
    audit(layout, journal, &release, deadline)?;
    promote_startup(layout, journal, deadline)?;
    // This remains the final commit point. A lost reply retries against the same
    // task-bound signed audit and accepts only previous/candidate pointers.
    replace_json(
        layout,
        &layout.active_slot(),
        &journal.plan().candidate,
        deadline,
    )?;
    current_metadata(layout, journal, true)?;
    verify_startup(layout, journal)
}

pub(super) fn finish(layout: &InstallLayout, journal: &OnlineJournal) -> Result<(), CliFailure> {
    if journal.phase() != OnlinePhase::Complete {
        return Err(failed("online transition is not complete"));
    }
    let mut job = load_job(layout, journal, true)?;
    let release = candidate_release(layout, journal)?;
    current_metadata(layout, journal, true)?;
    verify_startup(layout, journal)?;
    if super::sha256_file(&layout.stable_cli())?
        != super::sha256_file(&layout.release_binary(&release, "aster-team-cli"))?
    {
        return Err(failed("stable CLI does not match the committed release"));
    }
    audit(
        layout,
        journal,
        &release,
        Instant::now() + Duration::from_secs(30),
    )?;
    let running = running_path(layout, &job.id);
    if fs::symlink_metadata(&running).is_ok() {
        super::maintenance_executor::set_job_status(
            &running,
            &mut job,
            MaintenanceStatus::Succeeded,
            "在线升级完成，正在归档执行记录",
        )?;
        super::maintenance_executor::complete_job(layout, &running, &job)?;
    } else {
        if job.status != MaintenanceStatus::Succeeded {
            return Err(failed("archived online job is not successful"));
        }
        super::slot_preparation::cleanup_completed_material(layout)?;
    }
    Ok(())
}

/// Validate the retained original installation without repairing pointers.
pub(crate) fn retained_release(
    layout: &InstallLayout,
    old: &ActiveReleaseSlot,
) -> Result<PathBuf, CliFailure> {
    old.validate().map_err(|error| failed(error.to_string()))?;
    let release = layout.release(&old.version);
    let active: ActiveReleaseSlot = json(layout, &layout.active_slot())?;
    let selected: SelectedRelease = json(layout, &layout.selected_release())?;
    if active != *old
        || selected.version != old.version
        || layout
            .current()
            .canonicalize()
            .map_err(|error| failed(error.to_string()))?
            != release
                .canonicalize()
                .map_err(|error| failed(error.to_string()))?
    {
        return Err(failed(
            "switchback active metadata no longer names the original release",
        ));
    }
    if selected.schema != crate::SELECTED_RELEASE_SCHEMA
        || selected.release_root
            != release
                .canonicalize()
                .map_err(|error| failed(error.to_string()))?
        || selected.manifest_sha256 != crate::sha256_file(&release.join("RELEASE.json"))?
    {
        return Err(failed("retained release selection changed"));
    }
    let verified = crate::verify_release_at(&release)?;
    let manifest = &old
        .local_runner
        .as_ref()
        .ok_or_else(|| failed("old Runner identity is missing"))?
        .manifest_sha256;
    if verified.claims().version != old.version
        || verified.claims().platform != "linux"
        || verified.claims().architecture != "amd64"
        || crate::sha256_file(&release.join("RELEASE.json"))? != *manifest
        || crate::sha256_file(&layout.stable_cli())?
            != crate::sha256_file(&layout.release_binary(&release, "aster-team-cli"))?
    {
        return Err(failed(
            "switchback old signed release or stable CLI changed",
        ));
    }
    let path = layout.control_slot_environment(old.slot.id());
    let bytes = read(layout, &path, 16 * 1024)?;
    let environment =
        std::str::from_utf8(&bytes).map_err(|_| failed("old environment is not UTF-8"))?;
    if crate::slot_preparation::committed_control_environment(environment, old.slot)? != environment
    {
        return Err(failed(
            "old Control would restart with closed candidate admission",
        ));
    }
    Ok(release)
}

pub(crate) fn persisted_job(
    layout: &InstallLayout,
    path: &Path,
) -> Result<MaintenanceJob, CliFailure> {
    let job: MaintenanceJob = json(layout, path)?;
    job.validate().map_err(|error| failed(error.to_string()))?;
    Ok(job)
}

pub(crate) fn failed_job_audit(
    layout: &InstallLayout,
    job: &MaintenanceJob,
    release: &Path,
    deadline: Instant,
) -> Result<(), CliFailure> {
    let mut audit = Command::new("runuser");
    audit
        .args(["-u", "aster-team", "--"])
        .arg(layout.release_binary(release, "aster-control"))
        .args([
            "record-upgrade-audit",
            "--outcome",
            "failed",
            "--job-id",
            &job.id,
            "--target-version",
            job.target_version
                .as_deref()
                .ok_or_else(|| failed("failed upgrade has no target"))?,
        ]);
    command(audit, deadline)
}

#[cfg(test)]
mod tests;
