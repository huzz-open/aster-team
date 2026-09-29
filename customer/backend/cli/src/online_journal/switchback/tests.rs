use super::*;
use aster_upgrade_core::online::switchback::{SwitchbackPhase, SwitchbackPlan};
use aster_upgrade_core::runtime::{RuntimeRequestBudget, UpgradeClock};

fn fixture() -> (
    tempfile::TempDir,
    InstallLayout,
    JournalFile,
    SwitchbackPlan,
) {
    let (directory, layout, mut store, initial) = super::super::tests::fixture();
    let original = super::super::tests::successor(&initial);
    OnlineJournalStorage::save(&mut store, Some(&initial), &original).unwrap();
    let mut candidate = original.plan().candidate_process.clone();
    candidate.lifecycle.accepting = true;
    candidate.lifecycle.revision += 1;
    candidate.lifecycle.in_flight = 3;
    let mut proxy = original.plan().proxy.clone();
    proxy.slot = candidate.slot;
    let plan = SwitchbackPlan {
        previous: original.plan().previous_process.clone(),
        original,
        candidate: candidate.clone(),
        candidate_budget: RuntimeRequestBudget {
            runtime: candidate,
            request_budget_ms: 600_000,
        },
        proxy,
        clock: UpgradeClock {
            boot_id: "test-boot".into(),
            uptime_ms: 2_000,
        },
    };
    assert!(plan.valid());
    (directory, layout, store, plan)
}

fn next(journal: &SwitchbackJournal) -> SwitchbackJournal {
    let mut value = serde_json::to_value(journal).unwrap();
    value["revision"] = 1.into();
    value["phase"] = "switching_back".into();
    value["opened"] = serde_json::to_value(&journal.plan().previous).unwrap();
    value["switch_started_ms"] = 2_000.into();
    let next: SwitchbackJournal = serde_json::from_value(value).unwrap();
    assert!(next.follows(Some(journal)));
    next
}

#[test]
fn reverse_claim_survives_reopen_and_prevents_forward_or_maintenance_work() {
    let (_directory, layout, mut store, plan) = fixture();
    let original = plan.original.clone();
    let original_bytes = fs::read(&store.path).unwrap();
    let journal = SwitchbackJournal::create(plan, &mut store).unwrap();
    assert_eq!(
        SwitchbackJournalStorage::load(&store).unwrap().as_ref(),
        Some(&journal)
    );
    assert!(OnlineJournalStorage::assert_current(&mut store, &original).is_err());
    assert!(OnlineJournalStorage::save(&mut store, None, &original).is_err());
    assert_eq!(fs::read(&store.path).unwrap(), original_bytes);
    assert!(JournalFile::open(&layout).is_err());
    drop(store);
    assert!(ensure_maintenance_allowed(&layout).is_err());
    assert!(JournalFile::pending(&layout).unwrap());
    let mut reopened = JournalFile::open(&layout).unwrap();
    SwitchbackJournalStorage::assert_current(&mut reopened, &journal).unwrap();
    assert!(OnlineJournalStorage::assert_current(&mut reopened, &original).is_err());
    assert_eq!(fs::read(&reopened.path).unwrap(), original_bytes);
}

#[test]
fn reverse_file_cas_rejects_old_writer_changed_original_and_skipped_phase() {
    let (_directory, _layout, mut store, plan) = fixture();
    let journal = SwitchbackJournal::create(plan, &mut store).unwrap();
    let next = next(&journal);
    SwitchbackJournalStorage::save(&mut store, Some(&journal), &next).unwrap();
    let before = fs::read(store.root.join("state/upgrades").join(NAME)).unwrap();
    assert!(SwitchbackJournalStorage::save(&mut store, Some(&journal), &next).is_err());
    assert!(SwitchbackJournalStorage::save(&mut store, None, &journal).is_err());
    assert!(SwitchbackJournalStorage::assert_current(&mut store, &journal).is_err());
    let mut changed = serde_json::to_value(&next).unwrap();
    changed["plan"]["original"]["plan"]["job_id"] = "another-job".into();
    let changed: SwitchbackJournal = serde_json::from_value(changed).unwrap();
    assert!(changed.valid());
    assert!(SwitchbackJournalStorage::save(&mut store, Some(&next), &changed).is_err());
    assert_eq!(
        fs::read(store.root.join("state/upgrades").join(NAME)).unwrap(),
        before
    );
    assert_eq!(next.phase(), SwitchbackPhase::SwitchingBack);
}

#[test]
fn corrupt_or_oversized_reverse_claim_never_unfreezes_the_original() {
    for bytes in [
        b"{".to_vec(),
        b"{}".to_vec(),
        vec![b' '; MAX_JOURNAL_BYTES as usize + 1],
    ] {
        let (_directory, layout, mut store, plan) = fixture();
        let original = plan.original.clone();
        SwitchbackJournal::create(plan, &mut store).unwrap();
        let path = layout.upgrade_state().join(NAME);
        fs::write(&path, &bytes).unwrap();
        assert!(SwitchbackJournalStorage::load(&store).is_err());
        assert!(OnlineJournalStorage::assert_current(&mut store, &original).is_err());
        assert!(OnlineJournalStorage::save(&mut store, None, &original).is_err());
        drop(store);
        assert!(ensure_maintenance_allowed(&layout).is_err());
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn missing_original_keeps_reverse_evidence_and_blocks_maintenance() {
    let (_directory, layout, mut store, plan) = fixture();
    let journal = SwitchbackJournal::create(plan, &mut store).unwrap();
    let path = layout.upgrade_state().join(NAME);
    let before = fs::read(&path).unwrap();
    fs::remove_file(&store.path).unwrap();
    assert!(SwitchbackJournalStorage::load(&store).is_err());
    assert!(SwitchbackJournalStorage::save(&mut store, Some(&journal), &next(&journal)).is_err());
    drop(store);
    assert!(JournalFile::pending(&layout).unwrap());
    assert!(ensure_maintenance_allowed(&layout).is_err());
    assert_eq!(fs::read(path).unwrap(), before);
}

fn complete(journal: &SwitchbackJournal) -> SwitchbackJournal {
    let mut value = serde_json::to_value(next(journal)).unwrap();
    value["phase"] = "complete".into();
    value["revision"] = 6.into();
    value["drain_started_ms"] = 2_100.into();
    let mut candidate = journal.plan().candidate.clone();
    candidate.lifecycle.accepting = false;
    candidate.lifecycle.revision += 1;
    candidate.lifecycle.in_flight = 0;
    value["closed"] = serde_json::to_value(&candidate).unwrap();
    value["drained"] = serde_json::to_value(candidate).unwrap();
    let complete: SwitchbackJournal = serde_json::from_value(value).unwrap();
    assert!(complete.valid());
    complete
}

fn write_record(path: &Path, journal: &SwitchbackJournal) {
    fs::write(path, serde_json::to_vec(journal).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}

#[test]
fn complete_archive_keeps_full_evidence_and_releases_guard_only_after_original() {
    let (_directory, layout, mut store, plan) = fixture();
    let initial = SwitchbackJournal::create(plan, &mut store).unwrap();
    assert!(store.archive_switchback(&initial).is_err());
    let complete = complete(&initial);
    write_record(&layout.upgrade_state().join(NAME), &complete);
    store.archive_switchback(&complete).unwrap();
    assert!(!JournalFile::pending(&layout).unwrap());
    assert!(OnlineJournalStorage::load(&store).unwrap().is_none());
    assert!(SwitchbackJournalStorage::load(&store).unwrap().is_none());
    assert_eq!(
        store
            .read_switchback(&store.switchback_history(&complete))
            .unwrap(),
        Some(complete)
    );
    ensure_maintenance_allowed(&layout).unwrap();
}

#[test]
fn interrupted_archive_resumes_with_original_present_or_already_removed() {
    for removed in [false, true] {
        let (_directory, layout, mut store, plan) = fixture();
        let initial = SwitchbackJournal::create(plan, &mut store).unwrap();
        let complete = complete(&initial);
        write_record(&layout.upgrade_state().join(NAME), &complete);
        let history = store.switchback_history(&complete);
        fs::create_dir_all(history.parent().unwrap()).unwrap();
        write_record(&history, &complete);
        if removed {
            fs::remove_file(&store.path).unwrap();
        }
        drop(store);
        assert!(JournalFile::pending(&layout).unwrap());
        assert!(ensure_maintenance_allowed(&layout).is_err());
        let mut store = JournalFile::open(&layout).unwrap();
        assert_eq!(
            SwitchbackJournalStorage::load(&store).unwrap(),
            Some(complete.clone())
        );
        store.archive_switchback(&complete).unwrap();
        assert!(!JournalFile::pending(&layout).unwrap());
    }
}

#[test]
fn history_conflicts_and_missing_original_without_exact_complete_history_preserve_guard() {
    for fault in 0..4 {
        let (_directory, layout, mut store, plan) = fixture();
        let initial = SwitchbackJournal::create(plan, &mut store).unwrap();
        let complete = complete(&initial);
        let reverse = layout.upgrade_state().join(NAME);
        write_record(&reverse, &complete);
        let history = store.switchback_history(&complete);
        fs::create_dir_all(history.parent().unwrap()).unwrap();
        match fault {
            0 => write_record(&history, &initial),
            1 => {
                fs::write(&history, b"{").unwrap();
            }
            2 => {
                fs::remove_file(&store.path).unwrap();
            }
            _ => {
                write_record(&history, &initial);
                fs::remove_file(&store.path).unwrap();
            }
        }
        let before = fs::read(&reverse).unwrap();
        assert!(store.archive_switchback(&complete).is_err());
        assert_eq!(fs::read(&reverse).unwrap(), before);
        assert!(JournalFile::pending(&layout).unwrap());
        assert!(ensure_maintenance_allowed(&layout).is_err());
        assert!(store.ensure_forward_owned().is_err());
    }
}
