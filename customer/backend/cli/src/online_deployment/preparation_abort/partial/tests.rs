use super::*;
use aster_upgrade_core::preparation::{CandidateStartup, PartialCandidateActivation};

#[test]
fn finalized_partial_cancellation_retains_normal_exit_contract_for_captured_services() {
    let (_root, _layout, _store, initial, plan) = crate::online_journal::preparation_fixture();
    let starting = initial
        .provisioned(plan.candidate.clone())
        .unwrap()
        .starting(CandidateStartup {
            material_sha256: "d".repeat(64),
            clock: plan.clock.clone(),
        })
        .unwrap();
    for variant in 0..3 {
        let partial = PartialCandidateActivation {
            control: (variant == 0).then(|| plan.candidate_process.clone()),
            runner: (variant == 1).then(|| plan.candidate_runner_process.clone()),
        };
        let claimed = starting
            .cancel_partial_startup(partial.clone(), &"d".repeat(64), &plan.clock)
            .unwrap()
            .advance_abort()
            .unwrap();
        let retiring = claimed.retiring_partial_candidate(&partial).unwrap();
        assert!(assert_finalized(&retiring, |_| panic!("not finalized")).is_err());
        let finalizing = retiring.advance_abort().unwrap();
        for record in [&finalizing, &finalizing.advance_abort().unwrap()] {
            for absent in ["inactive", "failed"] {
                assert_finalized(record, |unit| {
                    let captured = if unit.contains("control") {
                        partial.control.is_some()
                    } else {
                        partial.runner.is_some()
                    };
                    Ok(if captured { "inactive" } else { absent }.into())
                })
                .unwrap();
            }
            for unit in ["aster-control@green.service", "aster-runner@green.service"] {
                for state in [
                    "active",
                    "activating",
                    "deactivating",
                    "unknown",
                    "not-installed",
                ] {
                    assert!(
                        assert_finalized(record, |query| Ok(if query == unit {
                            state
                        } else {
                            "inactive"
                        }
                        .into()))
                        .is_err()
                    );
                }
                assert!(
                    assert_finalized(record, |query| if query == unit {
                        Err(failed("observation failed"))
                    } else {
                        Ok("inactive".into())
                    })
                    .is_err()
                );
            }
            if variant < 2 {
                assert!(
                    assert_finalized(record, |_| Ok("failed".into())).is_err(),
                    "captured activation must exit normally"
                );
            }
        }
    }
}
