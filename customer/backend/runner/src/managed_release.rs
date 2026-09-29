use std::path::{Path, PathBuf};

use aster_install_layout::{InstallLayout, InstallLayoutError};

use super::RunnerFailure;

/// Only an executable within a discovered installation follows its `current`
/// pointer. Standalone remote Runners do not inspect the platform default root.
pub(super) struct ManagedRelease {
    executable: PathBuf,
    selected_executable: PathBuf,
}

impl ManagedRelease {
    pub(super) fn discover(
        slot: Option<aster_upgrade_core::ReleaseSlot>,
    ) -> Result<Option<Self>, RunnerFailure> {
        let executable = std::env::current_exe()
            .map_err(|_| RunnerFailure::new("runner_executable_unavailable"))?;
        Self::discover_selected(&executable, slot)
    }

    #[cfg(test)]
    fn discover_from(executable: &Path) -> Result<Option<Self>, RunnerFailure> {
        Self::discover_selected(executable, None)
    }

    fn discover_selected(
        executable: &Path,
        slot: Option<aster_upgrade_core::ReleaseSlot>,
    ) -> Result<Option<Self>, RunnerFailure> {
        let executable = executable
            .canonicalize()
            .map_err(|_| RunnerFailure::new("runner_executable_unavailable"))?;
        let layout = match InstallLayout::discover_from(&executable) {
            Ok(layout) => layout,
            Err(InstallLayoutError::MarkerNotFound) => return Ok(None),
            Err(_) => return Err(RunnerFailure::new("runner_installation_invalid")),
        };
        let releases = layout
            .releases()
            .canonicalize()
            .map_err(|_| RunnerFailure::new("runner_installation_invalid"))?;
        let Some(release) = executable.parent().and_then(Path::parent) else {
            return Err(RunnerFailure::new("runner_installation_invalid"));
        };
        if release.parent() != Some(releases.as_path())
            || layout.release_binary(release, "aster-runner") != executable
        {
            return Err(RunnerFailure::new("runner_installation_invalid"));
        }
        Ok(Some(Self {
            executable,
            selected_executable: layout.release_binary(
                &slot.map_or_else(|| layout.current(), |slot| layout.slot_release(slot.id())),
                "aster-runner",
            ),
        }))
    }

    pub(super) fn require_selected(&self) -> Result<(), RunnerFailure> {
        let selected = self
            .selected_executable
            .canonicalize()
            .map_err(|_| RunnerFailure::new("runner_selected_release_unavailable"))?;
        if selected != self.executable {
            return Err(RunnerFailure::new("runner_release_changed"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link_directory(target: &Path, link: &Path) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link)
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_dir(target, link)
        }
    }

    #[test]
    fn managed_candidate_detects_rollback_before_reconnecting() {
        let directory = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(directory.path()).unwrap();
        std::fs::write(layout.marker_path(), layout.marker_json().unwrap()).unwrap();
        let old = layout.release("2.0.0");
        let candidate = layout.release("2.1.0");
        for release in [&old, &candidate] {
            std::fs::create_dir_all(release.join("bin")).unwrap();
            std::fs::write(layout.release_binary(release, "aster-runner"), b"fixture").unwrap();
        }
        if let Err(error) = link_directory(&candidate, &layout.current()) {
            if cfg!(windows) && error.raw_os_error() == Some(1314) {
                eprintln!("skipping symlink rollback test without Windows symlink privileges");
                return;
            }
            panic!("failed to create release pointer: {error}");
        }
        let guard = ManagedRelease::discover_from(
            &layout.release_binary(&layout.current(), "aster-runner"),
        )
        .unwrap()
        .unwrap();
        guard.require_selected().unwrap();
        #[cfg(unix)]
        std::fs::remove_file(layout.current()).unwrap();
        #[cfg(windows)]
        std::fs::remove_dir(layout.current()).unwrap();
        // Missing/transient pointers never silently switch to unmanaged mode.
        assert!(guard.require_selected().is_err());
        link_directory(&old, &layout.current()).unwrap();
        assert_eq!(
            guard.require_selected().unwrap_err().category,
            "runner_release_changed"
        );
        ManagedRelease::discover_from(&layout.release_binary(&layout.current(), "aster-runner"))
            .unwrap()
            .unwrap()
            .require_selected()
            .unwrap();
    }

    #[test]
    fn standalone_runner_does_not_use_an_unrelated_installation() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("runner");
        std::fs::write(&executable, b"fixture").unwrap();
        assert!(
            ManagedRelease::discover_from(&executable)
                .unwrap()
                .is_none()
        );
        let layout = InstallLayout::new(directory.path()).unwrap();
        std::fs::write(layout.marker_path(), b"invalid").unwrap();
        assert!(ManagedRelease::discover_from(&executable).is_err());
        std::fs::write(layout.marker_path(), layout.marker_json().unwrap()).unwrap();
        std::fs::create_dir_all(layout.releases()).unwrap();
        assert!(ManagedRelease::discover_from(&executable).is_err());
    }
}
