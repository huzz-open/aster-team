use super::*;
use aster_upgrade_core::{
    MAINTENANCE_JOB_SCHEMA, MaintenanceJob, MaintenanceOperation, MaintenanceStatus, UpgradeMode,
    online::OnlinePlan,
    preparation::{CandidateActivation, CandidateStartup, PreparationIntent},
};

pub(crate) fn fixture() -> (
    tempfile::TempDir,
    InstallLayout,
    JournalFile,
    PreparationJournal,
    OnlinePlan,
) {
    let temporary = tempfile::tempdir().unwrap();
    let layout = InstallLayout::new(temporary.path()).unwrap();
    let store = JournalFile::open(&layout).unwrap();
    let mut plan = super::super::tests::plan();
    plan.previous.version = "2.0.1".into();
    plan.previous_process.product_version = "2.0.1".into();
    let current_migrations = aster_upgrade_core::migration::MigrationInspection {
        schema: "aster.migration-inspection.v2".into(),
        applied_schema_version: 4,
        candidate_schema_version: 4,
        applied_history_sha256: "a".repeat(64),
        candidate_history_sha256: "a".repeat(64),
        pending: vec![],
    };
    let intent = PreparationIntent {
        migrations: aster_upgrade_core::migration::MigrationPreflight {
            current: current_migrations.clone(),
            candidate: current_migrations,
        },
        job: MaintenanceJob {
            schema: MAINTENANCE_JOB_SCHEMA.into(),
            id: plan.job_id.clone(),
            requested_by: "admin".into(),
            operation: MaintenanceOperation::Upgrade {
                archive: layout
                    .upgrade_uploads()
                    .join(format!("{}.tar.gz", plan.job_id)),
                archive_sha256: "e".repeat(64),
            },
            status: MaintenanceStatus::StartingCandidate,
            upgrade_mode: Some(UpgradeMode::BlueGreen),
            runner_was_running: Some(true),
            current_version: plan.previous.version.clone(),
            target_version: Some(plan.candidate.version.clone()),
            previous_release: Some(layout.release(&plan.previous.version)),
            candidate_release: Some(layout.release(&plan.candidate.version)),
            message: "preparing".into(),
            created_at: "2026-09-09".into(),
            updated_at: "2026-09-09".into(),
        },
        previous: plan.previous.clone(),
        previous_process: plan.previous_process.clone(),
        previous_runner_process: plan.previous_runner_process.clone(),
        candidate_manifest_sha256: plan.readiness.manifest_sha256.clone(),
        proxy: plan.proxy.clone(),
        clock: plan.clock.clone(),
    };
    (
        temporary,
        layout,
        store,
        PreparationJournal::create(intent).unwrap(),
        plan,
    )
}

fn persist_activation(
    store: &mut JournalFile,
    record: PreparationJournal,
    plan: &OnlinePlan,
) -> PreparationJournal {
    let starting = record
        .starting(CandidateStartup {
            material_sha256: "d".repeat(64),
            clock: plan.clock.clone(),
        })
        .unwrap();
    store.save_preparation(Some(&record), &starting).unwrap();
    assert!(OnlineJournal::create(plan.clone(), store).is_err());
    let started = starting
        .started(
            CandidateActivation {
                control: plan.candidate_process.clone(),
                runner: plan.candidate_runner_process.clone(),
            },
            &"d".repeat(64),
            &plan.clock,
        )
        .unwrap();
    store.save_preparation(Some(&starting), &started).unwrap();
    started
}

#[test]
fn durable_preparation_survives_restart_and_rejects_stale_writers() {
    let (_temporary, layout, mut store, initial, plan) = fixture();
    store.save_preparation(None, &initial).unwrap();
    let ready = initial.provisioned(plan.candidate).unwrap();
    store.save_preparation(Some(&initial), &ready).unwrap();
    let before = fs::read(layout.upgrade_state().join(NAME)).unwrap();
    assert!(store.save_preparation(Some(&initial), &ready).is_err());
    assert!(store.save_preparation(None, &initial).is_err());
    assert_eq!(fs::read(layout.upgrade_state().join(NAME)).unwrap(), before);
    drop(store);
    let mut reopened = JournalFile::open(&layout).unwrap();
    assert_eq!(reopened.load_preparation().unwrap(), Some(ready.clone()));
    reopened.save_preparation(Some(&ready), &ready).unwrap();
}

#[test]
fn candidate_identity_must_be_durable_before_online_creation_and_archive() {
    let (_temporary, layout, mut store, initial, plan) = fixture();
    store.save_preparation(None, &initial).unwrap();
    assert!(OnlineJournal::create(plan.clone(), &mut store).is_err());
    assert!(store.load().unwrap().is_none());
    let ready = initial.provisioned(plan.candidate.clone()).unwrap();
    store.save_preparation(Some(&initial), &ready).unwrap();
    assert!(OnlineJournal::create(plan.clone(), &mut store).is_err());
    let ready = persist_activation(&mut store, ready, &plan);
    let mut wrong = plan.clone();
    wrong.job_id = "other-job".into();
    assert!(OnlineJournal::create(wrong, &mut store).is_err());
    let journal = OnlineJournal::create(plan, &mut store).unwrap();
    assert!(store.save_preparation(Some(&ready), &ready).is_err());
    store.archive_preparation(&journal).unwrap();
    assert!(store.load_preparation().unwrap().is_none());
    store.archive_preparation(&journal).unwrap();
    let archived: PreparationJournal = serde_json::from_slice(
        &fs::read(
            layout
                .upgrade_state()
                .join("online-prepared")
                .join(format!("{}.json", journal.plan().job_id)),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(archived, ready);
    assert_eq!(store.load().unwrap(), Some(journal));
}

#[test]
fn preparation_blocks_maintenance_and_corrupt_records_remain_untouched() {
    let (_temporary, layout, mut store, initial, _) = fixture();
    store.save_preparation(None, &initial).unwrap();
    drop(store);
    assert!(ensure_maintenance_allowed(&layout).is_err());
    let path = layout.upgrade_state().join(NAME);
    for bytes in [b"{".as_slice(), b"{}".as_slice()] {
        fs::write(&path, bytes).unwrap();
        assert!(ensure_maintenance_allowed(&layout).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    let file = OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(MAX_JOURNAL_BYTES + 1).unwrap();
    drop(file);
    assert!(ensure_maintenance_allowed(&layout).is_err());
}

#[test]
fn preparation_archive_never_overwrites_history() {
    let (_temporary, layout, mut store, initial, plan) = fixture();
    store.save_preparation(None, &initial).unwrap();
    let ready = initial.provisioned(plan.candidate.clone()).unwrap();
    store.save_preparation(Some(&initial), &ready).unwrap();
    assert!(OnlineJournal::create(plan.clone(), &mut store).is_err());
    let ready = persist_activation(&mut store, ready, &plan);
    let journal = OnlineJournal::create(plan, &mut store).unwrap();
    let directory = layout.upgrade_state().join("online-prepared");
    fs::create_dir(&directory).unwrap();
    let destination = directory.join(format!("{}.json", journal.plan().job_id));
    fs::write(&destination, b"original history").unwrap();
    assert!(store.archive_preparation(&journal).is_err());
    assert_eq!(fs::read(destination).unwrap(), b"original history");
    assert_eq!(store.load_preparation().unwrap(), Some(ready));
}

#[cfg(unix)]
#[test]
fn preparation_rejects_shared_permissions_and_links_without_repairing_them() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    let (_temporary, layout, mut store, initial, _) = fixture();
    store.save_preparation(None, &initial).unwrap();
    let path = layout.upgrade_state().join(NAME);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(store.load_preparation().is_err());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o644
    );
    fs::remove_file(&path).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_file = outside.path().join("untouched");
    fs::write(&outside_file, b"outside").unwrap();
    symlink(&outside_file, &path).unwrap();
    assert!(store.save_preparation(None, &initial).is_err());
    assert_eq!(fs::read(outside_file).unwrap(), b"outside");
}

#[test]
fn startup_intent_and_activation_survive_reopen_without_resetting_ownership() {
    let (_temporary, layout, mut store, initial, plan) = fixture();
    store.save_preparation(None, &initial).unwrap();
    let provisioned = initial.provisioned(plan.candidate.clone()).unwrap();
    store
        .save_preparation(Some(&initial), &provisioned)
        .unwrap();
    let starting = provisioned
        .starting(CandidateStartup {
            material_sha256: "d".repeat(64),
            clock: plan.clock.clone(),
        })
        .unwrap();
    store
        .save_preparation(Some(&provisioned), &starting)
        .unwrap();
    drop(store);
    let mut store = JournalFile::open(&layout).unwrap();
    assert_eq!(store.load_preparation().unwrap(), Some(starting.clone()));
    let started = starting
        .started(
            CandidateActivation {
                control: plan.candidate_process.clone(),
                runner: plan.candidate_runner_process.clone(),
            },
            &"d".repeat(64),
            &plan.clock,
        )
        .unwrap();
    assert!(
        store
            .save_preparation(Some(&provisioned), &started)
            .is_err()
    );
    store.save_preparation(Some(&starting), &started).unwrap();
    drop(store);
    let mut store = JournalFile::open(&layout).unwrap();
    assert_eq!(store.load_preparation().unwrap(), Some(started.clone()));
    assert!(store.save_preparation(Some(&started), &starting).is_err());
    assert!(store.load().unwrap().is_none());
    OnlineJournal::create(plan, &mut store).unwrap();
}

#[test]
fn cancelled_preparation_survives_reopen_and_rejects_stale_startup_writer() {
    let (_directory, layout, mut store, initial, plan) = fixture();
    store.save_preparation(None, &initial).unwrap();
    let provisioned = initial.provisioned(plan.candidate.clone()).unwrap();
    store
        .save_preparation(Some(&initial), &provisioned)
        .unwrap();
    let starting = provisioned
        .starting(CandidateStartup {
            material_sha256: "d".repeat(64),
            clock: plan.clock.clone(),
        })
        .unwrap();
    let claimed = provisioned.aborting().unwrap();
    store
        .save_preparation(Some(&provisioned), &claimed)
        .unwrap();
    drop(store);
    let mut store = JournalFile::open(&layout).unwrap();
    assert_eq!(store.load_preparation().unwrap(), Some(claimed.clone()));
    let path = layout.upgrade_state().join(NAME);
    let original_bytes = fs::read(&path).unwrap();
    assert!(
        store
            .save_preparation(Some(&provisioned), &starting)
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), original_bytes);
    let finalizing = claimed.advance_abort().unwrap();
    store.save_preparation(Some(&claimed), &finalizing).unwrap();
    let complete = finalizing.advance_abort().unwrap();
    store
        .save_preparation(Some(&finalizing), &complete)
        .unwrap();
    assert!(ensure_maintenance_allowed(&layout).is_err());
}

#[test]
fn reverse_guard_blocks_preparation_even_after_frozen_forward_file_was_removed() {
    let (_directory, layout, mut store, initial, plan) = fixture();
    store.save_preparation(None, &initial).unwrap();
    let reverse = layout.upgrade_state().join("online-switchback.json");
    fs::write(&reverse, b"{").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(reverse, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let next = initial.provisioned(plan.candidate).unwrap();
    assert!(store.save_preparation(Some(&initial), &next).is_err());
    assert_eq!(store.load_preparation().unwrap(), Some(initial));
}

#[test]
fn cancelled_history_survives_retry_and_conflicts_preserve_the_guard() {
    for conflict in [false, true] {
        let (_directory, layout, mut store, initial, plan) = fixture();
        store.save_preparation(None, &initial).unwrap();
        let provisioned = initial.provisioned(plan.candidate).unwrap();
        store
            .save_preparation(Some(&initial), &provisioned)
            .unwrap();
        let mut record = provisioned.aborting().unwrap();
        store.save_preparation(Some(&provisioned), &record).unwrap();
        assert!(store.archive_cancelled_preparation(&record).is_err());
        for _ in 0..2 {
            let next = record.advance_abort().unwrap();
            store.save_preparation(Some(&record), &next).unwrap();
            record = next;
        }
        let history = layout
            .upgrade_state()
            .join("preparation-cancelled")
            .join(format!("{}.json", record.intent().job.id));
        fs::create_dir_all(history.parent().unwrap()).unwrap();
        fs::write(
            &history,
            serde_json::to_vec(if conflict { &initial } else { &record }).unwrap(),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&history, fs::Permissions::from_mode(0o600)).unwrap();
        }
        drop(store);
        let mut store = JournalFile::open(&layout).unwrap();
        assert_eq!(
            store.archive_cancelled_preparation(&record).is_ok(),
            !conflict
        );
        assert_eq!(store.load_preparation().unwrap().is_some(), conflict);
        if !conflict {
            assert_eq!(store.read_preparation(&history).unwrap(), Some(record));
        }
    }
}

#[test]
fn activated_cancellation_reopens_without_allowing_handoff_or_skipped_retirement() {
    let (_directory, layout, mut store, initial, plan) = fixture();
    store.save_preparation(None, &initial).unwrap();
    let provisioned = initial.provisioned(plan.candidate.clone()).unwrap();
    store
        .save_preparation(Some(&initial), &provisioned)
        .unwrap();
    let started = persist_activation(&mut store, provisioned, &plan);
    let claimed = started.aborting().unwrap();
    store.save_preparation(Some(&started), &claimed).unwrap();
    let retiring = claimed.retiring_candidate(&plan.candidate_process).unwrap();
    let finalizing = retiring.advance_abort().unwrap();
    assert!(store.save_preparation(Some(&claimed), &finalizing).is_err());
    assert!(OnlineJournal::create(plan.clone(), &mut store).is_err());
    store.save_preparation(Some(&claimed), &retiring).unwrap();
    drop(store);
    let mut store = JournalFile::open(&layout).unwrap();
    assert_eq!(store.load_preparation().unwrap(), Some(retiring.clone()));
    assert!(store.save_preparation(Some(&claimed), &retiring).is_err());
    assert!(OnlineJournal::create(plan, &mut store).is_err());
    store
        .save_preparation(Some(&retiring), &finalizing)
        .unwrap();
    let complete = finalizing.advance_abort().unwrap();
    store
        .save_preparation(Some(&finalizing), &complete)
        .unwrap();
    store.archive_cancelled_preparation(&complete).unwrap();
    assert!(store.load_preparation().unwrap().is_none());
    let path = layout
        .upgrade_state()
        .join("preparation-cancelled")
        .join(format!("{}.json", complete.intent().job.id));
    assert_eq!(store.read_preparation(&path).unwrap(), Some(complete));
}

#[test]
fn late_cancellation_capture_is_durable_and_cannot_become_normal_activation() {
    let (_directory, layout, mut store, initial, plan) = fixture();
    store.save_preparation(None, &initial).unwrap();
    let provisioned = initial.provisioned(plan.candidate.clone()).unwrap();
    store
        .save_preparation(Some(&initial), &provisioned)
        .unwrap();
    let starting = provisioned
        .starting(CandidateStartup {
            material_sha256: "d".repeat(64),
            clock: plan.clock.clone(),
        })
        .unwrap();
    store
        .save_preparation(Some(&provisioned), &starting)
        .unwrap();
    let activation = CandidateActivation {
        control: plan.candidate_process.clone(),
        runner: plan.candidate_runner_process.clone(),
    };
    let normal = starting
        .started(activation.clone(), &"d".repeat(64), &plan.clock)
        .unwrap();
    let mut late = plan.clock.clone();
    late.uptime_ms += CandidateStartup::BUDGET_MS + 1;
    let captured = starting
        .cancel_observed_startup(activation, &"d".repeat(64), &late)
        .unwrap();
    store.save_preparation(Some(&starting), &captured).unwrap();
    drop(store);
    let mut store = JournalFile::open(&layout).unwrap();
    assert_eq!(store.load_preparation().unwrap(), Some(captured.clone()));
    assert!(store.save_preparation(Some(&captured), &normal).is_err());
    assert!(store.save_preparation(Some(&starting), &normal).is_err());
    assert!(OnlineJournal::create(plan, &mut store).is_err());
    let claimed = captured.advance_abort().unwrap();
    store.save_preparation(Some(&captured), &claimed).unwrap();
    assert_eq!(store.load_preparation().unwrap(), Some(claimed));
}

#[test]
fn unprovisioned_cancellation_reopens_and_cannot_accept_late_identity_or_skip_archival_guard() {
    let (_directory, layout, mut store, initial, plan) = fixture();
    store.save_preparation(None, &initial).unwrap();
    let provisioned = initial.provisioned(plan.candidate.clone()).unwrap();
    let claimed = initial.aborting().unwrap();
    store.save_preparation(Some(&initial), &claimed).unwrap();
    drop(store);
    let mut store = JournalFile::open(&layout).unwrap();
    assert_eq!(store.load_preparation().unwrap(), Some(claimed.clone()));
    assert!(
        store
            .save_preparation(Some(&initial), &provisioned)
            .is_err()
    );
    assert!(
        store
            .save_preparation(Some(&claimed), &provisioned.aborting().unwrap())
            .is_err()
    );
    assert!(OnlineJournal::create(plan, &mut store).is_err());
    assert!(store.archive_cancelled_preparation(&claimed).is_err());
    let finalizing = claimed.advance_abort().unwrap();
    let complete = finalizing.advance_abort().unwrap();
    assert!(store.save_preparation(Some(&claimed), &complete).is_err());
    store.save_preparation(Some(&claimed), &finalizing).unwrap();
    store
        .save_preparation(Some(&finalizing), &complete)
        .unwrap();
    // Adapter must already have reconciled signed ownership and archived task.
    store.archive_cancelled_preparation(&complete).unwrap();
    let history = layout
        .upgrade_state()
        .join("preparation-cancelled")
        .join(format!("{}.json", complete.intent().job.id));
    let restored = store.read_preparation(&history).unwrap().unwrap();
    assert_eq!(restored, complete);
    assert!(
        restored.candidate().is_none(),
        "never fabricate a provisioned identity in history"
    );
    assert!(store.load_preparation().unwrap().is_none());
}
