use super::*;
use aster_license_core::{TrustedLicenseKeys, v2};
use ed25519_dalek::SigningKey;
use tempfile::tempdir;
use time::macros::datetime;

const NOW: OffsetDateTime = datetime!(2026-09-08 0:00 UTC);

fn store(root: &Path) -> LicenseStateStore {
    LicenseStateStore::new(root.join("state.json"), &[91; 32]).unwrap()
}

fn free() -> VerifiedProductLicense {
    let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../../contracts/test-vectors/license.v2.json"
    ))
    .unwrap();
    let document: v2::Document =
        serde_json::from_value(fixture["cases"][1]["document"].clone()).unwrap();
    verify_test_document(&serde_json::to_vec(&document).unwrap())
}

fn verify_test_document(source: &[u8]) -> VerifiedProductLicense {
    let document: v2::Document = serde_json::from_slice(source).unwrap();
    let mut keys = TrustedLicenseKeys::new();
    keys.insert_scoped(
        "test-only-v2",
        SigningKey::from_bytes(&[42; 32]).verifying_key(),
        v2::IssuerPolicy {
            sources: vec![document.claims.source.kind()],
            bindings: vec![document.claims.binding.kind()],
            expiries: vec![document.claims.validity.expiry.kind()],
            entitlement_ceiling: document.claims.entitlements.clone(),
        },
    )
    .unwrap();
    v2::verify(source, &keys).unwrap().into()
}

fn audit() -> LicenseImportAudit {
    LicenseImportAudit {
        event_id: "license_import_free_switch_test".to_owned(),
        actor: crate::LicenseImportActor::Administrator {
            identity_id: "owner_test".to_owned(),
            role: "owner".to_owned(),
        },
        action: LicenseImportAction::SwitchFree,
    }
}

#[test]
fn explicit_free_switch_preserves_paid_watermark_across_restart_observation_and_import() {
    let directory = tempdir().unwrap();
    let history = store(directory.path());
    let target = directory.path().join("license.json");
    let paid = crate::tests::v2_license(4, "2026-09-06T00:00:00.000Z");
    let next = crate::tests::v2_license(0, "2026-09-07T00:00:00.000Z");
    let free = free();
    history.install_document(&target, &paid, NOW).unwrap();
    assert_eq!(
        history.install_document(&target, &free, NOW),
        Err(LicenseStateError::LicenseRollback)
    );
    let lease = history.begin_free_switch(&target, &paid, NOW).unwrap();
    history
        .commit_free_switch(lease, &free, NOW, audit())
        .unwrap();
    assert_eq!(
        history.pending_activation_audits(&target).unwrap()[0].action(),
        "license.switch_free"
    );

    let restarted = store(directory.path());
    assert_eq!(
        restarted.read_committed_document(&target).unwrap().unwrap(),
        free.source()
    );
    restarted
        .check_and_observe(&free, NOW + time::Duration::minutes(2))
        .unwrap();
    restarted
        .accept_replacement(&free, NOW + time::Duration::minutes(2))
        .unwrap();
    restarted
        .install_document(&target, &free, NOW + time::Duration::minutes(2))
        .unwrap();
    assert_eq!(
        restarted.load().unwrap().last_issued_at,
        paid.as_ref().issued_at()
    );
    assert_eq!(restarted.load().unwrap().last_transfer_sequence, 4);
    assert_eq!(
        restarted.install_document(&target, &paid, NOW),
        Err(LicenseStateError::LicenseRollback)
    );
    restarted.install_document(&target, &next, NOW).unwrap();
    assert_eq!(restarted.load().unwrap().last_transfer_sequence, 0);
    assert_eq!(
        restarted.install_document(&target, &free, NOW),
        Err(LicenseStateError::LicenseRollback)
    );
}

#[test]
fn free_switch_lease_blocks_concurrent_mutations_and_rejects_stale_or_non_free_inputs() {
    let directory = tempdir().unwrap();
    let history = store(directory.path());
    let other = store(directory.path());
    let target = directory.path().join("license.json");
    let paid = crate::tests::v2_license(1, "2026-09-05T00:00:00.000Z");
    let next = crate::tests::v2_license(2, "2026-09-06T00:00:00.000Z");
    history.install_document(&target, &paid, NOW).unwrap();
    let mutation = other
        .guard_mutation(Some(&paid), Some(&target), NOW)
        .unwrap();
    assert!(matches!(
        history.begin_free_switch(&target, &paid, NOW),
        Err(LicenseStateError::MutationConflict)
    ));
    drop(mutation);
    let lease = history.begin_free_switch(&target, &paid, NOW).unwrap();
    assert!(matches!(
        other.guard_mutation(Some(&paid), Some(&target), NOW),
        Err(LicenseStateError::MutationConflict)
    ));
    assert_eq!(
        other.install_document(&target, &next, NOW),
        Err(LicenseStateError::MutationConflict)
    );
    assert_eq!(
        history.commit_free_switch(lease, &next, NOW, audit()),
        Err(LicenseStateError::LicenseRollback)
    );
    assert_eq!(fs::read(&target).unwrap(), paid.source());
    let lease = history.begin_free_switch(&target, &paid, NOW).unwrap();
    assert_eq!(
        other.commit_free_switch(lease, &free(), NOW, audit()),
        Err(LicenseStateError::MutationConflict)
    );
    other.install_document(&target, &next, NOW).unwrap();
    assert!(matches!(
        history.begin_free_switch(&target, &paid, NOW),
        Err(LicenseStateError::MutationConflict)
    ));
    assert!(matches!(
        history.begin_free_switch(&target, &next, NOW - time::Duration::hours(1)),
        Err(LicenseStateError::ClockRollback)
    ));
}

#[test]
fn free_switch_recovers_each_partial_commit_and_preserves_scheduled_paid_document() {
    for phase in 0..=3 {
        let directory = tempdir().unwrap();
        let history = store(directory.path());
        let target = directory.path().join("license.json");
        let paid = crate::tests::v2_license(2, "2026-09-06T00:00:00.000Z");
        let scheduled = crate::tests::v2_license(0, "2026-09-09T00:00:00.000Z");
        let free = free();
        history.install_document(&target, &paid, NOW).unwrap();
        history.stage_document(&target, &scheduled, NOW).unwrap();
        let staged = fs::read(history.sidecar(".staged")).unwrap();
        let lease = history.begin_free_switch(&target, &paid, NOW).unwrap();
        let update = history
            .prepare_free_switch(&target, &paid, &free, NOW, audit())
            .unwrap();
        history.write_update_intent(&update).unwrap();
        if phase >= 1 {
            write_atomic(
                &history.path,
                &decode_bytes(&update.claims.state, MAX_STATE_BYTES).unwrap(),
            )
            .unwrap();
        }
        if phase >= 2 {
            write_atomic(&target, free.source()).unwrap();
        }
        if phase >= 3 {
            write_atomic(
                &history.sidecar(".activation"),
                &decode_bytes(
                    update.claims.activation_audits.as_ref().unwrap(),
                    MAX_ACTIVATION_AUDIT_BYTES,
                )
                .unwrap(),
            )
            .unwrap();
        }
        drop(lease);
        let restarted = store(directory.path());
        assert_eq!(
            restarted.read_committed_document(&target).unwrap().unwrap(),
            free.source()
        );
        assert_eq!(fs::read(restarted.sidecar(".staged")).unwrap(), staged);
        assert_eq!(
            restarted.pending_activation_audits(&target).unwrap().len(),
            1
        );
        assert_eq!(
            restarted.load().unwrap().last_issued_at,
            paid.as_ref().issued_at()
        );
        assert_eq!(
            restarted.install_document(&target, &paid, NOW),
            Err(LicenseStateError::LicenseRollback)
        );
        restarted
            .activate_staged_document(&target, &scheduled, NOW + time::Duration::days(1))
            .unwrap();
        assert_eq!(
            restarted.read_committed_document(&target).unwrap().unwrap(),
            scheduled.source()
        );
    }
}

#[test]
fn newer_free_document_never_supersedes_an_accepted_future_paid_renewal() {
    let directory = tempdir().unwrap();
    let history = store(directory.path());
    let target = directory.path().join("license.json");
    let paid = crate::tests::v2_license(2, "2026-08-26T00:00:00.000Z");
    let renewal = crate::tests::v2_license(0, "2026-08-27T00:00:00.000Z");
    let mut document: v2::Document = serde_json::from_slice(renewal.source()).unwrap();
    document.claims.validity.not_before = "2026-10-01T00:00:00.000Z".to_owned();
    let document = v2::sign(document.claims, &SigningKey::from_bytes(&[42; 32])).unwrap();
    let renewal = verify_test_document(&serde_json::to_vec(&document).unwrap());
    history.install_document(&target, &paid, NOW).unwrap();
    history.stage_document(&target, &renewal, NOW).unwrap();
    let free = free();
    assert_eq!(
        history.install_document(&target, &free, NOW),
        Err(LicenseStateError::LicenseRollback)
    );
    assert_eq!(
        history.stage_document(&target, &free, NOW),
        Err(LicenseStateError::LicenseRollback)
    );
    assert_eq!(
        history.validate_document_progress(&free),
        Err(LicenseStateError::LicenseRollback)
    );
    let lease = history.begin_free_switch(&target, &paid, NOW).unwrap();
    history
        .commit_free_switch(lease, &free, NOW, audit())
        .unwrap();
    history
        .check_and_observe(&free, NOW + time::Duration::hours(1))
        .unwrap();
    history
        .install_document(&target, &free, NOW + time::Duration::hours(1))
        .unwrap();
    assert_eq!(
        history.load().unwrap().last_issued_at,
        paid.as_ref().issued_at()
    );
    assert_eq!(
        history.install_document(&target, &paid, NOW),
        Err(LicenseStateError::LicenseRollback)
    );
    let outcome = history
        .activate_staged_document(&target, &renewal, datetime!(2026-10-01 0:00 UTC))
        .unwrap();
    assert!(matches!(outcome, StagedActivationOutcome::Activated { .. }));
    assert_eq!(
        history.read_committed_document(&target).unwrap().unwrap(),
        renewal.source()
    );
}

#[test]
fn oversized_verified_free_document_is_rejected_before_recording_an_intent() {
    let directory = tempdir().unwrap();
    let history = store(directory.path());
    let target = directory.path().join("license.json");
    let paid = crate::tests::v2_license(1, "2026-09-05T00:00:00.000Z");
    history.install_document(&target, &paid, NOW).unwrap();
    let mut source = free().source().to_vec();
    source.resize(MAX_LICENSE_DOCUMENT_BYTES + 1, b' ');
    let oversized = verify_test_document(&source);
    let lease = history.begin_free_switch(&target, &paid, NOW).unwrap();
    assert_eq!(
        history.commit_free_switch(lease, &oversized, NOW, audit()),
        Err(LicenseStateError::InvalidJson)
    );
    assert_eq!(
        history.read_committed_document(&target).unwrap().unwrap(),
        paid.source()
    );
    assert!(fs::read(history.sidecar(".pending")).unwrap().is_empty());
    assert!(
        history
            .pending_activation_audits(&target)
            .unwrap()
            .is_empty()
    );
}
