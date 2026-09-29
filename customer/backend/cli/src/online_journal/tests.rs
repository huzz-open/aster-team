use super::*;
use aster_upgrade_core::{
    ACTIVE_SLOT_RUNTIME_SCHEMA, ActiveLocalRunner, ActiveReleaseSlot, ReleaseSlot,
    online::OnlinePlan,
    runtime::{
        LifecycleSnapshot, ProxySnapshot, ReadinessExpectation, RuntimeSnapshot, UpgradeClock,
    },
};

pub(crate) fn plan() -> OnlinePlan {
    let release = |slot, digit: &str| ActiveReleaseSlot {
        schema: ACTIVE_SLOT_RUNTIME_SCHEMA.into(),
        slot,
        version: "2.1.0".into(),
        local_runner: Some(ActiveLocalRunner {
            runner_id: format!("runner_{}", digit.repeat(32)),
            manifest_sha256: digit.repeat(64),
        }),
    };
    let process = |slot: ReleaseSlot| RuntimeSnapshot {
        schema: "aster.control-runtime.v1".into(),
        installation_id: "installation".into(),
        slot,
        instance_id: format!("instance-{}", slot.id()),
        service: Some(aster_upgrade_core::runtime::ServiceInvocation {
            process_id: if slot == ReleaseSlot::Blue { 101 } else { 102 },
            invocation_id: if slot == ReleaseSlot::Blue {
                "a".repeat(32)
            } else {
                "b".repeat(32)
            },
        }),
        product_version: "2.1.0".into(),
        lifecycle: LifecycleSnapshot {
            accepting: slot == ReleaseSlot::Blue,
            stopping: false,
            revision: 0,
            in_flight: 0,
            oldest_request_age_ms: 0,
        },
    };
    OnlinePlan {
        job_id: "upgrade-one".into(),
        previous: release(ReleaseSlot::Blue, "a"),
        candidate: release(ReleaseSlot::Green, "b"),
        previous_process: process(ReleaseSlot::Blue),
        candidate_process: process(ReleaseSlot::Green),
        previous_runner_process: aster_upgrade_core::runtime::ServiceInvocation {
            process_id: 201,
            invocation_id: "c".repeat(32),
        },
        candidate_runner_process: aster_upgrade_core::runtime::ServiceInvocation {
            process_id: 202,
            invocation_id: "d".repeat(32),
        },
        readiness: ReadinessExpectation {
            manifest_sha256: "b".repeat(64),
            models: vec!["model".into()],
            runner_ids: vec![format!("runner_{}", "b".repeat(32))],
        },
        drain_budget_ms: 720_000,
        clock: UpgradeClock {
            boot_id: "test-boot".into(),
            uptime_ms: 1000,
        },
        proxy: ProxySnapshot {
            slot: ReleaseSlot::Blue,
            configuration_sha256: "c".repeat(64),
            stream_close_delay_ms: 900_000,
        },
    }
}

pub(super) fn fixture() -> (tempfile::TempDir, InstallLayout, JournalFile, OnlineJournal) {
    let temporary = tempfile::tempdir().unwrap();
    let layout = InstallLayout::new(temporary.path()).unwrap();
    let mut store = JournalFile::open(&layout).unwrap();
    let journal = OnlineJournal::create(plan(), &mut store).unwrap();
    (temporary, layout, store, journal)
}

pub(super) fn successor(journal: &OnlineJournal) -> OnlineJournal {
    let mut value = serde_json::to_value(journal).unwrap();
    let mut opened = journal.plan().candidate_process.clone();
    opened.lifecycle.accepting = true;
    opened.lifecycle.revision += 1;
    value["opened"] = serde_json::to_value(opened).unwrap();
    value["phase"] = "switching_traffic".into();
    value["revision"] = 1.into();
    value["cutover_started_at_ms"] = 1000.into();
    serde_json::from_value(value).unwrap()
}

#[test]
fn file_journal_roundtrips_and_only_accepts_the_exact_predecessor() {
    let (_temporary, layout, mut store, journal) = fixture();
    assert_eq!(store.load().unwrap().as_ref(), Some(&journal));
    let next = successor(&journal);
    store.save(Some(&journal), &next).unwrap();
    assert!(store.save(Some(&journal), &next).is_err());
    assert!(store.save(None, &journal).is_err());
    assert!(store.assert_current(&journal).is_err());
    store.assert_current(&next).unwrap();
    drop(store);
    let reopened = JournalFile::open(&layout).unwrap();
    assert_eq!(reopened.load().unwrap().as_ref(), Some(&next));
}

#[test]
fn valid_but_rewritten_plan_and_skipped_phase_are_rejected_without_changing_file() {
    let (_temporary, _layout, mut store, journal) = fixture();
    let before = fs::read(&store.path).unwrap();
    let mut value = serde_json::to_value(successor(&journal)).unwrap();
    value["plan"]["job_id"] = "another-upgrade".into();
    let changed: OnlineJournal = serde_json::from_value(value.clone()).unwrap();
    assert!(changed.valid());
    assert!(store.save(Some(&journal), &changed).is_err());
    value["plan"]["job_id"] = journal.plan().job_id.clone().into();
    value["revision"] = 2.into();
    value["phase"] = "closing_previous".into();
    value["drain_started_at_ms"] = 1000.into();
    let skipped: OnlineJournal = serde_json::from_value(value).unwrap();
    assert!(skipped.valid());
    assert!(store.save(Some(&journal), &skipped).is_err());
    assert_eq!(fs::read(&store.path).unwrap(), before);
}

#[test]
fn sidecar_lock_excludes_a_second_store_and_is_released_on_drop() {
    let (_temporary, layout, store, journal) = fixture();
    assert!(JournalFile::open(&layout).is_err());
    drop(store);
    let reopened = JournalFile::open(&layout).unwrap();
    assert_eq!(reopened.load().unwrap().as_ref(), Some(&journal));
}

#[test]
fn corrupt_and_oversized_records_block_recovery_without_erasing_evidence() {
    let (_temporary, layout, store, _journal) = fixture();
    let path = store.path.clone();
    drop(store);
    for bytes in [b"{".as_slice(), b"{}".as_slice(), b"[]".as_slice()] {
        fs::write(&path, bytes).unwrap();
        assert!(ensure_maintenance_allowed(&layout).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    let file = OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(MAX_JOURNAL_BYTES + 1).unwrap();
    drop(file);
    assert!(ensure_maintenance_allowed(&layout).is_err());
    assert_eq!(fs::metadata(&path).unwrap().len(), MAX_JOURNAL_BYTES + 1);
}

#[test]
fn missing_journal_keeps_legacy_maintenance_available_without_creating_online_state() {
    let temporary = tempfile::tempdir().unwrap();
    let layout = InstallLayout::new(temporary.path()).unwrap();
    ensure_maintenance_allowed(&layout).unwrap();
    assert!(!layout.upgrade_state().exists());
}

#[cfg(unix)]
#[test]
fn linked_or_shared_permission_journal_is_not_followed_or_repaired() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    let (_temporary, layout, store, _journal) = fixture();
    let path = store.path.clone();
    drop(store);
    let outside = tempfile::tempdir().unwrap();
    let outside_file = outside.path().join("record");
    fs::write(&outside_file, b"outside evidence").unwrap();
    fs::remove_file(&path).unwrap();
    symlink(&outside_file, &path).unwrap();
    assert!(ensure_maintenance_allowed(&layout).is_err());
    assert_eq!(fs::read(&outside_file).unwrap(), b"outside evidence");
    fs::remove_file(&path).unwrap();
    fs::write(&path, b"{}").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(JournalFile::open(&layout).unwrap().load().is_err());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o644
    );
}

pub(crate) fn completed(journal: &OnlineJournal) -> OnlineJournal {
    let mut value = serde_json::to_value(journal).unwrap();
    let mut opened = journal.plan().candidate_process.clone();
    opened.lifecycle.accepting = true;
    opened.lifecycle.revision += 1;
    let mut closed = journal.plan().previous_process.clone();
    closed.lifecycle.accepting = false;
    closed.lifecycle.revision += 1;
    value["opened"] = serde_json::to_value(opened).unwrap();
    value["closed"] = serde_json::to_value(&closed).unwrap();
    value["drained"] = serde_json::to_value(closed).unwrap();
    value["cutover_started_at_ms"] = 1001.into();
    value["drain_started_at_ms"] = 1002.into();
    value["phase"] = "complete".into();
    value["revision"] = 6.into();
    let completed: OnlineJournal = serde_json::from_value(value).unwrap();
    assert!(completed.valid());
    completed
}

#[test]
fn archival_requires_complete_evidence_and_never_overwrites_history() {
    let (_temporary, layout, mut store, journal) = fixture();
    assert!(JournalFile::pending(&layout).unwrap());
    assert!(store.archive(&journal).is_err());
    assert_eq!(store.load().unwrap(), Some(journal.clone()));
    let completed = completed(&journal);
    fs::write(&store.path, serde_json::to_vec(&completed).unwrap()).unwrap();
    let history = layout.root().join("state/upgrades/online-completed");
    fs::create_dir_all(&history).unwrap();
    let destination = history.join(format!("{}.json", journal.plan().job_id));
    fs::write(&destination, b"existing history").unwrap();
    assert!(store.archive(&completed).is_err());
    assert_eq!(fs::read(&destination).unwrap(), b"existing history");
    assert_eq!(store.load().unwrap(), Some(completed));
}

#[test]
fn completed_online_history_is_separate_from_maintenance_job_records() {
    let (_temporary, layout, mut store, journal) = fixture();
    let completed = completed(&journal);
    let bytes = serde_json::to_vec(&completed).unwrap();
    fs::write(&store.path, &bytes).unwrap();
    store.archive(&completed).unwrap();
    assert!(!JournalFile::pending(&layout).unwrap());
    assert!(store.load().unwrap().is_none());
    let archived = layout
        .root()
        .join("state/upgrades/online-completed")
        .join(format!("{}.json", journal.plan().job_id));
    assert_eq!(fs::read(archived).unwrap(), bytes);
    assert!(
        !layout
            .upgrade_completed()
            .join(format!("{}.json", journal.plan().job_id))
            .exists()
    );
}

#[cfg(unix)]
#[test]
fn sidecar_owner_release_does_not_wait_for_inherited_descriptor_close() {
    let (_temporary, layout, store, journal) = fixture();
    // dup has the same open-file-description ownership as a descriptor briefly
    // inherited by a concurrent fork before close-on-exec takes effect.
    let inherited = store._lock.try_clone().unwrap();
    assert!(JournalFile::open(&layout).is_err());
    drop(store);
    let reopened = JournalFile::open(&layout).unwrap();
    assert_eq!(reopened.load().unwrap(), Some(journal));
    drop(inherited);
    assert!(JournalFile::open(&layout).is_err());
    drop(reopened);
    assert!(JournalFile::open(&layout).is_ok());
}
