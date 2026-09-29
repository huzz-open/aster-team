use super::*;

fn fixture() -> (
    tempfile::TempDir,
    InstallLayout,
    JournalFile,
    PreparationJournal,
) {
    let (directory, layout, mut store, journal, _) = crate::online_journal::preparation_fixture();
    let previous = layout.release(&journal.intent().previous.version);
    let candidate = journal.intent().job.candidate_release.as_ref().unwrap();
    fs::create_dir_all(&previous).unwrap();
    fs::create_dir_all(candidate).unwrap();
    for path in [
        layout.slot_release("blue"),
        layout.slot_release("green"),
        layout.active_slot(),
    ] {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
    }
    symlink(&previous, layout.current()).unwrap();
    symlink(
        &previous,
        layout.slot_release(journal.intent().previous.slot.id()),
    )
    .unwrap();
    fs::write(
        layout.active_slot(),
        serde_json::to_vec(&journal.intent().previous).unwrap(),
    )
    .unwrap();
    store.save_preparation(None, &journal).unwrap();
    (directory, layout, store, journal)
}

#[test]
fn selection_and_retry_change_only_the_candidate_after_durable_intent() {
    let (_directory, layout, store, journal) = fixture();
    let before = fs::read(layout.active_slot()).unwrap();
    let candidate = journal.intent().job.candidate_release.as_ref().unwrap();
    let link = layout.slot_release(journal.intent().previous.slot.other().id());
    let previous = layout.release(&journal.intent().previous.version);
    // An inactive slot may still point at the previous release.
    symlink(&previous, &link).unwrap();
    stage(&layout, &store, &journal).unwrap();
    assert!(points_to(&link, candidate).unwrap());
    drop(store);
    let store = JournalFile::open(&layout).unwrap();
    stage(&layout, &store, &journal).unwrap();
    assert!(points_to(&layout.current(), &previous).unwrap());
    assert!(
        points_to(
            &layout.slot_release(journal.intent().previous.slot.id()),
            &previous
        )
        .unwrap()
    );
    assert_eq!(fs::read(layout.active_slot()).unwrap(), before);
    assert_eq!(store.load_preparation().unwrap().as_ref(), Some(&journal));
    assert!(store.load().unwrap().is_none());
}

#[test]
fn crash_before_rename_resumes_the_same_temporary_pointer() {
    let (_directory, layout, store, journal) = fixture();
    let link = layout.slot_release(journal.intent().previous.slot.other().id());
    let temporary = link
        .parent()
        .unwrap()
        .join(format!(".online-{}.link", journal.intent().job.id));
    symlink(
        journal.intent().job.candidate_release.as_ref().unwrap(),
        &temporary,
    )
    .unwrap();
    stage(&layout, &store, &journal).unwrap();
    assert!(fs::symlink_metadata(temporary).is_err());
    assert!(
        points_to(
            &link,
            journal.intent().job.candidate_release.as_ref().unwrap()
        )
        .unwrap()
    );
}

#[test]
fn collisions_and_changed_active_pointer_preserve_the_candidate_and_evidence() {
    for scenario in 0..4 {
        let (_directory, layout, store, journal) = fixture();
        let link = layout.slot_release(journal.intent().previous.slot.other().id());
        let temporary = link
            .parent()
            .unwrap()
            .join(format!(".online-{}.link", journal.intent().job.id));
        match scenario {
            0 => fs::write(&temporary, "unrelated").unwrap(),
            1 => symlink(
                layout.release(&journal.intent().previous.version),
                &temporary,
            )
            .unwrap(),
            2 => fs::write(&link, "unrelated").unwrap(),
            _ => {
                fs::remove_file(layout.current()).unwrap();
                symlink(
                    journal.intent().job.candidate_release.as_ref().unwrap(),
                    layout.current(),
                )
                .unwrap();
            }
        }
        assert!(stage(&layout, &store, &journal).is_err());
        assert_eq!(store.load_preparation().unwrap().as_ref(), Some(&journal));
        if scenario == 2 {
            assert_eq!(fs::read(&link).unwrap(), b"unrelated");
        } else {
            assert!(fs::symlink_metadata(&link).is_err());
        }
        if scenario == 0 {
            assert_eq!(fs::read(&temporary).unwrap(), b"unrelated");
        }
    }
}

#[test]
fn missing_intent_and_started_candidate_cannot_rewrite_a_pointer() {
    let (_directory, layout, mut store, journal) = fixture();
    let path = layout.upgrade_state().join("online-preparation.json");
    fs::remove_file(&path).unwrap();
    assert!(stage(&layout, &store, &journal).is_err());
    store.save_preparation(None, &journal).unwrap();
    let (_, _, _, _, plan) = crate::online_journal::preparation_fixture();
    let provisioned = journal.provisioned(plan.candidate).unwrap();
    store
        .save_preparation(Some(&journal), &provisioned)
        .unwrap();
    let starting = provisioned
        .starting(CandidateStartup {
            material_sha256: "d".repeat(64),
            clock: journal.intent().clock.clone(),
        })
        .unwrap();
    store
        .save_preparation(Some(&provisioned), &starting)
        .unwrap();
    assert!(stage(&layout, &store, &starting).is_err());
    assert!(
        fs::symlink_metadata(layout.slot_release(journal.intent().previous.slot.other().id()))
            .is_err()
    );
}
