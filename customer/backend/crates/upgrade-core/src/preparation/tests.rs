use super::*;
use crate::{
    ActiveLocalRunner, MAINTENANCE_JOB_SCHEMA, ReleaseSlot,
    runtime::{LifecycleSnapshot, ReadinessExpectation},
};

fn fixture() -> (PreparationIntent, OnlinePlan) {
    let root = std::env::temp_dir().join("aster-preparation-contract");
    let release = |slot, version: &str, digit: &str| ActiveReleaseSlot {
        schema: ACTIVE_SLOT_RUNTIME_SCHEMA.into(),
        slot,
        version: version.into(),
        local_runner: Some(ActiveLocalRunner {
            runner_id: format!("runner_{}", digit.repeat(32)),
            manifest_sha256: digit.repeat(64),
        }),
    };
    let previous = release(ReleaseSlot::Blue, "2.0.1", "a");
    let candidate = release(ReleaseSlot::Green, "2.1.0", "b");
    let service = |pid, digit: &str| ServiceInvocation {
        process_id: pid,
        invocation_id: digit.repeat(32),
    };
    let process = |release: &ActiveReleaseSlot, pid, digit: &str, accepting| RuntimeSnapshot {
        schema: "aster.control-runtime.v1".into(),
        installation_id: "installation-one".into(),
        slot: release.slot,
        instance_id: format!("instance-{digit}"),
        service: Some(service(pid, digit)),
        product_version: release.version.clone(),
        lifecycle: LifecycleSnapshot {
            accepting,
            stopping: false,
            revision: 3,
            in_flight: 0,
            oldest_request_age_ms: 0,
        },
    };
    let plan = OnlinePlan {
        job_id: "upgrade-one".into(),
        previous_process: process(&previous, 101, "a", true),
        candidate_process: process(&candidate, 102, "b", false),
        previous_runner_process: service(201, "c"),
        candidate_runner_process: service(202, "d"),
        readiness: ReadinessExpectation {
            manifest_sha256: "b".repeat(64),
            runner_ids: vec![candidate.local_runner.as_ref().unwrap().runner_id.clone()],
            models: vec!["model".into()],
        },
        previous,
        candidate,
        proxy: ProxySnapshot {
            slot: ReleaseSlot::Blue,
            configuration_sha256: "c".repeat(64),
            stream_close_delay_ms: 900_000,
        },
        clock: UpgradeClock {
            boot_id: "boot-one".into(),
            uptime_ms: 1000,
        },
        drain_budget_ms: 720_000,
    };
    let current_migrations = crate::migration::MigrationInspection {
        schema: "aster.migration-inspection.v2".into(),
        applied_schema_version: 4,
        candidate_schema_version: 4,
        applied_history_sha256: "a".repeat(64),
        candidate_history_sha256: "a".repeat(64),
        pending: vec![],
    };
    let intent = PreparationIntent {
        migrations: crate::migration::MigrationPreflight {
            current: current_migrations.clone(),
            candidate: current_migrations,
        },
        job: MaintenanceJob {
            schema: MAINTENANCE_JOB_SCHEMA.into(),
            id: plan.job_id.clone(),
            requested_by: "admin-one".into(),
            operation: MaintenanceOperation::Upgrade {
                archive: root.join("upload.tar.gz"),
                archive_sha256: "e".repeat(64),
            },
            status: MaintenanceStatus::StartingCandidate,
            upgrade_mode: Some(UpgradeMode::BlueGreen),
            runner_was_running: Some(true),
            current_version: plan.previous.version.clone(),
            target_version: Some(plan.candidate.version.clone()),
            previous_release: Some(root.join("2.0.1")),
            candidate_release: Some(root.join("2.1.0")),
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
    assert!(intent.valid());
    assert!(plan.valid());
    (intent, plan)
}

fn startup(plan: &OnlinePlan) -> CandidateStartup {
    CandidateStartup {
        material_sha256: "d".repeat(64),
        clock: plan.clock.clone(),
    }
}
fn activation(plan: &OnlinePlan) -> CandidateActivation {
    CandidateActivation {
        control: plan.candidate_process.clone(),
        runner: plan.candidate_runner_process.clone(),
    }
}
fn activate(record: PreparationJournal, plan: &OnlinePlan) -> PreparationJournal {
    record
        .starting(startup(plan))
        .unwrap()
        .started(activation(plan), &"d".repeat(64), &plan.clock)
        .unwrap()
}

#[test]
fn restart_roundtrip_requires_provisioning_before_handoff() {
    let (intent, plan) = fixture();
    let initial = PreparationJournal::create(intent).unwrap();
    assert!(initial.follows(None));
    assert!(!initial.permits_handoff(&plan));
    let provisioned = initial.provisioned(plan.candidate.clone()).unwrap();
    let restored: PreparationJournal =
        serde_json::from_slice(&serde_json::to_vec(&provisioned).unwrap()).unwrap();
    assert!(restored.follows(Some(&initial)));
    assert!(!restored.follows(None));
    assert!(!restored.permits_handoff(&plan));
    assert!(activate(restored.clone(), &plan).permits_handoff(&plan));
    assert_eq!(
        restored.provisioned(plan.candidate.clone()),
        Some(restored.clone())
    );
    assert!(restored.follows(Some(&restored)));
    assert!(!initial.follows(Some(&restored)));
}

#[test]
fn provisioning_cannot_replace_saved_runner_or_manifest() {
    let (intent, plan) = fixture();
    let initial = PreparationJournal::create(intent).unwrap();
    let saved = initial.provisioned(plan.candidate.clone()).unwrap();
    let mut replacement = plan.candidate.clone();
    replacement.local_runner.as_mut().unwrap().runner_id = format!("runner_{}", "e".repeat(32));
    assert!(initial.provisioned(replacement.clone()).is_some());
    assert!(saved.provisioned(replacement).is_none());
    let mut wrong_manifest = plan.candidate.clone();
    wrong_manifest
        .local_runner
        .as_mut()
        .unwrap()
        .manifest_sha256 = "f".repeat(64);
    assert!(initial.provisioned(wrong_manifest).is_none());
    assert!(initial.provisioned(plan.previous).is_none());
}

#[test]
fn natural_requests_do_not_break_identity_but_reopening_and_reboot_do() {
    let (intent, mut plan) = fixture();
    let saved = PreparationJournal::create(intent)
        .unwrap()
        .provisioned(plan.candidate.clone())
        .unwrap();
    let saved = activate(saved, &plan);
    plan.previous_process.lifecycle.in_flight = 4;
    plan.previous_process.lifecycle.oldest_request_age_ms = 12_000;
    plan.clock.uptime_ms += 12_000;
    assert!(saved.permits_handoff(&plan));
    let mut reopened = plan.clone();
    reopened.previous_process.lifecycle.revision += 2;
    assert!(reopened.valid());
    assert!(!saved.permits_handoff(&reopened));
    plan.clock.boot_id = "boot-two".into();
    assert!(!saved.permits_handoff(&plan));
    plan.clock = saved.intent().clock.clone();
    plan.clock.uptime_ms -= 1;
    assert!(!saved.permits_handoff(&plan));
}

#[test]
fn handoff_rejects_valid_but_unrelated_plan_evidence() {
    let (intent, plan) = fixture();
    let saved = PreparationJournal::create(intent)
        .unwrap()
        .provisioned(plan.candidate.clone())
        .unwrap();
    let saved = activate(saved, &plan);
    let mut variants = Vec::new();
    let mut changed = plan.clone();
    changed.job_id = "upgrade-two".into();
    variants.push(changed);
    let mut changed = plan.clone();
    changed.previous_runner_process.invocation_id = "e".repeat(32);
    variants.push(changed);
    let mut changed = plan.clone();
    changed
        .previous_process
        .service
        .as_mut()
        .unwrap()
        .invocation_id = "f".repeat(32);
    variants.push(changed);
    let mut changed = plan.clone();
    changed.proxy.configuration_sha256 = "e".repeat(64);
    variants.push(changed);
    let mut changed = plan.clone();
    changed.previous_process.installation_id = "installation-two".into();
    changed.candidate_process.installation_id = "installation-two".into();
    variants.push(changed);
    for changed in variants {
        assert!(changed.valid());
        assert!(!saved.permits_handoff(&changed));
    }
}

#[test]
fn mutable_job_progress_preserves_ownership_but_not_other_job_inputs() {
    let (intent, _) = fixture();
    let mut job = intent.job.clone();
    job.status = MaintenanceStatus::SwitchingTraffic;
    job.message = "candidate ready".into();
    job.updated_at = "2026-09-10".into();
    assert!(intent.matches_job(&job));
    job.requested_by = "admin-two".into();
    assert!(!intent.matches_job(&job));
    job = intent.job.clone();
    job.candidate_release = job.previous_release.clone();
    assert!(!intent.matches_job(&job));
    job = intent.job.clone();
    if let MaintenanceOperation::Upgrade { archive_sha256, .. } = &mut job.operation {
        *archive_sha256 = "f".repeat(64);
    }
    assert!(!intent.matches_job(&job));
}

#[test]
fn invalid_intent_cannot_create_preparation_or_change_existing_owner() {
    let (intent, _) = fixture();
    let initial = PreparationJournal::create(intent.clone()).unwrap();
    let mut other = intent.clone();
    other.job.id = "upgrade-two".into();
    let other = PreparationJournal::create(other).unwrap();
    assert!(!other.follows(Some(&initial)));
    let mut cases = Vec::new();
    let mut bad = intent.clone();
    bad.job.upgrade_mode = Some(UpgradeMode::Maintenance);
    cases.push(bad);
    let mut bad = intent.clone();
    bad.job.runner_was_running = Some(false);
    cases.push(bad);
    let mut bad = intent.clone();
    bad.job.status = MaintenanceStatus::Queued;
    cases.push(bad);
    let mut bad = intent.clone();
    bad.previous_process.lifecycle.accepting = false;
    cases.push(bad);
    let mut bad = intent.clone();
    bad.previous_runner_process = bad.previous_process.service.clone().unwrap();
    cases.push(bad);
    let mut bad = intent.clone();
    bad.job.candidate_release = Some("relative".into());
    cases.push(bad);
    let mut bad = intent;
    bad.clock.uptime_ms = u64::MAX;
    cases.push(bad);
    for bad in cases {
        assert!(PreparationJournal::create(bad).is_none());
    }
}

#[test]
fn malformed_serialized_state_does_not_validate_or_advance() {
    let (intent, _) = fixture();
    let initial = PreparationJournal::create(intent).unwrap();
    let mut value = serde_json::to_value(&initial).unwrap();
    value["revision"] = 1.into();
    let corrupt: PreparationJournal = serde_json::from_value(value.clone()).unwrap();
    assert!(!corrupt.valid());
    assert!(!corrupt.follows(Some(&initial)));
    value["revision"] = 0.into();
    value["skip_checks"] = true.into();
    assert!(serde_json::from_value::<PreparationJournal>(value).is_err());
}

#[test]
fn startup_is_durable_sequential_and_cannot_rewrite_prepared_material() {
    let (intent, plan) = fixture();
    let initial = PreparationJournal::create(intent).unwrap();
    assert!(initial.starting(startup(&plan)).is_none());
    let provisioned = initial.provisioned(plan.candidate.clone()).unwrap();
    let starting = provisioned.starting(startup(&plan)).unwrap();
    assert!(starting.follows(Some(&provisioned)));
    assert!(!starting.follows(Some(&initial)));
    assert_eq!(starting.starting(startup(&plan)), Some(starting.clone()));
    assert!(starting.provisioned(plan.candidate.clone()).is_none());
    let mut changed = startup(&plan);
    changed.material_sha256 = "e".repeat(64);
    assert!(starting.starting(changed).is_none());
    let mut changed = startup(&plan);
    changed.clock.uptime_ms += 1;
    assert!(
        starting.starting(changed).is_none(),
        "a retry cannot reset the start budget"
    );
    let started = starting
        .started(activation(&plan), &"d".repeat(64), &plan.clock)
        .unwrap();
    assert!(started.follows(Some(&starting)));
    assert!(!started.follows(Some(&provisioned)));
    assert!(started.starting(startup(&plan)).is_none());
    assert_eq!(
        started.started(activation(&plan), &"d".repeat(64), &plan.clock),
        Some(started.clone())
    );
    let restored: PreparationJournal =
        serde_json::from_slice(&serde_json::to_vec(&started).unwrap()).unwrap();
    assert_eq!(restored, started);
    assert!(restored.permits_handoff(&plan));
}

#[test]
fn startup_requires_same_material_boot_and_original_deadline() {
    let (intent, plan) = fixture();
    let provisioned = PreparationJournal::create(intent)
        .unwrap()
        .provisioned(plan.candidate.clone())
        .unwrap();
    let starting = provisioned.starting(startup(&plan)).unwrap();
    let mut clocks = Vec::new();
    let mut clock = plan.clock.clone();
    clock.boot_id = "different-boot".into();
    clocks.push(clock);
    let mut clock = plan.clock.clone();
    clock.uptime_ms -= 1;
    clocks.push(clock);
    let mut clock = plan.clock.clone();
    clock.uptime_ms += CandidateStartup::BUDGET_MS;
    clocks.push(clock);
    for clock in clocks {
        assert!(
            starting
                .started(activation(&plan), &"d".repeat(64), &clock)
                .is_none()
        );
    }
    assert!(
        starting
            .started(activation(&plan), &"e".repeat(64), &plan.clock)
            .is_none()
    );
    let mut overflow = startup(&plan);
    overflow.clock.uptime_ms = u64::MAX;
    assert!(provisioned.starting(overflow).is_none());
    let mut reboot = startup(&plan);
    reboot.clock.boot_id = "different-boot".into();
    assert!(provisioned.starting(reboot).is_none());
}

#[test]
fn activation_requires_closed_empty_candidate_and_distinct_processes() {
    let (intent, plan) = fixture();
    let starting = PreparationJournal::create(intent)
        .unwrap()
        .provisioned(plan.candidate.clone())
        .unwrap()
        .starting(startup(&plan))
        .unwrap();
    let original = activation(&plan);
    let mut variants = Vec::new();
    let mut bad = original.clone();
    bad.control.lifecycle.accepting = true;
    variants.push(bad);
    let mut bad = original.clone();
    bad.control.lifecycle.stopping = true;
    variants.push(bad);
    let mut bad = original.clone();
    bad.control.lifecycle.in_flight = 1;
    variants.push(bad);
    let mut bad = original.clone();
    bad.control.installation_id = "another-installation".into();
    variants.push(bad);
    let mut bad = original.clone();
    bad.control.slot = plan.previous.slot;
    variants.push(bad);
    let mut bad = original.clone();
    bad.control.service = None;
    variants.push(bad);
    let mut bad = original.clone();
    bad.runner = plan.previous_runner_process.clone();
    variants.push(bad);
    let mut bad = original.clone();
    bad.runner = plan.candidate_process.service.clone().unwrap();
    variants.push(bad);
    for bad in variants {
        assert!(
            starting
                .started(bad, &"d".repeat(64), &plan.clock)
                .is_none()
        );
    }
    let started = starting
        .started(original.clone(), &"d".repeat(64), &plan.clock)
        .unwrap();
    let mut replacement = original;
    replacement.control.instance_id = "replacement-instance".into();
    assert!(
        starting
            .started(replacement.clone(), &"d".repeat(64), &plan.clock)
            .is_some()
    );
    assert!(
        started
            .started(replacement, &"d".repeat(64), &plan.clock)
            .is_none()
    );
    let mut replacement_plan = plan;
    replacement_plan.candidate_process.instance_id = "replacement-instance".into();
    assert!(replacement_plan.valid());
    assert!(!started.permits_handoff(&replacement_plan));
}

#[test]
fn cancellation_freezes_provisioning_startup_and_handoff() {
    let (intent, plan) = fixture();
    let initial = PreparationJournal::create(intent).unwrap();
    assert!(initial.aborting().is_some());
    let provisioned = initial.provisioned(plan.candidate.clone()).unwrap();
    let claimed = provisioned.aborting().unwrap();
    assert!(claimed.follows(Some(&provisioned)));
    for record in [
        claimed.clone(),
        claimed.advance_abort().unwrap(),
        claimed.advance_abort().unwrap().advance_abort().unwrap(),
    ] {
        assert!(record.valid());
        assert!(record.provisioned(plan.candidate.clone()).is_none());
        assert!(record.starting(startup(&plan)).is_none());
        assert!(
            record
                .started(activation(&plan), &"d".repeat(64), &plan.clock)
                .is_none()
        );
        assert!(!record.permits_handoff(&plan));
        assert!(record.aborting().is_none());
        assert!(!provisioned.follows(Some(&record)));
        let restored: PreparationJournal =
            serde_json::from_slice(&serde_json::to_vec(&record).unwrap()).unwrap();
        assert_eq!(restored, record);
        assert_eq!(record.intent(), provisioned.intent());
        assert_eq!(record.candidate(), provisioned.candidate());
    }
    let starting = provisioned.starting(startup(&plan)).unwrap();
    assert!(starting.aborting().is_none());
    assert!(activate(provisioned, &plan).aborting().is_some());
    assert!(!claimed.follows(Some(&starting)));
}

#[test]
fn cancellation_cannot_skip_completion_or_rewrite_identity_or_restart_its_sequence() {
    let (intent, plan) = fixture();
    let provisioned = PreparationJournal::create(intent)
        .unwrap()
        .provisioned(plan.candidate.clone())
        .unwrap();
    let claimed = provisioned.aborting().unwrap();
    let finalizing = claimed.advance_abort().unwrap();
    let complete = finalizing.advance_abort().unwrap();
    assert!(finalizing.follows(Some(&claimed)));
    assert!(complete.follows(Some(&finalizing)));
    assert!(!complete.follows(Some(&claimed)));
    assert!(!claimed.follows(Some(&finalizing)));
    assert!(complete.advance_abort().is_none());
    let mut changed = claimed.clone();
    changed.intent.job.id = "another-job".into();
    assert!(changed.valid());
    assert!(!changed.follows(Some(&claimed)));
    changed = claimed.clone();
    changed.startup = Some(startup(&plan));
    assert!(!changed.valid());
    changed = claimed.clone();
    changed.abort = Some(PreparationAbortPhase::Complete);
    assert!(!changed.valid());
}

#[test]
fn activated_cancellation_requires_fresh_exact_drain_before_retirement() {
    let (intent, plan) = fixture();
    let provisioned = PreparationJournal::create(intent)
        .unwrap()
        .provisioned(plan.candidate.clone())
        .unwrap();
    let started = activate(provisioned, &plan);
    let claimed = started.aborting().unwrap();
    assert!(claimed.follows(Some(&started)));
    assert!(claimed.advance_abort().is_none());
    assert!(claimed.abort_drained().is_none());
    let mut variants = Vec::new();
    let mut wrong = plan.candidate_process.clone();
    wrong.lifecycle.in_flight = 1;
    variants.push(wrong);
    let mut wrong = plan.candidate_process.clone();
    wrong.lifecycle.accepting = true;
    variants.push(wrong);
    let mut wrong = plan.candidate_process.clone();
    wrong.lifecycle.stopping = true;
    variants.push(wrong);
    let mut wrong = plan.candidate_process.clone();
    wrong.lifecycle.revision += 2;
    variants.push(wrong);
    let mut wrong = plan.candidate_process.clone();
    wrong.service.as_mut().unwrap().invocation_id = "f".repeat(32);
    variants.push(wrong);
    variants.push(plan.previous_process.clone());
    for wrong in variants {
        assert!(claimed.retiring_candidate(&wrong).is_none());
    }
    let retiring = claimed.retiring_candidate(&plan.candidate_process).unwrap();
    let finalizing = retiring.advance_abort().unwrap();
    let complete = finalizing.advance_abort().unwrap();
    let mut previous = claimed;
    for record in [retiring, finalizing, complete.clone()] {
        assert!(record.follows(Some(&previous)));
        assert!(!record.follows(Some(&started)));
        assert_eq!(record.abort_drained(), Some(&plan.candidate_process));
        assert!(!record.permits_handoff(&plan));
        assert!(record.starting(startup(&plan)).is_none());
        assert!(record.retiring_candidate(&plan.candidate_process).is_none());
        let restored: PreparationJournal =
            serde_json::from_slice(&serde_json::to_vec(&record).unwrap()).unwrap();
        assert_eq!(record, restored);
        previous = record;
    }
    assert!(complete.advance_abort().is_none());
    let mut invalid = complete;
    invalid.activation = None;
    assert!(!invalid.valid());
}

#[test]
fn late_startup_observation_can_only_enter_cancellation_without_resetting_budget() {
    let (intent, plan) = fixture();
    let starting = PreparationJournal::create(intent)
        .unwrap()
        .provisioned(plan.candidate.clone())
        .unwrap()
        .starting(startup(&plan))
        .unwrap();
    for elapsed in [
        0,
        CandidateStartup::BUDGET_MS - 1,
        CandidateStartup::BUDGET_MS,
        CandidateStartup::BUDGET_MS + 60_000,
    ] {
        let mut clock = plan.clock.clone();
        clock.uptime_ms += elapsed;
        let captured = starting
            .cancel_observed_startup(activation(&plan), &"d".repeat(64), &clock)
            .unwrap();
        assert!(captured.follows(Some(&starting)));
        assert_eq!(captured.startup(), starting.startup());
        assert!(captured.abort_drained().is_none());
        assert!(
            captured
                .retiring_candidate(&plan.candidate_process)
                .is_none()
        );
        assert!(!captured.permits_handoff(&plan));
        assert!(
            captured
                .started(activation(&plan), &"d".repeat(64), &plan.clock)
                .is_none()
        );
        assert!(
            captured
                .cancel_observed_startup(activation(&plan), &"d".repeat(64), &clock)
                .is_none()
        );
        let normal = starting
            .started(activation(&plan), &"d".repeat(64), &plan.clock)
            .unwrap();
        assert!(!normal.follows(Some(&captured)));
        assert!(!captured.follows(Some(&normal)));
        let claimed = captured.advance_abort().unwrap();
        assert!(claimed.follows(Some(&captured)));
        assert!(claimed.advance_abort().is_none());
        let retiring = claimed.retiring_candidate(&plan.candidate_process).unwrap();
        assert!(retiring.follows(Some(&claimed)));
        assert!(!retiring.follows(Some(&captured)));
    }
}

#[test]
fn cancellation_observation_rejects_other_material_boot_clock_or_nonempty_activation() {
    let (intent, plan) = fixture();
    let provisioned = PreparationJournal::create(intent)
        .unwrap()
        .provisioned(plan.candidate.clone())
        .unwrap();
    assert!(
        provisioned
            .cancel_observed_startup(activation(&plan), &"d".repeat(64), &plan.clock)
            .is_none()
    );
    let starting = provisioned.starting(startup(&plan)).unwrap();
    assert!(
        starting
            .cancel_observed_startup(activation(&plan), &"e".repeat(64), &plan.clock)
            .is_none()
    );
    for variant in 0..7 {
        let mut observed = activation(&plan);
        let mut clock = plan.clock.clone();
        match variant {
            0 => clock.boot_id = "another-boot".into(),
            1 => clock.uptime_ms -= 1,
            2 => observed.control.lifecycle.in_flight = 1,
            3 => observed.control.lifecycle.accepting = true,
            4 => observed.control.lifecycle.stopping = true,
            5 => observed.runner = plan.previous_runner_process.clone(),
            _ => observed.control.installation_id = "another-installation".into(),
        }
        assert!(
            starting
                .cancel_observed_startup(observed, &"d".repeat(64), &clock)
                .is_none()
        );
    }
}

#[test]
fn partial_startup_cancellation_is_frozen_and_requires_a_fresh_retirement_checkpoint() {
    let (intent, plan) = fixture();
    let starting = PreparationJournal::create(intent)
        .unwrap()
        .provisioned(plan.candidate.clone())
        .unwrap()
        .starting(startup(&plan))
        .unwrap();
    for partial in [
        PartialCandidateActivation {
            control: Some(plan.candidate_process.clone()),
            runner: None,
        },
        PartialCandidateActivation {
            control: None,
            runner: Some(plan.candidate_runner_process.clone()),
        },
        PartialCandidateActivation {
            control: None,
            runner: None,
        },
    ] {
        let mut late = plan.clock.clone();
        late.uptime_ms += CandidateStartup::BUDGET_MS + 1000;
        let captured = starting
            .cancel_partial_startup(partial.clone(), &"d".repeat(64), &late)
            .unwrap();
        assert!(captured.follows(Some(&starting)));
        assert_eq!(captured.startup(), starting.startup());
        assert!(captured.activation().is_none());
        let claimed = captured.advance_abort().unwrap();
        assert!(claimed.follows(Some(&captured)));
        assert!(
            claimed.advance_abort().is_none(),
            "cannot skip fresh observation"
        );
        assert!(
            claimed
                .retiring_candidate(&plan.candidate_process)
                .is_none()
        );
        let retiring = claimed.retiring_partial_candidate(&partial).unwrap();
        let finalizing = retiring.advance_abort().unwrap();
        let complete = finalizing.advance_abort().unwrap();
        for (old, next) in [
            (&claimed, &retiring),
            (&retiring, &finalizing),
            (&finalizing, &complete),
        ] {
            let reopened: PreparationJournal =
                serde_json::from_slice(&serde_json::to_vec(next).unwrap()).unwrap();
            assert!(reopened.valid());
            assert!(reopened.follows(Some(old)));
            assert_eq!(reopened.partial_activation(), Some(&partial));
            assert!(!reopened.permits_handoff(&plan));
            assert!(reopened.starting(startup(&plan)).is_none());
            assert!(
                reopened
                    .started(activation(&plan), &"d".repeat(64), &plan.clock)
                    .is_none()
            );
            assert!(
                reopened
                    .cancel_partial_startup(partial.clone(), &"d".repeat(64), &late)
                    .is_none()
            );
            assert!(!starting.follows(Some(&reopened)));
        }
        assert!(!complete.follows(Some(&claimed)));
        let mut changed = partial.clone();
        if let Some(control) = &mut changed.control {
            control.instance_id = "replacement".into();
        } else {
            changed.runner = Some(ServiceInvocation {
                process_id: 301,
                invocation_id: "e".repeat(32),
            });
        }
        assert!(claimed.retiring_partial_candidate(&changed).is_none());
        let mut forged = retiring.clone();
        forged.partial_activation = Some(changed);
        assert!(!forged.follows(Some(&claimed)));
    }
}

#[test]
fn partial_capture_rejects_open_busy_wrong_identity_or_boot_and_mixed_full_activation() {
    let (intent, plan) = fixture();
    let starting = PreparationJournal::create(intent)
        .unwrap()
        .provisioned(plan.candidate.clone())
        .unwrap()
        .starting(startup(&plan))
        .unwrap();
    let partial = PartialCandidateActivation {
        control: Some(plan.candidate_process.clone()),
        runner: None,
    };
    for fault in 0..10 {
        let mut observed = partial.clone();
        let mut clock = plan.clock.clone();
        let mut digest = "d".repeat(64);
        match fault {
            0 => observed.control.as_mut().unwrap().lifecycle.accepting = true,
            1 => observed.control.as_mut().unwrap().lifecycle.in_flight = 1,
            2 => observed.control.as_mut().unwrap().lifecycle.stopping = true,
            3 => observed.control.as_mut().unwrap().installation_id = "other".into(),
            4 => {
                observed.control.as_mut().unwrap().service =
                    Some(plan.previous_runner_process.clone())
            }
            5 => {
                observed = PartialCandidateActivation {
                    control: None,
                    runner: Some(plan.previous_runner_process.clone()),
                }
            }
            6 => observed.runner = Some(plan.candidate_runner_process.clone()),
            7 => clock.boot_id = "different-boot".into(),
            8 => clock.uptime_ms -= 1,
            _ => digest = "e".repeat(64),
        }
        assert!(
            starting
                .cancel_partial_startup(observed, &digest, &clock)
                .is_none(),
            "fault {fault}"
        );
    }
    let captured = starting
        .cancel_partial_startup(partial, &"d".repeat(64), &plan.clock)
        .unwrap();
    let mut mixed = captured.clone();
    mixed.activation = Some(activation(&plan));
    assert!(!mixed.valid());
    let mut no_abort = captured.clone();
    no_abort.abort = None;
    assert!(!no_abort.valid());
    let mut old_schema = captured;
    old_schema.schema = "aster.online-preparation.v6".into();
    assert!(!old_schema.valid());
}

#[test]
fn unprovisioned_cancellation_preserves_unknown_identity_until_signed_reconciliation() {
    let (intent, plan) = fixture();
    let initial = PreparationJournal::create(intent).unwrap();
    let claimed = initial.aborting().unwrap();
    assert!(claimed.follows(Some(&initial)));
    assert_eq!(claimed.revision, 1);
    let finalizing = claimed.advance_abort().unwrap();
    let complete = finalizing.advance_abort().unwrap();
    for (old, next) in [
        (&initial, &claimed),
        (&claimed, &finalizing),
        (&finalizing, &complete),
    ] {
        assert!(next.valid());
        assert!(next.follows(Some(old)));
        assert!(next.candidate().is_none());
        assert!(next.startup().is_none());
        assert!(next.activation().is_none());
        assert!(next.partial_activation().is_none());
        assert!(next.provisioned(plan.candidate.clone()).is_none());
        assert!(next.starting(startup(&plan)).is_none());
        assert!(!next.permits_handoff(&plan));
        let reopened: PreparationJournal =
            serde_json::from_slice(&serde_json::to_vec(next).unwrap()).unwrap();
        assert_eq!(&reopened, next);
    }
    assert!(!complete.follows(Some(&claimed)));
    assert!(complete.advance_abort().is_none());
    let provisioned = initial.provisioned(plan.candidate.clone()).unwrap();
    let provisioned_claim = provisioned.aborting().unwrap();
    assert_eq!(provisioned_claim.revision, claimed.revision + 1);
    assert!(
        !provisioned_claim.follows(Some(&claimed)),
        "cancellation must freeze an absent identity too"
    );
    assert!(!provisioned.follows(Some(&claimed)));
    assert!(!claimed.follows(Some(&provisioned)));
    let mut forged = complete.clone();
    forged.candidate = Some(plan.candidate);
    assert!(
        !forged.valid(),
        "unknown and provisioned cancellation have different revision chains"
    );
}
