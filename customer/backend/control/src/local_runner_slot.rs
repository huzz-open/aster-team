use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read as _},
    path::Path,
};

use aster_control::BootstrapLocalRunnerIdentity;
use aster_install_layout::InstallLayout;
use aster_upgrade_core::ReleaseSlot;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize as _, Zeroizing};

mod protocol;

const JOURNAL_SCHEMA: &str = "aster.local-runner-slot.v2";
const IDENTITY_SCHEMA: &str = "aster.runner-identity.v1";
const MAX_MATERIAL_BYTES: u64 = 4096;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema: String,
    installation_id: String,
    slot: ReleaseSlot,
    job_id: String,
    identity: Identity,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    schema: String,
    runner_id: String,
    credential: String,
}

impl Drop for Identity {
    fn drop(&mut self) {
        self.credential.zeroize();
    }
}

pub(super) struct PreparedSlot {
    pub identity: BootstrapLocalRunnerIdentity,
    // Keep the stable sidecar locked through DB registration and publication.
    _lock: File,
}

fn invalid() -> io::Error {
    io::Error::other("local Runner slot material is invalid or belongs to another installation")
}

pub(super) fn parse_slot(value: &str) -> Result<ReleaseSlot, String> {
    match value {
        "blue" => Ok(ReleaseSlot::Blue),
        "green" => Ok(ReleaseSlot::Green),
        _ => Err("local Runner slot must be blue or green".to_owned()),
    }
}

pub(super) fn parse_job_id(value: &str) -> Result<String, String> {
    if value.is_empty()
        || value.len() > 160
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err("invalid local Runner upgrade job ID".into());
    }
    Ok(value.into())
}

fn document(path: &Path) -> io::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 256 * 1024 {
        return Err(invalid());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o022 != 0 {
            return Err(invalid());
        }
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(256 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 256 * 1024 {
        return Err(invalid());
    }
    Ok(bytes)
}

pub(super) struct UpgradeContext {
    pub job: aster_upgrade_core::MaintenanceJob,
    pub previous: BootstrapLocalRunnerIdentity,
}

impl UpgradeContext {
    pub fn load(
        layout: &InstallLayout,
        job_id: &str,
        actor_id: &str,
        slot: ReleaseSlot,
    ) -> io::Result<Self> {
        use aster_upgrade_core::{
            ActiveReleaseSlot, MaintenanceJob, MaintenanceOperation, MaintenanceStatus,
        };
        parse_job_id(job_id).map_err(|_| invalid())?;
        let job: MaintenanceJob = serde_json::from_slice(&document(
            &layout.upgrade_running().join(format!("{job_id}.json")),
        )?)
        .map_err(|_| invalid())?;
        let active: ActiveReleaseSlot =
            serde_json::from_slice(&document(&layout.active_slot())?).map_err(|_| invalid())?;
        job.validate().map_err(|_| invalid())?;
        active.validate().map_err(|_| invalid())?;
        if job.id != job_id
            || job.requested_by != actor_id
            || job.status != MaintenanceStatus::StartingCandidate
            || !matches!(job.operation, MaintenanceOperation::Upgrade { .. })
            || active.slot == slot
            || active.version != job.current_version
            || job.target_version.as_deref() != Some(env!("CARGO_PKG_VERSION"))
        {
            return Err(invalid());
        }
        let candidate = job.candidate_release.as_ref().ok_or_else(invalid)?;
        if layout
            .release_binary(candidate, "aster-control")
            .canonicalize()?
            != std::env::current_exe()?.canonicalize()?
        {
            return Err(invalid());
        }
        // This candidate-side check also runs when an older installed CLI
        // initiated the online task. It precedes license activation, database
        // opens and all candidate credential/Runner writes in main.
        if job.upgrade_mode == Some(aster_upgrade_core::UpgradeMode::BlueGreen) {
            let keys = aster_control::compiled_release_keys().map_err(|_| invalid())?;
            protocol::require_pair(layout, &job, &active, &keys)?;
        }
        // The privileged executor copies the active Runner's credential into
        // a task-specific root-owned file readable by Control. Control must not
        // gain read access to the Runner service's whole configuration directory.
        let identity_path = layout
            .runner_slot_identity_output(slot.id())
            .with_extension(format!("{job_id}.previous.json"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            if fs::symlink_metadata(&identity_path)?.uid() != 0 {
                return Err(invalid());
            }
        }
        let material = Zeroizing::new(document(&identity_path)?);
        let previous: Identity = serde_json::from_slice(&material).map_err(|_| invalid())?;
        if previous.schema != IDENTITY_SCHEMA
            || active
                .local_runner
                .as_ref()
                .is_some_and(|runner| runner.runner_id != previous.runner_id)
        {
            return Err(invalid());
        }
        let previous = BootstrapLocalRunnerIdentity {
            runner_id: previous.runner_id.clone(),
            credential: Zeroizing::new(previous.credential.clone()),
        };
        previous.validate().map_err(|_| invalid())?;
        Ok(Self { job, previous })
    }
}

pub(super) struct FinalizationContext {
    pub survivor: Option<BootstrapLocalRunnerIdentity>,
    pub requires_binding: bool,
}

impl FinalizationContext {
    pub fn load(layout: &InstallLayout, job_id: &str, slot: ReleaseSlot) -> io::Result<Self> {
        use aster_upgrade_core::{ActiveReleaseSlot, MaintenanceJob};
        parse_job_id(job_id).map_err(|_| invalid())?;
        let job: MaintenanceJob = serde_json::from_slice(&document(
            &layout.upgrade_running().join(format!("{job_id}.json")),
        )?)
        .map_err(|_| invalid())?;
        let active: ActiveReleaseSlot =
            serde_json::from_slice(&document(&layout.active_slot())?).map_err(|_| invalid())?;
        if job.id != job_id {
            return Err(invalid());
        }
        let requires_binding = validate_finalization_topology(&job, &active, slot)?;
        let candidate = job.candidate_release.as_ref().ok_or_else(invalid)?;
        if layout
            .release_binary(candidate, "aster-control")
            .canonicalize()?
            != std::env::current_exe()?.canonicalize()?
        {
            return Err(invalid());
        }
        let identity_path = layout
            .runner_slot_identity_output(slot.id())
            .with_extension(format!("{job_id}.survivor.json"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            if fs::symlink_metadata(&identity_path)?.uid() != 0 {
                return Err(invalid());
            }
        }
        let material = Zeroizing::new(document(&identity_path)?);
        if material.len() > MAX_MATERIAL_BYTES as usize {
            return Err(invalid());
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct SurvivorInput {
            schema: String,
            survivor: Option<Identity>,
        }
        let input: SurvivorInput = serde_json::from_slice(&material).map_err(|_| invalid())?;
        if input.schema != "aster.runner-upgrade-survivor.v1" {
            return Err(invalid());
        }
        let survivor = if let Some(survivor) = input.survivor {
            if survivor.schema != IDENTITY_SCHEMA
                || active
                    .local_runner
                    .as_ref()
                    .is_some_and(|runner| runner.runner_id != survivor.runner_id)
            {
                return Err(invalid());
            }
            let identity = BootstrapLocalRunnerIdentity {
                runner_id: survivor.runner_id.clone(),
                credential: Zeroizing::new(survivor.credential.clone()),
            };
            identity.validate().map_err(|_| invalid())?;
            Some(identity)
        } else {
            if active.local_runner.is_some() {
                return Err(invalid());
            }
            None
        };
        Ok(Self {
            survivor,
            requires_binding,
        })
    }
}

fn validate_finalization_topology(
    job: &aster_upgrade_core::MaintenanceJob,
    active: &aster_upgrade_core::ActiveReleaseSlot,
    candidate_slot: ReleaseSlot,
) -> io::Result<bool> {
    job.validate().map_err(|_| invalid())?;
    active.validate().map_err(|_| invalid())?;
    if !job.status.terminal()
        || !matches!(
            job.operation,
            aster_upgrade_core::MaintenanceOperation::Upgrade { .. }
        )
        || job.target_version.as_deref() != Some(env!("CARGO_PKG_VERSION"))
    {
        return Err(invalid());
    }
    let candidate_active = job.target_version.as_deref() == Some(active.version.as_str());
    if (candidate_active && active.slot != candidate_slot)
        || (!candidate_active
            && (active.version != job.current_version || active.slot == candidate_slot))
    {
        return Err(invalid());
    }
    Ok(candidate_active && active.local_runner.is_some())
}

fn check_file(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(invalid());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(invalid());
        }
    }
    Ok(())
}

fn read_material(path: &Path) -> io::Result<Zeroizing<Vec<u8>>> {
    check_file(path)?;
    let mut value = Zeroizing::new(Vec::new());
    File::open(path)?
        .take(MAX_MATERIAL_BYTES + 1)
        .read_to_end(&mut value)?;
    if value.len() as u64 > MAX_MATERIAL_BYTES {
        return Err(invalid());
    }
    Ok(value)
}

fn sync_material(path: &Path) -> io::Result<()> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?
        .sync_all()?;
    #[cfg(unix)]
    File::open(path.parent().ok_or_else(invalid)?)?.sync_all()?;
    Ok(())
}

impl PreparedSlot {
    pub fn load_or_create(
        layout: &InstallLayout,
        installation_id: &str,
        slot: ReleaseSlot,
        job_id: &str,
    ) -> io::Result<Self> {
        Self::load(layout, installation_id, slot, job_id, true)
    }

    pub fn load_existing(
        layout: &InstallLayout,
        installation_id: &str,
        slot: ReleaseSlot,
        job_id: &str,
    ) -> io::Result<Self> {
        Self::load(layout, installation_id, slot, job_id, false)
    }

    fn load(
        layout: &InstallLayout,
        installation_id: &str,
        slot: ReleaseSlot,
        job_id: &str,
        create: bool,
    ) -> io::Result<Self> {
        parse_job_id(job_id).map_err(|_| invalid())?;
        // The installation owns these directories. Do not traverse substituted
        // journal parents or accept group/world-writable runtime directories.
        for directory in [layout.root().to_path_buf(), layout.data(), layout.runtime()] {
            if directory != layout.root() && !directory.try_exists()? {
                let builder = &mut fs::DirBuilder::new();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt as _;
                    builder.mode(0o700);
                }
                match builder.create(&directory) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error),
                }
                #[cfg(unix)]
                File::open(directory.parent().ok_or_else(invalid)?)?.sync_all()?;
            }
            let metadata = fs::symlink_metadata(directory)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(invalid());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                if metadata.permissions().mode() & 0o022 != 0 {
                    return Err(invalid());
                }
            }
        }
        let lock_path = layout.runner_slot_provisioning_lock(slot.id());
        match check_file(&lock_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let lock = options.open(&lock_path)?;
        check_file(&lock_path)?;
        lock.try_lock().map_err(|error| match error {
            fs::TryLockError::WouldBlock => io::Error::new(
                io::ErrorKind::WouldBlock,
                "local Runner slot provisioning is already running",
            ),
            fs::TryLockError::Error(error) => error,
        })?;
        let path = layout
            .runner_slot_provisioning(slot.id())
            .with_extension(format!("{job_id}.json"));
        let journal = match read_material(&path) {
            Ok(bytes) => serde_json::from_slice::<Journal>(&bytes).map_err(|_| invalid())?,
            Err(error) if error.kind() == io::ErrorKind::NotFound && create => {
                let identity = BootstrapLocalRunnerIdentity::generate().map_err(|_| invalid())?;
                let journal = Journal {
                    schema: JOURNAL_SCHEMA.to_owned(),
                    installation_id: installation_id.to_owned(),
                    slot,
                    job_id: job_id.into(),
                    identity: Identity {
                        schema: IDENTITY_SCHEMA.to_owned(),
                        runner_id: identity.runner_id.clone(),
                        credential: identity.credential.to_string(),
                    },
                };
                let encoded = Zeroizing::new(serde_json::to_vec(&journal).map_err(|_| invalid())?);
                super::atomic_write_new(&path, &encoded, 0o600)?;
                journal
            }
            Err(error) => return Err(error),
        };
        if journal.schema != JOURNAL_SCHEMA
            || journal.installation_id != installation_id
            || journal.job_id != job_id
            || journal.slot != slot
            || journal.identity.schema != IDENTITY_SCHEMA
        {
            return Err(invalid());
        }
        let identity = BootstrapLocalRunnerIdentity {
            runner_id: journal.identity.runner_id.clone(),
            credential: Zeroizing::new(journal.identity.credential.clone()),
        };
        identity.validate().map_err(|_| invalid())?;
        // Also sync an existing journal after an earlier interrupted fsync. No
        // database mutation is allowed until its durable material is confirmed.
        sync_material(&path)?;
        Ok(Self {
            identity,
            _lock: lock,
        })
    }

    pub fn publish(&self, output: &Path) -> io::Result<()> {
        match read_material(output) {
            Ok(bytes) => {
                let existing: Identity = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
                if existing.schema != IDENTITY_SCHEMA
                    || existing.runner_id != self.identity.runner_id
                    || existing.credential != *self.identity.credential
                {
                    return Err(invalid());
                }
                sync_material(output)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let value = Identity {
                    schema: IDENTITY_SCHEMA.to_owned(),
                    runner_id: self.identity.runner_id.clone(),
                    credential: self.identity.credential.to_string(),
                };
                let bytes = Zeroizing::new(serde_json::to_vec(&value).map_err(|_| invalid())?);
                super::atomic_write_new(output, &bytes, 0o600)
            }
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_before_registration_reuses_material_and_publication_is_idempotent() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(temporary.path()).unwrap();
        let prepared =
            PreparedSlot::load_or_create(&layout, "installation-a", ReleaseSlot::Blue, "job_one")
                .unwrap();
        let id = prepared.identity.runner_id.clone();
        let secret = prepared.identity.credential.clone();
        assert!(
            PreparedSlot::load_or_create(&layout, "installation-a", ReleaseSlot::Blue, "job_one")
                .is_err()
        );
        let green =
            PreparedSlot::load_or_create(&layout, "installation-a", ReleaseSlot::Green, "job_one")
                .unwrap();
        assert_ne!(green.identity.runner_id, id);
        drop(prepared);
        let resumed =
            PreparedSlot::load_or_create(&layout, "installation-a", ReleaseSlot::Blue, "job_one")
                .unwrap();
        assert_eq!(resumed.identity.runner_id, id);
        assert_eq!(resumed.identity.credential, secret);
        let output = temporary.path().join("published.json");
        resumed.publish(&output).unwrap();
        resumed.publish(&output).unwrap();
        assert!(green.publish(&output).is_err());
        let published: Identity = serde_json::from_slice(&read_material(&output).unwrap()).unwrap();
        assert_eq!(published.runner_id, id);
        assert_eq!(published.credential, *secret);
    }

    #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
    #[tokio::test]
    async fn resumes_after_database_commit_before_identity_output_is_published() {
        use aster_auth_core::AuthCore;
        use aster_control::{ControlState, ControlStorage};
        use std::sync::{Arc, Mutex};
        let temporary = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(temporary.path()).unwrap();
        let store = Arc::new(Mutex::new(
            aster_storage::SqlCipherStore::initialize(&temporary.path().join("slot.db"), &[81; 32])
                .unwrap(),
        ));
        let control = || {
            ControlState::new("2.0.1", None)
                .with_license_state(Arc::new(
                    aster_license_state::LicenseStateStore::new(
                        temporary.path().join("bootstrap-history.json"),
                        &[82; 32],
                    )
                    .unwrap(),
                ))
                .with_storage(ControlStorage::SqlCipher(Arc::clone(&store)))
                .with_auth_core(AuthCore::new(&[82; 32], "installation-recovery").unwrap())
        };
        control()
            .initialize_owner_identity(
                "owner@example.com",
                "Owner",
                Zeroizing::new(b"owner-password-strong".to_vec()),
            )
            .await
            .unwrap();
        let prepared = PreparedSlot::load_or_create(
            &layout,
            "installation-recovery",
            ReleaseSlot::Blue,
            "job_one",
        )
        .unwrap();
        control()
            .bootstrap_local_runner_with_identity(
                "owner@example.com",
                "local-runner",
                "linux",
                "x86_64",
                4,
                BootstrapLocalRunnerIdentity {
                    runner_id: prepared.identity.runner_id.clone(),
                    credential: prepared.identity.credential.clone(),
                },
            )
            .await
            .unwrap();
        let original_id = prepared.identity.runner_id.clone();
        let audit_count = store.lock().unwrap().audit_events().unwrap().len();
        drop(prepared);
        let output = temporary.path().join("identity-output.json");
        assert!(!output.exists());
        let resumed = PreparedSlot::load_or_create(
            &layout,
            "installation-recovery",
            ReleaseSlot::Blue,
            "job_one",
        )
        .unwrap();
        control()
            .bootstrap_local_runner_with_identity(
                "owner@example.com",
                "local-runner",
                "linux",
                "x86_64",
                4,
                BootstrapLocalRunnerIdentity {
                    runner_id: resumed.identity.runner_id.clone(),
                    credential: resumed.identity.credential.clone(),
                },
            )
            .await
            .unwrap();
        resumed.publish(&output).unwrap();
        let published: Identity = serde_json::from_slice(&read_material(&output).unwrap()).unwrap();
        assert_eq!(published.runner_id, original_id);
        assert_eq!(published.credential, *resumed.identity.credential);
        assert_eq!(store.lock().unwrap().list_runners().unwrap().len(), 1);
        assert_eq!(
            store.lock().unwrap().audit_events().unwrap().len(),
            audit_count
        );
    }

    #[test]
    fn finalization_uses_committed_topology_even_after_a_later_terminal_failure() {
        use aster_upgrade_core::*;
        let directory = tempfile::tempdir().unwrap();
        let mut job = MaintenanceJob {
            schema: MAINTENANCE_JOB_SCHEMA.into(),
            id: "upgrade_topology".into(),
            requested_by: "owner".into(),
            operation: MaintenanceOperation::Upgrade {
                archive: directory.path().join("candidate.tar.gz"),
                archive_sha256: "a".repeat(64),
            },
            status: MaintenanceStatus::Succeeded,
            upgrade_mode: Some(UpgradeMode::Maintenance),
            runner_was_running: None,
            current_version: "0.1.0".into(),
            target_version: Some(env!("CARGO_PKG_VERSION").into()),
            previous_release: None,
            candidate_release: None,
            message: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
        };
        for candidate_slot in [ReleaseSlot::Blue, ReleaseSlot::Green] {
            let mut active = ActiveReleaseSlot {
                schema: ACTIVE_SLOT_RUNTIME_SCHEMA.into(),
                slot: candidate_slot,
                version: env!("CARGO_PKG_VERSION").into(),
                local_runner: Some(ActiveLocalRunner {
                    runner_id: format!("runner_{}", "a".repeat(32)),
                    manifest_sha256: "b".repeat(64),
                }),
            };
            for status in [MaintenanceStatus::Succeeded, MaintenanceStatus::Failed] {
                job.status = status;
                assert!(validate_finalization_topology(&job, &active, candidate_slot).unwrap());
            }
            assert!(validate_finalization_topology(&job, &active, candidate_slot.other()).is_err());
            active.version = job.current_version.clone();
            active.slot = candidate_slot.other();
            assert!(!validate_finalization_topology(&job, &active, candidate_slot).unwrap());
            active.version = "99.0.0".into();
            assert!(validate_finalization_topology(&job, &active, candidate_slot).is_err());
            active.version = job.current_version.clone();
            job.status = MaintenanceStatus::SwitchingTraffic;
            assert!(validate_finalization_topology(&job, &active, candidate_slot).is_err());
        }
    }

    #[test]
    fn finalization_never_creates_replacement_material_for_a_missing_journal() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(temporary.path()).unwrap();
        assert!(
            PreparedSlot::load_existing(
                &layout,
                "installation-a",
                ReleaseSlot::Green,
                "upgrade_missing"
            )
            .is_err()
        );
        let path = layout
            .runner_slot_provisioning("green")
            .with_extension("upgrade_missing.json");
        assert!(!path.exists());
        let prepared = PreparedSlot::load_or_create(
            &layout,
            "installation-a",
            ReleaseSlot::Green,
            "upgrade_missing",
        )
        .unwrap();
        let id = prepared.identity.runner_id.clone();
        drop(prepared);
        let existing = PreparedSlot::load_existing(
            &layout,
            "installation-a",
            ReleaseSlot::Green,
            "upgrade_missing",
        )
        .unwrap();
        assert_eq!(existing.identity.runner_id, id);
    }

    #[test]
    fn separate_upgrade_jobs_never_reuse_retired_slot_credentials() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(temporary.path()).unwrap();
        let first =
            PreparedSlot::load_or_create(&layout, "installation-a", ReleaseSlot::Blue, "job_first")
                .unwrap();
        let first_id = first.identity.runner_id.clone();
        drop(first);
        let second = PreparedSlot::load_or_create(
            &layout,
            "installation-a",
            ReleaseSlot::Blue,
            "job_second",
        )
        .unwrap();
        assert_ne!(second.identity.runner_id, first_id);
        drop(second);
        let retry =
            PreparedSlot::load_or_create(&layout, "installation-a", ReleaseSlot::Blue, "job_first")
                .unwrap();
        assert_eq!(retry.identity.runner_id, first_id);
        for invalid in ["", "../job", "job/child", "job\\child", "job.json"] {
            assert!(parse_job_id(invalid).is_err());
        }
    }

    #[test]
    fn corrupt_or_foreign_journals_are_never_replaced() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(temporary.path()).unwrap();
        drop(
            PreparedSlot::load_or_create(&layout, "installation-a", ReleaseSlot::Blue, "job_one")
                .unwrap(),
        );
        let path = layout
            .runner_slot_provisioning("blue")
            .with_extension("job_one.json");
        let original = fs::read(&path).unwrap();
        assert!(
            PreparedSlot::load_or_create(&layout, "installation-b", ReleaseSlot::Blue, "job_one")
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), original);
        let green_path = layout
            .runner_slot_provisioning("green")
            .with_extension("job_one.json");
        super::super::atomic_write_new(&green_path, &original, 0o600).unwrap();
        assert!(
            PreparedSlot::load_or_create(&layout, "installation-a", ReleaseSlot::Green, "job_one")
                .is_err()
        );
        for bytes in [b"invalid".to_vec(), vec![b'x'; 4097]] {
            fs::write(&path, &bytes).unwrap();
            assert!(
                PreparedSlot::load_or_create(
                    &layout,
                    "installation-a",
                    ReleaseSlot::Blue,
                    "job_one"
                )
                .is_err()
            );
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_exposed_material_and_symlinked_journals() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};
        let temporary = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(temporary.path()).unwrap();
        drop(
            PreparedSlot::load_or_create(&layout, "installation-a", ReleaseSlot::Blue, "job_one")
                .unwrap(),
        );
        let path = layout
            .runner_slot_provisioning("blue")
            .with_extension("job_one.json");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            PreparedSlot::load_or_create(&layout, "installation-a", ReleaseSlot::Blue, "job_one")
                .is_err()
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let green_path = layout
            .runner_slot_provisioning("green")
            .with_extension("job_one.json");
        symlink(&path, &green_path).unwrap();
        assert!(
            PreparedSlot::load_or_create(&layout, "installation-a", ReleaseSlot::Green, "job_one")
                .is_err()
        );
    }
}
