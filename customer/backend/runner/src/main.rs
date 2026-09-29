#![forbid(unsafe_code)]

mod tasks;
use tasks::{TaskExecution, TaskSupervisor};

use std::{
    collections::BTreeSet,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::Duration as StdDuration,
};

use aster_install_layout::InstallLayout;
mod managed_release;
use aster_runner_protocol::{
    ControlToRunner, CredentialRefreshResult, CredentialRefreshTask, PingFrame,
    RUNNER_PROTOCOL_VERSION, ReplayCache, ResponseHeader, RunnerHeartbeat, RunnerHello,
    RunnerToControl, TaskChunk, TaskCommand, TaskFailure, TaskFrame, TaskLifecycle,
    TaskResponseStarted, TaskResult, TrustedTaskKeys, UpstreamHttpRequest, decode_base64url,
    decode_control_frame, encode_base64url, encode_runner_frame, verify_task_ticket,
};
use aster_upgrade_core::ReleaseSlot;
use clap::{Args, Parser, Subcommand};
use futures_util::{SinkExt as _, StreamExt as _};
use reqwest::{Client, Method, StatusCode, Url, header::HeaderName};
use rustls::pki_types::{
    CertificateDer,
    pem::{PemObject as _, SectionKind},
};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite::{
    Connector, connect_async_tls_with_config,
    tungstenite::{
        Message,
        client::IntoClientRequest as _,
        http::{HeaderValue, header::AUTHORIZATION},
    },
};
use tracing::{error, info, warn};
use zeroize::{Zeroize, Zeroizing};

const MAX_TASK_PAYLOAD_BYTES: usize = 32 * 1024 * 1024;
const MAX_RESPONSE_HEADER_VALUE_BYTES: usize = 8 * 1024;
const MAX_CREDENTIAL_REFRESH_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_CA_BUNDLE_BYTES: u64 = 1024 * 1024;

mod task_execution;
use task_execution::TaskExecutionPermit;

#[derive(Debug, Parser)]
#[command(name = "aster-runner", version, about = "Aster Team outbound Runner")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Enroll(EnrollArgs),
    Serve(RunnerArgs),
    Preflight(RunnerArgs),
    /// Run a local Runner bound to one release slot (Linux amd64 only).
    ServeSlot(SlotRunnerArgs),
    /// Validate one local slot without connecting or executing tasks.
    PreflightSlot(SlotRunnerArgs),
}

#[derive(Clone, Debug, Args)]
struct EnrollArgs {
    #[arg(long)]
    control_url: String,
    #[arg(long)]
    token_file: PathBuf,
    #[arg(long, default_value_os_t = default_runner_identity())]
    identity_file: PathBuf,
    #[arg(long, default_value_os_t = default_runner_task_keys())]
    task_keys_file: PathBuf,
    #[arg(long, env = "ASTER_RUNNER_CONTROL_CA_CERTIFICATE")]
    control_ca_certificate: Option<PathBuf>,
    #[arg(
        long,
        env = "ASTER_RUNNER_ALLOW_INSECURE_HTTP",
        default_value_t = false,
        action = clap::ArgAction::Set
    )]
    allow_insecure_http: bool,
    #[arg(long, default_value_t = 4)]
    max_inflight: u32,
}

#[derive(Clone, Debug, Args)]
struct RunnerArgs {
    #[arg(long)]
    control_wss: String,
    #[arg(long, default_value_os_t = default_runner_identity())]
    identity_file: PathBuf,
    #[arg(long, default_value_os_t = default_runner_task_keys())]
    task_keys_file: PathBuf,
    #[arg(long, env = "ASTER_RUNNER_CONTROL_CA_CERTIFICATE")]
    control_ca_certificate: Option<PathBuf>,
    #[arg(long, env = "ASTER_RUNNER_UPSTREAM_CA_CERTIFICATE")]
    upstream_ca_certificate: Option<PathBuf>,
    #[arg(
        long,
        env = "ASTER_RUNNER_ALLOW_INSECURE_HTTP",
        default_value_t = false,
        action = clap::ArgAction::Set
    )]
    allow_insecure_http: bool,
    #[arg(long, default_value_t = 4)]
    max_inflight: u32,
    #[arg(long, default_value_t = 10)]
    heartbeat_seconds: u64,
    #[arg(long, required = true)]
    allowed_upstream_host: Vec<String>,
}

#[derive(Clone, Debug, Args)]
struct SlotRunnerArgs {
    #[arg(long, value_parser = parse_runner_slot)]
    slot: ReleaseSlot,
    #[arg(long, env = "ASTER_RUNNER_UPSTREAM_CA_CERTIFICATE")]
    upstream_ca_certificate: Option<PathBuf>,
    #[arg(long, env = "ASTER_RUNNER_MAX_INFLIGHT", default_value_t = 4)]
    max_inflight: u32,
    #[arg(long, env = "ASTER_RUNNER_HEARTBEAT_SECONDS", default_value_t = 10)]
    heartbeat_seconds: u64,
    #[arg(long, required = true)]
    allowed_upstream_host: Vec<String>,
}

fn parse_runner_slot(value: &str) -> Result<ReleaseSlot, String> {
    match value {
        "blue" => Ok(ReleaseSlot::Blue),
        "green" => Ok(ReleaseSlot::Green),
        _ => Err("Runner slot must be blue or green".to_owned()),
    }
}

impl SlotRunnerArgs {
    fn into_runner_args(self, layout: &InstallLayout) -> RunnerArgs {
        // Slot traffic bypasses the public Caddy configuration. Never fall back to
        // the shared identity or trust file when a slot has not been provisioned.
        RunnerArgs {
            control_wss: format!(
                "ws://127.0.0.1:{}/api/runner/channel",
                self.slot.ports().api
            ),
            identity_file: layout.runner_slot_identity(self.slot.id()),
            task_keys_file: layout.runner_slot_task_keys(self.slot.id()),
            control_ca_certificate: None,
            upstream_ca_certificate: self.upstream_ca_certificate,
            allow_insecure_http: true,
            max_inflight: self.max_inflight,
            heartbeat_seconds: self.heartbeat_seconds,
            allowed_upstream_host: self.allowed_upstream_host,
        }
    }

    fn installed_args(self) -> Result<RunnerArgs, RunnerFailure> {
        if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            return Err(RunnerFailure::new("runner_slot_platform_not_supported"));
        }
        let layout = InstallLayout::discover_or_default()
            .map_err(|_| RunnerFailure::new("runner_slot_install_root_invalid"))?;
        let marker = std::fs::read(layout.marker_path())
            .map_err(|_| RunnerFailure::new("runner_slot_install_root_invalid"))?;
        layout
            .verify_marker_bytes(&marker)
            .map_err(|_| RunnerFailure::new("runner_slot_install_root_invalid"))?;
        Ok(self.into_runner_args(&layout))
    }
}

fn default_layout() -> InstallLayout {
    InstallLayout::discover_or_default()
        .unwrap_or_else(|error| panic!("invalid Aster Team install root: {error}"))
}

fn default_runner_identity() -> PathBuf {
    default_layout().runner_identity()
}

fn default_runner_task_keys() -> PathBuf {
    default_layout().runner_task_keys()
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TrustedKeyFile {
    schema: String,
    keys: Vec<TrustedKeyEntry>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TrustedKeyEntry {
    key_id: String,
    public_key_spki: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RunnerIdentityFile {
    schema: String,
    runner_id: String,
    credential: String,
}

impl Drop for RunnerIdentityFile {
    fn drop(&mut self) {
        self.credential.zeroize();
    }
}

#[derive(Serialize)]
struct EnrollmentRequest<'a> {
    token: &'a str,
    version: &'static str,
    protocol_version: u32,
    platform: &'static str,
    architecture: &'static str,
    max_inflight: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrollmentResponse {
    runner_id: String,
    credential: String,
    protocol_version: u32,
    task_keys: TrustedKeyFile,
    #[serde(rename = "notice")]
    _notice: String,
}

impl Drop for EnrollmentResponse {
    fn drop(&mut self) {
        self.credential.zeroize();
    }
}

struct RuntimeConfig {
    control_wss: String,
    runner_id: String,
    credential: Zeroizing<String>,
    control_tls_connector: Option<Connector>,
    upstream_ca_certificate: Option<PathBuf>,
    task_keys: Arc<TrustedTaskKeys>,
    max_inflight: u32,
    heartbeat: StdDuration,
    allowed_upstream_hosts: Arc<BTreeSet<String>>,
}

#[derive(Default)]
struct Metrics {
    inflight: AtomicU32,
    recent_requests: AtomicU32,
    recent_errors: AtomicU32,
    draining: AtomicBool,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let service_logging = matches!(&cli.command, Command::Serve(_) | Command::ServeSlot(_));
    if service_logging {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "aster_runner=info".into()),
            )
            .init();
    }
    if let Err(failure) = run(cli).await {
        if service_logging {
            error!(
                category = failure.category,
                detail = failure.detail.as_deref().unwrap_or(""),
                "aster-runner stopped"
            );
        } else if let Some(detail) = &failure.detail {
            eprintln!("aster-runner: {}: {detail}", failure.category);
        } else {
            eprintln!("aster-runner: {}", failure.category);
        }
        std::process::exit(1);
    }
}

#[derive(Debug)]
struct RunnerFailure {
    category: &'static str,
    detail: Option<String>,
}

impl RunnerFailure {
    const fn new(category: &'static str) -> Self {
        Self {
            category,
            detail: None,
        }
    }

    /// Keeps the reason a Control refused a request, so an operator does not have
    /// to correlate timestamps with the Control log to learn what happened.
    fn with_detail(category: &'static str, detail: impl Into<String>) -> Self {
        Self {
            category,
            detail: Some(detail.into()),
        }
    }
}

async fn run(cli: Cli) -> Result<(), RunnerFailure> {
    match cli.command {
        Command::Enroll(args) => enroll(args).await,
        Command::Preflight(args) => preflight(args),
        Command::Serve(args) => serve(args, None).await,
        Command::PreflightSlot(args) => preflight(args.installed_args()?),
        Command::ServeSlot(args) => {
            let slot = args.slot;
            serve(args.installed_args()?, Some(slot)).await
        }
    }
}

fn preflight(args: RunnerArgs) -> Result<(), RunnerFailure> {
    let config = build_config(args)?;
    build_upstream_http_client(config.upstream_ca_certificate.as_deref())?;
    info!("Runner preflight passed");
    Ok(())
}

async fn serve(args: RunnerArgs, slot: Option<ReleaseSlot>) -> Result<(), RunnerFailure> {
    let config = Arc::new(build_config(args)?);
    let metrics = Arc::new(Metrics::default());
    let managed_release = managed_release::ManagedRelease::discover(slot)?;
    // Keep one signal listener alive across both sessions and reconnect delays.
    // A slot's deliberate clean exit must not be restarted by systemd.
    let sessions = async {
        loop {
            if let Some(release) = &managed_release {
                release.require_selected()?;
            }
            if let Err(failure) = run_session(Arc::clone(&config), Arc::clone(&metrics)).await {
                warn!(
                    category = failure.category,
                    "Runner session ended; reconnecting"
                );
            }
            tokio::time::sleep(StdDuration::from_secs(3)).await;
        }
    };
    tokio::select! {
        result = sessions => result,
        signal = shutdown_signal() => {
            signal?;
            info!("Runner shutdown requested");
            Ok(())
        }
    }
}

async fn shutdown_signal() -> Result<(), RunnerFailure> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut interrupt = signal(SignalKind::interrupt())
            .map_err(|_| RunnerFailure::new("shutdown_signal_failed"))?;
        let mut terminate = signal(SignalKind::terminate())
            .map_err(|_| RunnerFailure::new("shutdown_signal_failed"))?;
        tokio::select! {
            value = interrupt.recv() => value,
            value = terminate.recv() => value,
        }
        .ok_or_else(|| RunnerFailure::new("shutdown_signal_failed"))
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c()
        .await
        .map_err(|_| RunnerFailure::new("shutdown_signal_failed"))
}

/// The Control answers a refused enrollment with a bounded error envelope. Report
/// its status and error code so an operator does not have to match timestamps in
/// the Control log to learn why the enrollment was refused.
async fn enrollment_rejection_detail(response: reqwest::Response) -> String {
    const MAX_ERROR_BODY_BYTES: usize = 4 * 1024;
    let status = response.status();
    let number = response
        .headers()
        .get("X-Aster-Error-Number")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let code = match response.bytes().await {
        Ok(body) if body.len() <= MAX_ERROR_BODY_BYTES => {
            serde_json::from_slice::<serde_json::Value>(&body)
                .ok()
                .and_then(|envelope| {
                    envelope
                        .get("error")
                        .and_then(|error| error.get("code"))
                        .and_then(|code| code.as_str())
                        .map(str::to_owned)
                })
        }
        _ => None,
    };
    enrollment_rejection_message(status, number.as_deref(), code.as_deref())
}

fn enrollment_rejection_message(
    status: StatusCode,
    number: Option<&str>,
    code: Option<&str>,
) -> String {
    let status = status.as_u16();
    match (code, number) {
        (Some(code), Some(number)) => format!("HTTP {status} {code} ({number})"),
        (Some(code), None) => format!("HTTP {status} {code}"),
        (None, Some(number)) => format!("HTTP {status} error number {number}"),
        (None, None) => format!("HTTP {status}"),
    }
}

async fn enroll(args: EnrollArgs) -> Result<(), RunnerFailure> {
    if args.identity_file.exists() || args.task_keys_file.exists() {
        return Err(RunnerFailure::new("runner_identity_already_exists"));
    }
    let token = read_enrollment_token(&args.token_file)?;
    if !(1..=100_000).contains(&args.max_inflight) || !valid_enrollment_token(token.as_str()) {
        return Err(RunnerFailure::new("runner_enrollment_input_invalid"));
    }
    let endpoint = enrollment_endpoint(&args.control_url, args.allow_insecure_http)?;
    let parent = args
        .identity_file
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .ok_or_else(|| RunnerFailure::new("runner_identity_path_invalid"))?;
    std::fs::create_dir_all(parent)
        .map_err(|_| RunnerFailure::new("runner_identity_directory_unwritable"))?;
    let mut temporary = NamedTempFile::new_in(parent)
        .map_err(|_| RunnerFailure::new("runner_identity_directory_unwritable"))?;
    let client = build_control_http_client(args.control_ca_certificate.as_deref())?;
    let response = client
        .post(endpoint)
        .json(&EnrollmentRequest {
            token: token.as_str(),
            version: env!("CARGO_PKG_VERSION"),
            protocol_version: RUNNER_PROTOCOL_VERSION,
            platform: std::env::consts::OS,
            architecture: std::env::consts::ARCH,
            max_inflight: args.max_inflight,
        })
        .send()
        .await
        .map_err(|_| RunnerFailure::new("runner_enrollment_request_failed"))?;
    if !response.status().is_success() {
        let detail = enrollment_rejection_detail(response).await;
        return Err(RunnerFailure::with_detail(
            "runner_enrollment_rejected",
            detail,
        ));
    }
    let body = Zeroizing::new(
        response
            .bytes()
            .await
            .map_err(|_| RunnerFailure::new("runner_enrollment_response_invalid"))?
            .to_vec(),
    );
    if body.len() > 64 * 1024 {
        return Err(RunnerFailure::new("runner_enrollment_response_invalid"));
    }
    let mut response: EnrollmentResponse = decode_exact(body.as_slice())
        .map_err(|_| RunnerFailure::new("runner_enrollment_response_invalid"))?;
    if response.protocol_version != RUNNER_PROTOCOL_VERSION
        || !valid_runner_id(&response.runner_id)
        || !valid_runner_credential(&response.credential)
        || validate_task_keys(&response.task_keys).is_err()
    {
        return Err(RunnerFailure::new("runner_enrollment_response_invalid"));
    }
    let identity = RunnerIdentityFile {
        schema: "aster.runner-identity.v1".to_owned(),
        runner_id: response.runner_id.clone(),
        credential: std::mem::take(&mut response.credential),
    };
    let encoded = Zeroizing::new(
        serde_json::to_vec(&identity)
            .map_err(|_| RunnerFailure::new("runner_identity_encode_failed"))?,
    );
    temporary
        .write_all(encoded.as_slice())
        .and_then(|_| temporary.write_all(b"\n"))
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|_| RunnerFailure::new("runner_identity_write_failed"))?;
    set_secret_permissions(temporary.path())?;
    let task_keys_parent = args
        .task_keys_file
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .ok_or_else(|| RunnerFailure::new("task_keys_path_invalid"))?;
    std::fs::create_dir_all(task_keys_parent)
        .map_err(|_| RunnerFailure::new("task_keys_directory_unwritable"))?;
    let mut task_keys_temporary = NamedTempFile::new_in(task_keys_parent)
        .map_err(|_| RunnerFailure::new("task_keys_directory_unwritable"))?;
    let mut task_keys_encoded = Zeroizing::new(
        serde_json::to_vec_pretty(&response.task_keys)
            .map_err(|_| RunnerFailure::new("task_keys_invalid"))?,
    );
    task_keys_encoded.push(b'\n');
    task_keys_temporary
        .write_all(task_keys_encoded.as_slice())
        .and_then(|_| task_keys_temporary.as_file().sync_all())
        .map_err(|_| RunnerFailure::new("task_keys_write_failed"))?;
    set_secret_permissions(task_keys_temporary.path())?;
    task_keys_temporary
        .persist_noclobber(&args.task_keys_file)
        .map_err(|_| RunnerFailure::new("task_keys_write_failed"))?;
    temporary
        .persist_noclobber(&args.identity_file)
        .map_err(|_| {
            let _ = std::fs::remove_file(&args.task_keys_file);
            RunnerFailure::new("runner_identity_write_failed")
        })?;
    info!(
        runner_id = %identity.runner_id,
        identity_file = %args.identity_file.display(),
        "Runner enrollment completed"
    );
    Ok(())
}

fn read_enrollment_token(path: &Path) -> Result<Zeroizing<String>, RunnerFailure> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| RunnerFailure::new("runner_enrollment_token_unreadable"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 4096 {
        return Err(RunnerFailure::new("runner_enrollment_token_invalid"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if !enrollment_token_permissions_valid(metadata.uid(), metadata.mode()) {
            return Err(RunnerFailure::new(
                "runner_enrollment_token_permissions_invalid",
            ));
        }
    }
    let value = std::fs::read_to_string(path)
        .map_err(|_| RunnerFailure::new("runner_enrollment_token_unreadable"))?;
    Ok(Zeroizing::new(value.trim().to_owned()))
}

#[cfg(all(unix, feature = "local-demo"))]
fn enrollment_token_permissions_valid(_owner_uid: u32, mode: u32) -> bool {
    mode & 0o077 == 0
}

#[cfg(all(unix, not(feature = "local-demo")))]
fn enrollment_token_permissions_valid(owner_uid: u32, mode: u32) -> bool {
    owner_uid == 0 && mode & 0o077 == 0
}

fn read_ca_certificates(
    path: &Path,
    unreadable: &'static str,
    invalid: &'static str,
) -> Result<Vec<CertificateDer<'static>>, RunnerFailure> {
    let file = std::fs::File::open(path).map_err(|_| RunnerFailure::new(unreadable))?;
    let mut bytes = Vec::new();
    file.take(MAX_CA_BUNDLE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| RunnerFailure::new(unreadable))?;
    if bytes.is_empty()
        || bytes.len() as u64 > MAX_CA_BUNDLE_BYTES
        || bytes
            .windows(b"PRIVATE KEY".len())
            .any(|window| window == b"PRIVATE KEY")
    {
        return Err(RunnerFailure::new(invalid));
    }
    let mut certificates = Vec::new();
    for item in <(SectionKind, Vec<u8>)>::pem_slice_iter(&bytes) {
        match item.map_err(|_| RunnerFailure::new(invalid))? {
            (SectionKind::Certificate, certificate) => {
                certificates.push(CertificateDer::from(certificate));
            }
            _ => return Err(RunnerFailure::new(invalid)),
        }
    }
    if certificates.is_empty() {
        return Err(RunnerFailure::new(invalid));
    }
    Ok(certificates)
}

fn read_control_ca_certificates(
    path: &Path,
) -> Result<Vec<CertificateDer<'static>>, RunnerFailure> {
    read_ca_certificates(
        path,
        "control_ca_certificate_unreadable",
        "control_ca_certificate_invalid",
    )
}

fn build_upstream_http_client(
    upstream_ca_certificate: Option<&Path>,
) -> Result<Client, RunnerFailure> {
    let mut builder = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(StdDuration::from_secs(20))
        .timeout(StdDuration::from_secs(600));
    if let Some(path) = upstream_ca_certificate {
        for certificate in read_ca_certificates(
            path,
            "upstream_ca_certificate_unreadable",
            "upstream_ca_certificate_invalid",
        )? {
            let certificate = reqwest::Certificate::from_der(certificate.as_ref())
                .map_err(|_| RunnerFailure::new("upstream_ca_certificate_invalid"))?;
            builder = builder.add_root_certificate(certificate);
        }
    }
    builder
        .build()
        .map_err(|_| RunnerFailure::new("http_client_invalid"))
}

fn build_control_http_client(
    control_ca_certificate: Option<&Path>,
) -> Result<Client, RunnerFailure> {
    let mut builder = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(StdDuration::from_secs(20))
        .timeout(StdDuration::from_secs(60));
    if let Some(path) = control_ca_certificate {
        for certificate in read_control_ca_certificates(path)? {
            let certificate = reqwest::Certificate::from_der(certificate.as_ref())
                .map_err(|_| RunnerFailure::new("control_ca_certificate_invalid"))?;
            builder = builder.add_root_certificate(certificate);
        }
    }
    builder
        .build()
        .map_err(|_| RunnerFailure::new("http_client_invalid"))
}

fn build_control_tls_connector(
    control_ca_certificate: Option<&Path>,
) -> Result<Option<Connector>, RunnerFailure> {
    let Some(path) = control_ca_certificate else {
        return Ok(None);
    };
    let mut roots =
        rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    for certificate in read_control_ca_certificates(path)? {
        roots
            .add(certificate)
            .map_err(|_| RunnerFailure::new("control_ca_certificate_invalid"))?;
    }
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Some(Connector::Rustls(Arc::new(config))))
}

fn build_config(args: RunnerArgs) -> Result<RuntimeConfig, RunnerFailure> {
    if args.max_inflight == 0 || !(2..=60).contains(&args.heartbeat_seconds) {
        return Err(RunnerFailure::new("runner_capacity_invalid"));
    }
    let mut identity = read_runner_identity(&args.identity_file)?;
    let request = args
        .control_wss
        .as_str()
        .into_client_request()
        .map_err(|_| RunnerFailure::new("control_wss_invalid"))?;
    if !control_websocket_is_allowed(request.uri(), args.allow_insecure_http) {
        return Err(RunnerFailure::new("control_wss_must_use_tls"));
    }
    let allowed_upstream_hosts = args
        .allowed_upstream_host
        .into_iter()
        .map(|host| normalize_host(&host))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if allowed_upstream_hosts.is_empty() {
        return Err(RunnerFailure::new("upstream_allowlist_empty"));
    }
    let control_tls_connector =
        build_control_tls_connector(args.control_ca_certificate.as_deref())?;
    Ok(RuntimeConfig {
        control_wss: args.control_wss,
        runner_id: std::mem::take(&mut identity.runner_id),
        credential: Zeroizing::new(std::mem::take(&mut identity.credential)),
        control_tls_connector,
        upstream_ca_certificate: args.upstream_ca_certificate,
        task_keys: Arc::new(read_task_keys(&args.task_keys_file)?),
        max_inflight: args.max_inflight,
        heartbeat: StdDuration::from_secs(args.heartbeat_seconds),
        allowed_upstream_hosts: Arc::new(allowed_upstream_hosts),
    })
}

fn control_websocket_is_allowed(
    uri: &tokio_tungstenite::tungstenite::http::Uri,
    allow_insecure_http: bool,
) -> bool {
    if uri.scheme_str() == Some("wss") {
        return true;
    }
    uri.scheme_str() == Some("ws")
        && (allow_insecure_http
            || matches!(
                uri.host(),
                Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
            ))
}

async fn run_session(
    config: Arc<RuntimeConfig>,
    metrics: Arc<Metrics>,
) -> Result<(), RunnerFailure> {
    metrics.draining.store(false, Ordering::Relaxed);
    let mut request = config
        .control_wss
        .as_str()
        .into_client_request()
        .map_err(|_| RunnerFailure::new("control_wss_invalid"))?;
    let bearer = Zeroizing::new(format!("Bearer {}", config.credential.as_str()));
    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_str(bearer.as_str())
            .map_err(|_| RunnerFailure::new("runner_credential_invalid"))?,
    );
    let (socket, _) =
        connect_async_tls_with_config(request, None, false, config.control_tls_connector.clone())
            .await
            .map_err(|_| RunnerFailure::new("control_connect_failed"))?;
    info!(runner_id = %config.runner_id, "Runner connected");
    let (mut writer, mut reader) = socket.split();
    let (outbound, mut outbound_rx) = mpsc::channel::<RunnerToControl>(256);
    outbound
        .send(RunnerToControl::Hello(RunnerHello {
            runner_id: config.runner_id.clone(),
            protocol_version: RUNNER_PROTOCOL_VERSION,
            runner_version: env!("CARGO_PKG_VERSION").to_owned(),
            platform: std::env::consts::OS.to_owned(),
            architecture: std::env::consts::ARCH.to_owned(),
            max_inflight: config.max_inflight,
        }))
        .await
        .map_err(|_| RunnerFailure::new("control_channel_closed"))?;
    let writer_metrics = Arc::clone(&metrics);
    let heartbeat = config.heartbeat;
    let mut writer_task = AbortOnDrop(tokio::spawn(async move {
        let mut interval = tokio::time::interval(heartbeat);
        loop {
            let frame = tokio::select! {
                frame = outbound_rx.recv() => match frame {
                    Some(frame) => frame,
                    None => return Err(RunnerFailure::new("control_channel_closed")),
                },
                _ = interval.tick() => RunnerToControl::Heartbeat(RunnerHeartbeat {
                    inflight: writer_metrics.inflight.load(Ordering::Relaxed),
                    recent_request_count: writer_metrics.recent_requests.swap(0, Ordering::Relaxed),
                    recent_error_count: writer_metrics.recent_errors.swap(0, Ordering::Relaxed),
                    latency_ms: 0,
                    observed_at: time::OffsetDateTime::now_utc().unix_timestamp(),
                }),
            };
            let encoded = encode_runner_frame(&frame)
                .map_err(|_| RunnerFailure::new("runner_frame_encode_failed"))?;
            writer
                .send(Message::Text(
                    String::from_utf8(encoded)
                        .map_err(|_| RunnerFailure::new("runner_frame_encode_failed"))?
                        .into(),
                ))
                .await
                .map_err(|_| RunnerFailure::new("control_send_failed"))?;
        }
    }));

    let replay = Arc::new(Mutex::new(ReplayCache::new()));
    let mut tasks = TaskSupervisor::new(config.max_inflight as usize);
    let client = build_upstream_http_client(config.upstream_ca_certificate.as_deref())?;

    loop {
        let message = tokio::select! {
            result = &mut writer_task.0 => return result.unwrap_or_else(|_| Err(RunnerFailure::new("control_writer_failed"))),
            _ = tasks.next_finished(), if !tasks.is_empty() => continue,
            message = reader.next() => message,
        };
        let Some(message) = message else {
            break;
        };
        let message = message.map_err(|_| RunnerFailure::new("control_receive_failed"))?;
        let Message::Text(text) = message else {
            if matches!(message, Message::Close(_)) {
                break;
            }
            continue;
        };
        match decode_control_frame(text.as_bytes())
            .map_err(|_| RunnerFailure::new("control_frame_invalid"))?
        {
            ControlToRunner::Ping(PingFrame { nonce }) => {
                outbound
                    .send(RunnerToControl::Pong(PingFrame { nonce }))
                    .await
                    .map_err(|_| RunnerFailure::new("control_channel_closed"))?;
            }
            ControlToRunner::Drain => metrics.draining.store(true, Ordering::Relaxed),
            ControlToRunner::CancelTask(frame) => tasks.cancel(&frame.task_id),
            ControlToRunner::Task(frame) => {
                accept_task(
                    frame, &config, &metrics, &replay, &mut tasks, &client, &outbound,
                )
                .await?;
            }
        }
    }
    writer_task.0.abort();
    Err(RunnerFailure::new("control_connection_closed"))
}

struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);
impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn accept_task(
    mut frame: TaskFrame,
    config: &Arc<RuntimeConfig>,
    metrics: &Arc<Metrics>,
    replay: &Arc<Mutex<ReplayCache>>,
    tasks: &mut TaskSupervisor,
    client: &Client,
    outbound: &mpsc::Sender<RunnerToControl>,
) -> Result<(), RunnerFailure> {
    let ticket_encoded = Zeroizing::new(std::mem::take(&mut frame.ticket_json_base64url));
    let payload_encoded = Zeroizing::new(std::mem::take(&mut frame.payload_base64url));
    let ticket_json = Zeroizing::new(
        decode_base64url(ticket_encoded.as_str())
            .map_err(|_| RunnerFailure::new("task_ticket_encoding_invalid"))?,
    );
    let payload = Zeroizing::new(
        decode_base64url(payload_encoded.as_str())
            .map_err(|_| RunnerFailure::new("task_payload_encoding_invalid"))?,
    );
    if payload.len() > MAX_TASK_PAYLOAD_BYTES {
        return Err(RunnerFailure::new("task_payload_too_large"));
    }
    let received_instant = tokio::time::Instant::now();
    let now = time::OffsetDateTime::now_utc();
    let ticket = match verify_task_ticket(
        ticket_json.as_slice(),
        config.task_keys.as_ref(),
        &config.runner_id,
        payload.as_slice(),
        now,
    ) {
        Ok(ticket) => ticket,
        Err(aster_runner_protocol::RunnerProtocolError::ExpiredTask(task_id)) => {
            send_failure(outbound, &task_id, "task_expired_before_execution", true).await;
            return Ok(());
        }
        Err(_) => return Err(RunnerFailure::new("task_ticket_invalid")),
    };
    replay
        .lock()
        .await
        .consume(&ticket, time::OffsetDateTime::now_utc())
        .map_err(|error| {
            RunnerFailure::new(match error {
                aster_runner_protocol::RunnerProtocolError::InvalidTime => "task_ticket_expired",
                _ => "task_ticket_replayed",
            })
        })?;
    let remaining_ms = i128::from(ticket.claims().execution_deadline_ms)
        - time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000;
    if remaining_ms <= 0 {
        send_failure(
            outbound,
            &ticket.claims().task_id,
            "task_expired_before_execution",
            true,
        )
        .await;
        return Ok(());
    }
    let deadline =
        tokio::time::Instant::now()
            + StdDuration::from_millis(u64::try_from(remaining_ms).unwrap_or(u64::MAX).min(
                u64::from(aster_runner_protocol::MAX_TASK_EXECUTION_MILLISECONDS),
            ));
    let task_id = ticket.claims().task_id.clone();
    let command = ticket.claims().command;
    let upstream_host = ticket.claims().upstream_host.clone();
    let credential_instance_id = ticket.claims().credential_instance_id.clone();
    let credential_revision = ticket.claims().credential_revision;
    let execution = TaskExecutionPermit::new(ticket, now, received_instant)?;
    if metrics.draining.load(Ordering::Relaxed) {
        send_failure(outbound, &task_id, "runner_draining", true).await;
        return Ok(());
    }
    let permit = match tasks.try_permit() {
        Some(permit) => permit,
        None => {
            send_failure(outbound, &task_id, "runner_capacity", true).await;
            return Ok(());
        }
    };

    let client = client.clone();
    let allowed_hosts = Arc::clone(&config.allowed_upstream_hosts);
    let outbound = outbound.clone();
    let task = TaskExecution {
        id: task_id.clone(),
        deadline,
        outbound: outbound.clone(),
        metrics: metrics.clone(),
        permit,
    };
    let duplicate_task_id = task_id.clone();
    let failure_sender = outbound.clone();
    let started = tasks.start(task, async move {
        if execution.accepted(&outbound).await.is_err() {
            return Err(());
        }
        match command {
            TaskCommand::Probe => execute_probe(&task_id, payload.as_slice(), &outbound).await,
            TaskCommand::Execute | TaskCommand::DiscoverModels => {
                execute_task(
                    &client,
                    allowed_hosts.as_ref(),
                    &upstream_host,
                    false,
                    &execution,
                    payload,
                    &outbound,
                )
                .await
            }
            TaskCommand::FetchAsset => {
                execute_task(
                    &client,
                    allowed_hosts.as_ref(),
                    &upstream_host,
                    true,
                    &execution,
                    payload,
                    &outbound,
                )
                .await
            }
            TaskCommand::RefreshCredential => {
                let binding = CredentialRefreshBinding {
                    upstream_host: &upstream_host,
                    credential_instance_id: credential_instance_id.as_deref(),
                    credential_revision,
                };
                execute_credential_refresh(
                    &client,
                    allowed_hosts.as_ref(),
                    binding,
                    &execution,
                    payload,
                    &outbound,
                )
                .await
            }
            TaskCommand::AuthorizeCredential => {
                execute_task(
                    &client,
                    allowed_hosts.as_ref(),
                    &upstream_host,
                    false,
                    &execution,
                    payload,
                    &outbound,
                )
                .await
            }
        }
    });
    if !started {
        send_failure(
            &failure_sender,
            &duplicate_task_id,
            "task_already_active",
            false,
        )
        .await;
    }
    Ok(())
}

struct InflightGuard(Arc<Metrics>);

impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.0.inflight.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Echo a bounded random challenge through the signed task worker. No URL is
/// interpreted and no upstream client is used on this command path.
async fn execute_probe(
    task_id: &str,
    payload: &[u8],
    outbound: &mpsc::Sender<RunnerToControl>,
) -> Result<(), ()> {
    if payload.len() != 32 {
        send_failure(outbound, task_id, "probe_payload_invalid", false).await;
        return Err(());
    }
    outbound
        .send(RunnerToControl::TaskChunk(TaskChunk {
            task_id: task_id.to_owned(),
            sequence: 0,
            data_base64url: encode_base64url(payload),
        }))
        .await
        .map_err(|_| ())?;
    outbound
        .send(RunnerToControl::TaskFinished(TaskResult {
            task_id: task_id.to_owned(),
            status: 204,
            usage_json: None,
        }))
        .await
        .map_err(|_| ())
}

async fn execute_task(
    client: &Client,
    allowed_hosts: &BTreeSet<String>,
    expected_upstream_host: &str,
    asset_fetch: bool,
    execution: &TaskExecutionPermit,
    raw_payload: Zeroizing<Vec<u8>>,
    outbound: &mpsc::Sender<RunnerToControl>,
) -> Result<(), ()> {
    let task_id = execution.task_id();
    let payload: UpstreamHttpRequest = match decode_exact(raw_payload.as_slice()) {
        Ok(payload) => payload,
        Err(()) => {
            send_failure(outbound, task_id, "task_payload_invalid", true).await;
            return Err(());
        }
    };
    let (url, asset_client) = if asset_fetch {
        match build_asset_client(&payload.url, expected_upstream_host).await {
            Ok((url, pinned)) => (url, Some(pinned)),
            Err(()) => {
                send_failure(outbound, task_id, "asset_source_not_allowed", true).await;
                return Err(());
            }
        }
    } else {
        match validate_upstream_url(&payload.url, allowed_hosts, expected_upstream_host) {
            Ok(url) => (url, None),
            Err(()) => {
                send_failure(outbound, task_id, "upstream_not_allowed", true).await;
                return Err(());
            }
        }
    };
    let method = match Method::from_bytes(payload.method.as_bytes()) {
        Ok(Method::GET) if asset_fetch => Method::GET,
        Ok(method) if !asset_fetch && matches!(method, Method::GET | Method::POST) => method,
        _ => {
            send_failure(outbound, task_id, "upstream_method_invalid", true).await;
            return Err(());
        }
    };
    if payload.headers.len() > 64 {
        send_failure(outbound, task_id, "upstream_headers_invalid", true).await;
        return Err(());
    }
    if asset_fetch && (!payload.headers.is_empty() || !payload.body_base64url.is_empty()) {
        send_failure(outbound, task_id, "asset_request_invalid", true).await;
        return Err(());
    }
    let selected_client = asset_client.as_ref().unwrap_or(client);
    let mut request = selected_client.request(method, url);
    for header in &payload.headers {
        let Ok(name) = HeaderName::from_bytes(header.name.as_bytes()) else {
            send_failure(outbound, task_id, "upstream_headers_invalid", true).await;
            return Err(());
        };
        if is_forbidden_request_header(&name)
            || header.value.len() > MAX_RESPONSE_HEADER_VALUE_BYTES
        {
            send_failure(outbound, task_id, "upstream_headers_invalid", true).await;
            return Err(());
        }
        request = request.header(name, &header.value);
    }
    let body = match decode_base64url(&payload.body_base64url) {
        Ok(value) if value.len() <= MAX_TASK_PAYLOAD_BYTES => Zeroizing::new(value),
        _ => {
            send_failure(outbound, task_id, "upstream_body_invalid", true).await;
            return Err(());
        }
    };
    request = request.body(body.to_vec());
    execution.start_upstream(outbound).await?;
    let response = match request.send().await {
        Ok(response) => response,
        Err(_) => {
            send_failure(outbound, task_id, "upstream_transport", false).await;
            return Err(());
        }
    };
    let status = response.status().as_u16();
    if asset_fetch
        && response
            .content_length()
            .is_some_and(|length| length > 32 * 1024 * 1024)
    {
        send_failure(outbound, task_id, "asset_too_large", false).await;
        return Err(());
    }
    let headers = response_headers(response.headers());
    if outbound
        .send(RunnerToControl::TaskResponseStarted(TaskResponseStarted {
            task_id: task_id.to_owned(),
            status,
            headers,
        }))
        .await
        .is_err()
    {
        return Err(());
    }
    let mut sequence = 0_u32;
    let mut asset_bytes = 0_usize;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(_) => {
                send_failure(outbound, task_id, "upstream_stream", false).await;
                return Err(());
            }
        };
        if asset_fetch {
            asset_bytes = match asset_bytes.checked_add(chunk.len()) {
                Some(value) if value <= 32 * 1024 * 1024 => value,
                _ => {
                    send_failure(outbound, task_id, "asset_too_large", false).await;
                    return Err(());
                }
            };
        }
        if outbound
            .send(RunnerToControl::TaskChunk(TaskChunk {
                task_id: task_id.to_owned(),
                sequence,
                data_base64url: encode_base64url(&chunk),
            }))
            .await
            .is_err()
        {
            return Err(());
        }
        sequence = sequence.checked_add(1).ok_or(())?;
    }
    outbound
        .send(RunnerToControl::TaskFinished(TaskResult {
            task_id: task_id.to_owned(),
            status,
            usage_json: None,
        }))
        .await
        .map_err(|_| ())
}

async fn build_asset_client(source: &str, expected_host: &str) -> Result<(Url, Client), ()> {
    if source.len() > 8192 {
        return Err(());
    }
    let url = Url::parse(source).map_err(|_| ())?;
    let host = url.host_str().ok_or(())?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
        || url.fragment().is_some()
        || host != expected_host
        || host.parse::<std::net::IpAddr>().is_ok()
    {
        return Err(());
    }
    let addresses = tokio::net::lookup_host((host, 443))
        .await
        .map_err(|_| ())?
        .collect::<Vec<_>>();
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !public_asset_ip(address.ip()))
    {
        return Err(());
    }
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(StdDuration::from_secs(20))
        .timeout(StdDuration::from_secs(60))
        .resolve(host, addresses[0])
        .build()
        .map_err(|_| ())?;
    Ok((url, client))
}

fn public_asset_ip(address: std::net::IpAddr) -> bool {
    match address {
        std::net::IpAddr::V4(ip) => {
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_multicast()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && !ip.is_unspecified()
                && ip.octets()[0] != 0
                && ip.octets()[0] != 100
                && !(ip.octets()[0] == 192 && ip.octets()[1] == 0 && ip.octets()[2] == 0)
                && !(ip.octets()[0] == 198 && (ip.octets()[1] == 18 || ip.octets()[1] == 19))
                && ip.octets()[0] < 224
        }
        std::net::IpAddr::V6(ip) => {
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_unique_local()
                && !ip.is_unicast_link_local()
                && ip.to_ipv4_mapped().is_none()
                && ip.segments()[0] & 0xe000 == 0x2000
                && !(ip.segments()[0] == 0x2001 && ip.segments()[1] == 0x0db8)
        }
    }
}

struct CredentialRefreshBinding<'a> {
    upstream_host: &'a str,
    credential_instance_id: Option<&'a str>,
    credential_revision: Option<u32>,
}

async fn execute_credential_refresh(
    client: &Client,
    allowed_hosts: &BTreeSet<String>,
    binding: CredentialRefreshBinding<'_>,
    execution: &TaskExecutionPermit,
    raw_payload: Zeroizing<Vec<u8>>,
    outbound: &mpsc::Sender<RunnerToControl>,
) -> Result<(), ()> {
    let task_id = execution.task_id();
    let payload: CredentialRefreshTask = match decode_exact(raw_payload.as_slice()) {
        Ok(payload) => payload,
        Err(()) => {
            send_failure(
                outbound,
                task_id,
                "credential_refresh_payload_invalid",
                true,
            )
            .await;
            return Err(());
        }
    };
    if !valid_credential_id(&payload.credential_id)
        || binding.credential_instance_id != Some(payload.credential_id.as_str())
        || binding.credential_revision != Some(payload.expected_revision)
    {
        send_failure(
            outbound,
            task_id,
            "credential_refresh_payload_invalid",
            true,
        )
        .await;
        return Err(());
    }
    let url =
        match validate_upstream_url(&payload.request.url, allowed_hosts, binding.upstream_host) {
            Ok(url) => url,
            Err(()) => {
                send_failure(outbound, task_id, "upstream_not_allowed", true).await;
                return Err(());
            }
        };
    let method = match Method::from_bytes(payload.request.method.as_bytes()) {
        Ok(Method::POST) => Method::POST,
        _ => {
            send_failure(outbound, task_id, "upstream_method_invalid", true).await;
            return Err(());
        }
    };
    if payload.request.headers.len() > 64 {
        send_failure(outbound, task_id, "upstream_headers_invalid", true).await;
        return Err(());
    }
    let mut request = client.request(method, url);
    for header in &payload.request.headers {
        let Ok(name) = HeaderName::from_bytes(header.name.as_bytes()) else {
            send_failure(outbound, task_id, "upstream_headers_invalid", true).await;
            return Err(());
        };
        if is_forbidden_request_header(&name)
            || header.value.len() > MAX_RESPONSE_HEADER_VALUE_BYTES
        {
            send_failure(outbound, task_id, "upstream_headers_invalid", true).await;
            return Err(());
        }
        request = request.header(name, &header.value);
    }
    let body = match decode_base64url(&payload.request.body_base64url) {
        Ok(value) if value.len() <= MAX_CREDENTIAL_REFRESH_RESPONSE_BYTES => Zeroizing::new(value),
        _ => {
            send_failure(outbound, task_id, "upstream_body_invalid", true).await;
            return Err(());
        }
    };
    request = request.body(body.to_vec());
    execution.start_upstream(outbound).await?;
    let response = match request.send().await {
        Ok(response) => response,
        Err(_) => {
            send_failure(outbound, task_id, "upstream_transport", false).await;
            return Err(());
        }
    };
    let status = response.status().as_u16();
    let mut response_body = Zeroizing::new(Vec::new());
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(_) => {
                send_failure(outbound, task_id, "upstream_stream", false).await;
                return Err(());
            }
        };
        let Some(total) = response_body.len().checked_add(chunk.len()) else {
            send_failure(
                outbound,
                task_id,
                "credential_refresh_response_too_large",
                false,
            )
            .await;
            return Err(());
        };
        if total > MAX_CREDENTIAL_REFRESH_RESPONSE_BYTES {
            send_failure(
                outbound,
                task_id,
                "credential_refresh_response_too_large",
                false,
            )
            .await;
            return Err(());
        }
        response_body.extend_from_slice(&chunk);
    }
    outbound
        .send(RunnerToControl::CredentialRefreshed(
            CredentialRefreshResult {
                task_id: task_id.to_owned(),
                credential_id: payload.credential_id,
                expected_revision: payload.expected_revision,
                status,
                credential_payload_base64url: encode_base64url(&response_body),
            },
        ))
        .await
        .map_err(|_| ())
}

async fn send_failure(
    outbound: &mpsc::Sender<RunnerToControl>,
    task_id: &str,
    category: &str,
    retryable_before_upstream: bool,
) {
    let _ = outbound
        .send(RunnerToControl::TaskFailed(TaskFailure {
            task_id: task_id.to_owned(),
            category: category.to_owned(),
            retryable_before_upstream,
        }))
        .await;
}

fn validate_upstream_url(
    url: &str,
    allowed_hosts: &BTreeSet<String>,
    expected_upstream_host: &str,
) -> Result<Url, ()> {
    let url = Url::parse(url).map_err(|_| ())?;
    let expected_upstream_host = expected_upstream_host.to_ascii_lowercase();
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none_or(|host| {
            let host = host.trim_end_matches('.').to_ascii_lowercase();
            host != expected_upstream_host || !allowed_hosts.contains(&host)
        })
    {
        return Err(());
    }
    Ok(url)
}

fn valid_credential_id(value: &str) -> bool {
    value.strip_prefix("credential_").is_some_and(|suffix| {
        suffix.len() == 32 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn valid_enrollment_token(value: &str) -> bool {
    (40..=160).contains(&value.len())
        && value.strip_prefix("aren_").is_some_and(|suffix| {
            !suffix.is_empty()
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
}

fn valid_runner_id(value: &str) -> bool {
    value.strip_prefix("runner_").is_some_and(|suffix| {
        suffix.len() == 32 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn valid_runner_credential(value: &str) -> bool {
    (48..=256).contains(&value.len())
        && value.strip_prefix("arr_").is_some_and(|suffix| {
            !suffix.is_empty()
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
}

fn enrollment_endpoint(value: &str, allow_insecure_http: bool) -> Result<Url, RunnerFailure> {
    let mut url = Url::parse(value)
        .map_err(|_| RunnerFailure::new("runner_enrollment_control_url_invalid"))?;
    if !control_http_is_allowed(&url, allow_insecure_http)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
        || url.fragment().is_some()
    {
        return Err(RunnerFailure::new("runner_enrollment_control_url_invalid"));
    }
    url.set_path("/api/runner/enroll");
    url.set_query(None);
    Ok(url)
}

fn control_http_is_allowed(url: &Url, allow_insecure_http: bool) -> bool {
    if url.scheme() == "https" {
        return true;
    }
    url.scheme() == "http"
        && (allow_insecure_http
            || matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1")))
}

fn is_forbidden_request_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "host"
            | "content-length"
            | "connection"
            | "transfer-encoding"
            | "upgrade"
            | "proxy-authorization"
    )
}

fn response_headers(headers: &reqwest::header::HeaderMap) -> Vec<ResponseHeader> {
    const ALLOWED: [&str; 5] = [
        "content-type",
        "openai-processing-ms",
        "x-request-id",
        "request-id",
        "anthropic-ratelimit-unified-status",
    ];
    headers
        .iter()
        .filter(|(name, value)| {
            ALLOWED.contains(&name.as_str())
                && value.as_bytes().len() <= MAX_RESPONSE_HEADER_VALUE_BYTES
        })
        .filter_map(|(name, value)| {
            value.to_str().ok().map(|value| ResponseHeader {
                name: name.as_str().to_owned(),
                value: value.to_owned(),
            })
        })
        .collect()
}

fn read_task_keys(path: &Path) -> Result<TrustedTaskKeys, RunnerFailure> {
    let data = std::fs::read(path).map_err(|_| RunnerFailure::new("task_keys_unreadable"))?;
    let file: TrustedKeyFile =
        decode_exact(&data).map_err(|_| RunnerFailure::new("task_keys_invalid"))?;
    validate_task_keys(&file)
}

fn validate_task_keys(file: &TrustedKeyFile) -> Result<TrustedTaskKeys, RunnerFailure> {
    if file.schema != "aster.runner-task-keys.v1" || file.keys.is_empty() || file.keys.len() > 3 {
        return Err(RunnerFailure::new("task_keys_invalid"));
    }
    let mut keys = TrustedTaskKeys::new();
    for key in &file.keys {
        keys.insert_spki_base64url(key.key_id.clone(), &key.public_key_spki)
            .map_err(|_| RunnerFailure::new("task_keys_invalid"))?;
    }
    Ok(keys)
}

fn read_runner_identity(path: &Path) -> Result<RunnerIdentityFile, RunnerFailure> {
    let bytes = Zeroizing::new(
        std::fs::read(path).map_err(|_| RunnerFailure::new("runner_identity_unreadable"))?,
    );
    let identity: RunnerIdentityFile = decode_exact(bytes.as_slice())
        .map_err(|_| RunnerFailure::new("runner_identity_invalid"))?;
    if identity.schema != "aster.runner-identity.v1"
        || !valid_runner_id(&identity.runner_id)
        || !valid_runner_credential(&identity.credential)
    {
        return Err(RunnerFailure::new("runner_identity_invalid"));
    }
    Ok(identity)
}

#[cfg(unix)]
fn set_secret_permissions(path: &Path) -> Result<(), RunnerFailure> {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|_| RunnerFailure::new("runner_identity_permissions_failed"))
}

#[cfg(not(unix))]
fn set_secret_permissions(_path: &Path) -> Result<(), RunnerFailure> {
    Ok(())
}

fn normalize_host(value: &str) -> Result<String, RunnerFailure> {
    let value = value.trim().trim_end_matches('.').to_ascii_lowercase();
    if value.is_empty()
        || value.contains('/')
        || value.contains(':')
        || value.parse::<std::net::IpAddr>().is_ok()
        || !value
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'.' | b'-'))
    {
        return Err(RunnerFailure::new("upstream_allowlist_invalid"));
    }
    Ok(value)
}

fn decode_exact<'de, T: Deserialize<'de>>(data: &'de [u8]) -> Result<T, ()> {
    let mut deserializer = serde_json::Deserializer::from_slice(data);
    let value = T::deserialize(&mut deserializer).map_err(|_| ())?;
    deserializer.end().map_err(|_| ())?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use ed25519_dalek::{SigningKey, pkcs8::EncodePublicKey as _};
    use tempfile::NamedTempFile;

    use super::*;

    #[test]
    fn enrollment_rejection_reports_the_control_status_and_error_code() {
        assert_eq!(
            enrollment_rejection_message(
                StatusCode::SERVICE_UNAVAILABLE,
                Some("15012"),
                Some("MAINTENANCE_UNAVAILABLE"),
            ),
            "HTTP 503 MAINTENANCE_UNAVAILABLE (15012)"
        );
        assert_eq!(
            enrollment_rejection_message(StatusCode::CONFLICT, None, Some("MAINTENANCE_BUSY")),
            "HTTP 409 MAINTENANCE_BUSY"
        );
        assert_eq!(
            enrollment_rejection_message(StatusCode::SERVICE_UNAVAILABLE, Some("15012"), None),
            "HTTP 503 error number 15012"
        );
        assert_eq!(
            enrollment_rejection_message(StatusCode::INTERNAL_SERVER_ERROR, None, None),
            "HTTP 500"
        );
    }

    fn slot_args(slot: ReleaseSlot) -> SlotRunnerArgs {
        SlotRunnerArgs {
            slot,
            upstream_ca_certificate: None,
            max_inflight: 4,
            heartbeat_seconds: 10,
            allowed_upstream_host: vec!["api.openai.com".to_owned()],
        }
    }

    #[test]
    fn slot_cli_rejects_endpoint_identity_and_trust_overrides() {
        for command in ["serve-slot", "preflight-slot"] {
            let base = [
                "aster-runner",
                command,
                "--slot",
                "blue",
                "--allowed-upstream-host",
                "api.openai.com",
            ];
            assert!(Cli::try_parse_from(base).is_ok());
            for (flag, value) in [
                ("--control-wss", "wss://public.example/api/runner/channel"),
                ("--identity-file", "shared.json"),
                ("--task-keys-file", "shared-keys.json"),
                ("--control-ca-certificate", "public-ca.pem"),
                ("--allow-insecure-http", "false"),
            ] {
                assert!(Cli::try_parse_from(base.into_iter().chain([flag, value])).is_err());
            }
            assert!(
                Cli::try_parse_from([
                    "aster-runner",
                    command,
                    "--slot",
                    "../green",
                    "--allowed-upstream-host",
                    "api.openai.com"
                ])
                .is_err()
            );
        }
    }

    #[test]
    fn slot_config_pins_endpoint_identity_and_keys_without_legacy_fallback() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(temporary.path()).unwrap();
        std::fs::create_dir_all(layout.runner_config()).unwrap();
        std::fs::write(
            layout.runner_identity(),
            b"legacy identity must not be read",
        )
        .unwrap();
        std::fs::write(layout.runner_task_keys(), b"legacy keys must not be read").unwrap();
        for (slot, port) in [(ReleaseSlot::Blue, 11_380), (ReleaseSlot::Green, 11_480)] {
            let args = slot_args(slot).into_runner_args(&layout);
            assert_eq!(
                args.control_wss,
                format!("ws://127.0.0.1:{port}/api/runner/channel")
            );
            assert_eq!(args.identity_file, layout.runner_slot_identity(slot.id()));
            assert_eq!(args.task_keys_file, layout.runner_slot_task_keys(slot.id()));
            assert!(args.control_ca_certificate.is_none());
            assert!(args.allow_insecure_http);
            assert_eq!(
                build_config(args).err().unwrap().category,
                "runner_identity_unreadable"
            );
        }
    }

    #[test]
    fn slot_preflight_loads_only_its_own_identity_and_task_keys() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(temporary.path()).unwrap();
        let signing = SigningKey::from_bytes(&[82_u8; 32]);
        let public = signing.verifying_key().to_public_key_der().unwrap();
        for (slot, suffix) in [(ReleaseSlot::Blue, "a"), (ReleaseSlot::Green, "b")] {
            let identity = layout.runner_slot_identity(slot.id());
            std::fs::create_dir_all(identity.parent().unwrap()).unwrap();
            std::fs::write(
                &identity,
                serde_json::to_vec(&serde_json::json!({
                    "schema": "aster.runner-identity.v1",
                    "runner_id": format!("runner_{}", suffix.repeat(32)),
                    "credential": format!("arr_{}", suffix.repeat(64)),
                }))
                .unwrap(),
            )
            .unwrap();
            std::fs::write(layout.runner_slot_task_keys(slot.id()), serde_json::to_vec(&serde_json::json!({
                "schema": "aster.runner-task-keys.v1",
                "keys": [{"key_id": format!("slot-{}", slot.id()), "public_key_spki": URL_SAFE_NO_PAD.encode(public.as_bytes())}],
            })).unwrap()).unwrap();
            let config = build_config(slot_args(slot).into_runner_args(&layout)).unwrap();
            assert_eq!(config.runner_id, format!("runner_{}", suffix.repeat(32)));
        }
        std::fs::write(layout.runner_slot_task_keys("green"), b"invalid").unwrap();
        assert!(build_config(slot_args(ReleaseSlot::Blue).into_runner_args(&layout)).is_ok());
        assert!(build_config(slot_args(ReleaseSlot::Green).into_runner_args(&layout)).is_err());
    }

    #[test]
    fn preflight_requires_wss_and_an_explicit_dns_allowlist() {
        let signing = SigningKey::from_bytes(&[81_u8; 32]);
        let public = signing
            .verifying_key()
            .to_public_key_der()
            .expect("encode public key");
        let mut keys = NamedTempFile::new().expect("keys file");
        write!(
            keys,
            "{}",
            serde_json::json!({
                "schema": "aster.runner-task-keys.v1",
                "keys": [{
                    "key_id": "runner-task-test-01",
                    "public_key_spki": URL_SAFE_NO_PAD.encode(public.as_bytes()),
                }]
            })
        )
        .expect("write keys");
        let mut identity = NamedTempFile::new().expect("identity file");
        write!(
            identity,
            "{}",
            serde_json::json!({
                "schema": "aster.runner-identity.v1",
                "runner_id": "runner_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "credential": format!("arr_{}", "x".repeat(64)),
            })
        )
        .expect("write identity");
        let args = RunnerArgs {
            control_wss: "wss://control.example.test/api/runner/channel".to_owned(),
            identity_file: identity.path().to_owned(),
            task_keys_file: keys.path().to_owned(),
            control_ca_certificate: None,
            upstream_ca_certificate: None,
            allow_insecure_http: false,
            max_inflight: 4,
            heartbeat_seconds: 10,
            allowed_upstream_host: vec!["api.openai.com".to_owned()],
        };
        assert!(build_config(args).is_ok());
    }

    #[test]
    fn custom_control_ca_is_scoped_and_rejects_non_certificate_pem() {
        const TEST_CA_PEM: &str = r#"-----BEGIN CERTIFICATE-----
MIIBUzCCAQWgAwIBAgIUd24hZYmljYIhQ78QOb5iJbmekogwBQYDK2VwMB8xHTAb
BgNVBAMMFGFzdGVyLXJ1bm5lci10ZXN0LWNhMB4XDTI2MDgyODAxNTQ0MFoXDTM2
MDgyNTAxNTQ0MFowHzEdMBsGA1UEAwwUYXN0ZXItcnVubmVyLXRlc3QtY2EwKjAF
BgMrZXADIQChEaWMcl5kVVAOfVgl6UWqUnIUYZsVsRYjAfJq1r1T9KNTMFEwHQYD
VR0OBBYEFMbHeXErFPf4hQnSKGAgs6U5V15hMB8GA1UdIwQYMBaAFMbHeXErFPf4
hQnSKGAgs6U5V15hMA8GA1UdEwEB/wQFMAMBAf8wBQYDK2VwA0EA+3FvZFNf1j2f
D5Zn6OeXAMqvlQtJ5f6KLP4LTu/uc8CopLN5hjxBka62IZh2X4vrPbDSbeofzR8H
4jP1hnGQCQ==
-----END CERTIFICATE-----
"#;
        let mut certificate = NamedTempFile::new().expect("CA file");
        certificate
            .write_all(TEST_CA_PEM.as_bytes())
            .expect("write CA");
        let parsed = read_control_ca_certificates(certificate.path()).expect("parse CA");
        assert_eq!(parsed.len(), 1);
        assert!(build_control_http_client(Some(certificate.path())).is_ok());
        assert!(build_upstream_http_client(Some(certificate.path())).is_ok());
        assert!(
            build_control_tls_connector(Some(certificate.path()))
                .expect("TLS connector")
                .is_some()
        );

        let mut private_key = NamedTempFile::new().expect("private-key file");
        private_key
            .write_all(b"-----BEGIN PRIVATE KEY-----\nAA==\n-----END PRIVATE KEY-----\n")
            .expect("write private key");
        assert!(read_control_ca_certificates(private_key.path()).is_err());
        assert!(build_upstream_http_client(Some(private_key.path())).is_err());
    }

    #[test]
    fn enrollment_requires_https_and_uses_the_fixed_control_path() {
        let endpoint = enrollment_endpoint("https://control.example.test/customer/?old=1", false)
            .expect("valid enrollment endpoint");
        assert_eq!(
            endpoint.as_str(),
            "https://control.example.test/api/runner/enroll"
        );
        assert!(enrollment_endpoint("http://control.example.test", false).is_err());
        assert!(enrollment_endpoint("https://user@control.example.test", false).is_err());
        assert!(valid_enrollment_token(&format!("aren_{}", "x".repeat(43))));
        assert!(!valid_enrollment_token(&format!("arr_{}", "x".repeat(43))));
    }

    #[cfg(unix)]
    #[test]
    fn enrollment_token_permissions_keep_production_root_only_and_local_demo_private() {
        assert!(enrollment_token_permissions_valid(0, 0o100600));
        assert!(!enrollment_token_permissions_valid(0, 0o100640));
        assert!(!enrollment_token_permissions_valid(501, 0o100604));
        if cfg!(feature = "local-demo") {
            assert!(enrollment_token_permissions_valid(501, 0o100600));
        } else {
            assert!(!enrollment_token_permissions_valid(501, 0o100600));
        }
    }

    #[cfg(all(unix, feature = "local-demo"))]
    #[test]
    fn local_demo_reads_a_private_developer_owned_enrollment_token() {
        use std::os::unix::fs::PermissionsExt as _;

        let expected = format!("aren_{}", "x".repeat(43));
        let mut token = NamedTempFile::new().expect("token file");
        writeln!(token, "{expected}").expect("write token");
        token
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .expect("protect token");

        let actual = read_enrollment_token(token.path()).expect("read local token");
        assert_eq!(actual.as_str(), expected);

        token
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o640))
            .expect("change token permissions");
        assert_eq!(
            read_enrollment_token(token.path())
                .expect_err("reject group-readable token")
                .category,
            "runner_enrollment_token_permissions_invalid"
        );
    }

    #[test]
    fn runner_accepts_only_loopback_plaintext_control() {
        assert!(enrollment_endpoint("http://127.0.0.1:11080", false).is_ok());
        assert!(enrollment_endpoint("http://localhost:11080", false).is_ok());
        assert!(enrollment_endpoint("http://192.168.1.10:11080", false).is_err());
        assert!(enrollment_endpoint("http://control.example.test", false).is_err());
        assert!(enrollment_endpoint("http://192.168.1.10:11080", true).is_ok());

        let loopback: tokio_tungstenite::tungstenite::http::Uri =
            "ws://127.0.0.1:11080/api/runner/channel"
                .parse()
                .expect("URI");
        let remote: tokio_tungstenite::tungstenite::http::Uri =
            "ws://control.example.test/api/runner/channel"
                .parse()
                .expect("URI");
        assert!(control_websocket_is_allowed(&loopback, false));
        assert!(!control_websocket_is_allowed(&remote, false));
        assert!(control_websocket_is_allowed(&remote, true));
    }

    #[test]
    fn upstream_validation_rejects_http_credentials_ip_and_unlisted_hosts() {
        let allowed = BTreeSet::from(["api.openai.com".to_owned()]);
        assert!(
            validate_upstream_url(
                "https://api.openai.com/v1/models",
                &allowed,
                "api.openai.com"
            )
            .is_ok()
        );
        assert!(
            validate_upstream_url(
                "http://api.openai.com/v1/models",
                &allowed,
                "api.openai.com"
            )
            .is_err()
        );
        assert!(
            validate_upstream_url(
                "https://user:pass@api.openai.com/v1",
                &allowed,
                "api.openai.com"
            )
            .is_err()
        );
        assert!(validate_upstream_url("https://127.0.0.1/v1", &allowed, "api.openai.com").is_err());
        assert!(
            validate_upstream_url("https://evil.example/v1", &allowed, "api.openai.com").is_err()
        );
        assert!(
            validate_upstream_url(
                "https://api.openai.com/v1/models",
                &allowed,
                "auth.openai.com"
            )
            .is_err()
        );
    }

    #[test]
    fn credential_refresh_payload_is_scoped_to_one_independent_instance_revision() {
        let payload: CredentialRefreshTask = decode_exact(
            br#"{
              "credential_id":"credential_00000000000000000000000000000001",
              "expected_revision":9,
              "request":{
                "method":"POST",
                "url":"https://api.openai.com/oauth/token",
                "headers":[],
                "body_base64url":"e30"
              }
            }"#,
        )
        .expect("decode refresh payload");
        assert!(valid_credential_id(&payload.credential_id));
        assert_eq!(payload.expected_revision, 9);
        assert!(!valid_credential_id(
            "account_00000000000000000000000000000001"
        ));
    }
    #[tokio::test]
    async fn authenticated_expired_task_does_not_interrupt_other_session_work() {
        let key = ed25519_dalek::SigningKey::from_bytes(&[77; 32]);
        let mut keys = TrustedTaskKeys::new();
        keys.insert("test-key", key.verifying_key()).unwrap();
        let payload = b"{}";
        let ticket =
            aster_runner_protocol::issue_task_ticket(aster_runner_protocol::TaskTicketIssue {
                key_id: "test-key",
                signing_key: &key,
                task_id: "task-expired-001",
                runner_id: "runner-test-001",
                provider_id: "openai",
                credential_instance_id: None,
                credential_revision: None,
                upstream_host: "api.openai.com",
                command: TaskCommand::AuthorizeCredential,
                authorization: aster_runner_protocol::TaskAuthorization::AuthorizeCredential {
                    actor: aster_runner_protocol::AdminSubject {
                        identity_id: "admin-test".into(),
                    },
                    enrollment_id: "enrollment-test".into(),
                    session_expires_at: time::OffsetDateTime::now_utc().unix_timestamp() + 120,
                    license: aster_runner_protocol::TaskLicense {
                        license_id: "license-test".into(),
                        license_sha256: "ab".repeat(32),
                        expiry: aster_runner_protocol::SignedExpiry::Never {},
                    },
                },
                payload,
                nonce: "nonce-expired-001",
                now: time::OffsetDateTime::now_utc() - time::Duration::seconds(1),
                execution_timeout_ms: 1,
            })
            .unwrap();
        let config = Arc::new(RuntimeConfig {
            control_wss: "wss://unused.test".into(),
            runner_id: "runner-test-001".into(),
            credential: Zeroizing::new("test-only".into()),
            control_tls_connector: None,
            upstream_ca_certificate: None,
            task_keys: Arc::new(keys),
            max_inflight: 1,
            heartbeat: StdDuration::from_secs(10),
            allowed_upstream_hosts: Arc::new(BTreeSet::from(["api.openai.com".into()])),
        });
        let metrics = Arc::new(Metrics::default());
        let replay = Arc::new(Mutex::new(ReplayCache::new()));
        let (outbound, mut events) = mpsc::channel(4);
        let mut tasks = TaskSupervisor::new(1);
        tasks.start(
            TaskExecution {
                id: "other-task".into(),
                deadline: tokio::time::Instant::now() + StdDuration::from_secs(10),
                outbound: outbound.clone(),
                metrics: metrics.clone(),
                permit: tasks.try_permit().unwrap(),
            },
            std::future::pending(),
        );
        accept_task(
            TaskFrame {
                ticket_json_base64url: encode_base64url(&serde_json::to_vec(&ticket).unwrap()),
                payload_base64url: encode_base64url(payload),
            },
            &config,
            &metrics,
            &replay,
            &mut tasks,
            &Client::new(),
            &outbound,
        )
        .await
        .unwrap();
        let RunnerToControl::TaskFailed(failure) = events.recv().await.unwrap() else {
            panic!("expected expired task rejection")
        };
        assert_eq!(failure.task_id, "task-expired-001");
        assert_eq!(failure.category, "task_expired_before_execution");
        assert!(failure.retryable_before_upstream);
        assert_eq!(metrics.inflight.load(Ordering::Relaxed), 1);
        tasks.cancel("other-task");
        tasks.next_finished().await;
        assert_eq!(metrics.inflight.load(Ordering::Relaxed), 0);
    }
    #[tokio::test]
    async fn signed_probe_uses_task_worker_without_an_upstream_or_credential() {
        for length in [32, 31] {
            let key = ed25519_dalek::SigningKey::from_bytes(&[78; 32]);
            let mut keys = TrustedTaskKeys::new();
            keys.insert("probe-key", key.verifying_key()).unwrap();
            let payload = vec![91; length];
            let ticket =
                aster_runner_protocol::issue_task_ticket(aster_runner_protocol::TaskTicketIssue {
                    key_id: "probe-key",
                    signing_key: &key,
                    task_id: "probe-task",
                    runner_id: "runner-probe-test",
                    provider_id: aster_runner_protocol::PROBE_PROVIDER,
                    credential_instance_id: None,
                    credential_revision: None,
                    upstream_host: aster_runner_protocol::PROBE_HOST,
                    command: TaskCommand::Probe,
                    authorization: aster_runner_protocol::TaskAuthorization::Probe {},
                    payload: &payload,
                    nonce: "probe-nonce",
                    now: time::OffsetDateTime::now_utc(),
                    execution_timeout_ms: aster_runner_protocol::PROBE_TIMEOUT_MS,
                })
                .unwrap();
            let config = Arc::new(RuntimeConfig {
                control_wss: "wss://unused.test".into(),
                runner_id: "runner-probe-test".into(),
                credential: Zeroizing::new("test-only".into()),
                control_tls_connector: None,
                upstream_ca_certificate: None,
                task_keys: Arc::new(keys),
                max_inflight: 1,
                heartbeat: StdDuration::from_secs(10),
                allowed_upstream_hosts: Arc::new(BTreeSet::new()),
            });
            let metrics = Arc::new(Metrics::default());
            let replay = Arc::new(Mutex::new(ReplayCache::new()));
            let (outbound, mut events) = mpsc::channel(4);
            let mut tasks = TaskSupervisor::new(1);
            accept_task(
                TaskFrame {
                    ticket_json_base64url: encode_base64url(&serde_json::to_vec(&ticket).unwrap()),
                    payload_base64url: encode_base64url(&payload),
                },
                &config,
                &metrics,
                &replay,
                &mut tasks,
                &Client::new(),
                &outbound,
            )
            .await
            .unwrap();
            tasks.next_finished().await;
            assert!(matches!(
                events.recv().await.unwrap(),
                RunnerToControl::TaskAccepted(_)
            ));
            if length == 32 {
                let RunnerToControl::TaskChunk(chunk) = events.recv().await.unwrap() else {
                    panic!("expected challenge");
                };
                assert_eq!(decode_base64url(&chunk.data_base64url).unwrap(), payload);
                assert_eq!(chunk.sequence, 0);
                assert!(
                    matches!(events.recv().await.unwrap(), RunnerToControl::TaskFinished(result) if result.status == 204 && result.usage_json.is_none())
                );
            } else {
                assert!(
                    matches!(events.recv().await.unwrap(), RunnerToControl::TaskFailed(result) if result.category == "probe_payload_invalid")
                );
            }
            assert_eq!(metrics.inflight.load(Ordering::Relaxed), 0);
            assert!(events.try_recv().is_err());
        }
    }
}
