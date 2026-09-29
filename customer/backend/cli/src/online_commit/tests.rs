use super::*;
use aster_upgrade_core::{MAINTENANCE_JOB_SCHEMA, online::OnlineJournalStorage};

struct Store;
impl OnlineJournalStorage for Store {
    type Error = ();
    fn load(&self) -> Result<Option<OnlineJournal>, ()> {
        unreachable!()
    }
    fn assert_current(&mut self, _: &OnlineJournal) -> Result<(), ()> {
        unreachable!()
    }
    fn save(&mut self, _: Option<&OnlineJournal>, _: &OnlineJournal) -> Result<(), ()> {
        Ok(())
    }
}
fn fixture() -> (
    tempfile::TempDir,
    InstallLayout,
    OnlineJournal,
    MaintenanceJob,
) {
    let temporary = tempfile::tempdir().unwrap();
    let layout = InstallLayout::new(temporary.path()).unwrap();
    let mut plan = crate::online_journal::tests::plan();
    plan.previous.version = "2.0.1".into();
    plan.previous_process.product_version = "2.0.1".into();
    let journal = OnlineJournal::create(plan, &mut Store).unwrap();
    let plan = journal.plan();
    for directory in [
        layout.upgrade_running(),
        layout.upgrade_completed(),
        layout.upgrade_uploads(),
        layout.release("2.0.1"),
        layout.release("2.1.0"),
    ] {
        fs::create_dir_all(directory).unwrap();
    }
    let job = MaintenanceJob {
        schema: MAINTENANCE_JOB_SCHEMA.into(),
        id: plan.job_id.clone(),
        requested_by: "owner-one".into(),
        operation: MaintenanceOperation::Upgrade {
            archive: layout
                .upgrade_uploads()
                .join(format!("{}.tar.gz", plan.job_id)),
            archive_sha256: "a".repeat(64),
        },
        status: MaintenanceStatus::StartingCandidate,
        upgrade_mode: Some(UpgradeMode::BlueGreen),
        runner_was_running: Some(true),
        current_version: plan.previous.version.clone(),
        target_version: Some(plan.candidate.version.clone()),
        previous_release: Some(layout.release(&plan.previous.version)),
        candidate_release: Some(layout.release(&plan.candidate.version)),
        message: "preparing".into(),
        created_at: "2026-09-09T00:00:00Z".into(),
        updated_at: "2026-09-09T00:00:00Z".into(),
    };
    write_json(&running_path(&layout, &job.id), &job);
    for version in ["2.0.1", "2.1.0"] {
        fs::write(layout.release(version).join("RELEASE.json"), version).unwrap();
    }
    fs::create_dir_all(layout.active_slot().parent().unwrap()).unwrap();
    fs::create_dir_all(layout.selected_release().parent().unwrap()).unwrap();
    write_json(&layout.active_slot(), &plan.previous);
    symlink(layout.release("2.0.1"), layout.current()).unwrap();
    write_selection(&layout, "2.0.1");
    (temporary, layout, journal, job)
}
fn write_json(path: &Path, value: &impl Serialize) {
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o640)).unwrap();
}
fn write_selection(layout: &InstallLayout, version: &str) {
    write_json(
        &layout.selected_release(),
        &SelectedRelease {
            schema: crate::SELECTED_RELEASE_SCHEMA.into(),
            version: version.into(),
            release_root: layout.release(version).canonicalize().unwrap(),
            manifest_sha256: crate::sha256_file(&layout.release(version).join("RELEASE.json"))
                .unwrap(),
        },
    );
}

#[test]
fn recovery_accepts_only_the_matching_durable_job() {
    let (_temporary, layout, journal, original) = fixture();
    assert_eq!(load_job(&layout, &journal, false).unwrap(), original);
    for fault in 0..7 {
        let mut job = original.clone();
        match fault {
            0 => job.id = "other-job".into(),
            1 => job.upgrade_mode = Some(UpgradeMode::Maintenance),
            2 => job.target_version = Some("9.0.0".into()),
            3 => job.previous_release = Some(PathBuf::from("/unrelated")),
            4 => {
                job.operation = MaintenanceOperation::DeleteVersion {
                    version: "2.0.1".into(),
                }
            }
            5 => job.status = MaintenanceStatus::Failed,
            _ => {
                job.operation = MaintenanceOperation::Upgrade {
                    archive: layout.upgrade_uploads().join("different.tar.gz"),
                    archive_sha256: "a".repeat(64),
                }
            }
        }
        write_json(&running_path(&layout, &original.id), &job);
        assert!(load_job(&layout, &journal, false).is_err());
    }
}

#[test]
fn pointer_reconciliation_accepts_partial_commit_but_rejects_an_unrelated_release() {
    let (_temporary, layout, journal, _job) = fixture();
    current_metadata(&layout, &journal, false).unwrap();
    assert!(current_metadata(&layout, &journal, true).is_err());
    select_current(
        &layout,
        &layout.release("2.1.0"),
        Instant::now() + Duration::from_secs(5),
    )
    .unwrap();
    current_metadata(&layout, &journal, false).unwrap();
    write_selection(&layout, "2.1.0");
    assert!(current_metadata(&layout, &journal, true).is_err());
    write_json(&layout.active_slot(), &journal.plan().candidate);
    current_metadata(&layout, &journal, true).unwrap();
    let mut unrelated = journal.plan().candidate.clone();
    unrelated.version = "9.0.0".into();
    write_json(&layout.active_slot(), &unrelated);
    assert!(current_metadata(&layout, &journal, false).is_err());
}

#[test]
fn archive_collision_and_early_commit_fail_before_any_service_action() {
    let (_temporary, layout, journal, job) = fixture();
    let old = fs::read(layout.active_slot()).unwrap();
    assert!(commit(&layout, &journal, Instant::now() + Duration::from_secs(5)).is_err());
    assert_eq!(fs::read(layout.active_slot()).unwrap(), old);
    write_json(&completed_path(&layout, &job.id), &job);
    assert!(load_job(&layout, &journal, true).is_err());
}

#[test]
fn protected_commit_files_reject_symlinks_and_keep_a_lost_write_retry_idempotent() {
    let (_temporary, layout, _journal, _job) = fixture();
    let path = layout.root().join("commit-probe.json");
    fs::write(&path, b"before").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let original_uid = fs::metadata(&path).unwrap().uid();
    replace(
        &layout,
        &path,
        b"after",
        0o640,
        Instant::now() + Duration::from_secs(5),
    )
    .unwrap();
    replace(
        &layout,
        &path,
        b"after",
        0o640,
        Instant::now() + Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"after");
    assert_eq!(fs::metadata(&path).unwrap().uid(), original_uid);
    let link = layout.root().join("commit-link.json");
    symlink(&path, &link).unwrap();
    assert!(
        replace(
            &layout,
            &link,
            b"wrong",
            0o640,
            Instant::now() + Duration::from_secs(5)
        )
        .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), b"after");
}

#[test]
fn startup_promotion_waits_for_retirement_and_is_durable_for_the_next_restart() {
    let (_temporary, layout, journal, _job) = fixture();
    let slot = journal.plan().candidate.slot;
    let path = layout.control_slot_environment(slot.id());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let before = format!(
        "RUST_LOG=warn\nASTER_CONTROL_RUNTIME_SLOT={}\nASTER_CONTROL_RUNTIME_CANDIDATE=true\n",
        slot.id()
    );
    fs::write(&path, &before).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    assert!(verify_startup(&layout, &journal).is_err());
    assert!(promote_startup(&layout, &journal, Instant::now() + Duration::from_secs(5)).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
    let mut value =
        serde_json::to_value(crate::online_journal::tests::completed(&journal)).unwrap();
    value["phase"] = "committing".into();
    value["revision"] = 5.into();
    let committing: OnlineJournal = serde_json::from_value(value).unwrap();
    assert!(committing.valid());
    for _ in 0..2 {
        promote_startup(
            &layout,
            &committing,
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        verify_startup(&layout, &committing).unwrap();
    }
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        before.replace("CANDIDATE=true", "CANDIDATE=false")
    );
}
