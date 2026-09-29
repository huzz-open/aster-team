//! Candidate-side counterpart to CLI online admission. Release signatures
//! cover the capability bytes and the exact binaries for both installed peers.
use super::{document, invalid};
use aster_install_layout::InstallLayout;
use aster_release_core::{RELEASE_MANIFEST_FILE, TrustedReleaseKeys, verify, verify_release_tree};
use aster_upgrade_core::{ActiveReleaseSlot, MaintenanceJob, settlement};
use sha2::{Digest as _, Sha256};
use std::{fs, io, io::Read as _, path::Path};

pub(super) fn require_pair(
    layout: &InstallLayout,
    job: &MaintenanceJob,
    active: &ActiveReleaseSlot,
    keys: &TrustedReleaseKeys,
) -> io::Result<()> {
    job.validate().map_err(|_| invalid())?;
    active.validate().map_err(|_| invalid())?;
    if job.upgrade_mode != Some(aster_upgrade_core::UpgradeMode::BlueGreen) {
        return Ok(());
    }
    let target = job.target_version.as_deref().ok_or_else(invalid)?;
    let previous = layout.release(&job.current_version);
    let candidate = layout.release(target);
    if active.version != job.current_version
        || job.previous_release.as_ref() != Some(&previous)
        || job.candidate_release.as_ref() != Some(&candidate)
        || previous == candidate
        || layout.current().canonicalize()? != previous.canonicalize()?
        || layout.slot_release(active.slot.id()).canonicalize()? != previous.canonicalize()?
    {
        return Err(invalid());
    }
    let expected = &active
        .local_runner
        .as_ref()
        .ok_or_else(invalid)?
        .manifest_sha256;
    check_release(&previous, &job.current_version, Some(expected), keys)?;
    check_release(&candidate, target, None, keys)
}

fn check_release(
    root: &Path,
    version: &str,
    expected: Option<&str>,
    keys: &TrustedReleaseKeys,
) -> io::Result<()> {
    let path = root.join(RELEASE_MANIFEST_FILE);
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 16 * 1024 * 1024
    {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 16 * 1024 * 1024 || expected.is_some_and(|expected| digest(&bytes) != expected)
    {
        return Err(invalid());
    }
    let release = verify(&bytes, keys).map_err(|_| invalid())?;
    if release.claims().version != version
        || release.claims().platform != "linux"
        || release.claims().architecture != "amd64"
        || !release
            .claims()
            .files
            .iter()
            .any(|f| f.path == "bin/aster-control" && f.size > 0)
        || !release
            .claims()
            .files
            .iter()
            .any(|f| f.path == settlement::CAPABILITY_FILE)
    {
        return Err(invalid());
    }
    verify_release_tree(root, &release).map_err(|_| invalid())?;
    if !settlement::compatible(&document(&root.join(settlement::CAPABILITY_FILE))?) {
        return Err(io::Error::other(
            "online upgrade requires matching signed gateway settlement protocols; use maintenance upgrade",
        ));
    }
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(&mut encoded, "{byte:02x}").expect("String write");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use aster_release_core::{ReleaseClaims, ReleaseFile, sign};
    use aster_upgrade_core::{
        ACTIVE_SLOT_RUNTIME_SCHEMA, ActiveLocalRunner, MAINTENANCE_JOB_SCHEMA,
        MaintenanceOperation, MaintenanceStatus, ReleaseSlot, UpgradeMode,
    };
    use ed25519_dalek::SigningKey;
    fn fixture(contract: Option<&[u8]>) -> (tempfile::TempDir, TrustedReleaseKeys) {
        let root = tempfile::tempdir().unwrap();
        let keys = release_fixture(root.path(), "2.1.0", contract);
        (root, keys)
    }

    fn release_fixture(root: &Path, version: &str, contract: Option<&[u8]>) -> TrustedReleaseKeys {
        let key = SigningKey::from_bytes(&[97; 32]);
        let mut files = vec![("bin/aster-control", b"signed binary fixture".as_slice())];
        if let Some(bytes) = contract {
            files.push((settlement::CAPABILITY_FILE, bytes));
        }
        let mut signed = Vec::new();
        for (path, bytes) in files {
            let target = root.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(&target, bytes).unwrap();
            signed.push(ReleaseFile {
                path: path.into(),
                size: bytes.len() as u64,
                sha256: digest(bytes),
                executable: false,
            });
        }
        signed.sort_by(|a, b| a.path.cmp(&b.path));
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
                files: signed,
            },
            &key,
        )
        .unwrap();
        fs::write(
            root.join(RELEASE_MANIFEST_FILE),
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
        let mut keys = TrustedReleaseKeys::new();
        keys.insert("settlement_test", key.verifying_key()).unwrap();
        keys
    }

    fn pair_metadata(layout: &InstallLayout) -> (MaintenanceJob, ActiveReleaseSlot) {
        let job = MaintenanceJob {
            schema: MAINTENANCE_JOB_SCHEMA.into(),
            id: "upgrade_protocol".into(),
            requested_by: "owner".into(),
            operation: MaintenanceOperation::Upgrade {
                archive: layout.root().join("candidate.tar.gz"),
                archive_sha256: "a".repeat(64),
            },
            status: MaintenanceStatus::Staging,
            upgrade_mode: Some(UpgradeMode::BlueGreen),
            runner_was_running: Some(true),
            current_version: "2.1.0".into(),
            target_version: Some("2.2.0".into()),
            previous_release: Some(layout.release("2.1.0")),
            candidate_release: Some(layout.release("2.2.0")),
            message: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
        };
        let active = ActiveReleaseSlot {
            schema: ACTIVE_SLOT_RUNTIME_SCHEMA.into(),
            slot: ReleaseSlot::Blue,
            version: "2.1.0".into(),
            local_runner: Some(ActiveLocalRunner {
                runner_id: format!("runner_{}", "a".repeat(32)),
                manifest_sha256: "b".repeat(64),
            }),
        };
        (job, active)
    }

    #[test]
    fn maintenance_does_not_require_online_protocol_but_still_validates_metadata() {
        let root = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(root.path()).unwrap();
        let (mut job, mut active) = pair_metadata(&layout);
        job.upgrade_mode = Some(UpgradeMode::Maintenance);
        // No release files or slot links: only the online compatibility gate is exempt.
        require_pair(&layout, &job, &active, &TrustedReleaseKeys::new()).unwrap();
        active.schema = "untrusted".into();
        assert!(require_pair(&layout, &job, &active, &TrustedReleaseKeys::new()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn online_pair_requires_both_signed_protocols_and_the_actual_active_topology() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(root.path()).unwrap();
        let (job, mut active) = pair_metadata(&layout);
        let previous = layout.release("2.1.0");
        let candidate = layout.release("2.2.0");
        let keys = release_fixture(&previous, "2.1.0", None);
        release_fixture(
            &candidate,
            "2.2.0",
            Some(settlement::CAPABILITY_JSON.as_bytes()),
        );
        symlink(&previous, layout.current()).unwrap();
        let slot = layout.slot_release("blue");
        fs::create_dir_all(slot.parent().unwrap()).unwrap();
        symlink(&previous, &slot).unwrap();
        let freeze = |active: &mut ActiveReleaseSlot| {
            active.local_runner.as_mut().unwrap().manifest_sha256 =
                digest(&fs::read(previous.join(RELEASE_MANIFEST_FILE)).unwrap());
        };
        freeze(&mut active);
        assert!(require_pair(&layout, &job, &active, &keys).is_err());
        release_fixture(
            &previous,
            "2.1.0",
            Some(settlement::CAPABILITY_JSON.as_bytes()),
        );
        // A correctly signed replacement still differs from the frozen active release.
        assert!(require_pair(&layout, &job, &active, &keys).is_err());
        freeze(&mut active);
        require_pair(&layout, &job, &active, &keys).unwrap();
        let mut unrelated = job.clone();
        unrelated.candidate_release = Some(previous.clone());
        assert!(require_pair(&layout, &unrelated, &active, &keys).is_err());
        fs::remove_file(&slot).unwrap();
        symlink(&candidate, &slot).unwrap();
        assert!(require_pair(&layout, &job, &active, &keys).is_err());
        fs::remove_file(&slot).unwrap();
        symlink(&previous, &slot).unwrap();
        fs::remove_file(layout.current()).unwrap();
        symlink(&candidate, layout.current()).unwrap();
        assert!(require_pair(&layout, &job, &active, &keys).is_err());
        fs::remove_file(layout.current()).unwrap();
        symlink(&previous, layout.current()).unwrap();
        fs::write(candidate.join(settlement::CAPABILITY_FILE), b"{}").unwrap();
        assert!(require_pair(&layout, &job, &active, &keys).is_err());
    }
    #[test]
    fn signed_protocol_must_match_its_frozen_version_and_manifest() {
        let (root, keys) = fixture(Some(settlement::CAPABILITY_JSON.as_bytes()));
        let manifest = fs::read(root.path().join(RELEASE_MANIFEST_FILE)).unwrap();
        check_release(root.path(), "2.1.0", Some(&digest(&manifest)), &keys).unwrap();
        assert!(check_release(root.path(), "2.0.0", None, &keys).is_err());
        assert!(check_release(root.path(), "2.1.0", Some(&"a".repeat(64)), &keys).is_err());
        assert!(check_release(root.path(), "2.1.0", None, &TrustedReleaseKeys::new()).is_err());
    }
    #[test]
    fn missing_or_unsigned_added_protocol_cannot_authorize_an_old_release() {
        let (root, keys) = fixture(None);
        assert!(check_release(root.path(), "2.1.0", None, &keys).is_err());
        fs::create_dir(root.path().join("systemd")).unwrap();
        fs::write(
            root.path().join(settlement::CAPABILITY_FILE),
            settlement::CAPABILITY_JSON,
        )
        .unwrap();
        assert!(check_release(root.path(), "2.1.0", None, &keys).is_err());
    }
    #[test]
    fn protocol_tampering_and_legitimately_signed_incompatible_versions_are_rejected() {
        let (root, keys) = fixture(Some(settlement::CAPABILITY_JSON.as_bytes()));
        fs::write(
            root.path().join(settlement::CAPABILITY_FILE),
            format!("{} ", settlement::CAPABILITY_JSON),
        )
        .unwrap();
        assert!(check_release(root.path(), "2.1.0", None, &keys).is_err());
        let changed = settlement::CAPABILITY_JSON.replace(
            settlement::REQUEST_LOCK_SCHEMA,
            "aster.gateway-request-lock.v2",
        );
        let (root, keys) = fixture(Some(changed.as_bytes()));
        assert!(check_release(root.path(), "2.1.0", None, &keys).is_err());
        let changed = settlement::CAPABILITY_JSON
            .replace(settlement::INTENT_SCHEMA, "aster.gateway-settlement.v2");
        let (root, keys) = fixture(Some(changed.as_bytes()));
        assert!(check_release(root.path(), "2.1.0", None, &keys).is_err());
    }
}
