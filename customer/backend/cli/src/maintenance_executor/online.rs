//! New online tasks share archive verification with maintenance, but never
//! call its service shutdown, asset replacement or completion path.
use super::*;
use aster_install_layout::DatabaseConfiguration;

pub(super) fn preflight(
    layout: &InstallLayout,
    job: &MaintenanceJob,
    database: &DatabaseConfiguration,
) -> Result<ActiveReleaseSlot, CliFailure> {
    if !cfg!(target_arch = "x86_64")
        || !matches!(database, DatabaseConfiguration::Mariadb { .. })
        || job.upgrade_mode != Some(UpgradeMode::BlueGreen)
    {
        return Err(CliFailure::new(
            delivery::UPGRADE_FAILED,
            "online upgrade requires Linux amd64 and MariaDB",
        ));
    }
    super::super::online_journal::ensure_maintenance_allowed(layout)?;
    let active = load_active_slot(layout)?;
    if active.schema != aster_upgrade_core::ACTIVE_SLOT_RUNTIME_SCHEMA
        || active.version != job.current_version
        || current_release()?
            != layout
                .release(&active.version)
                .canonicalize()
                .map_err(|error| CliFailure::new(delivery::UPGRADE_FAILED, error.to_string()))?
    {
        return Err(CliFailure::new(
            delivery::UPGRADE_FAILED,
            "online upgrade requires the unchanged active runtime slot",
        ));
    }
    let previous = layout.release(&active.version);
    verify_release_at(&previous)?;
    validate_runner_release(layout, &active)?;
    super::super::slot_preparation::require_online_support(&previous)?;
    Ok(active)
}

pub(super) fn execute(
    layout: &InstallLayout,
    running_path: &Path,
    job: &mut MaintenanceJob,
    candidate: &Path,
) -> Result<(), CliFailure> {
    let previous = preflight(layout, job, &super::super::installed_database()?)?;
    super::super::slot_preparation::require_online_support(candidate)?;
    // Persist ownership before preparation can write credentials or a slot link.
    job.previous_release = Some(layout.release(&previous.version));
    job.runner_was_running = Some(service_is_active(&runner_unit(&previous))?);
    if job.runner_was_running != Some(true) {
        return Err(CliFailure::new(
            delivery::UPGRADE_FAILED,
            "online upgrade requires the active local Runner",
        ));
    }
    set_job_status(
        running_path,
        job,
        MaintenanceStatus::StartingCandidate,
        "正在准备在线升级候选实例，旧实例继续提供服务",
    )?;
    super::super::slot_preparation::prepare_if_supported(
        layout,
        candidate,
        previous.slot.other(),
        &job.requested_by,
        &job.id,
    )?
    .ok_or_else(|| {
        CliFailure::new(
            delivery::UPGRADE_FAILED,
            "online candidate preparation is unavailable",
        )
    })?;
    if !super::super::online_deployment::run_prepared(layout)? {
        return Err(CliFailure::new(
            delivery::UPGRADE_FAILED,
            "online preparation did not produce a durable transition",
        ));
    }
    // run_prepared/run_existing completed and archived the actual durable job.
    // The caller must not recreate it from its stale in-memory copy.
    let _ = cleanup_upgrade_archive(layout, job);
    Ok(())
}
