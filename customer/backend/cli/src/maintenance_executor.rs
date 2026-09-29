use std::{
    fs::{self, File, OpenOptions},
    io::{Read as _, Write as _},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    path::{Component, Path, PathBuf},
    process::Command as ProcessCommand,
    thread,
    time::{Duration, Instant},
};

use aster_error_catalog::delivery;
use aster_install_layout::{InstallLayout, Platform};
use aster_upgrade_core::ACTIVE_SLOT_SCHEMA;
use aster_upgrade_core::{
    ActiveReleaseSlot, MAINTENANCE_JOB_SCHEMA, MaintenanceJob, MaintenanceOperation,
    MaintenanceStatus, ReleaseSlot, UpgradeMode,
};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use semver::Version;
use serde::Serialize;
use tar::Archive;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use super::{
    CliFailure, SelectedRelease, atomic_write_selection, current_release, install_layout,
    load_selected_release, open_maintenance_lock, read_trimmed, run_service_action,
    service_is_active, sha256_file, verify_release_at,
};

use super::run_checked;

#[cfg(target_os = "linux")]
mod online;

const MAX_EXTRACTED_FILES: usize = 20_000;
const MAX_EXTRACTED_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const CANDIDATE_READY_TIMEOUT: Duration = Duration::from_secs(90);
#[cfg(any(target_os = "linux", test))]
const SYSTEM_UNITS: &[&str] = &[
    "aster-control@.service",
    "aster-runner.service",
    "aster-runner@.service",
    "aster-caddy.service",
    "aster-upgrade.path",
    "aster-upgrade.service",
];
#[cfg(any(target_os = "linux", test))]
type SystemUnitState = (PathBuf, Option<Vec<u8>>);
#[cfg(any(target_os = "linux", test))]
type SystemUnitSnapshot = Vec<SystemUnitState>;
#[cfg(any(target_os = "windows", target_os = "macos"))]
type FileSnapshot = Vec<(PathBuf, Option<Vec<u8>>)>;

#[cfg(target_os = "macos")]
const MACOS_SERVICE_LABELS: &[&str] = &[
    "com.aster-team.control.blue",
    "com.aster-team.control.green",
    "com.aster-team.runner",
    "com.aster-team.caddy",
    "com.aster-team.maintenance",
];

pub(super) fn run_next() -> Result<(), CliFailure> {
    let layout = install_layout();
    prepare_directories(&layout)?;
    let _lock = acquire_maintenance_lock()?;
    run_next_locked(&layout)
}

fn acquire_maintenance_lock() -> Result<File, CliFailure> {
    let lock = open_maintenance_lock()?;
    lock.try_lock().map_err(|error| match error {
        fs::TryLockError::WouldBlock => CliFailure::new(
            delivery::MAINTENANCE_BUSY,
            "another installation or maintenance operation is running",
        ),
        fs::TryLockError::Error(error) => {
            CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string())
        }
    })?;
    Ok(lock)
}

fn run_next_locked(layout: &InstallLayout) -> Result<(), CliFailure> {
    #[cfg(target_os = "linux")]
    if super::online_deployment::run_existing(layout)?
        || super::online_deployment::run_prepared(layout)?
    {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    super::slot_preparation::cleanup_completed_material(layout)?;
    #[cfg(target_os = "windows")]
    for directory in [layout.stable_bin(), layout.service_config()] {
        cleanup_retired_windows_files(&directory)?;
    }
    recover_interrupted_jobs(layout)?;
    let Some((running_path, mut job)) = claim_next_job(layout)? else {
        return Ok(());
    };
    let operation = job.operation.clone();
    let result = match operation {
        MaintenanceOperation::Upgrade {
            archive,
            archive_sha256,
        } => execute_upgrade(layout, &running_path, &mut job, &archive, &archive_sha256),
        MaintenanceOperation::DeleteVersion { version } => {
            execute_version_cleanup(layout, &running_path, &mut job, &version).map(Some)
        }
    };
    match result {
        Ok(None) => Ok(()), // Online completion already owns task archival.
        Ok(Some(message)) => {
            set_job_status(
                &running_path,
                &mut job,
                MaintenanceStatus::Succeeded,
                message,
            )?;
            complete_job(layout, &running_path, &job)?;
            let _ = cleanup_upgrade_archive(layout, &job);
            Ok(())
        }
        Err(error) => finish_failed_job(layout, &running_path, &mut job, error),
    }
}

fn finish_failed_job(
    layout: &InstallLayout,
    running_path: &Path,
    job: &mut MaintenanceJob,
    error: CliFailure,
) -> Result<(), CliFailure> {
    let detail = error.to_string();
    if job.upgrade_mode == Some(UpgradeMode::BlueGreen) {
        // The online owner may already have archived this job before journal
        // archival failed. Never resurrect it from the caller's stale copy.
        let saved = fs::read(running_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<MaintenanceJob>(&bytes).ok());
        let Some(saved) = saved.filter(|saved| {
            saved.validate().is_ok()
                && saved.id == job.id
                && saved.operation == job.operation
                && saved.upgrade_mode == job.upgrade_mode
                && saved.requested_by == job.requested_by
                && saved.current_version == job.current_version
                && saved.target_version == job.target_version
                && saved.previous_release == job.previous_release
                && saved.candidate_release == job.candidate_release
        }) else {
            return Err(error);
        };
        *job = saved;
    }
    // Once online preparation can have changed a slot, a missing journal is
    // not proof that nothing happened. Keep the task and upload for recovery.
    let online_recovery = job.upgrade_mode == Some(UpgradeMode::BlueGreen)
        && !matches!(
            job.status,
            MaintenanceStatus::Queued | MaintenanceStatus::Verifying | MaintenanceStatus::Staging
        );
    let evidence_error = super::online_journal::ensure_maintenance_allowed(layout).err();
    if online_recovery || evidence_error.is_some() {
        let phase = job.status;
        let context = evidence_error
            .map(|error| format!("；恢复状态：{error}"))
            .unwrap_or_default();
        set_job_status(
            running_path,
            job,
            phase,
            format!("升级尚未完成，已保留原任务和安装包等待恢复：{detail}{context}"),
        )?;
        return Err(CliFailure::new(delivery::UPGRADE_FAILED, detail));
    }
    let message = match &job.operation {
        MaintenanceOperation::Upgrade { .. } => format!("升级失败：{detail}"),
        MaintenanceOperation::DeleteVersion { .. } => format!("历史版本清理失败：{detail}"),
    };
    if job.status == MaintenanceStatus::RestoringPrevious {
        // The next executor run must recover before accepting another task.
        set_job_status(
            running_path,
            job,
            MaintenanceStatus::RestoringPrevious,
            message,
        )?;
    } else {
        set_job_status(running_path, job, MaintenanceStatus::Failed, message)?;
        complete_job(layout, running_path, job)?;
        let _ = cleanup_upgrade_archive(layout, job);
    }
    Err(CliFailure::new(delivery::UPGRADE_FAILED, detail))
}

fn recover_interrupted_jobs(layout: &InstallLayout) -> Result<(), CliFailure> {
    recover_interrupted_jobs_with(
        layout,
        |active, runner| reconcile_interrupted_upgrade(layout, active, runner),
        || service_is_active(&active_runner_unit()?),
        |active| {
            let release = layout.release(&active.version);
            verify_release_at(&release)?;
            install_stable_cli(layout, &release)
        },
    )
}

fn recover_interrupted_jobs_with(
    layout: &InstallLayout,
    mut reconcile: impl FnMut(&ActiveReleaseSlot, bool) -> Result<(), CliFailure>,
    mut runner_active: impl FnMut() -> Result<bool, CliFailure>,
    mut synchronize_cli: impl FnMut(&ActiveReleaseSlot) -> Result<(), CliFailure>,
) -> Result<(), CliFailure> {
    // An online transition owns both slots until its own recovery/archival has
    // completed. Never pass it through maintenance's stop-and-restore path.
    super::online_journal::ensure_maintenance_allowed(layout)?;
    let mut paths = fs::read_dir(layout.upgrade_running())
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?
        .map(|entry| {
            let entry = entry
                .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
            let kind = entry
                .file_type()
                .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
            Ok(
                (kind.is_file() && entry.path().extension().is_some_and(|ext| ext == "json"))
                    .then(|| entry.path()),
            )
        })
        .collect::<Result<Vec<_>, CliFailure>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        // An unreadable or corrupt journal may describe a partially applied
        // transition. Retain it and fail closed; never silently discard it.
        let encoded = fs::read(&path).map_err(|error| {
            CliFailure::new(
                delivery::UPGRADE_FAILED,
                format!("cannot read interrupted maintenance job: {error}"),
            )
        })?;
        if encoded.len() > 256 * 1024 {
            return Err(CliFailure::new(
                delivery::UPGRADE_FAILED,
                "interrupted maintenance job is too large",
            ));
        }
        let mut job: MaintenanceJob = serde_json::from_slice(&encoded).map_err(|error| {
            CliFailure::new(
                delivery::UPGRADE_FAILED,
                format!("invalid interrupted maintenance job: {error}"),
            )
        })?;
        job.validate().map_err(|error| {
            CliFailure::new(
                delivery::UPGRADE_FAILED,
                format!("invalid interrupted maintenance job: {error}"),
            )
        })?;
        if path.file_stem().and_then(|name| name.to_str()) != Some(&job.id) {
            return Err(CliFailure::new(
                delivery::UPGRADE_FAILED,
                "interrupted maintenance job identity does not match its file",
            ));
        }
        if job.status.terminal() {
            if matches!(job.operation, MaintenanceOperation::Upgrade { .. })
                && job.previous_release.is_some()
            {
                // Resume a crash after terminalizing the journal but before
                // restoring the stable CLI or archiving the task.
                persist_job(&path, &job)?;
                synchronize_cli(&load_active_slot(layout)?)?;
            }
            complete_job(layout, &path, &job)?;
            continue;
        }
        let needs_recovery = matches!(job.operation, MaintenanceOperation::Upgrade { .. })
            && !matches!(
                job.status,
                MaintenanceStatus::Queued
                    | MaintenanceStatus::Verifying
                    | MaintenanceStatus::Staging
            );
        if needs_recovery && job.upgrade_mode == Some(UpgradeMode::BlueGreen) {
            return Err(CliFailure::new(
                delivery::UPGRADE_FAILED,
                "interrupted online upgrade requires online recovery; refusing maintenance service shutdown",
            ));
        }
        if !needs_recovery {
            let message = match job.operation {
                MaintenanceOperation::Upgrade { .. } => {
                    "升级在修改业务服务之前中断，服务未因恢复流程重启，请重新提交任务"
                }
                MaintenanceOperation::DeleteVersion { .. } => {
                    "版本清理被中断，请根据已安装版本列表重新提交清理任务"
                }
            };
            set_job_status(&path, &mut job, MaintenanceStatus::Failed, message)?;
            complete_job(layout, &path, &job)?;
            let _ = cleanup_upgrade_archive(layout, &job);
            continue;
        }
        let active = load_active_slot(layout)?;
        let runner_was_running = match job.runner_was_running {
            Some(value) => value,
            None => {
                let value = runner_active()?;
                // Checkpoint before stopping anything, including legacy recovery.
                job.runner_was_running = Some(value);
                persist_job(&path, &job)?;
                value
            }
        };
        let candidate_is_active = job.target_version.as_deref() == Some(active.version.as_str())
            && matches!(
                job.status,
                MaintenanceStatus::SwitchingTraffic | MaintenanceStatus::DrainingPrevious
            );
        if let Err(error) = reconcile(&active, runner_was_running) {
            job.message =
                format!("升级中断后的服务恢复尚未完成，将在执行器下次运行时重试：{error}");
            job.updated_at = now()?;
            persist_job(&path, &job)?;
            return Err(error);
        }
        let (status, message) = if candidate_is_active {
            (
                MaintenanceStatus::Succeeded,
                "升级切换已提交，恢复时已确认新版本服务运行",
            )
        } else {
            (
                MaintenanceStatus::Failed,
                "升级执行被中断，已恢复原活动槽位和访问入口；数据库未回退",
            )
        };
        set_job_status(&path, &mut job, status, message)?;
        // Only restore an older CLI after the strict legacy job reader can read
        // the terminal task. A crash here must not strand an unparseable journal.
        if let Err(error) = synchronize_cli(&active) {
            set_job_status(
                &path,
                &mut job,
                MaintenanceStatus::Failed,
                format!("服务已恢复，但稳定 CLI 同步失败：{error}"),
            )?;
            return Err(error);
        }
        complete_job(layout, &path, &job)?;
        let _ = cleanup_upgrade_archive(layout, &job);
    }
    Ok(())
}

fn reconcile_interrupted_upgrade(
    layout: &InstallLayout,
    active: &ActiveReleaseSlot,
    runner_was_running: bool,
) -> Result<(), CliFailure> {
    let release = layout.release(&active.version);
    verify_release_at(&release)?;
    validate_runner_release(layout, active)?;
    quiesce_services()?;
    install_service_assets(layout, &release)?;
    replace_directory_link(layout, &layout.slot_release(active.slot.id()), &release)?;
    replace_directory_link(layout, &layout.current(), &release)?;
    run_service_action(
        "disable",
        &[slot_unit(active.slot.other())],
        delivery::UPGRADE_FAILED,
    )?;
    run_service_action(
        "enable",
        &[slot_unit(active.slot)],
        delivery::UPGRADE_FAILED,
    )?;
    run_service_action("start", &[slot_unit(active.slot)], delivery::UPGRADE_FAILED)?;
    wait_for_slot(layout, active.slot, CANDIDATE_READY_TIMEOUT)?;
    atomic_replace(
        layout.caddy_upstreams(),
        render_upstreams(active.slot, super::instance_ports(layout)?).as_bytes(),
        0o640,
    )?;
    reload_caddy(layout)?;
    write_selected_release(layout, &release, &active.version)?;
    restore_runner(layout, active, runner_was_running)?;
    Ok(())
}

pub(super) fn runner_unit(active: &ActiveReleaseSlot) -> String {
    if active.local_runner.is_some() {
        format!("aster-runner@{}.service", active.slot.id())
    } else {
        "aster-runner.service".into()
    }
}

pub(super) fn active_runner_unit() -> Result<String, CliFailure> {
    Ok(runner_unit(&load_active_slot(&install_layout())?))
}

pub(super) fn active_control_unit() -> Result<String, CliFailure> {
    let active = load_active_slot(&install_layout())?;
    Ok(format!("aster-control@{}.service", active.slot.id()))
}

pub(super) fn upgrade_selected_release() -> Result<(), CliFailure> {
    let layout = install_layout();
    prepare_directories(&layout)?;
    // Hold one lock across enqueue and execution: neither a scheduled worker
    // nor a competing CLI may consume a different task in this window.
    let _lock = acquire_maintenance_lock()?;
    recover_interrupted_jobs(&layout)?;
    require_empty_upgrade_queue(&layout)?;
    let selected = load_selected_release(&layout.selected_release())?;
    let current_version = read_trimmed(&current_release()?.join("VERSION"))?;
    if selected.version == current_version {
        return Err(CliFailure::new(
            delivery::UPGRADE_FAILED,
            "the selected release is already active",
        ));
    }
    let id = format!(
        "upgrade-cli-{}-{}",
        OffsetDateTime::now_utc().unix_timestamp_nanos(),
        std::process::id()
    );
    let archive = layout.upgrade_uploads().join(format!("{id}.tar.gz"));
    let output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&archive)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    let encoder = GzEncoder::new(output, Compression::best());
    let mut builder = tar::Builder::new(encoder);
    let archive_root = format!(
        "aster-team-{}-{}-{}",
        selected.version,
        Platform::current().id(),
        current_architecture()
    );
    if let Err(error) = builder.append_dir_all(&archive_root, &selected.release_root) {
        let _ = fs::remove_file(&archive);
        return Err(CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            error.to_string(),
        ));
    }
    let encoder = builder
        .into_inner()
        .and_then(GzEncoder::finish)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    encoder
        .sync_all()
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    let created_at = now()?;
    let job = MaintenanceJob {
        schema: MAINTENANCE_JOB_SCHEMA.to_owned(),
        id,
        requested_by: "local-cli".to_owned(),
        operation: MaintenanceOperation::Upgrade {
            archive: archive.clone(),
            archive_sha256: sha256_file(&archive)?,
        },
        status: MaintenanceStatus::Queued,
        // CLI also bridges old Controls with a strict v1 job reader.
        upgrade_mode: None,
        runner_was_running: None,
        current_version,
        target_version: Some(selected.version),
        previous_release: None,
        candidate_release: None,
        message: "维护升级：期间页面、API 和正在进行的流式调用可能中断".to_owned(),
        created_at: created_at.clone(),
        updated_at: created_at,
    };
    job.validate().map_err(|error| {
        CliFailure::new(delivery::UPGRADE_FAILED, format!("invalid job: {error}"))
    })?;
    let queue_path = layout.upgrade_queued().join(format!("{}.json", job.id));
    write_json_new(queue_path, &job, 0o640)?;
    run_next_locked(&layout)
}

fn require_empty_upgrade_queue(layout: &InstallLayout) -> Result<(), CliFailure> {
    for entry in fs::read_dir(layout.upgrade_queued())
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?
    {
        let entry = entry
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
        if entry
            .path()
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            return Err(CliFailure::new(
                delivery::MAINTENANCE_BUSY,
                "another maintenance task is queued",
            ));
        }
    }
    Ok(())
}

fn prepare_directories(layout: &InstallLayout) -> Result<(), CliFailure> {
    for path in [
        layout.upgrade_queued(),
        layout.upgrade_running(),
        layout.upgrade_completed(),
        layout.upgrade_uploads(),
        layout.upgrade_backups(),
        layout.slots(),
    ] {
        fs::create_dir_all(path)
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    }
    Ok(())
}

fn claim_next_job(layout: &InstallLayout) -> Result<Option<(PathBuf, MaintenanceJob)>, CliFailure> {
    let mut queued = fs::read_dir(layout.upgrade_queued())
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_type()
                .is_ok_and(|kind| kind.is_file() && !kind.is_symlink())
                && entry
                    .path()
                    .extension()
                    .is_some_and(|value| value == "json")
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    queued.sort();
    for source in queued {
        let Some(name) = source.file_name() else {
            continue;
        };
        let running = layout.upgrade_running().join(name);
        match fs::rename(&source, &running) {
            Ok(()) => {
                let encoded = fs::read(&running).map_err(|error| {
                    CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string())
                })?;
                if encoded.len() > 256 * 1024 {
                    return Err(CliFailure::new(
                        delivery::UPGRADE_FAILED,
                        "maintenance job file is too large",
                    ));
                }
                let job: MaintenanceJob = serde_json::from_slice(&encoded).map_err(|error| {
                    CliFailure::new(delivery::UPGRADE_FAILED, format!("invalid job: {error}"))
                })?;
                job.validate().map_err(|error| {
                    CliFailure::new(delivery::UPGRADE_FAILED, format!("invalid job: {error}"))
                })?;
                if job.schema != MAINTENANCE_JOB_SCHEMA
                    || running.file_stem().and_then(|value| value.to_str()) != Some(&job.id)
                    || job.status != MaintenanceStatus::Queued
                {
                    return Err(CliFailure::new(
                        delivery::UPGRADE_FAILED,
                        "maintenance job identity or state is invalid",
                    ));
                }
                return Ok(Some((running, job)));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(CliFailure::new(
                    delivery::FILESYSTEM_FAILED,
                    error.to_string(),
                ));
            }
        }
    }
    Ok(None)
}

fn execute_upgrade(
    layout: &InstallLayout,
    running_path: &Path,
    job: &mut MaintenanceJob,
    archive: &Path,
    archive_sha256: &str,
) -> Result<Option<String>, CliFailure> {
    #[cfg(not(target_os = "linux"))]
    if job.upgrade_mode == Some(UpgradeMode::BlueGreen) {
        return Err(CliFailure::new(
            delivery::UPGRADE_FAILED,
            "blue_green upgrade requires Linux amd64 and MariaDB",
        ));
    }
    let database = super::installed_database()?;
    #[cfg(target_os = "linux")]
    if job.upgrade_mode == Some(UpgradeMode::BlueGreen) {
        online::preflight(layout, job, &database)?;
    }
    if database
        .required_files(layout)
        .iter()
        .any(|path| !super::regular_file_without_symlink(path))
    {
        return Err(CliFailure::new(
            delivery::UPGRADE_FAILED,
            "installed database configuration or required files are missing",
        ));
    }
    let canonical_upload_root = layout.upgrade_uploads().canonicalize().map_err(|error| {
        CliFailure::new(
            delivery::UPGRADE_FAILED,
            format!("upload root unavailable: {error}"),
        )
    })?;
    let canonical_archive = archive.canonicalize().map_err(|error| {
        CliFailure::new(
            delivery::UPGRADE_FAILED,
            format!("uploaded package unavailable: {error}"),
        )
    })?;
    if !canonical_archive.starts_with(&canonical_upload_root)
        || canonical_archive
            .file_name()
            .and_then(|value| value.to_str())
            != Some(&format!("{}.tar.gz", job.id))
    {
        return Err(CliFailure::new(
            delivery::UPGRADE_FAILED,
            "uploaded package is outside the protected staging directory",
        ));
    }
    set_job_status(
        running_path,
        job,
        MaintenanceStatus::Verifying,
        "正在校验安装包摘要和发布签名",
    )?;
    if sha256_file(&canonical_archive)? != archive_sha256 {
        return Err(CliFailure::new(
            delivery::UPGRADE_FAILED,
            "uploaded package SHA-256 changed after upload",
        ));
    }

    set_job_status(
        running_path,
        job,
        MaintenanceStatus::Staging,
        "正在安全解压并准备候选版本",
    )?;
    let staging = layout.staging().join(format!(".upgrade-{}", job.id));
    remove_scoped_directory(layout, &staging)?;
    fs::create_dir(&staging)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    let release_root = match extract_release_archive(&canonical_archive, &staging) {
        Ok(path) => path,
        Err(error) => {
            let _ = remove_scoped_directory(layout, &staging);
            return Err(error);
        }
    };
    let staged = (|| -> Result<(String, PathBuf), CliFailure> {
        let verified = verify_release_at(&release_root)?;
        validate_release_target(
            verified.claims().platform.as_str(),
            verified.claims().architecture.as_str(),
        )?;
        #[cfg(target_os = "windows")]
        super::verify_windows_instance_support(layout, &release_root)?;
        let version = verified.claims().version.clone();
        validate_upgrade_version(&job.current_version, &version)?;
        let uploaded_manifest_sha256 = sha256_file(&release_root.join("RELEASE.json"))?;
        let candidate_release = layout.release(&version);
        if candidate_release.exists() {
            verify_release_at(&candidate_release)?;
            if sha256_file(&candidate_release.join("RELEASE.json"))? != uploaded_manifest_sha256 {
                return Err(CliFailure::new(
                    delivery::UPGRADE_FAILED,
                    "a different signed build already occupies the target version",
                ));
            }
        } else {
            fs::rename(&release_root, &candidate_release)
                .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
        }
        Ok((version, candidate_release))
    })();
    let (version, candidate_release) = match staged {
        Ok(value) => {
            remove_scoped_directory(layout, &staging)?;
            value
        }
        Err(error) => {
            let _ = remove_scoped_directory(layout, &staging);
            return Err(error);
        }
    };
    if database.is_external() {
        let binary = layout.release_binary(&candidate_release, "aster-control");
        let mut inspect = super::process_as_user("aster-team", &binary);
        inspect
            .arg("inspect-database-configuration")
            .arg("--source")
            .arg(layout.database_configuration());
        #[cfg(target_os = "linux")]
        if job.upgrade_mode == Some(UpgradeMode::BlueGreen) {
            super::slot_preparation::run_preparation_command(
                UpgradeMode::BlueGreen,
                Instant::now() + Duration::from_secs(6),
                inspect,
                "candidate external database support",
            )?;
        } else {
            run_checked(
                inspect,
                delivery::UPGRADE_FAILED,
                "candidate external database support",
            )?;
        }
        #[cfg(not(target_os = "linux"))]
        run_checked(
            inspect,
            delivery::UPGRADE_FAILED,
            "candidate external database support",
        )?;
    }
    prepare_settlement_directory(layout)?;
    job.target_version = Some(version.clone());
    job.candidate_release = Some(candidate_release.clone());
    persist_job(running_path, job)?;

    #[cfg(target_os = "linux")]
    if job.upgrade_mode == Some(UpgradeMode::BlueGreen) {
        online::execute(layout, running_path, job, &candidate_release)?;
        return Ok(None);
    }
    execute_platform_maintenance(layout, running_path, job, &candidate_release, &version)?;

    Ok(Some(format!("Aster Team 已升级到 {version}")))
}

// The service cannot create children of the root-owned data directory. Do
// this under the privileged maintenance owner before either candidate starts.
fn prepare_settlement_directory(layout: &InstallLayout) -> Result<(), CliFailure> {
    for path in [
        layout.root().to_path_buf(),
        layout.data(),
        layout.settlement_outbox(),
    ] {
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && path == layout.settlement_outbox() => {}
            _ => {
                return Err(CliFailure::new(
                    delivery::FILESYSTEM_FAILED,
                    "unsafe settlement directory",
                ));
            }
        }
    }
    #[cfg(unix)]
    {
        let mut command = ProcessCommand::new("install");
        command
            .args(["-d", "-o", "aster-team", "-g", "aster-team", "-m", "0700"])
            .arg(layout.settlement_outbox());
        run_checked(
            command,
            delivery::FILESYSTEM_FAILED,
            "prepare settlement directory",
        )?;
    }
    #[cfg(windows)]
    fs::create_dir_all(layout.settlement_outbox())
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    Ok(())
}

fn cleanup_upgrade_archive(layout: &InstallLayout, job: &MaintenanceJob) -> Result<(), CliFailure> {
    let MaintenanceOperation::Upgrade { archive, .. } = &job.operation else {
        return Ok(());
    };
    let expected = layout.upgrade_uploads().join(format!("{}.tar.gz", job.id));
    if archive != &expected {
        return Err(CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            "refused to remove an upload outside the protected staging directory",
        ));
    }
    match fs::symlink_metadata(archive) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            fs::remove_file(archive)
                .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))
        }
        Ok(_) => Err(CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            "refused to remove an unsafe uploaded package path",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            error.to_string(),
        )),
    }
}

pub(super) fn validate_upgrade_version(current: &str, candidate: &str) -> Result<(), CliFailure> {
    let current = Version::parse(current).map_err(|error| {
        CliFailure::new(
            delivery::UPGRADE_FAILED,
            format!("current version is invalid: {error}"),
        )
    })?;
    let candidate = Version::parse(candidate).map_err(|error| {
        CliFailure::new(
            delivery::UPGRADE_FAILED,
            format!("candidate version is invalid: {error}"),
        )
    })?;
    if candidate <= current {
        return Err(CliFailure::new(
            delivery::UPGRADE_FAILED,
            "the candidate version must be newer than the active version",
        ));
    }
    Ok(())
}

// Slots retain immutable releases; SQLite never runs both business instances.
fn execute_platform_maintenance(
    layout: &InstallLayout,
    running_path: &Path,
    job: &mut MaintenanceJob,
    candidate_release: &Path,
    version: &str,
) -> Result<(), CliFailure> {
    let previous_release = current_release()?;
    verify_release_at(&previous_release)?;
    if read_trimmed(&previous_release.join("VERSION"))? != job.current_version {
        return Err(CliFailure::new(
            delivery::UPGRADE_FAILED,
            "active version changed since the upgrade was queued",
        ));
    }
    let previous = load_or_initialize_active_slot(layout, &previous_release, &job.current_version)?;
    if previous.version != job.current_version {
        return Err(CliFailure::new(
            delivery::UPGRADE_FAILED,
            "active slot metadata requires recovery before another upgrade",
        ));
    }
    validate_runner_release(layout, &previous)?;
    let candidate_slot = previous.slot.other();
    let previous_unit = slot_unit(previous.slot);
    let candidate_unit = slot_unit(candidate_slot);
    job.previous_release = Some(previous_release.clone());
    job.runner_was_running = Some(service_is_active(&runner_unit(&previous))?);
    persist_job(running_path, job)?;
    write_upgrade_snapshot(layout, job, &previous, version)?;
    let old_upstreams = fs::read(layout.caddy_upstreams())
        .map_err(|error| CliFailure::new(delivery::UPGRADE_FAILED, error.to_string()))?;
    let old_selected_release = load_selected_release(&layout.selected_release())
        .ok()
        .filter(|selection| selection.version == previous.version);
    let old_assets = snapshot_service_assets(layout)?;
    let runner_was_running = job.runner_was_running == Some(true);

    let result = (|| {
        set_job_status(
            running_path,
            job,
            MaintenanceStatus::StoppingServices,
            "维护升级：正在停止旧服务，页面、API 和流式调用可能中断",
        )?;
        quiesce_services()?;
        replace_directory_link(
            layout,
            &layout.slot_release(candidate_slot.id()),
            candidate_release,
        )?;
        install_service_assets(layout, candidate_release)?;
        run_service_action("disable", &[previous_unit], delivery::UPGRADE_FAILED)?;
        set_job_status(
            running_path,
            job,
            MaintenanceStatus::StartingCandidate,
            "旧服务已停止，正在准备新版配置、执行资源和数据库迁移",
        )?;
        #[cfg(target_os = "linux")]
        let local_runner = super::slot_preparation::prepare_if_supported(
            layout,
            candidate_release,
            candidate_slot,
            &job.requested_by,
            &job.id,
        )?;
        #[cfg(not(target_os = "linux"))]
        let local_runner = None;
        let candidate = ActiveReleaseSlot {
            schema: if local_runner.is_some() {
                aster_upgrade_core::ACTIVE_SLOT_RUNTIME_SCHEMA
            } else {
                ACTIVE_SLOT_SCHEMA
            }
            .into(),
            slot: candidate_slot,
            version: version.into(),
            local_runner,
        };
        candidate
            .validate()
            .map_err(|error| CliFailure::new(delivery::UPGRADE_FAILED, error.to_string()))?;

        if !start_slot(candidate_slot)? {
            return Err(CliFailure::new(
                delivery::UPGRADE_FAILED,
                "candidate service did not start",
            ));
        }
        set_job_status(
            running_path,
            job,
            MaintenanceStatus::Migrating,
            "正在等待数据库迁移与新版服务健康检查",
        )?;
        wait_for_slot(layout, candidate_slot, CANDIDATE_READY_TIMEOUT)?;
        restore_runner(layout, &candidate, runner_was_running)?;
        run_service_action("enable", &[candidate_unit], delivery::UPGRADE_FAILED)?;
        set_job_status(
            running_path,
            job,
            MaintenanceStatus::SwitchingTraffic,
            "新版健康，正在恢复访问入口",
        )?;
        atomic_replace(
            layout.caddy_upstreams(),
            render_upstreams(candidate_slot, super::instance_ports(layout)?).as_bytes(),
            0o640,
        )?;
        reload_caddy(layout)?;
        replace_directory_link(layout, &layout.current(), candidate_release)?;
        write_selected_release(layout, candidate_release, version)?;
        install_stable_cli(layout, candidate_release)?;
        record_upgrade_audit(candidate_release, "succeeded", version)?;
        // Commit last: recovery must select the old release until all steps succeed.
        write_json(layout.active_slot(), &candidate, 0o640)
    })();
    let Err(error) = result else {
        return Ok(());
    };
    let _ = set_job_status(
        running_path,
        job,
        MaintenanceStatus::RestoringPrevious,
        "升级失败，正在停止候选并检查原版本能否恢复；数据库不回退",
    );
    let restore = (|| {
        // A failed stop is fatal. Never start old code beside a live candidate.
        quiesce_services()?;
        restore_service_assets(&old_assets)?;
        replace_directory_link(layout, &layout.current(), &previous_release)?;
        replace_directory_link(
            layout,
            &layout.slot_release(previous.slot.id()),
            &previous_release,
        )?;
        run_service_action("disable", &[candidate_unit], delivery::UPGRADE_FAILED)?;
        run_service_action("enable", &[previous_unit], delivery::UPGRADE_FAILED)?;
        run_service_action("start", &[previous_unit], delivery::UPGRADE_FAILED)?;
        wait_for_slot(layout, previous.slot, CANDIDATE_READY_TIMEOUT)?;
        atomic_replace(layout.caddy_upstreams(), &old_upstreams, 0o640)?;
        reload_caddy(layout)?;
        if let Some(selection) = &old_selected_release {
            atomic_write_selection(&layout.selected_release(), selection)?;
        } else {
            write_selected_release(layout, &previous_release, &previous.version)?;
        }
        restore_runner(layout, &previous, runner_was_running)?;
        write_json(layout.active_slot(), &previous, 0o640)?;
        let _ = record_upgrade_audit(&previous_release, "failed", version);
        set_job_status(
            running_path,
            job,
            MaintenanceStatus::Failed,
            "原版本服务已恢复，正在同步稳定 CLI；数据库未回退",
        )?;
        install_stable_cli(layout, &previous_release)?;
        Ok::<(), CliFailure>(())
    })();
    Err(CliFailure::new(
        delivery::UPGRADE_FAILED,
        match restore {
            Ok(()) => format!("升级失败，已确认原版本恢复；数据库未回退：{error}"),
            Err(restore_error) => format!(
                "升级失败且原版本恢复未完成，需检查服务状态：{error}; recovery: {restore_error}"
            ),
        },
    ))
}

// Every transition (including recovery) proves both Control slots have stopped.
fn quiesce_services() -> Result<(), CliFailure> {
    stop_services_exclusively(
        |unit| run_service_action("stop", &[unit], delivery::UPGRADE_FAILED),
        super::service_state,
    )?;
    #[cfg(target_os = "linux")]
    for unit in ["aster-runner@blue.service", "aster-runner@green.service"] {
        if super::service_definition_exists(unit)? {
            run_service_action("disable", &[unit], delivery::UPGRADE_FAILED)?;
        }
    }
    Ok(())
}

fn stop_services_exclusively(
    mut stop: impl FnMut(&str) -> Result<(), CliFailure>,
    mut state: impl FnMut(&str) -> Result<String, CliFailure>,
) -> Result<(), CliFailure> {
    for unit in [
        #[cfg(any(target_os = "linux", test))]
        "aster-runner@blue.service",
        #[cfg(any(target_os = "linux", test))]
        "aster-runner@green.service",
        "aster-runner.service",
        "aster-control@blue.service",
        "aster-control@green.service",
    ] {
        let before = state(unit)?;
        if before == "not-installed" {
            continue;
        }
        stop(unit)?;
        let after = state(unit)?;
        if !matches!(
            after.as_str(),
            "inactive" | "failed" | "disabled" | "not-installed"
        ) {
            return Err(CliFailure::new(
                delivery::UPGRADE_FAILED,
                format!("service has not stopped: {unit} ({after})"),
            ));
        }
    }
    Ok(())
}

fn restore_runner(
    layout: &InstallLayout,
    active: &ActiveReleaseSlot,
    was_running: bool,
) -> Result<(), CliFailure> {
    validate_runner_release(layout, active)?;
    let unit = runner_unit(active);
    #[cfg(target_os = "linux")]
    for other in [
        "aster-runner.service",
        "aster-runner@blue.service",
        "aster-runner@green.service",
    ] {
        if other != unit && super::service_definition_exists(other)? {
            run_service_action("disable", &[other], delivery::UPGRADE_FAILED)?;
        }
    }
    if !was_running {
        #[cfg(target_os = "linux")]
        if super::service_definition_exists(&unit)? {
            run_service_action("disable", &[&unit], delivery::UPGRADE_FAILED)?;
        }
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    run_service_action("enable", &[&unit], delivery::UPGRADE_FAILED)?;
    run_service_action("start", &[&unit], delivery::UPGRADE_FAILED)?;
    #[cfg(target_os = "linux")]
    let client = active
        .local_runner
        .as_ref()
        .map(|_| super::runtime_client::RuntimeClient::new(layout, active))
        .transpose()?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let ready = service_is_active(&unit)?;
        #[cfg(target_os = "linux")]
        let ready = ready
            && client
                .as_ref()
                .is_none_or(|client| client.probe_runner(active).is_ok());
        if ready {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(CliFailure::new(
                delivery::UPGRADE_FAILED,
                "Runner did not resume with a verified channel",
            ));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn validate_runner_release(
    layout: &InstallLayout,
    active: &ActiveReleaseSlot,
) -> Result<(), CliFailure> {
    let Some(runner) = &active.local_runner else {
        return Ok(());
    };
    active
        .validate()
        .map_err(|error| CliFailure::new(delivery::UPGRADE_FAILED, error.to_string()))?;
    if sha256_file(&layout.release(&active.version).join("RELEASE.json"))? != runner.manifest_sha256
    {
        return Err(CliFailure::new(
            delivery::UPGRADE_FAILED,
            "active Runner release does not match its committed manifest",
        ));
    }
    let path = layout.runner_slot_identity(active.slot.id());
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| CliFailure::new(delivery::UPGRADE_FAILED, error.to_string()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 4096 {
        return Err(CliFailure::new(
            delivery::UPGRADE_FAILED,
            "active Runner identity is unsafe",
        ));
    }
    let bytes = zeroize::Zeroizing::new(
        fs::read(&path)
            .map_err(|error| CliFailure::new(delivery::UPGRADE_FAILED, error.to_string()))?,
    );
    #[derive(serde::Deserialize)]
    struct Identity {
        schema: String,
        runner_id: String,
    }
    let identity: Identity = serde_json::from_slice(&bytes).map_err(|_| {
        CliFailure::new(
            delivery::UPGRADE_FAILED,
            "active Runner identity is invalid",
        )
    })?;
    if identity.schema != "aster.runner-identity.v1" || identity.runner_id != runner.runner_id {
        return Err(CliFailure::new(
            delivery::UPGRADE_FAILED,
            "active Runner identity does not match its committed slot",
        ));
    }
    Ok(())
}

type ServiceAssetSnapshot = Vec<(PathBuf, Option<Vec<u8>>)>;

fn snapshot_service_assets(layout: &InstallLayout) -> Result<ServiceAssetSnapshot, CliFailure> {
    #[cfg(target_os = "linux")]
    {
        snapshot_system_units(layout)
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        snapshot_files(&platform_asset_destinations(layout)?)
    }
}

fn install_service_assets(layout: &InstallLayout, release: &Path) -> Result<(), CliFailure> {
    #[cfg(target_os = "linux")]
    {
        install_system_units(layout, release)
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        install_platform_service_assets(layout, release)
    }
}

fn restore_service_assets(assets: &[(PathBuf, Option<Vec<u8>>)]) -> Result<(), CliFailure> {
    #[cfg(target_os = "linux")]
    {
        restore_system_units(assets)
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        restore_files(assets)
    }
}

fn reload_caddy(layout: &InstallLayout) -> Result<(), CliFailure> {
    #[cfg(target_os = "linux")]
    {
        let _ = layout;
        systemctl(&["reload", "aster-caddy.service"])
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        reload_platform_caddy(layout)
    }
}

fn execute_version_cleanup(
    layout: &InstallLayout,
    running_path: &Path,
    job: &mut MaintenanceJob,
    version: &str,
) -> Result<String, CliFailure> {
    set_job_status(
        running_path,
        job,
        MaintenanceStatus::Verifying,
        "正在确认版本不再运行",
    )?;
    let active = load_active_slot(layout)?;
    if active.version == version || job.current_version == version {
        return Err(CliFailure::new(
            delivery::UPGRADE_FAILED,
            "current version cannot be deleted",
        ));
    }
    let release = layout.release(version);
    let canonical_release = release.canonicalize().map_err(|error| {
        CliFailure::new(
            delivery::UPGRADE_FAILED,
            format!("version unavailable: {error}"),
        )
    })?;
    let canonical_releases = layout.releases().canonicalize().map_err(|error| {
        CliFailure::new(
            delivery::UPGRADE_FAILED,
            format!("release root unavailable: {error}"),
        )
    })?;
    if canonical_release.parent() != Some(canonical_releases.as_path()) {
        return Err(CliFailure::new(
            delivery::UPGRADE_FAILED,
            "version path is outside the protected release directory",
        ));
    }
    for slot in [ReleaseSlot::Blue, ReleaseSlot::Green] {
        let link = layout.slot_release(slot.id());
        if link.canonicalize().ok().as_ref() == Some(&canonical_release) {
            #[cfg(target_os = "linux")]
            {
                let runner = format!("aster-runner@{}.service", slot.id());
                if super::service_definition_exists(&runner)? {
                    run_service_action("stop", &[&runner], delivery::UPGRADE_FAILED)?;
                    run_service_action("disable", &[&runner], delivery::UPGRADE_FAILED)?;
                }
                systemctl(&["stop", slot_unit(slot)])?;
            }
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            run_service_action("stop", &[slot_unit(slot)], delivery::UPGRADE_FAILED)?;
            remove_link(&link)?;
        }
    }
    let snapshots = layout.upgrade_backups().join(version);
    remove_scoped_directory(layout, &snapshots)?;
    fs::remove_dir_all(&canonical_release)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    Ok(format!("版本 {version} 及对应升级快照已删除"))
}

pub(super) fn set_job_status(
    path: &Path,
    job: &mut MaintenanceJob,
    status: MaintenanceStatus,
    message: impl Into<String>,
) -> Result<(), CliFailure> {
    job.status = status;
    job.message = message.into();
    job.updated_at = now()?;
    persist_job(path, job)
}

fn persist_job(path: &Path, job: &MaintenanceJob) -> Result<(), CliFailure> {
    job.validate().map_err(|error| {
        CliFailure::new(
            delivery::UPGRADE_FAILED,
            format!("invalid job state: {error}"),
        )
    })?;
    let mut persisted = job.clone();
    if persisted.status.terminal() {
        persisted.runner_was_running = None;
    }
    write_json(path.to_path_buf(), &persisted, 0o640)
}

pub(super) fn complete_job(
    layout: &InstallLayout,
    running_path: &Path,
    job: &MaintenanceJob,
) -> Result<(), CliFailure> {
    archive_job_with(layout, running_path, job, || {
        #[cfg(target_os = "linux")]
        super::slot_preparation::finalize_if_supported(layout, job)?;
        Ok(())
    })?;
    #[cfg(target_os = "linux")]
    super::slot_preparation::cleanup_completed_material(layout)?;
    Ok(())
}

fn archive_job_with(
    layout: &InstallLayout,
    running_path: &Path,
    job: &MaintenanceJob,
    finalize: impl FnOnce() -> Result<(), CliFailure>,
) -> Result<(), CliFailure> {
    if !job.status.terminal() {
        return Err(CliFailure::new(
            delivery::UPGRADE_FAILED,
            "cannot archive an unfinished maintenance job",
        ));
    }
    job.validate()
        .map_err(|error| CliFailure::new(delivery::UPGRADE_FAILED, error.to_string()))?;
    let destination = layout.upgrade_completed().join(format!("{}.json", job.id));
    match fs::symlink_metadata(&destination) {
        Ok(_) => {
            return Err(CliFailure::new(
                delivery::UPGRADE_FAILED,
                "a completed job already has this identity",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(CliFailure::new(
                delivery::FILESYSTEM_FAILED,
                error.to_string(),
            ));
        }
    }
    // A failed or lost finalization reply leaves a terminal job in running.
    // Recovery retries it against the same durable active slot and signed receipt.
    persist_job(running_path, job)?;
    finalize()?;
    fs::rename(running_path, destination)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    #[cfg(unix)]
    for directory in [layout.upgrade_running(), layout.upgrade_completed()] {
        File::open(directory)
            .and_then(|file| file.sync_all())
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    }
    Ok(())
}

fn extract_release_archive(archive: &Path, staging: &Path) -> Result<PathBuf, CliFailure> {
    let source = File::open(archive)
        .map_err(|error| CliFailure::new(delivery::UPGRADE_FAILED, error.to_string()))?;
    let mut archive = Archive::new(GzDecoder::new(source));
    let mut root = None::<PathBuf>;
    let mut files = 0_usize;
    let mut bytes = 0_u64;
    let entries = archive
        .entries()
        .map_err(|error| CliFailure::new(delivery::UPGRADE_FAILED, error.to_string()))?;
    for entry in entries {
        let mut entry =
            entry.map_err(|error| CliFailure::new(delivery::UPGRADE_FAILED, error.to_string()))?;
        let entry_type = entry.header().entry_type();
        if !entry_type.is_file() && !entry_type.is_dir() {
            return Err(CliFailure::new(
                delivery::UPGRADE_FAILED,
                "release archive contains a link or unsupported entry type",
            ));
        }
        let path = entry
            .path()
            .map_err(|error| CliFailure::new(delivery::UPGRADE_FAILED, error.to_string()))?;
        let mut components = path.components();
        let Some(Component::Normal(first)) = components.next() else {
            return Err(CliFailure::new(
                delivery::UPGRADE_FAILED,
                "release archive contains an unsafe path",
            ));
        };
        if components.any(|component| !matches!(component, Component::Normal(_))) {
            return Err(CliFailure::new(
                delivery::UPGRADE_FAILED,
                "release archive contains path traversal",
            ));
        }
        let first = PathBuf::from(first);
        if root.as_ref().is_some_and(|existing| existing != &first) {
            return Err(CliFailure::new(
                delivery::UPGRADE_FAILED,
                "release archive must contain exactly one top-level directory",
            ));
        }
        root = Some(first);
        files = files.saturating_add(1);
        bytes = bytes.saturating_add(entry.size());
        if files > MAX_EXTRACTED_FILES || bytes > MAX_EXTRACTED_BYTES {
            return Err(CliFailure::new(
                delivery::UPGRADE_FAILED,
                "release archive expands beyond the allowed limit",
            ));
        }
        if !entry
            .unpack_in(staging)
            .map_err(|error| CliFailure::new(delivery::UPGRADE_FAILED, error.to_string()))?
        {
            return Err(CliFailure::new(
                delivery::UPGRADE_FAILED,
                "release archive attempted to escape the staging directory",
            ));
        }
    }
    let root =
        root.ok_or_else(|| CliFailure::new(delivery::UPGRADE_FAILED, "release archive is empty"))?;
    let release_root = staging.join(root);
    if !release_root.join("RELEASE.json").is_file() {
        return Err(CliFailure::new(
            delivery::UPGRADE_FAILED,
            "release archive does not contain RELEASE.json at its root",
        ));
    }
    Ok(release_root)
}

fn validate_release_target(platform: &str, architecture: &str) -> Result<(), CliFailure> {
    let expected_platform = Platform::current().id();
    let expected_architecture = current_architecture();
    if platform != expected_platform || architecture != expected_architecture {
        return Err(CliFailure::new(
            delivery::UPGRADE_FAILED,
            format!(
                "release targets {platform}/{architecture}, expected {expected_platform}/{expected_architecture}"
            ),
        ));
    }
    Ok(())
}

fn current_architecture() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "amd64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "unsupported"
    }
}

fn load_or_initialize_active_slot(
    layout: &InstallLayout,
    current: &Path,
    version: &str,
) -> Result<ActiveReleaseSlot, CliFailure> {
    if layout.active_slot().is_file() {
        return load_active_slot(layout);
    }
    let active = ActiveReleaseSlot {
        schema: ACTIVE_SLOT_SCHEMA.to_owned(),
        local_runner: None,
        slot: ReleaseSlot::Blue,
        version: version.to_owned(),
    };
    replace_directory_link(layout, &layout.slot_release("blue"), current)?;
    write_json(layout.active_slot(), &active, 0o640)?;
    Ok(active)
}

fn load_active_slot(layout: &InstallLayout) -> Result<ActiveReleaseSlot, CliFailure> {
    let source = fs::read(layout.active_slot()).map_err(|error| {
        CliFailure::new(
            delivery::UPGRADE_FAILED,
            format!("active slot unavailable: {error}"),
        )
    })?;
    let active: ActiveReleaseSlot = serde_json::from_slice(&source).map_err(|error| {
        CliFailure::new(
            delivery::UPGRADE_FAILED,
            format!("active slot invalid: {error}"),
        )
    })?;
    active.validate().map_err(|error| {
        CliFailure::new(
            delivery::UPGRADE_FAILED,
            format!("active slot invalid: {error}"),
        )
    })?;
    if active.local_runner.is_some()
        && !cfg!(any(all(target_os = "linux", target_arch = "x86_64"), test))
    {
        return Err(CliFailure::new(
            delivery::UPGRADE_FAILED,
            "local Runner topology requires Linux amd64",
        ));
    }
    Ok(active)
}

#[derive(Serialize)]
struct UpgradeSnapshot<'a> {
    schema: &'static str,
    job_id: &'a str,
    previous_version: &'a str,
    target_version: &'a str,
    previous_slot: ReleaseSlot,
    created_at: String,
    database_policy: &'static str,
}

fn write_upgrade_snapshot(
    layout: &InstallLayout,
    job: &MaintenanceJob,
    previous: &ActiveReleaseSlot,
    target_version: &str,
) -> Result<(), CliFailure> {
    let directory = layout
        .upgrade_backups()
        .join(&previous.version)
        .join(&job.id);
    fs::create_dir_all(&directory)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    write_json(
        directory.join("snapshot.json"),
        &UpgradeSnapshot {
            schema: "aster.upgrade-snapshot.v1",
            job_id: &job.id,
            previous_version: &previous.version,
            target_version,
            previous_slot: previous.slot,
            created_at: now()?,
            database_policy: "forward_only_no_automatic_restore",
        },
        0o600,
    )
}

fn write_selected_release(
    layout: &InstallLayout,
    release: &Path,
    version: &str,
) -> Result<(), CliFailure> {
    let release = release.canonicalize().map_err(|error| {
        CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            format!(
                "cannot resolve selected release {}: {error}",
                release.display()
            ),
        )
    })?;
    atomic_write_selection(
        &layout.selected_release(),
        &SelectedRelease {
            schema: super::SELECTED_RELEASE_SCHEMA.to_owned(),
            version: version.to_owned(),
            release_root: release.clone(),
            manifest_sha256: sha256_file(&release.join("RELEASE.json"))?,
        },
    )
}

fn configured_slot_ports(
    slot: ReleaseSlot,
    ports: aster_install_layout::WindowsPorts,
) -> aster_upgrade_core::ReleaseSlotPorts {
    let (api, member, admin) = match slot {
        ReleaseSlot::Blue => (ports.blue_api, ports.blue_member, ports.blue_admin),
        ReleaseSlot::Green => (ports.green_api, ports.green_member, ports.green_admin),
    };
    aster_upgrade_core::ReleaseSlotPorts { api, member, admin }
}

pub(super) fn render_upstreams(
    slot: ReleaseSlot,
    ports: aster_install_layout::WindowsPorts,
) -> String {
    let ports = configured_slot_ports(slot, ports);
    // Established by installation/maintenance before any online cutover. The
    // online adapter must still compare live retention with its actual budget.
    [("api", ports.api), ("member", ports.member), ("admin", ports.admin)]
        .into_iter()
        .map(|(role, port)| format!(
            "(aster_{role}_upstream) {{\n\treverse_proxy 127.0.0.1:{port} {{\n\t\tstream_close_delay 15m\n\t}}\n}}\n"
        ))
        .collect::<Vec<_>>()
        .join("\n")
}

fn start_slot(slot: ReleaseSlot) -> Result<bool, CliFailure> {
    let unit = slot_unit(slot);
    #[cfg(target_os = "windows")]
    {
        start_windows_candidate(|action| {
            run_service_action(action, &[unit], delivery::UPGRADE_FAILED)
        })
    }
    #[cfg(not(target_os = "windows"))]
    {
        #[cfg(target_os = "linux")]
        let _ = systemctl(&["reset-failed", unit]);
        run_service_action("start", &[unit], delivery::UPGRADE_FAILED)?;
        service_is_active(unit)
    }
}

#[cfg(any(target_os = "windows", test))]
fn start_windows_candidate(
    mut dispatch: impl FnMut(&str) -> Result<(), CliFailure>,
) -> Result<bool, CliFailure> {
    // The inactive slot is disabled after installation or a previous upgrade.
    // Task Scheduler dispatch is asynchronous; wait_for_slot verifies readiness.
    dispatch("enable")?;
    dispatch("start")?;
    Ok(true)
}

fn wait_for_slot(
    layout: &InstallLayout,
    slot: ReleaseSlot,
    timeout: Duration,
) -> Result<(), CliFailure> {
    let ports = configured_slot_ports(slot, super::instance_ports(layout)?);
    let deadline = Instant::now() + timeout;
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), ports.admin);
    let request = format!(
        "GET /healthz HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
        ports.admin
    );
    let mut last_error = String::new();
    while Instant::now() < deadline {
        match TcpStream::connect_timeout(&address, Duration::from_secs(2)) {
            Ok(mut stream) => {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                if let Err(error) = stream.write_all(request.as_bytes()) {
                    last_error = error.to_string();
                } else {
                    let mut response = [0_u8; 256];
                    match stream.read(&mut response) {
                        Ok(count)
                            if String::from_utf8_lossy(&response[..count])
                                .starts_with("HTTP/1.1 2") =>
                        {
                            return Ok(());
                        }
                        Ok(count) => {
                            last_error = String::from_utf8_lossy(&response[..count]).into_owned()
                        }
                        Err(error) => last_error = error.to_string(),
                    }
                }
            }
            Err(error) => last_error = error.to_string(),
        }
        thread::sleep(Duration::from_secs(1));
    }
    Err(CliFailure::new(
        delivery::UPGRADE_FAILED,
        format!("candidate health check timed out: {last_error}"),
    ))
}

fn slot_unit(slot: ReleaseSlot) -> &'static str {
    match slot {
        ReleaseSlot::Blue => "aster-control@blue.service",
        ReleaseSlot::Green => "aster-control@green.service",
    }
}

#[cfg(any(target_os = "linux", test))]
fn systemctl(arguments: &[&str]) -> Result<(), CliFailure> {
    let mut command = ProcessCommand::new("systemctl");
    command.args(arguments);
    run_checked(command, delivery::SERVICE_FAILED, "systemctl")
}

#[cfg(any(target_os = "linux", test))]
fn snapshot_system_units(layout: &InstallLayout) -> Result<SystemUnitSnapshot, CliFailure> {
    let root = layout
        .service_registration_root()
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?
        .ok_or_else(|| {
            CliFailure::new(
                delivery::FILESYSTEM_FAILED,
                "service registration root is unavailable on this platform",
            )
        })?;
    SYSTEM_UNITS
        .iter()
        .map(|name| {
            let path = root.join(name);
            let source = match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                    Some(fs::read(&path).map_err(|error| {
                        CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string())
                    })?)
                }
                Ok(_) => {
                    return Err(CliFailure::new(
                        delivery::FILESYSTEM_FAILED,
                        format!("service unit path is unsafe: {}", path.display()),
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(CliFailure::new(
                        delivery::FILESYSTEM_FAILED,
                        error.to_string(),
                    ));
                }
            };
            Ok((path, source))
        })
        .collect()
}

#[cfg(any(target_os = "linux", test))]
fn install_system_units(layout: &InstallLayout, release: &Path) -> Result<(), CliFailure> {
    let root = layout
        .service_registration_root()
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?
        .ok_or_else(|| {
            CliFailure::new(
                delivery::FILESYSTEM_FAILED,
                "service registration root is unavailable on this platform",
            )
        })?;
    let install_root = layout.root().to_str().ok_or_else(|| {
        CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            "installation root is not valid UTF-8",
        )
    })?;
    let source_root = layout
        .release_platform_path(release, "service_templates")
        .map_err(|error| CliFailure::new(delivery::UPGRADE_FAILED, error.to_string()))?;
    for name in SYSTEM_UNITS {
        if let Some(template) = read_system_unit_template(&source_root, name)? {
            let rendered = template.replace("@ASTER_ROOT@", install_root);
            atomic_replace(root.join(name), rendered.as_bytes(), 0o644)?;
        } else {
            remove_optional_system_unit(&root.join(name))?;
        }
    }
    systemctl(&["daemon-reload"])
}

#[cfg(any(target_os = "linux", test))]
fn read_system_unit_template(source_root: &Path, name: &str) -> Result<Option<String>, CliFailure> {
    let source = source_root.join(name);
    let metadata = match fs::symlink_metadata(&source) {
        Ok(metadata) => metadata,
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound && name == "aster-runner@.service" =>
        {
            return Ok(None);
        }
        Err(error) => {
            return Err(CliFailure::new(
                delivery::UPGRADE_FAILED,
                format!(
                    "candidate service unit {} is unavailable: {error}",
                    source.display()
                ),
            ));
        }
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(CliFailure::new(
            delivery::UPGRADE_FAILED,
            format!("candidate service unit is unsafe: {}", source.display()),
        ));
    }
    fs::read_to_string(&source)
        .map(Some)
        .map_err(|error| CliFailure::new(delivery::UPGRADE_FAILED, error.to_string()))
}

#[cfg(any(target_os = "linux", test))]
fn remove_optional_system_unit(path: &Path) -> Result<(), CliFailure> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            fs::remove_file(path)
                .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))
        }
        Ok(_) => Err(CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            "optional service unit path is unsafe",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            error.to_string(),
        )),
    }
}

#[cfg(any(target_os = "linux", test))]
fn restore_system_units(units: &[SystemUnitState]) -> Result<(), CliFailure> {
    for (path, source) in units {
        if let Some(source) = source {
            atomic_replace(path.clone(), source, 0o644)?;
        } else {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(CliFailure::new(
                        delivery::FILESYSTEM_FAILED,
                        error.to_string(),
                    ));
                }
            }
        }
    }
    systemctl(&["daemon-reload"])
}

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos", test))]
fn install_stable_cli(layout: &InstallLayout, release: &Path) -> Result<(), CliFailure> {
    let source = layout.release_binary(release, "aster-team-cli");
    let metadata = fs::symlink_metadata(&source).map_err(|error| {
        CliFailure::new(
            delivery::UPGRADE_FAILED,
            format!("candidate CLI is unavailable: {error}"),
        )
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(CliFailure::new(
            delivery::UPGRADE_FAILED,
            "candidate CLI path is unsafe",
        ));
    }
    let bytes = fs::read(source)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    atomic_replace(layout.stable_cli(), &bytes, 0o755)
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn platform_asset_destinations(layout: &InstallLayout) -> Result<Vec<PathBuf>, CliFailure> {
    let paths = vec![
        layout
            .platform_service_launcher()
            .map_err(|error| CliFailure::new(delivery::UPGRADE_FAILED, error.to_string()))?,
        layout.stable_caddy(),
    ];
    #[cfg(target_os = "windows")]
    return Ok(paths);
    #[cfg(target_os = "macos")]
    {
        let mut paths = paths;
        let registration_root = layout
            .service_registration_root()
            .map_err(|error| CliFailure::new(delivery::UPGRADE_FAILED, error.to_string()))?
            .ok_or_else(|| {
                CliFailure::new(
                    delivery::UPGRADE_FAILED,
                    "macOS service registration root is unavailable",
                )
            })?;
        paths.extend(
            MACOS_SERVICE_LABELS
                .iter()
                .map(|label| registration_root.join(format!("{label}.plist"))),
        );
        Ok(paths)
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn snapshot_files(paths: &[PathBuf]) -> Result<FileSnapshot, CliFailure> {
    paths
        .iter()
        .map(|path| {
            let source = match fs::symlink_metadata(path) {
                Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                    Some(fs::read(path).map_err(|error| {
                        CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string())
                    })?)
                }
                Ok(_) => {
                    return Err(CliFailure::new(
                        delivery::FILESYSTEM_FAILED,
                        format!("service asset path is unsafe: {}", path.display()),
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(CliFailure::new(
                        delivery::FILESYSTEM_FAILED,
                        error.to_string(),
                    ));
                }
            };
            Ok((path.clone(), source))
        })
        .collect()
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn restore_files(files: &[(PathBuf, Option<Vec<u8>>)]) -> Result<(), CliFailure> {
    for (path, source) in files {
        if let Some(source) = source {
            atomic_replace(path.clone(), source, 0o755)?;
        } else {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(CliFailure::new(
                        delivery::FILESYSTEM_FAILED,
                        error.to_string(),
                    ));
                }
            }
        }
    }
    Ok(())
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn install_platform_service_assets(
    layout: &InstallLayout,
    release: &Path,
) -> Result<(), CliFailure> {
    let assets = [
        (
            layout
                .release_platform_path(release, "service_launcher")
                .map_err(|error| CliFailure::new(delivery::UPGRADE_FAILED, error.to_string()))?,
            layout
                .platform_service_launcher()
                .map_err(|error| CliFailure::new(delivery::UPGRADE_FAILED, error.to_string()))?,
        ),
        (
            layout.release_binary(release, "caddy"),
            layout.stable_caddy(),
        ),
    ];
    for (source, destination) in assets {
        let metadata = fs::symlink_metadata(&source).map_err(|error| {
            CliFailure::new(
                delivery::UPGRADE_FAILED,
                format!(
                    "candidate service asset {} is unavailable: {error}",
                    source.display()
                ),
            )
        })?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(CliFailure::new(
                delivery::UPGRADE_FAILED,
                format!("candidate service asset is unsafe: {}", source.display()),
            ));
        }
        let bytes = fs::read(&source)
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
        atomic_replace(destination, &bytes, 0o755)?;
    }
    #[cfg(target_os = "macos")]
    install_macos_service_definitions(layout, release)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn install_macos_service_definitions(
    layout: &InstallLayout,
    release: &Path,
) -> Result<(), CliFailure> {
    let source_root = layout
        .release_platform_path(release, "service_templates")
        .map_err(|error| CliFailure::new(delivery::UPGRADE_FAILED, error.to_string()))?;
    let destination_root = layout
        .service_registration_root()
        .map_err(|error| CliFailure::new(delivery::UPGRADE_FAILED, error.to_string()))?
        .ok_or_else(|| {
            CliFailure::new(
                delivery::UPGRADE_FAILED,
                "macOS service registration root is unavailable",
            )
        })?;
    let root = layout.root().to_str().ok_or_else(|| {
        CliFailure::new(
            delivery::UPGRADE_FAILED,
            "installation root is not valid UTF-8",
        )
    })?;
    for label in MACOS_SERVICE_LABELS {
        let name = format!("{label}.plist");
        let source = source_root.join(&name);
        let metadata = fs::symlink_metadata(&source).map_err(|error| {
            CliFailure::new(
                delivery::UPGRADE_FAILED,
                format!(
                    "candidate launchd definition {} is unavailable: {error}",
                    source.display()
                ),
            )
        })?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(CliFailure::new(
                delivery::UPGRADE_FAILED,
                format!(
                    "candidate launchd definition is unsafe: {}",
                    source.display()
                ),
            ));
        }
        let rendered = fs::read_to_string(&source)
            .map_err(|error| CliFailure::new(delivery::UPGRADE_FAILED, error.to_string()))?
            .replace("@ASTER_ROOT@", &xml_escape(root));
        atomic_replace(destination_root.join(name), rendered.as_bytes(), 0o644)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn reload_platform_caddy(layout: &InstallLayout) -> Result<(), CliFailure> {
    let mut command = ProcessCommand::new(layout.stable_caddy());
    command
        .args(["reload", "--config"])
        .arg(layout.caddyfile())
        .args(["--adapter", "caddyfile", "--force", "--address"])
        .arg(format!(
            "127.0.0.1:{}",
            super::instance_ports(layout)?.caddy_admin
        ))
        .env("HOME", layout.caddy_data())
        .env("XDG_DATA_HOME", layout.caddy_data())
        .env("XDG_CONFIG_HOME", layout.caddy_config());
    run_checked(command, delivery::UPGRADE_FAILED, "Caddy reload")
}

fn append_upgrade_audit_arguments(
    command: &mut ProcessCommand,
    outcome: &str,
    target_version: &str,
) {
    command.args([
        "record-upgrade-audit",
        "--outcome",
        outcome,
        "--target-version",
        target_version,
    ]);
}

#[cfg(target_os = "linux")]
fn record_upgrade_audit(
    release: &Path,
    outcome: &str,
    target_version: &str,
) -> Result<(), CliFailure> {
    let mut command = ProcessCommand::new("runuser");
    command
        .args(["-u", "aster-team", "--"])
        .arg(install_layout().release_binary(release, "aster-control"));
    append_upgrade_audit_arguments(&mut command, outcome, target_version);
    run_checked(command, delivery::UPGRADE_FAILED, "upgrade audit")
}

#[cfg(target_os = "windows")]
fn record_upgrade_audit(
    release: &Path,
    outcome: &str,
    target_version: &str,
) -> Result<(), CliFailure> {
    let mut command =
        ProcessCommand::new(install_layout().release_binary(release, "aster-control"));
    append_upgrade_audit_arguments(&mut command, outcome, target_version);
    run_checked(command, delivery::UPGRADE_FAILED, "upgrade audit")
}

#[cfg(target_os = "macos")]
fn record_upgrade_audit(
    release: &Path,
    outcome: &str,
    target_version: &str,
) -> Result<(), CliFailure> {
    let mut command = ProcessCommand::new("sudo");
    command
        .args(["-u", "aster-team", "--"])
        .arg(install_layout().release_binary(release, "aster-control"));
    append_upgrade_audit_arguments(&mut command, outcome, target_version);
    run_checked(command, delivery::UPGRADE_FAILED, "upgrade audit")
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn replace_directory_link(
    layout: &InstallLayout,
    link: &Path,
    target: &Path,
) -> Result<(), CliFailure> {
    if !link.starts_with(layout.root()) || !target.starts_with(layout.releases()) {
        return Err(CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            "release link is outside the installation root",
        ));
    }
    let parent = link.parent().ok_or_else(|| {
        CliFailure::new(delivery::FILESYSTEM_FAILED, "release link has no parent")
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    let temporary = parent.join(format!(".slot-link-{}", std::process::id()));
    remove_link(&temporary)?;
    create_directory_link(target, &temporary)?;
    fs::rename(&temporary, link)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn create_directory_link(target: &Path, link: &Path) -> Result<(), CliFailure> {
    std::os::unix::fs::symlink(target, link)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))
}

#[cfg(target_os = "windows")]
fn replace_directory_link(
    layout: &InstallLayout,
    link: &Path,
    target: &Path,
) -> Result<(), CliFailure> {
    use std::os::windows::fs::MetadataExt as _;

    // canonicalize() returns a verbatim Windows path. Rollback must compare
    // that against the same normalized root used by installation discovery.
    let normalized_target = InstallLayout::new(target)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    let target = normalized_target.root();
    if !link.starts_with(layout.root()) || !target.starts_with(layout.releases()) {
        return Err(CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            "release link is outside the installation root",
        ));
    }
    let parent = link.parent().ok_or_else(|| {
        CliFailure::new(delivery::FILESYSTEM_FAILED, "release link has no parent")
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        OffsetDateTime::now_utc().unix_timestamp_nanos()
    );
    let temporary = parent.join(format!(".slot-link-{nonce}"));
    let previous = parent.join(format!(".slot-previous-{nonce}"));
    remove_link(&temporary)?;
    create_directory_link(target, &temporary)?;
    let had_previous = match fs::symlink_metadata(link) {
        Ok(metadata)
            if metadata.file_type().is_symlink() && metadata.file_attributes() & 0x10 != 0 =>
        {
            fs::rename(link, &previous).map_err(|error| {
                let _ = remove_link(&temporary);
                CliFailure::new(
                    delivery::FILESYSTEM_FAILED,
                    format!("cannot retire release link {}: {error}", link.display()),
                )
            })?;
            true
        }
        Ok(_) => {
            let _ = remove_link(&temporary);
            return Err(CliFailure::new(
                delivery::FILESYSTEM_FAILED,
                format!(
                    "refused to replace non-directory-link path {}",
                    link.display()
                ),
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            let _ = remove_link(&temporary);
            return Err(CliFailure::new(
                delivery::FILESYSTEM_FAILED,
                error.to_string(),
            ));
        }
    };
    if let Err(error) = fs::rename(&temporary, link) {
        if had_previous {
            let _ = fs::rename(&previous, link);
        }
        let _ = remove_link(&temporary);
        return Err(CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            format!("cannot publish release link {}: {error}", link.display()),
        ));
    }
    if had_previous {
        remove_link(&previous)?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn create_directory_link(target: &Path, link: &Path) -> Result<(), CliFailure> {
    std::os::windows::fs::symlink_dir(target, link)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))
}

fn remove_link(path: &Path) -> Result<(), CliFailure> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt as _;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 => {
                // symlink_metadata().is_dir() is false for directory symlinks
                // and junctions. The Windows directory attribute still applies.
                if metadata.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0 {
                    fs::remove_dir(path)
                } else {
                    fs::remove_file(path)
                }
                .map_err(|error| {
                    CliFailure::new(
                        delivery::FILESYSTEM_FAILED,
                        format!("cannot remove release link {}: {error}", path.display()),
                    )
                })
            }
            Ok(_) => Err(CliFailure::new(
                delivery::FILESYSTEM_FAILED,
                format!("refused to replace non-link path {}", path.display()),
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(CliFailure::new(
                delivery::FILESYSTEM_FAILED,
                error.to_string(),
            )),
        }
    }
    #[cfg(not(target_os = "windows"))]
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::remove_file(path)
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string())),
        Ok(_) => Err(CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            format!("refused to replace non-link path {}", path.display()),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            error.to_string(),
        )),
    }
}

fn write_json(path: PathBuf, value: &impl Serialize, mode: u32) -> Result<(), CliFailure> {
    let mut encoded = serde_json::to_vec_pretty(value)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    encoded.push(b'\n');
    atomic_replace(path, &encoded, mode)
}

fn write_json_new(path: PathBuf, value: &impl Serialize, mode: u32) -> Result<(), CliFailure> {
    let mut encoded = serde_json::to_vec_pretty(value)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    encoded.push(b'\n');
    let parent = path
        .parent()
        .ok_or_else(|| CliFailure::new(delivery::FILESYSTEM_FAILED, "file path has no parent"))?;
    fs::create_dir_all(parent)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    super::set_mode(&path, mode)?;
    output
        .write_all(&encoded)
        .and_then(|()| output.sync_all())
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))
}

pub(crate) fn atomic_replace(path: PathBuf, value: &[u8], mode: u32) -> Result<(), CliFailure> {
    let parent = path
        .parent()
        .ok_or_else(|| CliFailure::new(delivery::FILESYSTEM_FAILED, "file path has no parent"))?;
    fs::create_dir_all(parent)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    #[cfg(target_os = "windows")]
    {
        cleanup_retired_windows_files(parent)?;
        if fs::symlink_metadata(&path)
            .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
            && fs::read(&path).is_ok_and(|current| current == value)
        {
            return Ok(());
        }
    }
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        OffsetDateTime::now_utc().unix_timestamp_nanos()
    );
    let temporary = parent.join(format!(".maintenance-{nonce}.tmp"));
    let _ = fs::remove_file(&temporary);
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    super::set_mode(&temporary, mode)?;
    #[cfg(target_os = "linux")]
    if path.exists() {
        let status = ProcessCommand::new("chown")
            .arg(format!("--reference={}", path.display()))
            .arg(&temporary)
            .status()
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
        if !status.success() {
            let _ = fs::remove_file(&temporary);
            return Err(CliFailure::new(
                delivery::FILESYSTEM_FAILED,
                format!("could not preserve ownership for {}", path.display()),
            ));
        }
    }
    let result = output.write_all(value).and_then(|()| output.sync_all());
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            error.to_string(),
        ));
    }
    drop(output);
    #[cfg(not(target_os = "windows"))]
    if let Err(error) = fs::rename(&temporary, &path) {
        let _ = fs::remove_file(&temporary);
        return Err(CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            error.to_string(),
        ));
    }
    #[cfg(target_os = "windows")]
    {
        let previous = parent.join(format!(".maintenance-{nonce}.previous"));
        let had_previous = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                fs::rename(&path, &previous).map_err(|error| {
                    let _ = fs::remove_file(&temporary);
                    CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string())
                })?;
                true
            }
            Ok(_) => {
                let _ = fs::remove_file(&temporary);
                return Err(CliFailure::new(
                    delivery::FILESYSTEM_FAILED,
                    format!("refused to replace unsafe file path {}", path.display()),
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                return Err(CliFailure::new(
                    delivery::FILESYSTEM_FAILED,
                    error.to_string(),
                ));
            }
        };
        if let Err(error) = fs::rename(&temporary, &path) {
            if had_previous {
                let _ = fs::rename(&previous, &path);
            }
            let _ = fs::remove_file(&temporary);
            return Err(CliFailure::new(
                delivery::FILESYSTEM_FAILED,
                error.to_string(),
            ));
        }
        if had_previous && !remove_retired_windows_file(&previous)? {
            eprintln!(
                "[WARN] Windows still has the previous service executable open; maintenance will retry cleanup after it exits: {}",
                previous.display()
            );
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn remove_retired_windows_file(path: &Path) -> Result<bool, CliFailure> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.permissions().readonly()
    {
        return Err(CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            "unsafe retired service asset",
        ));
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        // Windows permits renaming an image while it runs, but not deleting it.
        // Keep the uniquely named old image; it is not a failure to switch files.
        Err(error) if matches!(error.raw_os_error(), Some(5 | 32)) => Ok(false),
        Err(error) => Err(CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            error.to_string(),
        )),
    }
}

#[cfg(target_os = "windows")]
fn cleanup_retired_windows_files(directory: &Path) -> Result<(), CliFailure> {
    if !directory.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(directory)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?
    {
        let entry = entry
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
        let name = entry.file_name();
        let Some(nonce) = name
            .to_str()
            .and_then(|name| name.strip_prefix(".maintenance-"))
            .and_then(|name| name.strip_suffix(".previous"))
        else {
            continue;
        };
        let Some((pid, timestamp)) = nonce.split_once('-') else {
            continue;
        };
        if pid.parse::<u32>().is_ok() && timestamp.parse::<u128>().is_ok() {
            remove_retired_windows_file(&entry.path())?;
        }
    }
    Ok(())
}

fn remove_scoped_directory(layout: &InstallLayout, path: &Path) -> Result<(), CliFailure> {
    if !path.starts_with(layout.root()) || path == layout.root() {
        return Err(CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            "refused to remove a directory outside the installation root",
        ));
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            fs::remove_dir_all(path)
                .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))
        }
        Ok(_) => Err(CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            format!("refused to remove unsafe path {}", path.display()),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            error.to_string(),
        )),
    }
}

fn now() -> Result<String, CliFailure> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_releases_may_omit_only_the_new_slot_runner_template() {
        let directory = tempfile::tempdir().unwrap();
        assert!(
            read_system_unit_template(directory.path(), "aster-runner@.service")
                .unwrap()
                .is_none()
        );
        assert!(read_system_unit_template(directory.path(), "aster-runner.service").is_err());
        let template = directory.path().join("aster-runner@.service");
        fs::write(&template, "[Service]\nExecStart=@ASTER_ROOT@/slot-runner\n").unwrap();
        assert!(
            read_system_unit_template(directory.path(), "aster-runner@.service")
                .unwrap()
                .is_some()
        );
        remove_optional_system_unit(&template).unwrap();
        remove_optional_system_unit(&template).unwrap();
        fs::create_dir(&template).unwrap();
        assert!(read_system_unit_template(directory.path(), "aster-runner@.service").is_err());
        assert!(remove_optional_system_unit(&template).is_err());
        assert!(template.is_dir());
    }

    #[test]
    fn maintenance_requires_confirmed_stops_and_fails_closed() {
        use std::cell::RefCell;
        let units = [
            "aster-runner@blue.service",
            "aster-runner@green.service",
            "aster-runner.service",
            "aster-control@blue.service",
            "aster-control@green.service",
        ];
        for (failed_index, failing_unit) in units.iter().enumerate() {
            for failure in [None, Some("stop"), Some("active"), Some("unknown")] {
                let stopped = RefCell::new(Vec::new());
                let result = stop_services_exclusively(
                    |unit| {
                        stopped.borrow_mut().push(unit.to_owned());
                        if unit == *failing_unit && failure == Some("stop") {
                            return Err(CliFailure::new(delivery::UPGRADE_FAILED, "stop failed"));
                        }
                        Ok(())
                    },
                    |unit| {
                        Ok(if stopped.borrow().iter().any(|item| item == unit) {
                            if unit == *failing_unit {
                                match failure {
                                    Some("active") => "active",
                                    Some("unknown") => "unknown",
                                    _ => "inactive",
                                }
                            } else {
                                "inactive"
                            }
                        } else {
                            "active"
                        }
                        .to_owned())
                    },
                );
                assert_eq!(result.is_ok(), failure.is_none());
                let calls = stopped.into_inner();
                let expected = if failure.is_none() {
                    units.len()
                } else {
                    failed_index + 1
                };
                assert_eq!(calls, units[..expected]);
            }
        }
    }

    #[test]
    fn legacy_maintenance_does_not_stop_uninstalled_slot_runners() {
        let mut stopped = Vec::new();
        stop_services_exclusively(
            |unit| {
                stopped.push(unit.to_owned());
                Ok(())
            },
            |unit| {
                super::super::linux_service_state(
                    if unit.starts_with("aster-runner@") {
                        "LoadState=not-found\nActiveState=inactive\n"
                    } else {
                        "LoadState=loaded\nActiveState=inactive\n"
                    },
                    true,
                )
            },
        )
        .unwrap();
        assert_eq!(
            stopped,
            [
                "aster-runner.service",
                "aster-control@blue.service",
                "aster-control@green.service"
            ]
        );
    }

    #[test]
    fn archival_waits_for_retryable_finalization_and_never_archives_unfinished_jobs() {
        let (_directory, layout, path, mut job) =
            recovery_fixture(MaintenanceStatus::SwitchingTraffic);
        let calls = std::cell::Cell::new(0);
        assert!(
            archive_job_with(&layout, &path, &job, || {
                calls.set(calls.get() + 1);
                Ok(())
            })
            .is_err()
        );
        assert_eq!(calls.get(), 0);
        job.status = MaintenanceStatus::Succeeded;
        let completed = layout.upgrade_completed().join(format!("{}.json", job.id));
        assert!(
            archive_job_with(&layout, &path, &job, || {
                calls.set(calls.get() + 1);
                Err(CliFailure::new(
                    delivery::UPGRADE_FAILED,
                    "lost finalization reply",
                ))
            })
            .is_err()
        );
        assert!(path.exists());
        assert!(!completed.exists());
        let retained: MaintenanceJob = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(retained.status, MaintenanceStatus::Succeeded);
        archive_job_with(&layout, &path, &job, || {
            calls.set(calls.get() + 1);
            Ok(())
        })
        .unwrap();
        assert_eq!(calls.get(), 2);
        assert!(!path.exists());
        assert!(completed.exists());
    }

    #[test]
    fn archival_identity_collision_never_runs_finalization_or_replaces_history() {
        let (_directory, layout, path, mut job) =
            recovery_fixture(MaintenanceStatus::SwitchingTraffic);
        job.status = MaintenanceStatus::Succeeded;
        let completed = layout.upgrade_completed().join(format!("{}.json", job.id));
        fs::write(&completed, b"existing historical record").unwrap();
        let called = std::cell::Cell::new(false);
        assert!(
            archive_job_with(&layout, &path, &job, || {
                called.set(true);
                Ok(())
            })
            .is_err()
        );
        assert!(!called.get());
        assert!(path.exists());
        assert_eq!(fs::read(&completed).unwrap(), b"existing historical record");
    }

    #[test]
    fn completed_legacy_job_omits_runtime_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(directory.path()).unwrap();
        fs::create_dir_all(layout.upgrade_running()).unwrap();
        fs::create_dir_all(layout.upgrade_completed()).unwrap();
        let path = layout.upgrade_running().join("legacy.json");
        let job = MaintenanceJob {
            schema: MAINTENANCE_JOB_SCHEMA.to_owned(),
            id: "legacy".into(),
            requested_by: "cli".into(),
            operation: MaintenanceOperation::Upgrade {
                archive: directory.path().join("legacy.tar.gz"),
                archive_sha256: "a".repeat(64),
            },
            status: MaintenanceStatus::Failed,
            upgrade_mode: None,
            runner_was_running: Some(true),
            current_version: "2.0.0".into(),
            target_version: Some("2.0.1".into()),
            previous_release: None,
            candidate_release: None,
            message: "failed; old service recovered".into(),
            created_at: now().unwrap(),
            updated_at: now().unwrap(),
        };
        complete_job(&layout, &path, &job).unwrap();
        let encoded = fs::read(layout.upgrade_completed().join("legacy.json")).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        assert!(value.get("upgrade_mode").is_none());
        assert!(value.get("runner_was_running").is_none());
        let restored: MaintenanceJob = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(restored.status, MaintenanceStatus::Failed);
        assert_eq!(restored.message, job.message);
        assert!(!path.exists());
    }

    fn recovery_fixture(
        status: MaintenanceStatus,
    ) -> (tempfile::TempDir, InstallLayout, PathBuf, MaintenanceJob) {
        let directory = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(directory.path()).unwrap();
        fs::create_dir_all(layout.upgrade_running()).unwrap();
        fs::create_dir_all(layout.upgrade_completed()).unwrap();
        fs::create_dir_all(layout.active_slot().parent().unwrap()).unwrap();
        write_json(
            layout.active_slot(),
            &ActiveReleaseSlot {
                schema: ACTIVE_SLOT_SCHEMA.into(),
                local_runner: None,
                slot: ReleaseSlot::Blue,
                version: "2.0.0".into(),
            },
            0o640,
        )
        .unwrap();
        let job = MaintenanceJob {
            schema: MAINTENANCE_JOB_SCHEMA.into(),
            id: "interrupted".into(),
            requested_by: "cli".into(),
            operation: MaintenanceOperation::Upgrade {
                archive: layout.upgrade_uploads().join("interrupted.tar.gz"),
                archive_sha256: "a".repeat(64),
            },
            status,
            upgrade_mode: None,
            runner_was_running: None,
            current_version: "2.0.0".into(),
            target_version: Some("2.0.1".into()),
            previous_release: Some(layout.release("2.0.0")),
            candidate_release: Some(layout.release("2.0.1")),
            message: String::new(),
            created_at: now().unwrap(),
            updated_at: now().unwrap(),
        };
        let path = layout.upgrade_running().join("interrupted.json");
        persist_job(&path, &job).unwrap();
        (directory, layout, path, job)
    }

    #[test]
    fn online_journal_blocks_maintenance_before_any_recovery_or_archival() {
        use aster_upgrade_core::online::OnlineJournal;
        let (_directory, layout, path, _) = recovery_fixture(MaintenanceStatus::DrainingPrevious);
        let job_before = fs::read(&path).unwrap();
        let mut store = super::super::online_journal::JournalFile::open(&layout).unwrap();
        OnlineJournal::create(super::super::online_journal::tests::plan(), &mut store).unwrap();
        drop(store);
        assert!(
            recover_interrupted_jobs_with(
                &layout,
                |_, _| panic!("online slots must not enter maintenance recovery"),
                || panic!("must not query or stop maintenance services"),
                |_| panic!("must not change CLI pointers"),
            )
            .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), job_before);
        assert_eq!(fs::read_dir(layout.upgrade_completed()).unwrap().count(), 0);
    }

    #[test]
    fn missing_online_journal_does_not_authorize_maintenance_shutdown() {
        let (_directory, layout, path, mut job) =
            recovery_fixture(MaintenanceStatus::DrainingPrevious);
        job.upgrade_mode = Some(UpgradeMode::BlueGreen);
        persist_job(&path, &job).unwrap();
        let before = fs::read(&path).unwrap();
        assert!(
            recover_interrupted_jobs_with(
                &layout,
                |_, _| panic!("a missing journal is not permission to stop online slots"),
                || panic!(),
                |_| panic!(),
            )
            .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn online_failures_preserve_task_upload_and_uncertain_evidence() {
        use aster_upgrade_core::online::OnlineJournal;
        for kind in ["transition", "corrupt-preparation", "missing"] {
            let (_directory, layout, path, mut job) =
                recovery_fixture(MaintenanceStatus::StartingCandidate);
            job.upgrade_mode = if kind == "corrupt-preparation" {
                None
            } else {
                Some(UpgradeMode::BlueGreen)
            };
            persist_job(&path, &job).unwrap();
            fs::create_dir_all(layout.upgrade_uploads()).unwrap();
            let archive = layout.upgrade_uploads().join("interrupted.tar.gz");
            fs::write(&archive, b"retained upload").unwrap();
            let evidence = match kind {
                "transition" => {
                    let mut store =
                        super::super::online_journal::JournalFile::open(&layout).unwrap();
                    OnlineJournal::create(super::super::online_journal::tests::plan(), &mut store)
                        .unwrap();
                    Some(layout.upgrade_state().join("online-transition.json"))
                }
                "corrupt-preparation" => {
                    let evidence = layout.upgrade_state().join("online-preparation.json");
                    fs::write(&evidence, b"{").unwrap();
                    Some(evidence)
                }
                _ => None,
            };
            let before = evidence.as_ref().map(|path| fs::read(path).unwrap());
            let error = finish_failed_job(
                &layout,
                &path,
                &mut job,
                CliFailure::new(delivery::UPGRADE_FAILED, "candidate response lost"),
            )
            .unwrap_err();
            assert!(error.to_string().contains("candidate response lost"));
            let saved: MaintenanceJob = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(saved.status, MaintenanceStatus::StartingCandidate);
            assert_eq!(saved.upgrade_mode, job.upgrade_mode);
            assert!(saved.message.contains("candidate response lost"));
            assert_eq!(fs::read(&archive).unwrap(), b"retained upload");
            assert_eq!(fs::read_dir(layout.upgrade_completed()).unwrap().count(), 0);
            assert_eq!(
                evidence.as_ref().map(|path| fs::read(path).unwrap()),
                before
            );
        }
    }

    #[test]
    fn online_archival_failure_never_resurrects_an_already_completed_job() {
        let (_directory, layout, path, mut job) =
            recovery_fixture(MaintenanceStatus::StartingCandidate);
        job.upgrade_mode = Some(UpgradeMode::BlueGreen);
        let completed = layout.upgrade_completed().join(format!("{}.json", job.id));
        let mut finished = job.clone();
        finished.status = MaintenanceStatus::Succeeded;
        fs::write(&completed, serde_json::to_vec(&finished).unwrap()).unwrap();
        fs::remove_file(&path).unwrap();
        let before = fs::read(&completed).unwrap();
        assert!(
            finish_failed_job(
                &layout,
                &path,
                &mut job,
                CliFailure::new(delivery::UPGRADE_FAILED, "journal archival failed")
            )
            .is_err()
        );
        assert!(!path.exists());
        assert_eq!(fs::read(completed).unwrap(), before);
    }

    #[test]
    fn failed_recovery_is_not_archived_as_a_finished_upgrade() {
        let (_directory, layout, path, mut job) =
            recovery_fixture(MaintenanceStatus::RestoringPrevious);
        assert!(
            finish_failed_job(
                &layout,
                &path,
                &mut job,
                CliFailure::new(delivery::UPGRADE_FAILED, "old service unavailable")
            )
            .is_err()
        );
        assert!(path.exists());
        assert_eq!(fs::read_dir(layout.upgrade_completed()).unwrap().count(), 0);
        let saved: MaintenanceJob = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved.status, MaintenanceStatus::RestoringPrevious);
        assert!(saved.message.contains("old service unavailable"));
    }

    #[test]
    fn direct_upgrade_refuses_an_existing_task_instead_of_executing_it() {
        let (_directory, layout, _, _) = recovery_fixture(MaintenanceStatus::Queued);
        fs::create_dir_all(layout.upgrade_queued()).unwrap();
        require_empty_upgrade_queue(&layout).unwrap();
        let existing = layout.upgrade_queued().join("existing.json");
        fs::write(&existing, b"existing job").unwrap();
        assert!(require_empty_upgrade_queue(&layout).is_err());
        assert_eq!(fs::read(existing).unwrap(), b"existing job");
    }

    #[test]
    fn validation_failure_finishes_without_a_recovery_loop() {
        let (_directory, layout, path, mut job) = recovery_fixture(MaintenanceStatus::Verifying);
        assert!(
            finish_failed_job(
                &layout,
                &path,
                &mut job,
                CliFailure::new(delivery::UPGRADE_FAILED, "invalid signature")
            )
            .is_err()
        );
        assert!(!path.exists());
        let saved: MaintenanceJob = serde_json::from_slice(
            &fs::read(layout.upgrade_completed().join("interrupted.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(saved.status, MaintenanceStatus::Failed);
    }

    #[test]
    fn preparation_interruptions_do_not_restart_healthy_services() {
        for status in [
            MaintenanceStatus::Queued,
            MaintenanceStatus::Verifying,
            MaintenanceStatus::Staging,
        ] {
            let (_directory, layout, path, _) = recovery_fixture(status);
            recover_interrupted_jobs_with(
                &layout,
                |_, _| panic!("must not restart services"),
                || panic!("must not query Runner"),
                |_| panic!("must not replace CLI"),
            )
            .unwrap();
            assert!(!path.exists());
            let job: MaintenanceJob = serde_json::from_slice(
                &fs::read(layout.upgrade_completed().join("interrupted.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(job.status, MaintenanceStatus::Failed);
        }
    }

    #[test]
    fn recovery_failure_retains_checkpoint_and_retries_without_losing_runner_state() {
        let (_directory, layout, path, _) = recovery_fixture(MaintenanceStatus::RestoringPrevious);
        assert!(
            recover_interrupted_jobs_with(
                &layout,
                |_, runner| {
                    assert!(runner);
                    Err(CliFailure::new(
                        delivery::UPGRADE_FAILED,
                        "candidate stop failed",
                    ))
                },
                || Ok(true),
                |_| panic!("cannot replace CLI during failed recovery")
            )
            .is_err()
        );
        let pending: MaintenanceJob = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(pending.status, MaintenanceStatus::RestoringPrevious);
        assert_eq!(pending.runner_was_running, Some(true));
        assert_eq!(fs::read_dir(layout.upgrade_completed()).unwrap().count(), 0);
        recover_interrupted_jobs_with(
            &layout,
            |active, runner| {
                assert_eq!(active.version, "2.0.0");
                assert!(runner);
                Ok(())
            },
            || panic!("must reuse the checkpoint even if Runner is now stopped"),
            |_| {
                // This is the point at which the older CLI becomes active.
                let encoded: serde_json::Value =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                assert_eq!(encoded["status"], "failed");
                assert!(encoded.get("runner_was_running").is_none());
                assert!(encoded.get("upgrade_mode").is_none());
                Ok(())
            },
        )
        .unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn committed_switch_recovers_candidate_and_preserves_a_stopped_runner() {
        let (_directory, layout, path, mut job) =
            recovery_fixture(MaintenanceStatus::SwitchingTraffic);
        job.runner_was_running = Some(false);
        persist_job(&path, &job).unwrap();
        write_json(
            layout.active_slot(),
            &ActiveReleaseSlot {
                schema: ACTIVE_SLOT_SCHEMA.into(),
                local_runner: None,
                slot: ReleaseSlot::Green,
                version: "2.0.1".into(),
            },
            0o640,
        )
        .unwrap();
        recover_interrupted_jobs_with(
            &layout,
            |active, runner| {
                assert_eq!(active.slot, ReleaseSlot::Green);
                assert!(!runner);
                Ok(())
            },
            || panic!("checkpoint already exists"),
            |_| Ok(()),
        )
        .unwrap();
        let completed: MaintenanceJob = serde_json::from_slice(
            &fs::read(layout.upgrade_completed().join("interrupted.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(completed.status, MaintenanceStatus::Succeeded);
    }

    #[test]
    fn recovery_chooses_the_committed_control_runner_pair() {
        for committed in [false, true] {
            let (_directory, layout, path, mut job) =
                recovery_fixture(MaintenanceStatus::SwitchingTraffic);
            job.runner_was_running = Some(true);
            persist_job(&path, &job).unwrap();
            if committed {
                write_json(
                    layout.active_slot(),
                    &ActiveReleaseSlot {
                        schema: aster_upgrade_core::ACTIVE_SLOT_RUNTIME_SCHEMA.into(),
                        slot: ReleaseSlot::Green,
                        version: "2.0.1".into(),
                        local_runner: Some(aster_upgrade_core::ActiveLocalRunner {
                            runner_id: format!("runner_{}", "a".repeat(32)),
                            manifest_sha256: "b".repeat(64),
                        }),
                    },
                    0o640,
                )
                .unwrap();
            }
            recover_interrupted_jobs_with(
                &layout,
                |active, was_running| {
                    assert!(was_running);
                    assert_eq!(
                        runner_unit(active),
                        if committed {
                            "aster-runner@green.service"
                        } else {
                            "aster-runner.service"
                        }
                    );
                    assert_eq!(
                        active.slot,
                        if committed {
                            ReleaseSlot::Green
                        } else {
                            ReleaseSlot::Blue
                        }
                    );
                    Ok(())
                },
                || panic!("must use persisted Runner state"),
                |_| Ok(()),
            )
            .unwrap();
            let completed: MaintenanceJob = serde_json::from_slice(
                &fs::read(layout.upgrade_completed().join("interrupted.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(
                completed.status,
                if committed {
                    MaintenanceStatus::Succeeded
                } else {
                    MaintenanceStatus::Failed
                }
            );
        }
    }

    #[test]
    fn committed_runner_identity_and_release_must_match_before_start() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(temporary.path()).unwrap();
        fs::create_dir_all(layout.release("2.1.0")).unwrap();
        let manifest = layout.release("2.1.0").join("RELEASE.json");
        fs::write(&manifest, b"signed release fixture").unwrap();
        let path = layout.runner_slot_identity("green");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let runner_id = format!("runner_{}", "a".repeat(32));
        fs::write(
            &path,
            serde_json::to_vec(
                &serde_json::json!({"schema":"aster.runner-identity.v1", "runner_id":runner_id}),
            )
            .unwrap(),
        )
        .unwrap();
        let active = ActiveReleaseSlot {
            schema: aster_upgrade_core::ACTIVE_SLOT_RUNTIME_SCHEMA.into(),
            slot: ReleaseSlot::Green,
            version: "2.1.0".into(),
            local_runner: Some(aster_upgrade_core::ActiveLocalRunner {
                runner_id,
                manifest_sha256: sha256_file(&manifest).unwrap(),
            }),
        };
        validate_runner_release(&layout, &active).unwrap();
        fs::write(&manifest, b"different release").unwrap();
        assert!(validate_runner_release(&layout, &active).is_err());
        fs::write(&manifest, b"signed release fixture").unwrap();
        fs::write(
            &path,
            br#"{"schema":"aster.runner-identity.v1","runner_id":"other"}"#,
        )
        .unwrap();
        assert!(validate_runner_release(&layout, &active).is_err());
    }

    #[test]
    fn unreadable_or_corrupt_recovery_records_are_never_silently_deleted() {
        for bytes in [b"{broken".to_vec(), vec![b'x'; 256 * 1024 + 1]] {
            let (_directory, layout, path, _) =
                recovery_fixture(MaintenanceStatus::RestoringPrevious);
            fs::write(&path, &bytes).unwrap();
            assert!(
                recover_interrupted_jobs_with(
                    &layout,
                    |_, _| panic!("invalid recovery cannot touch services"),
                    || panic!(),
                    |_| panic!()
                )
                .is_err()
            );
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn terminal_checkpoint_retries_cli_sync_without_restarting_services() {
        let (_directory, layout, path, _) = recovery_fixture(MaintenanceStatus::Failed);
        assert!(
            recover_interrupted_jobs_with(
                &layout,
                |_, _| panic!(),
                || panic!(),
                |_| Err(CliFailure::new(delivery::UPGRADE_FAILED, "CLI locked"))
            )
            .is_err()
        );
        assert!(path.exists());
        recover_interrupted_jobs_with(&layout, |_, _| panic!(), || panic!(), |_| Ok(())).unwrap();
        assert!(!path.exists());
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    #[test]
    fn service_asset_recovery_cannot_replace_the_executor_before_terminal_checkpoint() {
        let directory = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(directory.path()).unwrap();
        let assets = platform_asset_destinations(&layout).unwrap();
        assert!(assets.contains(&layout.platform_service_launcher().unwrap()));
        assert!(!assets.contains(&layout.stable_cli()));
    }

    #[test]
    fn windows_candidate_is_enabled_before_asynchronous_start() {
        let mut actions = Vec::new();
        assert!(
            start_windows_candidate(|action| {
                actions.push(action.to_owned());
                Ok(())
            })
            .unwrap()
        );
        assert_eq!(actions, ["enable", "start"]);
    }

    #[test]
    fn windows_candidate_dispatch_failure_is_not_treated_as_started() {
        for failing_action in ["enable", "start"] {
            let mut actions = Vec::new();
            let result = start_windows_candidate(|action| {
                actions.push(action.to_owned());
                if action == failing_action {
                    Err(CliFailure::new(delivery::UPGRADE_FAILED, "dispatch failed"))
                } else {
                    Ok(())
                }
            });
            assert!(result.is_err());
            let expected = if failing_action == "enable" { 1 } else { 2 };
            assert_eq!(actions.len(), expected);
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_atomic_child_holds_its_executable() {
        if let Some(ready) = std::env::var_os("ASTER_WINDOWS_ATOMIC_CHILD") {
            fs::write(ready, b"ready").unwrap();
            thread::sleep(Duration::from_secs(30));
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_replaces_a_running_image_and_retries_only_its_retired_files() {
        struct ChildGuard(std::process::Child);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path();
        let executable = directory.join("fixture.exe");
        let ready = directory.join("ready");
        fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
        let mut child = ChildGuard(
            ProcessCommand::new(&executable)
                .args([
                    "--exact",
                    "maintenance_executor::tests::windows_atomic_child_holds_its_executable",
                    "--nocapture",
                ])
                .env("ASTER_WINDOWS_ATOMIC_CHILD", &ready)
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "child exited before probe"
            );
            assert!(Instant::now() < deadline, "child startup timed out");
            thread::sleep(Duration::from_millis(25));
        }
        let original = fs::read(&executable).unwrap();
        atomic_replace(executable.clone(), &original, 0o755).unwrap();
        assert_eq!(
            fs::read_dir(directory).unwrap().count(),
            2,
            "identical executable was needlessly replaced"
        );
        atomic_replace(executable.clone(), b"new executable contents", 0o755).unwrap();
        assert_eq!(fs::read(&executable).unwrap(), b"new executable contents");
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "replacement stopped the old process"
        );
        assert_eq!(fs::read_dir(directory).unwrap().count(), 3);
        let unrelated = directory.join(".maintenance-user.previous");
        fs::write(&unrelated, b"keep").unwrap();
        cleanup_retired_windows_files(directory).unwrap();
        assert_eq!(
            fs::read_dir(directory).unwrap().count(),
            4,
            "locked previous image was not retained"
        );
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        cleanup_retired_windows_files(directory).unwrap();
        assert_eq!(fs::read_dir(directory).unwrap().count(), 3);
        assert_eq!(fs::read(unrelated).unwrap(), b"keep");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_upgrade_selection_preserves_the_canonical_path_contract() {
        let directory = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(directory.path()).unwrap();
        let release = layout.release("2.0.1");
        fs::create_dir_all(&release).unwrap();
        fs::write(release.join("RELEASE.json"), b"manifest-hash-fixture").unwrap();
        write_selected_release(&layout, &release, "2.0.1").unwrap();
        let selected: SelectedRelease =
            serde_json::from_slice(&fs::read(layout.selected_release()).unwrap()).unwrap();
        // load_selected_release rejects noncanonical roots before signature verification.
        assert_eq!(selected.release_root, release.canonicalize().unwrap());
        assert_eq!(selected.version, "2.0.1");
        assert_eq!(
            selected.manifest_sha256,
            sha256_file(&release.join("RELEASE.json")).unwrap()
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_release_links_switch_and_restore_canonical_targets() {
        let directory = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(directory.path()).unwrap();
        let previous = layout.release("2.0.0");
        let candidate = layout.release("2.0.1");
        for (release, value) in [
            (&previous, b"previous".as_slice()),
            (&candidate, b"candidate"),
        ] {
            fs::create_dir_all(release).unwrap();
            fs::write(release.join("sentinel"), value).unwrap();
        }
        // init.ps1 creates a junction. Subsequent switches create symlink dirs.
        let script = format!(
            "$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path '{}' -Target '{}' | Out-Null",
            layout.current().display().to_string().replace('\'', "''"),
            previous.display().to_string().replace('\'', "''")
        );
        assert!(
            ProcessCommand::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                .status()
                .unwrap()
                .success()
        );
        let canonical_previous = layout.current().canonicalize().unwrap();
        if let Err(error) = replace_directory_link(&layout, &layout.current(), &candidate) {
            if error.descriptor.code == "DELIVERY_FILESYSTEM_FAILED"
                && error.detail.ends_with("(os error 1314)")
            {
                eprintln!("skipping symlink switch test without Windows symlink privileges");
                return;
            }
            panic!("failed to switch the release pointer: {error:?}");
        }
        assert_eq!(
            fs::read(layout.current().join("sentinel")).unwrap(),
            b"candidate"
        );
        replace_directory_link(&layout, &layout.current(), &canonical_previous).unwrap();
        assert_eq!(
            fs::read(layout.current().join("sentinel")).unwrap(),
            b"previous"
        );
        remove_link(&layout.current()).unwrap();
        assert_eq!(fs::read(previous.join("sentinel")).unwrap(), b"previous");
        assert_eq!(fs::read(candidate.join("sentinel")).unwrap(), b"candidate");
        assert_eq!(fs::read_dir(layout.root()).unwrap().count(), 1);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_release_links_refuse_real_directories_before_mutation() {
        let directory = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(directory.path()).unwrap();
        let candidate = layout.release("2.0.1");
        fs::create_dir_all(&candidate).unwrap();
        fs::create_dir(layout.current()).unwrap();
        fs::write(layout.current().join("sentinel"), b"keep").unwrap();
        assert!(replace_directory_link(&layout, &layout.current(), &candidate).is_err());
        assert!(
            !fs::symlink_metadata(layout.current())
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read(layout.current().join("sentinel")).unwrap(),
            b"keep"
        );
        assert_eq!(fs::read_dir(layout.root()).unwrap().count(), 2);
    }

    #[test]
    fn blue_and_green_upstreams_are_distinct() {
        let blue = render_upstreams(
            ReleaseSlot::Blue,
            aster_install_layout::WindowsPorts::default(),
        );
        let green = render_upstreams(
            ReleaseSlot::Green,
            aster_install_layout::WindowsPorts::default(),
        );
        assert!(blue.contains("127.0.0.1:11380"));
        assert!(green.contains("127.0.0.1:11482"));
        assert_ne!(blue, green);
    }

    #[test]
    fn target_validation_matches_the_current_binary() {
        let architecture = if cfg!(target_arch = "x86_64") {
            "amd64"
        } else {
            "arm64"
        };
        validate_release_target(Platform::current().id(), architecture).unwrap();
        assert!(validate_release_target("other", architecture).is_err());
    }

    #[test]
    fn custom_upstreams_and_health_probes_use_the_same_slot_ports() {
        let ports = aster_install_layout::WindowsInstance::from_environment(|name| {
            (name == "ASTER_PORT_OFFSET").then(|| "10000".into())
        })
        .unwrap()
        .ports;
        for (slot, api, member, admin) in [
            (ReleaseSlot::Blue, 21380, 21381, 21382),
            (ReleaseSlot::Green, 21480, 21481, 21482),
        ] {
            let actual = configured_slot_ports(slot, ports);
            assert_eq!(
                (actual.api, actual.member, actual.admin),
                (api, member, admin)
            );
            let upstreams = render_upstreams(slot, ports);
            for port in [api, member, admin] {
                assert!(upstreams.contains(&format!("127.0.0.1:{port}")));
            }
        }
    }

    #[test]
    fn upgrades_only_move_to_a_newer_semantic_version() {
        validate_upgrade_version("2.0.0", "2.0.1").unwrap();
        validate_upgrade_version("2.0.0-rc.1", "2.0.0").unwrap();
        assert!(validate_upgrade_version("2.0.0", "2.0.0").is_err());
        assert!(validate_upgrade_version("2.0.1", "2.0.0").is_err());
        assert!(validate_upgrade_version("2.0.0", "invalid").is_err());
    }

    #[test]
    fn upgrade_audit_uses_the_installation_database_configuration() {
        let mut command = ProcessCommand::new("aster-control");
        append_upgrade_audit_arguments(&mut command, "succeeded", "2.0.1");
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            arguments,
            [
                "record-upgrade-audit",
                "--outcome",
                "succeeded",
                "--target-version",
                "2.0.1",
            ]
        );
    }

    #[test]
    fn linux_service_update_helpers_are_part_of_the_checked_upgrade_contract() {
        let _: fn(&[&str]) -> Result<(), CliFailure> = systemctl;
        let _: fn(&InstallLayout) -> Result<SystemUnitSnapshot, CliFailure> = snapshot_system_units;
        let _: fn(&InstallLayout, &Path) -> Result<(), CliFailure> = install_system_units;
        let _: fn(&[SystemUnitState]) -> Result<(), CliFailure> = restore_system_units;
        let _: fn(&InstallLayout, &Path) -> Result<(), CliFailure> = install_stable_cli;
        assert_eq!(SYSTEM_UNITS.len(), 6);
    }
}
