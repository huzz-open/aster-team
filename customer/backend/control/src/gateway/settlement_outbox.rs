//! Installation-local durable results shared by both Control slots.
//! The registry inode is permanent. Every open/unlink of a request lock occurs
//! under it, so a removed lock cannot leave a competing holder on an old inode.
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use aster_auth_core::{AuthCore, SecurityStateIntegrityInput};
use aster_install_layout::InstallLayout;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::durable_settlement::SettlementIntent;
use crate::ControlError;

const SCHEMA: &str = aster_upgrade_core::settlement::INTENT_SCHEMA;
const OWNER: &str = "aster.gateway-settlement-owner.v1";
const MAX_BYTES: u64 = 64 * 1024;

#[derive(Debug)]
pub(crate) struct Outbox {
    directory: PathBuf,
}

#[derive(Debug)]
pub(crate) struct RequestLock {
    store: Arc<Outbox>,
    key: String,
    file: Option<File>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Claims {
    schema: String,
    key: String,
    intent: SettlementIntent,
    completed: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    claims: Claims,
    mac: String,
}

fn invalid() -> ControlError {
    ControlError::DataIntegrityInvalid
}
fn io_error(_: std::io::Error) -> ControlError {
    ControlError::Io("settlement outbox I/O failed".into())
}
fn busy(error: fs::TryLockError) -> ControlError {
    match error {
        fs::TryLockError::WouldBlock => ControlError::QuotaSettlementConflict,
        fs::TryLockError::Error(error) => io_error(error),
    }
}
fn request_key(request_id: &str) -> String {
    let encoded = serde_json::to_vec(&(
        aster_upgrade_core::settlement::REQUEST_LOCK_SCHEMA,
        request_id,
    ))
    .expect("string tuple is serializable");
    crate::hex_encode(&Sha256::digest(encoded))
}
fn valid_key(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn check(path: &Path, directory: bool) -> Result<(), ControlError> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if metadata.file_type().is_symlink()
        || if directory {
            !metadata.is_dir()
        } else {
            !metadata.is_file()
        }
    {
        return Err(invalid());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        if metadata.permissions().mode() & if directory { 0o022 } else { 0o077 } != 0
            || (!directory && metadata.nlink() != 1)
        {
            return Err(invalid());
        }
    }
    Ok(())
}

fn sync_directory(directory: &Path) -> Result<(), ControlError> {
    #[cfg(unix)]
    File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(io_error)?;
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
}

fn read(path: &Path) -> Result<Option<Vec<u8>>, ControlError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(error)),
        Ok(_) => check(path, false)?,
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(io_error)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(invalid());
    }
    Ok(Some(bytes))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ControlError> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err(invalid());
    }
    let parent = path.parent().ok_or_else(invalid)?;
    check(parent, true)?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".intent-")
        .tempfile_in(parent)
        .map_err(io_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(io_error)?;
    }
    temporary
        .write_all(bytes)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(io_error)?;
    #[cfg(windows)]
    {
        let (file, temporary_path) = temporary.keep().map_err(|_| invalid())?;
        file.sync_all().map_err(io_error)?;
        drop(file);
        let result = atomicwrites::replace_atomic(&temporary_path, path);
        if result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        result.map_err(io_error)?;
    }
    #[cfg(not(windows))]
    temporary.persist(path).map_err(|_| invalid())?;
    sync_directory(parent)
}

fn open_lock(path: &Path) -> Result<File, ControlError> {
    match fs::symlink_metadata(path) {
        Ok(_) => check(path, false)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_error(error)),
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(io_error)?;
    check(path, false)?;
    Ok(file)
}

impl Outbox {
    pub(crate) fn open(layout: &InstallLayout, auth: &AuthCore) -> Result<Arc<Self>, ControlError> {
        check(layout.root(), true)?;
        for directory in [layout.data(), layout.settlement_outbox()] {
            let builder = fs::DirBuilder::new();
            #[cfg(unix)]
            let builder = {
                use std::os::unix::fs::DirBuilderExt as _;
                let mut builder = builder;
                builder.mode(0o700);
                builder
            };
            match builder.create(&directory) {
                Ok(()) => sync_directory(directory.parent().ok_or_else(invalid)?)?,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(io_error(error)),
            }
            check(&directory, true)?;
        }
        let store = Arc::new(Self {
            directory: fs::canonicalize(layout.settlement_outbox()).map_err(io_error)?,
        });
        let _registry = store.registry()?;
        let owner = store.directory.join("installation.json");
        let binding = SecurityStateIntegrityInput {
            key: OWNER,
            value: SCHEMA.as_bytes(),
            revision: 0,
        };
        match read(&owner)? {
            Some(bytes) => {
                let tag = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
                if !auth
                    .verify_security_state_integrity_hmac(binding, tag)
                    .map_err(ControlError::Auth)?
                {
                    return Err(invalid());
                }
            }
            None => {
                if !store.page("", 1)?.is_empty() {
                    return Err(invalid());
                }
                write_atomic(
                    &owner,
                    auth.security_state_integrity_hmac(binding)
                        .map_err(ControlError::Auth)?
                        .as_bytes(),
                )?;
            }
        }
        Ok(store)
    }

    fn try_registry(&self) -> Result<File, ControlError> {
        check(&self.directory, true)?;
        let file = open_lock(&self.directory.join("registry.lock"))?;
        file.try_lock().map_err(busy)?;
        Ok(file)
    }

    // Registry sections only coordinate file opens/unlinks. Brief contention
    // from unrelated requests must not become an admission failure. This runs
    // on the blocking pool; request ownership itself remains try-only.
    fn registry(&self) -> Result<File, ControlError> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match self.try_registry() {
                Err(ControlError::QuotaSettlementConflict) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                result => return result,
            }
        }
    }

    pub(crate) fn acquire(
        self: &Arc<Self>,
        request_id: &str,
    ) -> Result<Option<Arc<RequestLock>>, ControlError> {
        self.acquire_key(&request_key(request_id))
    }

    pub(crate) fn acquire_key(
        self: &Arc<Self>,
        key: &str,
    ) -> Result<Option<Arc<RequestLock>>, ControlError> {
        if !valid_key(key) {
            return Err(invalid());
        }
        let _registry = self.registry()?;
        let file = open_lock(&self.directory.join(format!("{key}.lock")))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Arc::new(RequestLock {
                store: Arc::clone(self),
                key: key.into(),
                file: Some(file),
            }))),
            Err(fs::TryLockError::WouldBlock) => {
                drop(file);
                Ok(None)
            }
            Err(error) => Err(busy(error)),
        }
    }

    /// Keep bounded memory even when many completed/abandoned requests exist.
    /// Both records and orphan locks are candidates; actual contents are checked
    /// only after acquiring their request lock. A live owner is skipped.
    pub(crate) fn page(&self, after: &str, limit: usize) -> Result<Vec<String>, ControlError> {
        check(&self.directory, true)?;
        let limit = limit.clamp(1, 128);
        let mut keys = BTreeSet::new();
        for entry in fs::read_dir(&self.directory).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let name = entry.file_name();
            let name = name.to_str().ok_or_else(invalid)?;
            if name == "installation.json"
                || name == "registry.lock"
                || name.starts_with(".intent-")
            {
                continue;
            }
            let key = name
                .strip_suffix(".json")
                .or_else(|| name.strip_suffix(".lock"))
                .ok_or_else(invalid)?;
            if !valid_key(key) {
                return Err(invalid());
            }
            if key > after {
                keys.insert(key.to_owned());
                if keys.len() > limit {
                    keys.pop_last();
                }
            }
        }
        Ok(keys.into_iter().collect())
    }
}

impl RequestLock {
    pub(crate) fn matches(&self, store: &Outbox, request_id: &str) -> bool {
        self.file.is_some()
            && self.store.directory == store.directory
            && self.key == request_key(request_id)
    }
    fn path(&self) -> PathBuf {
        self.store.directory.join(format!("{}.json", self.key))
    }

    fn load_claims(&self, auth: &AuthCore) -> Result<Option<Claims>, ControlError> {
        let Some(bytes) = read(&self.path())? else {
            return Ok(None);
        };
        let value: Envelope = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        let claims = &value.claims;
        let source = serde_json::to_vec(claims).map_err(|_| invalid())?;
        if claims.schema != SCHEMA
            || claims.key != self.key
            || request_key(&claims.intent.reservation.request_id) != self.key
            || !claims.intent.valid()
            || !auth
                .verify_security_state_integrity_hmac(
                    SecurityStateIntegrityInput {
                        key: SCHEMA,
                        value: &source,
                        revision: 0,
                    },
                    &value.mac,
                )
                .map_err(ControlError::Auth)?
        {
            return Err(invalid());
        }
        Ok(Some(value.claims))
    }

    pub(crate) fn load(&self, auth: &AuthCore) -> Result<Option<SettlementIntent>, ControlError> {
        Ok(self.load_claims(auth)?.map(|claims| claims.intent))
    }

    fn write(&self, auth: &AuthCore, claims: Claims) -> Result<(), ControlError> {
        let source = serde_json::to_vec(&claims).map_err(|_| invalid())?;
        let mac = auth
            .security_state_integrity_hmac(SecurityStateIntegrityInput {
                key: SCHEMA,
                value: &source,
                revision: 0,
            })
            .map_err(ControlError::Auth)?;
        write_atomic(
            &self.path(),
            &serde_json::to_vec(&Envelope { claims, mac }).map_err(|_| invalid())?,
        )
    }

    pub(crate) fn publish(
        &self,
        auth: &AuthCore,
        intent: &SettlementIntent,
    ) -> Result<(), ControlError> {
        if !intent.valid() || request_key(&intent.reservation.request_id) != self.key {
            return Err(invalid());
        }
        if let Some(existing) = self.load_claims(auth)? {
            if existing.intent != *intent {
                return Err(invalid());
            }
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(self.path())
                .and_then(|file| file.sync_all())
                .map_err(io_error)?;
            return sync_directory(&self.store.directory);
        }
        self.write(
            auth,
            Claims {
                schema: SCHEMA.into(),
                key: self.key.clone(),
                intent: intent.clone(),
                completed: false,
            },
        )
    }

    pub(crate) fn finish(
        &self,
        auth: &AuthCore,
        intent: &SettlementIntent,
    ) -> Result<(), ControlError> {
        let Some(mut claims) = self.load_claims(auth)? else {
            // Deletion may have succeeded before its directory-sync response
            // failed. The caller has already authenticated database completion.
            return sync_directory(&self.store.directory);
        };
        if claims.intent != *intent {
            return Err(invalid());
        }
        if !claims.completed {
            claims.completed = true;
            self.write(auth, claims)?;
        }
        fs::remove_file(self.path()).map_err(io_error)?;
        sync_directory(&self.store.directory)
    }
}

impl Drop for RequestLock {
    fn drop(&mut self) {
        // If the short registry lease is busy, leave an orphan for the scanner.
        // Never unlink without it: another process may already have this inode.
        let registry = self.store.try_registry();
        drop(self.file.take());
        if let Ok(_registry) = registry {
            let _ = fs::remove_file(self.store.directory.join(format!("{}.lock", self.key)));
        }
    }
}

/// Existing systemd units run the candidate's identity preflight as root,
/// including when the installed CLI predates the settlement outbox.
#[cfg(target_os = "linux")]
pub fn prepare_settlement_outbox() -> Result<(), Box<dyn std::error::Error>> {
    use aster_install_layout::InstallLayoutError;
    use rustix::process::{Gid, Uid, geteuid};
    if !geteuid().is_root() {
        return Ok(());
    }
    let layout = match InstallLayout::discover() {
        Ok(layout) => layout,
        Err(InstallLayoutError::MarkerNotFound) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let account_id = |option: &str| -> Result<u32, Box<dyn std::error::Error>> {
        let output = std::process::Command::new("/usr/bin/id")
            .args([option, "aster-team"])
            .output()?;
        if !output.status.success() {
            return Err(std::io::Error::other("Aster service account is unavailable").into());
        }
        Ok(std::str::from_utf8(&output.stdout)?.trim().parse()?)
    };
    prepare_directory(
        &layout,
        Uid::from_raw(account_id("-u")?),
        Gid::from_raw(account_id("-g")?),
    )
}

#[cfg(target_os = "linux")]
fn prepare_directory(
    layout: &InstallLayout,
    user: rustix::process::Uid,
    group: rustix::process::Gid,
) -> Result<(), Box<dyn std::error::Error>> {
    use rustix::fs::{Mode, OFlags, fchmod, fchown, fsync, mkdirat, open, openat};
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let root = open(layout.root(), flags, Mode::empty())?;
    let data = openat(&root, "data", flags, Mode::empty())?;
    let name = layout.settlement_outbox();
    let name = name
        .file_name()
        .ok_or_else(|| std::io::Error::other("invalid outbox path"))?;
    match mkdirat(&data, name, Mode::RWXU) {
        Ok(()) | Err(rustix::io::Errno::EXIST) => {}
        Err(error) => return Err(error.into()),
    }
    let directory = openat(&data, name, flags, Mode::empty())?;
    fchown(&directory, Some(user), Some(group))?;
    fchmod(&directory, Mode::RWXU)?;
    fsync(&directory)?;
    fsync(&data)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ModelUsage, QuotaReservation, gateway::durable_settlement::Outcome};

    fn auth() -> AuthCore {
        AuthCore::new(&[75; 32], "installation_outbox_test").unwrap()
    }
    fn intent() -> SettlementIntent {
        SettlementIntent::new(
            &QuotaReservation {
                id: "quota_test".into(),
                identity_id: "identity_test".into(),
                request_id: "killed-request".into(),
                client_request_id: None,
                reserved_tokens: 1,
                billing: None,
                _money_permit: None,
                execution: None,
            },
            Outcome::Usage(ModelUsage {
                uncached_input: 1,
                cached_input: 2,
                cache_write: 3,
                output_tokens: 4,
                multiplier_micros: 1_000_000,
                protocol: "openai_responses".into(),
                model: "example".into(),
                requested_model: None,
                processing_tier: None,
                reasoning_effort: None,
                runner_id: "runner_test".into(),
            }),
        )
    }
    fn fixture() -> (tempfile::TempDir, Arc<Outbox>) {
        let root = tempfile::tempdir().unwrap();
        let store = Outbox::open(
            &InstallLayout::new(root.path().to_path_buf()).unwrap(),
            &auth(),
        )
        .unwrap();
        (root, store)
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn installed_directory_preparation_keeps_parent_permissions_and_live_lock_inode() {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        let (root, store) = fixture();
        let layout = InstallLayout::new(root.path()).unwrap();
        let parent_before = fs::metadata(layout.data()).unwrap();
        let owner = store.acquire("active").unwrap().unwrap();
        let path = store
            .directory
            .join(format!("{}.lock", request_key("active")));
        let inode = fs::metadata(&path).unwrap().ino();
        prepare_directory(
            &layout,
            rustix::process::geteuid(),
            rustix::process::getegid(),
        )
        .unwrap();
        assert_eq!(
            fs::metadata(layout.data()).unwrap().permissions().mode(),
            parent_before.permissions().mode()
        );
        assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
        assert!(store.acquire("active").unwrap().is_none());
        assert_eq!(
            fs::metadata(layout.settlement_outbox())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        drop(owner);
        assert!(store.acquire("active").unwrap().is_some());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn installed_directory_preparation_rejects_a_symlink_without_changing_its_target() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(root.path()).unwrap();
        fs::create_dir(layout.data()).unwrap();
        let mode = fs::metadata(outside.path()).unwrap().permissions().mode();
        symlink(outside.path(), layout.settlement_outbox()).unwrap();
        assert!(
            prepare_directory(
                &layout,
                rustix::process::geteuid(),
                rustix::process::getegid()
            )
            .is_err()
        );
        assert_eq!(
            fs::metadata(outside.path()).unwrap().permissions().mode(),
            mode
        );
    }

    #[test]
    fn brief_registry_contention_waits_without_rejecting_an_unrelated_request() {
        let (_root, store) = fixture();
        // Use production acquisition for setup too: concurrent subprocess
        // creation can briefly retain a pre-exec FD even with CLOEXEC.
        let registry = store.registry().unwrap();
        let (started, waiting) = std::sync::mpsc::channel();
        let (completed, result) = std::sync::mpsc::channel();
        let other = Arc::clone(&store);
        let worker = std::thread::spawn(move || {
            started.send(()).unwrap();
            let lock = other.acquire("other");
            completed.send(lock).unwrap();
        });
        waiting.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(
            result.recv_timeout(Duration::from_millis(30)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        drop(registry);
        assert!(
            result
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .unwrap()
                .is_some()
        );
        worker.join().unwrap();
    }

    #[test]
    fn last_shared_owner_releases_one_inode_and_other_requests_remain_independent() {
        let (_root, store) = fixture();
        let first = store.acquire("a").unwrap().unwrap();
        let shared = Arc::clone(&first);
        let registry = store.directory.join("registry.lock");
        assert!(store.acquire("a").unwrap().is_none());
        assert!(store.acquire("b").unwrap().is_some());
        drop(first);
        assert!(store.acquire("a").unwrap().is_none());
        drop(shared);
        assert!(
            !store
                .directory
                .join(format!("{}.lock", request_key("a")))
                .exists()
        );
        assert!(registry.is_file());
        assert!(store.acquire("a").unwrap().is_some());
    }

    #[test]
    fn immutable_authenticated_intent_rejects_replacement_and_other_installation() {
        let (root, store) = fixture();
        let owner = store.acquire("killed-request").unwrap().unwrap();
        let original = intent();
        owner.publish(&auth(), &original).unwrap();
        let bytes = fs::read(owner.path()).unwrap();
        owner.publish(&auth(), &original).unwrap();
        let mut changed = original.clone();
        changed.reservation.client_request_id = Some("replacement".into());
        assert!(owner.publish(&auth(), &changed).is_err());
        assert_eq!(fs::read(owner.path()).unwrap(), bytes);
        assert_eq!(owner.load(&auth()).unwrap(), Some(original));
        let wrong = AuthCore::new(&[75; 32], "another_installation").unwrap();
        assert!(
            Outbox::open(
                &InstallLayout::new(root.path().to_path_buf()).unwrap(),
                &wrong
            )
            .is_err()
        );
        assert!(owner.load(&wrong).is_err());
    }

    #[test]
    fn completed_file_and_lost_cleanup_ack_are_recoverable_without_rewriting_intent() {
        let (_root, store) = fixture();
        let owner = store.acquire("killed-request").unwrap().unwrap();
        let expected = intent();
        owner.publish(&auth(), &expected).unwrap();
        let mut claims = owner.load_claims(&auth()).unwrap().unwrap();
        claims.completed = true;
        owner.write(&auth(), claims).unwrap();
        assert_eq!(owner.load(&auth()).unwrap(), Some(expected.clone()));
        owner.finish(&auth(), &expected).unwrap();
        owner.finish(&auth(), &expected).unwrap();
        assert!(!owner.path().exists());
    }

    #[test]
    fn crashed_owner_releases_os_lock_and_preserves_synced_intent() {
        use std::{
            process::{Child, Command, Stdio},
            time::{Duration, Instant},
        };
        struct Guard(Child);
        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let (root, store) = fixture();
        let inherited = store.acquire("parent-only").unwrap().unwrap();
        let ready = root.path().join("child.ready");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "gateway::settlement_outbox::tests::request_lock_child_process",
                "--ignored",
            ])
            .env("ASTER_OUTBOX_CHILD_DIRECTORY", root.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            command.creation_flags(0x0800_0000);
        }
        let mut child = Guard(command.spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(15);
        while !ready.is_file() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "lock helper exited before fsync"
            );
            assert!(
                Instant::now() < deadline,
                "lock helper did not publish its ready marker"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(child.0.try_wait().unwrap().is_none());
        assert!(store.acquire("killed-request").unwrap().is_none());
        drop(inherited);
        // A subprocess must not inherit and retain the parent's request handle.
        assert!(store.acquire("parent-only").unwrap().is_some());
        child.0.kill().unwrap();
        assert!(!child.0.wait().unwrap().success());
        let recovered = store.acquire("killed-request").unwrap().unwrap();
        assert_eq!(recovered.load(&auth()).unwrap(), Some(intent()));
        recovered.finish(&auth(), &intent()).unwrap();
    }

    #[test]
    #[ignore = "helper spawned by crashed_owner_releases_os_lock_and_preserves_synced_intent"]
    fn request_lock_child_process() {
        let root = PathBuf::from(
            std::env::var_os("ASTER_OUTBOX_CHILD_DIRECTORY").expect("parent fixture is required"),
        );
        let store = Outbox::open(&InstallLayout::new(root.clone()).unwrap(), &auth()).unwrap();
        let owner = store.acquire("killed-request").unwrap().unwrap();
        owner.publish(&auth(), &intent()).unwrap();
        fs::write(root.join("child.ready"), b"synced").unwrap();
        loop {
            std::thread::park();
        }
    }
}
