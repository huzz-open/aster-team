#![forbid(unsafe_code)]

use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, IsTerminal as _, Read as _, Write as _},
    net::{IpAddr, UdpSocket},
    path::{Component, Path, PathBuf},
    process::{Command as ProcessCommand, ExitStatus},
};

use aster_error_catalog::{ErrorDescriptor, delivery};
use aster_install_layout::{InstallLayout, Platform, WindowsPorts, executable_name};
use aster_release_core::{
    RELEASE_MANIFEST_FILE, TrustedReleaseKeys, VerifiedRelease, verify, verify_release_subset,
    verify_release_tree,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use clap::{Args, Parser, Subcommand, ValueEnum};
use image::Luma;
use qrcode::{Color, EcLevel, QrCode, Version, bits::Bits};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

#[cfg(any(target_os = "linux", test))]
mod control_process;
mod maintenance_executor;
#[cfg(target_os = "linux")]
mod online_commit;
#[cfg(target_os = "linux")]
mod online_deployment;
mod online_journal;
#[cfg(target_os = "linux")]
mod preparation_command;
#[cfg(any(target_os = "linux", test))]
mod proxy_client;
#[cfg(any(target_os = "linux", test))]
mod proxy_disk;
#[cfg(any(target_os = "linux", test))]
mod runner_process;
#[cfg(any(target_os = "linux", test))]
mod runtime_client;
#[cfg(any(target_os = "linux", test))]
mod slot_preparation;
#[cfg(target_os = "linux")]
mod upgrade_clock;
#[cfg(any(target_os = "windows", test))]
mod windows_backup;

const SELECTED_RELEASE_SCHEMA: &str = "aster.team.selected-release/v1";
const DEFAULT_OWNER_EMAIL: &str = "admin@example.com";
const ACCESS_CONFIG_SCHEMA: &str = "aster.team.access/v1";
const LICENSE_REQUEST_FILE_PREFIX: &str = "aster-team-license-request";
const LICENSE_REQUEST_QR_MAGIC: [u8; 4] = *b"ALR\x03";
const LICENSE_REQUEST_QR_FIXED_BYTES: usize = 123;
const LICENSE_REQUEST_FILE_TIME_FORMAT: &[time::format_description::FormatItem<'static>] =
    time::macros::format_description!("[year][month][day]T[hour][minute][second]Z");
const BACKUP_FILE_TIME_FORMAT: &[time::format_description::FormatItem<'static>] =
    time::macros::format_description!("[year][month][day]T[hour][minute][second]Z");
const RUNNER_ALLOWED_UPSTREAM_HOSTS: [&str; 7] = [
    "api.openai.com",
    "auth.openai.com",
    "api.anthropic.com",
    "chatgpt.com",
    "api.deepseek.com",
    "open.bigmodel.cn",
    "api.z.ai",
];
fn install_layout() -> InstallLayout {
    InstallLayout::discover_or_default()
        .unwrap_or_else(|error| panic!("invalid Aster Team install root: {error}"))
}

fn cli_path() -> PathBuf {
    install_layout().stable_cli()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn service_registration_root() -> Result<PathBuf, CliFailure> {
    install_layout()
        .service_registration_root()
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?
        .ok_or_else(|| {
            CliFailure::new(
                delivery::SERVICE_FAILED,
                "this platform does not use filesystem service registrations",
            )
        })
}

fn preserved_control_required() -> Result<Vec<PathBuf>, CliFailure> {
    let layout = install_layout();
    let mut paths = vec![
        layout.control_role(),
        layout.access_configuration(),
        layout.control_environment(),
        layout.control_slot_environment("blue"),
        layout.control_slot_environment("green"),
        layout.installation_key(),
        layout.runner_task_key(),
        layout.control_runner_task_keys(),
        layout.installation_profile(),
    ];
    paths.extend(installed_database()?.required_files(&layout));
    Ok(paths)
}

fn installed_database() -> Result<aster_install_layout::DatabaseConfiguration, CliFailure> {
    let config = aster_install_layout::DatabaseConfiguration::load(&install_layout())
        .map_err(|error| CliFailure::new(delivery::INPUT_INVALID, error.to_string()))?;
    config
        .validate_platform(Platform::current(), std::env::consts::ARCH)
        .map_err(|error| CliFailure::new(delivery::INPUT_INVALID, error.to_string()))?;
    Ok(config)
}

fn preserved_control_optional_sentinels() -> Vec<PathBuf> {
    let layout = install_layout();
    vec![
        layout.database_configuration(),
        layout.database_password(),
        layout.database_ca_certificate(),
        layout.initial_owner_credentials(),
        layout.initialization_complete(),
        layout.license_request(),
        layout.license_file(),
        layout.runner_role(),
        layout.runner_identity(),
        layout.runner_environment(),
        layout.runner_task_keys(),
        layout.caddyfile(),
        layout.caddy_upstreams(),
    ]
}

mod compiled_keys {
    include!(concat!(env!("OUT_DIR"), "/trusted_release_keys.rs"));
}

mod build_info {
    include!(concat!(env!("OUT_DIR"), "/build_info.rs"));
}

#[derive(Debug, Parser)]
#[command(
    name = "aster-team-cli",
    version,
    about = "Aster Team Customer lifecycle manager"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Show CLI build metadata and the installed Aster Team version.
    Version,
    /// Install a Control host from the release selected by init.sh.
    Install(InstallArgs),
    /// Upgrade a Control host from the release selected by init.sh.
    Upgrade,
    /// Show the installed role, version, endpoints, and important paths.
    Info,
    /// Show the Control, Runner, license, and database status.
    Status,
    /// Start, stop, or restart Aster Team services.
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
    /// Read the structured service journal.
    Logs(LogsArgs),
    /// Find one gateway request in the Control service log.
    Trace(TraceArgs),
    /// Run host, identity, database, license, TLS, and service diagnostics.
    Doctor {
        /// Include successful child-process output.
        #[arg(long)]
        verbose: bool,
    },
    /// Manage the offline Customer license.
    License {
        #[command(subcommand)]
        command: LicenseCommand,
    },
    /// Reset a Customer administrator password on this Control host.
    Password {
        #[command(subcommand)]
        command: PasswordCommand,
    },
    /// Create or restore a protected host backup.
    Backup {
        #[command(subcommand)]
        command: BackupCommand,
    },
    /// Install, enroll, inspect, upgrade, or back up a dedicated Runner.
    Runner {
        #[command(subcommand)]
        command: RunnerCommand,
    },
    /// Remove Aster Team services and programs, preserving data by default.
    Uninstall {
        /// Also delete local configuration and data after an explicit confirmation.
        #[arg(long)]
        purge: bool,
    },
    /// Verify and select a signed release, then install this CLI.
    #[command(hide = true)]
    Bootstrap(BootstrapArgs),
    /// Verify a signed release tree without changing the host.
    #[command(hide = true)]
    VerifyRelease(VerifyReleaseArgs),
    /// Resolve persisted Windows instance configuration for lifecycle scripts.
    #[cfg(target_os = "windows")]
    #[command(hide = true)]
    WindowsInstance {
        #[arg(long)]
        install_root: Option<PathBuf>,
        #[arg(long)]
        initialize: bool,
    },
    /// Preflight a Windows backup with the installed CLI, never archive code.
    #[cfg(target_os = "windows")]
    #[command(hide = true)]
    PrepareWindowsRestore {
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        destination: PathBuf,
        #[arg(long)]
        install_root: PathBuf,
        #[arg(long)]
        runner_only: bool,
    },
    /// Execute one queued host maintenance job. Invoked by the platform scheduler.
    #[command(hide = true)]
    Maintenance {
        #[command(subcommand)]
        command: MaintenanceCommand,
    },
}

#[derive(Clone, Copy, Debug, Subcommand)]
enum MaintenanceCommand {
    RunNext,
}

#[derive(Debug, Args)]
struct InstallArgs {
    /// Disable prompts and require explicit owner input files.
    #[arg(long)]
    unattended: bool,
    /// Recover a previously uninstalled or interrupted Control installation without changing its data or credentials.
    #[arg(
        long,
        conflicts_with_all = [
            "owner_email",
            "owner_password_file",
            "install_local_runner",
            "access_protocol",
            "access_host",
            "bind_address",
            "certificate_source",
            "tls_certificate",
            "tls_private_key",
            "database_config",
            "database_password_file",
            "database_ca_certificate"
        ]
    )]
    recover_preserved: bool,
    /// Initial Control owner email (unattended mode only).
    #[arg(long, requires = "unattended")]
    owner_email: Option<String>,
    /// Root-owned 0600 file containing the initial owner password.
    #[arg(long, requires = "unattended")]
    owner_password_file: Option<PathBuf>,
    /// Install and register a Runner on the Control host.
    #[arg(long, requires = "unattended")]
    install_local_runner: bool,
    /// Linux amd64 only: non-secret database JSON for a new installation.
    #[arg(long, requires = "unattended")]
    database_config: Option<PathBuf>,
    /// Root-owned 0600 MariaDB password file, paired with --database-config.
    #[arg(long, requires_all = ["unattended", "database_config"])]
    database_password_file: Option<PathBuf>,
    /// Trusted database CA PEM when the configuration enables custom_ca.
    #[arg(long, requires_all = ["unattended", "database_config"])]
    database_ca_certificate: Option<PathBuf>,
    /// Public access protocol.
    #[arg(long, value_enum, default_value_t = AccessProtocol::Http, requires = "unattended")]
    access_protocol: AccessProtocol,
    /// Public IP or base domain. The primary LAN IPv4 address is used when omitted.
    #[arg(long, requires = "unattended")]
    access_host: Option<String>,
    /// LAN address used by Aster or its managed Caddy entry.
    #[arg(long, requires = "unattended")]
    bind_address: Option<IpAddr>,
    /// HTTPS certificate source. Defaults to Caddy's internal CA for HTTPS.
    #[arg(long, value_enum, requires = "unattended")]
    certificate_source: Option<CertificateSource>,
    /// PEM certificate chain used when --certificate-source=provided.
    #[arg(long, requires = "unattended")]
    tls_certificate: Option<PathBuf>,
    /// Unencrypted PEM private key matching --tls-certificate.
    #[arg(long, requires = "unattended")]
    tls_private_key: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum AccessProtocol {
    Http,
    Https,
}

impl AccessProtocol {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CertificateSource {
    Caddy,
    Provided,
}

impl CertificateSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Caddy => "caddy",
            Self::Provided => "provided",
        }
    }
}

#[derive(Clone, Copy, Debug, Subcommand)]
enum ServiceCommand {
    Start { target: ServiceTarget },
    Stop { target: ServiceTarget },
    Restart { target: ServiceTarget },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ServiceTarget {
    Control,
    Runner,
    All,
}

#[derive(Debug, Args)]
struct LogsArgs {
    target: LogTarget,
    #[arg(long)]
    follow: bool,
}

#[derive(Debug, Args)]
struct TraceArgs {
    /// Request ID shown in an Aster gateway error. Prompts when omitted.
    request_id: Option<String>,
    /// Search this many hours of recent service logs.
    #[arg(long, default_value_t = 24, value_parser = clap::value_parser!(u16).range(1..=720))]
    hours: u16,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum LogTarget {
    Control,
    Runner,
}

#[derive(Debug, Subcommand)]
enum LicenseCommand {
    /// Generate a machine authorization request, QR PNG, and terminal QR.
    Request {
        /// Output file. Defaults to a timestamped JSON file in the current safe directory.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Install {
        #[arg(long)]
        source: PathBuf,
    },
    Status,
}

#[derive(Debug, Subcommand)]
enum PasswordCommand {
    /// Reset one active administrator password and revoke all of its sessions.
    ResetAdmin {
        /// Administrator email. When omitted, the CLI prompts for it.
        #[arg(long)]
        email: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum BackupCommand {
    Create {
        /// New archive path. Defaults to <install-root>/backups with a UTC timestamp.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Restore {
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        confirm: bool,
    },
}

#[derive(Debug, Subcommand)]
enum RunnerCommand {
    Install,
    Enroll {
        #[arg(long)]
        control_url: String,
        #[arg(long)]
        token_file: PathBuf,
        #[arg(long)]
        control_ca_certificate: Option<PathBuf>,
        #[arg(long)]
        allow_insecure_http: bool,
    },
    Status,
    Upgrade,
    Backup {
        #[command(subcommand)]
        command: RunnerBackupCommand,
    },
}

#[derive(Debug, Subcommand)]
enum RunnerBackupCommand {
    Create {
        /// New archive path. Defaults to <install-root>/backups with a UTC timestamp.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Restore {
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        confirm: bool,
    },
}

#[derive(Debug, Args)]
struct BootstrapArgs {
    #[arg(long)]
    release_root: PathBuf,
    #[arg(long)]
    install_root: Option<PathBuf>,
    #[arg(long, hide = true)]
    install_target: Option<PathBuf>,
    #[arg(long, hide = true)]
    selection_file: Option<PathBuf>,
    #[arg(long, hide = true)]
    runner_only: bool,
}

#[derive(Debug, Args)]
struct VerifyReleaseArgs {
    #[arg(long)]
    root: PathBuf,
    #[arg(long, hide = true)]
    runner_only: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SelectedRelease {
    schema: String,
    version: String,
    release_root: PathBuf,
    manifest_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AccessConfiguration {
    schema: String,
    protocol: String,
    address_kind: String,
    host: String,
    bind_address: String,
    certificate_source: String,
    caddy_enabled: bool,
    member_url: String,
    admin_url: String,
    api_url: String,
    runner_websocket_url: String,
}

#[derive(Debug, Eq, PartialEq)]
enum PreservedControlState {
    Fresh,
    Recoverable,
    Incomplete { missing: Vec<String> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompiledReleaseKey {
    key_id: String,
    public_key_spki: String,
}

#[derive(Debug)]
struct CliFailure {
    descriptor: ErrorDescriptor,
    detail: String,
}

impl CliFailure {
    fn new(descriptor: ErrorDescriptor, detail: impl Into<String>) -> Self {
        Self {
            descriptor,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for CliFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}：{}（{} / 错误码：{}）",
            self.descriptor.default_message,
            self.detail,
            self.descriptor.code,
            self.descriptor.number
        )
    }
}

impl std::error::Error for CliFailure {}

fn main() {
    if let Err(error) = run(Cli::parse()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), CliFailure> {
    match cli.command {
        Command::Version => show_version(),
        #[cfg(target_os = "windows")]
        Command::WindowsInstance {
            install_root,
            initialize,
        } => {
            if initialize {
                require_root()?;
            }
            let layout = match install_root {
                Some(root) => InstallLayout::new(root)
                    .map_err(|error| CliFailure::new(delivery::INPUT_INVALID, error.to_string()))?,
                None => install_layout(),
            };
            let instance =
                resolve_windows_instance(&layout, initialize, |name| std::env::var(name).ok())?;
            println!(
                "{}",
                serde_json::json!({"configuration": instance, "task_path": instance.task_path()})
            );
            Ok(())
        }
        #[cfg(target_os = "windows")]
        Command::PrepareWindowsRestore {
            source,
            destination,
            install_root,
            runner_only,
        } => {
            require_root()?;
            let plan = windows_backup::prepare(&source, &destination, &install_root, runner_only)?;
            println!(
                "{}",
                serde_json::to_string(&plan)
                    .map_err(|error| CliFailure::new(delivery::BACKUP_FAILED, error.to_string()))?
            );
            Ok(())
        }
        Command::VerifyRelease(arguments) => {
            let release = if arguments.runner_only {
                verify_selected_release_at(&arguments.root)?
            } else {
                verify_release_at(&arguments.root)?
            };
            println!(
                "Verified Aster Team {} ({}) signed by {}.",
                release.claims().version,
                release.claims().architecture,
                release.claims().key_id
            );
            Ok(())
        }
        Command::Bootstrap(arguments) => {
            require_root()?;
            bootstrap(arguments)
        }
        Command::Maintenance {
            command: MaintenanceCommand::RunNext,
        } => {
            require_root()?;
            maintenance_executor::run_next()
        }
        Command::Install(arguments) => {
            require_root()?;
            install_control(arguments)
        }
        Command::Upgrade => {
            require_root()?;
            maintenance_executor::upgrade_selected_release()
        }
        Command::Runner {
            command: RunnerCommand::Install,
        } => {
            require_root()?;
            run_selected_installer(InstallRole::Runner, InstallAction::Install, &[])
        }
        Command::Runner {
            command: RunnerCommand::Upgrade,
        } => {
            require_root()?;
            run_selected_installer(InstallRole::Runner, InstallAction::Upgrade, &[])
        }
        Command::Backup {
            command: BackupCommand::Restore { source, confirm },
        } => {
            require_root()?;
            restore_backup(InstallRole::Control, &source, confirm)
        }
        Command::Runner {
            command:
                RunnerCommand::Backup {
                    command: RunnerBackupCommand::Restore { source, confirm },
                },
        } => {
            require_root()?;
            restore_backup(InstallRole::Runner, &source, confirm)
        }
        Command::Info => {
            require_root()?;
            show_info()
        }
        Command::Status => {
            require_root()?;
            show_status()
        }
        Command::Service { command } => {
            require_root()?;
            manage_service(command)
        }
        Command::Logs(arguments) => {
            require_root()?;
            show_logs(arguments)
        }
        Command::Trace(arguments) => {
            require_root()?;
            trace_request(arguments)
        }
        Command::Doctor { verbose } => {
            require_root()?;
            doctor(verbose)
        }
        Command::License { command } => {
            require_root()?;
            manage_license(command)
        }
        Command::Password { command } => {
            require_root()?;
            manage_password(command)
        }
        Command::Backup {
            command: BackupCommand::Create { output },
        } => {
            require_root()?;
            let output = resolve_backup_output(InstallRole::Control, output)?;
            create_backup(InstallRole::Control, &output)
        }
        Command::Runner {
            command:
                RunnerCommand::Backup {
                    command: RunnerBackupCommand::Create { output },
                },
        } => {
            require_root()?;
            let output = resolve_backup_output(InstallRole::Runner, output)?;
            create_backup(InstallRole::Runner, &output)
        }
        Command::Runner {
            command:
                RunnerCommand::Enroll {
                    control_url,
                    token_file,
                    control_ca_certificate,
                    allow_insecure_http,
                },
        } => {
            require_root()?;
            enroll_runner(
                &control_url,
                &token_file,
                control_ca_certificate.as_deref(),
                allow_insecure_http,
            )
        }
        Command::Runner {
            command: RunnerCommand::Status,
        } => {
            require_root()?;
            show_runner_status()
        }
        Command::Uninstall { purge } => {
            require_root()?;
            uninstall(purge)
        }
    }
}

fn show_version() -> Result<(), CliFailure> {
    println!("{}", version_banner());
    let installed = install_layout().current().join("VERSION");
    match fs::read_to_string(&installed) {
        Ok(version) => println!("installed Aster Team {}", version.trim()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            println!("Aster Team is not installed")
        }
        Err(error) => {
            return Err(CliFailure::new(
                delivery::FILESYSTEM_FAILED,
                format!("could not read {}: {error}", installed.display()),
            ));
        }
    }
    Ok(())
}

fn version_banner() -> String {
    format!(
        "aster-team-cli {}\nCommit:     {}\nBuild time: {}",
        env!("CARGO_PKG_VERSION"),
        build_info::BUILD_COMMIT,
        build_info::BUILD_TIMESTAMP,
    )
}

fn show_info() -> Result<(), CliFailure> {
    let role = installed_role()?;
    let release = current_release()?;
    let version = read_trimmed(&release.join("VERSION"))?;
    println!("Aster Team {version}");
    println!("role: {}", role.as_str());
    println!("release: {}", release.display());
    println!("CLI: {}", cli_path().display());
    match role {
        InstallRole::Control => {
            let access = load_access_configuration()?;
            println!("access protocol: {}", access.protocol);
            println!("access address:  {}", access.host);
            println!("bind address:    {}", access.bind_address);
            println!("certificate:     {}", access.certificate_source);
            println!("Admin:  {}", access.admin_url);
            println!("Member: {}", access.member_url);
            println!("API:    {}", access.api_url);
            println!("Runner: {}", access.runner_websocket_url);
            if access.address_kind == "domain" {
                println!(
                    "hosts:  {} app.{} admin.{} api.{}",
                    access.bind_address, access.host, access.host, access.host
                );
            }
            if access.certificate_source == "caddy" {
                println!(
                    "CA:     {}",
                    install_layout().caddy_root_certificate().display()
                );
            }
            println!("configuration: {}", install_layout().config().display());
            println!("data:          {}", install_layout().data().display());
        }
        InstallRole::Runner => {
            println!(
                "configuration: {}",
                install_layout().runner_config().display()
            );
        }
    }
    let selected_release_path = install_layout().selected_release();
    if selected_release_path.is_file() {
        let selected = load_selected_release(&selected_release_path)?;
        println!("selected release: {}", selected.version);
    }
    Ok(())
}

fn selected_runner_unit() -> Result<String, CliFailure> {
    if installed_role()? == InstallRole::Control {
        maintenance_executor::active_runner_unit()
    } else {
        Ok("aster-runner.service".into())
    }
}

fn show_status() -> Result<(), CliFailure> {
    let role = installed_role()?;
    println!("Aster Team status ({})", role.as_str());
    let runner = service_state(&selected_runner_unit()?)?;
    if matches!(role, InstallRole::Runner) {
        println!(
            "[{}] Runner service: {runner}",
            if runner == "active" { "PASS" } else { "FAIL" }
        );
        return if runner == "active" {
            Ok(())
        } else {
            Err(CliFailure::new(
                delivery::DIAGNOSTIC_FAILED,
                "Runner service is not active; run `aster-team-cli doctor`",
            ))
        };
    }
    let control_unit = maintenance_executor::active_control_unit()?;
    let control = service_state(&control_unit)?;
    println!(
        "[{}] Control service: {control}",
        if control == "active" { "PASS" } else { "FAIL" }
    );
    println!(
        "[{}] Local Runner:    {runner}",
        if runner == "active" { "PASS" } else { "WARN" }
    );
    {
        let access = load_access_configuration()?;
        let caddy = service_state("aster-caddy.service")?;
        if access.caddy_enabled {
            println!(
                "[{}] Caddy service:   {caddy}",
                if caddy == "active" { "PASS" } else { "FAIL" }
            );
        } else {
            println!("[PASS] Caddy service:   not required");
        }
        println!(
            "[{}] License: {}",
            if install_layout().license_file().is_file() {
                "PASS"
            } else {
                "WARN"
            },
            if install_layout().license_file().is_file() {
                "installed"
            } else {
                "missing; licensed features are unavailable"
            },
        );
        let database = installed_database()?;
        let database_files_present = database
            .required_files(&install_layout())
            .iter()
            .all(|path| regular_file_without_symlink(path));
        println!(
            "[{}] Database: {}",
            if database_files_present {
                "PASS"
            } else {
                "FAIL"
            },
            if database.is_external() {
                "MariaDB configuration (use diagnose for connectivity)"
            } else {
                "SQLCipher files"
            }
        );
        let mut failed = control != "active"
            || !database_files_present
            || (access.caddy_enabled && caddy != "active");
        for (label, url) in [
            ("Member", access.member_url.as_str()),
            ("Admin", access.admin_url.as_str()),
            ("Model API", access.api_url.as_str()),
        ] {
            let healthy = public_health(&access, url)?;
            println!("[{}] {label}: {url}", if healthy { "PASS" } else { "FAIL" });
            failed |= !healthy;
        }
        if failed {
            return Err(CliFailure::new(
                delivery::DIAGNOSTIC_FAILED,
                "one or more runtime checks failed; run `aster-team-cli doctor`",
            ));
        }
    }
    Ok(())
}

fn public_health(access: &AccessConfiguration, origin: &str) -> Result<bool, CliFailure> {
    let parsed = url::Url::parse(origin).map_err(|error| {
        CliFailure::new(
            delivery::DIAGNOSTIC_FAILED,
            format!("configured endpoint is invalid: {error}"),
        )
    })?;
    let host = parsed.host_str().ok_or_else(|| {
        CliFailure::new(
            delivery::DIAGNOSTIC_FAILED,
            "configured endpoint does not contain a host",
        )
    })?;
    let mut command = ProcessCommand::new("curl");
    command.args([
        "--fail",
        "--silent",
        "--show-error",
        "--noproxy",
        "*",
        "--max-time",
        "3",
    ]);
    if access.address_kind == "domain" {
        let port = parsed.port_or_known_default().ok_or_else(|| {
            CliFailure::new(delivery::DIAGNOSTIC_FAILED, "endpoint port is unavailable")
        })?;
        command
            .arg("--resolve")
            .arg(format!("{host}:{port}:{}", access.bind_address));
    }
    if access.protocol == "https" {
        let certificate = if access.certificate_source == "caddy" {
            install_layout().caddy_root_certificate()
        } else {
            install_layout().caddy_server_certificate()
        };
        command.arg("--cacert").arg(certificate);
    }
    command.arg(format!("{}/healthz", origin.trim_end_matches('/')));
    command
        .output()
        .map(|output| output.status.success())
        .map_err(|error| {
            CliFailure::new(
                delivery::DIAGNOSTIC_FAILED,
                format!("could not execute curl health check: {error}"),
            )
        })
}

fn manage_service(command: ServiceCommand) -> Result<(), CliFailure> {
    let (action, target) = match command {
        ServiceCommand::Start { target } => ("start", target),
        ServiceCommand::Stop { target } => ("stop", target),
        ServiceCommand::Restart { target } => ("restart", target),
    };
    let _lock = acquire_maintenance_lock()?;
    let mut units = service_units(target)?;
    if action == "stop" {
        units.reverse();
    }
    let references = units.iter().map(String::as_str).collect::<Vec<_>>();
    run_service_action(action, &references, delivery::SERVICE_FAILED)?;
    for unit in units {
        println!("{unit}: {}", service_state(&unit)?);
    }
    Ok(())
}

fn show_logs(arguments: LogsArgs) -> Result<(), CliFailure> {
    installed_role()?;
    let unit = match arguments.target {
        LogTarget::Control => maintenance_executor::active_control_unit()?,
        LogTarget::Runner => selected_runner_unit()?,
    };
    #[cfg(target_os = "linux")]
    {
        let mut process = ProcessCommand::new("journalctl");
        process.arg("-u").arg(&unit);
        if arguments.follow {
            process.arg("--follow");
        } else {
            process.arg("--no-pager").arg("--lines=200");
        }
        run_checked(process, delivery::SERVICE_FAILED, "journalctl")
    }
    #[cfg(target_os = "windows")]
    {
        let name = match unit.as_str() {
            "aster-control@blue.service" => "control-blue.log",
            "aster-control@green.service" => "control-green.log",
            "aster-runner.service" => "runner.log",
            _ => {
                return Err(CliFailure::new(
                    delivery::SERVICE_FAILED,
                    "unknown Windows log target",
                ));
            }
        };
        let path = install_layout().logs().join(name);
        if !path.is_file() {
            return Err(CliFailure::new(
                delivery::SERVICE_FAILED,
                format!("service log is unavailable: {}", path.display()),
            ));
        }
        let mut process = ProcessCommand::new("powershell.exe");
        process.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            if arguments.follow {
                "Get-Content -LiteralPath $args[0] -Tail 200 -Wait"
            } else {
                "Get-Content -LiteralPath $args[0] -Tail 200"
            },
        ]);
        process.arg(path);
        run_checked(process, delivery::SERVICE_FAILED, "Windows service log")
    }
    #[cfg(target_os = "macos")]
    {
        let process_name = match arguments.target {
            LogTarget::Control => "aster-control",
            LogTarget::Runner => "aster-runner",
        };
        let predicate = format!("process == \"{process_name}\"");
        let mut process = ProcessCommand::new("log");
        if arguments.follow {
            process.args(["stream", "--style", "compact", "--predicate", &predicate]);
        } else {
            process.args([
                "show",
                "--last",
                "1h",
                "--style",
                "compact",
                "--predicate",
                &predicate,
            ]);
        }
        run_checked(process, delivery::SERVICE_FAILED, "macOS unified log")
    }
}

fn trace_request(arguments: TraceArgs) -> Result<(), CliFailure> {
    installed_role()?;
    let request_id = match arguments.request_id {
        Some(value) => value,
        None => read_line("Aster request ID: ")?,
    };
    if !valid_trace_request_id(&request_id) {
        return Err(CliFailure::new(
            delivery::INPUT_INVALID,
            "request ID must contain 8-160 letters, numbers, dots, underscores, or hyphens",
        ));
    }
    let unit = maintenance_executor::active_control_unit()?;
    let logs = read_recent_service_logs(&unit, arguments.hours)?;
    let matches = logs
        .lines()
        .filter(|line| line.contains(&request_id))
        .collect::<Vec<_>>();
    println!("Aster request trace: {request_id}");
    println!("Control unit: {unit}");
    println!("Search window: last {} hour(s)", arguments.hours);
    if matches.is_empty() {
        println!("No matching Control log entries were found.");
        return Ok(());
    }
    for line in matches {
        println!("{line}");
    }
    Ok(())
}

fn valid_trace_request_id(value: &str) -> bool {
    (8..=160).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(target_os = "linux")]
fn read_recent_service_logs(unit: &str, hours: u16) -> Result<String, CliFailure> {
    let output = ProcessCommand::new("journalctl")
        .arg("-u")
        .arg(unit)
        .arg("--no-pager")
        .arg("--since")
        .arg(format!("{hours} hours ago"))
        .output()
        .map_err(|error| {
            CliFailure::new(
                delivery::SERVICE_FAILED,
                format!("could not read the system journal: {error}"),
            )
        })?;
    if !output.status.success() {
        return Err(command_failure(
            delivery::SERVICE_FAILED,
            "journalctl",
            output.status,
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "windows")]
fn read_recent_service_logs(unit: &str, _hours: u16) -> Result<String, CliFailure> {
    let name = match unit {
        "aster-control@blue.service" => "control-blue.log",
        "aster-control@green.service" => "control-green.log",
        _ => {
            return Err(CliFailure::new(
                delivery::SERVICE_FAILED,
                "unknown Windows Control log target",
            ));
        }
    };
    fs::read_to_string(install_layout().logs().join(name)).map_err(|error| {
        CliFailure::new(
            delivery::SERVICE_FAILED,
            format!("could not read the Control log: {error}"),
        )
    })
}

#[cfg(target_os = "macos")]
fn read_recent_service_logs(_unit: &str, hours: u16) -> Result<String, CliFailure> {
    let output = ProcessCommand::new("log")
        .args([
            "show",
            "--last",
            &format!("{hours}h"),
            "--style",
            "compact",
            "--predicate",
            "process == \"aster-control\"",
        ])
        .output()
        .map_err(|error| {
            CliFailure::new(
                delivery::SERVICE_FAILED,
                format!("could not read the unified log: {error}"),
            )
        })?;
    if !output.status.success() {
        return Err(command_failure(
            delivery::SERVICE_FAILED,
            "macOS unified log",
            output.status,
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn doctor(verbose: bool) -> Result<(), CliFailure> {
    let role = installed_role()?;
    println!("Aster Team {} diagnostics", role.as_str());
    let warnings = match role {
        InstallRole::Control => doctor_control(verbose)?,
        InstallRole::Runner => doctor_runner(verbose)?,
    };
    let primary = match role {
        InstallRole::Control => maintenance_executor::active_control_unit()?,
        InstallRole::Runner => "aster-runner.service".to_owned(),
    };
    if service_state(&primary)? != "active" {
        println!("[FAIL] Primary service: {primary} is not active");
        return Err(CliFailure::new(
            delivery::DIAGNOSTIC_FAILED,
            format!("{primary} is not active"),
        ));
    }
    println!("[PASS] Primary service: {primary} is active");
    if warnings == 0 {
        println!("Result: diagnostics passed.");
    } else {
        println!("Result: diagnostics passed with {warnings} warning(s).");
    }
    Ok(())
}

fn doctor_control(verbose: bool) -> Result<usize, CliFailure> {
    let mut warnings = 0;
    let release = current_release()?;
    let binary = install_layout().release_binary(&release, "aster-control");
    let mut verify = ProcessCommand::new(&binary);
    verify.arg("verify-machine-identity");
    run_diagnostic_command(
        verify,
        "verify-machine-identity",
        "Installation identity",
        verbose,
    )?;
    let access = load_access_configuration()?;
    println!(
        "[PASS] Access configuration: {} via {} ({})",
        access.host, access.protocol, access.address_kind
    );
    let environment = read_environment_file(
        &install_layout().control_environment(),
        &[
            "ASTER_CONTROL_LISTEN",
            "ASTER_CONTROL_ADMIN_LISTEN",
            "ASTER_CONTROL_MEMBER_LISTEN",
            "ASTER_CONTROL_ALLOW_INSECURE_HTTP",
            "ASTER_CONTROL_SECURE_COOKIES",
            "ASTER_CONTROL_API_TLS_CERTIFICATE",
            "ASTER_CONTROL_API_TLS_PRIVATE_KEY",
        ],
    )?;
    let mut preflight = process_as_user("aster-team", &binary);
    preflight.envs(environment);
    preflight
        .arg("preflight")
        .arg("--admin-assets")
        .arg(release.join("admin"))
        .arg("--member-assets")
        .arg(release.join("member"));
    run_diagnostic_command(preflight, "Control preflight", "Control preflight", verbose)?;
    let database = installed_database()?;
    println!(
        "[PASS] Database: {} preflight succeeded",
        if database.is_external() {
            "MariaDB"
        } else {
            "SQLCipher"
        }
    );
    if access.caddy_enabled {
        #[cfg(target_os = "linux")]
        let mut caddy = process_as_user(
            "aster-caddy",
            &install_layout().release_binary(&release, "caddy"),
        );
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        let mut caddy = ProcessCommand::new(install_layout().release_binary(&release, "caddy"));
        caddy
            .env("HOME", install_layout().caddy_data())
            .env("XDG_DATA_HOME", install_layout().caddy_data())
            .env("XDG_CONFIG_HOME", install_layout().caddy_config())
            .arg("validate")
            .arg("--config")
            .arg(install_layout().caddyfile())
            .arg("--adapter")
            .arg("caddyfile");
        run_diagnostic_command(caddy, "Caddy validation", "Caddy configuration", verbose)?;
        if service_state("aster-caddy.service")? != "active" {
            println!("[FAIL] Caddy service: inactive");
            return Err(CliFailure::new(
                delivery::DIAGNOSTIC_FAILED,
                "managed Caddy is required but not active",
            ));
        }
        println!("[PASS] Caddy service: active");
    } else {
        println!("[PASS] Caddy service: not required for direct IP HTTP");
    }
    for (label, url) in [
        ("Member endpoint", access.member_url.as_str()),
        ("Admin endpoint", access.admin_url.as_str()),
        ("Model API endpoint", access.api_url.as_str()),
    ] {
        if !public_health(&access, url)? {
            println!("[FAIL] {label}: {url}");
            return Err(CliFailure::new(
                delivery::DIAGNOSTIC_FAILED,
                format!("{label} health check failed"),
            ));
        }
        println!("[PASS] {label}: {url}");
    }
    if install_layout().license_file().is_file() {
        println!("[PASS] License: installed");
    } else {
        println!("[WARN] License: missing; licensed features are unavailable");
        warnings += 1;
    }
    if service_state(&selected_runner_unit()?)? == "active" {
        println!("[PASS] Local Runner: active");
    } else {
        println!("[WARN] Local Runner: not active");
        warnings += 1;
    }
    Ok(warnings)
}

fn doctor_runner(verbose: bool) -> Result<usize, CliFailure> {
    let release = current_release()?;
    let environment = read_environment_file(
        &install_layout().runner_environment(),
        &[
            "ASTER_RUNNER_CONTROL_WSS",
            "ASTER_RUNNER_CONTROL_CA_CERTIFICATE",
            "ASTER_RUNNER_ALLOW_INSECURE_HTTP",
        ],
    )?;
    let control_wss = environment
        .iter()
        .find(|(key, _)| key == "ASTER_RUNNER_CONTROL_WSS")
        .map(|(_, value)| value.clone())
        .ok_or_else(|| {
            CliFailure::new(
                delivery::DIAGNOSTIC_FAILED,
                "runner.env is missing ASTER_RUNNER_CONTROL_WSS",
            )
        })?;
    let runner_binary = install_layout().release_binary(&release, "aster-runner");
    let mut preflight = process_as_user("aster-runner", &runner_binary);
    for (key, value) in environment {
        if matches!(
            key.as_str(),
            "ASTER_RUNNER_CONTROL_CA_CERTIFICATE" | "ASTER_RUNNER_ALLOW_INSECURE_HTTP"
        ) {
            preflight.env(key, value);
        }
    }
    preflight
        .arg("preflight")
        .arg("--control-wss")
        .arg(control_wss);
    for host in RUNNER_ALLOWED_UPSTREAM_HOSTS {
        preflight.arg("--allowed-upstream-host").arg(host);
    }
    run_diagnostic_command(preflight, "Runner preflight", "Runner preflight", verbose)?;
    Ok(0)
}

fn run_diagnostic_command(
    mut command: ProcessCommand,
    command_name: &str,
    label: &str,
    verbose: bool,
) -> Result<(), CliFailure> {
    let output = command.output().map_err(|error| {
        CliFailure::new(
            delivery::DIAGNOSTIC_FAILED,
            format!("could not execute {command_name}: {error}"),
        )
    })?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if !output.status.success() {
        println!("[FAIL] {label}");
        let detail = [stdout, stderr]
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        return Err(CliFailure::new(
            delivery::DIAGNOSTIC_FAILED,
            if detail.is_empty() {
                format!("{command_name} exited with {}", output.status)
            } else {
                format!("{command_name} failed:\n{detail}")
            },
        ));
    }
    println!("[PASS] {label}");
    if verbose {
        for line in [stdout, stderr]
            .into_iter()
            .filter(|value| !value.is_empty())
        {
            println!("  {line}");
        }
    }
    Ok(())
}

fn manage_license(command: LicenseCommand) -> Result<(), CliFailure> {
    if !matches!(installed_role()?, InstallRole::Control) {
        return Err(CliFailure::new(
            delivery::LICENSE_FAILED,
            "license commands are available on a Control host only",
        ));
    }
    match command {
        LicenseCommand::Request { output } => {
            let selection = resolve_license_request_output(output.as_deref())?;
            if let Some(release_root) = selection.redirected_from_release {
                println!(
                    "Signed release directory detected at {}; writing the request outside it.",
                    release_root.display()
                );
            }
            let release = current_release()?;
            let mut process =
                ProcessCommand::new(install_layout().release_binary(&release, "aster-control"));
            process
                .arg("generate-license-request")
                .arg("--output")
                .arg(&selection.path);
            run_checked(
                process,
                delivery::LICENSE_FAILED,
                "generate-license-request",
            )?;
            print_license_request_qr(&selection.path)
        }
        LicenseCommand::Install { source } => install_license(&source),
        LicenseCommand::Status => show_license_status(),
    }
}

fn initial_license_is_installed(path: &Path) -> Result<bool, CliFailure> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => Err(CliFailure::new(
            delivery::LICENSE_FAILED,
            format!(
                "installed license path is not an ordinary file: {}",
                path.display()
            ),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(CliFailure::new(
            delivery::LICENSE_FAILED,
            format!("could not inspect installed license: {error}"),
        )),
    }
}

fn complete_initial_license_setup() -> Result<(), CliFailure> {
    if initial_license_is_installed(&install_layout().license_file())? {
        println!("A license was installed during setup.");
        show_license_status()
    } else {
        manage_license(LicenseCommand::Request { output: None })
    }
}

fn manage_password(command: PasswordCommand) -> Result<(), CliFailure> {
    if !matches!(installed_role()?, InstallRole::Control) {
        return Err(CliFailure::new(
            delivery::COMMAND_FAILED,
            "password reset is available on a Control host only",
        ));
    }
    match command {
        PasswordCommand::ResetAdmin { email } => reset_admin_password(email.as_deref()),
    }
}

fn reset_admin_password(requested_email: Option<&str>) -> Result<(), CliFailure> {
    let email = match requested_email.map(str::trim) {
        Some(value) if !value.is_empty() => value.to_owned(),
        _ => {
            let value = read_line(&format!("Administrator email [{DEFAULT_OWNER_EMAIL}]: "))?;
            if value.is_empty() {
                DEFAULT_OWNER_EMAIL.to_owned()
            } else {
                value
            }
        }
    };
    let password = Zeroizing::new(
        rpassword::prompt_password("New administrator password: ")
            .map_err(|error| CliFailure::new(delivery::COMMAND_FAILED, error.to_string()))?,
    );
    if !(12..=1024).contains(&password.len()) {
        return Err(CliFailure::new(
            delivery::INPUT_INVALID,
            "the new password must contain between 12 and 1024 bytes",
        ));
    }
    let confirmation = Zeroizing::new(
        rpassword::prompt_password("Confirm new administrator password: ")
            .map_err(|error| CliFailure::new(delivery::COMMAND_FAILED, error.to_string()))?,
    );
    if password.as_str() != confirmation.as_str() {
        return Err(CliFailure::new(
            delivery::INPUT_INVALID,
            "the password confirmation does not match",
        ));
    }
    let release = current_release()?;
    let control_binary = install_layout().release_binary(&release, "aster-control");
    validate_regular_file(&control_binary, "aster-control")?;
    let _lock = acquire_maintenance_lock()?;
    let control_unit = maintenance_executor::active_control_unit()?;
    let was_active = service_is_active(&control_unit)?;
    if was_active {
        run_service_action("stop", &[&control_unit], delivery::COMMAND_FAILED)?;
    }
    let mut process = process_as_user("aster-team", &control_binary);
    process
        .arg("reset-admin-password")
        .arg("--email")
        .arg(&email)
        .arg("--password-stdin");
    let result = run_checked_with_secret_stdin(
        process,
        password.as_bytes(),
        delivery::COMMAND_FAILED,
        "reset administrator password",
    );
    let restart = if was_active {
        run_service_action("start", &[&control_unit], delivery::COMMAND_FAILED)
    } else {
        Ok(())
    };
    result.and(restart)?;
    println!("Administrator password reset for {email}; existing sessions were revoked.");
    Ok(())
}

#[derive(Debug, Eq, PartialEq)]
struct LicenseRequestOutput {
    path: PathBuf,
    redirected_from_release: Option<PathBuf>,
}

fn resolve_license_request_output(
    requested: Option<&Path>,
) -> Result<LicenseRequestOutput, CliFailure> {
    let current = std::env::current_dir()
        .and_then(|path| path.canonicalize())
        .map_err(|error| {
            CliFailure::new(
                delivery::INPUT_INVALID,
                format!("current directory is unavailable: {error}"),
            )
        })?;
    resolve_license_request_output_at(requested, &current, time::OffsetDateTime::now_utc())
}

fn resolve_license_request_output_at(
    requested: Option<&Path>,
    current: &Path,
    now: time::OffsetDateTime,
) -> Result<LicenseRequestOutput, CliFailure> {
    if let Some(requested) = requested {
        let candidate = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            current.join(requested)
        };
        let file_name = candidate.file_name().ok_or_else(|| {
            CliFailure::new(
                delivery::INPUT_INVALID,
                "license request output must include a file name",
            )
        })?;
        let parent = candidate
            .parent()
            .filter(|path| path.is_dir())
            .ok_or_else(|| {
                CliFailure::new(
                    delivery::INPUT_INVALID,
                    "license request output directory is unavailable",
                )
            })?
            .canonicalize()
            .map_err(|error| CliFailure::new(delivery::INPUT_INVALID, error.to_string()))?;
        if let Some(release_root) = signed_release_root(&parent) {
            return Err(CliFailure::new(
                delivery::INPUT_INVALID,
                format!(
                    "license request output cannot be inside the signed release directory {}; omit --output to select a safe location automatically",
                    release_root.display()
                ),
            ));
        }
        let path = parent.join(file_name);
        if path.exists() {
            return Err(CliFailure::new(
                delivery::INPUT_INVALID,
                format!("license request output already exists: {}", path.display()),
            ));
        }
        return Ok(LicenseRequestOutput {
            path,
            redirected_from_release: None,
        });
    }

    let release_root = signed_release_root(current);
    let directory = match release_root.as_deref() {
        Some(root) if root.starts_with(install_layout().root()) => std::env::current_dir()
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?,
        Some(root) => root.parent().map(Path::to_path_buf).ok_or_else(|| {
            CliFailure::new(
                delivery::INPUT_INVALID,
                "signed release directory has no safe parent for the license request",
            )
        })?,
        None => current.to_path_buf(),
    };
    let timestamp = now
        .format(LICENSE_REQUEST_FILE_TIME_FORMAT)
        .map_err(|error| {
            CliFailure::new(
                delivery::FILESYSTEM_FAILED,
                format!("could not format the license request timestamp: {error}"),
            )
        })?;
    for sequence in 0..1000_u16 {
        let suffix = if sequence == 0 {
            String::new()
        } else {
            format!("-{sequence:02}")
        };
        let path = directory.join(format!(
            "{LICENSE_REQUEST_FILE_PREFIX}-{timestamp}{suffix}.json"
        ));
        if !path.exists() {
            return Ok(LicenseRequestOutput {
                path,
                redirected_from_release: release_root,
            });
        }
    }
    Err(CliFailure::new(
        delivery::FILESYSTEM_FAILED,
        "could not allocate a unique license request file name",
    ))
}

fn signed_release_root(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|ancestor| {
            ancestor.join(RELEASE_MANIFEST_FILE).is_file()
                && ancestor.join("VERSION").is_file()
                && ancestor
                    .join("bin")
                    .join(executable_name("aster-team-cli"))
                    .is_file()
        })
        .map(Path::to_path_buf)
}

fn print_license_request_qr(output: &Path) -> Result<(), CliFailure> {
    let encoded = fs::read(output).map_err(|error| {
        CliFailure::new(
            delivery::LICENSE_FAILED,
            format!(
                "license request was generated but could not be read from {}: {error}",
                output.display()
            ),
        )
    })?;
    let payload = compact_license_request_qr_payload(&encoded)?;
    let code = license_request_qr(&payload)?;
    let png = output.with_extension("qr.png");
    code.render::<Luma<u8>>()
        .min_dimensions(1024, 1024)
        .quiet_zone(true)
        .build()
        .save(&png)
        .map_err(|error| {
            CliFailure::new(
                delivery::LICENSE_FAILED,
                format!("could not write the license request QR PNG: {error}"),
            )
        })?;
    println!("Machine authorization request: {}", output.display());
    println!("QR PNG: {}", png.display());
    println!("Capture the complete terminal QR code below, including its quiet border:");
    print!("{}", render_terminal_qr(&code));
    io::stdout().flush().map_err(|error| {
        CliFailure::new(
            delivery::LICENSE_FAILED,
            format!("could not print the license request QR code: {error}"),
        )
    })?;
    Ok(())
}

fn compact_license_request_qr_payload(encoded: &[u8]) -> Result<Vec<u8>, CliFailure> {
    // Validate the original v2 bytes before producing the compact transport;
    // compatibility declarations are carried explicitly below.
    let request = aster_license_core::request_v2::parse_request(encoded).map_err(|error| {
        CliFailure::new(
            delivery::LICENSE_FAILED,
            format!("compact QR requires a valid v2 license request: {error}"),
        )
    })?;
    let document: serde_json::Value = serde_json::from_slice(encoded).map_err(|error| {
        CliFailure::new(
            delivery::LICENSE_FAILED,
            format!("generated license request is invalid JSON: {error}"),
        )
    })?;

    let field = |name: &str| {
        document
            .get(name)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                CliFailure::new(
                    delivery::LICENSE_FAILED,
                    format!("generated license request is missing {name}"),
                )
            })
    };
    let factors = document
        .get("machine_factors")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            CliFailure::new(
                delivery::LICENSE_FAILED,
                "generated license request is missing machine_factors",
            )
        })?;
    let factor = |kind: &str| {
        factors
            .iter()
            .find(|factor| factor.get("kind").and_then(serde_json::Value::as_str) == Some(kind))
            .and_then(|factor| factor.get("sha256"))
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                CliFailure::new(
                    delivery::LICENSE_FAILED,
                    format!("generated license request is missing machine factor {kind}"),
                )
            })
    };
    let request_id = decode_qr_identifier(field("request_id")?, "request", "request_id")?;
    let installation_id = field("installation_id")?;
    let installation_token =
        decode_qr_identifier(installation_id, "installation", "installation_id")?;
    let dmi_product_uuid = decode_qr_digest(factor("dmi_product_uuid")?, "dmi_product_uuid")?;
    let machine_id = decode_qr_digest(factor("machine_id")?, "machine_id")?;
    let expected_fingerprint = URL_SAFE_NO_PAD.encode(Sha256::digest(
        format!(
            "aster-team\n{installation_id}\ndmi_product_uuid={}\nmachine_id={}\n",
            factor("dmi_product_uuid")?,
            factor("machine_id")?,
        )
        .as_bytes(),
    ));
    if field("machine_fingerprint_sha256")? != expected_fingerprint {
        return Err(CliFailure::new(
            delivery::LICENSE_FAILED,
            "generated license request machine fingerprint is inconsistent",
        ));
    }
    let platform = match field("platform")? {
        "linux" => 1,
        "windows" => 2,
        "macos" => 3,
        _ => {
            return Err(CliFailure::new(
                delivery::LICENSE_FAILED,
                "generated license request platform is invalid",
            ));
        }
    };
    let architecture = match field("architecture")? {
        "amd64" => 1,
        "arm64" => 2,
        _ => {
            return Err(CliFailure::new(
                delivery::LICENSE_FAILED,
                "generated license request architecture is invalid",
            ));
        }
    };
    let product_version = field("product_version")?.as_bytes();
    let version_length = u8::try_from(product_version.len())
        .ok()
        .filter(|length| *length > 0 && *length <= 64)
        .ok_or_else(|| {
            CliFailure::new(
                delivery::LICENSE_FAILED,
                "generated license request product_version is invalid",
            )
        })?;
    let generated_at = time::OffsetDateTime::parse(
        field("generated_at")?,
        &time::format_description::well_known::Rfc3339,
    )
    .map_err(|error| {
        CliFailure::new(
            delivery::LICENSE_FAILED,
            format!("generated license request generated_at is invalid: {error}"),
        )
    })?;
    let generated_at_millis = u64::try_from(generated_at.unix_timestamp_nanos() / 1_000_000)
        .map_err(|_| {
            CliFailure::new(
                delivery::LICENSE_FAILED,
                "generated license request generated_at is outside the supported range",
            )
        })?;

    let mut compact = Vec::with_capacity(LICENSE_REQUEST_QR_FIXED_BYTES + product_version.len());
    compact.extend_from_slice(&LICENSE_REQUEST_QR_MAGIC);
    compact.push(platform);
    compact.push(architecture);
    compact.push(version_length);
    compact.extend_from_slice(&request.capability_catalog_version.to_be_bytes());
    compact.extend_from_slice(&request.quota_policy_version.to_be_bytes());
    compact.extend_from_slice(&generated_at_millis.to_be_bytes());
    compact.extend_from_slice(&request_id);
    compact.extend_from_slice(&installation_token);
    compact.extend_from_slice(&dmi_product_uuid);
    compact.extend_from_slice(&machine_id);
    compact.extend_from_slice(product_version);
    Ok(compact)
}

fn decode_qr_identifier(value: &str, prefix: &str, field: &str) -> Result<[u8; 18], CliFailure> {
    let encoded = value
        .strip_prefix(prefix)
        .and_then(|suffix| suffix.strip_prefix('_'))
        .ok_or_else(|| {
            CliFailure::new(
                delivery::LICENSE_FAILED,
                format!("generated license request {field} is invalid"),
            )
        })?;
    let decoded = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| {
        CliFailure::new(
            delivery::LICENSE_FAILED,
            format!("generated license request {field} is invalid"),
        )
    })?;
    decoded.try_into().map_err(|_| {
        CliFailure::new(
            delivery::LICENSE_FAILED,
            format!("generated license request {field} is invalid"),
        )
    })
}

fn decode_qr_digest(value: &str, field: &str) -> Result<[u8; 32], CliFailure> {
    let decoded = URL_SAFE_NO_PAD.decode(value).map_err(|_| {
        CliFailure::new(
            delivery::LICENSE_FAILED,
            format!("generated license request machine factor {field} is invalid"),
        )
    })?;
    decoded.try_into().map_err(|_| {
        CliFailure::new(
            delivery::LICENSE_FAILED,
            format!("generated license request machine factor {field} is invalid"),
        )
    })
}

fn license_request_qr(payload: &[u8]) -> Result<QrCode, CliFailure> {
    for number in 1..=40 {
        let mut bits = Bits::new(Version::Normal(number));
        if bits.push_eci_designator(3).is_err()
            || bits.push_byte_data(payload).is_err()
            || bits.push_terminator(EcLevel::M).is_err()
        {
            continue;
        }
        if let Ok(code) = QrCode::with_bits(bits, EcLevel::M) {
            return Ok(code);
        }
    }
    Err(CliFailure::new(
        delivery::LICENSE_FAILED,
        "license request is too large to render as a QR code",
    ))
}

// Each QR module is painted with the terminal cell background rather than a
// block glyph, so adjacent black and white modules cannot acquire font gaps.
fn render_terminal_qr(code: &QrCode) -> String {
    const RESET: &str = "\x1b[0m";
    const BLACK: &str = "\x1b[0;30;40m";
    const WHITE: &str = "\x1b[0;30;47m";
    const BORDER: usize = 1;
    const MODULE_SPACES: usize = 2;

    let size = code.width();
    let row_width = (size + 2 * BORDER) * MODULE_SPACES;
    let mut output = String::new();

    for _ in 0..BORDER {
        output.push_str(WHITE);
        output.push_str(&" ".repeat(row_width));
        output.push_str(RESET);
        output.push('\n');
    }

    for y in 0..size {
        let mut previous = WHITE;
        output.push_str(WHITE);
        output.push_str(&" ".repeat(BORDER * MODULE_SPACES));
        for x in 0..size {
            let colour = if code[(x, y)] == Color::Dark {
                BLACK
            } else {
                WHITE
            };
            if colour != previous {
                output.push_str(colour);
                previous = colour;
            }
            output.push_str(&" ".repeat(MODULE_SPACES));
        }
        if previous != WHITE {
            output.push_str(WHITE);
        }
        output.push_str(&" ".repeat(BORDER * MODULE_SPACES));
        output.push_str(RESET);
        output.push('\n');
    }

    for _ in 0..BORDER {
        output.push_str(WHITE);
        output.push_str(&" ".repeat(row_width));
        output.push_str(RESET);
        output.push('\n');
    }
    output
}

fn install_license(source: &Path) -> Result<(), CliFailure> {
    validate_regular_file(source, "license source")?;
    let _lock = acquire_maintenance_lock()?;
    let control_unit = maintenance_executor::active_control_unit()?;
    let was_active = service_is_active(&control_unit)?;
    if was_active {
        run_service_action("stop", &[&control_unit], delivery::LICENSE_FAILED)?;
    }
    let release = current_release()?;
    let mut process =
        ProcessCommand::new(install_layout().release_binary(&release, "aster-control"));
    process.arg("install-license").arg("--source").arg(source);
    let install_result = run_checked(process, delivery::LICENSE_FAILED, "install-license");
    #[cfg(unix)]
    let result = match (install_result, restore_license_ownership(&install_layout())) {
        (result, Ok(())) => result,
        (Ok(()), Err(ownership_error)) => Err(ownership_error),
        (Err(install_error), Err(ownership_error)) => Err(CliFailure::new(
            delivery::LICENSE_FAILED,
            format!(
                "{}; license state ownership restoration also failed: {}",
                install_error.detail, ownership_error.detail
            ),
        )),
    };
    #[cfg(not(unix))]
    let result = install_result;
    let restart = if was_active {
        run_service_action("start", &[&control_unit], delivery::LICENSE_FAILED)
    } else {
        Ok(())
    };
    result.and(restart)
}

#[cfg(unix)]
fn restore_license_ownership(layout: &InstallLayout) -> Result<(), CliFailure> {
    let mut config = ProcessCommand::new("chown");
    config
        .arg("-R")
        .arg("aster-team:aster-team")
        .arg(layout.license());
    run_checked(
        config,
        delivery::LICENSE_FAILED,
        "chown license configuration",
    )?;
    for path in license_state_ownership_paths(layout) {
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(CliFailure::new(
                    delivery::LICENSE_FAILED,
                    format!("inspect license state ownership: {error}"),
                ));
            }
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(CliFailure::new(
                delivery::LICENSE_FAILED,
                format!(
                    "license state path is not an ordinary file: {}",
                    path.display()
                ),
            ));
        }
        let mut chown = ProcessCommand::new("chown");
        chown
            .arg("--no-dereference")
            .arg("aster-team:aster-team")
            .arg(&path);
        run_checked(
            chown,
            delivery::LICENSE_FAILED,
            "chown license transaction state",
        )?;
    }
    Ok(())
}

#[cfg(any(unix, test))]
fn license_state_ownership_paths(layout: &InstallLayout) -> Vec<PathBuf> {
    let state = layout.license_state();
    [
        "",
        ".pending",
        ".lock",
        ".staged",
        ".activation",
        ".mutation",
    ]
    .into_iter()
    .map(|suffix| {
        let mut path = state.as_os_str().to_os_string();
        path.push(suffix);
        PathBuf::from(path)
    })
    .collect()
}

fn show_license_status() -> Result<(), CliFailure> {
    let path = install_layout().license_file();
    if !path.is_file() {
        println!("license: missing");
        return Ok(());
    }
    doctor_control(false)?;
    let value: serde_json::Value = serde_json::from_slice(
        &fs::read(&path)
            .map_err(|error| CliFailure::new(delivery::LICENSE_FAILED, error.to_string()))?,
    )
    .map_err(|error| CliFailure::new(delivery::LICENSE_FAILED, error.to_string()))?;
    let claims = license_claims(&value);
    let text = |key: &str| {
        claims
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("-")
    };
    println!("license: active");
    println!("id: {}", text("license_id"));
    println!("serial: {}", text("serial"));
    if let Some(plan_id) = claims.get("plan_id").and_then(serde_json::Value::as_str) {
        println!("plan: {plan_id}");
    }
    println!(
        "member seats: {}",
        license_quota_display(claims, "member_seats")
    );
    if claims.get("entitlements").is_some() {
        println!("runners: {}", license_quota_display(claims, "runners"));
        println!(
            "subscriptions/accounts: {}",
            license_quota_display(claims, "upstream_accounts")
        );
        println!(
            "API keys per member: {}",
            license_quota_display(claims, "api_keys_per_member")
        );
    }
    println!("expires: {}", license_expiry_display(claims));
    Ok(())
}

fn license_claims(value: &serde_json::Value) -> &serde_json::Value {
    &value["claims"]
}

fn license_quota_display(claims: &serde_json::Value, quota_id: &str) -> String {
    let limit = claims
        .pointer("/entitlements/quotas")
        .and_then(serde_json::Value::as_array)
        .and_then(|quotas| {
            quotas
                .iter()
                .find(|quota| quota.get("id").and_then(serde_json::Value::as_str) == Some(quota_id))
        })
        .and_then(|quota| quota.get("limit"));
    match limit.and_then(|value| value.get("mode").and_then(serde_json::Value::as_str)) {
        Some("limited") => limit
            .and_then(|value| value.get("value"))
            .and_then(serde_json::Value::as_u64)
            .map_or_else(|| "-".to_owned(), |value| value.to_string()),
        Some("unlimited") => "unlimited".to_owned(),
        _ => "-".to_owned(),
    }
}

fn license_expiry_display(claims: &serde_json::Value) -> &str {
    let expiry = claims.pointer("/validity/expiry");
    match expiry.and_then(|value| value.get("mode").and_then(serde_json::Value::as_str)) {
        Some("none") => "never",
        Some("fixed") => expiry
            .and_then(|value| value.get("expires_at"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("-"),
        _ => "-",
    }
}

fn resolve_backup_output(
    role: InstallRole,
    requested: Option<PathBuf>,
) -> Result<PathBuf, CliFailure> {
    if let Some(path) = requested {
        return Ok(path);
    }
    let directory = install_layout().backups();
    fs::create_dir_all(&directory)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    let timestamp = time::OffsetDateTime::now_utc()
        .format(BACKUP_FILE_TIME_FORMAT)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    Ok(directory.join(format!(
        "aster-team-{}-backup-{timestamp}.tar.gz",
        role.as_str()
    )))
}

fn create_backup(role: InstallRole, output: &Path) -> Result<(), CliFailure> {
    if installed_role()? != role {
        return Err(CliFailure::new(
            delivery::BACKUP_FAILED,
            "backup role does not match this host",
        ));
    }
    if role == InstallRole::Control && installed_database()?.is_external() {
        return Err(CliFailure::new(
            delivery::BACKUP_FAILED,
            "external database backup requires a database-native snapshot plus installation configuration; the local SQLCipher backup command is unavailable",
        ));
    }
    validate_new_output(output)?;
    let _lock = acquire_maintenance_lock()?;
    let control_unit = maintenance_executor::active_control_unit().ok();
    let units: Vec<&str> = match role {
        InstallRole::Control => vec![
            "aster-runner.service",
            "aster-caddy.service",
            control_unit.as_deref().ok_or_else(|| {
                CliFailure::new(
                    delivery::BACKUP_FAILED,
                    "active Control slot is unavailable",
                )
            })?,
        ],
        InstallRole::Runner => vec!["aster-runner.service"],
    };
    let active = active_units(&units)?;
    let root = install_layout().root().to_path_buf();
    let parent = root.parent().ok_or_else(|| {
        CliFailure::new(
            delivery::BACKUP_FAILED,
            "install root has no parent directory",
        )
    })?;
    let name = root.file_name().ok_or_else(|| {
        CliFailure::new(
            delivery::BACKUP_FAILED,
            "install root has no directory name",
        )
    })?;
    let backup_workspace = install_layout().backups();
    fs::create_dir_all(&backup_workspace)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    let temporary = create_backup_temporary(&backup_workspace)?;
    let mut tar = ProcessCommand::new("tar");
    tar.arg("-C")
        .arg(parent)
        .arg("-czf")
        .arg(&temporary)
        .arg(format!("--exclude={}/backups/*", name.to_string_lossy()))
        .arg(format!("--exclude={}/staging/*", name.to_string_lossy()))
        .arg(name);
    // Prepare every fallible path before stopping; restart even when stopping
    // a later service or creating/publishing the archive fails.
    let references: Vec<_> = active.iter().map(String::as_str).collect();
    let stop = if references.is_empty() {
        Ok(())
    } else {
        run_service_action("stop", &references, delivery::BACKUP_FAILED)
    };
    let result = stop
        .and_then(|()| run_checked(tar, delivery::BACKUP_FAILED, "tar backup"))
        .and_then(|()| {
            set_mode(&temporary, 0o600)?;
            validate_regular_file(&temporary, "temporary backup")?;
            publish_backup(&temporary, output)
        });
    let restart = if active.is_empty() {
        Ok(())
    } else {
        let references: Vec<_> = active.iter().rev().map(String::as_str).collect();
        run_service_action("start", &references, delivery::BACKUP_FAILED)
    };
    let _ = fs::remove_file(&temporary);
    if result.is_err() {
        let _ = fs::remove_file(output);
    }
    result.and(restart)?;
    println!("Backup created: {}", output.display());
    Ok(())
}

fn enroll_runner(
    control_url: &str,
    token_file: &Path,
    control_ca_certificate: Option<&Path>,
    allow_insecure_http: bool,
) -> Result<(), CliFailure> {
    if installed_role()? != InstallRole::Runner {
        return Err(CliFailure::new(
            delivery::RUNNER_FAILED,
            "Runner enrollment requires a dedicated Runner host",
        ));
    }
    validate_secret_file(token_file)?;
    if let Some(path) = control_ca_certificate {
        validate_regular_file(path, "Control CA certificate")?;
    }
    let control_wss = runner_wss_url(control_url, allow_insecure_http)?;
    let _lock = acquire_maintenance_lock()?;
    let environment_path = install_layout().runner_environment();
    let environment_existed = environment_path.exists();
    let mut environment = format!("ASTER_RUNNER_CONTROL_WSS={control_wss}\n");
    if let Some(path) = control_ca_certificate {
        if path != install_layout().runner_control_ca() {
            return Err(CliFailure::new(
                delivery::INPUT_INVALID,
                format!(
                    "the Control CA certificate must be {}",
                    install_layout().runner_control_ca().display()
                ),
            ));
        }
        environment.push_str(&format!(
            "ASTER_RUNNER_CONTROL_CA_CERTIFICATE={}\n",
            install_layout().runner_control_ca().display()
        ));
    }
    if allow_insecure_http {
        environment.push_str("ASTER_RUNNER_ALLOW_INSECURE_HTTP=true\n");
    }
    install_restricted_text(&environment_path, "aster-runner", environment.as_bytes())?;
    let release = current_release()?;
    let mut process =
        ProcessCommand::new(install_layout().release_binary(&release, "aster-runner"));
    process
        .arg("enroll")
        .arg("--control-url")
        .arg(control_url)
        .arg("--token-file")
        .arg(token_file);
    if let Some(path) = control_ca_certificate {
        process.arg("--control-ca-certificate").arg(path);
    }
    if allow_insecure_http {
        process.arg("--allow-insecure-http=true");
    }
    let result =
        run_checked(process, delivery::RUNNER_FAILED, "Runner enrollment").and_then(|()| {
            #[cfg(unix)]
            {
                let identity = install_layout().runner_identity().display().to_string();
                let task_keys = install_layout().runner_task_keys().display().to_string();
                run_checked(
                    process_with_args("chown", &["root:aster-runner", &identity, &task_keys]),
                    delivery::RUNNER_FAILED,
                    "protect Runner identity",
                )?;
                run_checked(
                    process_with_args("chmod", &["0640", &identity, &task_keys]),
                    delivery::RUNNER_FAILED,
                    "protect Runner identity",
                )?;
            }
            run_service_action("enable", &["aster-runner.service"], delivery::RUNNER_FAILED)?;
            run_service_action("start", &["aster-runner.service"], delivery::RUNNER_FAILED)
        });
    if result.is_err() && !environment_existed {
        let _ = fs::remove_file(environment_path);
    }
    result?;
    println!("Runner enrollment completed; service is active.");
    Ok(())
}

fn show_runner_status() -> Result<(), CliFailure> {
    let unit = selected_runner_unit()?;
    let layout = install_layout();
    let slot = unit
        .strip_prefix("aster-runner@")
        .and_then(|unit| unit.strip_suffix(".service"));
    let identity = slot
        .map(|slot| layout.runner_slot_identity(slot))
        .unwrap_or_else(|| layout.runner_identity());
    let keys = slot
        .map(|slot| layout.runner_slot_task_keys(slot))
        .unwrap_or_else(|| layout.runner_task_keys());
    if !service_definition_exists(&unit)? {
        return Err(CliFailure::new(
            delivery::RUNNER_FAILED,
            "Runner service is not installed on this host",
        ));
    }
    println!("Runner: {}", service_state(&unit)?);
    println!(
        "identity: {}",
        if identity.is_file() {
            "present"
        } else {
            "missing"
        }
    );
    println!(
        "task keys: {}",
        if keys.is_file() { "present" } else { "missing" }
    );
    Ok(())
}

fn uninstall(purge: bool) -> Result<(), CliFailure> {
    // An interrupted purge may already have removed the role marker. Requiring it here would
    // permanently strand the remaining product directories, even though --purge has its own
    // explicit confirmation and a closed path allowlist.
    if !purge {
        installed_role()?;
    }
    if purge {
        if !io::stdin().is_terminal() {
            return Err(CliFailure::new(
                delivery::INPUT_INVALID,
                "--purge requires an interactive terminal confirmation",
            ));
        }
        if read_line("Type PURGE to delete all Aster Team data: ")? != "PURGE" {
            return Err(CliFailure::new(
                delivery::INPUT_INVALID,
                "purge confirmation did not match",
            ));
        }
    }
    let _lock = acquire_maintenance_lock()?;
    remove_platform_services()?;
    remove_command_link(&install_layout())?;
    #[cfg(target_os = "windows")]
    {
        let paths = if purge {
            vec![install_layout().root().to_path_buf()]
        } else {
            vec![
                install_layout().current(),
                install_layout().releases(),
                install_layout().staging(),
                install_layout().stable_bin(),
            ]
        };
        schedule_windows_cleanup(&paths)?;
        println!(
            "Aster Team programs stopped. {} will be removed immediately after this CLI exits.",
            if purge {
                "The installation root and all data"
            } else {
                "Program files"
            }
        );
        if !purge {
            println!(
                "To restore this installation, run init.ps1 from a signed release, then run `aster-team-cli.exe install`; existing accounts, passwords, settings, and data will be retained."
            );
        }
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        if purge {
            remove_tree(install_layout().root())?;
        } else {
            remove_managed_reference(&install_layout().current())?;
            for path in [
                install_layout().stable_bin(),
                install_layout().releases(),
                install_layout().staging(),
            ] {
                remove_tree(&path)?;
            }
        }
        println!(
            "Aster Team programs removed. Data {}.",
            if purge { "deleted" } else { "preserved" }
        );
        if !purge {
            let initializer = if cfg!(target_os = "macos") {
                "init-macos.sh"
            } else {
                "init.sh"
            };
            println!(
                "To restore this installation, run {initializer} from a signed release, then run `sudo aster-team-cli install`; existing accounts, passwords, settings, and data will be retained."
            );
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn schedule_windows_cleanup(paths: &[PathBuf]) -> Result<(), CliFailure> {
    if paths.is_empty() || paths.iter().any(|path| !uninstall_path_allowed(path)) {
        return Err(CliFailure::new(
            delivery::UNINSTALL_FAILED,
            "refused to schedule cleanup outside the installation root",
        ));
    }
    let layout = install_layout();
    let root = layout.root();
    if std::env::current_dir().is_ok_and(|directory| directory.starts_with(root)) {
        return Err(CliFailure::new(
            delivery::UNINSTALL_FAILED,
            format!(
                "change the current directory to a location outside {} before uninstalling",
                root.display()
            ),
        ));
    }
    let helper_directory = root.parent().ok_or_else(|| {
        CliFailure::new(
            delivery::UNINSTALL_FAILED,
            "installation root has no safe parent directory",
        )
    })?;
    let encoded = serde_json::to_string(paths)
        .map_err(|error| CliFailure::new(delivery::UNINSTALL_FAILED, error.to_string()))?;
    let script = r#"$parentId = [int][Environment]::GetEnvironmentVariable('ASTER_TEAM_UNINSTALL_PARENT_PID', 'Process'); $encoded = [Environment]::GetEnvironmentVariable('ASTER_TEAM_UNINSTALL_PATHS', 'Process'); Wait-Process -Id $parentId -ErrorAction SilentlyContinue; foreach ($path in ($encoded | ConvertFrom-Json)) { if (Get-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue) { Remove-Item -LiteralPath $path -Recurse -Force -ErrorAction Stop } }"#;
    ProcessCommand::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            script,
        ])
        .env(
            "ASTER_TEAM_UNINSTALL_PARENT_PID",
            std::process::id().to_string(),
        )
        .env("ASTER_TEAM_UNINSTALL_PATHS", encoded)
        .current_dir(helper_directory)
        .spawn()
        .map(|_| ())
        .map_err(|error| {
            CliFailure::new(
                delivery::UNINSTALL_FAILED,
                format!("could not schedule post-exit cleanup: {error}"),
            )
        })
}

fn remove_platform_services() -> Result<(), CliFailure> {
    let runtime_units = [
        #[cfg(target_os = "linux")]
        "aster-runner@blue.service",
        #[cfg(target_os = "linux")]
        "aster-runner@green.service",
        "aster-runner.service",
        "aster-caddy.service",
        "aster-control@blue.service",
        "aster-control@green.service",
        "aster-upgrade.path",
    ];
    #[cfg(target_os = "windows")]
    for unit in runtime_units {
        verify_windows_task_ownership(unit)?;
    }
    for unit in runtime_units {
        if service_definition_exists(unit)? {
            let _ = run_service_action("stop", &[unit], delivery::UNINSTALL_FAILED);
            let _ = run_service_action("disable", &[unit], delivery::UNINSTALL_FAILED);
        }
    }
    #[cfg(target_os = "linux")]
    {
        for unit in [
            "aster-runner@.service",
            "aster-runner.service",
            "aster-caddy.service",
            "aster-control@.service",
            "aster-upgrade.path",
            "aster-upgrade.service",
        ] {
            let path = service_registration_root()?.join(unit);
            if path.is_file() {
                fs::remove_file(path).map_err(|error| {
                    CliFailure::new(delivery::UNINSTALL_FAILED, error.to_string())
                })?;
            }
        }
        run_checked(
            process_with_args("systemctl", &["daemon-reload"]),
            delivery::UNINSTALL_FAILED,
            "systemctl daemon-reload",
        )?;
    }
    #[cfg(target_os = "windows")]
    {
        let mut removed = std::collections::BTreeSet::new();
        for unit in runtime_units {
            let task = windows_task_name(unit)?;
            if removed.insert(task.clone()) {
                let status = ProcessCommand::new("schtasks.exe")
                    .args(["/Delete", "/TN", &task, "/F"])
                    .status()
                    .map_err(|error| {
                        CliFailure::new(delivery::UNINSTALL_FAILED, error.to_string())
                    })?;
                if !status.success() && service_definition_exists(unit)? {
                    return Err(CliFailure::new(
                        delivery::UNINSTALL_FAILED,
                        format!("could not delete scheduled task {task}"),
                    ));
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let root = service_registration_root()?;
        let mut removed = std::collections::BTreeSet::new();
        for unit in runtime_units {
            let label = macos_service_label(unit)?;
            if removed.insert(label) {
                let path = root.join(format!("{label}.plist"));
                match fs::remove_file(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(CliFailure::new(
                            delivery::UNINSTALL_FAILED,
                            error.to_string(),
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

fn bootstrap(arguments: BootstrapArgs) -> Result<(), CliFailure> {
    let layout = match arguments.install_root {
        Some(root) => InstallLayout::new(root).map_err(|error| {
            CliFailure::new(
                delivery::INPUT_INVALID,
                format!("invalid install root: {error}"),
            )
        })?,
        None => InstallLayout::platform_default().map_err(|error| {
            CliFailure::new(
                delivery::INPUT_INVALID,
                format!("invalid install root: {error}"),
            )
        })?,
    };
    let directories = if arguments.runner_only {
        vec![
            layout.root().to_path_buf(),
            layout.stable_bin(),
            layout.releases(),
            layout.cli_private(),
            layout.runner_config(),
            layout.service_config(),
            layout.runner_data(),
            layout.runtime(),
            layout.state(),
            layout.locks(),
            layout.staging(),
            layout.backups(),
            layout.logs(),
        ]
    } else {
        layout.required_directories().map_err(|error| {
            CliFailure::new(
                delivery::FILESYSTEM_FAILED,
                format!("invalid install layout: {error}"),
            )
        })?
    };
    for directory in directories {
        fs::create_dir_all(&directory)
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    }
    let marker_path = layout.marker_path();
    if marker_path.is_file() {
        layout
            .verify_marker_bytes(
                &fs::read(&marker_path).map_err(|error| {
                    CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string())
                })?,
            )
            .map_err(|error| CliFailure::new(delivery::INPUT_INVALID, error.to_string()))?;
    } else {
        atomic_write_new_file(
            &marker_path,
            &layout
                .marker_json()
                .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?,
            0o644,
        )?;
    }
    let lock = open_lock_file(&layout.maintenance_lock())?;
    lock.try_lock().map_err(|error| match error {
        fs::TryLockError::WouldBlock => CliFailure::new(
            delivery::MAINTENANCE_BUSY,
            "the maintenance lock is already held",
        ),
        fs::TryLockError::Error(error) => {
            CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string())
        }
    })?;
    let release_root = arguments.release_root.canonicalize().map_err(|error| {
        CliFailure::new(
            delivery::RELEASE_INVALID,
            format!("could not resolve release root: {error}"),
        )
    })?;
    let release = if arguments.runner_only {
        verify_selected_release_at(&release_root)?
    } else {
        verify_release_at(&release_root)?
    };
    let expected_cli = layout.release_binary(&release_root, "aster-team-cli");
    let running_cli = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    let expected_cli = expected_cli.canonicalize().map_err(|error| {
        CliFailure::new(
            delivery::RELEASE_INVALID,
            format!("the signed release CLI is unavailable: {error}"),
        )
    })?;
    if running_cli != expected_cli {
        return Err(CliFailure::new(
            delivery::RELEASE_INVALID,
            "bootstrap must run the aster-team-cli contained in the selected release",
        ));
    }
    let install_target = arguments
        .install_target
        .unwrap_or_else(|| layout.stable_cli());
    let selection_file = arguments
        .selection_file
        .unwrap_or_else(|| layout.selected_release());
    atomic_copy_executable(&expected_cli, &install_target)?;
    install_command_link(&layout, &install_target)?;
    let selection = SelectedRelease {
        schema: SELECTED_RELEASE_SCHEMA.to_owned(),
        version: release.claims().version.clone(),
        release_root,
        manifest_sha256: sha256_file(&arguments.release_root.join(RELEASE_MANIFEST_FILE))?,
    };
    atomic_write_selection(&selection_file, &selection)?;
    println!(
        "Aster Team {} verified; aster-team-cli installed at {}.",
        selection.version,
        install_target.display()
    );
    if !arguments.runner_only {
        print_next_command(&layout);
    }
    Ok(())
}

fn print_next_command(layout: &InstallLayout) {
    let current = layout.current();
    let command = layout
        .command_link()
        .ok()
        .flatten()
        .unwrap_or_else(|| layout.stable_cli());
    let command = command.display();
    let privileged = if cfg!(target_os = "windows") {
        format!("\"{command}\"")
    } else {
        format!("sudo \"{command}\"")
    };
    if current.exists() {
        let runner_role = layout.runner_role();
        if runner_role.is_file() {
            println!("Next: {privileged} runner upgrade");
        } else {
            println!("Next: {privileged} upgrade");
        }
    } else {
        println!("Next (Control): {privileged} install");
        println!("Next (Runner):  {privileged} runner install");
    }
}

fn append_install_database_arguments(
    arguments: &InstallArgs,
    output: &mut Vec<String>,
) -> Result<(), CliFailure> {
    let Some(source) = &arguments.database_config else {
        return Ok(());
    };
    if Platform::current() != Platform::Linux || std::env::consts::ARCH != "x86_64" {
        return Err(CliFailure::new(
            delivery::INPUT_INVALID,
            "database configuration installation is supported only on Linux amd64",
        ));
    }
    validate_regular_file(source, "database configuration")?;
    let config = aster_install_layout::DatabaseConfiguration::parse(
        &fs::read(source)
            .map_err(|error| CliFailure::new(delivery::INPUT_INVALID, error.to_string()))?,
    )
    .map_err(|error| CliFailure::new(delivery::INPUT_INVALID, error.to_string()))?;
    match config {
        aster_install_layout::DatabaseConfiguration::Mariadb { custom_ca, .. } => {
            let password = arguments.database_password_file.as_ref().ok_or_else(|| {
                CliFailure::new(
                    delivery::INPUT_INVALID,
                    "--database-password-file is required for MariaDB",
                )
            })?;
            validate_secret_file(password)?;
            if custom_ca != arguments.database_ca_certificate.is_some() {
                return Err(CliFailure::new(
                    delivery::INPUT_INVALID,
                    "database custom_ca must match --database-ca-certificate",
                ));
            }
            output.extend([
                "--database-password-file".to_owned(),
                password.display().to_string(),
            ]);
            if let Some(certificate) = &arguments.database_ca_certificate {
                validate_regular_file(certificate, "database CA certificate")?;
                output.extend([
                    "--database-ca-certificate".to_owned(),
                    certificate.display().to_string(),
                ]);
            }
        }
        aster_install_layout::DatabaseConfiguration::Sqlcipher {} => {
            if arguments.database_password_file.is_some()
                || arguments.database_ca_certificate.is_some()
            {
                return Err(CliFailure::new(
                    delivery::INPUT_INVALID,
                    "SQLCipher does not accept MariaDB password or CA inputs",
                ));
            }
        }
    }
    output.extend(["--database-config".to_owned(), source.display().to_string()]);
    Ok(())
}

fn install_control(arguments: InstallArgs) -> Result<(), CliFailure> {
    if install_layout().current().exists() {
        return Err(CliFailure::new(
            delivery::INPUT_INVALID,
            "Aster Team is already installed; use `aster-team-cli upgrade`",
        ));
    }
    match preserved_control_state()? {
        PreservedControlState::Fresh if arguments.recover_preserved => {
            return Err(CliFailure::new(
                delivery::INPUT_INVALID,
                "no complete preserved Control installation was found; run `aster-team-cli install` without --recover-preserved for a new installation",
            ));
        }
        PreservedControlState::Fresh => {}
        PreservedControlState::Incomplete { missing } => {
            return Err(CliFailure::new(
                delivery::INPUT_INVALID,
                format!(
                    "preserved or interrupted installation data is incomplete (missing: {}); restore a complete backup or run `aster-team-cli uninstall --purge` before starting a new installation",
                    missing.join(", ")
                ),
            ));
        }
        PreservedControlState::Recoverable => {
            return recover_preserved_control(&arguments);
        }
    }
    if arguments.unattended {
        let email =
            arguments.owner_email.as_ref().cloned().ok_or_else(|| {
                CliFailure::new(delivery::INPUT_INVALID, "--owner-email is required")
            })?;
        validate_email(&email)?;
        let password_file = arguments
            .owner_password_file
            .as_ref()
            .cloned()
            .ok_or_else(|| {
                CliFailure::new(delivery::INPUT_INVALID, "--owner-password-file is required")
            })?;
        validate_secret_file(&password_file)?;
        let mut installer_arguments = vec![
            "--owner-email".to_owned(),
            email,
            "--owner-password-file".to_owned(),
            password_file.display().to_string(),
        ];
        append_install_database_arguments(&arguments, &mut installer_arguments)?;
        append_install_access_arguments(&arguments, &mut installer_arguments)?;
        if arguments.install_local_runner {
            installer_arguments.push("--install-local-runner".to_owned());
        }
        run_selected_installer(
            InstallRole::Control,
            InstallAction::Install,
            &installer_arguments,
        )?;
        return complete_initial_license_setup();
    }
    if arguments.owner_email.is_some()
        || arguments.owner_password_file.is_some()
        || arguments.install_local_runner
        || arguments.access_host.is_some()
        || arguments.bind_address.is_some()
        || arguments.certificate_source.is_some()
        || arguments.tls_certificate.is_some()
        || arguments.tls_private_key.is_some()
    {
        return Err(CliFailure::new(
            delivery::INPUT_INVALID,
            "install flags require --unattended; interactive installs collect these choices",
        ));
    }
    if !io::stdin().is_terminal() {
        return Err(CliFailure::new(
            delivery::INPUT_INVALID,
            "interactive install requires a terminal; use --unattended",
        ));
    }
    let email = read_line(&format!("Owner email [{DEFAULT_OWNER_EMAIL}]: "))?;
    let email = if email.is_empty() {
        DEFAULT_OWNER_EMAIL.to_owned()
    } else {
        email
    };
    validate_email(&email)?;
    println!("Data storage: local SQLCipher");
    println!(
        "Data directory: {} (included in backup and restore)",
        install_layout().data().display()
    );
    let mut installer_arguments = Vec::new();
    println!("Access protocol:");
    println!("  1) HTTP (default)");
    println!("  2) HTTPS");
    let protocol = match read_line("Select access protocol [1]: ")?.as_str() {
        "" | "1" => AccessProtocol::Http,
        "2" => AccessProtocol::Https,
        _ => {
            return Err(CliFailure::new(
                delivery::INPUT_INVALID,
                "access protocol must be 1 or 2",
            ));
        }
    };
    let detected_ip = primary_lan_ipv4();
    let address_prompt = detected_ip.map_or_else(
        || "Access IP or base domain: ".to_owned(),
        |value| format!("Access IP or base domain [{value}]: "),
    );
    let raw_host = read_line(&address_prompt)?;
    let raw_host = if raw_host.is_empty() {
        detected_ip.map(|value| value.to_string()).ok_or_else(|| {
            CliFailure::new(
                delivery::INPUT_INVALID,
                "the primary LAN IPv4 address could not be detected; enter an IP or domain",
            )
        })?
    } else {
        raw_host
    };
    let (access_host, host_ip) = normalize_access_host(&raw_host)?;
    let bind_address = host_ip.or(detected_ip).ok_or_else(|| {
        CliFailure::new(
            delivery::INPUT_INVALID,
            "a LAN bind address is required when the access address is a domain",
        )
    })?;
    installer_arguments.extend([
        "--access-protocol".to_owned(),
        protocol.as_str().to_owned(),
        "--access-host".to_owned(),
        access_host,
        "--bind-address".to_owned(),
        bind_address.to_string(),
    ]);
    if protocol == AccessProtocol::Https {
        println!("HTTPS certificate source:");
        println!("  1) Generate with Caddy's internal CA (default)");
        println!("  2) Use an existing certificate and private key");
        match read_line("Select certificate source [1]: ")?.as_str() {
            "" | "1" => installer_arguments.extend([
                "--certificate-source".to_owned(),
                CertificateSource::Caddy.as_str().to_owned(),
            ]),
            "2" => {
                let certificate = PathBuf::from(read_line("TLS certificate chain path: ")?);
                let private_key = PathBuf::from(read_line("TLS private key path: ")?);
                validate_regular_file(&certificate, "TLS certificate")?;
                validate_secret_file(&private_key)?;
                installer_arguments.extend([
                    "--certificate-source".to_owned(),
                    CertificateSource::Provided.as_str().to_owned(),
                    "--tls-certificate".to_owned(),
                    certificate.display().to_string(),
                    "--tls-private-key".to_owned(),
                    private_key.display().to_string(),
                ]);
            }
            _ => {
                return Err(CliFailure::new(
                    delivery::INPUT_INVALID,
                    "certificate source must be 1 or 2",
                ));
            }
        }
    }
    let install_local_runner = read_yes_no("Install a local Runner on this host? [Y/n]: ", true)?;
    if install_local_runner {
        installer_arguments.push("--install-local-runner".to_owned());
    }

    let password = generate_initial_password()?;
    let secret = TemporarySecret::new(password.as_bytes())?;
    installer_arguments.extend([
        "--owner-email".to_owned(),
        email.clone(),
        "--owner-password-file".to_owned(),
        secret.path.display().to_string(),
    ]);
    run_selected_installer(
        InstallRole::Control,
        InstallAction::Install,
        &installer_arguments,
    )?;
    complete_initial_license_setup()?;
    print_initial_credentials(&email, password.as_str());
    Ok(())
}

fn recover_preserved_control(arguments: &InstallArgs) -> Result<(), CliFailure> {
    if arguments.unattended && !arguments.recover_preserved {
        return Err(CliFailure::new(
            delivery::INPUT_INVALID,
            "a complete preserved installation was found; rerun with `--unattended --recover-preserved` to restore it without changing accounts or data",
        ));
    }
    if !arguments.recover_preserved {
        if !io::stdin().is_terminal() {
            return Err(CliFailure::new(
                delivery::INPUT_INVALID,
                "a complete preserved installation was found; rerun with `--recover-preserved` in a non-interactive environment",
            ));
        }
        let access = load_access_configuration()?;
        println!("A complete preserved Aster Team installation was found.");
        println!("  Admin:  {}", access.admin_url);
        println!("  Member: {}", access.member_url);
        println!("  API:    {}", access.api_url);
        println!("Existing accounts, passwords, settings, and SQLCipher data will be retained.");
        if !read_yes_no("Restore this installation now? [Y/n]: ", true)? {
            return Err(CliFailure::new(
                delivery::INPUT_INVALID,
                "recovery was cancelled before any installation changes were made",
            ));
        }
    }
    let access = load_access_configuration()?;
    let mut installer_arguments = vec![
        "--recover-preserved".to_owned(),
        "--access-protocol".to_owned(),
        access.protocol,
        "--access-host".to_owned(),
        access.host,
        "--bind-address".to_owned(),
        access.bind_address,
    ];
    if access.certificate_source != "none" {
        installer_arguments.extend(["--certificate-source".to_owned(), access.certificate_source]);
    }
    run_selected_installer(
        InstallRole::Control,
        InstallAction::Install,
        &installer_arguments,
    )?;
    println!("Preserved installation restored; existing accounts and passwords were not changed.");
    Ok(())
}

fn append_install_access_arguments(
    arguments: &InstallArgs,
    installer_arguments: &mut Vec<String>,
) -> Result<(), CliFailure> {
    let detected_ip = primary_lan_ipv4();
    let raw_host = arguments
        .access_host
        .clone()
        .or_else(|| detected_ip.map(|value| value.to_string()))
        .ok_or_else(|| {
            CliFailure::new(
                delivery::INPUT_INVALID,
                "--access-host is required when the primary LAN IPv4 address cannot be detected",
            )
        })?;
    let (host, host_ip) = normalize_access_host(&raw_host)?;
    let bind_address = arguments
        .bind_address
        .or(host_ip)
        .or(detected_ip)
        .ok_or_else(|| {
            CliFailure::new(
                delivery::INPUT_INVALID,
                "--bind-address is required when the access address is a domain",
            )
        })?;
    if bind_address.is_unspecified() || bind_address.is_multicast() {
        return Err(CliFailure::new(
            delivery::INPUT_INVALID,
            "--bind-address must identify a concrete unicast interface address",
        ));
    }
    installer_arguments.extend([
        "--access-protocol".to_owned(),
        arguments.access_protocol.as_str().to_owned(),
        "--access-host".to_owned(),
        host,
        "--bind-address".to_owned(),
        bind_address.to_string(),
    ]);
    match arguments.access_protocol {
        AccessProtocol::Http => {
            if arguments.certificate_source.is_some()
                || arguments.tls_certificate.is_some()
                || arguments.tls_private_key.is_some()
            {
                return Err(CliFailure::new(
                    delivery::INPUT_INVALID,
                    "HTTP access does not accept certificate options",
                ));
            }
        }
        AccessProtocol::Https => match arguments
            .certificate_source
            .unwrap_or(CertificateSource::Caddy)
        {
            CertificateSource::Caddy => {
                if arguments.tls_certificate.is_some() || arguments.tls_private_key.is_some() {
                    return Err(CliFailure::new(
                        delivery::INPUT_INVALID,
                        "Caddy-generated certificates do not accept certificate files",
                    ));
                }
                installer_arguments.extend([
                    "--certificate-source".to_owned(),
                    CertificateSource::Caddy.as_str().to_owned(),
                ]);
            }
            CertificateSource::Provided => {
                let certificate = arguments.tls_certificate.as_deref().ok_or_else(|| {
                    CliFailure::new(delivery::INPUT_INVALID, "--tls-certificate is required")
                })?;
                let private_key = arguments.tls_private_key.as_deref().ok_or_else(|| {
                    CliFailure::new(delivery::INPUT_INVALID, "--tls-private-key is required")
                })?;
                validate_regular_file(certificate, "TLS certificate")?;
                validate_secret_file(private_key)?;
                installer_arguments.extend([
                    "--certificate-source".to_owned(),
                    CertificateSource::Provided.as_str().to_owned(),
                    "--tls-certificate".to_owned(),
                    certificate.display().to_string(),
                    "--tls-private-key".to_owned(),
                    private_key.display().to_string(),
                ]);
            }
        },
    }
    Ok(())
}

fn primary_lan_ipv4() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    let address = socket.local_addr().ok()?.ip();
    matches!(address, IpAddr::V4(value) if !value.is_loopback() && !value.is_unspecified())
        .then_some(address)
}

fn normalize_access_host(value: &str) -> Result<(String, Option<IpAddr>), CliFailure> {
    let value = value.trim();
    if let Ok(address) = value.parse::<IpAddr>() {
        if address.is_unspecified() || address.is_multicast() {
            return Err(CliFailure::new(
                delivery::INPUT_INVALID,
                "the access IP must be a concrete unicast address",
            ));
        }
        return Ok((address.to_string(), Some(address)));
    }
    let value = value.trim_end_matches('.').to_ascii_lowercase();
    let valid = (1..=253).contains(&value.len())
        && value.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        });
    if !valid {
        return Err(CliFailure::new(
            delivery::INPUT_INVALID,
            "the access address must be an IP or a valid base domain without a scheme or port",
        ));
    }
    Ok((value, None))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InstallRole {
    Control,
    Runner,
}

impl InstallRole {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Runner => "runner",
        }
    }
}

#[derive(Clone, Copy)]
enum InstallAction {
    Install,
    Upgrade,
}

fn run_selected_installer(
    role: InstallRole,
    action: InstallAction,
    extra_arguments: &[String],
) -> Result<(), CliFailure> {
    let selected = load_selected_release(&install_layout().selected_release())?;
    let current_exists = install_layout().current().exists();
    match (action, current_exists) {
        (InstallAction::Install, true) => {
            return Err(CliFailure::new(
                delivery::INPUT_INVALID,
                "Aster Team is already installed; use the matching upgrade command",
            ));
        }
        (InstallAction::Upgrade, false) => {
            return Err(CliFailure::new(
                delivery::INPUT_INVALID,
                "Aster Team is not installed; use the matching install command",
            ));
        }
        _ => {}
    }
    if matches!(action, InstallAction::Upgrade) {
        let current_version = read_trimmed(&current_release()?.join("VERSION"))?;
        maintenance_executor::validate_upgrade_version(&current_version, &selected.version)?;
    }
    #[cfg(target_os = "windows")]
    verify_windows_instance_support(&install_layout(), &selected.release_root)?;
    let installer = install_layout()
        .release_platform_path(&selected.release_root, "install_engine")
        .map_err(|error| CliFailure::new(delivery::RELEASE_INVALID, error.to_string()))?;
    if !installer.is_file() {
        return Err(CliFailure::new(
            delivery::RELEASE_INVALID,
            format!(
                "signed internal installer is missing: {}",
                installer.display()
            ),
        ));
    }
    #[cfg(target_os = "linux")]
    let mut command = {
        let mut command = ProcessCommand::new("bash");
        command
            .arg(&installer)
            .arg("--install-root")
            .arg(install_layout().root())
            .arg("--service-registration-root")
            .arg(service_registration_root()?);
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = ProcessCommand::new("powershell.exe");
        command
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&installer)
            .arg("--install-root")
            .arg(install_layout().root())
            .arg("--release-root")
            .arg(&selected.release_root);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = ProcessCommand::new("bash");
        command
            .arg(&installer)
            .arg("--install-root")
            .arg(install_layout().root())
            .arg("--service-registration-root")
            .arg(service_registration_root()?);
        command
    };
    if matches!(role, InstallRole::Runner) {
        command.arg("--runner-only");
    }
    if matches!(action, InstallAction::Install) {
        command
            .arg("--release-manifest-sha256")
            .arg(&selected.manifest_sha256);
    }
    command.args(extra_arguments);
    let status = command.status().map_err(|error| {
        CliFailure::new(
            delivery::COMMAND_FAILED,
            format!("could not start internal installer: {error}"),
        )
    })?;
    if status.success() {
        let release_root = current_release()?;
        let release = match role {
            InstallRole::Control => verify_release_at(&release_root)?,
            InstallRole::Runner => verify_selected_release_at(&release_root)?,
        };
        let installed = SelectedRelease {
            schema: SELECTED_RELEASE_SCHEMA.to_owned(),
            version: release.claims().version.clone(),
            manifest_sha256: sha256_file(&release_root.join(RELEASE_MANIFEST_FILE))?,
            release_root,
        };
        atomic_write_selection(&install_layout().selected_release(), &installed)
    } else {
        let descriptor = match action {
            InstallAction::Install => delivery::INSTALL_FAILED,
            InstallAction::Upgrade => delivery::UPGRADE_FAILED,
        };
        Err(command_failure(descriptor, "internal installer", status))
    }
}

fn restore_backup(role: InstallRole, source: &Path, confirm: bool) -> Result<(), CliFailure> {
    if role == InstallRole::Control && installed_database()?.is_external() {
        return Err(CliFailure::new(
            delivery::BACKUP_FAILED,
            "local SQLCipher restore is unavailable for an external database installation; restore the database snapshot and matching installation configuration together",
        ));
    }

    if !confirm {
        return Err(CliFailure::new(
            delivery::INPUT_INVALID,
            "--confirm is required because restore replaces live data",
        ));
    }
    validate_secret_file(source)?;
    if installed_role()? != role {
        return Err(CliFailure::new(
            delivery::BACKUP_FAILED,
            "restore role does not match this host",
        ));
    }
    let _lock = acquire_maintenance_lock()?;
    let restore = install_layout()
        .release_platform_path(&install_layout().current(), "restore_engine")
        .map_err(|error| CliFailure::new(delivery::RELEASE_INVALID, error.to_string()))?;
    if !restore.is_file() {
        return Err(CliFailure::new(
            delivery::RELEASE_INVALID,
            format!(
                "signed internal restore engine is missing: {}",
                restore.display()
            ),
        ));
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let mut command = {
        let mut command = ProcessCommand::new("bash");
        command
            .arg(restore)
            .arg("--backup")
            .arg(source)
            .arg("--install-root")
            .arg(install_layout().root())
            .arg("--service-registration-root")
            .arg(service_registration_root()?)
            .arg("--confirm-restore");
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = ProcessCommand::new("powershell.exe");
        command
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(restore)
            .arg("--backup")
            .arg(source)
            .arg("--install-root")
            .arg(install_layout().root())
            .arg("--confirm-restore");
        command
    };
    if matches!(role, InstallRole::Runner) {
        command.arg("--runner-only");
    }
    let status = command.status().map_err(|error| {
        CliFailure::new(
            delivery::BACKUP_FAILED,
            format!("could not start internal restore engine: {error}"),
        )
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(command_failure(
            delivery::BACKUP_FAILED,
            "internal restore engine",
            status,
        ))
    }
}

fn installed_role() -> Result<InstallRole, CliFailure> {
    let control = install_layout().control_role();
    let runner = install_layout().runner_role();
    match (control.is_file(), runner.is_file()) {
        (true, false) if read_trimmed(&control)? == "control" => Ok(InstallRole::Control),
        (false, true) if read_trimmed(&runner)? == "runner" => Ok(InstallRole::Runner),
        (false, false) => Err(CliFailure::new(
            delivery::INPUT_INVALID,
            "Aster Team is not installed",
        )),
        _ => Err(CliFailure::new(
            delivery::DIAGNOSTIC_FAILED,
            "installation role markers are conflicting or invalid",
        )),
    }
}

fn current_release() -> Result<PathBuf, CliFailure> {
    let current = install_layout().current();
    let release = current.canonicalize().map_err(|error| {
        CliFailure::new(
            delivery::DIAGNOSTIC_FAILED,
            format!("current release is unavailable: {error}"),
        )
    })?;
    let releases = install_layout()
        .releases()
        .canonicalize()
        .map_err(|error| {
            CliFailure::new(
                delivery::DIAGNOSTIC_FAILED,
                format!("release directory is unavailable: {error}"),
            )
        })?;
    if !release.starts_with(&releases) || !release.join("VERSION").is_file() {
        return Err(CliFailure::new(
            delivery::DIAGNOSTIC_FAILED,
            "current release link is outside the protected release directory",
        ));
    }
    Ok(release)
}

fn read_trimmed(path: &Path) -> Result<String, CliFailure> {
    fs::read_to_string(path)
        .map(|value| value.trim().to_owned())
        .map_err(|error| {
            CliFailure::new(
                delivery::FILESYSTEM_FAILED,
                format!("could not read {}: {error}", path.display()),
            )
        })
}

// An absent optional unit is different from an unreachable service manager.
// Preserve active state even when a running unit's file has been removed.
#[cfg(any(target_os = "linux", test))]
fn linux_service_state(properties: &str, succeeded: bool) -> Result<String, CliFailure> {
    let mut load = None;
    let mut active = None;
    for line in properties.lines().filter(|line| !line.trim().is_empty()) {
        match line.split_once('=') {
            Some(("LoadState", value)) if load.is_none() => load = Some(value),
            Some(("ActiveState", value)) if active.is_none() => active = Some(value),
            _ => {
                return Err(CliFailure::new(
                    delivery::SERVICE_FAILED,
                    "invalid systemd service properties",
                ));
            }
        }
    }
    match (load, active, succeeded) {
        (Some("not-found"), Some("inactive"), _) => Ok("not-installed".into()),
        (Some(_), Some(state), true)
            if matches!(
                state,
                "active" | "reloading" | "activating" | "deactivating" | "refreshing"
            ) =>
        {
            Ok(state.into())
        }
        (Some("loaded" | "masked"), Some(state @ ("inactive" | "failed")), true) => {
            Ok(state.into())
        }
        _ => Err(CliFailure::new(
            delivery::SERVICE_FAILED,
            "could not confirm systemd service state",
        )),
    }
}

fn service_state(unit: &str) -> Result<String, CliFailure> {
    #[cfg(target_os = "linux")]
    let output = ProcessCommand::new("systemctl")
        .args([
            "show",
            "--property=LoadState",
            "--property=ActiveState",
            "--",
        ])
        .arg(unit)
        .output()
        .map_err(|error| CliFailure::new(delivery::SERVICE_FAILED, error.to_string()))?;
    #[cfg(target_os = "windows")]
    let output = {
        let (path, name) = windows_task_parts(unit)?;
        ProcessCommand::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!(
                    "(Get-ScheduledTask -TaskPath '{}' -TaskName '{}').State.ToString()",
                    path, name
                ),
            ])
            .output()
            .map_err(|error| CliFailure::new(delivery::SERVICE_FAILED, error.to_string()))?
    };
    #[cfg(target_os = "macos")]
    let output = ProcessCommand::new("launchctl")
        .args(["print", &format!("system/{}", macos_service_label(unit)?)])
        .output()
        .map_err(|error| CliFailure::new(delivery::SERVICE_FAILED, error.to_string()))?;
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    #[cfg(target_os = "linux")]
    let value = linux_service_state(&value, output.status.success())?;
    #[cfg(target_os = "windows")]
    let value = match value.to_ascii_lowercase().as_str() {
        "running" => "active".to_owned(),
        "disabled" => "disabled".to_owned(),
        "ready" | "queued" => "inactive".to_owned(),
        _ if !output.status.success() => "not-installed".to_owned(),
        _ => "unknown".to_owned(),
    };
    #[cfg(target_os = "macos")]
    let value = if !output.status.success() {
        "inactive".to_owned()
    } else if value.lines().any(|line| line.trim() == "state = running") {
        "active".to_owned()
    } else {
        "inactive".to_owned()
    };
    Ok(if value.is_empty() {
        "unknown".to_owned()
    } else {
        value
    })
}

fn service_is_active(unit: &str) -> Result<bool, CliFailure> {
    Ok(service_state(unit)? == "active")
}

fn service_is_enabled(unit: &str) -> Result<bool, CliFailure> {
    #[cfg(target_os = "linux")]
    return ProcessCommand::new("systemctl")
        .arg("is-enabled")
        .arg("--quiet")
        .arg(unit)
        .status()
        .map(|status| status.success())
        .map_err(|error| CliFailure::new(delivery::SERVICE_FAILED, error.to_string()));
    #[cfg(target_os = "windows")]
    return Ok(!matches!(
        service_state(unit)?.as_str(),
        "disabled" | "not-installed"
    ));
    #[cfg(target_os = "macos")]
    return Ok(service_definition_exists(unit)?);
}

fn service_definition_exists(unit: &str) -> Result<bool, CliFailure> {
    #[cfg(target_os = "linux")]
    {
        let template = if unit.starts_with("aster-control@") {
            "aster-control@.service"
        } else if unit.starts_with("aster-runner@") {
            "aster-runner@.service"
        } else {
            unit
        };
        Ok(service_registration_root()?.join(template).is_file())
    }
    #[cfg(target_os = "windows")]
    {
        let task = windows_task_name(unit)?;
        ProcessCommand::new("schtasks.exe")
            .args(["/Query", "/TN", &task])
            .status()
            .map(|status| status.success())
            .map_err(|error| CliFailure::new(delivery::SERVICE_FAILED, error.to_string()))
    }
    #[cfg(target_os = "macos")]
    {
        Ok(service_registration_root()?
            .join(format!("{}.plist", macos_service_label(unit)?))
            .is_file())
    }
}

fn service_units(target: ServiceTarget) -> Result<Vec<String>, CliFailure> {
    let role = installed_role()?;
    match target {
        ServiceTarget::Control if role != InstallRole::Control => Err(CliFailure::new(
            delivery::SERVICE_FAILED,
            "a dedicated Runner host has no Control service",
        )),
        ServiceTarget::Control => {
            let mut units = vec![maintenance_executor::active_control_unit()?];
            if service_definition_exists("aster-caddy.service")?
                && (service_is_active("aster-caddy.service")?
                    || service_is_enabled("aster-caddy.service")?)
            {
                units.push("aster-caddy.service".to_owned());
            }
            Ok(units)
        }
        ServiceTarget::Runner => {
            let runner = selected_runner_unit()?;
            if !service_definition_exists(&runner)? {
                return Err(CliFailure::new(
                    delivery::SERVICE_FAILED,
                    "Runner service is not installed",
                ));
            }
            Ok(vec![runner])
        }
        ServiceTarget::All => {
            let mut units = Vec::new();
            let control_unit = maintenance_executor::active_control_unit().ok();
            let runner_unit = selected_runner_unit()?;
            for unit in [
                control_unit.as_deref(),
                Some("aster-caddy.service"),
                Some(runner_unit.as_str()),
            ]
            .into_iter()
            .flatten()
            {
                if service_definition_exists(unit)?
                    && (service_is_active(unit)? || service_is_enabled(unit)?)
                {
                    units.push(unit.to_owned());
                }
            }
            if units.is_empty() {
                units.push(match role {
                    InstallRole::Control => maintenance_executor::active_control_unit()?,
                    InstallRole::Runner => "aster-runner.service".to_owned(),
                });
            }
            Ok(units)
        }
    }
}

fn acquire_maintenance_lock() -> Result<File, CliFailure> {
    let lock = open_maintenance_lock()?;
    lock.try_lock().map_err(|error| match error {
        fs::TryLockError::WouldBlock => CliFailure::new(
            delivery::MAINTENANCE_BUSY,
            "the maintenance lock is already held",
        ),
        fs::TryLockError::Error(error) => {
            CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string())
        }
    })?;
    Ok(lock)
}

fn run_checked(
    mut command: ProcessCommand,
    descriptor: ErrorDescriptor,
    label: &str,
) -> Result<(), CliFailure> {
    let status = command
        .status()
        .map_err(|error| CliFailure::new(descriptor, format!("{label}: {error}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(command_failure(descriptor, label, status))
    }
}

fn run_checked_with_secret_stdin(
    mut command: ProcessCommand,
    secret: &[u8],
    descriptor: ErrorDescriptor,
    label: &str,
) -> Result<(), CliFailure> {
    command.stdin(std::process::Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| CliFailure::new(descriptor, format!("{label}: {error}")))?;
    let mut stdin = child.stdin.take().ok_or_else(|| {
        CliFailure::new(descriptor, format!("{label}: child stdin is unavailable"))
    })?;
    stdin
        .write_all(secret)
        .and_then(|()| stdin.write_all(b"\n"))
        .map_err(|error| CliFailure::new(descriptor, format!("{label}: {error}")))?;
    drop(stdin);
    let status = child
        .wait()
        .map_err(|error| CliFailure::new(descriptor, format!("{label}: {error}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(command_failure(descriptor, label, status))
    }
}

fn process_with_args(command: &str, arguments: &[&str]) -> ProcessCommand {
    let mut process = ProcessCommand::new(command);
    process.args(arguments);
    process
}

#[cfg(target_os = "linux")]
fn process_as_user(user: &str, program: &Path) -> ProcessCommand {
    let mut process = ProcessCommand::new("runuser");
    process.args(["-u", user, "--"]).arg(program);
    process
}

#[cfg(target_os = "macos")]
fn process_as_user(user: &str, program: &Path) -> ProcessCommand {
    let mut process = ProcessCommand::new("sudo");
    process.args(["-u", user, "--"]).arg(program);
    process
}

#[cfg(target_os = "windows")]
fn process_as_user(_user: &str, program: &Path) -> ProcessCommand {
    ProcessCommand::new(program)
}

pub(crate) fn run_service_action(
    action: &str,
    units: &[&str],
    descriptor: ErrorDescriptor,
) -> Result<(), CliFailure> {
    #[cfg(target_os = "linux")]
    {
        let mut command = ProcessCommand::new("systemctl");
        command.arg(action).args(units);
        run_checked(command, descriptor, "systemctl")
    }
    #[cfg(target_os = "windows")]
    {
        for unit in units {
            verify_windows_task_ownership(unit)?;
        }
        for unit in units {
            let task = windows_task_name(unit)?;
            match action {
                "start" => run_checked(
                    process_with_args("schtasks.exe", &["/Run", "/TN", &task]),
                    descriptor,
                    "start scheduled task",
                )?,
                "stop" => stop_windows_service(unit, descriptor)?,
                "restart" => {
                    stop_windows_service(unit, descriptor)?;
                    run_checked(
                        process_with_args("schtasks.exe", &["/Run", "/TN", &task]),
                        descriptor,
                        "start scheduled task",
                    )?;
                }
                "enable" | "disable" => run_checked(
                    process_with_args(
                        "schtasks.exe",
                        &[
                            "/Change",
                            "/TN",
                            &task,
                            if action == "enable" {
                                "/ENABLE"
                            } else {
                                "/DISABLE"
                            },
                        ],
                    ),
                    descriptor,
                    "change scheduled task",
                )?,
                _ => {
                    return Err(CliFailure::new(
                        descriptor,
                        format!("unsupported Windows service action: {action}"),
                    ));
                }
            }
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        for unit in units {
            let label = macos_service_label(unit)?;
            let target = format!("system/{label}");
            let plist = service_registration_root()?.join(format!("{label}.plist"));
            match action {
                "start" => {
                    if !service_is_active(unit)? {
                        let mut bootstrap = ProcessCommand::new("launchctl");
                        bootstrap.args(["bootstrap", "system"]).arg(&plist);
                        run_checked(bootstrap, descriptor, "launchctl bootstrap")?;
                    }
                    run_checked(
                        process_with_args("launchctl", &["kickstart", "-k", &target]),
                        descriptor,
                        "launchctl kickstart",
                    )?;
                }
                "stop" => {
                    if service_is_active(unit)? {
                        run_checked(
                            process_with_args("launchctl", &["bootout", &target]),
                            descriptor,
                            "launchctl bootout",
                        )?;
                    }
                }
                "restart" => {
                    if service_is_active(unit)? {
                        run_checked(
                            process_with_args("launchctl", &["bootout", &target]),
                            descriptor,
                            "launchctl bootout",
                        )?;
                    }
                    let mut bootstrap = ProcessCommand::new("launchctl");
                    bootstrap.args(["bootstrap", "system"]).arg(&plist);
                    run_checked(bootstrap, descriptor, "launchctl bootstrap")?;
                }
                "enable" | "disable" => run_checked(
                    process_with_args("launchctl", &[action, &target]),
                    descriptor,
                    "launchctl enablement",
                )?,
                _ => {
                    return Err(CliFailure::new(
                        descriptor,
                        format!("unsupported macOS service action: {action}"),
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn stop_windows_service(unit: &str, descriptor: ErrorDescriptor) -> Result<(), CliFailure> {
    let service = match unit {
        "aster-control@blue.service" => "control-blue",
        "aster-control@green.service" => "control-green",
        "aster-runner.service" => "runner",
        "aster-caddy.service" => "caddy",
        "aster-upgrade.path" | "aster-upgrade.service" => "maintenance",
        _ => {
            return Err(CliFailure::new(
                descriptor,
                format!("unknown Windows service: {unit}"),
            ));
        }
    };
    let mut command = ProcessCommand::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(
            install_layout()
                .platform_service_launcher()
                .map_err(|error| CliFailure::new(descriptor, error.to_string()))?,
        )
        .args(["-Service", service, "-Stop"]);
    run_checked(command, descriptor, "stop service process tree")
}

#[cfg(target_os = "windows")]
fn resolve_windows_instance(
    layout: &InstallLayout,
    initialize: bool,
    read: impl FnMut(&str) -> Option<String>,
) -> Result<aster_install_layout::WindowsInstance, CliFailure> {
    let mut marker = layout
        .read_marker()
        .map_err(|error| CliFailure::new(delivery::INPUT_INVALID, error.to_string()))?;
    // A rejected fresh preflight may be retried with another unused namespace or
    // port range. Once a role exists, terminal variables can never retarget it.
    if initialize && !layout.control_role().exists() && !layout.runner_role().exists() {
        let instance = aster_install_layout::WindowsInstance::from_environment(read)
            .map_err(|error| CliFailure::new(delivery::INPUT_INVALID, error.to_string()))?;
        let persisted =
            (instance != aster_install_layout::WindowsInstance::default()).then_some(instance);
        if marker.windows_instance != persisted {
            marker.windows_instance = persisted;
            let bytes = serde_json::to_vec_pretty(&marker)
                .map_err(|error| CliFailure::new(delivery::INPUT_INVALID, error.to_string()))?;
            maintenance_executor::atomic_replace(layout.marker_path(), &bytes, 0o644)?;
        }
    }
    Ok(marker.windows_instance.unwrap_or_default())
}

#[cfg(target_os = "windows")]
fn windows_task_name(unit: &str) -> Result<String, CliFailure> {
    let (path, name) = windows_task_parts(unit)?;
    Ok(format!("{path}{name}"))
}

#[cfg(target_os = "windows")]
fn verify_windows_task_ownership(unit: &str) -> Result<(), CliFailure> {
    let (path, name) = windows_task_parts(unit)?;
    let launcher = install_layout().service_config().join("service-launch.ps1");
    let expected = format!("\"{}\"", launcher.display()).replace('\'', "''");
    let script = format!(
        "$ErrorActionPreference = 'Stop'; \
         $task = @(Get-ScheduledTask -ErrorAction Stop | Where-Object {{ $_.TaskPath -eq '{path}' -and $_.TaskName -eq '{name}' }}); \
         if ($task.Count -gt 0) {{ \
         $actions = @($task[0].Actions); \
         if ($task.Count -ne 1 -or $actions.Count -ne 1 -or ([string]$actions[0].Arguments).IndexOf('{expected}', [StringComparison]::OrdinalIgnoreCase) -lt 0) \
         {{ throw 'Scheduled task belongs to another installation.' }} }}"
    );
    let mut command = ProcessCommand::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
    run_checked(
        command,
        delivery::SERVICE_FAILED,
        "scheduled task ownership verification",
    )
}

#[cfg(target_os = "windows")]
fn windows_task_parts(unit: &str) -> Result<(String, &'static str), CliFailure> {
    let name = match unit {
        "aster-control@blue.service" => "Control Blue",
        "aster-control@green.service" => "Control Green",
        "aster-runner.service" => "Runner",
        "aster-caddy.service" => "Caddy",
        "aster-upgrade.path" | "aster-upgrade.service" => "Maintenance",
        _ => {
            return Err(CliFailure::new(
                delivery::SERVICE_FAILED,
                format!("unknown Windows service: {unit}"),
            ));
        }
    };
    let instance = install_layout()
        .windows_instance()
        .map_err(|error| CliFailure::new(delivery::SERVICE_FAILED, error.to_string()))?;
    Ok((instance.task_path(), name))
}

fn instance_ports(layout: &InstallLayout) -> Result<WindowsPorts, CliFailure> {
    #[cfg(target_os = "windows")]
    return layout
        .windows_instance()
        .map(|instance| instance.ports)
        .map_err(|error| CliFailure::new(delivery::INPUT_INVALID, error.to_string()));
    #[cfg(not(target_os = "windows"))]
    {
        let _ = layout;
        Ok(WindowsPorts::default())
    }
}

#[cfg(target_os = "windows")]
fn verify_windows_instance_support(
    layout: &InstallLayout,
    release: &Path,
) -> Result<(), CliFailure> {
    let instance = layout
        .windows_instance()
        .map_err(|error| CliFailure::new(delivery::RELEASE_INVALID, error.to_string()))?;
    if instance == aster_install_layout::WindowsInstance::default() {
        return Ok(());
    }
    verify_selected_release_at(release)?;
    let output = ProcessCommand::new(layout.release_binary(release, "aster-team-cli"))
        .args(["windows-instance", "--install-root"])
        .arg(layout.root())
        .output()
        .map_err(|error| CliFailure::new(delivery::RELEASE_INVALID, error.to_string()))?;
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|_| {
        CliFailure::new(
            delivery::RELEASE_INVALID,
            "candidate does not support persisted Windows instances",
        )
    })?;
    if !output.status.success()
        || value["configuration"]
            != serde_json::to_value(instance)
                .map_err(|error| CliFailure::new(delivery::RELEASE_INVALID, error.to_string()))?
    {
        return Err(CliFailure::new(
            delivery::RELEASE_INVALID,
            "candidate changed or ignored the persisted Windows instance",
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn macos_service_label(unit: &str) -> Result<&'static str, CliFailure> {
    Ok(match unit {
        "aster-control@blue.service" => "com.aster-team.control.blue",
        "aster-control@green.service" => "com.aster-team.control.green",
        "aster-runner.service" => "com.aster-team.runner",
        "aster-caddy.service" => "com.aster-team.caddy",
        "aster-upgrade.path" | "aster-upgrade.service" => "com.aster-team.maintenance",
        _ => {
            return Err(CliFailure::new(
                delivery::SERVICE_FAILED,
                format!("unknown macOS service: {unit}"),
            ));
        }
    })
}

fn active_units(units: &[&str]) -> Result<Vec<String>, CliFailure> {
    let mut active = Vec::new();
    for unit in units {
        if service_is_active(unit)? {
            active.push((*unit).to_owned());
        }
    }
    Ok(active)
}

fn read_environment_file(
    path: &Path,
    allowed: &[&str],
) -> Result<Vec<(String, String)>, CliFailure> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(CliFailure::new(
                delivery::DIAGNOSTIC_FAILED,
                format!("could not read {}: {error}", path.display()),
            ));
        }
    };
    let mut output = Vec::new();
    for raw in source.lines() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or_else(|| {
            CliFailure::new(
                delivery::DIAGNOSTIC_FAILED,
                format!("{} contains a malformed setting", path.display()),
            )
        })?;
        if !allowed.contains(&key) || value.is_empty() || value.chars().any(char::is_whitespace) {
            return Err(CliFailure::new(
                delivery::DIAGNOSTIC_FAILED,
                format!("{} contains an unsupported setting", path.display()),
            ));
        }
        if output.iter().any(|(existing, _)| existing == key) {
            return Err(CliFailure::new(
                delivery::DIAGNOSTIC_FAILED,
                format!("{} contains a duplicate setting", path.display()),
            ));
        }
        output.push((key.to_owned(), value.to_owned()));
    }
    Ok(output)
}

fn validate_regular_file(path: &Path, label: &str) -> Result<(), CliFailure> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        CliFailure::new(
            delivery::INPUT_INVALID,
            format!("{label} is unavailable: {error}"),
        )
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(CliFailure::new(
            delivery::INPUT_INVALID,
            format!("{label} must be a regular, non-symlinked file"),
        ));
    }
    Ok(())
}

fn validate_new_output(path: &Path) -> Result<(), CliFailure> {
    if !path.is_absolute()
        || path.exists()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(CliFailure::new(
            delivery::INPUT_INVALID,
            "backup output must be a new normalized absolute path",
        ));
    }
    if path.starts_with(install_layout().root()) && !path.starts_with(install_layout().backups()) {
        return Err(CliFailure::new(
            delivery::INPUT_INVALID,
            format!(
                "backup output inside the installation root must be below {}",
                install_layout().backups().display()
            ),
        ));
    }
    let parent = path
        .parent()
        .filter(|parent| parent.is_dir())
        .ok_or_else(|| {
            CliFailure::new(
                delivery::INPUT_INVALID,
                "backup output parent is unavailable",
            )
        })?;
    if fs::symlink_metadata(parent)
        .map_err(|error| CliFailure::new(delivery::INPUT_INVALID, error.to_string()))?
        .file_type()
        .is_symlink()
    {
        return Err(CliFailure::new(
            delivery::INPUT_INVALID,
            "backup output parent must not be a symlink",
        ));
    }
    Ok(())
}

fn create_backup_temporary(parent: &Path) -> Result<PathBuf, CliFailure> {
    let timestamp = time::OffsetDateTime::now_utc().unix_timestamp_nanos();
    for attempt in 0..64_u8 {
        let path = parent.join(format!(
            ".aster-team-backup.{}.{}.{}.tmp",
            std::process::id(),
            timestamp,
            attempt
        ));
        match OpenOptions::new().create_new(true).write(true).open(&path) {
            Ok(file) => {
                set_mode(&path, 0o600)?;
                file.sync_all().map_err(|error| {
                    CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string())
                })?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(CliFailure::new(
                    delivery::FILESYSTEM_FAILED,
                    error.to_string(),
                ));
            }
        }
    }
    Err(CliFailure::new(
        delivery::FILESYSTEM_FAILED,
        "could not allocate a temporary backup path",
    ))
}

fn publish_backup(temporary: &Path, output: &Path) -> Result<(), CliFailure> {
    let mut source = File::open(temporary)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    let mut destination = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    let result = io::copy(&mut source, &mut destination)
        .and_then(|_| destination.sync_all())
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))
        .and_then(|()| set_mode(output, 0o600))
        .and_then(|()| validate_regular_file(output, "backup output"));
    if result.is_err() {
        let _ = fs::remove_file(output);
    }
    result
}

fn runner_wss_url(control_url: &str, allow_insecure_http: bool) -> Result<String, CliFailure> {
    let mut value = url::Url::parse(control_url).map_err(|error| {
        CliFailure::new(
            delivery::INPUT_INVALID,
            format!("Control URL is invalid: {error}"),
        )
    })?;
    let secure = value.scheme() == "https";
    let loopback = value.scheme() == "http"
        && matches!(value.host_str(), Some("127.0.0.1" | "localhost" | "::1"));
    if (!secure && !loopback && !allow_insecure_http)
        || value.host_str().is_none()
        || !value.username().is_empty()
        || value.password().is_some()
        || !matches!(value.path(), "" | "/")
        || value.query().is_some()
        || value.fragment().is_some()
    {
        return Err(CliFailure::new(
            delivery::INPUT_INVALID,
            "Control URL must be an HTTPS origin, an explicitly allowed HTTP origin, or a loopback HTTP origin without credentials, path, query, or fragment",
        ));
    }
    value
        .set_scheme(if secure { "wss" } else { "ws" })
        .map_err(|()| CliFailure::new(delivery::INPUT_INVALID, "Control URL is invalid"))?;
    value.set_path("/api/runner/channel");
    Ok(value.to_string())
}

fn install_restricted_text(path: &Path, _group: &str, value: &[u8]) -> Result<(), CliFailure> {
    #[cfg(target_os = "windows")]
    {
        let parent = path.parent().ok_or_else(|| {
            CliFailure::new(delivery::FILESYSTEM_FAILED, "restricted file has no parent")
        })?;
        fs::create_dir_all(parent)
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
        let temporary = parent.join(format!(".restricted.{}.tmp", std::process::id()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
        let result = file
            .write_all(value)
            .and_then(|()| file.sync_all())
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))
            .and_then(|()| replace_temporary_file(&temporary, path));
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
    #[cfg(unix)]
    {
        let temporary = temporary_runtime_file("restricted")?;
        let result = (|| -> Result<(), CliFailure> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
            set_mode(&temporary, 0o600)?;
            file.write_all(value)
                .and_then(|()| file.sync_all())
                .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
            let mut install = ProcessCommand::new("install");
            install
                .args(["-o", "root", "-g", _group, "-m", "0640"])
                .arg(&temporary)
                .arg(path);
            run_checked(
                install,
                delivery::FILESYSTEM_FAILED,
                "install restricted file",
            )
        })();
        let _ = fs::remove_file(&temporary);
        result
    }
}

#[cfg(not(target_os = "windows"))]
fn remove_tree(path: &Path) -> Result<(), CliFailure> {
    if !uninstall_path_allowed(path) {
        return Err(CliFailure::new(
            delivery::UNINSTALL_FAILED,
            format!(
                "refused to remove path outside the uninstall allowlist: {}",
                path.display()
            ),
        ));
    }
    if !path.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| CliFailure::new(delivery::UNINSTALL_FAILED, error.to_string()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CliFailure::new(
            delivery::UNINSTALL_FAILED,
            format!(
                "refused to recursively remove unsafe path {}",
                path.display()
            ),
        ));
    }
    fs::remove_dir_all(path)
        .map_err(|error| CliFailure::new(delivery::UNINSTALL_FAILED, error.to_string()))
}

#[cfg(not(target_os = "windows"))]
fn remove_managed_reference(path: &Path) -> Result<(), CliFailure> {
    if !uninstall_path_allowed(path) || path == install_layout().root() {
        return Err(CliFailure::new(
            delivery::UNINSTALL_FAILED,
            format!("refused to remove unsafe reference {}", path.display()),
        ));
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::remove_file(path)
            .map_err(|error| CliFailure::new(delivery::UNINSTALL_FAILED, error.to_string())),
        Ok(_) => Err(CliFailure::new(
            delivery::UNINSTALL_FAILED,
            format!("refused to remove non-link reference {}", path.display()),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CliFailure::new(
            delivery::UNINSTALL_FAILED,
            error.to_string(),
        )),
    }
}

fn uninstall_path_allowed(path: &Path) -> bool {
    path.starts_with(install_layout().root())
}

fn load_selected_release(path: &Path) -> Result<SelectedRelease, CliFailure> {
    let encoded = fs::read(path).map_err(|error| {
        let descriptor = if error.kind() == io::ErrorKind::NotFound {
            delivery::RELEASE_NOT_SELECTED
        } else {
            delivery::FILESYSTEM_FAILED
        };
        CliFailure::new(descriptor, format!("{}: {error}", path.display()))
    })?;
    if encoded.len() > 16 * 1024 {
        return Err(CliFailure::new(
            delivery::RELEASE_INVALID,
            "selected release record is too large",
        ));
    }
    let selected: SelectedRelease = serde_json::from_slice(&encoded).map_err(|error| {
        CliFailure::new(
            delivery::RELEASE_INVALID,
            format!("selected release record is invalid: {error}"),
        )
    })?;
    if selected.schema != SELECTED_RELEASE_SCHEMA || !selected.release_root.is_absolute() {
        return Err(CliFailure::new(
            delivery::RELEASE_INVALID,
            "selected release record has an invalid schema or path",
        ));
    }
    let canonical = selected.release_root.canonicalize().map_err(|error| {
        CliFailure::new(
            delivery::RELEASE_INVALID,
            format!("selected release directory is unavailable: {error}"),
        )
    })?;
    if canonical != selected.release_root {
        return Err(CliFailure::new(
            delivery::RELEASE_INVALID,
            "selected release path is not canonical",
        ));
    }
    let release = verify_selected_release_at(&selected.release_root)?;
    let manifest_sha256 = sha256_file(&selected.release_root.join(RELEASE_MANIFEST_FILE))?;
    if release.claims().version != selected.version || manifest_sha256 != selected.manifest_sha256 {
        return Err(CliFailure::new(
            delivery::RELEASE_INVALID,
            "selected release metadata does not match the signed release",
        ));
    }
    Ok(selected)
}

fn preserved_control_state() -> Result<PreservedControlState, CliFailure> {
    let mut state =
        classify_preserved_control_state(|path| regular_file_without_symlink(Path::new(path)));
    if matches!(&state, PreservedControlState::Fresh) {
        return Ok(state);
    }

    let mut missing = match state {
        PreservedControlState::Incomplete { missing } => missing,
        PreservedControlState::Recoverable => Vec::new(),
        PreservedControlState::Fresh => unreachable!(),
    };
    if missing.is_empty() {
        if !regular_file_without_symlink(&install_layout().initialization_complete())
            && !regular_file_without_symlink(&install_layout().license_request())
        {
            missing.push(format!(
                "{} (or {})",
                install_layout().initialization_complete().display(),
                install_layout().license_request().display()
            ));
        }
        let role = read_trimmed(&install_layout().control_role())?;
        if role != "control" {
            return Err(invalid_preserved_installation(format!(
                "{} does not identify a Control installation",
                install_layout().control_role().display()
            )));
        }
        if regular_file_without_symlink(&install_layout().runner_role()) {
            return Err(invalid_preserved_installation(
                "a conflicting dedicated Runner role marker is present",
            ));
        }

        let access = load_access_configuration().map_err(|error| {
            invalid_preserved_installation(format!(
                "the preserved access configuration is invalid: {}",
                error.detail
            ))
        })?;
        if access.caddy_enabled {
            append_missing_regular_file(&mut missing, &install_layout().caddyfile());
            match access.certificate_source.as_str() {
                "caddy" => {
                    let caddy_ca_paths = [
                        install_layout().caddy_root_certificate(),
                        install_layout()
                            .caddy_data()
                            .join("caddy/pki/authorities/local/root.crt"),
                        install_layout()
                            .caddy_data()
                            .join("caddy/pki/authorities/local/root.key"),
                    ];
                    let present = caddy_ca_paths
                        .iter()
                        .filter(|path| regular_file_without_symlink(path))
                        .count();
                    if present != 0 && present != caddy_ca_paths.len() {
                        for path in caddy_ca_paths {
                            append_missing_regular_file(&mut missing, &path);
                        }
                    }
                }
                "provided" => {
                    append_missing_regular_file(
                        &mut missing,
                        &install_layout().caddy_server_certificate(),
                    );
                    append_missing_regular_file(
                        &mut missing,
                        &install_layout().caddy_server_private_key(),
                    );
                }
                _ => {}
            }
        }

        let runner_identity = regular_file_without_symlink(&install_layout().runner_identity());
        let runner_environment =
            regular_file_without_symlink(&install_layout().runner_environment());
        if runner_identity != runner_environment {
            if !runner_identity {
                missing.push(install_layout().runner_identity().display().to_string());
            }
            if !runner_environment {
                missing.push(install_layout().runner_environment().display().to_string());
            }
        }
    }
    state = if missing.is_empty() {
        PreservedControlState::Recoverable
    } else {
        PreservedControlState::Incomplete { missing }
    };
    Ok(state)
}

fn classify_preserved_control_state(mut exists: impl FnMut(&str) -> bool) -> PreservedControlState {
    let required = match preserved_control_required() {
        Ok(required) => required,
        Err(error) => {
            return PreservedControlState::Incomplete {
                missing: vec![error.to_string()],
            };
        }
    };
    let optional = preserved_control_optional_sentinels();
    let any_preserved = required
        .iter()
        .chain(optional.iter())
        .any(|path| exists(&path.to_string_lossy()));
    if !any_preserved {
        return PreservedControlState::Fresh;
    }
    let missing = required
        .iter()
        .filter(|path| !exists(&path.to_string_lossy()))
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    if missing.is_empty() {
        PreservedControlState::Recoverable
    } else {
        PreservedControlState::Incomplete { missing }
    }
}

fn regular_file_without_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
}

fn append_missing_regular_file(missing: &mut Vec<String>, path: &Path) {
    if !regular_file_without_symlink(path) {
        missing.push(path.display().to_string());
    }
}

fn invalid_preserved_installation(detail: impl Into<String>) -> CliFailure {
    CliFailure::new(
        delivery::INPUT_INVALID,
        format!(
            "preserved installation recovery was refused: {}; restore a complete backup or run `aster-team-cli uninstall --purge`",
            detail.into()
        ),
    )
}

fn load_access_configuration() -> Result<AccessConfiguration, CliFailure> {
    let path = install_layout().access_configuration();
    let metadata = fs::metadata(&path).map_err(|error| {
        CliFailure::new(
            delivery::DIAGNOSTIC_FAILED,
            format!("could not inspect {}: {error}", path.display()),
        )
    })?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 {
        return Err(CliFailure::new(
            delivery::DIAGNOSTIC_FAILED,
            format!("{} is not a bounded regular file", path.display()),
        ));
    }
    let source = fs::read(&path).map_err(|error| {
        CliFailure::new(
            delivery::DIAGNOSTIC_FAILED,
            format!("could not read {}: {error}", path.display()),
        )
    })?;
    let configuration: AccessConfiguration = serde_json::from_slice(&source).map_err(|error| {
        CliFailure::new(
            delivery::DIAGNOSTIC_FAILED,
            format!("{} is invalid: {error}", path.display()),
        )
    })?;
    let bind_address = configuration.bind_address.parse::<IpAddr>().map_err(|_| {
        CliFailure::new(
            delivery::DIAGNOSTIC_FAILED,
            "access configuration contains an invalid bind address",
        )
    })?;
    if configuration.schema != ACCESS_CONFIG_SCHEMA
        || !matches!(configuration.protocol.as_str(), "http" | "https")
        || !matches!(configuration.address_kind.as_str(), "ip" | "domain")
        || !matches!(
            configuration.certificate_source.as_str(),
            "none" | "caddy" | "provided"
        )
        || bind_address.is_unspecified()
        || bind_address.is_multicast()
    {
        return Err(CliFailure::new(
            delivery::DIAGNOSTIC_FAILED,
            "access configuration contains unsupported values",
        ));
    }
    let (expected_host, host_ip) = normalize_access_host(&configuration.host)
        .map_err(|error| CliFailure::new(delivery::DIAGNOSTIC_FAILED, error.detail))?;
    if expected_host != configuration.host
        || (configuration.address_kind == "ip") != host_ip.is_some()
        || (configuration.protocol == "http" && configuration.certificate_source != "none")
        || (configuration.protocol == "https" && configuration.certificate_source == "none")
        || !configuration.caddy_enabled
    {
        return Err(CliFailure::new(
            delivery::DIAGNOSTIC_FAILED,
            "access configuration fields are inconsistent",
        ));
    }
    let expected = expected_access_urls(
        &configuration.protocol,
        &configuration.address_kind,
        &configuration.host,
        instance_ports(&install_layout())?,
    );
    if configuration.member_url != expected.0
        || configuration.admin_url != expected.1
        || configuration.api_url != expected.2
        || configuration.runner_websocket_url != expected.3
    {
        return Err(CliFailure::new(
            delivery::DIAGNOSTIC_FAILED,
            "access configuration URLs do not match the selected protocol and address",
        ));
    }
    Ok(configuration)
}

fn expected_access_urls(
    protocol: &str,
    address_kind: &str,
    host: &str,
    ports: WindowsPorts,
) -> (String, String, String, String) {
    let (member, admin, api) = if address_kind == "domain" {
        let port = if protocol == "https" {
            ports.domain_https
        } else {
            ports.domain_http
        };
        let suffix = if (protocol == "https" && port == 443) || (protocol == "http" && port == 80) {
            String::new()
        } else {
            format!(":{port}")
        };
        (
            format!("{protocol}://app.{host}{suffix}"),
            format!("{protocol}://admin.{host}{suffix}"),
            format!("{protocol}://api.{host}{suffix}"),
        )
    } else {
        (
            format!("{protocol}://{host}:{}", ports.member),
            format!("{protocol}://{host}:{}", ports.admin),
            format!("{protocol}://{host}:{}", ports.api),
        )
    };
    let websocket = format!(
        "{}://{}/api/runner/channel",
        if protocol == "https" { "wss" } else { "ws" },
        api.split_once("://")
            .map_or(api.as_str(), |(_, value)| value)
    );
    (member, admin, api, websocket)
}

fn verify_release_at(root: &Path) -> Result<VerifiedRelease, CliFailure> {
    let trusted_keys = compiled_release_keys()?;
    verify_release_at_with_keys(root, &trusted_keys)
}

fn runner_release_files() -> &'static [&'static str] {
    #[cfg(target_os = "windows")]
    {
        &[
            "VERSION",
            "bin/aster-runner.exe",
            "bin/aster-team-cli.exe",
            "init.ps1",
            "libexec/install.ps1",
            "libexec/restore-backup.ps1",
            "windows/service-launch.ps1",
        ]
    }
    #[cfg(target_os = "linux")]
    {
        &[
            "VERSION",
            "bin/aster-runner",
            "bin/aster-team-cli",
            "init.sh",
            "libexec/install.sh",
            "libexec/restore-backup.sh",
            "libexec/service-health.sh",
            "systemd/aster-runner.service",
        ]
    }
    #[cfg(target_os = "macos")]
    {
        &[
            "VERSION",
            "bin/aster-runner",
            "bin/aster-team-cli",
            "init-macos.sh",
            "launchd/com.aster-team.runner.plist",
            "libexec/install-macos.sh",
            "libexec/restore-backup-macos.sh",
            "macos/service-launch.sh",
        ]
    }
}

fn verify_runner_release_at(root: &Path) -> Result<VerifiedRelease, CliFailure> {
    let trusted_keys = compiled_release_keys()?;
    let document = fs::read(root.join(RELEASE_MANIFEST_FILE)).map_err(|error| {
        CliFailure::new(
            delivery::RELEASE_INVALID,
            format!("could not read RELEASE.json: {error}"),
        )
    })?;
    let release = verify(&document, &trusted_keys)
        .map_err(|error| CliFailure::new(delivery::RELEASE_INVALID, error.to_string()))?;
    verify_release_subset(root, &release, runner_release_files().iter().copied())
        .map_err(|error| CliFailure::new(delivery::RELEASE_INVALID, error.to_string()))?;
    verify_release_platform(&release)?;
    Ok(release)
}

fn verify_selected_release_at(root: &Path) -> Result<VerifiedRelease, CliFailure> {
    verify_release_at(root).or_else(|_| verify_runner_release_at(root))
}

fn verify_release_at_with_keys(
    root: &Path,
    trusted_keys: &TrustedReleaseKeys,
) -> Result<VerifiedRelease, CliFailure> {
    let document = fs::read(root.join(RELEASE_MANIFEST_FILE)).map_err(|error| {
        CliFailure::new(
            delivery::RELEASE_INVALID,
            format!("could not read RELEASE.json: {error}"),
        )
    })?;
    let release = verify(&document, trusted_keys)
        .map_err(|error| CliFailure::new(delivery::RELEASE_INVALID, error.to_string()))?;
    verify_release_tree(root, &release)
        .map_err(|error| CliFailure::new(delivery::RELEASE_INVALID, error.to_string()))?;
    verify_release_platform(&release)?;
    Ok(release)
}

fn verify_release_platform(release: &VerifiedRelease) -> Result<(), CliFailure> {
    let expected_architecture = if cfg!(target_arch = "x86_64") {
        "amd64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        return Err(CliFailure::new(
            delivery::PLATFORM_UNSUPPORTED,
            "the current CPU architecture is unsupported",
        ));
    };
    let expected_runtime = match Platform::current() {
        Platform::Linux => "musl-static",
        Platform::Windows => "msvc",
        Platform::Macos => "native",
    };
    if release.claims().platform != Platform::current().id()
        || release.claims().architecture != expected_architecture
        || release.claims().runtime != expected_runtime
    {
        return Err(CliFailure::new(
            delivery::PLATFORM_UNSUPPORTED,
            format!(
                "release targets {}/{}/{} but this host requires {}/{}/{}",
                release.claims().platform,
                release.claims().architecture,
                release.claims().runtime,
                Platform::current().id(),
                expected_architecture,
                expected_runtime
            ),
        ));
    }
    Ok(())
}

fn compiled_release_keys() -> Result<TrustedReleaseKeys, CliFailure> {
    let entries: Vec<CompiledReleaseKey> =
        serde_json::from_str(compiled_keys::COMPILED_RELEASE_KEYS_JSON).map_err(|error| {
            CliFailure::new(
                delivery::RELEASE_INVALID,
                format!("compiled Release keyring is invalid: {error}"),
            )
        })?;
    if entries.is_empty() {
        return Err(CliFailure::new(
            delivery::RELEASE_INVALID,
            "compiled Release keyring is empty",
        ));
    }
    let mut keys = TrustedReleaseKeys::new();
    for entry in entries {
        keys.insert_spki_base64url(entry.key_id, &entry.public_key_spki)
            .map_err(|error| CliFailure::new(delivery::RELEASE_INVALID, error.to_string()))?;
    }
    Ok(keys)
}

fn require_root() -> Result<(), CliFailure> {
    #[cfg(target_os = "linux")]
    {
        let status = fs::read_to_string("/proc/self/status").map_err(|error| {
            CliFailure::new(
                delivery::ROOT_REQUIRED,
                format!("could not determine the effective uid: {error}"),
            )
        })?;
        let effective_uid = status
            .lines()
            .find(|line| line.starts_with("Uid:"))
            .and_then(|line| line.split_whitespace().nth(2))
            .and_then(|value| value.parse::<u32>().ok())
            .ok_or_else(|| {
                CliFailure::new(delivery::ROOT_REQUIRED, "effective uid is unavailable")
            })?;
        if effective_uid != 0 {
            return Err(CliFailure::new(
                delivery::ROOT_REQUIRED,
                "rerun the command with sudo",
            ));
        }
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        let output = ProcessCommand::new("whoami.exe")
            .args(["/groups", "/fo", "csv", "/nh"])
            .output()
            .map_err(|error| {
                CliFailure::new(
                    delivery::ROOT_REQUIRED,
                    format!("could not determine Windows elevation: {error}"),
                )
            })?;
        let groups = String::from_utf8_lossy(&output.stdout);
        if !output.status.success()
            || !(groups.contains("S-1-16-12288") || groups.contains("S-1-16-16384"))
        {
            return Err(CliFailure::new(
                delivery::ROOT_REQUIRED,
                "rerun the command from an elevated Administrator terminal",
            ));
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        let output = ProcessCommand::new("id")
            .arg("-u")
            .output()
            .map_err(|error| {
                CliFailure::new(
                    delivery::ROOT_REQUIRED,
                    format!("could not determine the effective uid: {error}"),
                )
            })?;
        if !output.status.success() || String::from_utf8_lossy(&output.stdout).trim() != "0" {
            return Err(CliFailure::new(
                delivery::ROOT_REQUIRED,
                "rerun the command with sudo",
            ));
        }
        Ok(())
    }
}

fn open_maintenance_lock() -> Result<File, CliFailure> {
    open_lock_file(&install_layout().maintenance_lock())
}

fn open_lock_file(path: &Path) -> Result<File, CliFailure> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    }
    OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))
}

fn atomic_write_new_file(path: &Path, value: &[u8], mode: u32) -> Result<(), CliFailure> {
    let parent = path.parent().ok_or_else(|| {
        CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            "file path has no parent directory",
        )
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    let temporary = parent.join(format!(".aster-team.{}.tmp", std::process::id()));
    let result = (|| -> Result<(), CliFailure> {
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
        set_mode(&temporary, mode)?;
        output
            .write_all(value)
            .and_then(|()| output.sync_all())
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
        fs::rename(&temporary, path)
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
        sync_parent(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn atomic_copy_executable(source: &Path, target: &Path) -> Result<(), CliFailure> {
    let parent = target.parent().ok_or_else(|| {
        CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            "CLI target has no parent directory",
        )
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    let temporary = parent.join(format!(".aster-team-cli.{}.tmp", std::process::id()));
    let result = (|| -> Result<(), CliFailure> {
        let mut input = File::open(source)
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
        io::copy(&mut input, &mut output)
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
        set_mode(&temporary, 0o755)?;
        output
            .sync_all()
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
        replace_temporary_file(&temporary, target)?;
        sync_parent(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(unix)]
fn install_command_link(layout: &InstallLayout, target: &Path) -> Result<(), CliFailure> {
    use std::os::unix::fs::symlink;

    let Some(link) = layout
        .command_link()
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?
    else {
        return Ok(());
    };
    let parent = link.parent().ok_or_else(|| {
        CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            "CLI command link has no parent directory",
        )
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    if let Ok(metadata) = fs::symlink_metadata(&link)
        && !metadata.file_type().is_symlink()
    {
        return Err(CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            format!(
                "refused to replace non-link command entry {}",
                link.display()
            ),
        ));
    }
    let temporary = parent.join(format!(".aster-team-cli-link.{}.tmp", std::process::id()));
    let _ = fs::remove_file(&temporary);
    symlink(target, &temporary)
        .and_then(|()| fs::rename(&temporary, &link))
        .map_err(|error| {
            let _ = fs::remove_file(&temporary);
            CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string())
        })?;
    sync_parent(parent)
}

#[cfg(not(unix))]
fn install_command_link(_layout: &InstallLayout, _target: &Path) -> Result<(), CliFailure> {
    Ok(())
}

#[cfg(unix)]
fn remove_command_link(layout: &InstallLayout) -> Result<(), CliFailure> {
    let Some(link) = layout
        .command_link()
        .map_err(|error| CliFailure::new(delivery::UNINSTALL_FAILED, error.to_string()))?
    else {
        return Ok(());
    };
    match fs::symlink_metadata(&link) {
        Ok(metadata)
            if metadata.file_type().is_symlink()
                && fs::read_link(&link).ok().as_deref() == Some(layout.stable_cli().as_path()) =>
        {
            fs::remove_file(&link)
                .map_err(|error| CliFailure::new(delivery::UNINSTALL_FAILED, error.to_string()))?;
            if let Some(parent) = link.parent() {
                sync_parent(parent)?;
            }
            Ok(())
        }
        Ok(_) => Err(CliFailure::new(
            delivery::UNINSTALL_FAILED,
            format!("refused to remove unowned command entry {}", link.display()),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CliFailure::new(
            delivery::UNINSTALL_FAILED,
            error.to_string(),
        )),
    }
}

#[cfg(not(unix))]
fn remove_command_link(_layout: &InstallLayout) -> Result<(), CliFailure> {
    Ok(())
}

fn atomic_write_selection(path: &Path, value: &SelectedRelease) -> Result<(), CliFailure> {
    let parent = path.parent().ok_or_else(|| {
        CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            "selected release path has no parent directory",
        )
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    set_mode(parent, 0o700)?;
    let mut encoded = serde_json::to_vec_pretty(value)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    encoded.push(b'\n');
    let temporary = parent.join(format!(".selected-release.{}.tmp", std::process::id()));
    let result = (|| -> Result<(), CliFailure> {
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
        set_mode(&temporary, 0o600)?;
        output
            .write_all(&encoded)
            .and_then(|()| output.sync_all())
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
        replace_temporary_file(&temporary, path)?;
        sync_parent(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> Result<(), CliFailure> {
    File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) -> Result<(), CliFailure> {
    Ok(())
}

#[cfg(unix)]
fn replace_temporary_file(temporary: &Path, target: &Path) -> Result<(), CliFailure> {
    fs::rename(temporary, target)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))
}

#[cfg(target_os = "windows")]
fn replace_temporary_file(temporary: &Path, target: &Path) -> Result<(), CliFailure> {
    let parent = target.parent().ok_or_else(|| {
        CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            "replacement target has no parent directory",
        )
    })?;
    let previous = parent.join(format!(
        ".aster-team.{}.{}.previous",
        std::process::id(),
        time::OffsetDateTime::now_utc().unix_timestamp_nanos()
    ));
    let had_previous = match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            fs::rename(target, &previous)
                .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
            true
        }
        Ok(_) => {
            return Err(CliFailure::new(
                delivery::FILESYSTEM_FAILED,
                format!("refused to replace unsafe path {}", target.display()),
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(CliFailure::new(
                delivery::FILESYSTEM_FAILED,
                error.to_string(),
            ));
        }
    };
    if let Err(error) = fs::rename(temporary, target) {
        if had_previous {
            let _ = fs::rename(&previous, target);
        }
        return Err(CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            error.to_string(),
        ));
    }
    if had_previous {
        fs::remove_file(previous)
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), CliFailure> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<(), CliFailure> {
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, CliFailure> {
    let mut source = File::open(path)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = source
            .read(&mut buffer)
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(lowercase_hex(&digest.finalize()))
}

fn lowercase_hex(value: &[u8]) -> String {
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value {
        use fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

fn validate_email(value: &str) -> Result<(), CliFailure> {
    let (local, domain) = value
        .split_once('@')
        .ok_or_else(|| CliFailure::new(delivery::INPUT_INVALID, "owner email is invalid"))?;
    if local.is_empty()
        || !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
        || value.chars().any(char::is_whitespace)
    {
        return Err(CliFailure::new(
            delivery::INPUT_INVALID,
            "owner email is invalid",
        ));
    }
    Ok(())
}

fn validate_secret_file(path: &Path) -> Result<(), CliFailure> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        CliFailure::new(
            delivery::INPUT_INVALID,
            format!("password file is unavailable: {error}"),
        )
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(CliFailure::new(
            delivery::INPUT_INVALID,
            "password file must be a regular file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.uid() != 0 || metadata.mode() & 0o077 != 0 {
            return Err(CliFailure::new(
                delivery::INPUT_INVALID,
                "password file must be owned by root and must not be accessible by group or others",
            ));
        }
    }
    Ok(())
}

fn read_line(prompt: &str) -> Result<String, CliFailure> {
    print!("{prompt}");
    io::stdout()
        .flush()
        .map_err(|error| CliFailure::new(delivery::COMMAND_FAILED, error.to_string()))?;
    let mut value = String::new();
    io::stdin()
        .read_line(&mut value)
        .map_err(|error| CliFailure::new(delivery::COMMAND_FAILED, error.to_string()))?;
    Ok(value.trim().to_owned())
}

fn read_yes_no(prompt: &str, default: bool) -> Result<bool, CliFailure> {
    match read_line(prompt)?.to_ascii_lowercase().as_str() {
        "" => Ok(default),
        "y" | "yes" => Ok(true),
        "n" | "no" => Ok(false),
        _ => Err(CliFailure::new(
            delivery::INPUT_INVALID,
            "answer must be yes or no",
        )),
    }
}

fn generate_initial_password() -> Result<Zeroizing<String>, CliFailure> {
    let mut random = [0_u8; 24];
    getrandom::fill(&mut random).map_err(|_| {
        CliFailure::new(
            delivery::FILESYSTEM_FAILED,
            "operating-system randomness is unavailable",
        )
    })?;
    Ok(Zeroizing::new(format!(
        "{}-Aa1!",
        URL_SAFE_NO_PAD.encode(random)
    )))
}

fn print_initial_credentials(email: &str, password: &str) {
    println!();
    println!("============================================================");
    println!("  INITIAL ADMIN CREDENTIALS — SAVE THESE NOW");
    println!("  Email:              {email}");
    println!("  Temporary password: {password}");
    println!(
        "  Credential file:    {}",
        install_layout().initial_owner_credentials().display()
    );
    println!("  You must change this password at the first login.");
    println!("============================================================");
}

struct TemporarySecret {
    path: PathBuf,
}

impl TemporarySecret {
    fn new(value: &[u8]) -> Result<Self, CliFailure> {
        let path = temporary_runtime_file("owner-password")?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
        set_mode(&path, 0o600)?;
        file.write_all(value)
            .and_then(|()| file.write_all(b"\n"))
            .and_then(|()| file.sync_all())
            .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
        Ok(Self { path })
    }
}

fn temporary_runtime_file(label: &str) -> Result<PathBuf, CliFailure> {
    let runtime = install_layout().cli_private();
    fs::create_dir_all(&runtime)
        .map_err(|error| CliFailure::new(delivery::FILESYSTEM_FAILED, error.to_string()))?;
    Ok(runtime.join(format!(
        ".aster-team-{label}.{}.{}.tmp",
        std::process::id(),
        time::OffsetDateTime::now_utc().unix_timestamp_nanos()
    )))
}

impl Drop for TemporarySecret {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn command_failure(descriptor: ErrorDescriptor, command: &str, status: ExitStatus) -> CliFailure {
    CliFailure::new(
        descriptor,
        format!(
            "{command} exited with {}",
            status
                .code()
                .map_or_else(|| "a signal".to_owned(), |code| format!("status {code}"))
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_license_completion_distinguishes_missing_and_installed_files() {
        let directory = tempfile::tempdir().expect("temporary license directory");
        let missing = directory.path().join("missing.json");
        assert!(!initial_license_is_installed(&missing).expect("missing license"));

        let installed = directory.path().join("license.json");
        fs::write(&installed, b"signed-license-fixture").expect("write license fixture");
        assert!(initial_license_is_installed(&installed).expect("installed license"));
        assert!(initial_license_is_installed(directory.path()).is_err());
    }

    #[test]
    fn paid_license_install_restores_every_transaction_state_owner() {
        let layout = InstallLayout::platform_default().expect("test install layout");
        let state = layout.license_state();
        let paths = license_state_ownership_paths(&layout);
        assert_eq!(paths.len(), 6);
        assert_eq!(paths[0], state);
        assert_eq!(
            paths[1],
            PathBuf::from(format!("{}.pending", state.display()))
        );
        assert_eq!(paths[2], PathBuf::from(format!("{}.lock", state.display())));
        assert_eq!(
            paths[3],
            PathBuf::from(format!("{}.staged", state.display()))
        );
        assert_eq!(
            paths[4],
            PathBuf::from(format!("{}.activation", state.display()))
        );
        assert_eq!(
            paths[5],
            PathBuf::from(format!("{}.mutation", state.display()))
        );
    }

    #[test]
    fn license_status_fields_use_only_v2_claims() {
        let v1 = serde_json::json!({
            "license_id": "license-v1",
            "limits": { "member_seats": 20 },
            "expires_at": "2027-01-01T00:00:00.000Z"
        });
        let v1_claims = license_claims(&v1);
        assert_eq!(license_quota_display(v1_claims, "member_seats"), "-");
        assert_eq!(license_expiry_display(v1_claims), "-");

        let v2 = serde_json::json!({
            "claims": {
                "license_id": "license-v2",
                "entitlements": {
                    "quotas": [
                        { "id": "member_seats", "limit": { "mode": "limited", "value": 3 } },
                        { "id": "runners", "limit": { "mode": "unlimited" } }
                    ]
                },
                "validity": { "expiry": { "mode": "none" } }
            },
            "signature": "test"
        });
        let v2_claims = license_claims(&v2);
        assert_eq!(v2_claims["license_id"], "license-v2");
        assert_eq!(license_quota_display(v2_claims, "member_seats"), "3");
        assert_eq!(license_quota_display(v2_claims, "runners"), "unlimited");
        assert_eq!(license_quota_display(v2_claims, "unknown"), "-");
        assert_eq!(license_expiry_display(v2_claims), "never");
    }

    #[test]
    fn trace_accepts_generated_and_client_request_ids() {
        for value in ["request_12345678", "diag-20260904182907-fast"] {
            assert!(valid_trace_request_id(value));
        }
        for value in ["short", "request id", "request/unsafe"] {
            assert!(!valid_trace_request_id(value));
        }

        let cli = Cli::try_parse_from([
            "aster-team-cli",
            "trace",
            "diag-20260904182907-fast",
            "--hours",
            "48",
        ])
        .expect("parse request trace");
        let Command::Trace(arguments) = cli.command else {
            panic!("expected trace command");
        };
        assert_eq!(
            arguments.request_id.as_deref(),
            Some("diag-20260904182907-fast")
        );
        assert_eq!(arguments.hours, 48);
    }

    #[test]
    fn admin_password_reset_accepts_optional_email() {
        let cli = Cli::try_parse_from([
            "aster-team-cli",
            "password",
            "reset-admin",
            "--email",
            "admin@example.com",
        ])
        .expect("parse administrator password reset");
        let Command::Password {
            command: PasswordCommand::ResetAdmin { email },
        } = cli.command
        else {
            panic!("expected password reset command");
        };
        assert_eq!(email.as_deref(), Some("admin@example.com"));
    }

    #[test]
    fn version_banner_includes_short_commit_and_utc_build_time() {
        let banner = version_banner();
        assert!(banner.starts_with(&format!("aster-team-cli {}\n", env!("CARGO_PKG_VERSION"))));
        assert!(banner.contains(&format!("Commit:     {}", build_info::BUILD_COMMIT)));
        assert!(banner.contains(&format!("Build time: {}", build_info::BUILD_TIMESTAMP)));
        assert!(
            build_info::BUILD_COMMIT == "unknown"
                || ((7..=12).contains(&build_info::BUILD_COMMIT.len())
                    && build_info::BUILD_COMMIT
                        .bytes()
                        .all(|value| value.is_ascii_hexdigit()))
        );
        let timestamp = time::OffsetDateTime::parse(
            build_info::BUILD_TIMESTAMP,
            &time::format_description::well_known::Rfc3339,
        )
        .expect("build timestamp is RFC 3339");
        assert_eq!(timestamp.offset(), time::UtcOffset::UTC);
    }

    #[test]
    fn public_command_contract_parses() {
        for arguments in [
            vec!["aster-team-cli", "version"],
            vec!["aster-team-cli", "install"],
            vec!["aster-team-cli", "install", "--recover-preserved"],
            vec![
                "aster-team-cli",
                "install",
                "--unattended",
                "--recover-preserved",
            ],
            vec!["aster-team-cli", "upgrade"],
            vec!["aster-team-cli", "service", "restart", "all"],
            vec!["aster-team-cli", "logs", "control", "--follow"],
            vec!["aster-team-cli", "license", "status"],
            vec!["aster-team-cli", "license", "request"],
            vec!["aster-team-cli", "backup", "create"],
            vec![
                "aster-team-cli",
                "license",
                "request",
                "--output",
                "/root/license-request.json",
            ],
            vec![
                "aster-team-cli",
                "backup",
                "restore",
                "--source",
                "/root/backup.tar.gz",
                "--confirm",
            ],
            vec!["aster-team-cli", "runner", "install"],
            vec![
                "aster-team-cli",
                "runner",
                "enroll",
                "--control-url",
                "https://team.example.com",
                "--token-file",
                "/root/runner.token",
            ],
            vec!["aster-team-cli", "runner", "status"],
            vec!["aster-team-cli", "runner", "upgrade"],
            vec!["aster-team-cli", "runner", "backup", "create"],
            vec!["aster-team-cli", "uninstall", "--purge"],
        ] {
            Cli::try_parse_from(arguments).expect("public command must parse");
        }
    }

    #[test]
    fn license_request_defaults_to_the_current_directory_with_a_timestamp() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let output = resolve_license_request_output_at(
            None,
            directory.path(),
            time::macros::datetime!(2026-08-29 13:32:20 UTC),
        )
        .expect("default request path");
        assert_eq!(
            output.path,
            directory
                .path()
                .join("aster-team-license-request-20260829T133220Z.json")
        );
        assert_eq!(output.redirected_from_release, None);
    }

    #[test]
    fn license_request_never_writes_into_a_signed_release_tree() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let release = directory.path().join("aster-team-2.0.0-linux-amd64");
        fs::create_dir_all(release.join("bin")).expect("release directories");
        for path in [
            release.join(RELEASE_MANIFEST_FILE),
            release.join("VERSION"),
            release.join("bin").join(executable_name("aster-team-cli")),
        ] {
            fs::write(path, b"fixture").expect("release marker");
        }
        let nested = release.join("nested");
        fs::create_dir(&nested).expect("nested directory");
        let output = resolve_license_request_output_at(
            None,
            &nested,
            time::macros::datetime!(2026-08-29 13:32:20 UTC),
        )
        .expect("redirected request path");
        assert_eq!(output.path.parent(), release.parent());
        assert_eq!(
            output.redirected_from_release.as_deref(),
            Some(release.as_path())
        );
        assert!(
            resolve_license_request_output_at(
                Some(Path::new("a.request")),
                &nested,
                time::macros::datetime!(2026-08-29 13:32:20 UTC),
            )
            .is_err()
        );
    }

    #[test]
    fn renders_a_compact_license_request_as_png_and_terminal_qr() {
        let encoded = br#"{
          "schema":"aster.license-request.v2",
          "request_id":"request_0uTL4YW_I-t6LFy8KTd9J891",
          "product":"aster-team",
          "product_version":"2.0.0",
          "platform":"linux",
          "architecture":"amd64",
          "installation_id":"installation_Dmz9pQ4AVs6kzaRgM79_8JDJ",
          "machine_fingerprint_sha256":"UJn2n58Eq5N9_d_VDYJbTKdRpvH9fCQReCSPXs8iBx8",
          "machine_factors":[
            {"kind":"dmi_product_uuid","sha256":"u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7s"},
            {"kind":"machine_id","sha256":"zMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMw"}
          ],
          "generated_at":"2026-08-29T13:32:20.000Z",
          "license_schema":"aster.license.v2",
          "capability_catalog_version":1,
          "quota_policy_version":1
        }"#;
        let payload = compact_license_request_qr_payload(encoded).expect("compact request");
        let vector: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../contracts/test-vectors/license-request.v2.json"
        ))
        .expect("parse shared request fixture");
        assert_eq!(
            URL_SAFE_NO_PAD.encode(&payload),
            vector["compact_qr_v3"]["payload_base64url"]
                .as_str()
                .expect("shared compact payload")
        );
        let original = std::str::from_utf8(encoded).unwrap();
        for changed in [
            original.replace("aster.license-request.v2", "aster.license-request.v1"),
            original.replace(
                "\"license_schema\":\"aster.license.v2\"",
                "\"license_schema\":\"aster.license.v1\"",
            ),
            original.replace(
                "\"request_id\":",
                "\"request_id\":\"replacement\",\"request_id\":",
            ),
        ] {
            assert!(compact_license_request_qr_payload(changed.as_bytes()).is_err());
        }
        assert_eq!(payload.len(), 128);
        assert_eq!(&payload[..4], &LICENSE_REQUEST_QR_MAGIC);
        assert_eq!(payload[4], 1);
        assert_eq!(payload[5], 1);
        assert_eq!(payload[6], 5);
        assert_eq!(u32::from_be_bytes(payload[7..11].try_into().unwrap()), 1);
        assert_eq!(u32::from_be_bytes(payload[11..15].try_into().unwrap()), 1);
        assert_eq!(
            URL_SAFE_NO_PAD.encode(&payload[23..41]),
            "0uTL4YW_I-t6LFy8KTd9J891"
        );
        assert_eq!(
            URL_SAFE_NO_PAD.encode(&payload[41..59]),
            "Dmz9pQ4AVs6kzaRgM79_8JDJ"
        );
        assert_eq!(&payload[123..], b"2.0.0");
        assert!(payload.len() * 2 < encoded.len());

        let code = license_request_qr(&payload).expect("standard request QR code");
        assert_eq!(code.width(), 49);
        let directory = tempfile::tempdir().expect("temporary QR directory");
        let png = directory.path().join("request.qr.png");
        code.render::<Luma<u8>>()
            .min_dimensions(1024, 1024)
            .quiet_zone(true)
            .build()
            .save(&png)
            .expect("write request QR PNG");
        assert!(
            fs::read(&png)
                .expect("read request QR PNG")
                .starts_with(b"\x89PNG\r\n\x1a\n")
        );
        let original_document: serde_json::Value =
            serde_json::from_slice(encoded).expect("request JSON");
        let original_payload = serde_json::to_vec(&original_document).expect("original QR payload");
        let original_code = QrCode::with_error_correction_level(&original_payload, EcLevel::M)
            .expect("original request QR code");
        assert!(code.width() < original_code.width());
        let terminal = render_terminal_qr(&code);
        assert_eq!(terminal.lines().count(), code.width() + 2);
        assert!(terminal.contains("\x1b[0;30;47m"));
        assert!(terminal.contains("\x1b[0;30;40m"));
        assert!(!terminal.contains('▀'));
    }

    #[test]
    fn purge_targets_and_recursive_delete_allowlist_cannot_drift() {
        let root = install_layout().root().to_path_buf();
        assert!(uninstall_path_allowed(&root));
        assert!(uninstall_path_allowed(&install_layout().releases()));
        assert!(uninstall_path_allowed(&install_layout().data()));
        assert!(!uninstall_path_allowed(root.parent().expect("root parent")));
        assert!(!uninstall_path_allowed(
            &root.parent().expect("root parent").join("aster-team-extra")
        ));
    }

    #[test]
    fn preserved_installation_state_requires_the_complete_control_identity_and_database() {
        let complete = preserved_control_required()
            .unwrap()
            .into_iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            classify_preserved_control_state(|path| complete.iter().any(|item| item == path)),
            PreservedControlState::Recoverable
        );
        assert_eq!(
            classify_preserved_control_state(|_| false),
            PreservedControlState::Fresh
        );
        let database = install_layout().database_file().display().to_string();
        let incomplete = classify_preserved_control_state(|path| path == database);
        let PreservedControlState::Incomplete { missing } = incomplete else {
            panic!("a database without its identity and keys must not be recoverable");
        };
        assert!(missing.contains(&install_layout().database_key().display().to_string()));
        assert!(
            missing.contains(
                &install_layout()
                    .installation_profile()
                    .display()
                    .to_string()
            )
        );
    }

    #[test]
    fn unattended_install_requires_the_mode_switch() {
        assert!(
            Cli::try_parse_from([
                "aster-team-cli",
                "install",
                "--owner-email",
                "owner@example.com"
            ])
            .is_err()
        );
    }

    #[test]
    fn validates_owner_email_without_accepting_whitespace() {
        validate_email("owner@example.com").expect("valid email");
        assert!(validate_email("owner @example.com").is_err());
        assert!(validate_email("owner@example").is_err());
    }

    #[test]
    fn derives_only_secure_or_loopback_runner_channels() {
        assert_eq!(
            runner_wss_url("https://team.example.com", false).expect("valid Control origin"),
            "wss://team.example.com/api/runner/channel"
        );
        assert_eq!(
            runner_wss_url("http://127.0.0.1:11080", false).expect("valid loopback origin"),
            "ws://127.0.0.1:11080/api/runner/channel"
        );
        for invalid in [
            "http://team.example.com",
            "http://192.168.1.10:11080",
            "https://user@team.example.com",
            "https://team.example.com/path",
            "https://team.example.com/?query=1",
        ] {
            assert!(runner_wss_url(invalid, false).is_err());
        }
        assert_eq!(
            runner_wss_url("http://192.168.1.10:11080", true)
                .expect("explicitly allowed LAN Control origin"),
            "ws://192.168.1.10:11080/api/runner/channel"
        );
    }

    #[test]
    fn access_urls_use_ports_for_ip_and_subdomains_without_ports_for_domains() {
        assert_eq!(
            expected_access_urls("http", "ip", "10.13.74.140", WindowsPorts::default()),
            (
                "http://10.13.74.140:11081".to_owned(),
                "http://10.13.74.140:11082".to_owned(),
                "http://10.13.74.140:11080".to_owned(),
                "ws://10.13.74.140:11080/api/runner/channel".to_owned(),
            )
        );
        assert_eq!(
            expected_access_urls("http", "domain", "inner-aster.com", WindowsPorts::default()),
            (
                "http://app.inner-aster.com".to_owned(),
                "http://admin.inner-aster.com".to_owned(),
                "http://api.inner-aster.com".to_owned(),
                "ws://api.inner-aster.com/api/runner/channel".to_owned(),
            )
        );
        assert_eq!(
            expected_access_urls(
                "https",
                "domain",
                "inner-aster.com",
                WindowsPorts::default()
            ),
            (
                "https://app.inner-aster.com".to_owned(),
                "https://admin.inner-aster.com".to_owned(),
                "https://api.inner-aster.com".to_owned(),
                "wss://api.inner-aster.com/api/runner/channel".to_owned(),
            )
        );
    }

    #[test]
    fn delivery_error_numbers_use_the_reserved_module() {
        for descriptor in [
            delivery::ROOT_REQUIRED,
            delivery::RELEASE_NOT_SELECTED,
            delivery::RELEASE_INVALID,
            delivery::MAINTENANCE_BUSY,
            delivery::INSTALL_FAILED,
            delivery::UPGRADE_FAILED,
            delivery::INPUT_INVALID,
            delivery::FILESYSTEM_FAILED,
            delivery::COMMAND_FAILED,
            delivery::SERVICE_FAILED,
            delivery::LICENSE_FAILED,
            delivery::BACKUP_FAILED,
            delivery::RUNNER_FAILED,
            delivery::DIAGNOSTIC_FAILED,
            delivery::UNINSTALL_FAILED,
        ] {
            assert!((43_001..=43_999).contains(&descriptor.number));
        }
    }

    #[test]
    fn custom_instance_urls_include_public_and_domain_ports() {
        let ports = aster_install_layout::WindowsInstance::from_environment(|name| {
            (name == "ASTER_PORT_OFFSET").then(|| "10000".into())
        })
        .unwrap()
        .ports;
        let (member, admin, api, runner) = expected_access_urls("http", "ip", "127.0.0.1", ports);
        assert_eq!(member, "http://127.0.0.1:21081");
        assert_eq!(admin, "http://127.0.0.1:21082");
        assert_eq!(api, "http://127.0.0.1:21080");
        assert_eq!(runner, "ws://127.0.0.1:21080/api/runner/channel");
        let (_, _, api, runner) =
            expected_access_urls("https", "domain", "team.example.com", ports);
        assert_eq!(api, "https://api.team.example.com:10443");
        assert_eq!(
            runner,
            "wss://api.team.example.com:10443/api/runner/channel"
        );
        let (_, _, api, _) = expected_access_urls("http", "domain", "team.example.com", ports);
        assert_eq!(api, "http://api.team.example.com:10080");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn fresh_instance_can_retry_but_control_and_runner_ignore_later_environment() {
        let temp = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(temp.path()).unwrap();
        fs::write(layout.marker_path(), layout.marker_json().unwrap()).unwrap();
        let first = resolve_windows_instance(&layout, true, |name| {
            (name == "ASTER_SERVICE_PREFIX").then(|| "first".into())
        })
        .unwrap();
        assert_eq!(first.service_prefix, "first");
        let readonly =
            resolve_windows_instance(&layout, false, |_| Some("invalid".into())).unwrap();
        assert_eq!(readonly, first);
        let retried = resolve_windows_instance(&layout, true, |name| {
            (name == "ASTER_SERVICE_PREFIX").then(|| "retry".into())
        })
        .unwrap();
        assert_eq!(retried.service_prefix, "retry");
        for role in [layout.control_role(), layout.runner_role()] {
            fs::create_dir_all(role.parent().unwrap()).unwrap();
            fs::write(&role, b"installed").unwrap();
            let retained = resolve_windows_instance(&layout, true, |_| {
                panic!("installed instance read terminal environment")
            })
            .unwrap();
            assert_eq!(retained, retried);
            fs::remove_file(role).unwrap();
        }
        let default = resolve_windows_instance(&layout, true, |_| None).unwrap();
        assert_eq!(default, aster_install_layout::WindowsInstance::default());
        assert!(layout.read_marker().unwrap().windows_instance.is_none());
    }
}

#[cfg(test)]
mod linux_service_state_tests {
    use super::linux_service_state;

    #[test]
    fn missing_optional_units_are_skipped_but_manager_failures_are_not() {
        for success in [true, false] {
            assert_eq!(
                linux_service_state("LoadState=not-found\nActiveState=inactive\n", success)
                    .unwrap(),
                "not-installed"
            );
        }
        for (properties, success) in [
            ("", false),
            ("LoadState=loaded\nActiveState=inactive", false),
            ("LoadState=loaded", true),
            (
                "LoadState=loaded\nActiveState=inactive\nActiveState=active",
                true,
            ),
            ("LoadState=error\nActiveState=inactive", true),
        ] {
            assert!(linux_service_state(properties, success).is_err());
        }
    }

    #[test]
    fn a_running_unit_is_never_classified_as_uninstalled() {
        for load in ["loaded", "not-found", "masked"] {
            for active in ["active", "reloading", "activating", "deactivating"] {
                assert_eq!(
                    linux_service_state(&format!("LoadState={load}\nActiveState={active}\n"), true)
                        .unwrap(),
                    active
                );
            }
        }
        assert_eq!(
            linux_service_state("ActiveState=inactive\nLoadState=loaded\n", true).unwrap(),
            "inactive"
        );
    }
}
