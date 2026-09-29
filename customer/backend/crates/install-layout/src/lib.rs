#![forbid(unsafe_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const INSTALL_LAYOUT_SCHEMA: &str = "aster.install-layout.v1";
pub const INSTALL_MARKER_SCHEMA: &str = "aster.installation-root.v1";

mod database;
pub use database::{DatabaseConfiguration, DatabaseConfigurationError};

mod windows_instance;
pub use windows_instance::{WindowsInstance, WindowsPorts};

const CONTRACT_JSON: &str = include_str!("../../../../../contracts/install-layout.json");

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Platform {
    Linux,
    Windows,
    Macos,
}

impl Platform {
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Windows => "windows",
            Self::Macos => "macos",
        }
    }

    #[must_use]
    pub const fn current() -> Self {
        #[cfg(target_os = "linux")]
        {
            Self::Linux
        }
        #[cfg(target_os = "windows")]
        {
            Self::Windows
        }
        #[cfg(target_os = "macos")]
        {
            Self::Macos
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        compile_error!("Aster Team supports only Linux, Windows and macOS");
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallLayout {
    root: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallMarker {
    pub schema: String,
    pub root: PathBuf,
    pub platform: String,
    // Omitted for legacy/default installations. Older CLIs reject custom
    // instances instead of silently falling back to shared task names/ports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows_instance: Option<WindowsInstance>,
}

#[derive(Debug, Error)]
pub enum InstallLayoutError {
    #[error("the install root must be an absolute directory path")]
    InvalidRoot,
    #[error("the install root cannot be a filesystem root")]
    FilesystemRoot,
    #[error("the install layout contract is invalid: {0}")]
    InvalidContract(String),
    #[error("the install root marker is invalid: {0}")]
    InvalidMarker(String),
    #[error("the install root marker does not match the selected root or platform")]
    MarkerMismatch,
    #[error("the current executable path is unavailable")]
    ExecutableUnavailable,
    #[error("no Aster Team install root marker was found")]
    MarkerNotFound,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LayoutContract {
    schema: String,
    default_roots: BTreeMap<String, String>,
    command_links: BTreeMap<String, String>,
    service_registration_roots: BTreeMap<String, String>,
    release_paths: BTreeMap<String, BTreeMap<String, String>>,
    directories: Vec<String>,
    paths: BTreeMap<String, String>,
}

impl InstallLayout {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, InstallLayoutError> {
        let root = root.into();
        #[cfg(target_os = "windows")]
        let root = normalize_windows_root(root)?;
        validate_current_platform_root(&root)?;
        let root = root.components().collect();
        Ok(Self { root })
    }

    pub fn platform_default() -> Result<Self, InstallLayoutError> {
        Self::new(default_root(Platform::current())?)
    }

    pub fn discover() -> Result<Self, InstallLayoutError> {
        let executable =
            env::current_exe().map_err(|_| InstallLayoutError::ExecutableUnavailable)?;
        Self::discover_from(&executable)
    }

    pub fn discover_or_default() -> Result<Self, InstallLayoutError> {
        match Self::discover() {
            Ok(layout) => Ok(layout),
            Err(InstallLayoutError::MarkerNotFound | InstallLayoutError::ExecutableUnavailable) => {
                Self::platform_default()
            }
            Err(error) => Err(error),
        }
    }

    pub fn discover_from(path: &Path) -> Result<Self, InstallLayoutError> {
        let start = if path.is_dir() {
            path
        } else {
            path.parent().ok_or(InstallLayoutError::MarkerNotFound)?
        };
        for candidate in start.ancestors() {
            let marker_path = candidate.join(contract_path("marker")?);
            if marker_path.is_file() {
                let layout = Self::new(candidate.to_path_buf())?;
                layout.verify_marker_bytes(
                    &std::fs::read(marker_path)
                        .map_err(|error| InstallLayoutError::InvalidMarker(error.to_string()))?,
                )?;
                return Ok(layout);
            }
        }
        Err(InstallLayoutError::MarkerNotFound)
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn marker(&self) -> InstallMarker {
        InstallMarker {
            schema: INSTALL_MARKER_SCHEMA.to_owned(),
            root: self.root.clone(),
            platform: Platform::current().id().to_owned(),
            windows_instance: None,
        }
    }

    pub fn marker_json(&self) -> Result<Vec<u8>, InstallLayoutError> {
        serde_json::to_vec_pretty(&self.marker())
            .map_err(|error| InstallLayoutError::InvalidMarker(error.to_string()))
    }

    pub fn verify_marker_bytes(&self, bytes: &[u8]) -> Result<(), InstallLayoutError> {
        let marker: InstallMarker = serde_json::from_slice(bytes)
            .map_err(|error| InstallLayoutError::InvalidMarker(error.to_string()))?;
        if let Some(instance) = &marker.windows_instance {
            if marker.platform != "windows" {
                return Err(InstallLayoutError::InvalidMarker(
                    "Windows instance settings on another platform".to_owned(),
                ));
            }
            instance.validate()?;
        }
        let marker_root = Self::new(marker.root)
            .map_err(|error| InstallLayoutError::InvalidMarker(error.to_string()))?;
        if marker.schema != INSTALL_MARKER_SCHEMA
            || !same_platform_path(marker_root.root(), &self.root)
            || marker.platform != Platform::current().id()
        {
            return Err(InstallLayoutError::MarkerMismatch);
        }
        Ok(())
    }

    pub fn marker_path(&self) -> PathBuf {
        self.join("marker")
    }

    pub fn plugin_incoming_dir(&self) -> PathBuf {
        self.join("plugin_incoming_dir")
    }

    pub fn plugin_versions_dir(&self) -> PathBuf {
        self.join("plugin_versions_dir")
    }

    pub fn plugin_state_dir(&self) -> PathBuf {
        self.join("plugin_state_dir")
    }

    pub fn read_marker(&self) -> Result<InstallMarker, InstallLayoutError> {
        let bytes = std::fs::read(self.marker_path()).map_err(|error| match error.kind() {
            // An absent marker means the root is not an installed layout, which is
            // a different condition from a marker that cannot be trusted. Callers
            // that tolerate development roots match on `MarkerNotFound`.
            std::io::ErrorKind::NotFound => InstallLayoutError::MarkerNotFound,
            _ => InstallLayoutError::InvalidMarker(error.to_string()),
        })?;
        self.verify_marker_bytes(&bytes)?;
        serde_json::from_slice(&bytes)
            .map_err(|error| InstallLayoutError::InvalidMarker(error.to_string()))
    }

    /// Missing optional settings mean legacy defaults; an unreadable/corrupt
    /// marker is never treated as a default installation.
    pub fn windows_instance(&self) -> Result<WindowsInstance, InstallLayoutError> {
        Ok(self.read_marker()?.windows_instance.unwrap_or_default())
    }

    pub fn stable_bin(&self) -> PathBuf {
        self.join("stable_bin")
    }

    pub fn stable_cli(&self) -> PathBuf {
        self.stable_bin().join(executable_name("aster-team-cli"))
    }

    pub fn command_link(&self) -> Result<Option<PathBuf>, InstallLayoutError> {
        absolute_platform_path(&contract()?.command_links, Platform::current())
    }

    pub fn service_registration_root(&self) -> Result<Option<PathBuf>, InstallLayoutError> {
        absolute_platform_path(&contract()?.service_registration_roots, Platform::current())
    }

    pub fn current(&self) -> PathBuf {
        self.join("current")
    }

    pub fn releases(&self) -> PathBuf {
        self.join("releases")
    }

    pub fn release(&self, version: &str) -> PathBuf {
        self.releases().join(version)
    }

    pub fn config(&self) -> PathBuf {
        self.join("config")
    }

    pub fn cli_private(&self) -> PathBuf {
        self.join("cli_private")
    }

    pub fn control_config(&self) -> PathBuf {
        self.join("control_config")
    }

    pub fn runner_config(&self) -> PathBuf {
        self.join("runner_config")
    }

    pub fn caddy_config(&self) -> PathBuf {
        self.join("caddy_config")
    }

    pub fn service_config(&self) -> PathBuf {
        self.join("service_config")
    }

    pub fn keys(&self) -> PathBuf {
        self.join("keys")
    }

    pub fn license(&self) -> PathBuf {
        self.join("license")
    }

    pub fn tls(&self) -> PathBuf {
        self.join("tls")
    }

    pub fn data(&self) -> PathBuf {
        self.join("data")
    }

    pub fn database(&self) -> PathBuf {
        self.join("database")
    }

    pub fn runner_data(&self) -> PathBuf {
        self.join("runner_data")
    }

    pub fn caddy_data(&self) -> PathBuf {
        self.join("caddy_data")
    }

    pub fn runtime(&self) -> PathBuf {
        self.join("runtime")
    }

    pub fn state(&self) -> PathBuf {
        self.join("state")
    }

    pub fn migration_state(&self) -> PathBuf {
        self.join("migration_state")
    }

    pub fn upgrade_state(&self) -> PathBuf {
        self.join("upgrade_state")
    }

    pub fn upgrade_queued(&self) -> PathBuf {
        self.join("upgrade_queued")
    }

    pub fn upgrade_running(&self) -> PathBuf {
        self.join("upgrade_running")
    }

    pub fn upgrade_completed(&self) -> PathBuf {
        self.join("upgrade_completed")
    }

    pub fn slots(&self) -> PathBuf {
        self.join("slots")
    }

    pub fn active_slot(&self) -> PathBuf {
        self.join("active_slot")
    }

    pub fn slot_release(&self, slot: &str) -> PathBuf {
        match slot {
            "blue" => self.join("blue_release"),
            "green" => self.join("green_release"),
            _ => panic!("invalid Aster Team release slot"),
        }
    }

    pub fn locks(&self) -> PathBuf {
        self.join("locks")
    }

    pub fn staging(&self) -> PathBuf {
        self.join("staging")
    }

    pub fn upgrade_uploads(&self) -> PathBuf {
        self.join("upgrade_uploads")
    }

    pub fn backups(&self) -> PathBuf {
        self.join("backups")
    }

    pub fn upgrade_backups(&self) -> PathBuf {
        self.join("upgrade_backups")
    }

    pub fn logs(&self) -> PathBuf {
        self.join("logs")
    }

    pub fn selected_release(&self) -> PathBuf {
        self.join("selected_release")
    }

    pub fn maintenance_lock(&self) -> PathBuf {
        self.join("maintenance_lock")
    }

    pub fn control_role(&self) -> PathBuf {
        self.join("control_role")
    }

    pub fn runner_role(&self) -> PathBuf {
        self.join("runner_role")
    }

    pub fn control_environment(&self) -> PathBuf {
        self.join("control_environment")
    }

    pub fn control_slot_environment(&self, slot: &str) -> PathBuf {
        match slot {
            "blue" => self.join("control_blue_environment"),
            "green" => self.join("control_green_environment"),
            _ => panic!("invalid Aster Team release slot"),
        }
    }

    pub fn runner_environment(&self) -> PathBuf {
        self.join("runner_environment")
    }

    pub fn runner_slot_environment(&self, slot: &str) -> PathBuf {
        match slot {
            "blue" => self.join("runner_blue_environment"),
            "green" => self.join("runner_green_environment"),
            _ => panic!("invalid Aster Team release slot"),
        }
    }

    pub fn runner_slot_identity(&self, slot: &str) -> PathBuf {
        match slot {
            "blue" => self.join("runner_blue_identity"),
            "green" => self.join("runner_green_identity"),
            _ => panic!("invalid Aster Team release slot"),
        }
    }

    pub fn runner_slot_task_keys(&self, slot: &str) -> PathBuf {
        match slot {
            "blue" => self.join("runner_blue_task_keys"),
            "green" => self.join("runner_green_task_keys"),
            _ => panic!("invalid Aster Team release slot"),
        }
    }

    pub fn runner_slot_provisioning(&self, slot: &str) -> PathBuf {
        match slot {
            "blue" => self.join("runner_blue_provisioning"),
            "green" => self.join("runner_green_provisioning"),
            _ => panic!("invalid Aster Team release slot"),
        }
    }

    pub fn runner_slot_provisioning_lock(&self, slot: &str) -> PathBuf {
        match slot {
            "blue" => self.join("runner_blue_provisioning_lock"),
            "green" => self.join("runner_green_provisioning_lock"),
            _ => panic!("invalid Aster Team release slot"),
        }
    }

    pub fn runner_slot_identity_output(&self, slot: &str) -> PathBuf {
        match slot {
            "blue" => self.join("runner_blue_identity_output"),
            "green" => self.join("runner_green_identity_output"),
            _ => panic!("invalid Aster Team release slot"),
        }
    }

    pub fn runner_slot_task_keys_output(&self, slot: &str) -> PathBuf {
        match slot {
            "blue" => self.join("runner_blue_task_keys_output"),
            "green" => self.join("runner_green_task_keys_output"),
            _ => panic!("invalid Aster Team release slot"),
        }
    }

    pub fn runner_slot_upstream_ca(&self, slot: &str) -> PathBuf {
        match slot {
            "blue" => self.join("runner_blue_upstream_ca"),
            "green" => self.join("runner_green_upstream_ca"),
            _ => panic!("invalid Aster Team release slot"),
        }
    }

    pub fn control_slot_runtime_token(&self, slot: &str) -> PathBuf {
        match slot {
            "blue" => self.join("control_blue_runtime_token"),
            "green" => self.join("control_green_runtime_token"),
            _ => panic!("invalid Aster Team release slot"),
        }
    }

    pub fn access_configuration(&self) -> PathBuf {
        self.join("access_configuration")
    }

    pub fn initial_owner_credentials(&self) -> PathBuf {
        self.join("initial_owner_credentials")
    }

    pub fn stable_caddy(&self) -> PathBuf {
        if cfg!(target_os = "windows") {
            self.join("stable_caddy").with_extension("exe")
        } else {
            self.join("stable_caddy")
        }
    }

    pub fn release_binary(&self, release: &Path, name: &str) -> PathBuf {
        release.join("bin").join(executable_name(name))
    }

    pub fn release_platform_path(
        &self,
        release: &Path,
        name: &str,
    ) -> Result<PathBuf, InstallLayoutError> {
        let relative = contract()?
            .release_paths
            .get(name)
            .and_then(|paths| paths.get(Platform::current().id()))
            .ok_or_else(|| {
                InstallLayoutError::InvalidContract(format!(
                    "missing release path {name} for {}",
                    Platform::current().id()
                ))
            })?;
        join_relative(release, relative)
    }

    pub fn windows_service_launcher(&self) -> PathBuf {
        self.join("windows_service_launcher")
    }

    pub fn platform_service_launcher(&self) -> Result<PathBuf, InstallLayoutError> {
        match Platform::current() {
            Platform::Windows => Ok(self.join("windows_service_launcher")),
            Platform::Macos => Ok(self.join("macos_service_launcher")),
            Platform::Linux => Err(InstallLayoutError::InvalidContract(
                "Linux services do not use a service launcher".to_owned(),
            )),
        }
    }

    pub fn installation_profile(&self) -> PathBuf {
        self.join("installation_profile")
    }

    pub fn installation_key(&self) -> PathBuf {
        self.join("installation_key")
    }

    pub fn database_key(&self) -> PathBuf {
        self.join("database_key")
    }

    pub fn database_configuration(&self) -> PathBuf {
        self.join("database_configuration")
    }

    pub fn database_ca_certificate(&self) -> PathBuf {
        self.join("database_ca_certificate")
    }

    pub fn database_password(&self) -> PathBuf {
        self.join("database_password")
    }

    pub fn database_file(&self) -> PathBuf {
        self.join("database_file")
    }

    pub fn runner_task_key(&self) -> PathBuf {
        self.join("runner_task_key")
    }

    pub fn control_runner_task_keys(&self) -> PathBuf {
        self.join("control_runner_task_keys")
    }

    pub fn runner_task_keys(&self) -> PathBuf {
        self.join("runner_task_keys")
    }

    pub fn runner_identity(&self) -> PathBuf {
        self.join("runner_identity")
    }

    pub fn runner_control_ca(&self) -> PathBuf {
        self.join("runner_control_ca")
    }

    pub fn license_file(&self) -> PathBuf {
        self.join("license_file")
    }

    pub fn license_request(&self) -> PathBuf {
        self.join("license_request")
    }

    pub fn settlement_outbox(&self) -> PathBuf {
        self.join("settlement_outbox")
    }

    pub fn license_state(&self) -> PathBuf {
        self.join("license_state")
    }

    pub fn license_state_lock(&self) -> PathBuf {
        self.join("license_state_lock")
    }

    pub fn initialization_complete(&self) -> PathBuf {
        self.join("initialization_complete")
    }

    pub fn caddyfile(&self) -> PathBuf {
        self.join("caddyfile")
    }

    pub fn caddy_upstreams(&self) -> PathBuf {
        self.join("caddy_upstreams")
    }

    pub fn caddy_root_certificate(&self) -> PathBuf {
        self.join("caddy_root_certificate")
    }

    pub fn caddy_server_certificate(&self) -> PathBuf {
        self.join("caddy_server_certificate")
    }

    pub fn caddy_server_private_key(&self) -> PathBuf {
        self.join("caddy_server_private_key")
    }

    pub fn admin_assets(&self) -> PathBuf {
        self.join("admin_assets")
    }

    pub fn member_assets(&self) -> PathBuf {
        self.join("member_assets")
    }

    pub fn required_directories(&self) -> Result<Vec<PathBuf>, InstallLayoutError> {
        let contract = contract()?;
        let mut paths = contract
            .directories
            .iter()
            .map(|name| {
                contract
                    .paths
                    .get(name)
                    .ok_or_else(|| {
                        InstallLayoutError::InvalidContract(format!("directory {name} has no path"))
                    })
                    .and_then(|relative| join_relative(&self.root, relative))
            })
            .collect::<Result<Vec<_>, _>>()?;
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    fn join(&self, name: &str) -> PathBuf {
        let relative = contract_path(name)
            .unwrap_or_else(|error| panic!("embedded install layout contract: {error}"));
        join_relative(&self.root, relative)
            .unwrap_or_else(|error| panic!("embedded install layout contract: {error}"))
    }
}

pub fn default_root(platform: Platform) -> Result<PathBuf, InstallLayoutError> {
    contract()?
        .default_roots
        .get(platform.id())
        .map(PathBuf::from)
        .ok_or_else(|| {
            InstallLayoutError::InvalidContract(format!(
                "missing default root for {}",
                platform.id()
            ))
        })
}

fn contract() -> Result<&'static LayoutContract, InstallLayoutError> {
    static CONTRACT: OnceLock<Result<LayoutContract, String>> = OnceLock::new();
    CONTRACT
        .get_or_init(|| {
            let parsed: LayoutContract =
                serde_json::from_str(CONTRACT_JSON).map_err(|error| error.to_string())?;
            if parsed.schema != INSTALL_LAYOUT_SCHEMA {
                return Err(format!("unexpected schema {}", parsed.schema));
            }
            for relative in parsed.paths.values() {
                validate_relative(relative).map_err(|error| error.to_string())?;
            }
            let mut directories = BTreeSet::new();
            for name in &parsed.directories {
                if !directories.insert(name) {
                    return Err(format!("duplicate directory path name {name}"));
                }
                if !parsed.paths.contains_key(name) {
                    return Err(format!("directory {name} has no path"));
                }
            }
            for paths in [
                &parsed.default_roots,
                &parsed.command_links,
                &parsed.service_registration_roots,
            ] {
                for (platform, absolute) in paths {
                    if !contract_absolute_path(platform, absolute) {
                        return Err(format!(
                            "{platform} platform path is not absolute: {absolute}"
                        ));
                    }
                }
            }
            for (name, paths) in &parsed.release_paths {
                if paths.is_empty() {
                    return Err(format!("release path {name} has no platform values"));
                }
                for (platform, relative) in paths {
                    if !matches!(platform.as_str(), "linux" | "windows" | "macos") {
                        return Err(format!(
                            "release path {name} uses unsupported platform {platform}"
                        ));
                    }
                    validate_relative(relative).map_err(|error| error.to_string())?;
                }
            }
            Ok(parsed)
        })
        .as_ref()
        .map_err(|error| InstallLayoutError::InvalidContract(error.clone()))
}

fn absolute_platform_path(
    paths: &BTreeMap<String, String>,
    platform: Platform,
) -> Result<Option<PathBuf>, InstallLayoutError> {
    Ok(paths.get(platform.id()).map(PathBuf::from))
}

fn contract_absolute_path(platform: &str, value: &str) -> bool {
    match platform {
        "windows" => {
            let bytes = value.as_bytes();
            bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && matches!(bytes[2], b'\\' | b'/')
        }
        "linux" | "macos" => value.starts_with('/') && value != "/",
        _ => false,
    }
}

fn contract_path(name: &str) -> Result<&'static str, InstallLayoutError> {
    contract()?
        .paths
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| InstallLayoutError::InvalidContract(format!("missing path {name}")))
}

fn join_relative(root: &Path, relative: &str) -> Result<PathBuf, InstallLayoutError> {
    validate_relative(relative)?;
    let mut result = root.to_path_buf();
    for component in relative.split('/') {
        result.push(component);
    }
    Ok(result)
}

fn validate_relative(relative: &str) -> Result<(), InstallLayoutError> {
    if relative.is_empty()
        || relative.starts_with('/')
        || relative.starts_with('\\')
        || relative
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || relative.contains('\\')
        || relative.contains(':')
    {
        return Err(InstallLayoutError::InvalidContract(format!(
            "invalid relative path {relative}"
        )));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn normalize_windows_root(root: PathBuf) -> Result<PathBuf, InstallLayoutError> {
    use std::path::Prefix;
    let Some(Component::Prefix(prefix)) = root.components().next() else {
        return Ok(root);
    };
    let mut normalized = match prefix.kind() {
        Prefix::VerbatimDisk(drive) => PathBuf::from(format!("{}:\\", char::from(drive))),
        Prefix::VerbatimUNC(server, share) => {
            let mut path = std::ffi::OsString::from(r"\\");
            path.push(server);
            path.push(r"\");
            path.push(share);
            path.push(r"\");
            PathBuf::from(path)
        }
        Prefix::Disk(_) | Prefix::UNC(_, _) => return Ok(root),
        _ => return Err(InstallLayoutError::InvalidRoot),
    };
    for component in root.components().skip(1) {
        match component {
            Component::RootDir => {}
            Component::Normal(part) => {
                // Verbatim paths can name trailing-dot/space files that ordinary
                // Win32 paths cannot. Never normalize those into another root.
                let text = part.to_str().ok_or(InstallLayoutError::InvalidRoot)?;
                if text.ends_with(['.', ' ']) {
                    return Err(InstallLayoutError::InvalidRoot);
                }
                normalized.push(part);
            }
            _ => return Err(InstallLayoutError::InvalidRoot),
        }
    }
    Ok(normalized)
}

fn validate_current_platform_root(root: &Path) -> Result<(), InstallLayoutError> {
    let encoded = root.to_str().ok_or(InstallLayoutError::InvalidRoot)?;
    if encoded.chars().any(char::is_control)
        || !root.is_absolute()
        || root
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(InstallLayoutError::InvalidRoot);
    }
    if root.parent().is_none() {
        return Err(InstallLayoutError::FilesystemRoot);
    }
    Ok(())
}

fn same_platform_path(left: &Path, right: &Path) -> bool {
    if cfg!(target_os = "windows") {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    } else {
        left == right
    }
}

#[must_use]
pub fn executable_name(name: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "windows")]
    #[test]
    fn canonical_windows_executable_paths_discover_the_same_installation() {
        let temp = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(temp.path()).unwrap();
        std::fs::write(layout.marker_path(), layout.marker_json().unwrap()).unwrap();
        let canonical_root = temp.path().canonicalize().unwrap();
        assert_eq!(
            InstallLayout::new(&canonical_root).unwrap().root(),
            layout.root()
        );
        assert_eq!(
            InstallLayout::discover_from(&canonical_root.join("bin/aster-team-cli.exe"))
                .unwrap()
                .root(),
            layout.root()
        );
        assert_eq!(
            InstallLayout::new(r"\\?\UNC\server\share\Aster Team")
                .unwrap()
                .root(),
            Path::new(r"\\server\share\Aster Team")
        );
        assert!(InstallLayout::new(r"\\?\C:\Aster Team.").is_err());
        assert!(InstallLayout::new(r"\\.\C:\Aster Team").is_err());
    }

    #[test]
    fn an_absent_marker_is_reported_as_not_found_and_a_corrupt_one_as_invalid() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(temporary.path()).unwrap();
        assert!(matches!(
            layout.read_marker(),
            Err(InstallLayoutError::MarkerNotFound)
        ));
        std::fs::write(layout.marker_path(), b"{").unwrap();
        assert!(matches!(
            layout.read_marker(),
            Err(InstallLayoutError::InvalidMarker(_))
        ));
    }

    #[test]
    fn legacy_markers_omit_instance_settings_but_invalid_markers_fail_closed() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(temporary.path()).unwrap();
        assert!(layout.windows_instance().is_err());
        let bytes = layout.marker_json().unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(json.get("windows_instance").is_none());
        std::fs::write(layout.marker_path(), bytes).unwrap();
        assert_eq!(
            layout.windows_instance().unwrap(),
            WindowsInstance::default()
        );
        std::fs::write(layout.marker_path(), b"{").unwrap();
        assert!(layout.windows_instance().is_err());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn persisted_instance_roundtrips_and_rejects_corrupt_settings() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(temporary.path()).unwrap();
        let instance = WindowsInstance::from_environment(|name| match name {
            "ASTER_SERVICE_PREFIX" => Some("lab-a".into()),
            "ASTER_PORT_OFFSET" => Some("10000".into()),
            _ => None,
        })
        .unwrap();
        let mut marker = layout.marker();
        marker.windows_instance = Some(instance.clone());
        std::fs::write(layout.marker_path(), serde_json::to_vec(&marker).unwrap()).unwrap();
        assert_eq!(layout.windows_instance().unwrap(), instance);
        let mut json = serde_json::to_value(marker).unwrap();
        json["windows_instance"]["ports"]["api"] = 0.into();
        assert!(
            layout
                .verify_marker_bytes(&serde_json::to_vec(&json).unwrap())
                .is_err()
        );
        json["windows_instance"]["ports"]["api"] = 21080.into();
        json["windows_instance"]["unexpected"] = true.into();
        assert!(
            layout
                .verify_marker_bytes(&serde_json::to_vec(&json).unwrap())
                .is_err()
        );
    }

    #[test]
    fn contract_declares_all_supported_platforms() {
        assert_eq!(
            default_root(Platform::Linux).unwrap(),
            Path::new("/opt/aster-team")
        );
        assert_eq!(
            default_root(Platform::Windows).unwrap(),
            Path::new(r"C:\ProgramData\Aster Team")
        );
        assert_eq!(
            default_root(Platform::Macos).unwrap(),
            Path::new("/Library/Application Support/Aster Team")
        );
        let layout = InstallLayout::platform_default().unwrap();
        if Platform::current() == Platform::Windows {
            assert_eq!(layout.command_link().unwrap(), None);
        } else {
            assert_eq!(
                layout.command_link().unwrap().as_deref(),
                Some(Path::new("/usr/local/bin/aster-team-cli"))
            );
        }
    }

    #[test]
    fn layout_keeps_configuration_data_state_and_backups_below_one_root() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(temporary.path()).unwrap();
        for path in layout.required_directories().unwrap() {
            assert!(path.starts_with(temporary.path()), "{}", path.display());
        }
        assert_eq!(layout.database(), temporary.path().join("data/database"));
        assert_eq!(
            layout.upgrade_backups(),
            temporary.path().join("backups/upgrades")
        );
    }

    #[test]
    fn slot_material_is_separate_from_shared_runner_and_other_slot() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(temporary.path()).unwrap();
        let mut unique = std::collections::BTreeSet::new();
        for slot in ["blue", "green"] {
            let base = temporary.path().join("config/runner/slots").join(slot);
            assert_eq!(
                layout.runner_slot_identity(slot),
                base.join("identity.json")
            );
            assert_eq!(
                layout.runner_slot_task_keys(slot),
                base.join("task-keys.json")
            );
            assert_eq!(
                layout.runner_slot_environment(slot),
                base.join("runner.env")
            );
            for path in [
                layout.runner_slot_identity(slot),
                layout.runner_slot_task_keys(slot),
                layout.runner_slot_environment(slot),
                layout.control_slot_runtime_token(slot),
            ] {
                assert!(path.starts_with(temporary.path()));
                assert!(unique.insert(path));
            }
        }
        for legacy in [
            layout.runner_identity(),
            layout.runner_task_keys(),
            layout.runner_environment(),
        ] {
            assert!(!unique.contains(&legacy));
        }
    }

    #[test]
    fn marker_must_match_root_and_platform() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(temporary.path()).unwrap();
        layout
            .verify_marker_bytes(&layout.marker_json().unwrap())
            .unwrap();

        let other = tempfile::tempdir().unwrap();
        let other_layout = InstallLayout::new(other.path()).unwrap();
        assert!(matches!(
            other_layout.verify_marker_bytes(&layout.marker_json().unwrap()),
            Err(InstallLayoutError::MarkerMismatch)
        ));
    }

    #[test]
    fn discovers_root_from_release_binary_location() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(temporary.path()).unwrap();
        std::fs::write(layout.marker_path(), layout.marker_json().unwrap()).unwrap();
        let executable = layout.release("2.1.0").join("bin/aster-control");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::write(&executable, []).unwrap();

        assert_eq!(InstallLayout::discover_from(&executable).unwrap(), layout);
    }

    #[test]
    fn rejects_relative_and_filesystem_roots() {
        assert!(matches!(
            InstallLayout::new("relative"),
            Err(InstallLayoutError::InvalidRoot)
        ));
        let current = std::env::current_dir().unwrap();
        let filesystem_root = current.ancestors().last().unwrap();
        assert!(matches!(
            InstallLayout::new(filesystem_root),
            Err(InstallLayoutError::FilesystemRoot)
        ));
    }

    #[test]
    fn normalizes_repeated_separators_and_trailing_separator() {
        let temporary = tempfile::tempdir().unwrap();
        let source = format!("{}//nested/", temporary.path().display());
        let layout = InstallLayout::new(source).unwrap();
        assert_eq!(layout.root(), temporary.path().join("nested"));
    }
}
