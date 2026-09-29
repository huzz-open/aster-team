//! Windows tar materializes junctions. Extract only ordinary files, verify with
//! the installed trust store, and return references to rebuild at the final root.
use std::{
    collections::BTreeSet,
    fs::{self, File},
    path::{Path, PathBuf},
};

use aster_install_layout::InstallLayout;
use flate2::read::GzDecoder;
use serde::Serialize;

use crate::{
    CliFailure, SELECTED_RELEASE_SCHEMA, SelectedRelease, delivery, sha256_file, verify_release_at,
    verify_selected_release_at,
};

fn failure(message: impl ToString) -> CliFailure {
    CliFailure::new(delivery::BACKUP_FAILED, message.to_string())
}

#[derive(Serialize)]
pub(crate) struct RestorePlan {
    references: Vec<ReleaseReference>,
}

#[derive(Serialize)]
struct ReleaseReference {
    path: String,
    version: String,
}

fn same_windows_path(left: &Path, right: &Path) -> bool {
    fn key(path: &Path) -> String {
        let path = path.to_string_lossy().replace('/', "\\");
        if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{unc}")
        } else {
            path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
        }
    }
    // canonicalize() records \\?\ paths in selected-release.json. Do not
    // resolve against live files: the selected version may only be in backup.
    key(left).eq_ignore_ascii_case(&key(right))
}

// Apply Windows rules on every host so the hostile-archive tests also run on Linux.
fn safe_components(name: &str) -> Result<Vec<&str>, CliFailure> {
    let components: Vec<_> = name.trim_end_matches('/').split('/').collect();
    for part in &components {
        let base = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if part.is_empty()
            || *part == "."
            || *part == ".."
            || part.ends_with(['.', ' '])
            || part
                .chars()
                .any(|c| c.is_control() || "\\:<>\"|?*".contains(c))
            || matches!(
                base.as_str(),
                "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
            )
            || ["COM", "LPT"].iter().any(|prefix| {
                base.strip_prefix(prefix).is_some_and(|suffix| {
                    matches!(
                        suffix,
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                    )
                })
            })
        {
            return Err(failure(format!(
                "backup contains an unsafe Windows path: {name}"
            )));
        }
    }
    Ok(components)
}

fn extract(source: &Path, destination: &Path, root_name: &str) -> Result<PathBuf, CliFailure> {
    if fs::read_dir(destination).map_err(failure)?.next().is_some() {
        return Err(failure("backup extraction requires an empty destination"));
    }
    let mut archive = tar::Archive::new(GzDecoder::new(File::open(source).map_err(failure)?));
    let mut seen = BTreeSet::new();
    let mut total = 0_u64;
    for entry in archive.entries().map_err(failure)? {
        let mut entry = entry.map_err(failure)?;
        let kind = entry.header().entry_type();
        if !kind.is_file() && !kind.is_dir() {
            return Err(failure("backup contains a link or unsupported entry type"));
        }
        let raw = entry.path_bytes();
        let name = std::str::from_utf8(&raw).map_err(failure)?;
        let components = safe_components(name)?;
        if components[0] != root_name
            || (components.len() == 1 && !kind.is_dir())
            || (components.len() > 1
                && ![
                    "bin",
                    "releases",
                    "config",
                    "data",
                    "state",
                    "logs",
                    "current",
                    "install.json",
                    "backups",
                    "staging",
                ]
                .contains(&components[1]))
            || (components.len() > 1
                && ["backups", "staging"].contains(&components[1])
                && (components.len() > 2 || !kind.is_dir()))
        {
            return Err(failure(format!(
                "backup contains an unexpected path: {name}"
            )));
        }
        if !seen.insert(name.trim_end_matches('/').to_lowercase()) {
            return Err(failure(format!(
                "backup contains a duplicate Windows path: {name}"
            )));
        }
        total = total.saturating_add(entry.size());
        // Backups include customer databases, unlike the smaller release limits.
        if seen.len() > 1_000_000 || total > 1024 * 1024 * 1024 * 1024 {
            return Err(failure(
                "backup exceeds 1,000,000 entries or 1 TiB expanded size",
            ));
        }
        if !entry.unpack_in(destination).map_err(failure)? {
            return Err(failure(
                "backup attempted to escape the extraction directory",
            ));
        }
    }
    if seen.is_empty() {
        return Err(failure("backup archive is empty"));
    }
    Ok(destination.join(root_name))
}

pub(crate) fn prepare(
    source: &Path,
    destination: &Path,
    root: &Path,
    runner_only: bool,
) -> Result<RestorePlan, CliFailure> {
    let layout = InstallLayout::new(root).map_err(failure)?;
    let root_name = root
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| failure("invalid installation root"))?;
    // The caller may only prepare a private per-restore directory, never live data.
    let restores = root.join("staging/restores");
    let relative = destination.strip_prefix(&restores).map_err(failure)?;
    let parts: Vec<_> = relative.components().collect();
    if parts.len() != 2
        || parts[1].as_os_str() != "prepared"
        || !parts[0]
            .as_os_str()
            .to_str()
            .is_some_and(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(failure("invalid restore workspace"));
    }
    // Refuse a pre-existing junction in any staging ancestor.
    let mut ancestor = destination.to_path_buf();
    loop {
        let metadata = fs::symlink_metadata(&ancestor).map_err(failure)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(failure(
                "restore workspace contains a link or non-directory",
            ));
        }
        if ancestor == root {
            break;
        }
        if !ancestor.pop() {
            return Err(failure("restore workspace escapes installation"));
        }
    }
    let prepared = extract(source, destination, root_name)?;
    layout
        .verify_marker_bytes(&fs::read(prepared.join("install.json")).map_err(failure)?)
        .map_err(failure)?;
    let archived_marker: aster_install_layout::InstallMarker =
        serde_json::from_slice(&fs::read(prepared.join("install.json")).map_err(failure)?)
            .map_err(failure)?;
    if archived_marker.windows_instance.unwrap_or_default()
        != layout.windows_instance().map_err(failure)?
    {
        return Err(failure(
            "backup Windows instance settings do not match this installation",
        ));
    }
    let role = if runner_only { "runner" } else { "control" };
    let other = if runner_only { "control" } else { "runner" };
    if fs::read_to_string(prepared.join(format!("config/{role}/install-role")))
        .map_err(failure)?
        .trim()
        != role
        || prepared
            .join(format!("config/{other}/install-role"))
            .exists()
    {
        return Err(failure("backup role does not match this installation"));
    }
    validate_releases(&prepared, root, runner_only, |path| {
        let release = if runner_only {
            verify_selected_release_at(path)?
        } else {
            verify_release_at(path)?
        };
        let version = release.claims().version.clone();
        #[cfg(target_os = "windows")]
        super::verify_windows_instance_support(&layout, path)?;
        Ok(version)
    })
}

fn validate_releases(
    prepared: &Path,
    root: &Path,
    runner_only: bool,
    verify: impl Fn(&Path) -> Result<String, CliFailure>,
) -> Result<RestorePlan, CliFailure> {
    let mut releases = BTreeSet::new();
    for entry in fs::read_dir(prepared.join("releases")).map_err(failure)? {
        let entry = entry.map_err(failure)?;
        let version = entry
            .file_name()
            .into_string()
            .map_err(|_| failure("invalid release name"))?;
        semver::Version::parse(&version).map_err(failure)?;
        if !entry.file_type().map_err(failure)?.is_dir() || verify(&entry.path())? != version {
            return Err(failure(
                "backup release directory does not match its signed version",
            ));
        }
        releases.insert(version);
    }
    let mut references = Vec::new();
    for path in [
        "current",
        "state/slots/blue-release",
        "state/slots/green-release",
    ] {
        let reference = prepared.join(path);
        if path != "current" && !reference.exists() {
            continue;
        }
        let version = verify(&reference)?;
        if !releases.contains(&version)
            || sha256_file(&reference.join("RELEASE.json"))?
                != sha256_file(
                    &prepared
                        .join("releases")
                        .join(&version)
                        .join("RELEASE.json"),
                )?
        {
            return Err(failure(format!(
                "backup release reference is inconsistent: {path}"
            )));
        }
        references.push(ReleaseReference {
            path: path.to_owned(),
            version,
        });
    }
    let current = &references[0].version;
    if !runner_only {
        let active: aster_upgrade_core::ActiveReleaseSlot = serde_json::from_slice(
            &fs::read(prepared.join("state/slots/active.json")).map_err(failure)?,
        )
        .map_err(failure)?;
        let slot_path = format!("state/slots/{}-release", active.slot.id());
        if active.schema != "aster.active-release-slot.v1"
            || &active.version != current
            || !references
                .iter()
                .any(|item| item.path == slot_path && item.version == active.version)
        {
            return Err(failure(
                "backup active slot and current release are inconsistent",
            ));
        }
    }
    let selection: SelectedRelease = serde_json::from_slice(
        &fs::read(prepared.join("state/selected-release.json")).map_err(failure)?,
    )
    .map_err(failure)?;
    if selection.schema != SELECTED_RELEASE_SCHEMA
        || !releases.contains(&selection.version)
        || !same_windows_path(
            &selection.release_root,
            &root.join("releases").join(&selection.version),
        )
        || selection.manifest_sha256
            != sha256_file(
                &prepared
                    .join("releases")
                    .join(&selection.version)
                    .join("RELEASE.json"),
            )?
    {
        return Err(failure("backup selected release is inconsistent"));
    }
    // Restore executable service assets only from the verified release tree.
    let release = prepared.join("releases").join(current);
    for (source, target) in [
        ("bin/aster-team-cli.exe", "bin/aster-team-cli.exe"),
        ("bin/caddy.exe", "bin/caddy.exe"),
        (
            "windows/service-launch.ps1",
            "config/services/service-launch.ps1",
        ),
    ] {
        fs::copy(release.join(source), prepared.join(target)).map_err(failure)?;
    }
    // Only ordinary, verified copies are removed. PowerShell rebuilds junctions
    // after the move, pointing at root/releases rather than temporary staging.
    for reference in &references {
        fs::remove_dir_all(prepared.join(&reference.path)).map_err(failure)?;
    }
    Ok(RestorePlan { references })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn selected_paths_accept_canonical_windows_prefixes_but_not_other_roots() {
        assert!(same_windows_path(
            Path::new(r"\\?\D:\Aster Team\releases\2.0.1"),
            Path::new(r"d:\Aster Team\releases\2.0.1")
        ));
        assert!(same_windows_path(
            Path::new(r"\\?\UNC\server\share\releases\2.0.1"),
            Path::new(r"\\server\share\releases\2.0.1")
        ));
        assert!(!same_windows_path(
            Path::new(r"\\?\D:\other\releases\2.0.1"),
            Path::new(r"D:\Aster Team\releases\2.0.1")
        ));
        assert!(!same_windows_path(
            Path::new(r"D:\Aster Team\releases\..\2.0.1"),
            Path::new(r"D:\Aster Team\releases\2.0.1")
        ));
    }

    fn fixture(prepared: &Path, live: &Path, runner: bool) {
        for reference in [
            "releases/2.0.1",
            "current",
            "state/slots/blue-release",
            "state/slots/green-release",
        ] {
            for file in [
                "VERSION",
                "RELEASE.json",
                "bin/aster-team-cli.exe",
                "bin/caddy.exe",
                "windows/service-launch.ps1",
            ] {
                let path = prepared.join(reference).join(file);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, "2.0.1").unwrap();
            }
        }
        for directory in ["bin", "config/services"] {
            fs::create_dir_all(prepared.join(directory)).unwrap();
        }
        fs::write(
            prepared.join("state/slots/active.json"),
            r#"{"schema":"aster.active-release-slot.v1","slot":"blue","version":"2.0.1"}"#,
        )
        .unwrap();
        if runner {
            fs::remove_file(prepared.join("state/slots/active.json")).unwrap();
        }
        let selection = SelectedRelease {
            schema: SELECTED_RELEASE_SCHEMA.to_owned(),
            version: "2.0.1".to_owned(),
            release_root: live.join("releases/2.0.1"),
            manifest_sha256: sha256_file(&prepared.join("releases/2.0.1/RELEASE.json")).unwrap(),
        };
        fs::write(
            prepared.join("state/selected-release.json"),
            serde_json::to_vec(&selection).unwrap(),
        )
        .unwrap();
    }

    fn fixture_verify(path: &Path) -> Result<String, CliFailure> {
        fs::read_to_string(path.join("VERSION")).map_err(failure)
    }

    #[test]
    fn validates_control_and_runner_materialized_references_and_replaces_untrusted_assets() {
        for runner in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let prepared = temp.path().join("prepared");
            let live = temp.path().join("live");
            fixture(&prepared, &live, runner);
            fs::write(prepared.join("bin/aster-team-cli.exe"), "untrusted").unwrap();
            let plan = validate_releases(&prepared, &live, runner, fixture_verify).unwrap();
            assert_eq!(plan.references.len(), 3);
            assert_eq!(plan.references[0].version, "2.0.1");
            assert!(!prepared.join("current").exists());
            assert_eq!(
                fs::read_to_string(prepared.join("bin/aster-team-cli.exe")).unwrap(),
                "2.0.1"
            );
            assert!(prepared.join("releases/2.0.1/RELEASE.json").is_file());
        }
    }

    #[test]
    fn inconsistent_metadata_or_signature_failure_never_removes_references() {
        for broken in ["active", "selection", "reference", "signature"] {
            let temp = tempfile::tempdir().unwrap();
            let prepared = temp.path().join("prepared");
            let live = temp.path().join("live");
            fixture(&prepared, &live, false);
            match broken {
                "active" => fs::write(
                    prepared.join("state/slots/active.json"),
                    r#"{"schema":"aster.active-release-slot.v1","slot":"blue","version":"9.0.0"}"#,
                )
                .unwrap(),
                "selection" => {
                    fs::write(prepared.join("state/selected-release.json"), "{}").unwrap()
                }
                "reference" => {
                    fs::write(prepared.join("current/RELEASE.json"), "different manifest").unwrap()
                }
                _ => (),
            }
            let result = validate_releases(&prepared, &live, false, |path| {
                if broken == "signature" {
                    Err(failure("invalid signature"))
                } else {
                    fixture_verify(path)
                }
            });
            assert!(result.is_err(), "{broken}");
            assert!(prepared.join("current/RELEASE.json").is_file());
        }
    }

    #[test]
    fn preflight_refuses_destinations_outside_the_private_restore_workspace() {
        let temp = tempfile::tempdir().unwrap();
        assert!(
            prepare(
                &temp.path().join("backup"),
                &temp.path().join("data"),
                temp.path(),
                false
            )
            .is_err()
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn restore_rejects_a_different_instance_before_inspecting_or_executing_releases() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("install");
        let destination = root.join("staging/restores/0123456789abcdef0123456789abcdef/prepared");
        fs::create_dir_all(&destination).unwrap();
        let layout = InstallLayout::new(&root).unwrap();
        let mut marker = layout.marker();
        let instance = aster_install_layout::WindowsInstance::from_environment(|name| {
            (name == "ASTER_SERVICE_PREFIX").then(|| "lab-a".into())
        })
        .unwrap();
        marker.windows_instance = Some(instance);
        fs::write(layout.marker_path(), serde_json::to_vec(&marker).unwrap()).unwrap();
        marker.windows_instance.as_mut().unwrap().service_prefix = "lab-b".into();
        let archived = serde_json::to_vec(&marker).unwrap();
        let archive = temp.path().join("backup.tar.gz");
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            File::create(&archive).unwrap(),
            flate2::Compression::fast(),
        ));
        let mut header = tar::Header::new_gnu();
        header.set_size(archived.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        builder
            .append_data(&mut header, "install/install.json", archived.as_slice())
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();
        let error = prepare(&archive, &destination, &root, false).err().unwrap();
        assert!(error.to_string().contains("instance settings do not match"));
        assert_eq!(layout.windows_instance().unwrap().service_prefix, "lab-a");
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn real_windows_tar_materializes_junctions_and_the_same_validator_accepts_them() {
        let temp = tempfile::tempdir().unwrap();
        let live = temp.path().join("install");
        fixture(&live, &live, false);
        fs::remove_dir_all(live.join("current")).unwrap();
        let status = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path $env:ASTER_TEST_LINK -Target $env:ASTER_TEST_RELEASE | Out-Null"])
            .env("ASTER_TEST_LINK", live.join("current"))
            .env("ASTER_TEST_RELEASE", live.join("releases/2.0.1"))
            .status().unwrap();
        assert!(status.success());
        let archive = temp.path().join("backup.tar.gz");
        assert!(
            std::process::Command::new("tar.exe")
                .arg("-C")
                .arg(temp.path())
                .arg("-czf")
                .arg(&archive)
                .arg("install")
                .status()
                .unwrap()
                .success()
        );
        let extraction = temp.path().join("extracted");
        fs::create_dir(&extraction).unwrap();
        let prepared = extract(&archive, &extraction, "install").unwrap();
        assert!(
            !fs::symlink_metadata(prepared.join("current"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        validate_releases(&prepared, &live, false, fixture_verify).unwrap();
        assert!(live.join("current/RELEASE.json").is_file());
    }

    #[test]
    fn rejects_windows_aliases_and_traversal_on_every_platform() {
        for name in [
            "../root/x",
            "/root/x",
            "root/../x",
            "root//x",
            "root/x:stream",
            "root/x.",
            "root/x ",
            "root/NUL.txt",
            "root/COM1",
            "root/LPT¹.txt",
            "root/a\\b",
            "root/C:/x",
            "root/a\nx",
        ] {
            assert!(safe_components(name).is_err(), "{name}");
        }
        assert!(safe_components("root/releases/2.0.1-rc.1/bin/工具.exe").is_ok());
    }

    fn archive(directory: &Path, entries: &[(&str, tar::EntryType)]) -> PathBuf {
        let path = directory.join("backup.tar.gz");
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            File::create(&path).unwrap(),
            flate2::Compression::fast(),
        ));
        for (name, kind) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(*kind);
            header.set_mode(if kind.is_dir() { 0o700 } else { 0o600 });
            header.set_size(0);
            if kind.is_symlink() || kind.is_hard_link() {
                header.set_link_name("../../outside").unwrap();
            }
            header.set_cksum();
            builder.append_data(&mut header, name, &[][..]).unwrap();
        }
        builder
            .into_inner()
            .unwrap()
            .finish()
            .unwrap()
            .flush()
            .unwrap();
        path
    }

    #[test]
    fn rejects_links_duplicate_paths_and_staging_payloads() {
        for entries in [
            vec![("install/current", tar::EntryType::Symlink)],
            vec![("install/data/db", tar::EntryType::Link)],
            vec![
                ("install/data/DB", tar::EntryType::Regular),
                ("install/data/db", tar::EntryType::Regular),
            ],
            vec![("install/staging/maintenance.lock", tar::EntryType::Regular)],
            vec![("other/install.json", tar::EntryType::Regular)],
        ] {
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("prepared");
            fs::create_dir(&destination).unwrap();
            let source = archive(temp.path(), &entries);
            assert!(extract(&source, &destination, "install").is_err());
        }
    }

    #[test]
    fn extracts_materialized_references_without_executing_archive_files() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("prepared");
        fs::create_dir(&destination).unwrap();
        let source = archive(
            temp.path(),
            &[
                ("install/", tar::EntryType::Directory),
                ("install/current/", tar::EntryType::Directory),
                ("install/current/RELEASE.json", tar::EntryType::Regular),
            ],
        );
        let prepared = extract(&source, &destination, "install").unwrap();
        assert!(prepared.join("current/RELEASE.json").is_file());
        assert!(extract(&source, &destination, "install").is_err());
    }
}
