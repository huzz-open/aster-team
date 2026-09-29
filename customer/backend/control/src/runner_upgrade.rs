use std::fs::{File, TryLockError};

use aster_install_layout::{InstallLayout, InstallLayoutError};

use super::{ControlError, ControlState};

/// Older CLIs already hold this exact lock throughout upgrade and recovery.
/// Keep its inode and hold the shared lease through the actual DB commit.
pub(super) fn registration_lease(state: &ControlState) -> Result<Option<File>, ControlError> {
    let Some(maintenance) = &state.maintenance else {
        return Ok(None);
    };
    let layout = &maintenance.layout;
    // A development root carries no installation marker, and an upgrade executor
    // only resolves an installed layout through that marker: without one no
    // executor can hold this root's maintenance lock, so there is no upgrade to
    // coordinate with and the registration needs no lease. Installed roots keep
    // every check below.
    if !layout.marker_path().is_file() {
        return Ok(None);
    }
    layout.read_marker().map_err(unavailable)?;
    let lease = File::open(layout.maintenance_lock()).map_err(unavailable)?;
    lease.try_lock_shared().map_err(|error| match error {
        TryLockError::WouldBlock => ControlError::MaintenanceBusy,
        TryLockError::Error(error) => unavailable(error),
    })?;
    // A crashed executor releases its lock before its job has been recovered.
    // Even a terminal job still here has not finished the existing CLI flow.
    if let Some(entry) = std::fs::read_dir(layout.upgrade_running())
        .map_err(unavailable)?
        .next()
    {
        entry.map_err(unavailable)?;
        return Err(ControlError::MaintenanceBusy);
    }
    let executable = std::env::current_exe()
        .map_err(unavailable)?
        .canonicalize()
        .map_err(unavailable)?;
    match InstallLayout::discover_from(&executable) {
        Ok(installed) => {
            if installed != *layout
                || layout
                    .release_binary(&layout.current(), "aster-control")
                    .canonicalize()
                    .map_err(unavailable)?
                    != executable
            {
                return Err(ControlError::MaintenanceBusy);
            }
        }
        // Explicitly configured development/test binaries are not installed
        // release executables. They still participate in the lock above.
        Err(InstallLayoutError::MarkerNotFound) => {}
        Err(error) => return Err(unavailable(error)),
    }
    Ok(Some(lease))
}

fn unavailable(error: impl std::fmt::Display) -> ControlError {
    ControlError::MaintenanceUnavailable(error.to_string())
}

/// Called by the existing privileged Linux identity preflight, including when
/// an OLD CLI starts a candidate package. Grant only the service group's read
/// access to the existing empty lock, without replacing or unlocking its inode.
#[cfg(target_os = "linux")]
pub fn prepare_runner_upgrade_lock() -> Result<(), Box<dyn std::error::Error>> {
    use rustix::process::{Gid, geteuid};

    if !geteuid().is_root() {
        return Ok(());
    }
    let layout = match InstallLayout::discover() {
        Ok(layout) => layout,
        Err(InstallLayoutError::MarkerNotFound) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let output = std::process::Command::new("/usr/bin/id")
        .args(["-g", "aster-team"])
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::other("Aster service group is unavailable").into());
    }
    let gid: u32 = std::str::from_utf8(&output.stdout)?.trim().parse()?;
    prepare_lock_file(&layout, Gid::from_raw(gid))
}

#[cfg(target_os = "linux")]
fn prepare_lock_file(
    layout: &InstallLayout,
    group: rustix::process::Gid,
) -> Result<(), Box<dyn std::error::Error>> {
    use rustix::fs::{Mode, OFlags, fchown, fstat, open, openat};
    use std::os::unix::fs::PermissionsExt as _;

    let directory = open(
        layout.staging(),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let fd = openat(
        &directory,
        "maintenance.lock",
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let file = File::from(fd);
    let metadata = file.metadata()?;
    let stat = fstat(&file)?;
    if !metadata.is_file() || metadata.len() != 0 || stat.st_nlink != 1 {
        return Err(std::io::Error::other("invalid maintenance lock file").into());
    }
    fchown(&file, None, Some(group))?;
    file.set_permissions(std::fs::Permissions::from_mode(0o640))?;
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    #[test]
    fn permission_repair_preserves_the_executors_locked_inode() {
        let directory = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(directory.path()).unwrap();
        std::fs::create_dir_all(layout.staging()).unwrap();
        let executor = std::fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(layout.maintenance_lock())
            .unwrap();
        executor
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .unwrap();
        executor.try_lock().unwrap();
        let before = executor.metadata().unwrap();
        prepare_lock_file(&layout, rustix::process::getegid()).unwrap();
        let reader = File::open(layout.maintenance_lock()).unwrap();
        let after = reader.metadata().unwrap();
        assert_eq!((after.dev(), after.ino()), (before.dev(), before.ino()));
        assert_eq!(after.permissions().mode() & 0o777, 0o640);
        assert!(matches!(
            reader.try_lock_shared(),
            Err(TryLockError::WouldBlock)
        ));
        drop(executor);
        reader.try_lock_shared().unwrap();
    }

    #[test]
    fn permission_repair_rejects_links_and_nonempty_files() {
        let directory = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(directory.path()).unwrap();
        std::fs::create_dir_all(layout.staging()).unwrap();
        let unrelated = directory.path().join("unrelated");
        std::fs::write(&unrelated, b"").unwrap();
        std::fs::set_permissions(&unrelated, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(&unrelated, layout.maintenance_lock()).unwrap();
        assert!(prepare_lock_file(&layout, rustix::process::getegid()).is_err());
        assert_eq!(
            std::fs::metadata(&unrelated).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::remove_file(layout.maintenance_lock()).unwrap();
        std::fs::hard_link(&unrelated, layout.maintenance_lock()).unwrap();
        assert!(prepare_lock_file(&layout, rustix::process::getegid()).is_err());
        std::fs::remove_file(layout.maintenance_lock()).unwrap();
        std::fs::write(layout.maintenance_lock(), b"unexpected").unwrap();
        assert!(prepare_lock_file(&layout, rustix::process::getegid()).is_err());
    }
}
