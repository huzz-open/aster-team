use super::*;
use aster_upgrade_core::preparation::CandidateActivation;

fn ready() -> (
    tempfile::TempDir,
    InstallLayout,
    JournalFile,
    PreparationJournal,
    OnlinePlan,
) {
    let (directory, layout, mut store, initial, plan) =
        crate::online_journal::preparation_fixture();
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
    (directory, layout, store, started, plan)
}

#[test]
fn handoff_is_durable_before_preparation_is_archived_and_survives_reopen() {
    let (_directory, layout, mut store, prepared, plan) = ready();
    let journal = handoff(&mut store, &prepared, plan.clone()).unwrap();
    assert_eq!(journal.plan(), &plan);
    assert_eq!(store.load().unwrap().as_ref(), Some(&journal));
    assert!(store.load_preparation().unwrap().is_none());
    let archived: PreparationJournal = serde_json::from_slice(
        &std::fs::read(
            layout
                .upgrade_state()
                .join("online-prepared")
                .join(format!("{}.json", plan.job_id)),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(archived, prepared);
    drop(store);
    let reopened = JournalFile::open(&layout).unwrap();
    assert_eq!(reopened.load().unwrap().as_ref(), Some(&journal));
}

#[test]
fn archive_collision_preserves_created_transition_and_original_preparation() {
    let (_directory, layout, mut store, prepared, plan) = ready();
    let history = layout.upgrade_state().join("online-prepared");
    std::fs::create_dir_all(&history).unwrap();
    let collision = history.join(format!("{}.json", plan.job_id));
    std::fs::write(&collision, b"existing history").unwrap();
    assert!(handoff(&mut store, &prepared, plan.clone()).is_err());
    let journal = store
        .load()
        .unwrap()
        .expect("transition must survive archival failure");
    assert_eq!(journal.plan(), &plan);
    assert_eq!(store.load_preparation().unwrap().as_ref(), Some(&prepared));
    assert!(handoff(&mut store, &prepared, plan).is_err());
    assert_eq!(store.load().unwrap().as_ref(), Some(&journal));
    assert_eq!(std::fs::read(collision).unwrap(), b"existing history");
}

#[test]
fn changed_plan_or_stale_preparation_never_creates_a_transition() {
    let (_directory, _layout, mut store, prepared, plan) = ready();
    let mut cases = Vec::new();
    let mut changed = plan.clone();
    changed
        .candidate_process
        .instance_id
        .push_str("-replacement");
    cases.push(changed);
    let mut changed = plan.clone();
    changed.previous_runner_process.process_id += 1;
    cases.push(changed);
    let mut changed = plan.clone();
    changed.drain_budget_ms = changed.proxy.stream_close_delay_ms;
    cases.push(changed);
    let mut changed = plan.clone();
    changed.clock.uptime_ms += CandidateStartup::BUDGET_MS;
    cases.push(changed);
    for changed in cases {
        assert!(handoff(&mut store, &prepared, changed).is_err());
        assert!(store.load().unwrap().is_none());
        assert_eq!(store.load_preparation().unwrap().as_ref(), Some(&prepared));
    }
    let stale = PreparationJournal::create(prepared.intent().clone()).unwrap();
    assert!(handoff(&mut store, &stale, plan).is_err());
    assert!(store.load().unwrap().is_none());
}

#[test]
fn handoff_budget_consumes_original_startup_time_and_rejects_reboot_or_expiry() {
    let (_directory, _layout, _store, prepared, plan) = ready();
    let mut clock = plan.clock;
    clock.uptime_ms += CandidateStartup::BUDGET_MS - 1_000;
    let before = Instant::now();
    let deadline = remaining_deadline(&prepared, &clock).unwrap();
    assert!(deadline > before);
    assert!(deadline.duration_since(before) <= Duration::from_millis(1_100));
    clock.uptime_ms += 1_000;
    assert!(remaining_deadline(&prepared, &clock).is_err());
    clock.uptime_ms -= 1_000;
    clock.boot_id.push_str("-reboot");
    assert!(remaining_deadline(&prepared, &clock).is_err());
}
