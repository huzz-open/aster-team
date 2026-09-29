use super::*;
use aster_release_core::{ReleaseClaims, ReleaseFile, TrustedReleaseKeys, sign};
use aster_upgrade_core::settlement;
use ed25519_dalek::SigningKey;
use std::{fs, path::Path};

fn signed_release(root: &Path, version: &str, marker: Option<&str>) -> TrustedReleaseKeys {
    let key = SigningKey::from_bytes(&[97; 32]);
    let mut files = vec![("bin/aster-control", "signed binary fixture")];
    if let Some(marker) = marker {
        files.push((settlement::CAPABILITY_FILE, marker));
    }
    let mut claims = Vec::new();
    for (path, bytes) in files {
        let target = root.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, bytes).unwrap();
        claims.push(ReleaseFile {
            path: path.into(),
            size: bytes.len() as u64,
            sha256: crate::sha256_file(&target).unwrap(),
            executable: false,
        });
    }
    claims.sort_by(|a, b| a.path.cmp(&b.path));
    let document = sign(
        ReleaseClaims {
            schema: aster_release_core::RELEASE_SCHEMA.into(),
            key_id: "settlement_test".into(),
            product: "aster-team".into(),
            version: version.into(),
            platform: "linux".into(),
            architecture: "amd64".into(),
            runtime: "musl-static".into(),
            created_at: "2026-09-09T00:00:00.000Z".into(),
            files: claims,
        },
        &key,
    )
    .unwrap();
    fs::write(
        root.join("RELEASE.json"),
        serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();
    let mut keys = TrustedReleaseKeys::new();
    keys.insert("settlement_test", key.verifying_key()).unwrap();
    keys
}

fn journal(layout: &InstallLayout) -> OnlineJournal {
    let mut plan = crate::online_journal::tests::plan();
    plan.candidate.version = "2.2.0".into();
    plan.candidate_process.product_version = "2.2.0".into();
    for slot in [&mut plan.previous, &mut plan.candidate] {
        slot.local_runner.as_mut().unwrap().manifest_sha256 =
            crate::sha256_file(&layout.release(&slot.version).join("RELEASE.json")).unwrap();
    }
    plan.readiness.manifest_sha256 = plan
        .candidate
        .local_runner
        .as_ref()
        .unwrap()
        .manifest_sha256
        .clone();
    let mut store = JournalFile::open(layout).unwrap();
    OnlineJournal::create(plan, &mut store).unwrap()
}

fn at_retirement(journal: &OnlineJournal, phase: &str, revision: u64) -> OnlineJournal {
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
    value["cutover_started_at_ms"] = 1000.into();
    value["drain_started_at_ms"] = 1000.into();
    value["phase"] = phase.into();
    value["revision"] = revision.into();
    let result: OnlineJournal = serde_json::from_value(value).unwrap();
    assert!(result.valid());
    result
}

#[test]
fn existing_forward_journal_requires_signed_compatible_peers_without_using_current_pointer() {
    let temporary = tempfile::tempdir().unwrap();
    let layout = InstallLayout::new(temporary.path()).unwrap();
    let previous = layout.release("2.1.0");
    let candidate = layout.release("2.2.0");
    let keys = signed_release(&previous, "2.1.0", Some(settlement::CAPABILITY_JSON));
    signed_release(&candidate, "2.2.0", Some(settlement::CAPABILITY_JSON));
    let journal = journal(&layout);
    // This gate uses immutable release identities, including after current moved.
    std::os::unix::fs::symlink(&candidate, layout.current()).unwrap();
    require_forward_protocol_with_keys(&layout, &journal, &keys).unwrap();
    fs::write(previous.join(settlement::CAPABILITY_FILE), "{}").unwrap();
    assert!(require_forward_protocol_with_keys(&layout, &journal, &keys).is_err());
    signed_release(&previous, "2.1.0", Some(settlement::CAPABILITY_JSON));
    let incompatible = settlement::CAPABILITY_JSON
        .replace(settlement::INTENT_SCHEMA, "aster.gateway-settlement.v2");
    signed_release(&candidate, "2.2.0", Some(&incompatible));
    assert!(require_forward_protocol_with_keys(&layout, &journal, &keys).is_err());
}

#[test]
fn old_package_can_finish_committing_but_cannot_continue_coexistence() {
    let temporary = tempfile::tempdir().unwrap();
    let layout = InstallLayout::new(temporary.path()).unwrap();
    let previous = layout.release("2.1.0");
    let candidate = layout.release("2.2.0");
    let keys = signed_release(&previous, "2.1.0", None);
    signed_release(&candidate, "2.2.0", None);
    let journal = journal(&layout);
    assert!(require_forward_protocol_with_keys(&layout, &journal, &keys).is_err());
    let retiring = at_retirement(&journal, "retiring_previous", 4);
    assert!(require_forward_protocol_with_keys(&layout, &retiring, &keys).is_err());
    let committing = at_retirement(&journal, "committing", 5);
    require_forward_protocol_with_keys(&layout, &committing, &keys).unwrap();
    // Finishing never waives release authentication or the frozen manifest.
    assert!(
        require_forward_protocol_with_keys(&layout, &committing, &TrustedReleaseKeys::new())
            .is_err()
    );
    signed_release(&previous, "2.1.0", Some(settlement::CAPABILITY_JSON));
    assert!(require_forward_protocol_with_keys(&layout, &committing, &keys).is_err());
    signed_release(&previous, "2.1.0", None);
    fs::remove_file(previous.join(settlement::CAPABILITY_FILE)).unwrap();
    fs::write(candidate.join("bin/aster-control"), "tampered").unwrap();
    assert!(require_forward_protocol_with_keys(&layout, &committing, &keys).is_err());
}
