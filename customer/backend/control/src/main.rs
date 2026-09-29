#![forbid(unsafe_code)]

mod local_runner_slot;

use std::{
    future::Future,
    io::{self, Read as _, Write as _},
    net::SocketAddr,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    time::Duration,
};

#[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
use std::sync::Mutex;

use aster_auth_core::AuthCore;
use aster_control::{
    ControlState, ControlStorage, RunnerTaskIssuer, compiled_license_keys, compiled_release_keys,
    generate_bootstrap_local_runner_identity, install_or_stage_license,
    load_installed_license_with_staged_activation, router, verify_bundled_free_license, web_router,
};
use aster_credential_vault::CredentialVault;
use aster_install_layout::{DatabaseConfiguration, InstallLayout, Platform};
use aster_license_core::{
    PRODUCT,
    catalog::CATALOG_VERSION,
    request_v2::{Request as LicenseRequest, SCHEMA as LICENSE_REQUEST_SCHEMA},
    v2::{QUOTA_POLICY_VERSION, SCHEMA as LICENSE_SCHEMA},
};
use aster_license_state::LicenseStateStore;
use aster_machine_identity::{
    InstallationProfile, MachineIdentityError, RawMachineFactors, create_profile,
    parse_and_verify_profile, parse_profile,
};
use aster_release_core::{RELEASE_MANIFEST_FILE, verify as verify_release, verify_release_tree};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use clap::{Args, Parser, Subcommand, ValueEnum};
use ed25519_dalek::{SigningKey, pkcs8::EncodePublicKey as _};
use futures_util::{StreamExt as _, stream::FuturesUnordered};
use rustls::pki_types::{
    CertificateDer, PrivateKeyDer, PrivatePkcs1KeyDer, PrivatePkcs8KeyDer, PrivateSec1KeyDer,
    pem::{PemObject as _, SectionKind},
};
use tempfile::NamedTempFile;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, server::TlsStream};
use tracing::{debug, error, info};
use zeroize::Zeroizing;

const LICENSE_TIME_FORMAT: &[time::format_description::FormatItem<'static>] = time::macros::format_description!(
    "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
);

#[derive(Debug, Parser)]
#[command(
    name = "aster-control",
    version,
    about = "Aster Team customer control backend"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate a non-secret installation database configuration.
    InspectDatabaseConfiguration {
        #[arg(long)]
        source: PathBuf,
    },
    /// Inspect installed MariaDB migration history without applying migrations.
    InspectDatabaseMigrations {
        #[arg(long, default_value_os_t = default_installation_profile())]
        installation_profile: PathBuf,
        #[arg(long, default_value_os_t = default_installation_key())]
        installation_key: PathBuf,
        #[command(flatten)]
        database: DatabaseArgs,
    },
    InitializeInstallation {
        #[arg(long, default_value_os_t = default_installation_profile())]
        installation_profile: PathBuf,
        #[arg(long, default_value_os_t = default_installation_key())]
        installation_key: PathBuf,
    },
    VerifyMachineIdentity {
        #[arg(long, default_value_os_t = default_installation_profile())]
        installation_profile: PathBuf,
    },
    GenerateLicenseRequest {
        #[arg(long, default_value_os_t = default_installation_profile())]
        installation_profile: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    VerifyRelease {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        manifest: Option<PathBuf>,
    },
    Serve {
        #[arg(long)]
        install_root: Option<PathBuf>,
        #[arg(long, env = "ASTER_CONTROL_LISTEN", default_value = "127.0.0.1:11080")]
        listen: SocketAddr,
        #[arg(long, default_value_os_t = default_license_file())]
        license_file: PathBuf,
        #[arg(long, default_value_os_t = default_installation_profile())]
        installation_profile: PathBuf,
        #[arg(long, default_value_os_t = default_installation_key())]
        installation_key: PathBuf,
        #[arg(long, default_value_os_t = default_license_state())]
        license_state: PathBuf,
        #[command(flatten)]
        database: DatabaseArgs,
        #[command(flatten)]
        runner_task_key: RunnerTaskKeyArgs,
        #[command(flatten)]
        web: WebArgs,
        #[command(flatten)]
        api_tls: ApiTlsArgs,
        #[command(flatten)]
        runtime: RuntimeControlArgs,
    },
    Preflight {
        #[arg(long, env = "ASTER_CONTROL_LISTEN", default_value = "127.0.0.1:11080")]
        listen: SocketAddr,
        #[arg(long, default_value_os_t = default_license_file())]
        license_file: PathBuf,
        #[arg(long, default_value_os_t = default_installation_profile())]
        installation_profile: PathBuf,
        #[arg(long, default_value_os_t = default_installation_key())]
        installation_key: PathBuf,
        #[arg(long, default_value_os_t = default_license_state())]
        license_state: PathBuf,
        #[command(flatten)]
        database: DatabaseArgs,
        #[command(flatten)]
        runner_task_key: RunnerTaskKeyArgs,
        #[arg(long, default_value_os_t = default_control_runner_task_keys())]
        runner_task_public_keys_file: PathBuf,
        #[command(flatten)]
        web: WebArgs,
        #[command(flatten)]
        api_tls: ApiTlsArgs,
    },
    InstallLicense {
        #[arg(long)]
        source: PathBuf,
        #[arg(long, default_value_os_t = default_license_file())]
        license_file: PathBuf,
        #[arg(long, default_value_os_t = default_installation_profile())]
        installation_profile: PathBuf,
        #[arg(long, default_value_os_t = default_installation_key())]
        installation_key: PathBuf,
        #[arg(long, default_value_os_t = default_license_state())]
        license_state: PathBuf,
    },
    VerifyBundledFreeLicense {
        #[arg(long)]
        source: PathBuf,
        #[arg(long, default_value_t = 0)]
        minimum_valid_for_seconds: u16,
    },
    InitializeDatabase {
        #[command(flatten)]
        database: DatabaseArgs,
    },
    InitializeRuntimeConfiguration {
        #[arg(long, default_value_os_t = default_installation_profile())]
        installation_profile: PathBuf,
        #[arg(long, default_value_os_t = default_installation_key())]
        installation_key: PathBuf,
        #[command(flatten)]
        database: DatabaseArgs,
        #[arg(long)]
        public_api_base_url: String,
    },
    RecordUpgradeAudit {
        #[arg(long, value_enum)]
        outcome: UpgradeAuditOutcome,
        #[arg(long)]
        target_version: String,
        /// Bind executor retries to one immutable signed audit event.
        #[arg(long)]
        job_id: Option<String>,
        #[arg(long, default_value_os_t = default_installation_profile())]
        installation_profile: PathBuf,
        #[arg(long, default_value_os_t = default_installation_key())]
        installation_key: PathBuf,
        #[command(flatten)]
        database: DatabaseArgs,
    },
    InitializeOwner {
        #[arg(long, default_value_os_t = default_installation_profile())]
        installation_profile: PathBuf,
        #[arg(long, default_value_os_t = default_installation_key())]
        installation_key: PathBuf,
        #[command(flatten)]
        database: DatabaseArgs,
        #[arg(long)]
        email: String,
        #[arg(long, default_value = "Aster Owner")]
        display_name: String,
        #[arg(long)]
        password_file: PathBuf,
    },
    ResetAdminPassword {
        #[arg(long, default_value_os_t = default_installation_profile())]
        installation_profile: PathBuf,
        #[arg(long, default_value_os_t = default_installation_key())]
        installation_key: PathBuf,
        #[command(flatten)]
        database: DatabaseArgs,
        #[arg(long)]
        email: String,
        #[arg(
            long,
            required_unless_present = "password_stdin",
            conflicts_with = "password_stdin"
        )]
        password_file: Option<PathBuf>,
        #[arg(long, required_unless_present = "password_file")]
        password_stdin: bool,
    },
    InitializeLocalRunner {
        #[arg(long, default_value_os_t = default_installation_profile())]
        installation_profile: PathBuf,
        #[arg(long, default_value_os_t = default_installation_key())]
        installation_key: PathBuf,
        #[arg(long, default_value_os_t = default_license_file())]
        license_file: PathBuf,
        #[arg(long, default_value_os_t = default_license_state())]
        license_state: PathBuf,
        #[command(flatten)]
        database: DatabaseArgs,
        #[arg(long)]
        owner_email: String,
        #[arg(long, default_value = "local-runner")]
        name: String,
        #[arg(long)]
        identity_output: PathBuf,
        #[arg(long, default_value_t = 4)]
        max_inflight: u32,
    },
    /// Persist and register one local slot identity; invoked by the installation executor.
    PrepareLocalRunnerSlot {
        #[arg(long, default_value_os_t = default_installation_profile())]
        installation_profile: PathBuf,
        #[arg(long, default_value_os_t = default_installation_key())]
        installation_key: PathBuf,
        #[arg(long, default_value_os_t = default_license_file())]
        license_file: PathBuf,
        #[arg(long, default_value_os_t = default_license_state())]
        license_state: PathBuf,
        #[arg(long, value_parser = local_runner_slot::parse_job_id)]
        job_id: String,
        #[command(flatten)]
        database: DatabaseArgs,
        #[arg(long)]
        actor_id: String,
        #[arg(long, value_parser = local_runner_slot::parse_slot)]
        slot: aster_upgrade_core::ReleaseSlot,
        /// Private staging output; the executor installs it with Runner ownership.
        #[arg(long)]
        identity_output: PathBuf,
    },
    /// Retire the non-active member before archiving a durable upgrade job.
    FinalizeLocalRunnerSlot {
        #[arg(long, default_value_os_t = default_installation_profile())]
        installation_profile: PathBuf,
        #[arg(long, default_value_os_t = default_installation_key())]
        installation_key: PathBuf,
        #[arg(long, default_value_os_t = default_license_file())]
        license_file: PathBuf,
        #[arg(long, default_value_os_t = default_license_state())]
        license_state: PathBuf,
        #[arg(long, value_parser = local_runner_slot::parse_job_id)]
        job_id: String,
        #[command(flatten)]
        database: DatabaseArgs,
        #[arg(long, value_parser = local_runner_slot::parse_slot)]
        slot: aster_upgrade_core::ReleaseSlot,
    },
    InitializeRunnerTaskKey {
        #[arg(long, default_value_os_t = default_runner_task_key())]
        key_file: PathBuf,
    },
    ExportRunnerTaskKeys {
        #[arg(long, default_value = "runner-task-installation-v1")]
        key_id: String,
        #[arg(long, default_value_os_t = default_runner_task_key())]
        key_file: PathBuf,
        #[arg(long, default_value_os_t = default_control_runner_task_keys())]
        target: PathBuf,
    },
}

#[derive(Clone, Debug, Args)]
struct RunnerTaskKeyArgs {
    #[arg(long, default_value = "runner-task-installation-v1")]
    runner_task_key_id: String,
    #[arg(long, default_value_os_t = default_runner_task_key())]
    runner_task_key_file: PathBuf,
}

#[derive(Clone, Debug, Args)]
struct WebArgs {
    #[arg(
        long,
        env = "ASTER_CONTROL_ADMIN_LISTEN",
        default_value = "127.0.0.1:11082"
    )]
    admin_listen: SocketAddr,
    #[arg(
        long,
        env = "ASTER_CONTROL_MEMBER_LISTEN",
        default_value = "127.0.0.1:11081"
    )]
    member_listen: SocketAddr,
    #[arg(long, default_value_os_t = default_admin_assets())]
    admin_assets: PathBuf,
    #[arg(long, default_value_os_t = default_member_assets())]
    member_assets: PathBuf,
    #[arg(
        long,
        env = "ASTER_CONTROL_ALLOW_INSECURE_HTTP",
        default_value_t = false,
        action = clap::ArgAction::Set
    )]
    allow_insecure_http: bool,
    #[arg(
        long,
        env = "ASTER_CONTROL_SECURE_COOKIES",
        default_value_t = true,
        action = clap::ArgAction::Set
    )]
    secure_cookies: bool,
}

#[derive(Clone, Debug, Default, Args)]
struct RuntimeControlArgs {
    #[arg(long, env = "ASTER_CONTROL_RUNTIME_LISTEN")]
    runtime_listen: Option<SocketAddr>,
    #[arg(long, env = "ASTER_CONTROL_RUNTIME_TOKEN_FILE")]
    runtime_token_file: Option<PathBuf>,
    #[arg(long, env = "ASTER_CONTROL_RUNTIME_SLOT", value_parser = ["blue", "green"])]
    runtime_slot: Option<String>,
    #[arg(long, env = "ASTER_CONTROL_RUNTIME_CANDIDATE", default_value_t = false, action = clap::ArgAction::Set)]
    runtime_candidate: bool,
}

struct RuntimeControlConfig {
    listen: SocketAddr,
    slot: aster_upgrade_core::ReleaseSlot,
    token: Zeroizing<[u8; 32]>,
    candidate: bool,
}

impl RuntimeControlArgs {
    fn load(&self) -> io::Result<Option<RuntimeControlConfig>> {
        let (listen, path, slot) = match (
            self.runtime_listen,
            self.runtime_token_file.as_deref(),
            self.runtime_slot.as_deref(),
        ) {
            (None, None, None) if !self.runtime_candidate => return Ok(None),
            (Some(listen), Some(path), Some(slot)) if listen.ip().is_loopback() => {
                (listen, path, slot)
            }
            _ => {
                return Err(io::Error::other(
                    "runtime control requires a loopback listener, token file and slot",
                ));
            }
        };
        let slot = match slot {
            "blue" => aster_upgrade_core::ReleaseSlot::Blue,
            "green" => aster_upgrade_core::ReleaseSlot::Green,
            _ => return Err(io::Error::other("invalid runtime control slot")),
        };
        if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
            return Err(io::Error::other("runtime token must not be a symlink"));
        }
        let mut file = std::fs::File::open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() != 32 {
            return Err(io::Error::other(
                "runtime token must be a regular file containing exactly 32 random bytes",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            if metadata.permissions().mode() & 0o027 != 0 {
                return Err(io::Error::other(
                    "runtime token must not be group-writable or accessible to others",
                ));
            }
        }
        let mut token = Zeroizing::new([0; 32]);
        file.read_exact(&mut *token)?;
        Ok(Some(RuntimeControlConfig {
            listen,
            slot,
            token,
            candidate: self.runtime_candidate,
        }))
    }
}

#[derive(Clone, Debug, Args)]
struct ApiTlsArgs {
    #[arg(long, env = "ASTER_CONTROL_API_TLS_CERTIFICATE")]
    api_tls_certificate: Option<PathBuf>,
    #[arg(long, env = "ASTER_CONTROL_API_TLS_PRIVATE_KEY")]
    api_tls_private_key: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum DatabaseDriver {
    Mariadb,
    Sqlcipher,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum UpgradeAuditOutcome {
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, Args)]
struct DatabaseArgs {
    #[arg(long, value_enum)]
    database_driver: Option<DatabaseDriver>,
    #[arg(long, default_value = "127.0.0.1")]
    database_host: String,
    #[arg(long, default_value_t = 3306)]
    database_port: u16,
    #[arg(long, default_value = "aster_team")]
    database_name: String,
    #[arg(long, default_value = "aster_team")]
    database_user: String,
    #[arg(long, default_value_os_t = default_database_password())]
    database_password_file: PathBuf,
    #[arg(long, default_value_t = false)]
    database_tls: bool,
    #[arg(long)]
    database_ca_certificate: Option<PathBuf>,
    #[arg(long, default_value_t = 10)]
    database_max_connections: u32,
    #[arg(long, default_value_os_t = default_database_file())]
    sqlcipher_file: PathBuf,
    #[arg(long, default_value_os_t = default_database_key())]
    sqlcipher_key_file: PathBuf,
}

impl DatabaseArgs {
    fn resolved(&self) -> Result<Self, Box<dyn std::error::Error>> {
        let layout = default_layout();
        if !layout.marker_path().is_file() {
            return Ok(self.clone());
        }
        let config = DatabaseConfiguration::load(&layout)?;
        self.resolve_installed(&layout, &config)
    }

    fn resolve_installed(
        &self,
        layout: &InstallLayout,
        config: &DatabaseConfiguration,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        config.validate()?;
        config.validate_platform(Platform::current(), std::env::consts::ARCH)?;
        let mut resolved = self.clone();
        let driver = match config {
            DatabaseConfiguration::Sqlcipher {} => DatabaseDriver::Sqlcipher,
            DatabaseConfiguration::Mariadb {
                host,
                port,
                database,
                username,
                tls,
                custom_ca,
                max_connections,
            } => {
                resolved.database_host = host.clone();
                resolved.database_port = *port;
                resolved.database_name = database.clone();
                resolved.database_user = username.clone();
                resolved.database_password_file = layout.database_password();
                resolved.database_tls = *tls;
                resolved.database_ca_certificate =
                    custom_ca.then(|| layout.database_ca_certificate());
                resolved.database_max_connections = *max_connections;
                DatabaseDriver::Mariadb
            }
        };
        if self
            .database_driver
            .is_some_and(|selected| selected != driver)
        {
            return Err(
                "database driver conflicts with the installed database configuration".into(),
            );
        }
        resolved.database_driver = Some(driver);
        resolved.sqlcipher_file = layout.database_file();
        resolved.sqlcipher_key_file = layout.database_key();
        Ok(resolved)
    }
}

fn default_layout() -> InstallLayout {
    InstallLayout::discover_or_default()
        .unwrap_or_else(|error| panic!("invalid Aster Team install root: {error}"))
}

fn default_installation_profile() -> PathBuf {
    default_layout().installation_profile()
}

fn default_installation_key() -> PathBuf {
    default_layout().installation_key()
}

fn default_license_file() -> PathBuf {
    default_layout().license_file()
}

fn default_license_state() -> PathBuf {
    default_layout().license_state()
}

fn default_runner_task_key() -> PathBuf {
    default_layout().runner_task_key()
}

fn default_control_runner_task_keys() -> PathBuf {
    default_layout().control_runner_task_keys()
}

fn default_admin_assets() -> PathBuf {
    default_layout().admin_assets()
}

fn default_member_assets() -> PathBuf {
    default_layout().member_assets()
}

fn default_database_password() -> PathBuf {
    default_layout().database_password()
}

fn default_database_file() -> PathBuf {
    default_layout().database_file()
}

fn default_database_key() -> PathBuf {
    default_layout().database_key()
}

fn init_service_logging() {
    // Kept out of the async entry point: every temporary in an `async fn` body
    // reserves its own slot in the future, so the subscriber builder chain would
    // otherwise be part of the main-thread stack requirement.
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "aster_control=info".into()),
        )
        .init();
}

/// Windows reserves only 1 MiB of stack for the process main thread, while this
/// CLI runs command dispatch, database migration, upgrade and serving code on the
/// thread that polls the entry future. Unoptimized builds need far more than that
/// reserve, so the entry point runs on a thread whose stack is sized here instead
/// of inheriting the platform default.
const ENTRY_STACK_BYTES: usize = 32 * 1024 * 1024;

fn main() {
    let entry = std::thread::Builder::new()
        .name("aster-control-entry".to_owned())
        .stack_size(ENTRY_STACK_BYTES)
        .spawn(entry_point)
        .expect("spawn the aster-control entry thread");
    if entry.join().is_err() {
        // A panicked entry thread reports the status this process used before the
        // CLI moved off the main thread.
        std::process::exit(101);
    }
}

fn entry_point() {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("start the aster-control runtime")
        .block_on(run_command_line());
}

async fn run_command_line() {
    let cli = Cli::parse();
    let service_logging = matches!(&cli.command, Command::Serve { .. });
    if service_logging {
        init_service_logging();
    }
    // Keep the dispatch state machine on the heap as well: it is polled on the
    // entry thread, and its size must not add to what that stack has to provide.
    if let Err(error) = Box::pin(run(cli)).await {
        if service_logging {
            error!(error = %error, "aster-control stopped");
        } else {
            eprintln!("aster-control: {error}");
        }
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    // Linux service and installer entrypoints verify the host identity as root
    // before crossing into the unprivileged Control process. Some supported
    // hosts expose the DMI product UUID as root-only, so those unprivileged
    // commands must retain the already-verified profile instead of trying to
    // read the privileged machine factor again.
    let runtime_profile = match &cli.command {
        Command::Preflight {
            installation_profile,
            ..
        }
        | Command::Serve {
            installation_profile,
            ..
        }
        | Command::InitializeLocalRunner {
            installation_profile,
            ..
        } => Some(load_profile(installation_profile)?),
        Command::InspectDatabaseMigrations {
            installation_profile,
            ..
        }
        | Command::InstallLicense {
            installation_profile,
            ..
        }
        | Command::PrepareLocalRunnerSlot {
            installation_profile,
            ..
        }
        | Command::FinalizeLocalRunnerSlot {
            installation_profile,
            ..
        } => Some(verify_machine_profile(installation_profile)?),
        _ => None,
    };
    let trusted_keys = compiled_license_keys().map_err(debug_error)?;
    match cli.command {
        Command::InspectDatabaseConfiguration { source } => {
            let config = DatabaseConfiguration::parse(&std::fs::read(source)?)?;
            config.validate_platform(Platform::current(), std::env::consts::ARCH)?;
            if config.is_external() && !cfg!(feature = "mariadb") {
                return Err("this binary was built without MariaDB support".into());
            }
            println!(
                "{}",
                if config.is_external() {
                    "mariadb"
                } else {
                    "sqlcipher"
                }
            );
        }
        Command::InspectDatabaseMigrations {
            installation_key,
            database,
            ..
        } => {
            let profile = runtime_profile.ok_or("verified installation profile is missing")?;
            inspect_database_migrations(&database, &profile, &installation_key).await?;
        }
        Command::InitializeInstallation {
            installation_profile,
            installation_key,
        } => {
            initialize_installation(&installation_profile, &installation_key)?;
            let profile = verify_machine_profile(&installation_profile)?;
            info!(
                installation_id = %profile.installation_id,
                profile = %installation_profile.display(),
                "installation identity initialized"
            );
        }
        Command::VerifyMachineIdentity {
            installation_profile,
        } => {
            let profile = verify_machine_profile(&installation_profile)?;
            #[cfg(target_os = "linux")]
            aster_control::prepare_runner_upgrade_lock()?;
            #[cfg(target_os = "linux")]
            aster_control::prepare_settlement_outbox()?;
            info!(
                installation_id = %profile.installation_id,
                profile = %installation_profile.display(),
                "installation identity matches the current machine"
            );
        }
        Command::GenerateLicenseRequest {
            installation_profile,
            output,
        } => {
            let profile = load_profile(&installation_profile)?;
            let request = generate_license_request(
                &profile,
                std::env::consts::ARCH,
                time::OffsetDateTime::now_utc(),
            )?;
            let mut encoded = serde_json::to_vec_pretty(&request)?;
            encoded.push(b'\n');
            atomic_write_new(&output, &encoded, 0o640)?;
            info!(
                request_id = %request.request_id,
                output = %output.display(),
                "offline license request generated"
            );
        }
        Command::VerifyRelease { root, manifest } => {
            let manifest = manifest.unwrap_or_else(|| root.join(RELEASE_MANIFEST_FILE));
            let trusted_release_keys = compiled_release_keys().map_err(debug_error)?;
            let document = std::fs::read(&manifest)?;
            let release = verify_release(&document, &trusted_release_keys).map_err(debug_error)?;
            verify_release_tree(&root, &release).map_err(debug_error)?;
            info!(
                version = %release.claims().version,
                key_id = %release.claims().key_id,
                root = %root.display(),
                "release signature and file tree verified"
            );
        }
        Command::Preflight {
            listen,
            license_file,
            installation_profile: _,
            installation_key,
            license_state,
            database,
            runner_task_key,
            runner_task_public_keys_file,
            web,
            api_tls,
        } => {
            let profile = runtime_profile.ok_or(MachineIdentityError::InvalidProfile)?;
            let installation_key = read_installation_key(&installation_key)?;
            AuthCore::new(&installation_key, &profile.installation_id)?;
            CredentialVault::new(&installation_key, &profile.installation_id)?;
            let state_store = LicenseStateStore::new(license_state, &installation_key)?;
            let license = load_installed_license_with_staged_activation(
                &license_file,
                &trusted_keys,
                &profile,
                &state_store,
                env!("CARGO_PKG_VERSION"),
                time::OffsetDateTime::now_utc(),
            )
            .map_err(debug_error)?;
            if let Some(license) = license.as_ref() {
                state_store.validate_replacement(license, time::OffsetDateTime::now_utc())?;
            }
            let storage = open_database(&database).await?;
            let occupied_seats = storage.occupied_seats().await.map_err(debug_error)?;
            load_runner_task_issuer(&runner_task_key)?;
            verify_runner_task_public_keys(
                &runner_task_key.runner_task_key_id,
                &runner_task_key.runner_task_key_file,
                &runner_task_public_keys_file,
            )?;
            validate_web_assets(&web)?;
            let api_tls = load_api_tls(&api_tls)?;
            validate_listener_security(listen, &web, api_tls.is_some())?;
            if let Some(license) = license.as_ref() {
                info!(
                    license_id = license.as_ref().license_id(),
                    key_id = license.as_ref().key_id(),
                    occupied_seats,
                    currently_active = aster_policy_core::check_license_current(
                        license,
                        env!("CARGO_PKG_VERSION"),
                        time::OffsetDateTime::now_utc()
                    )
                    .is_ok(),
                    "control preflight passed with a verified license"
                );
            } else {
                tracing::warn!(
                    occupied_seats,
                    "control preflight passed without a license; licensed features are unavailable"
                );
            }
        }
        Command::Serve {
            install_root,
            listen,
            license_file,
            installation_profile: _,
            installation_key,
            license_state,
            database,
            runner_task_key,
            web,
            api_tls,
            runtime,
        } => {
            let runtime = runtime.load()?;
            let api_tls = load_api_tls(&api_tls)?;
            validate_listener_security(listen, &web, api_tls.is_some())?;
            let storage = if runtime.as_ref().is_some_and(|runtime| runtime.candidate) {
                open_slot_database(&database, DatabaseOpenPolicy::Existing).await?
            } else {
                open_database(&database).await?
            };
            let task_issuer = load_runner_task_issuer(&runner_task_key)?;
            let profile = runtime_profile.ok_or(MachineIdentityError::InvalidProfile)?;
            let runtime_installation_id = profile.installation_id.clone();
            let installation_key = read_installation_key(&installation_key)?;
            let credential_vault =
                CredentialVault::new(&installation_key, &profile.installation_id)?;
            let auth_core = AuthCore::new(&installation_key, &profile.installation_id)?;
            let state_store = Arc::new(LicenseStateStore::new(license_state, &installation_key)?);
            let license = match load_installed_license_with_staged_activation(
                &license_file,
                &trusted_keys,
                &profile,
                &state_store,
                env!("CARGO_PKG_VERSION"),
                time::OffsetDateTime::now_utc(),
            ) {
                Ok(license) => license,
                Err(error) => {
                    // The configured installer remains authoritative below. It
                    // will report unavailable, never use this empty cache to
                    // grant access; keep authenticated recovery reachable.
                    tracing::warn!(
                        ?error,
                        "license recovery is required; licensed features are unavailable"
                    );
                    None
                }
            };
            validate_web_assets(&web)?;
            let maintenance_layout = match install_root {
                Some(root) => InstallLayout::new(root)
                    .map_err(|error| std::io::Error::other(error.to_string()))?,
                None => default_layout(),
            };
            let mut state = ControlState::new(env!("CARGO_PKG_VERSION"), license)
                .with_storage(storage)
                .with_license_state(state_store)
                .with_license_installer(license_file, trusted_keys, profile)
                .with_maintenance_layout(maintenance_layout.clone())
                .with_task_issuer(task_issuer)
                .with_secure_session_cookies(web.secure_cookies);
            state = state
                .with_credential_vault(credential_vault)
                .with_auth_core(auth_core)
                .with_settlement_outbox(&maintenance_layout)
                .map_err(debug_error)?;
            state
                .initialize_migrated_model_access()
                .await
                .map_err(debug_error)?;
            state = state
                .with_plugin_bundle(maintenance_layout.clone())
                .map_err(debug_error)?;
            if state.plugins_configured() {
                state.poll_plugin_updates().await.map_err(debug_error)?;
                let plugin_state = state.clone();
                tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        if let Err(error) = plugin_state.poll_plugin_updates().await {
                            tracing::warn!(?error, "signed Lua bundle poll failed");
                        }
                    }
                });
            }
            let runtime = runtime
                .map(|config| {
                    if config.candidate {
                        state.request_lifecycle().begin_drain();
                    }
                    aster_control::runtime_control::RuntimeControl::new(
                        state.clone(),
                        runtime_installation_id,
                        config.slot,
                        &config.token,
                    )
                    .map(|runtime| {
                        (
                            config.listen,
                            runtime.with_web_assets(
                                web.admin_assets.clone(),
                                web.member_assets.clone(),
                            ),
                        )
                    })
                    .map_err(debug_error)
                })
                .transpose()?;
            serve_control_surfaces(listen, &web, api_tls, state, runtime).await?;
        }
        Command::InstallLicense {
            source,
            license_file,
            installation_profile: _,
            installation_key,
            license_state,
        } => {
            let profile = runtime_profile.ok_or(MachineIdentityError::InvalidProfile)?;
            let installation_key = read_installation_key(&installation_key)?;
            let state_store = LicenseStateStore::new(license_state, &installation_key)?;
            let source = std::fs::read(source)?;
            let outcome = install_or_stage_license(
                &source,
                &license_file,
                &trusted_keys,
                &profile,
                &state_store,
                env!("CARGO_PKG_VERSION"),
                time::OffsetDateTime::now_utc(),
            )
            .map_err(debug_error)?;
            let license = outcome.license();
            info!(
                license_id = %license.as_ref().license_id(),
                key_id = %license.as_ref().key_id(),
                activation = outcome.activation(),
                "license accepted"
            );
        }
        Command::VerifyBundledFreeLicense {
            source,
            minimum_valid_for_seconds,
        } => {
            let source = std::fs::read(source)?;
            let now = time::OffsetDateTime::now_utc();
            let license = verify_bundled_free_license(
                &source,
                &trusted_keys,
                env!("CARGO_PKG_VERSION"),
                now,
                now + time::Duration::seconds(i64::from(minimum_valid_for_seconds)),
            )
            .map_err(debug_error)?;
            info!(
                license_id = %license.as_ref().license_id(),
                key_id = %license.as_ref().key_id(),
                "bundled free license verified"
            );
        }
        Command::InitializeDatabase { database } => {
            initialize_database(&database).await?;
            info!(driver = ?database.database_driver, "database initialized");
        }
        Command::InitializeRuntimeConfiguration {
            installation_profile,
            installation_key,
            database,
            public_api_base_url,
        } => {
            let profile = load_profile(&installation_profile)?;
            let installation_key = read_installation_key(&installation_key)?;
            let auth_core = AuthCore::new(&installation_key, &profile.installation_id)?;
            let storage = open_database(&database).await?;
            ControlState::new(env!("CARGO_PKG_VERSION"), None)
                .with_storage(storage)
                .with_auth_core(auth_core)
                .initialize_runtime_configuration(&public_api_base_url)
                .await
                .map_err(debug_error)?;
            info!(public_api_base_url, "runtime configuration initialized");
        }
        Command::RecordUpgradeAudit {
            outcome,
            target_version,
            job_id,
            installation_profile,
            installation_key,
            database,
        } => {
            let profile = load_profile(&installation_profile)?;
            let installation_key = read_installation_key(&installation_key)?;
            let auth_core = AuthCore::new(&installation_key, &profile.installation_id)?;
            let storage = open_database(&database).await?;
            let state = ControlState::new(env!("CARGO_PKG_VERSION"), None)
                .with_storage(storage)
                .with_auth_core(auth_core);
            let succeeded = outcome == UpgradeAuditOutcome::Succeeded;
            if let Some(job_id) = job_id {
                state
                    .record_upgrade_audit_for_job(&job_id, &target_version, succeeded)
                    .await
            } else {
                state.record_upgrade_audit(&target_version, succeeded).await
            }
            .map_err(debug_error)?;
            info!(
                target_version,
                outcome = ?outcome,
                "upgrade audit event recorded"
            );
        }
        Command::InitializeOwner {
            installation_profile,
            installation_key,
            database,
            email,
            display_name,
            password_file,
        } => {
            let profile = load_profile(&installation_profile)?;
            let installation_key = read_installation_key(&installation_key)?;
            let auth_core = AuthCore::new(&installation_key, &profile.installation_id)?;
            let storage = open_database(&database).await?;
            let password = read_password(&password_file)?;
            let owner = ControlState::new(env!("CARGO_PKG_VERSION"), None)
                .with_storage(storage)
                .with_auth_core(auth_core)
                .ensure_owner_identity(&email, &display_name, password)
                .await
                .map_err(debug_error)?;
            info!(identity_id = %owner.id, email = %owner.email, "owner identity initialized");
        }
        Command::ResetAdminPassword {
            installation_profile,
            installation_key,
            database,
            email,
            password_file,
            password_stdin,
        } => {
            let profile = load_profile(&installation_profile)?;
            let installation_key = read_installation_key(&installation_key)?;
            let auth_core = AuthCore::new(&installation_key, &profile.installation_id)?;
            let storage = open_database(&database).await?;
            let password = match (password_file, password_stdin) {
                (Some(path), false) => read_password(&path)?,
                (None, true) => read_password_stdin()?,
                _ => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "exactly one password source is required",
                    )
                    .into());
                }
            };
            ControlState::new(env!("CARGO_PKG_VERSION"), None)
                .with_storage(storage)
                .with_auth_core(auth_core)
                .reset_admin_password(&email, password)
                .await
                .map_err(debug_error)?;
            println!("Administrator password reset for {email}.");
        }
        Command::InitializeLocalRunner {
            installation_profile: _,
            installation_key,
            license_file,
            license_state,
            database,
            owner_email,
            name,
            identity_output,
            max_inflight,
        } => {
            let profile = runtime_profile.ok_or(MachineIdentityError::InvalidProfile)?;
            let installation_key = read_installation_key(&installation_key)?;
            let auth_core = AuthCore::new(&installation_key, &profile.installation_id)?;
            let state_store = Arc::new(LicenseStateStore::new(license_state, &installation_key)?);
            let license = load_installed_license_with_staged_activation(
                &license_file,
                &trusted_keys,
                &profile,
                &state_store,
                env!("CARGO_PKG_VERSION"),
                time::OffsetDateTime::now_utc(),
            )
            .map_err(debug_error)?;
            let storage = open_database(&database).await?;
            let state = ControlState::new(env!("CARGO_PKG_VERSION"), license)
                .with_storage(storage)
                .with_license_state(state_store)
                .with_license_installer(license_file, trusted_keys, profile)
                .with_auth_core(auth_core);
            let pending_identity = pending_output_path(&identity_output);
            let prepared = if identity_output.exists() {
                read_local_runner_identity(&identity_output)?
            } else if pending_identity.exists() {
                read_local_runner_identity(&pending_identity)?
            } else {
                let prepared = generate_bootstrap_local_runner_identity().map_err(debug_error)?;
                atomic_write_new(
                    &pending_identity,
                    &encode_local_runner_identity(&prepared)?,
                    0o600,
                )?;
                prepared
            };
            let identity = state
                .bootstrap_local_runner_with_identity(
                    &owner_email,
                    &name,
                    std::env::consts::OS,
                    std::env::consts::ARCH,
                    max_inflight,
                    prepared,
                )
                .await
                .map_err(debug_error)?;
            finish_local_runner_identity(&pending_identity, &identity_output, &identity)?;
            info!(
                runner_id = %identity.runner_id,
                identity_output = %identity_output.display(),
                "local Runner identity initialized"
            );
        }
        Command::PrepareLocalRunnerSlot {
            installation_profile: _,
            installation_key,
            license_file,
            license_state,
            job_id,
            database,
            actor_id,
            slot,
            identity_output,
        } => {
            if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
                return Err(io::Error::other("local Runner slots require Linux amd64").into());
            }
            let layout = InstallLayout::discover_or_default()?;
            layout.verify_marker_bytes(&std::fs::read(layout.marker_path())?)?;
            let context =
                local_runner_slot::UpgradeContext::load(&layout, &job_id, &actor_id, slot)?;
            let profile = runtime_profile.ok_or(MachineIdentityError::InvalidProfile)?;
            let installation_key = read_installation_key(&installation_key)?;
            let auth_core = AuthCore::new(&installation_key, &profile.installation_id)?;
            let state_store = Arc::new(LicenseStateStore::new(license_state, &installation_key)?);
            let license = load_installed_license_with_staged_activation(
                &license_file,
                &trusted_keys,
                &profile,
                &state_store,
                env!("CARGO_PKG_VERSION"),
                time::OffsetDateTime::now_utc(),
            )
            .map_err(debug_error)?;
            let policy =
                if context.job.upgrade_mode == Some(aster_upgrade_core::UpgradeMode::BlueGreen) {
                    DatabaseOpenPolicy::Online
                } else {
                    DatabaseOpenPolicy::Maintenance
                };
            let storage = open_slot_database(&database, policy).await?;
            let prepared = local_runner_slot::PreparedSlot::load_or_create(
                &layout,
                &profile.installation_id,
                slot,
                &job_id,
            )?;
            let state = ControlState::new(env!("CARGO_PKG_VERSION"), license)
                .with_storage(storage)
                .with_license_state(state_store)
                .with_license_installer(license_file, trusted_keys, profile)
                .with_auth_core(auth_core);
            state
                .provision_local_runner_upgrade(
                    &context.job,
                    slot,
                    &context.previous,
                    &prepared.identity,
                )
                .await
                .map_err(debug_error)?;
            state.drain_licensed_mutations().await;
            prepared.publish(&identity_output)?;
            info!(slot = slot.id(), runner_id = %prepared.identity.runner_id, "local Runner slot prepared");
        }
        Command::FinalizeLocalRunnerSlot {
            installation_profile: _,
            installation_key,
            license_file,
            license_state,
            job_id,
            database,
            slot,
        } => {
            if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
                return Err(io::Error::other("local Runner slots require Linux amd64").into());
            }
            let layout = InstallLayout::discover_or_default()?;
            layout.verify_marker_bytes(&std::fs::read(layout.marker_path())?)?;
            let context = local_runner_slot::FinalizationContext::load(&layout, &job_id, slot)?;
            let profile = runtime_profile.ok_or(MachineIdentityError::InvalidProfile)?;
            let installation_key = read_installation_key(&installation_key)?;
            let auth_core = AuthCore::new(&installation_key, &profile.installation_id)?;
            let state_store = Arc::new(LicenseStateStore::new(license_state, &installation_key)?);
            let license = load_installed_license_with_staged_activation(
                &license_file,
                &trusted_keys,
                &profile,
                &state_store,
                env!("CARGO_PKG_VERSION"),
                time::OffsetDateTime::now_utc(),
            )
            .map_err(debug_error)?;
            let storage = open_slot_database(&database, DatabaseOpenPolicy::Existing).await?;
            let journal = layout
                .runner_slot_provisioning(slot.id())
                .with_extension(format!("{job_id}.json"));
            let prepared = match std::fs::symlink_metadata(&journal) {
                Ok(_) => Some(local_runner_slot::PreparedSlot::load_existing(
                    &layout,
                    &profile.installation_id,
                    slot,
                    &job_id,
                )?),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            let state = ControlState::new(env!("CARGO_PKG_VERSION"), license)
                .with_storage(storage)
                .with_license_state(state_store)
                .with_license_installer(license_file, trusted_keys, profile)
                .with_auth_core(auth_core);
            let recorded = state
                .reconcile_local_runner_upgrade(
                    &job_id,
                    prepared.as_ref().map(|prepared| &prepared.identity),
                    context.survivor.as_ref(),
                )
                .await
                .map_err(debug_error)?;
            if context.requires_binding && !recorded {
                return Err(io::Error::other(
                    "active candidate has no authenticated upgrade binding",
                )
                .into());
            }
            state.drain_licensed_mutations().await;
            info!(slot = slot.id(), job_id, "local Runner upgrade finalized");
        }
        Command::InitializeRunnerTaskKey { key_file } => {
            initialize_secret_key(&key_file)?;
            info!(path = %key_file.display(), "Runner task signing key initialized");
        }
        Command::ExportRunnerTaskKeys {
            key_id,
            key_file,
            target,
        } => {
            export_runner_task_keys(&key_id, &key_file, &target)?;
            info!(path = %target.display(), key_id, "Runner task public key exported");
        }
    }
    Ok(())
}

fn current_machine_factors() -> Result<RawMachineFactors, MachineIdentityError> {
    aster_machine_identity::current_machine_factors()
}

fn load_profile(path: &Path) -> Result<InstallationProfile, MachineIdentityError> {
    let data = std::fs::read(path).map_err(|_| MachineIdentityError::Io)?;
    parse_profile(&data)
}

fn verify_machine_profile(path: &Path) -> Result<InstallationProfile, MachineIdentityError> {
    let data = std::fs::read(path).map_err(|_| MachineIdentityError::Io)?;
    parse_and_verify_profile(&data, &current_machine_factors()?)
}

fn initialize_installation(
    profile_path: &Path,
    key_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    match (profile_path.exists(), key_path.exists()) {
        (true, true) => {
            load_profile(profile_path)?;
            read_installation_key(key_path)?;
            return Ok(());
        }
        (true, false) | (false, true) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "partial installation identity detected; restore the missing profile or key from the same backup",
            )
            .into());
        }
        (false, false) => {}
    }

    let factors = current_machine_factors()?;
    let profile = create_profile(random_cli_identifier("installation")?, &factors)?;
    let mut encoded_profile = serde_json::to_vec_pretty(&profile)?;
    encoded_profile.push(b'\n');
    let mut installation_key = Zeroizing::new([0_u8; 32]);
    getrandom::fill(installation_key.as_mut())
        .map_err(|_| std::io::Error::other("operating-system randomness unavailable"))?;

    atomic_write_new(profile_path, &encoded_profile, 0o640)?;
    if let Err(error) = atomic_write_new(key_path, installation_key.as_ref(), 0o600) {
        let _ = std::fs::remove_file(profile_path);
        return Err(error.into());
    }
    Ok(())
}

fn generate_license_request(
    profile: &InstallationProfile,
    rust_architecture: &str,
    now: time::OffsetDateTime,
) -> Result<LicenseRequest, Box<dyn std::error::Error>> {
    let architecture = match rust_architecture {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "license requests support only amd64 and arm64 targets",
            )
            .into());
        }
    };
    let request = LicenseRequest {
        schema: LICENSE_REQUEST_SCHEMA.to_owned(),
        request_id: random_cli_identifier("request")?,
        product: PRODUCT.to_owned(),
        product_version: env!("CARGO_PKG_VERSION").to_owned(),
        platform: Platform::current().id().to_owned(),
        architecture: architecture.to_owned(),
        installation_id: profile.installation_id.clone(),
        machine_fingerprint_sha256: profile.machine_fingerprint_sha256.clone(),
        machine_factors: profile.machine_factors.clone(),
        generated_at: now.format(LICENSE_TIME_FORMAT)?,
        license_schema: LICENSE_SCHEMA.to_owned(),
        capability_catalog_version: CATALOG_VERSION,
        quota_policy_version: QUOTA_POLICY_VERSION,
    };
    let encoded = serde_json::to_vec(&request)?;
    aster_license_core::request_v2::parse_request(&encoded)?;
    Ok(request)
}

fn random_cli_identifier(prefix: &str) -> Result<String, std::io::Error> {
    let mut random = [0_u8; 18];
    getrandom::fill(&mut random)
        .map_err(|_| std::io::Error::other("operating-system randomness unavailable"))?;
    Ok(format!(
        "{prefix}_{}",
        URL_SAFE_NO_PAD.encode(random.as_slice())
    ))
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum DatabaseOpenPolicy {
    Maintenance,
    Online,
    Existing,
}

async fn open_slot_database(
    args: &DatabaseArgs,
    policy: DatabaseOpenPolicy,
) -> Result<ControlStorage, Box<dyn std::error::Error>> {
    let args = args.resolved()?;
    if policy != DatabaseOpenPolicy::Maintenance && !default_layout().marker_path().is_file() {
        return Err(
            io::Error::other("online slot storage requires an installed database binding").into(),
        );
    }
    if !matches!(args.database_driver, Some(DatabaseDriver::Mariadb)) {
        return Err(
            io::Error::other("local Runner slots require the supported external database").into(),
        );
    }
    #[cfg(feature = "mariadb")]
    {
        Ok(ControlStorage::MariaDb(
            open_mariadb_with_policy(&args, false, policy).await?,
        ))
    }
    #[cfg(not(feature = "mariadb"))]
    {
        Err("this binary was built without MariaDB support".into())
    }
}

async fn open_database(args: &DatabaseArgs) -> Result<ControlStorage, Box<dyn std::error::Error>> {
    let args = args.resolved()?;
    match args.database_driver.unwrap_or(DatabaseDriver::Mariadb) {
        DatabaseDriver::Mariadb => {
            #[cfg(feature = "mariadb")]
            {
                let store = open_mariadb(&args, false).await?;
                Ok(ControlStorage::MariaDb(store))
            }
            #[cfg(not(feature = "mariadb"))]
            {
                Err("this binary was built without MariaDB support".into())
            }
        }
        DatabaseDriver::Sqlcipher => {
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            {
                let key = read_exact_key(&args.sqlcipher_key_file, "SQLCipher key")?;
                let store = aster_storage::SqlCipherStore::open(&args.sqlcipher_file, &key)?;
                Ok(ControlStorage::SqlCipher(Arc::new(Mutex::new(store))))
            }
            #[cfg(not(any(feature = "sqlcipher", feature = "sqlite-dev")))]
            {
                Err("this binary was built without SQLCipher support".into())
            }
        }
    }
}

fn validate_web_assets(args: &WebArgs) -> Result<(), std::io::Error> {
    for (name, root) in [
        ("admin", &args.admin_assets),
        ("member", &args.member_assets),
    ] {
        let metadata = std::fs::metadata(root).map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!(
                    "{name} web asset directory {} is unavailable",
                    root.display()
                ),
            )
        })?;
        if !metadata.is_dir() || !root.join("index.html").is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "{name} web asset directory {} does not contain index.html",
                    root.display()
                ),
            ));
        }
    }
    Ok(())
}

const MAX_TLS_CERTIFICATE_BYTES: u64 = 1024 * 1024;
const MAX_TLS_PRIVATE_KEY_BYTES: u64 = 128 * 1024;

fn load_api_tls(args: &ApiTlsArgs) -> Result<Option<Arc<rustls::ServerConfig>>, io::Error> {
    let (certificate_path, private_key_path) =
        match (&args.api_tls_certificate, &args.api_tls_private_key) {
            (None, None) => return Ok(None),
            (Some(certificate), Some(private_key)) => (certificate, private_key),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "API TLS certificate and private key must be configured together",
                ));
            }
        };

    let certificate_bytes = read_limited_tls_file(
        certificate_path,
        MAX_TLS_CERTIFICATE_BYTES,
        "API TLS certificate",
    )?;
    let mut certificates = Vec::new();
    for item in <(SectionKind, Vec<u8>)>::pem_slice_iter(&certificate_bytes) {
        match item.map_err(invalid_tls_pem)? {
            (SectionKind::Certificate, certificate) => {
                certificates.push(CertificateDer::from(certificate));
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "API TLS certificate file may contain only X.509 certificates",
                ));
            }
        }
    }
    if certificates.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "API TLS certificate file contains no X.509 certificate",
        ));
    }

    let private_key_bytes = Zeroizing::new(read_limited_tls_file(
        private_key_path,
        MAX_TLS_PRIVATE_KEY_BYTES,
        "API TLS private key",
    )?);
    let mut private_key = None;
    for item in <(SectionKind, Vec<u8>)>::pem_slice_iter(private_key_bytes.as_slice()) {
        let candidate = match item.map_err(invalid_tls_pem)? {
            (SectionKind::RsaPrivateKey, value) => {
                PrivateKeyDer::Pkcs1(PrivatePkcs1KeyDer::from(value))
            }
            (SectionKind::PrivateKey, value) => {
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(value))
            }
            (SectionKind::EcPrivateKey, value) => {
                PrivateKeyDer::Sec1(PrivateSec1KeyDer::from(value))
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "API TLS private key file may contain only one unencrypted private key",
                ));
            }
        };
        if private_key.replace(candidate).is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "API TLS private key file contains more than one private key",
            ));
        }
    }
    let private_key = private_key.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "API TLS private key file contains no supported private key",
        )
    })?;

    let provider = rustls::crypto::ring::default_provider();
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(provider))
        .with_safe_default_protocol_versions()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
        .with_no_client_auth()
        .with_single_cert(certificates, private_key)
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("API TLS certificate and private key are invalid or do not match: {error}"),
            )
        })?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Some(Arc::new(config)))
}

fn read_limited_tls_file(path: &Path, maximum: u64, name: &str) -> Result<Vec<u8>, io::Error> {
    let file = std::fs::File::open(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("{name} {} is unavailable", path.display()),
        )
    })?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{name} must be a regular file"),
        ));
    }
    let mut value = Vec::new();
    file.take(maximum + 1).read_to_end(&mut value)?;
    if value.is_empty() || value.len() as u64 > maximum {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{name} must be non-empty and no larger than {maximum} bytes"),
        ));
    }
    Ok(value)
}

fn invalid_tls_pem(error: rustls::pki_types::pem::Error) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("API TLS PEM is invalid: {error}"),
    )
}

fn validate_listener_security(
    api_address: SocketAddr,
    web: &WebArgs,
    api_tls_enabled: bool,
) -> Result<(), io::Error> {
    if listeners_conflict(api_address, web.admin_listen)
        || listeners_conflict(api_address, web.member_listen)
        || listeners_conflict(web.admin_listen, web.member_listen)
    {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "API, admin, and member listeners must use distinct addresses",
        ));
    }
    if (!web.admin_listen.ip().is_loopback() || !web.member_listen.ip().is_loopback())
        && !web.allow_insecure_http
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "admin and member HTTP listeners may leave loopback only when insecure LAN HTTP is explicitly enabled",
        ));
    }
    if !api_tls_enabled && !api_address.ip().is_loopback() && !web.allow_insecure_http {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the API may listen beyond loopback only with TLS or explicitly enabled insecure LAN HTTP",
        ));
    }
    if web.allow_insecure_http && web.secure_cookies {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "insecure LAN HTTP requires secure session cookies to be disabled",
        ));
    }
    Ok(())
}

fn listeners_conflict(left: SocketAddr, right: SocketAddr) -> bool {
    left.port() == right.port()
        && (left.ip() == right.ip() || left.ip().is_unspecified() || right.ip().is_unspecified())
}

const MAX_PENDING_TLS_HANDSHAKES: usize = 256;
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

type PendingTlsHandshake =
    Pin<Box<dyn Future<Output = Option<(TlsStream<TcpStream>, SocketAddr)>> + Send + 'static>>;

struct ApiTlsListener {
    listener: TcpListener,
    acceptor: TlsAcceptor,
    handshakes: FuturesUnordered<PendingTlsHandshake>,
}

impl axum::serve::Listener for ApiTlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            if self.handshakes.len() >= MAX_PENDING_TLS_HANDSHAKES {
                if let Some(Some(completed)) = self.handshakes.next().await {
                    return completed;
                }
                continue;
            }
            tokio::select! {
                completed = self.handshakes.next(), if !self.handshakes.is_empty() => {
                    if let Some(Some(completed)) = completed {
                        return completed;
                    }
                }
                accepted = self.listener.accept() => {
                    let (stream, address) = match accepted {
                        Ok(value) => value,
                        Err(error) => {
                            error!(error = %error, "API TCP accept failed");
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            continue;
                        }
                    };
                    let acceptor = self.acceptor.clone();
                    self.handshakes.push(Box::pin(async move {
                        match tokio::time::timeout(TLS_HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await {
                            Ok(Ok(stream)) => Some((stream, address)),
                            Ok(Err(error)) => {
                                debug!(peer = %address, error = %error, "API TLS handshake rejected");
                                None
                            }
                            Err(_) => {
                                debug!(peer = %address, "API TLS handshake timed out");
                                None
                            }
                        }
                    }));
                }
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

async fn serve_api(
    listener: TcpListener,
    state: ControlState,
    tls: Option<Arc<rustls::ServerConfig>>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> io::Result<()> {
    let shutdown_signal =
        async move { while !*shutdown.borrow() && shutdown.changed().await.is_ok() {} };
    if let Some(config) = tls {
        axum::serve(
            ApiTlsListener {
                listener,
                acceptor: TlsAcceptor::from(config),
                handshakes: FuturesUnordered::new(),
            },
            router(state),
        )
        .with_graceful_shutdown(shutdown_signal)
        .await
    } else {
        axum::serve(listener, router(state))
            .with_graceful_shutdown(shutdown_signal)
            .await
    }
}

async fn serve_control_surfaces(
    api_address: SocketAddr,
    web: &WebArgs,
    api_tls: Option<Arc<rustls::ServerConfig>>,
    state: ControlState,
    runtime: Option<(SocketAddr, aster_control::runtime_control::RuntimeControl)>,
) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = match runtime {
        Some((address, runtime)) => Some((TcpListener::bind(address).await?, runtime)),
        None => None,
    };
    let api_listener = TcpListener::bind(api_address).await?;
    let admin_listener = TcpListener::bind(web.admin_listen).await?;
    let member_listener = TcpListener::bind(web.member_listen).await?;
    info!(address = %api_address, tls = api_tls.is_some(), "aster-control API listening");
    info!(address = %web.admin_listen, assets = %web.admin_assets.display(), "aster-control admin UI listening");
    info!(address = %web.member_listen, assets = %web.member_assets.display(), "aster-control member UI listening");

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let lifecycle = state.request_lifecycle().clone();
    let audit_state = state.clone();
    let audit_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error) = audit_state.flush_pending_license_audits().await {
                tracing::warn!(
                    ?error,
                    "license audit remains durable and pending for retry"
                );
            }
        }
    });
    let recovery_state = state.clone();
    let recovery_task = tokio::spawn(async move {
        recovery_state.run_quota_recovery().await;
    });
    let shutdown_task = tokio::spawn(async move {
        tokio::select! {
            _ = shutdown_signal() => {},
            _ = lifecycle.wait_shutdown_started() => {},
        }
        lifecycle.begin_shutdown();
        lifecycle.wait_drained().await;
        let _ = shutdown_tx.send(true);
    });
    let api = serve_api(api_listener, state.clone(), api_tls, shutdown_rx.clone());
    let mut admin_shutdown = shutdown_rx.clone();
    let admin = axum::serve(admin_listener, web_router(state.clone(), &web.admin_assets))
        .with_graceful_shutdown(async move {
            while !*admin_shutdown.borrow() && admin_shutdown.changed().await.is_ok() {}
        });
    let mut runtime_shutdown = shutdown_rx.clone();
    let mut member_shutdown = shutdown_rx;
    let member = axum::serve(
        member_listener,
        web_router(state.clone(), &web.member_assets),
    )
    .with_graceful_shutdown(async move {
        while !*member_shutdown.borrow() && member_shutdown.changed().await.is_ok() {}
    });
    let runtime = async move {
        let stop = async move {
            while !*runtime_shutdown.borrow() && runtime_shutdown.changed().await.is_ok() {}
        };
        match runtime {
            Some((listener, runtime)) => {
                axum::serve(listener, runtime.router())
                    .with_graceful_shutdown(stop)
                    .await
            }
            None => {
                stop.await;
                Ok(())
            }
        }
    };
    let result = tokio::try_join!(api, admin, member, runtime).map(|_| ());
    shutdown_task.abort();
    audit_task.abort();
    recovery_task.abort();
    state.drain_licensed_mutations().await;
    Ok(result?)
}

async fn initialize_database(args: &DatabaseArgs) -> Result<(), Box<dyn std::error::Error>> {
    let args = args.resolved()?;
    match args.database_driver.unwrap_or(DatabaseDriver::Mariadb) {
        DatabaseDriver::Mariadb => {
            #[cfg(feature = "mariadb")]
            {
                open_mariadb(&args, true).await?;
                Ok(())
            }
            #[cfg(not(feature = "mariadb"))]
            {
                Err("this binary was built without MariaDB support".into())
            }
        }
        DatabaseDriver::Sqlcipher => {
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            {
                let key = read_exact_key(&args.sqlcipher_key_file, "SQLCipher key")?;
                aster_storage::SqlCipherStore::initialize(&args.sqlcipher_file, &key)?;
                Ok(())
            }
            #[cfg(not(any(feature = "sqlcipher", feature = "sqlite-dev")))]
            {
                Err("this binary was built without SQLCipher support".into())
            }
        }
    }
}

async fn inspect_database_migrations(
    args: &DatabaseArgs,
    profile: &InstallationProfile,
    installation_key: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "mariadb")]
    {
        use sha2::{Digest as _, Sha256};
        let args = args.resolved()?;
        if args.database_driver != Some(DatabaseDriver::Mariadb) {
            return Err("migration inspection requires the installed MariaDB configuration".into());
        }
        let config = mariadb_config(&args)?;
        let key = read_installation_key(installation_key)?;
        let mut digest = Sha256::new();
        digest.update(b"aster-customer-database-binding-v1");
        digest.update(key.as_ref());
        digest.update(profile.installation_id.as_bytes());
        let report = aster_storage::MariaDbStore::inspect_installed_migrations(
            &config,
            &profile.installation_id,
            &digest.finalize().into(),
        )
        .await?;
        println!("{}", serde_json::to_string(&report)?);
        Ok(())
    }
    #[cfg(not(feature = "mariadb"))]
    {
        let _ = (args, profile, installation_key);
        Err("this binary was built without MariaDB support".into())
    }
}

#[cfg(feature = "mariadb")]
async fn open_mariadb(
    args: &DatabaseArgs,
    initialize: bool,
) -> Result<aster_storage::MariaDbStore, Box<dyn std::error::Error>> {
    open_mariadb_with_policy(args, initialize, DatabaseOpenPolicy::Maintenance).await
}

#[cfg(feature = "mariadb")]
async fn open_mariadb_with_policy(
    args: &DatabaseArgs,
    initialize: bool,
    policy: DatabaseOpenPolicy,
) -> Result<aster_storage::MariaDbStore, Box<dyn std::error::Error>> {
    use sha2::{Digest as _, Sha256};
    let config = mariadb_config(args)?;
    let layout = default_layout();
    if !layout.marker_path().is_file() {
        if policy != DatabaseOpenPolicy::Maintenance {
            return Err(io::Error::other(
                "online migration requires an installed database binding",
            )
            .into());
        }
        return Ok(if initialize {
            aster_storage::MariaDbStore::initialize(&config).await?
        } else {
            aster_storage::MariaDbStore::open(&config).await?
        });
    }
    let profile = load_profile(&layout.installation_profile())?;
    let key = read_installation_key(&layout.installation_key())?;
    let mut digest = Sha256::new();
    digest.update(b"aster-customer-database-binding-v1");
    digest.update(key.as_ref());
    digest.update(profile.installation_id.as_bytes());
    Ok(aster_storage::MariaDbStore::open_installed_with_policy(
        &config,
        &profile.installation_id,
        &digest.finalize().into(),
        initialize,
        match policy {
            DatabaseOpenPolicy::Maintenance => aster_storage::MariaDbMigrationPolicy::Maintenance,
            DatabaseOpenPolicy::Online => aster_storage::MariaDbMigrationPolicy::Online,
            DatabaseOpenPolicy::Existing => aster_storage::MariaDbMigrationPolicy::Existing,
        },
    )
    .await?)
}

#[cfg(feature = "mariadb")]
fn mariadb_config(
    args: &DatabaseArgs,
) -> Result<aster_storage::MariaDbConfig, Box<dyn std::error::Error>> {
    Ok(aster_storage::MariaDbConfig {
        host: args.database_host.clone(),
        port: args.database_port,
        database: args.database_name.clone(),
        username: args.database_user.clone(),
        password: read_text_secret(&args.database_password_file, "MariaDB password")?,
        tls: args.database_tls,
        ca_certificate: args.database_ca_certificate.clone(),
        max_connections: args.database_max_connections,
    })
}

#[cfg(feature = "mariadb")]
fn read_text_secret(path: &Path, name: &str) -> Result<String, std::io::Error> {
    let bytes = std::fs::read(path)?;
    let secret = String::from_utf8(bytes).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{name} must be valid UTF-8"),
        )
    })?;
    let secret = secret.trim_end_matches(['\r', '\n']).to_owned();
    if secret.is_empty() || secret.contains('\0') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{name} must not be empty or contain NUL"),
        ));
    }
    Ok(secret)
}

fn read_exact_key(path: &Path, name: &str) -> Result<[u8; 32], std::io::Error> {
    let bytes = std::fs::read(path)?;
    bytes.try_into().map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{name} must contain exactly 32 bytes"),
        )
    })
}

fn load_runner_task_issuer(args: &RunnerTaskKeyArgs) -> Result<RunnerTaskIssuer, std::io::Error> {
    validate_key_id(&args.runner_task_key_id)?;
    let seed = Zeroizing::new(read_exact_key(
        &args.runner_task_key_file,
        "Runner task signing key",
    )?);
    let issuer = RunnerTaskIssuer::new(
        args.runner_task_key_id.clone(),
        SigningKey::from_bytes(&seed),
    );
    Ok(issuer)
}

fn initialize_secret_key(path: &Path) -> Result<(), std::io::Error> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "key path has no parent")
    })?;
    std::fs::create_dir_all(parent)?;
    if path.exists() {
        read_exact_key(path, "Runner task signing key")?;
        return Ok(());
    }
    let mut seed = Zeroizing::new([0_u8; 32]);
    getrandom::fill(seed.as_mut())
        .map_err(|_| std::io::Error::other("operating-system randomness unavailable"))?;
    let mut temporary = NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        temporary
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    temporary.write_all(seed.as_ref())?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(path)
        .map_err(|error| error.error)?;
    Ok(())
}

fn export_runner_task_keys(
    key_id: &str,
    key_file: &Path,
    target: &Path,
) -> Result<(), std::io::Error> {
    let encoded = runner_task_public_keys_document(key_id, key_file)?;
    atomic_write_public(target, &encoded)
}

fn runner_task_public_keys_document(
    key_id: &str,
    key_file: &Path,
) -> Result<Vec<u8>, std::io::Error> {
    validate_key_id(key_id)?;
    let seed = Zeroizing::new(read_exact_key(key_file, "Runner task signing key")?);
    let signing_key = SigningKey::from_bytes(&seed);
    let public_key = signing_key
        .verifying_key()
        .to_public_key_der()
        .map_err(|_| std::io::Error::other("Runner task public key encoding failed"))?;
    let value = serde_json::json!({
        "schema": "aster.runner-task-keys.v1",
        "keys": [{
            "key_id": key_id,
            "public_key_spki": URL_SAFE_NO_PAD.encode(public_key.as_bytes()),
        }],
    });
    let mut encoded = serde_json::to_vec_pretty(&value)
        .map_err(|_| std::io::Error::other("Runner task public key JSON encoding failed"))?;
    encoded.push(b'\n');
    Ok(encoded)
}

fn verify_runner_task_public_keys(
    key_id: &str,
    key_file: &Path,
    public_file: &Path,
) -> Result<(), std::io::Error> {
    let expected = runner_task_public_keys_document(key_id, key_file)?;
    let actual = std::fs::read(public_file)?;
    if actual != expected {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Runner task public key file does not match the signing key",
        ));
    }
    Ok(())
}

fn atomic_write_public(path: &Path, value: &[u8]) -> Result<(), std::io::Error> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "public key path has no parent",
        )
    })?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = NamedTempFile::new_in(parent)?;
    temporary.write_all(value)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn atomic_write_new(path: &Path, value: &[u8], unix_mode: u32) -> Result<(), std::io::Error> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "output path has no parent directory",
        )
    })?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        temporary
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(unix_mode))?;
    }
    #[cfg(not(unix))]
    let _ = unix_mode;
    temporary.write_all(value)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(path)
        .map_err(|error| error.error)?;
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalRunnerIdentityDocument {
    schema: String,
    runner_id: String,
    credential: String,
}

fn pending_output_path(output: &Path) -> PathBuf {
    let mut value = output.as_os_str().to_os_string();
    value.push(".pending");
    PathBuf::from(value)
}

fn encode_local_runner_identity(
    identity: &aster_control::BootstrapLocalRunnerIdentity,
) -> Result<Vec<u8>, serde_json::Error> {
    let mut encoded = serde_json::to_vec_pretty(&serde_json::json!({
        "schema": "aster.runner-identity.v1",
        "runner_id": &identity.runner_id,
        "credential": identity.credential.as_str(),
    }))?;
    encoded.push(b'\n');
    Ok(encoded)
}

fn read_local_runner_identity(
    path: &Path,
) -> Result<aster_control::BootstrapLocalRunnerIdentity, std::io::Error> {
    let source = std::fs::read(path)?;
    if source.is_empty() || source.len() > 4096 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "local Runner identity file size is invalid",
        ));
    }
    let document: LocalRunnerIdentityDocument = serde_json::from_slice(&source).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "local Runner identity file is invalid",
        )
    })?;
    if document.schema != "aster.runner-identity.v1" {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "local Runner identity schema is invalid",
        ));
    }
    Ok(aster_control::BootstrapLocalRunnerIdentity {
        runner_id: document.runner_id,
        credential: Zeroizing::new(document.credential),
    })
}

fn finish_local_runner_identity(
    pending: &Path,
    output: &Path,
    identity: &aster_control::BootstrapLocalRunnerIdentity,
) -> Result<(), std::io::Error> {
    let matches = |candidate: &aster_control::BootstrapLocalRunnerIdentity| {
        candidate.runner_id == identity.runner_id
            && candidate.credential.as_str() == identity.credential.as_str()
    };
    if output.exists() {
        if !matches(&read_local_runner_identity(output)?) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "local Runner identity output belongs to another Runner",
            ));
        }
        if pending.exists() {
            if !matches(&read_local_runner_identity(pending)?) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "pending local Runner identity does not match the final output",
                ));
            }
            std::fs::remove_file(pending)?;
        }
        return Ok(());
    }
    if !matches(&read_local_runner_identity(pending)?) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "pending local Runner identity changed during initialization",
        ));
    }
    std::fs::rename(pending, output)?;
    #[cfg(unix)]
    if let Some(parent) = output.parent() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn validate_key_id(value: &str) -> Result<(), std::io::Error> {
    if !(3..=128).contains(&value.len())
        || !value.bytes().all(|value| {
            value.is_ascii_alphanumeric() || matches!(value, b'_' | b'-' | b'.' | b':')
        })
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Runner task key id is invalid",
        ));
    }
    Ok(())
}

fn read_installation_key(path: &Path) -> Result<[u8; 32], std::io::Error> {
    read_exact_key(path, "installation key")
}

fn read_password(path: &Path) -> Result<Zeroizing<Vec<u8>>, std::io::Error> {
    normalize_password(Zeroizing::new(std::fs::read(path)?))
}

fn read_password_stdin() -> Result<Zeroizing<Vec<u8>>, std::io::Error> {
    let mut value = Zeroizing::new(Vec::new());
    std::io::stdin().take(1026).read_to_end(&mut value)?;
    normalize_password(value)
}

fn normalize_password(value: Zeroizing<Vec<u8>>) -> Result<Zeroizing<Vec<u8>>, std::io::Error> {
    let start = value
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(start, |position| position + 1);
    let password = Zeroizing::new(value[start..end].to_vec());
    if !(12..=1024).contains(&password.len()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "password must contain between 12 and 1024 non-whitespace bytes",
        ));
    }
    Ok(password)
}

fn debug_error(error: impl std::fmt::Debug) -> std::io::Error {
    std::io::Error::other(format!("{error:?}"))
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    result = tokio::signal::ctrl_c() => {
                        if let Err(error) = result {
                            error!(error = %error, "Control shutdown signal failed");
                        }
                    }
                    _ = terminate.recv() => {}
                }
            }
            Err(error) => error!(error = %error, "Control shutdown signal registration failed"),
        }
    }
    #[cfg(not(unix))]
    if let Err(error) = tokio::signal::ctrl_c().await {
        error!(error = %error, "Control shutdown signal failed");
    }
}

#[cfg(test)]
mod tests {
    use aster_machine_identity::RawMachineFactors;
    use aster_runner_protocol::TrustedTaskKeys;
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt as _;
    use tempfile::tempdir;
    use time::macros::datetime;
    use tower::ServiceExt as _;

    use super::*;

    #[test]
    fn machine_profile_verifier_accepts_only_this_hosts_identity() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("installation.json");
        let factors = match current_machine_factors() {
            Ok(factors) => factors,
            Err(error) => {
                let profile = create_profile(
                    "installation_unavailable_host",
                    &RawMachineFactors {
                        dmi_product_uuid: "test-dmi".to_owned(),
                        machine_id: "test-machine".to_owned(),
                    },
                )
                .unwrap();
                std::fs::write(&path, serde_json::to_vec(&profile).unwrap()).unwrap();
                assert_eq!(verify_machine_profile(&path), Err(error));
                return;
            }
        };
        let profile = create_profile("installation_actual_host", &factors).unwrap();
        std::fs::write(&path, serde_json::to_vec(&profile).unwrap()).unwrap();
        assert_eq!(verify_machine_profile(&path), Ok(profile));
    }

    /// The CLI entry point runs on a thread with an explicit stack
    /// (`ENTRY_STACK_BYTES`) because unoptimized builds need far more than the
    /// 1 MiB Windows reserves for the main thread. On top of that, `main` keeps the
    /// dispatch state machine on the heap with `Box::pin`; this budget records the
    /// current debug-build measurement (52 KiB) so a new subcommand that multiplies
    /// that state is noticed rather than silently consuming the entry stack.
    #[test]
    fn dispatch_state_machine_stays_within_its_heap_budget() {
        const DISPATCH_FUTURE_BUDGET: usize = 96 * 1024;
        let cli = Cli::try_parse_from(["aster-control", "initialize-installation"]).unwrap();
        let bytes = std::mem::size_of_val(&run(cli));
        assert!(
            bytes <= DISPATCH_FUTURE_BUDGET,
            "Control dispatch state machine grew to {bytes} bytes; split the subcommand \
             handler into its own function and keep the entry dispatch boxed"
        );
    }

    #[tokio::test]
    async fn privileged_license_install_rejects_a_copied_machine_profile() {
        let directory = tempdir().unwrap();
        let profile_path = directory.path().join("installation.json");
        let target = directory.path().join("license.json");
        let profile = create_profile(
            "installation_copied_runtime",
            &RawMachineFactors {
                dmi_product_uuid: "test-only-foreign-machine".to_owned(),
                machine_id: "test-only-foreign-os".to_owned(),
            },
        )
        .unwrap();
        std::fs::write(&profile_path, serde_json::to_vec(&profile).unwrap()).unwrap();
        let expected = current_machine_factors()
            .map(|_| MachineIdentityError::MachineMismatch)
            .unwrap_or_else(|error| error);
        let args = [
            "aster-control",
            "install-license",
            "--installation-profile",
            profile_path.to_str().unwrap(),
            "--license-file",
            target.to_str().unwrap(),
            "--source",
            "test-only-nonexistent-license.json",
        ];
        let error = run(Cli::try_parse_from(args).unwrap()).await.unwrap_err();
        assert_eq!(
            error.downcast_ref::<MachineIdentityError>(),
            Some(&expected)
        );
        assert!(!target.exists());
    }

    #[test]
    fn upgrade_audit_cli_is_closed_to_a_fixed_outcome_and_version_target() {
        let bound = Cli::try_parse_from([
            "aster-control",
            "record-upgrade-audit",
            "--outcome",
            "succeeded",
            "--target-version",
            "2.1.0",
            "--job-id",
            "upgrade-one",
        ])
        .unwrap();
        assert!(
            matches!(bound.command, Command::RecordUpgradeAudit { job_id: Some(ref id), .. } if id == "upgrade-one")
        );
        let cli = Cli::try_parse_from([
            "aster-control",
            "record-upgrade-audit",
            "--outcome",
            "succeeded",
            "--target-version",
            "1.1.1",
            "--database-driver",
            "sqlcipher",
        ])
        .expect("parse upgrade audit command");
        let Command::RecordUpgradeAudit {
            outcome,
            target_version,
            database,
            ..
        } = cli.command
        else {
            panic!("expected upgrade audit command");
        };
        assert_eq!(outcome, UpgradeAuditOutcome::Succeeded);
        assert_eq!(target_version, "1.1.1");
        assert_eq!(database.database_driver, Some(DatabaseDriver::Sqlcipher));
        assert!(
            Cli::try_parse_from([
                "aster-control",
                "record-upgrade-audit",
                "--outcome",
                "invented",
                "--target-version",
                "1.1.1",
            ])
            .is_err()
        );
    }

    #[test]
    fn bundled_free_license_preflight_accepts_a_bounded_bootstrap_window() {
        let cli = Cli::try_parse_from([
            "aster-control",
            "verify-bundled-free-license",
            "--source",
            "free-license.json",
            "--minimum-valid-for-seconds",
            "900",
        ])
        .expect("parse bundled license preflight");
        let Command::VerifyBundledFreeLicense {
            source,
            minimum_valid_for_seconds,
        } = cli.command
        else {
            panic!("expected bundled license preflight command");
        };
        assert_eq!(source, PathBuf::from("free-license.json"));
        assert_eq!(minimum_valid_for_seconds, 900);
        assert!(
            Cli::try_parse_from([
                "aster-control",
                "verify-bundled-free-license",
                "--source",
                "free-license.json",
                "--minimum-valid-for-seconds",
                "65536",
            ])
            .is_err()
        );
    }

    fn create_test_tls_pair(directory: &Path) -> (PathBuf, PathBuf, Vec<u8>) {
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["aster-control-test.invalid".to_owned()])
                .expect("generate TLS test pair");
        let certificate_bytes = cert.pem().into_bytes();
        let certificate = directory.join("server.crt");
        let private_key = directory.join("server.key");
        std::fs::write(&certificate, &certificate_bytes).expect("write TLS test certificate");
        std::fs::write(&private_key, signing_key.serialize_pem()).expect("write TLS test key");
        (certificate, private_key, certificate_bytes)
    }

    #[test]
    fn initializes_an_idempotent_non_overwriting_task_key_and_exports_its_public_key() {
        let directory = tempdir().expect("temp directory");
        let key_file = directory.path().join("keys/runner-task.key");
        let public_file = directory.path().join("runner-task-keys.json");
        initialize_secret_key(&key_file).expect("initialize task key");
        let seed = std::fs::read(&key_file).expect("read task key");
        assert_eq!(seed.len(), 32);
        assert!(seed.iter().any(|value| *value != 0));
        initialize_secret_key(&key_file).expect("recheck existing task key");
        assert_eq!(std::fs::read(&key_file).expect("reread task key"), seed);

        export_runner_task_keys("runner-task-test-01", &key_file, &public_file)
            .expect("export public key");
        verify_runner_task_public_keys("runner-task-test-01", &key_file, &public_file)
            .expect("verify exported public key");
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&public_file).expect("read public key file"))
                .expect("parse public key file");
        assert_eq!(value["schema"], "aster.runner-task-keys.v1");
        let mut keys = TrustedTaskKeys::new();
        keys.insert_spki_base64url(
            value["keys"][0]["key_id"].as_str().expect("public key id"),
            value["keys"][0]["public_key_spki"]
                .as_str()
                .expect("public key SPKI"),
        )
        .expect("load exported public key");

        std::fs::write(&public_file, b"{}\n").expect("tamper public key file");
        assert!(
            verify_runner_task_public_keys("runner-task-test-01", &key_file, &public_file).is_err()
        );
    }

    #[test]
    fn generates_a_strict_offline_license_request_without_overwriting_output() {
        let profile = create_profile(
            "installation_request_test",
            &RawMachineFactors {
                dmi_product_uuid: "dmi-request-test".to_owned(),
                machine_id: "machine-request-test".to_owned(),
            },
        )
        .expect("create profile");
        let request =
            generate_license_request(&profile, "x86_64", datetime!(2026-08-28 12:34:56.789 UTC))
                .expect("generate request");
        assert_eq!(request.schema, LICENSE_REQUEST_SCHEMA);
        assert_eq!(request.platform, Platform::current().id());
        assert_eq!(request.architecture, "amd64");
        assert_eq!(request.generated_at, "2026-08-28T12:34:56.789Z");
        assert_eq!(request.installation_id, profile.installation_id);
        assert_eq!(request.license_schema, LICENSE_SCHEMA);
        assert_eq!(request.capability_catalog_version, CATALOG_VERSION);
        assert_eq!(request.quota_policy_version, QUOTA_POLICY_VERSION);
        aster_license_core::request_v2::parse_request(
            &serde_json::to_vec(&request).expect("serialize request"),
        )
        .expect("parse generated request");

        let directory = tempdir().expect("temp directory");
        let output = directory.path().join("license-request.json");
        atomic_write_new(&output, b"first", 0o640).expect("write first request");
        assert!(atomic_write_new(&output, b"second", 0o640).is_err());
        assert_eq!(std::fs::read(output).expect("read request"), b"first");
    }

    #[test]
    fn runtime_control_is_opt_in_and_requires_complete_private_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("runtime-token");
        std::fs::write(&path, [31; 32]).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        }
        let mut args = RuntimeControlArgs::default();
        assert!(args.load().unwrap().is_none());
        args.runtime_candidate = true;
        assert!(args.load().is_err());
        args.runtime_candidate = false;
        args.runtime_listen = Some("127.0.0.1:12000".parse().unwrap());
        assert!(args.load().is_err());
        args.runtime_token_file = Some(path.clone());
        args.runtime_slot = Some("blue".into());
        assert_eq!(
            args.load().unwrap().unwrap().slot,
            aster_upgrade_core::ReleaseSlot::Blue
        );
        for address in ["0.0.0.0:12000", "192.0.2.1:12000", "[::]:12000"] {
            args.runtime_listen = Some(address.parse().unwrap());
            assert!(args.load().is_err());
        }
        args.runtime_listen = Some("[::1]:12000".parse().unwrap());
        assert!(args.load().unwrap().is_some());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            for mode in [0o660, 0o644] {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
                assert!(args.load().is_err());
            }
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
            let link = directory.path().join("runtime-token-link");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            args.runtime_token_file = Some(link);
            assert!(args.load().is_err());
            args.runtime_token_file = Some(path.clone());
        }
        std::fs::write(path, [31; 33]).unwrap();
        assert!(args.load().is_err());
    }

    #[test]
    fn local_runner_identity_publication_recovers_from_a_pending_file() {
        let directory = tempdir().expect("temp directory");
        let output = directory.path().join("runner/identity.json");
        let pending = pending_output_path(&output);
        let identity = aster_control::BootstrapLocalRunnerIdentity {
            runner_id: "runner_0123456789abcdef0123456789abcdef".to_owned(),
            credential: Zeroizing::new(format!("arr_{}", "A".repeat(64))),
        };
        atomic_write_new(
            &pending,
            &encode_local_runner_identity(&identity).expect("encode identity"),
            0o600,
        )
        .expect("write recoverable identity");

        finish_local_runner_identity(&pending, &output, &identity)
            .expect("publish pending identity");
        assert!(output.is_file());
        assert!(!pending.exists());
        let recovered = read_local_runner_identity(&output).expect("read published identity");
        assert_eq!(recovered.runner_id, identity.runner_id);
        assert_eq!(recovered.credential.as_str(), identity.credential.as_str());
        finish_local_runner_identity(&pending, &output, &identity)
            .expect("repeat publication after an uncertain response");
    }

    #[test]
    fn listener_security_rejects_plaintext_remote_api_and_remote_web_surfaces() {
        let mut web = WebArgs {
            admin_listen: "127.0.0.1:11082".parse().expect("admin address"),
            member_listen: "127.0.0.1:11081".parse().expect("member address"),
            admin_assets: PathBuf::from("admin"),
            member_assets: PathBuf::from("member"),
            allow_insecure_http: false,
            secure_cookies: true,
        };
        assert!(
            validate_listener_security(
                "127.0.0.1:11080".parse().expect("local API address"),
                &web,
                false,
            )
            .is_ok()
        );
        assert!(
            validate_listener_security(
                "0.0.0.0:11080".parse().expect("remote API address"),
                &web,
                false,
            )
            .is_err()
        );
        assert!(
            validate_listener_security(
                "0.0.0.0:11080".parse().expect("TLS API address"),
                &web,
                true,
            )
            .is_ok()
        );

        web.admin_listen = "0.0.0.0:11082".parse().expect("remote admin address");
        assert!(
            validate_listener_security(
                "127.0.0.1:11080".parse().expect("local API address"),
                &web,
                true,
            )
            .is_err()
        );
        web.member_listen = "0.0.0.0:11081".parse().expect("remote member address");
        web.allow_insecure_http = true;
        web.secure_cookies = false;
        assert!(
            validate_listener_security(
                "0.0.0.0:11080".parse().expect("LAN API address"),
                &web,
                false,
            )
            .is_ok()
        );
    }

    #[test]
    fn api_tls_requires_a_valid_single_certificate_and_private_key_pair() {
        let directory = tempdir().expect("temp directory");
        let certificate = directory.path().join("server.crt");
        assert!(
            load_api_tls(&ApiTlsArgs {
                api_tls_certificate: Some(certificate),
                api_tls_private_key: None,
            })
            .is_err()
        );
        assert!(
            load_api_tls(&ApiTlsArgs {
                api_tls_certificate: None,
                api_tls_private_key: Some(directory.path().join("server.key")),
            })
            .is_err()
        );

        let (fixture_certificate, fixture_private_key, _) = create_test_tls_pair(directory.path());
        assert!(
            load_api_tls(&ApiTlsArgs {
                api_tls_certificate: Some(fixture_certificate.clone()),
                api_tls_private_key: Some(fixture_private_key.clone()),
            })
            .expect("load matching TLS pair")
            .is_some()
        );

        let duplicate_private_key = directory.path().join("duplicate.key");
        let key = std::fs::read(&fixture_private_key).expect("read TLS test key");
        let mut duplicate = key.clone();
        duplicate.extend_from_slice(&key);
        std::fs::write(&duplicate_private_key, duplicate).expect("write duplicate TLS key");
        assert!(
            load_api_tls(&ApiTlsArgs {
                api_tls_certificate: Some(fixture_certificate),
                api_tls_private_key: Some(duplicate_private_key),
            })
            .is_err()
        );
    }

    #[tokio::test]
    async fn retired_lifecycle_stops_all_surfaces_without_an_os_signal() {
        let directory = tempdir().unwrap();
        let web = WebArgs {
            admin_listen: "127.0.0.1:0".parse().unwrap(),
            member_listen: "127.0.0.1:0".parse().unwrap(),
            admin_assets: directory.path().join("admin"),
            member_assets: directory.path().join("member"),
            allow_insecure_http: true,
            secure_cookies: false,
        };
        let state = ControlState::new("test", None);
        let runtime = aster_control::runtime_control::RuntimeControl::new(
            state.clone(),
            "installation-test".into(),
            aster_upgrade_core::ReleaseSlot::Blue,
            &[29; 32],
        )
        .unwrap();
        state.request_lifecycle().begin_drain();
        assert!(state.request_lifecycle().retire_drained(1));
        // Even when retirement precedes the serving task's subscription, the
        // durable in-process state must wake the same path used for SIGTERM.
        tokio::time::timeout(
            Duration::from_secs(3),
            serve_control_surfaces(
                "127.0.0.1:0".parse().unwrap(),
                &web,
                None,
                state,
                Some(("127.0.0.1:0".parse().unwrap(), runtime)),
            ),
        )
        .await
        .expect("retired surfaces remained alive")
        .unwrap();
    }

    #[tokio::test]
    async fn api_tls_serves_https_and_does_not_accept_plaintext_http() {
        let directory = tempdir().expect("TLS temp directory");
        let (certificate_path, private_key_path, certificate_bytes) =
            create_test_tls_pair(directory.path());
        let config = load_api_tls(&ApiTlsArgs {
            api_tls_certificate: Some(certificate_path),
            api_tls_private_key: Some(private_key_path),
        })
        .expect("load TLS test configuration")
        .expect("TLS is configured");
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind TLS API");
        let address = listener.local_addr().expect("TLS API address");
        let (_shutdown, shutdown) = tokio::sync::watch::channel(false);
        let server = tokio::spawn(serve_api(
            listener,
            ControlState::new("0.1.0", None),
            Some(config),
            shutdown,
        ));

        let root =
            reqwest::Certificate::from_pem(&certificate_bytes).expect("parse TLS test certificate");
        let client = reqwest::Client::builder()
            .add_root_certificate(root)
            .resolve("aster-control-test.invalid", address)
            .timeout(Duration::from_secs(2))
            .build()
            .expect("build HTTPS client");
        let stalled_handshake = TcpStream::connect(address)
            .await
            .expect("open stalled TLS handshake");
        let response = client
            .get(format!(
                "https://aster-control-test.invalid:{}/healthz",
                address.port()
            ))
            .send()
            .await
            .expect("HTTPS health request");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        drop(stalled_handshake);

        use tokio_tungstenite::{
            Connector, client_async_tls_with_config,
            tungstenite::{Error as WebSocketError, client::IntoClientRequest as _},
        };
        let mut roots = rustls::RootCertStore::empty();
        for certificate in CertificateDer::pem_slice_iter(&certificate_bytes) {
            roots
                .add(certificate.expect("parse WSS test certificate"))
                .expect("trust WSS test certificate");
        }
        let provider = rustls::crypto::ring::default_provider();
        let client_config = rustls::ClientConfig::builder_with_provider(Arc::new(provider))
            .with_safe_default_protocol_versions()
            .expect("safe WSS protocol versions")
            .with_root_certificates(roots)
            .with_no_client_auth();
        let request = format!(
            "wss://aster-control-test.invalid:{}/api/runner/channel",
            address.port()
        )
        .into_client_request()
        .expect("build WSS request");
        let socket = TcpStream::connect(address).await.expect("connect WSS TCP");
        let websocket = client_async_tls_with_config(
            request,
            socket,
            None,
            Some(Connector::Rustls(Arc::new(client_config))),
        )
        .await;
        match websocket {
            Err(WebSocketError::Http(response)) => {
                assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
                assert_eq!(
                    response
                        .headers()
                        .get("X-Aster-Error-Number")
                        .expect("fixed WSS license error"),
                    "51001"
                );
            }
            Err(error) => panic!("WSS reached an unexpected transport failure: {error}"),
            Ok(_) => panic!("unlicensed WSS connection unexpectedly upgraded"),
        }

        let plaintext = reqwest::get(format!("http://{address}/healthz")).await;
        assert!(plaintext.is_err());
        server.abort();
    }

    #[tokio::test]
    async fn static_spa_fallback_never_masks_unknown_api_routes() {
        let directory = tempdir().expect("temp directory");
        let admin = directory.path().join("admin");
        let member = directory.path().join("member");
        std::fs::create_dir_all(&admin).expect("create admin assets");
        std::fs::create_dir_all(&member).expect("create member assets");
        std::fs::write(admin.join("index.html"), b"<html>admin-spa</html>")
            .expect("write admin index");
        std::fs::write(member.join("index.html"), b"<html>member-spa</html>")
            .expect("write member index");
        let member_docs = member.join("docs/zh-cn");
        std::fs::create_dir_all(&member_docs).expect("create member documentation assets");
        std::fs::write(member_docs.join("index.html"), b"<html>member-docs</html>")
            .expect("write member documentation index");
        let web = WebArgs {
            admin_listen: "127.0.0.1:11082".parse().expect("admin address"),
            member_listen: "127.0.0.1:11081".parse().expect("member address"),
            admin_assets: admin.clone(),
            member_assets: member,
            allow_insecure_http: false,
            secure_cookies: true,
        };
        validate_web_assets(&web).expect("valid asset roots");

        let response = web_router(ControlState::new("0.1.0", None), &admin)
            .oneshot(
                Request::builder()
                    .uri("/overview")
                    .body(Body::empty())
                    .expect("SPA request"),
            )
            .await
            .expect("SPA response");
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("collect SPA response")
            .to_bytes();
        assert!(body.windows(9).any(|value| value == b"admin-spa"));

        let response = web_router(ControlState::new("0.1.0", None), &web.member_assets)
            .oneshot(
                Request::builder()
                    .uri("/docs/zh-cn/")
                    .body(Body::empty())
                    .expect("documentation request"),
            )
            .await
            .expect("documentation response");
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("collect documentation response")
            .to_bytes();
        assert!(body.windows(11).any(|value| value == b"member-docs"));

        for prefix in ["/api/not-a-real-route", "/v1/not-a-real-route"] {
            for method in ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"] {
                let response = web_router(ControlState::new("0.1.0", None), &admin)
                    .oneshot(
                        Request::builder()
                            .method(method)
                            .uri(prefix)
                            .body(Body::empty())
                            .expect("unknown API request"),
                    )
                    .await
                    .expect("unknown API response");
                assert_eq!(
                    response.status(),
                    axum::http::StatusCode::NOT_FOUND,
                    "{method} {prefix} must not return the application shell"
                );
            }
        }

        let response = web_router(ControlState::new("0.1.0", None), &admin)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/overview")
                    .body(Body::empty())
                    .expect("non-read SPA request"),
            )
            .await
            .expect("non-read SPA response");
        assert_eq!(
            response.status(),
            axum::http::StatusCode::METHOD_NOT_ALLOWED
        );
    }
    #[tokio::test]
    async fn web_and_api_requests_share_one_drain_registry_without_double_counting() {
        let directory = tempdir().unwrap();
        std::fs::write(directory.path().join("index.html"), b"<html>member</html>").unwrap();
        let state = ControlState::new("0.1.0", None);
        let app = web_router(state.clone(), directory.path());
        let html = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/overview")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(html.status(), axum::http::StatusCode::OK);
        assert_eq!(state.request_lifecycle().snapshot().in_flight, 1);
        let api = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/public/license-state")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(api.status(), axum::http::StatusCode::OK);
        assert_eq!(state.request_lifecycle().snapshot().in_flight, 2);
        state.request_lifecycle().begin_drain();
        let rejected = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/models")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            rejected.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
        let error: serde_json::Value =
            serde_json::from_slice(&rejected.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(error["error"]["code"], "BACKEND_INSTANCE_DRAINING");
        let health = app
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(health.status(), axum::http::StatusCode::OK);
        assert_eq!(state.request_lifecycle().snapshot().in_flight, 2);
        drop(html);
        drop(api);
        state.request_lifecycle().wait_drained().await;
    }
}
