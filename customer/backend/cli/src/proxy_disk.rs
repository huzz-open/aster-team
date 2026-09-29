//! Persist installer-managed upstreams before the live proxy mutation. Errors
//! retain whichever disk/live phase was reached; never reload or roll back here.
use std::{fs, io::Read as _, path::PathBuf, time::Instant};

use aster_error_catalog::delivery;
use aster_install_layout::InstallLayout;
use aster_upgrade_core::ReleaseSlot;

use super::CliFailure;

pub(super) struct InstalledUpstreams {
    layout: InstallLayout,
    path: PathBuf,
}

fn failed() -> CliFailure {
    CliFailure::new(
        delivery::UPGRADE_FAILED,
        "installed Caddy upstreams are unsafe, changed or not durably confirmed; preserve the online transition",
    )
}

impl InstalledUpstreams {
    pub(super) fn open(layout: &InstallLayout) -> Result<Self, CliFailure> {
        let value = Self {
            layout: layout.clone(),
            path: layout.caddy_upstreams(),
        };
        value.read()?;
        Ok(value)
    }

    fn contents(&self, slot: ReleaseSlot) -> String {
        super::maintenance_executor::render_upstreams(
            slot,
            aster_install_layout::WindowsPorts::default(),
        )
    }

    fn read(&self) -> Result<(Vec<u8>, fs::Metadata), CliFailure> {
        if !self.path.starts_with(self.layout.root()) {
            return Err(failed());
        }
        for path in self.path.parent().ok_or_else(failed)?.ancestors() {
            let metadata = fs::symlink_metadata(path).map_err(|_| failed())?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(failed());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt as _;
                if metadata.mode() & 0o022 != 0 {
                    return Err(failed());
                }
            }
            if path == self.layout.root() {
                break;
            }
        }
        let metadata = fs::symlink_metadata(&self.path).map_err(|_| failed())?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 8192 {
            return Err(failed());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            if metadata.mode() & 0o7777 != 0o640 || metadata.nlink() != 1 {
                return Err(failed());
            }
        }
        let mut bytes = Vec::new();
        fs::File::open(&self.path)
            .and_then(|file| file.take(8193).read_to_end(&mut bytes))
            .map_err(|_| failed())?;
        if bytes.len() > 8192
            || ![ReleaseSlot::Blue, ReleaseSlot::Green]
                .into_iter()
                .any(|slot| self.contents(slot).as_bytes() == bytes)
        {
            return Err(failed());
        }
        Ok((bytes, metadata))
    }

    pub(super) fn observe(&self, deadline: Instant) -> Result<ReleaseSlot, CliFailure> {
        if Instant::now() >= deadline {
            return Err(failed());
        }
        let bytes = self.read()?.0;
        let slot = if bytes == self.contents(ReleaseSlot::Blue).as_bytes() {
            ReleaseSlot::Blue
        } else {
            ReleaseSlot::Green
        };
        if Instant::now() >= deadline {
            return Err(failed());
        }
        Ok(slot)
    }

    pub(super) fn assert_target(
        &self,
        target: ReleaseSlot,
        deadline: Instant,
    ) -> Result<(), CliFailure> {
        if Instant::now() >= deadline {
            return Err(failed());
        }
        if self.read()?.0 != self.contents(target).as_bytes() || Instant::now() >= deadline {
            return Err(failed());
        }
        Ok(())
    }

    /// Idempotent after a lost write acknowledgement. Production callers hold
    /// the installation lock throughout this write and the following proxy CAS.
    pub(super) fn prepare(&self, target: ReleaseSlot, deadline: Instant) -> Result<(), CliFailure> {
        if Instant::now() >= deadline {
            return Err(failed());
        }
        let (previous, metadata) = self.read()?;
        let next = self.contents(target);
        if previous == next.as_bytes() {
            // A previous rename may have succeeded while parent fsync failed.
            // Matching bytes alone cannot turn that uncertain write durable.
            #[cfg(unix)]
            {
                fs::File::open(&self.path)
                    .and_then(|file| file.sync_all())
                    .map_err(|_| failed())?;
                fs::File::open(self.path.parent().ok_or_else(failed)?)
                    .and_then(|parent| parent.sync_all())
                    .map_err(|_| failed())?;
            }
            return self.assert_target(target, deadline);
        }
        #[cfg(not(any(all(target_os = "linux", target_arch = "x86_64"), test)))]
        {
            let _ = metadata;
            return Err(failed());
        }
        #[cfg(any(all(target_os = "linux", target_arch = "x86_64"), test))]
        {
            #[cfg(unix)]
            self.replace(&previous, next.as_bytes(), &metadata, deadline)?;
            #[cfg(target_os = "windows")]
            {
                let _ = metadata;
                // Format/CAS fixture only; this is not Linux durability evidence.
                if self.read()?.0 != previous || Instant::now() >= deadline {
                    return Err(failed());
                }
                super::maintenance_executor::atomic_replace(
                    self.path.clone(),
                    next.as_bytes(),
                    0o640,
                )?;
            }
            self.assert_target(target, deadline)
        }
    }

    #[cfg(unix)]
    fn replace(
        &self,
        previous: &[u8],
        next: &[u8],
        metadata: &fs::Metadata,
        deadline: Instant,
    ) -> Result<(), CliFailure> {
        use std::{
            io::Write as _,
            os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _, chown},
        };
        let parent = self.path.parent().ok_or_else(failed)?;
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).map_err(|_| failed())?;
        let suffix: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
        let temporary = parent.join(format!(".online-upstreams-{suffix}.tmp"));
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o640)
            .open(&temporary)
            .map_err(|_| failed())?;
        struct Temporary(PathBuf);
        impl Drop for Temporary {
            fn drop(&mut self) {
                let _ = fs::remove_file(&self.0);
            }
        }
        let _temporary = Temporary(temporary.clone());
        chown(&temporary, Some(metadata.uid()), Some(metadata.gid())).map_err(|_| failed())?;
        file.set_permissions(fs::Permissions::from_mode(0o640))
            .map_err(|_| failed())?;
        file.write_all(next)
            .and_then(|()| file.sync_all())
            .map_err(|_| failed())?;
        let (actual, current_metadata) = self.read()?;
        if actual != previous
            || current_metadata.ino() != metadata.ino()
            || current_metadata.dev() != metadata.dev()
            || current_metadata.uid() != metadata.uid()
            || current_metadata.gid() != metadata.gid()
            || Instant::now() >= deadline
        {
            return Err(failed());
        }
        fs::rename(&temporary, &self.path).map_err(|_| failed())?;
        fs::File::open(parent)
            .and_then(|parent| parent.sync_all())
            .map_err(|_| failed())?;
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests;
