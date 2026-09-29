use std::{
    fs,
    io::Read as _,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use aster_error_catalog::delivery;
use aster_install_layout::{DatabaseConfiguration, InstallLayout};
use aster_upgrade_core::{ReleaseSlot, UpgradeMode};
use serde::Deserialize;
use zeroize::Zeroizing;

use super::{CliFailure, RUNNER_ALLOWED_UPSTREAM_HOSTS, run_checked};

#[cfg(target_os = "linux")]
pub(crate) mod online;

const CAPABILITIES: &str = "systemd/local-slot-preparation.json";
const MAX_ENVIRONMENT_BYTES: usize = 16 * 1024;

#[cfg(target_os = "linux")]
pub(crate) fn require_online_support(release: &Path) -> Result<(), CliFailure> {
    require_recovery_support(release)?;
    require_settlement_support(release)
}

#[cfg(target_os = "linux")]
pub(crate) fn require_settlement_support(release: &Path) -> Result<(), CliFailure> {
    use aster_upgrade_core::settlement::{CAPABILITY_FILE, compatible};
    let path = release.join(CAPABILITY_FILE);
    if !optional_file(&path)? || !compatible(&read_regular(&path, 4096)?) {
        return Err(failed(
            "online upgrade requires matching signed gateway settlement protocols; use maintenance upgrade",
        ));
    }
    Ok(())
}

// Recovery of an older failed task must not require a new coexistence format.
#[cfg(target_os = "linux")]
pub(crate) fn require_recovery_support(release: &Path) -> Result<(), CliFailure> {
    if !supports_preparation(release)? || !supports_slot_runtime(release)? {
        return Err(failed(
            "online upgrade requires signed preparation and runtime capabilities",
        ));
    }
    Ok(())
}

#[derive(Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct Capabilities {
    schema: String,
    runner_protocol: u32,
    runner_identity_schema: String,
    control_runtime_schema: String,
}

struct Plan {
    control_environment: String,
    runner_environment: String,
    upstream_ca: Option<Vec<u8>>,
    capacity: u32,
    heartbeat: u64,
}

fn failed(message: impl Into<String>) -> CliFailure {
    CliFailure::new(delivery::UPGRADE_FAILED, message)
}

fn read_regular(path: &Path, limit: usize) -> Result<Zeroizing<Vec<u8>>, CliFailure> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        failed(format!(
            "slot material unavailable: {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit as u64 {
        return Err(failed(format!(
            "slot material is unsafe or too large: {}",
            path.display()
        )));
    }
    let mut bytes = Zeroizing::new(Vec::new());
    fs::File::open(path)
        .map_err(|error| failed(error.to_string()))?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| failed(error.to_string()))?;
    if bytes.len() > limit {
        return Err(failed("slot material exceeded its size limit"));
    }
    Ok(bytes)
}

fn optional_file(path: &Path) -> Result<bool, CliFailure> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(failed(format!("unsafe slot file: {}", path.display()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(failed(error.to_string())),
    }
}

fn supports_preparation(release: &Path) -> Result<bool, CliFailure> {
    let path = release.join(CAPABILITIES);
    if !optional_file(&path)? {
        return Ok(false);
    }
    let actual: Capabilities = serde_json::from_slice(&read_regular(&path, 4096)?)
        .map_err(|_| failed("invalid signed local-slot preparation capabilities"))?;
    let expected: Capabilities = serde_json::from_str(include_str!(
        "../../../deploy/systemd/local-slot-preparation.json"
    ))
    .map_err(|_| failed("invalid compiled local-slot preparation contract"))?;
    if actual != expected {
        return Err(failed("unsupported local-slot preparation capabilities"));
    }
    if !optional_file(&release.join("systemd/aster-runner@.service"))? {
        return Err(failed("local-slot Runner service template is missing"));
    }
    Ok(true)
}

fn supports_slot_runtime(release: &Path) -> Result<bool, CliFailure> {
    let path = release.join("systemd/local-slot-runtime.json");
    if !optional_file(&path)? {
        return Ok(false);
    }
    #[derive(Deserialize, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct RuntimeCapabilities {
        schema: String,
        active_slot_schema: String,
    }
    let actual: RuntimeCapabilities = serde_json::from_slice(&read_regular(&path, 4096)?)
        .map_err(|_| failed("invalid slot runtime capabilities"))?;
    let expected: RuntimeCapabilities = serde_json::from_str(include_str!(
        "../../../deploy/systemd/local-slot-runtime.json"
    ))
    .map_err(|_| failed("invalid built-in slot runtime capabilities"))?;
    if actual != expected {
        return Err(failed("unsupported slot runtime capabilities"));
    }
    Ok(true)
}

fn environment_value<'a>(source: &'a str, key: &str) -> Option<&'a str> {
    source
        .lines()
        .filter_map(|line| {
            let (name, value) = line.trim().split_once('=')?;
            (name.trim() == key).then_some(value.trim())
        })
        .next_back()
}

fn read_environment(path: &Path) -> Result<String, CliFailure> {
    String::from_utf8(read_regular(path, MAX_ENVIRONMENT_BYTES)?.to_vec())
        .map_err(|_| failed("slot environment is not UTF-8"))
}

fn make_plan(
    layout: &InstallLayout,
    slot: ReleaseSlot,
    mode: UpgradeMode,
) -> Result<Plan, CliFailure> {
    let source = read_environment(&layout.control_slot_environment(slot.id()))?;
    let ports = slot.ports();
    for (key, port) in [
        ("ASTER_CONTROL_LISTEN", ports.api),
        ("ASTER_CONTROL_MEMBER_LISTEN", ports.member),
        ("ASTER_CONTROL_ADMIN_LISTEN", ports.admin),
    ] {
        if environment_value(&source, key) != Some(format!("127.0.0.1:{port}").as_str()) {
            return Err(failed(
                "local slots require the standard loopback Control listeners",
            ));
        }
    }
    let managed = [
        "ASTER_CONTROL_RUNTIME_LISTEN",
        "ASTER_CONTROL_RUNTIME_TOKEN_FILE",
        "ASTER_CONTROL_RUNTIME_SLOT",
        "ASTER_CONTROL_RUNTIME_CANDIDATE",
    ];
    let mut control = source
        .lines()
        .filter(|line| {
            !line
                .trim()
                .split_once('=')
                .is_some_and(|(key, _)| managed.contains(&key.trim()))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let closed = mode == UpgradeMode::BlueGreen;
    control.push_str(&format!("\nASTER_CONTROL_RUNTIME_LISTEN=127.0.0.1:{}\nASTER_CONTROL_RUNTIME_TOKEN_FILE={}\nASTER_CONTROL_RUNTIME_SLOT={}\nASTER_CONTROL_RUNTIME_CANDIDATE={closed}\n", slot.runtime_port(), layout.control_slot_runtime_token(slot.id()).display(), slot.id()));
    let source_environment = if optional_file(&layout.active_slot())? {
        let active: aster_upgrade_core::ActiveReleaseSlot =
            serde_json::from_slice(&read_regular(&layout.active_slot(), 8192)?)
                .map_err(|_| failed("active slot metadata is invalid"))?;
        active
            .validate()
            .map_err(|_| failed("active slot metadata is invalid"))?;
        if active.local_runner.is_some() {
            layout.runner_slot_environment(active.slot.id())
        } else {
            layout.runner_environment()
        }
    } else {
        layout.runner_environment()
    };
    let shared = read_environment(&source_environment)?;
    let capacity = environment_value(&shared, "ASTER_RUNNER_MAX_INFLIGHT")
        .unwrap_or("4")
        .parse::<u32>()
        .map_err(|_| failed("invalid local Runner capacity"))?;
    let heartbeat = environment_value(&shared, "ASTER_RUNNER_HEARTBEAT_SECONDS")
        .unwrap_or("10")
        .parse::<u64>()
        .map_err(|_| failed("invalid local Runner heartbeat interval"))?;
    if !(1..=100_000).contains(&capacity) || !(2..=60).contains(&heartbeat) {
        return Err(failed(
            "invalid local Runner capacity or heartbeat interval",
        ));
    }
    let mut runner = format!(
        "ASTER_RUNNER_MAX_INFLIGHT={capacity}\nASTER_RUNNER_HEARTBEAT_SECONDS={heartbeat}\n"
    );
    let upstream_ca =
        if let Some(path) = environment_value(&shared, "ASTER_RUNNER_UPSTREAM_CA_CERTIFICATE") {
            let path = Path::new(path);
            if !path.is_absolute() {
                return Err(failed("local Runner upstream CA path must be absolute"));
            }
            let bytes = read_regular(path, 1024 * 1024)?;
            runner.push_str(&format!(
                "ASTER_RUNNER_UPSTREAM_CA_CERTIFICATE={}\n",
                layout.runner_slot_upstream_ca(slot.id()).display()
            ));
            Some(bytes.to_vec())
        } else {
            None
        };
    Ok(Plan {
        control_environment: control,
        runner_environment: runner,
        upstream_ca,
        capacity,
        heartbeat,
    })
}

fn safe_owned_path(layout: &InstallLayout, path: &Path) -> Result<(), CliFailure> {
    let relative = path
        .strip_prefix(layout.root())
        .map_err(|_| failed("slot output is outside the installation"))?;
    let mut cursor = layout.root().to_path_buf();
    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err(failed("invalid slot output path"));
        }
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Err(failed("slot output has an unsafe parent")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(failed(error.to_string())),
        }
        cursor.push(component);
    }
    optional_file(path)?;
    Ok(())
}

fn owned_directory(path: &Path, group: &str) -> Result<(), CliFailure> {
    let mut command = Command::new("install");
    command
        .args(["-d", "-o", "root", "-g", group, "-m", "0750", "--"])
        .arg(path);
    run_checked(
        command,
        delivery::UPGRADE_FAILED,
        "prepare local-slot directory",
    )
}

fn owned_file(
    layout: &InstallLayout,
    path: PathBuf,
    bytes: &[u8],
    group: &str,
) -> Result<(), CliFailure> {
    safe_owned_path(layout, &path)?;
    super::maintenance_executor::atomic_replace(path.clone(), bytes, 0o640)?;
    let mut command = Command::new("chown");
    command.arg(format!("root:{group}")).arg("--").arg(path);
    run_checked(
        command,
        delivery::UPGRADE_FAILED,
        "set local-slot file ownership",
    )
}

fn preparation_mode(
    layout: &InstallLayout,
    release: &Path,
    actor_id: &str,
    job_id: &str,
) -> Result<UpgradeMode, CliFailure> {
    let path = layout.upgrade_running().join(format!("{job_id}.json"));
    let job: aster_upgrade_core::MaintenanceJob =
        serde_json::from_slice(&read_regular(&path, 256 * 1024)?)
            .map_err(|_| failed("candidate preparation job is invalid"))?;
    job.validate()
        .map_err(|_| failed("candidate preparation job is invalid"))?;
    if job.id != job_id
        || job.requested_by != actor_id
        || job.status != aster_upgrade_core::MaintenanceStatus::StartingCandidate
        || job.candidate_release.as_deref() != Some(release)
        || !matches!(
            job.operation,
            aster_upgrade_core::MaintenanceOperation::Upgrade { .. }
        )
        || job
            .target_version
            .as_ref()
            .is_none_or(|version| layout.release(version) != release)
    {
        return Err(failed(
            "candidate startup mode does not match its durable preparation job",
        ));
    }
    // The direct maintenance CLI currently leaves this optional field unset.
    Ok(job.upgrade_mode.unwrap_or(UpgradeMode::Maintenance))
}

/// Only the startup admission value changes after a completed online cutover.
/// The running Control was opened through its instance-bound readiness permit.
pub(super) fn committed_control_environment(
    source: &str,
    slot: ReleaseSlot,
) -> Result<String, CliFailure> {
    let exact = |key: &str| -> Result<&str, CliFailure> {
        let mut values = source.lines().filter_map(|line| {
            let (name, value) = line.trim().split_once('=')?;
            (name.trim() == key).then_some(value.trim())
        });
        let value = values
            .next()
            .ok_or_else(|| failed("candidate runtime setting is missing"))?;
        if values.next().is_some() {
            return Err(failed("candidate runtime setting is duplicated"));
        }
        Ok(value)
    };
    if exact("ASTER_CONTROL_RUNTIME_SLOT")? != slot.id()
        || !matches!(exact("ASTER_CONTROL_RUNTIME_CANDIDATE")?, "true" | "false")
    {
        return Err(failed(
            "candidate startup configuration does not match the committed slot",
        ));
    }
    let mut result = String::new();
    for line in source.lines() {
        if line
            .trim()
            .split_once('=')
            .is_some_and(|(key, _)| key.trim() == "ASTER_CONTROL_RUNTIME_CANDIDATE")
        {
            result.push_str("ASTER_CONTROL_RUNTIME_CANDIDATE=false");
        } else {
            result.push_str(line);
        }
        result.push('\n');
    }
    Ok(result)
}

fn control_command(layout: &InstallLayout, release: &Path) -> Command {
    let mut command = Command::new("runuser");
    command
        .args(["-u", "aster-team", "--"])
        .arg(layout.release_binary(release, "aster-control"));
    command
}

pub(crate) fn assert_candidate_stopped(
    slot: ReleaseSlot,
    mut state: impl FnMut(&str) -> Result<String, CliFailure>,
) -> Result<(), CliFailure> {
    for unit in [
        format!("aster-control@{}.service", slot.id()),
        format!("aster-runner@{}.service", slot.id()),
    ] {
        if !matches!(
            state(&unit)?.as_str(),
            "inactive" | "failed" | "disabled" | "not-installed"
        ) {
            return Err(failed(format!(
                "refusing to prepare a running or unknown candidate: {unit}"
            )));
        }
    }
    Ok(())
}

/// Uses the same signed preparation commands for maintenance and online jobs.
/// Online jobs persist intent before changing the stopped candidate slot and
/// start it with admission closed; maintenance callers already stopped services.
pub(super) fn prepare_if_supported(
    layout: &InstallLayout,
    release: &Path,
    slot: ReleaseSlot,
    actor_id: &str,
    job_id: &str,
) -> Result<Option<aster_upgrade_core::ActiveLocalRunner>, CliFailure> {
    if !supports_preparation(release)?
        || !matches!(
            DatabaseConfiguration::load(layout).map_err(|error| failed(error.to_string()))?,
            DatabaseConfiguration::Mariadb { .. }
        )
    {
        return Ok(None);
    }
    let activate_runtime = supports_slot_runtime(release)?;
    let active = if optional_file(&layout.active_slot())? {
        let active: aster_upgrade_core::ActiveReleaseSlot =
            serde_json::from_slice(&read_regular(&layout.active_slot(), 8192)?)
                .map_err(|_| failed("active slot metadata is invalid"))?;
        active
            .validate()
            .map_err(|_| failed("active slot metadata is invalid"))?;
        Some(active)
    } else {
        None
    };
    let (identity_path, environment_path) = match active
        .as_ref()
        .filter(|active| active.local_runner.is_some())
    {
        Some(active) => (
            layout.runner_slot_identity(active.slot.id()),
            layout.runner_slot_environment(active.slot.id()),
        ),
        None => (layout.runner_identity(), layout.runner_environment()),
    };
    let shared_identity = optional_file(&identity_path)?;
    let shared_environment = optional_file(&environment_path)?;
    if !shared_identity && !shared_environment {
        return Ok(None);
    }
    if !shared_identity || !shared_environment {
        return Err(failed("existing local Runner material is incomplete"));
    }
    if job_id.is_empty()
        || job_id.len() > 160
        || !job_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(failed("invalid local-slot upgrade job"));
    }
    if actor_id.is_empty()
        || actor_id.len() > 128
        || !actor_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(failed("invalid local-slot upgrade actor"));
    }
    #[cfg(target_os = "linux")]
    if !layout
        .root()
        .to_string_lossy()
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"/._+-:".contains(&byte))
    {
        return Err(failed(
            "local slots require an installation root without whitespace or shell metacharacters",
        ));
    }
    let mode = preparation_mode(layout, release, actor_id, job_id)?;
    #[cfg(target_os = "linux")]
    if mode == UpgradeMode::BlueGreen
        && let Some(runner) = online::resume(layout, release, slot, actor_id, job_id)?
    {
        return Ok(Some(runner));
    }
    assert_candidate_stopped(slot, super::service_state)?;
    #[cfg(target_os = "linux")]
    let preparation = if mode == UpgradeMode::BlueGreen {
        if !activate_runtime {
            return Err(failed(
                "online preparation requires the signed runtime capability",
            ));
        }
        Some(online::begin(
            layout,
            release,
            slot,
            actor_id,
            job_id,
            active
                .as_ref()
                .ok_or_else(|| failed("online preparation requires an active runtime slot"))?,
        )?)
    } else {
        None
    };
    let command_deadline = Instant::now() + Duration::from_secs(90);
    let plan = make_plan(layout, slot, mode)?;
    let identity_output = layout
        .runner_slot_identity_output(slot.id())
        .with_extension(format!("{job_id}.json"));
    let keys_output = layout.runner_slot_task_keys_output(slot.id());
    for path in [
        &identity_output,
        &keys_output,
        &layout.runner_slot_identity(slot.id()),
        &layout.runner_slot_task_keys(slot.id()),
        &layout.runner_slot_environment(slot.id()),
        &layout.runner_slot_upstream_ca(slot.id()),
        &layout.control_slot_runtime_token(slot.id()),
    ] {
        safe_owned_path(layout, path)?;
    }
    let directory = layout
        .runner_slot_environment(slot.id())
        .parent()
        .ok_or_else(|| failed("slot configuration has no parent"))?
        .to_path_buf();
    owned_directory(
        directory
            .parent()
            .ok_or_else(|| failed("slot directory has no parent"))?,
        "aster-runner",
    )?;
    owned_directory(&directory, "aster-runner")?;
    let previous_output = layout
        .runner_slot_identity_output(slot.id())
        .with_extension(format!("{job_id}.previous.json"));
    let previous_identity = read_regular(&identity_path, 4096)?;
    owned_file(layout, previous_output, &previous_identity, "aster-team")?;
    let mut command = control_command(layout, release);
    command
        .args([
            "prepare-local-runner-slot",
            "--actor-id",
            actor_id,
            "--job-id",
            job_id,
            "--slot",
            slot.id(),
            "--identity-output",
        ])
        .arg(&identity_output);
    run_preparation_command(
        mode,
        command_deadline,
        command,
        "prepare local-slot identity",
    )?;
    let mut command = control_command(layout, release);
    command
        .arg("export-runner-task-keys")
        .arg("--target")
        .arg(&keys_output);
    run_preparation_command(
        mode,
        command_deadline,
        command,
        "export local-slot task keys",
    )?;
    let identity = read_regular(&identity_output, 4096)?;
    let task_keys = read_regular(&keys_output, 16 * 1024)?;
    owned_file(
        layout,
        layout.runner_slot_identity(slot.id()),
        &identity,
        "aster-runner",
    )?;
    owned_file(
        layout,
        layout.runner_slot_task_keys(slot.id()),
        &task_keys,
        "aster-runner",
    )?;
    if let Some(certificate) = &plan.upstream_ca {
        owned_file(
            layout,
            layout.runner_slot_upstream_ca(slot.id()),
            certificate,
            "aster-runner",
        )?;
    }
    owned_file(
        layout,
        layout.runner_slot_environment(slot.id()),
        plan.runner_environment.as_bytes(),
        "aster-runner",
    )?;
    let mut command = Command::new("runuser");
    command.env_remove("ASTER_RUNNER_UPSTREAM_CA_CERTIFICATE");
    command
        .args(["-u", "aster-runner", "--"])
        .arg(layout.release_binary(release, "aster-runner"))
        .args([
            "preflight-slot",
            "--slot",
            slot.id(),
            "--max-inflight",
            &plan.capacity.to_string(),
            "--heartbeat-seconds",
            &plan.heartbeat.to_string(),
        ]);
    if plan.upstream_ca.is_some() {
        command
            .arg("--upstream-ca-certificate")
            .arg(layout.runner_slot_upstream_ca(slot.id()));
    }
    for host in RUNNER_ALLOWED_UPSTREAM_HOSTS {
        command.args(["--allowed-upstream-host", host]);
    }
    run_preparation_command(
        mode,
        command_deadline,
        command,
        "preflight local-slot Runner",
    )?;
    let token_path = layout.control_slot_runtime_token(slot.id());
    let token = if optional_file(&token_path)? {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::symlink_metadata(&token_path)
                .map_err(|error| failed(error.to_string()))?
                .permissions()
                .mode();
            if mode & 0o027 != 0 {
                return Err(failed(
                    "existing local-slot runtime token has unsafe permissions",
                ));
            }
        }
        let value = read_regular(&token_path, 32)?;
        if value.len() != 32 {
            return Err(failed("invalid local-slot runtime token"));
        }
        value
    } else {
        let mut value = Zeroizing::new(vec![0; 32]);
        getrandom::fill(value.as_mut_slice())
            .map_err(|_| failed("operating-system randomness unavailable"))?;
        value
    };
    owned_file(layout, token_path, &token, "aster-team")?;
    owned_file(
        layout,
        layout.control_slot_environment(slot.id()),
        plan.control_environment.as_bytes(),
        "aster-team",
    )?;
    if !activate_runtime {
        return Ok(None);
    }
    #[derive(Deserialize)]
    struct Identity {
        runner_id: String,
    }
    let identity: Identity = serde_json::from_slice(&identity)
        .map_err(|_| failed("invalid prepared Runner identity"))?;
    let runner_id = identity.runner_id;
    let manifest_sha256 = super::sha256_file(&release.join("RELEASE.json"))?;
    let runner = aster_upgrade_core::ActiveLocalRunner {
        runner_id,
        manifest_sha256,
    };
    #[cfg(target_os = "linux")]
    if let Some(preparation) = preparation {
        preparation.finish(layout, runner.clone())?;
    }
    Ok(Some(runner))
}

pub(crate) fn run_preparation_command(
    mode: UpgradeMode,
    deadline: Instant,
    command: Command,
    label: &str,
) -> Result<(), CliFailure> {
    if mode == UpgradeMode::BlueGreen {
        if Instant::now() >= deadline {
            return Err(failed("online preparation command budget exhausted"));
        }
        #[cfg(target_os = "linux")]
        return super::preparation_command::run(command, deadline, label);
        #[cfg(not(target_os = "linux"))]
        return Err(failed("online preparation commands require Linux"));
    }
    run_checked(command, delivery::UPGRADE_FAILED, label)
}

fn valid_job_id(job_id: &str) -> bool {
    !job_id.is_empty()
        && job_id.len() <= 160
        && job_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn prepared_slot(layout: &InstallLayout, job_id: &str) -> Result<Option<ReleaseSlot>, CliFailure> {
    if !valid_job_id(job_id) {
        return Err(failed("invalid local-slot upgrade job"));
    }
    let mut found = None;
    for slot in [ReleaseSlot::Blue, ReleaseSlot::Green] {
        let path = layout
            .runner_slot_provisioning(slot.id())
            .with_extension(format!("{job_id}.json"));
        safe_owned_path(layout, &path)?;
        if optional_file(&path)? && found.replace(slot).is_some() {
            return Err(failed("multiple candidates claim one upgrade job"));
        }
    }
    Ok(found)
}

/// Called with the installation upgrade lock held, after persisting the final
/// active slot and terminal job, but before moving the job out of running.
/// A supported package always queries signed database ownership: absence of a
/// private journal never proves that registration did not commit.
pub(super) fn finalize_if_supported(
    layout: &InstallLayout,
    job: &aster_upgrade_core::MaintenanceJob,
) -> Result<(), CliFailure> {
    use aster_upgrade_core::{ActiveReleaseSlot, MaintenanceOperation};
    if !matches!(job.operation, MaintenanceOperation::Upgrade { .. }) {
        return Ok(());
    }
    let prepared = prepared_slot(layout, &job.id)?;
    if !job.status.terminal() {
        return Err(failed(
            "Runner finalization requires a terminal upgrade job",
        ));
    }
    let Some(release) = job.candidate_release.as_ref() else {
        return if prepared.is_none() {
            Ok(())
        } else {
            Err(failed("prepared candidate release is missing"))
        };
    };
    if !supports_preparation(release)?
        || !matches!(
            DatabaseConfiguration::load(layout).map_err(|error| failed(error.to_string()))?,
            DatabaseConfiguration::Mariadb { .. }
        )
    {
        return if prepared.is_none() {
            Ok(())
        } else {
            Err(failed(
                "prepared Runner requires its signed MariaDB slot contract",
            ))
        };
    }
    super::verify_release_at(release)?;
    let active: ActiveReleaseSlot =
        serde_json::from_slice(&read_regular(&layout.active_slot(), 8192)?)
            .map_err(|_| failed("active slot metadata is invalid"))?;
    active
        .validate()
        .map_err(|_| failed("active slot metadata is invalid"))?;
    let candidate_active = job.target_version.as_deref() == Some(active.version.as_str());
    if !candidate_active && active.version != job.current_version {
        return Err(failed("active slot does not match the upgrade outcome"));
    }
    let slot = if candidate_active {
        active.slot
    } else {
        active.slot.other()
    };
    if prepared.is_some_and(|prepared| prepared != slot) {
        return Err(failed("prepared Runner does not match the upgrade slot"));
    }
    assert_candidate_stopped(active.slot.other(), super::service_state)?;
    if candidate_active
        && active.local_runner.is_some()
        && !matches!(
            super::service_state("aster-runner.service")?.as_str(),
            "inactive" | "failed" | "disabled" | "not-installed"
        )
    {
        return Err(failed(
            "the previous shared Runner is still running or has an unknown state",
        ));
    }
    if super::service_state(&format!("aster-control@{}.service", active.slot.id()))? != "active" {
        return Err(failed(
            "the selected Control is not running; retaining the upgrade job",
        ));
    }
    let identity_path = if active.local_runner.is_some() {
        layout.runner_slot_identity(active.slot.id())
    } else {
        layout.runner_identity()
    };
    let identity = if optional_file(&identity_path)? {
        Some(read_regular(&identity_path, 4096)?)
    } else {
        None
    };
    let output = layout
        .runner_slot_identity_output(slot.id())
        .with_extension(format!("{}.survivor.json", job.id));
    // Keep credential bytes in zeroizing buffers; the Control command performs
    // the strict JSON/schema and signed-binding validation before any mutation.
    let mut input =
        Zeroizing::new(br#"{"schema":"aster.runner-upgrade-survivor.v1","survivor":"#.to_vec());
    input.extend_from_slice(
        identity
            .as_ref()
            .map_or(b"null".as_slice(), |bytes| bytes.as_slice()),
    );
    input.push(b'}');
    owned_file(layout, output, &input, "aster-team")?;
    let mut command = control_command(layout, release);
    command.args([
        "finalize-local-runner-slot",
        "--job-id",
        &job.id,
        "--slot",
        slot.id(),
    ]);
    // A lost DB response keeps the terminal task and its preparation guard.
    // Online recovery uses the same bounded process-group ownership as prepare;
    // timeout must never be interpreted as an unregistered Runner or success.
    run_preparation_command(
        job.upgrade_mode.unwrap_or(UpgradeMode::Maintenance),
        Instant::now() + Duration::from_secs(90),
        command,
        "finalize local Runner upgrade",
    )
}

/// Only job-specific private material is removed. Stable slot configuration and
/// lock files may already belong to a later upgrade and must never be touched.
fn cleanup_job_material(layout: &InstallLayout, job_id: &str) -> Result<(), CliFailure> {
    if !valid_job_id(job_id) {
        return Err(failed("invalid completed local-slot job"));
    }
    for slot in [ReleaseSlot::Blue, ReleaseSlot::Green] {
        let output = layout.runner_slot_identity_output(slot.id());
        let paths = [
            output.with_extension(format!("{job_id}.json")),
            output.with_extension(format!("{job_id}.previous.json")),
            output.with_extension(format!("{job_id}.survivor.json")),
            layout
                .runner_slot_provisioning(slot.id())
                .with_extension(format!("{job_id}.json")),
        ];
        for path in paths {
            safe_owned_path(layout, &path)?;
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(failed(format!(
                        "cannot clean completed upgrade material: {error}"
                    )));
                }
            }
        }
    }
    Ok(())
}

/// A crash after archival but during private-file removal is retried on the next
/// executor run. Unarchived jobs retain every secret needed to reconcile them.
pub(super) fn cleanup_completed_material(layout: &InstallLayout) -> Result<(), CliFailure> {
    #[cfg(unix)]
    fs::File::open(layout.upgrade_completed())
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            failed(format!(
                "cannot sync completed upgrade records before cleanup: {error}"
            ))
        })?;
    for entry in
        fs::read_dir(layout.upgrade_completed()).map_err(|error| failed(error.to_string()))?
    {
        let entry = entry.map_err(|error| failed(error.to_string()))?;
        if entry
            .path()
            .extension()
            .is_none_or(|extension| extension != "json")
        {
            continue;
        }
        let job: aster_upgrade_core::MaintenanceJob =
            serde_json::from_slice(&read_regular(&entry.path(), 256 * 1024)?)
                .map_err(|_| failed("completed upgrade job is invalid"))?;
        job.validate()
            .map_err(|_| failed("completed upgrade job is invalid"))?;
        if !job.status.terminal()
            || entry.path().file_stem().and_then(|name| name.to_str()) != Some(job.id.as_str())
        {
            return Err(failed(
                "completed upgrade job identity or status is invalid",
            ));
        }
        cleanup_job_material(layout, &job.id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, InstallLayout) {
        let temporary = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(temporary.path()).unwrap();
        fs::create_dir_all(layout.control_config()).unwrap();
        fs::create_dir_all(layout.runner_config()).unwrap();
        for slot in [ReleaseSlot::Blue, ReleaseSlot::Green] {
            let ports = slot.ports();
            fs::write(layout.control_slot_environment(slot.id()), format!("# preserved\nASTER_CONTROL_LISTEN=127.0.0.1:{}\nASTER_CONTROL_MEMBER_LISTEN=127.0.0.1:{}\nASTER_CONTROL_ADMIN_LISTEN=127.0.0.1:{}\nRUST_LOG=warn\n", ports.api, ports.member, ports.admin)).unwrap();
        }
        fs::write(layout.runner_environment(), "ASTER_RUNNER_CONTROL_WSS=wss://public.example/api/runner/channel\nASTER_RUNNER_CONTROL_CA_CERTIFICATE=/shared/control-ca.pem\nASTER_RUNNER_MAX_INFLIGHT=8\n").unwrap();
        (temporary, layout)
    }

    fn terminal_job(layout: &InstallLayout, id: &str) -> aster_upgrade_core::MaintenanceJob {
        use aster_upgrade_core::*;
        MaintenanceJob {
            schema: MAINTENANCE_JOB_SCHEMA.into(),
            id: id.into(),
            requested_by: "owner".into(),
            operation: MaintenanceOperation::Upgrade {
                archive: layout.root().join("candidate.tar.gz"),
                archive_sha256: "a".repeat(64),
            },
            status: MaintenanceStatus::Failed,
            upgrade_mode: Some(UpgradeMode::Maintenance),
            runner_was_running: None,
            current_version: "2.0.0".into(),
            target_version: Some("2.0.1".into()),
            previous_release: None,
            candidate_release: None,
            message: String::new(),
            created_at: "2026-09-08T00:00:00.000Z".into(),
            updated_at: "2026-09-08T00:00:00.000Z".into(),
        }
    }

    #[test]
    fn finalization_cannot_skip_a_prepared_job_or_accept_multiple_candidates() {
        let (_directory, layout) = fixture();
        fs::create_dir_all(layout.runtime()).unwrap();
        let job = terminal_job(&layout, "upgrade_one");
        finalize_if_supported(&layout, &job).unwrap();
        let green = layout
            .runner_slot_provisioning("green")
            .with_extension("upgrade_one.json");
        fs::write(&green, b"private journal").unwrap();
        assert_eq!(
            prepared_slot(&layout, &job.id).unwrap(),
            Some(ReleaseSlot::Green)
        );
        assert!(finalize_if_supported(&layout, &job).is_err());
        let blue = layout
            .runner_slot_provisioning("blue")
            .with_extension("upgrade_one.json");
        fs::write(&blue, b"another candidate").unwrap();
        assert!(prepared_slot(&layout, &job.id).is_err());
        assert!(prepared_slot(&layout, "../escape").is_err());
    }

    #[test]
    fn completed_material_cleanup_preserves_running_jobs_and_stable_slot_files() {
        let (_directory, layout) = fixture();
        fs::create_dir_all(layout.runtime()).unwrap();
        fs::create_dir_all(layout.upgrade_completed()).unwrap();
        let job = terminal_job(&layout, "upgrade_completed");
        fs::write(
            layout.upgrade_completed().join("upgrade_completed.json"),
            serde_json::to_vec(&job).unwrap(),
        )
        .unwrap();
        let completed = layout
            .runner_slot_provisioning("green")
            .with_extension("upgrade_completed.json");
        let active_job = layout
            .runner_slot_provisioning("green")
            .with_extension("upgrade_running.json");
        let stable_lock = layout.runner_slot_provisioning_lock("green");
        for path in [&completed, &active_job, &stable_lock] {
            fs::write(path, b"preserve unless completed").unwrap();
        }
        cleanup_completed_material(&layout).unwrap();
        cleanup_completed_material(&layout).unwrap();
        assert!(!completed.exists());
        assert!(active_job.exists());
        assert!(stable_lock.exists());
        assert!(
            layout
                .upgrade_completed()
                .join("upgrade_completed.json")
                .exists()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn old_runtime_can_be_recovered_but_cannot_enter_new_online_preparation() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("systemd")).unwrap();
        for (path, contents) in [
            (
                CAPABILITIES,
                include_str!("../../../deploy/systemd/local-slot-preparation.json"),
            ),
            (
                "systemd/local-slot-runtime.json",
                include_str!("../../../deploy/systemd/local-slot-runtime.json"),
            ),
            ("systemd/aster-runner@.service", "fixture"),
        ] {
            fs::write(root.path().join(path), contents).unwrap();
        }
        require_recovery_support(root.path()).unwrap();
        assert!(require_online_support(root.path()).is_err());
        let marker = root
            .path()
            .join(aster_upgrade_core::settlement::CAPABILITY_FILE);
        fs::write(&marker, aster_upgrade_core::settlement::CAPABILITY_JSON).unwrap();
        require_online_support(root.path()).unwrap();
        fs::write(&marker, "{}").unwrap();
        assert!(require_online_support(root.path()).is_err());
        require_recovery_support(root.path()).unwrap();
    }

    #[test]
    fn old_releases_skip_preparation_and_partial_new_contracts_fail_closed() {
        let (temporary, _) = fixture();
        assert!(!supports_preparation(temporary.path()).unwrap());
        fs::create_dir(temporary.path().join("systemd")).unwrap();
        fs::write(
            temporary.path().join(CAPABILITIES),
            include_str!("../../../deploy/systemd/local-slot-preparation.json"),
        )
        .unwrap();
        assert!(supports_preparation(temporary.path()).is_err());
        fs::write(
            temporary.path().join("systemd/aster-runner@.service"),
            "service",
        )
        .unwrap();
        assert!(supports_preparation(temporary.path()).unwrap());
        fs::write(temporary.path().join(CAPABILITIES), "{}").unwrap();
        assert!(supports_preparation(temporary.path()).is_err());
    }

    #[test]
    fn runtime_activation_requires_its_own_backward_compatible_package_declaration() {
        let directory = tempfile::tempdir().unwrap();
        assert!(!supports_slot_runtime(directory.path()).unwrap());
        fs::create_dir(directory.path().join("systemd")).unwrap();
        let path = directory.path().join("systemd/local-slot-runtime.json");
        fs::write(
            &path,
            include_str!("../../../deploy/systemd/local-slot-runtime.json"),
        )
        .unwrap();
        assert!(supports_slot_runtime(directory.path()).unwrap());
        fs::write(
            &path,
            r#"{"schema":"aster.local-slot-runtime.v1","active_slot_schema":"unknown"}"#,
        )
        .unwrap();
        assert!(supports_slot_runtime(directory.path()).is_err());
        // Previous executors reject unknown preparation fields, so retain the original shape.
        let preparation: serde_json::Value = serde_json::from_str(include_str!(
            "../../../deploy/systemd/local-slot-preparation.json"
        ))
        .unwrap();
        assert_eq!(preparation.as_object().unwrap().len(), 4);
    }

    #[test]
    fn maintenance_skips_sqlite_old_packages_and_deployments_without_local_runner() {
        let (temporary, layout) = fixture();
        let unchanged = fs::read(layout.control_slot_environment("green")).unwrap();
        fs::write(
            layout.database_configuration(),
            br#"{"driver":"sqlcipher"}"#,
        )
        .unwrap();
        prepare_if_supported(
            &layout,
            temporary.path(),
            ReleaseSlot::Green,
            "owner",
            "job_one",
        )
        .unwrap();
        let database = DatabaseConfiguration::Mariadb {
            host: "127.0.0.1".into(),
            port: 3306,
            database: "aster_team".into(),
            username: "aster_team".into(),
            tls: false,
            custom_ca: false,
            max_connections: 2,
        };
        fs::write(
            layout.database_configuration(),
            serde_json::to_vec(&database).unwrap(),
        )
        .unwrap();
        prepare_if_supported(
            &layout,
            temporary.path(),
            ReleaseSlot::Green,
            "owner",
            "job_one",
        )
        .unwrap();
        fs::create_dir(temporary.path().join("systemd")).unwrap();
        fs::write(
            temporary.path().join(CAPABILITIES),
            include_str!("../../../deploy/systemd/local-slot-preparation.json"),
        )
        .unwrap();
        fs::write(
            temporary.path().join("systemd/aster-runner@.service"),
            "service",
        )
        .unwrap();
        assert!(
            prepare_if_supported(
                &layout,
                temporary.path(),
                ReleaseSlot::Green,
                "owner",
                "job_one"
            )
            .is_err()
        );
        fs::remove_file(layout.runner_environment()).unwrap();
        prepare_if_supported(
            &layout,
            temporary.path(),
            ReleaseSlot::Green,
            "owner",
            "job_one",
        )
        .unwrap();
        assert_eq!(
            fs::read(layout.control_slot_environment("green")).unwrap(),
            unchanged
        );
        assert!(!layout.runner_slot_identity("green").exists());
        assert!(!layout.control_slot_runtime_token("green").exists());
    }

    #[test]
    fn preparation_startup_mode_comes_from_the_matching_running_job() {
        let (_temporary, layout) = fixture();
        fs::create_dir_all(layout.upgrade_running()).unwrap();
        let release = layout.release("2.1.0");
        let path = layout.upgrade_running().join("job_one.json");
        let mut job = serde_json::json!({
            "schema": aster_upgrade_core::MAINTENANCE_JOB_SCHEMA,
            "id": "job_one", "requested_by": "owner", "status": "starting_candidate",
            "operation": {"type": "upgrade", "archive": layout.upgrade_uploads().join("job_one.tar.gz"), "archive_sha256": "a".repeat(64)},
            "upgrade_mode": "blue_green", "runner_was_running": true,
            "current_version": "2.0.1", "target_version": "2.1.0",
            "previous_release": layout.release("2.0.1"), "candidate_release": release,
            "message": "preparing", "created_at": "2026-09-09T00:00:00Z", "updated_at": "2026-09-09T00:00:00Z"
        });
        for (encoded, expected) in [
            (serde_json::json!("blue_green"), UpgradeMode::BlueGreen),
            (serde_json::json!("maintenance"), UpgradeMode::Maintenance),
            (serde_json::Value::Null, UpgradeMode::Maintenance),
        ] {
            job["upgrade_mode"] = encoded;
            fs::write(&path, serde_json::to_vec(&job).unwrap()).unwrap();
            assert_eq!(
                preparation_mode(&layout, &release, "owner", "job_one").unwrap(),
                expected
            );
        }
        for (field, value) in [
            ("id", serde_json::json!("other_job")),
            ("requested_by", serde_json::json!("other_owner")),
            ("status", serde_json::json!("switching_traffic")),
            (
                "candidate_release",
                serde_json::json!(layout.release("9.0.0")),
            ),
            ("target_version", serde_json::json!("9.0.0")),
            (
                "operation",
                serde_json::json!({"type":"delete_version", "version":"2.0.1"}),
            ),
        ] {
            let mut changed = job.clone();
            changed[field] = value;
            fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
            assert!(preparation_mode(&layout, &release, "owner", "job_one").is_err());
        }
    }

    #[test]
    fn online_candidate_starts_closed_and_commit_preserves_the_other_settings() {
        let (_temporary, layout) = fixture();
        let closed = make_plan(&layout, ReleaseSlot::Green, UpgradeMode::BlueGreen).unwrap();
        assert_eq!(
            environment_value(
                &closed.control_environment,
                "ASTER_CONTROL_RUNTIME_CANDIDATE"
            ),
            Some("true")
        );
        let committed =
            committed_control_environment(&closed.control_environment, ReleaseSlot::Green).unwrap();
        let maintenance = make_plan(&layout, ReleaseSlot::Green, UpgradeMode::Maintenance).unwrap();
        assert_eq!(committed, maintenance.control_environment);
        assert_eq!(
            committed_control_environment(&committed, ReleaseSlot::Green).unwrap(),
            committed
        );
        for fault in [
            closed
                .control_environment
                .replace("CANDIDATE=true", "CANDIDATE=maybe"),
            closed
                .control_environment
                .replace("SLOT=green", "SLOT=blue"),
            closed
                .control_environment
                .replace("ASTER_CONTROL_RUNTIME_CANDIDATE=true\n", ""),
            format!(
                "{}ASTER_CONTROL_RUNTIME_CANDIDATE=false\n",
                closed.control_environment
            ),
            format!(
                "{}ASTER_CONTROL_RUNTIME_SLOT=green\n",
                closed.control_environment
            ),
        ] {
            assert!(committed_control_environment(&fault, ReleaseSlot::Green).is_err());
        }
    }

    #[test]
    fn candidate_environment_preserves_custom_control_settings_without_public_runner_routing() {
        let (_temporary, layout) = fixture();
        let original_blue = fs::read(layout.control_slot_environment("blue")).unwrap();
        let plan = make_plan(&layout, ReleaseSlot::Green, UpgradeMode::Maintenance).unwrap();
        assert!(plan.control_environment.contains("RUST_LOG=warn\n"));
        assert!(
            plan.control_environment
                .contains("ASTER_CONTROL_RUNTIME_LISTEN=127.0.0.1:11483\n")
        );
        assert!(
            plan.control_environment
                .contains("ASTER_CONTROL_RUNTIME_CANDIDATE=false\n")
        );
        assert!(!plan.runner_environment.contains("CONTROL_WSS"));
        assert!(!plan.runner_environment.contains("CONTROL_CA"));
        assert_eq!(plan.capacity, 8);
        fs::write(
            layout.control_slot_environment("green"),
            &plan.control_environment,
        )
        .unwrap();
        let repeated = make_plan(&layout, ReleaseSlot::Green, UpgradeMode::Maintenance).unwrap();
        assert_eq!(repeated.control_environment, plan.control_environment);
        assert_eq!(
            fs::read(layout.control_slot_environment("blue")).unwrap(),
            original_blue
        );
    }

    #[test]
    fn later_upgrades_copy_active_slot_runner_settings() {
        let (_temporary, layout) = fixture();
        fs::create_dir_all(layout.active_slot().parent().unwrap()).unwrap();
        fs::write(
            layout.active_slot(),
            serde_json::to_vec(&aster_upgrade_core::ActiveReleaseSlot {
                schema: aster_upgrade_core::ACTIVE_SLOT_RUNTIME_SCHEMA.into(),
                slot: ReleaseSlot::Blue,
                version: "2.1.0".into(),
                local_runner: Some(aster_upgrade_core::ActiveLocalRunner {
                    runner_id: format!("runner_{}", "a".repeat(32)),
                    manifest_sha256: "b".repeat(64),
                }),
            })
            .unwrap(),
        )
        .unwrap();
        fs::create_dir_all(layout.runner_slot_environment("blue").parent().unwrap()).unwrap();
        fs::write(
            layout.runner_slot_environment("blue"),
            "ASTER_RUNNER_MAX_INFLIGHT=12\nASTER_RUNNER_HEARTBEAT_SECONDS=7\n",
        )
        .unwrap();
        let plan = make_plan(&layout, ReleaseSlot::Green, UpgradeMode::Maintenance).unwrap();
        assert_eq!((plan.capacity, plan.heartbeat), (12, 7));
        fs::remove_file(layout.runner_slot_environment("blue")).unwrap();
        assert!(
            make_plan(&layout, ReleaseSlot::Green, UpgradeMode::Maintenance).is_err(),
            "must not fall back to stale shared settings"
        );
    }

    #[test]
    fn invalid_listeners_capacity_and_ca_are_not_silently_rewritten() {
        let (_temporary, layout) = fixture();
        fs::write(layout.runner_environment(), "ASTER_RUNNER_MAX_INFLIGHT=0\n").unwrap();
        assert!(make_plan(&layout, ReleaseSlot::Blue, UpgradeMode::Maintenance).is_err());
        fs::write(
            layout.runner_environment(),
            "ASTER_RUNNER_UPSTREAM_CA_CERTIFICATE=relative.pem\n",
        )
        .unwrap();
        assert!(make_plan(&layout, ReleaseSlot::Blue, UpgradeMode::Maintenance).is_err());
        fs::write(layout.runner_environment(), "").unwrap();
        fs::write(
            layout.control_slot_environment("blue"),
            "ASTER_CONTROL_LISTEN=0.0.0.0:11380\n",
        )
        .unwrap();
        assert!(make_plan(&layout, ReleaseSlot::Blue, UpgradeMode::Maintenance).is_err());
    }

    #[test]
    fn upstream_ca_is_snapshotted_into_the_candidate_slot() {
        let (temporary, layout) = fixture();
        let ca = temporary.path().join("upstream.pem");
        fs::write(&ca, "fixture certificate").unwrap();
        fs::write(
            layout.runner_environment(),
            format!("ASTER_RUNNER_UPSTREAM_CA_CERTIFICATE={}\n", ca.display()),
        )
        .unwrap();
        let plan = make_plan(&layout, ReleaseSlot::Blue, UpgradeMode::Maintenance).unwrap();
        assert_eq!(
            plan.upstream_ca.as_deref(),
            Some(b"fixture certificate".as_slice())
        );
        assert!(
            plan.runner_environment
                .contains(&layout.runner_slot_upstream_ca("blue").display().to_string())
        );
    }

    #[test]
    fn a_running_or_unknown_candidate_is_never_prepared() {
        for unsafe_state in ["active", "activating", "deactivating", "unknown"] {
            for target in ["aster-control@green.service", "aster-runner@green.service"] {
                assert!(
                    assert_candidate_stopped(ReleaseSlot::Green, |unit| Ok(if unit == target {
                        unsafe_state
                    } else {
                        "inactive"
                    }
                    .into()))
                    .is_err()
                );
            }
        }
        assert!(
            assert_candidate_stopped(ReleaseSlot::Blue, |_| Ok("not-installed".into())).is_ok()
        );
    }
}
