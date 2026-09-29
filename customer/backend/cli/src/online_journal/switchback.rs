//! One sidecar owns both journals. A durable reverse journal freezes forward
//! execution even if its first write acknowledgement or later readback is lost.
use super::*;
use aster_upgrade_core::online::switchback::{
    SwitchbackJournal, SwitchbackJournalStorage, SwitchbackPhase,
};

pub(super) const NAME: &str = "online-switchback.json";

impl JournalFile {
    pub(super) fn ensure_forward_owned(&self) -> Result<(), CliFailure> {
        if plain_file(&self.root.join("state/upgrades").join(NAME))? {
            return Err(invalid("switchback owns the original online transition"));
        }
        Ok(())
    }
}

impl JournalFile {
    fn switchback_history(&self, journal: &SwitchbackJournal) -> PathBuf {
        self.root
            .join("state/upgrades/switchback-completed")
            .join(format!("{}.json", journal.plan().original.plan().job_id))
    }

    fn read_switchback(&self, path: &Path) -> Result<Option<SwitchbackJournal>, CliFailure> {
        plain_directories(
            path.parent()
                .ok_or_else(|| invalid("switchback has no parent"))?,
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
            return Err(invalid("switchback journal is too large"));
        }
        let journal: SwitchbackJournal = serde_json::from_slice(&bytes)
            .map_err(|_| invalid("switchback journal cannot be decoded"))?;
        if !journal.valid() {
            return Err(invalid("switchback journal violates transition invariants"));
        }
        Ok(Some(journal))
    }

    /// The matching failed maintenance job must already be archived. Keep the
    /// reverse guard until deleting the frozen original has been made durable.
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn archive_switchback(
        &mut self,
        journal: &SwitchbackJournal,
    ) -> Result<(), CliFailure> {
        if journal.phase() != SwitchbackPhase::Complete {
            return Err(invalid("unfinished switchback cannot be archived"));
        }
        SwitchbackJournalStorage::assert_current(self, journal)?;
        let history = self.switchback_history(journal);
        let directory = history
            .parent()
            .ok_or_else(|| invalid("history has no parent"))?;
        plain_directories(directory, &self.root)?;
        fs::create_dir_all(directory).map_err(filesystem)?;
        #[cfg(unix)]
        File::open(
            directory
                .parent()
                .ok_or_else(|| invalid("history has no ancestor"))?,
        )
        .and_then(|file| file.sync_all())
        .map_err(filesystem)?;
        match self.read_switchback(&history)? {
            Some(existing) if &existing == journal => {}
            Some(_) => return Err(invalid("switchback history belongs to another outcome")),
            None => {
                let mut bytes = serde_json::to_vec_pretty(journal)
                    .map_err(|_| invalid("cannot encode switchback history"))?;
                bytes.push(b'\n');
                #[cfg(unix)]
                crate::atomic_write_new_file(&history, &bytes, 0o600)?;
                #[cfg(target_os = "windows")]
                crate::maintenance_executor::atomic_replace(history.clone(), &bytes, 0o600)?;
            }
        }
        if self.read_switchback(&history)?.as_ref() != Some(journal) {
            return Err(invalid("switchback history readback changed"));
        }
        // A previous attempt may have renamed this exact history file but
        // lost the directory-sync acknowledgement. Reading it is not proof of
        // durability; resync the file and its directory before either deletion.
        #[cfg(unix)]
        for path in [history.as_path(), directory] {
            File::open(path)
                .and_then(|file| file.sync_all())
                .map_err(filesystem)?;
        }
        if let Some(original) = OnlineJournalStorage::load(self)? {
            if original != journal.plan().original {
                return Err(invalid("frozen forward journal changed"));
            }
            fs::remove_file(&self.path).map_err(filesystem)?;
        }
        #[cfg(unix)]
        File::open(
            self.path
                .parent()
                .ok_or_else(|| invalid("journal has no parent"))?,
        )
        .and_then(|file| file.sync_all())
        .map_err(filesystem)?;
        // load() can now recover Complete from this exact history bundle.
        SwitchbackJournalStorage::assert_current(self, journal)?;
        let path = self.root.join("state/upgrades").join(NAME);
        fs::remove_file(&path).map_err(filesystem)?;
        #[cfg(unix)]
        File::open(
            path.parent()
                .ok_or_else(|| invalid("switchback has no parent"))?,
        )
        .and_then(|file| file.sync_all())
        .map_err(filesystem)?;
        Ok(())
    }
}

impl SwitchbackJournalStorage for JournalFile {
    type Error = CliFailure;

    fn load(&self) -> Result<Option<SwitchbackJournal>, Self::Error> {
        let path = self.root.join("state/upgrades").join(NAME);
        let Some(journal) = self.read_switchback(&path)? else {
            return Ok(None);
        };
        match OnlineJournalStorage::load(self)? {
            Some(original) if original == journal.plan().original => {}
            None if journal.phase() == SwitchbackPhase::Complete
                && self
                    .read_switchback(&self.switchback_history(&journal))?
                    .as_ref()
                    == Some(&journal) => {}
            _ => return Err(invalid("switchback lost its original forward journal")),
        }
        Ok(Some(journal))
    }

    fn assert_current(&mut self, journal: &SwitchbackJournal) -> Result<(), Self::Error> {
        if !journal.valid() || SwitchbackJournalStorage::load(self)?.as_ref() != Some(journal) {
            return Err(invalid("switchback changed; reload its durable state"));
        }
        Ok(())
    }

    fn save(
        &mut self,
        previous: Option<&SwitchbackJournal>,
        next: &SwitchbackJournal,
    ) -> Result<(), Self::Error> {
        #[cfg(not(any(all(target_os = "linux", target_arch = "x86_64"), test)))]
        {
            let _ = (previous, next);
            Err(invalid(
                "switchback journal writes are unsupported on this platform",
            ))
        }
        #[cfg(any(all(target_os = "linux", target_arch = "x86_64"), test))]
        {
            if !next.follows(previous)
                || SwitchbackJournalStorage::load(self)?.as_ref() != previous
                || OnlineJournalStorage::load(self)?.as_ref() != Some(&next.plan().original)
                || self.load_preparation()?.is_some()
            {
                return Err(invalid(
                    "switchback compare-and-swap or original ownership failed",
                ));
            }
            let path = self.root.join("state/upgrades").join(NAME);
            let mut bytes = serde_json::to_vec_pretty(next)
                .map_err(|_| invalid("switchback cannot be encoded"))?;
            bytes.push(b'\n');
            if bytes.len() as u64 > MAX_JOURNAL_BYTES {
                return Err(invalid("switchback journal is too large"));
            }
            #[cfg(unix)]
            crate::atomic_write_new_file(&path, &bytes, 0o600)?;
            #[cfg(target_os = "windows")]
            crate::maintenance_executor::atomic_replace(path, &bytes, 0o600)?;
            SwitchbackJournalStorage::assert_current(self, next)
        }
    }
}

#[cfg(test)]
mod tests;
