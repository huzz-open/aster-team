use super::*;
use aster_upgrade_core::{MAINTENANCE_JOB_SCHEMA, ReleaseSlot};

fn fixture(
    phase: SwitchbackPhase,
) -> (
    tempfile::TempDir,
    InstallLayout,
    SwitchbackJournal,
    MaintenanceJob,
) {
    let (directory, layout) = crate::proxy_disk::tests::fixture();
    let journal = crate::proxy_client::switchback_tests::journal(&layout, ReleaseSlot::Blue, phase);
    let plan = journal.plan().original.plan();
    fs::create_dir_all(layout.upgrade_running()).unwrap();
    fs::create_dir_all(layout.upgrade_completed()).unwrap();
    let job = MaintenanceJob {
        schema: MAINTENANCE_JOB_SCHEMA.into(),
        id: plan.job_id.clone(),
        requested_by: "owner".into(),
        operation: MaintenanceOperation::Upgrade {
            archive: layout
                .upgrade_uploads()
                .join(format!("{}.tar.gz", plan.job_id)),
            archive_sha256: "a".repeat(64),
        },
        status: MaintenanceStatus::DrainingPrevious,
        upgrade_mode: Some(UpgradeMode::BlueGreen),
        runner_was_running: Some(true),
        current_version: plan.previous.version.clone(),
        target_version: Some(plan.candidate.version.clone()),
        previous_release: Some(layout.release(&plan.previous.version)),
        candidate_release: Some(layout.release(&plan.candidate.version)),
        message: "in progress".into(),
        created_at: "2026-09-09T00:00:00Z".into(),
        updated_at: "2026-09-09T00:00:00Z".into(),
    };
    write(&running_path(&layout, &job.id), &job);
    (directory, layout, journal, job)
}
fn write(path: &Path, job: &MaintenanceJob) {
    fs::write(path, serde_json::to_vec(job).unwrap()).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o640)).unwrap();
}

#[test]
fn reverse_terminal_failure_is_allowed_only_after_durable_complete() {
    for phase in [
        SwitchbackPhase::PreparingPrevious,
        SwitchbackPhase::CommittingPrevious,
        SwitchbackPhase::Complete,
    ] {
        let (_directory, layout, journal, original) = fixture(phase);
        assert_eq!(load_job(&layout, &journal).unwrap(), original);
        for status in [MaintenanceStatus::Failed, MaintenanceStatus::Succeeded] {
            let mut changed = original.clone();
            changed.status = status;
            write(&running_path(&layout, &original.id), &changed);
            assert_eq!(
                load_job(&layout, &journal).is_ok(),
                phase == SwitchbackPhase::Complete && status == MaintenanceStatus::Failed
            );
        }
    }
}

#[test]
fn reverse_recovery_rejects_unrelated_or_duplicate_jobs_and_accepts_exact_failed_archive() {
    let (_directory, layout, journal, mut original) = fixture(SwitchbackPhase::Complete);
    for fault in 0..4 {
        let mut changed = original.clone();
        match fault {
            0 => changed.id = "unrelated".into(),
            1 => changed.current_version = "9.0.0".into(),
            2 => changed.upgrade_mode = Some(UpgradeMode::Maintenance),
            _ => changed.candidate_release = Some(layout.release("9.0.0")),
        }
        write(&running_path(&layout, &original.id), &changed);
        assert!(load_job(&layout, &journal).is_err());
    }
    original.status = MaintenanceStatus::Failed;
    write(&running_path(&layout, &original.id), &original);
    write(&completed_path(&layout, &original.id), &original);
    assert!(load_job(&layout, &journal).is_err());
    fs::remove_file(running_path(&layout, &original.id)).unwrap();
    assert_eq!(load_job(&layout, &journal).unwrap(), original);
    original.status = MaintenanceStatus::Succeeded;
    write(&completed_path(&layout, &original.id), &original);
    assert!(load_job(&layout, &journal).is_err());
}

#[test]
fn unfinished_or_unverified_reverse_cannot_run_commit_commands_or_archive_job() {
    for phase in [
        SwitchbackPhase::PreparingPrevious,
        SwitchbackPhase::CommittingPrevious,
        SwitchbackPhase::Complete,
    ] {
        let (_directory, layout, journal, original) = fixture(phase);
        let path = running_path(&layout, &original.id);
        let bytes = fs::read(&path).unwrap();
        // No signed installation has been provided: even a structurally valid
        // terminal journal must not reach systemctl/runuser or change the task.
        assert!(commit(&layout, &journal, Instant::now() + Duration::from_secs(5)).is_err());
        assert!(finish(&layout, &journal).is_err());
        assert_eq!(fs::read(path).unwrap(), bytes);
        assert!(!completed_path(&layout, &original.id).exists());
    }
}
