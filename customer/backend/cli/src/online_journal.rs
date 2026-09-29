//! Online transition persistence. The executor also holds its installation lock;
//! this sidecar lock serializes journal writers across atomic file replacement.
use std::{
    fs::{self, File, OpenOptions},
    io::Read as _,
    path::{Path, PathBuf},
};

use aster_error_catalog::delivery;
use aster_install_layout::InstallLayout;
use aster_upgrade_core::online::{OnlineJournal, OnlineJournalStorage};

use super::CliFailure;

mod preparation;
mod switchback;
#[cfg(test)]
pub(crate) use preparation::tests::fixture as preparation_fixture;

const JOURNAL_NAME: &str = "online-transition.json";
const MAX_JOURNAL_BYTES: u64 = 256 * 1024;

pub(super) struct JournalFile {
    path: PathBuf,
    root: PathBuf,
    _lock: File,
}

#[cfg(unix)]
impl Drop for JournalFile {
    fn drop(&mut self) {
        // flock belongs to the open file description. A descriptor inherited
        // during fork can outlive this owner until exec; closing our File alone
        // would then retain its lock. Release ownership explicitly. If unlock
        // fails, closing the File remains the conservative cleanup fallback.
        let _ = self._lock.unlock();
    }
}

impl JournalFile {
    #[cfg(any(target_os = "linux", test))]
    pub(super) fn pending(layout: &InstallLayout) -> Result<bool, CliFailure> {
        plain_directories(&layout.upgrade_state(), layout.root())?;
        Ok(plain_file(&layout.upgrade_state().join(JOURNAL_NAME))?
            || plain_file(&layout.upgrade_state().join(switchback::NAME))?)
    }

    /// The caller has already durably archived the matching maintenance job.
    /// Never overwrite another history record or archive an unfinished cutover.
    #[cfg(any(target_os = "linux", test))]
    pub(super) fn archive(&mut self, journal: &OnlineJournal) -> Result<(), CliFailure> {
        use aster_upgrade_core::online::OnlinePhase;
        if journal.phase() != OnlinePhase::Complete {
            return Err(invalid("unfinished online journal cannot be archived"));
        }
        self.assert_current(journal)?;
        let directory = self.root.join("state/upgrades/online-completed");
        plain_directories(&directory, &self.root)?;
        fs::create_dir_all(&directory).map_err(filesystem)?;
        let destination = directory.join(format!("{}.json", journal.plan().job_id));
        match fs::symlink_metadata(&destination) {
            Ok(_) => {
                return Err(invalid(
                    "an online history record already has this task identity",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(filesystem(error)),
        }
        fs::rename(&self.path, &destination).map_err(filesystem)?;
        #[cfg(unix)]
        for parent in [
            directory.as_path(),
            self.path
                .parent()
                .ok_or_else(|| invalid("journal has no parent"))?,
        ] {
            File::open(parent)
                .and_then(|file| file.sync_all())
                .map_err(filesystem)?;
        }
        Ok(())
    }

    /// Call only while holding the installation maintenance lock. The sidecar
    /// stays locked for the lifetime of this store, including proxy mutations.
    pub(super) fn open(layout: &InstallLayout) -> Result<Self, CliFailure> {
        let parent = layout.upgrade_state();
        plain_directories(&parent, layout.root())?;
        fs::create_dir_all(&parent).map_err(filesystem)?;
        plain_directories(&parent, layout.root())?;
        let lock_path = parent.join("online-transition.lock");
        plain_file(&lock_path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let lock = options.open(&lock_path).map_err(filesystem)?;
        plain_file(&lock_path)?;
        lock.try_lock().map_err(|error| match error {
            fs::TryLockError::WouldBlock => CliFailure::new(
                delivery::MAINTENANCE_BUSY,
                "another executor owns the online transition journal",
            ),
            fs::TryLockError::Error(error) => filesystem(error),
        })?;
        Ok(Self {
            path: parent.join(JOURNAL_NAME),
            root: layout.root().to_path_buf(),
            _lock: lock,
        })
    }
}

impl OnlineJournalStorage for JournalFile {
    type Error = CliFailure;

    fn load(&self) -> Result<Option<OnlineJournal>, Self::Error> {
        plain_directories(
            self.path
                .parent()
                .ok_or_else(|| invalid("journal has no parent"))?,
            &self.root,
        )?;
        if !plain_file(&self.path)? {
            return Ok(None);
        }
        let file = File::open(&self.path).map_err(filesystem)?;
        let mut bytes = Vec::new();
        file.take(MAX_JOURNAL_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(filesystem)?;
        if bytes.len() as u64 > MAX_JOURNAL_BYTES {
            return Err(invalid("online journal is too large"));
        }
        let journal: OnlineJournal = serde_json::from_slice(&bytes)
            .map_err(|_| invalid("online journal cannot be decoded"))?;
        if !journal.valid() {
            return Err(invalid("online journal violates transition invariants"));
        }
        Ok(Some(journal))
    }

    fn assert_current(&mut self, journal: &OnlineJournal) -> Result<(), Self::Error> {
        self.ensure_forward_owned()?;
        if !journal.valid() || self.load()?.as_ref() != Some(journal) {
            return Err(invalid(
                "online journal changed; reload durable state before continuing",
            ));
        }
        Ok(())
    }

    fn save(
        &mut self,
        previous: Option<&OnlineJournal>,
        next: &OnlineJournal,
    ) -> Result<(), Self::Error> {
        // Production online execution is Linux amd64 only. Windows tests exercise
        // serialization/CAS, not Linux rename/fsync or power-loss guarantees.
        #[cfg(not(any(all(target_os = "linux", target_arch = "x86_64"), test)))]
        {
            let _ = (previous, next);
            Err(invalid(
                "online journal writes are unsupported on this platform",
            ))
        }
        #[cfg(any(all(target_os = "linux", target_arch = "x86_64"), test))]
        {
            self.ensure_forward_owned()?;
            if !next.follows(previous) {
                return Err(invalid("online journal successor is invalid"));
            }
            if previous.is_none()
                && self
                    .load_preparation()?
                    .is_some_and(|record| !record.permits_handoff(next.plan()))
            {
                return Err(invalid(
                    "online journal does not match the prepared candidate",
                ));
            }
            if self.load()?.as_ref() != previous {
                return Err(invalid(
                    "online journal compare-and-swap failed; reload before continuing",
                ));
            }
            let mut bytes = serde_json::to_vec_pretty(next)
                .map_err(|_| invalid("online journal cannot be encoded"))?;
            bytes.push(b'\n');
            if bytes.len() as u64 > MAX_JOURNAL_BYTES {
                return Err(invalid("online journal is too large"));
            }
            // The Unix helper writes, fsyncs, renames and syncs the parent without
            // spawning a chown command or inheriting ownership from old data.
            #[cfg(unix)]
            super::atomic_write_new_file(&self.path, &bytes, 0o600)?;
            #[cfg(target_os = "windows")]
            super::maintenance_executor::atomic_replace(self.path.clone(), &bytes, 0o600)?;
            // A failed sync or readback is an uncertain write; never restore the
            // previous journal and thereby erase evidence of a possible action.
            self.assert_current(next)
        }
    }
}

pub(super) fn ensure_maintenance_allowed(layout: &InstallLayout) -> Result<(), CliFailure> {
    plain_directories(&layout.upgrade_state(), layout.root())?;
    if plain_file(&layout.upgrade_state().join(switchback::NAME))? {
        return Err(invalid(
            "online switchback must be reconciled before maintenance can run",
        ));
    }
    if plain_file(&layout.upgrade_state().join(preparation::NAME))? {
        let store = JournalFile::open(layout)?;
        let record = store
            .load_preparation()?
            .ok_or_else(|| invalid("online preparation disappeared"))?;
        return Err(invalid(format!(
            "online preparation {} must be reconciled before maintenance can run",
            record.intent().job.id,
        )));
    }
    if !plain_file(&layout.upgrade_state().join(JOURNAL_NAME))? {
        return Ok(());
    }
    let store = JournalFile::open(layout)?;
    let journal = store
        .load()?
        .ok_or_else(|| invalid("online journal disappeared during recovery"))?;
    #[cfg(target_os = "linux")]
    {
        use aster_upgrade_core::runtime::UpgradeClockSource as _;
        let clock = super::upgrade_clock::LinuxUpgradeClock::sample()?;
        if clock.boot_id != journal.plan().clock.boot_id {
            return Err(invalid(
                "online upgrade belongs to a previous machine boot; preserve its evidence for online recovery",
            ));
        }
    }
    // Even Complete must be reconciled with active pointers and archived by the
    // online executor first. Maintenance cannot infer completion from a phase.
    Err(invalid(format!(
        "online upgrade {} is at {:?}; online recovery must reconcile and archive it before maintenance can run",
        journal.plan().job_id,
        journal.phase(),
    )))
}

fn plain_directories(path: &Path, root: &Path) -> Result<(), CliFailure> {
    if !path.starts_with(root) {
        return Err(invalid("online journal escaped the installation"));
    }
    for component in path.ancestors() {
        match fs::symlink_metadata(component) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Err(invalid("online journal parent is not a plain directory")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(filesystem(error)),
        }
        if component == root {
            break;
        }
    }
    Ok(())
}

fn plain_file(path: &Path) -> Result<bool, CliFailure> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(filesystem(error)),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(invalid("online journal or lock is not a plain file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.mode() & 0o077 != 0 || metadata.nlink() != 1 {
            return Err(invalid(
                "online journal or lock must have private permissions and one link",
            ));
        }
    }
    Ok(true)
}

fn invalid(message: impl Into<String>) -> CliFailure {
    CliFailure::new(delivery::UPGRADE_FAILED, message)
}

fn filesystem(error: std::io::Error) -> CliFailure {
    CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string())
}

#[cfg(test)]
pub(crate) mod tests;
