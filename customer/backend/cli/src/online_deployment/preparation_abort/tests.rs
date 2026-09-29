use super::*;
use std::os::unix::fs::PermissionsExt as _;

fn fixture() -> (
    tempfile::TempDir,
    InstallLayout,
    JournalFile,
    PreparationJournal,
) {
    let (directory, layout, mut store, initial, plan) =
        crate::online_journal::preparation_fixture();
    store.save_preparation(None, &initial).unwrap();
    let record = initial.provisioned(plan.candidate).unwrap();
    store.save_preparation(Some(&initial), &record).unwrap();
    fs::create_dir_all(layout.upgrade_running()).unwrap();
    fs::create_dir_all(layout.upgrade_completed()).unwrap();
    write(
        &layout
            .upgrade_running()
            .join(format!("{}.json", record.intent().job.id)),
        &record.intent().job,
    );
    (directory, layout, store, record)
}
fn write(path: &std::path::Path, task: &MaintenanceJob) {
    fs::write(path, serde_json::to_vec(task).unwrap()).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o640)).unwrap();
}

#[test]
fn cancellation_job_accepts_failed_terminal_only_after_complete_and_retains_identity() {
    let (_directory, layout, _store, record) = fixture();
    let claimed = record.aborting().unwrap();
    let finalizing = claimed.advance_abort().unwrap();
    let complete = finalizing.advance_abort().unwrap();
    let path = layout
        .upgrade_running()
        .join(format!("{}.json", record.intent().job.id));
    for journal in [&claimed, &finalizing, &complete] {
        for status in [
            MaintenanceStatus::StartingCandidate,
            MaintenanceStatus::Failed,
            MaintenanceStatus::Succeeded,
        ] {
            let mut task = record.intent().job.clone();
            task.status = status;
            if status.terminal() {
                task.runner_was_running = None;
            }
            write(&path, &task);
            assert_eq!(
                job(&layout, journal).is_ok(),
                status == MaintenanceStatus::StartingCandidate
                    || (journal.abort_phase() == Some(PreparationAbortPhase::Complete)
                        && status == MaintenanceStatus::Failed)
            );
        }
    }
    let mut task = record.intent().job.clone();
    task.status = MaintenanceStatus::Failed;
    task.runner_was_running = None;
    task.requested_by = "another-owner".into();
    write(&path, &task);
    assert!(job(&layout, &complete).is_err());
}

#[test]
fn only_matching_failed_archive_can_resume_without_a_running_task() {
    let (_directory, layout, _store, record) = fixture();
    let complete = record
        .aborting()
        .unwrap()
        .advance_abort()
        .unwrap()
        .advance_abort()
        .unwrap();
    let mut task = record.intent().job.clone();
    task.status = MaintenanceStatus::Failed;
    task.runner_was_running = None;
    let running = layout.upgrade_running().join(format!("{}.json", task.id));
    let completed = layout.upgrade_completed().join(format!("{}.json", task.id));
    write(&completed, &task);
    assert!(job(&layout, &complete).is_err());
    fs::remove_file(running).unwrap();
    assert_eq!(job(&layout, &complete).unwrap(), task);
    assert!(job(&layout, &record.aborting().unwrap()).is_err());
    task.target_version = Some("9.0.0".into());
    write(&completed, &task);
    assert!(job(&layout, &complete).is_err());
}

#[test]
fn finalized_cancellation_survives_disable_lost_reply_and_rejects_reactivation() {
    use aster_upgrade_core::preparation::{CandidateActivation, CandidateStartup};
    let (_directory, _layout, _store, initial, plan) = crate::online_journal::preparation_fixture();
    let started = initial
        .provisioned(plan.candidate.clone())
        .unwrap()
        .starting(CandidateStartup {
            material_sha256: "d".repeat(64),
            clock: plan.clock.clone(),
        })
        .unwrap()
        .started(
            CandidateActivation {
                control: plan.candidate_process.clone(),
                runner: plan.candidate_runner_process.clone(),
            },
            &"d".repeat(64),
            &plan.clock,
        )
        .unwrap();
    let claimed = started.aborting().unwrap();
    let retiring = claimed.retiring_candidate(&plan.candidate_process).unwrap();
    let finalizing = retiring.advance_abort().unwrap();
    let complete = finalizing.advance_abort().unwrap();
    for record in [&started, &claimed, &retiring] {
        assert!(
            assert_finalized_candidate_inactive(record, |_| panic!("must reject before query"))
                .is_err()
        );
    }
    for record in [&finalizing, &complete] {
        let record: PreparationJournal =
            serde_json::from_slice(&serde_json::to_vec(record).unwrap()).unwrap();
        // The durable phase is unchanged when disable loses its reply. Enabled
        // and disabled inactive units remain reloadable from their templates.
        for enabled in ["enabled", "disabled"] {
            let mut calls = Vec::new();
            assert_finalized_candidate_inactive(&record, |unit| {
                calls.push(unit.to_owned());
                let properties = format!("Id={unit}\nLoadState=loaded\nUnitFileState={enabled}\nActiveState=inactive\nSubState=dead\nType=simple\nRestart=on-failure\nKillMode=control-group\nMainPID=0\nControlPID=0\nControlGroup=\nTasksCurrent=[not set]\nJob=\n");
                crate::control_process::quiescence::classify(&properties, unit)
            })
            .unwrap();
            assert_eq!(
                calls,
                ["aster-control@green.service", "aster-runner@green.service"]
            );
        }
        for bad_unit in ["aster-control@green.service", "aster-runner@green.service"] {
            for state in [
                "active",
                "activating",
                "deactivating",
                "failed",
                "unknown",
                "not-installed",
            ] {
                assert!(
                    assert_finalized_candidate_inactive(&record, |unit| {
                        Ok(if unit == bad_unit { state } else { "inactive" }.into())
                    })
                    .is_err()
                );
            }
            assert!(
                assert_finalized_candidate_inactive(&record, |unit| {
                    if unit == bad_unit {
                        Err(failed("read failed"))
                    } else {
                        Ok("inactive".into())
                    }
                })
                .is_err()
            );
        }
    }
}

#[test]
fn unknown_provisioning_terminal_retry_keeps_original_task_identity() {
    let (_directory, layout, mut store, initial, _plan) =
        crate::online_journal::preparation_fixture();
    store.save_preparation(None, &initial).unwrap();
    fs::create_dir_all(layout.upgrade_running()).unwrap();
    fs::create_dir_all(layout.upgrade_completed()).unwrap();
    let path = layout
        .upgrade_running()
        .join(format!("{}.json", initial.intent().job.id));
    write(&path, &initial.intent().job);
    let claimed = initial.aborting().unwrap();
    assert!(job(&layout, &claimed).is_ok());
    let complete = claimed.advance_abort().unwrap().advance_abort().unwrap();
    let mut task = initial.intent().job.clone();
    task.status = MaintenanceStatus::Failed;
    task.runner_was_running = None;
    write(&path, &task);
    assert!(job(&layout, &claimed).is_err());
    assert_eq!(job(&layout, &complete).unwrap(), task);
    assert!(complete.candidate().is_none());
    task.target_version = Some("9.0.0".into());
    write(&path, &task);
    assert!(job(&layout, &complete).is_err());
    assert!(
        verify_unprovisioned_release(&layout, &initial).is_err(),
        "missing signed release cannot authorize cleanup"
    );
}

#[test]
fn startup_disabling_waits_for_quiescent_checkpoint_even_without_a_published_identity() {
    let (_root, _layout, _store, initial, plan) = crate::online_journal::preparation_fixture();
    for prepared in [
        initial.clone(),
        initial.provisioned(plan.candidate).unwrap(),
    ] {
        let claimed = prepared.aborting().unwrap();
        let finalizing = claimed.advance_abort().unwrap();
        for record in [&prepared, &claimed, &finalizing.advance_abort().unwrap()] {
            assert!(candidate_disable_units(record).is_err());
        }
        assert_eq!(
            candidate_disable_units(&finalizing).unwrap(),
            ["aster-control@green.service", "aster-runner@green.service"]
        );
    }
}
