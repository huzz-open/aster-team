//! Shares the transition sidecar lock: preparing a candidate and handing it to
//! the transition journal cannot race a second executor in this installation.
use super::*;
use aster_upgrade_core::preparation::PreparationJournal;

pub(super) const NAME: &str = "online-preparation.json";

impl JournalFile {
    pub(crate) fn load_preparation(&self) -> Result<Option<PreparationJournal>, CliFailure> {
        self.read_preparation(&self.root.join("state/upgrades").join(NAME))
    }

    fn read_preparation(&self, path: &Path) -> Result<Option<PreparationJournal>, CliFailure> {
        plain_directories(
            path.parent()
                .ok_or_else(|| invalid("preparation has no parent"))?,
            &self.root,
        )?;
        if !plain_file(path)? {
            return Ok(None);
        }
        let mut bytes = Vec::new();
        File::open(path)
            .map_err(filesystem)?
            .take(MAX_JOURNAL_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(filesystem)?;
        if bytes.len() as u64 > MAX_JOURNAL_BYTES {
            return Err(invalid("online preparation is too large"));
        }
        let journal: PreparationJournal = serde_json::from_slice(&bytes)
            .map_err(|_| invalid("online preparation cannot be decoded"))?;
        if !journal.valid() {
            return Err(invalid("online preparation violates identity invariants"));
        }
        Ok(Some(journal))
    }

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn save_preparation(
        &mut self,
        previous: Option<&PreparationJournal>,
        next: &PreparationJournal,
    ) -> Result<(), CliFailure> {
        self.ensure_forward_owned()?;
        if self.load()?.is_some() {
            return Err(invalid("a transition already owns this installation"));
        }
        if !next.follows(previous) || self.load_preparation()?.as_ref() != previous {
            return Err(invalid("online preparation compare-and-swap failed"));
        }
        let mut bytes = serde_json::to_vec_pretty(next)
            .map_err(|_| invalid("cannot encode online preparation"))?;
        bytes.push(b'\n');
        if bytes.len() as u64 > MAX_JOURNAL_BYTES {
            return Err(invalid("online preparation is too large"));
        }
        let path = self.root.join("state/upgrades").join(NAME);
        #[cfg(unix)]
        crate::atomic_write_new_file(&path, &bytes, 0o600)?;
        #[cfg(target_os = "windows")]
        crate::maintenance_executor::atomic_replace(path, &bytes, 0o600)?;
        if self.load_preparation()?.as_ref() != Some(next) {
            return Err(invalid("online preparation write outcome is uncertain"));
        }
        Ok(())
    }

    /// The caller must also validate the current/completed task before handing
    /// off. Keep the evidence until the exact transition is durably readable.
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn archive_preparation(&mut self, online: &OnlineJournal) -> Result<(), CliFailure> {
        self.assert_current(online)?;
        let Some(preparation) = self.load_preparation()? else {
            return Ok(());
        };
        if !preparation.permits_handoff(online.plan()) {
            return Err(invalid("online transition does not match its preparation"));
        }
        let directory = self.root.join("state/upgrades/online-prepared");
        plain_directories(&directory, &self.root)?;
        fs::create_dir_all(&directory).map_err(filesystem)?;
        let destination = directory.join(format!("{}.json", preparation.intent().job.id));
        match fs::symlink_metadata(&destination) {
            Ok(_) => return Err(invalid("preparation history already exists")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(filesystem(error)),
        }
        let path = self.root.join("state/upgrades").join(NAME);
        fs::rename(&path, &destination).map_err(filesystem)?;
        #[cfg(unix)]
        for parent in [
            directory.as_path(),
            path.parent()
                .ok_or_else(|| invalid("preparation has no parent"))?,
        ] {
            File::open(parent)
                .and_then(|file| file.sync_all())
                .map_err(filesystem)?;
        }
        Ok(())
    }
}

impl JournalFile {
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn archive_cancelled_preparation(
        &mut self,
        journal: &PreparationJournal,
    ) -> Result<(), CliFailure> {
        use aster_upgrade_core::preparation::PreparationAbortPhase;
        self.ensure_forward_owned()?;
        if !journal.valid()
            || journal.abort_phase() != Some(PreparationAbortPhase::Complete)
            || self.load()?.is_some()
            || self.load_preparation()?.as_ref() != Some(journal)
        {
            return Err(invalid(
                "cancelled preparation cannot be archived before completion",
            ));
        }
        let directory = self.root.join("state/upgrades/preparation-cancelled");
        plain_directories(&directory, &self.root)?;
        fs::create_dir_all(&directory).map_err(filesystem)?;
        let history = directory.join(format!("{}.json", journal.intent().job.id));
        match self.read_preparation(&history)? {
            Some(saved) if &saved == journal => {}
            Some(_) => return Err(invalid("cancelled preparation history conflicts")),
            None => {
                let mut bytes = serde_json::to_vec_pretty(journal)
                    .map_err(|_| invalid("cannot encode cancelled preparation"))?;
                bytes.push(b'\n');
                #[cfg(unix)]
                crate::atomic_write_new_file(&history, &bytes, 0o600)?;
                #[cfg(target_os = "windows")]
                crate::maintenance_executor::atomic_replace(history.clone(), &bytes, 0o600)?;
            }
        }
        if self.read_preparation(&history)?.as_ref() != Some(journal) {
            return Err(invalid("cancelled history readback changed"));
        }
        #[cfg(unix)]
        for path in [
            history.as_path(),
            directory.as_path(),
            directory
                .parent()
                .ok_or_else(|| invalid("history has no parent"))?,
        ] {
            File::open(path)
                .and_then(|file| file.sync_all())
                .map_err(filesystem)?;
        }
        let path = self.root.join("state/upgrades").join(NAME);
        fs::remove_file(&path).map_err(filesystem)?;
        #[cfg(unix)]
        File::open(
            path.parent()
                .ok_or_else(|| invalid("preparation has no parent"))?,
        )
        .and_then(|file| file.sync_all())
        .map_err(filesystem)?;
        Ok(())
    }
}

#[cfg(test)]
pub(super) mod tests;
