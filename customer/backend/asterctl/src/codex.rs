use std::{
    collections::BTreeMap,
    env,
    fs::{self, File, OpenOptions},
    io::{self, IsTerminal as _, Read as _, Write as _},
    path::{Path, PathBuf},
    process::Command,
    str::FromStr as _,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(test)]
use aster_codex_config::{ACTOR_HEADER_VALUE, PROVIDER_NAME};
use aster_codex_config::{MODEL_PROVIDER, PROVIDER, PROVIDER_ACTOR_HEADER, PROVIDER_BASE_URL};
use reqwest::{Client, StatusCode, redirect::Policy};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;
use toml_edit::{DocumentMut, Item};
use url::Url;

use crate::user_environment::{self, API_KEY_NAME};

mod catalog;

type Result<T> = std::result::Result<T, String>;

const STATE_SCHEMA: &str = "asterctl.codex-state/v1";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ManagedField {
    path: String,
    before: Option<String>,
    installed: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CodexState {
    schema: String,
    status: TransactionStatus,
    config_path: PathBuf,
    config_existed_before_first_setup: bool,
    fields: Vec<ManagedField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    catalog: Option<catalog::ManagedCatalog>,
    api_key_sha256: Option<String>,
    updated_at_unix_ms: u128,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TransactionStatus {
    Pending,
    Committed,
}

pub async fn setup(base_url: Option<&str>, set_key: bool, launch: bool) -> Result<()> {
    require_windows()?;
    if base_url.is_none() && !set_key {
        return Err("setup codex requires --base-url, --set-key, or both".to_owned());
    }

    let requested_url = base_url.map(normalize_base_url).transpose()?;
    ensure_codex_closed()?;
    let codex_cli = discover_codex_cli()?;
    let config_path = codex_config_path()?;
    let state_path = state_path()?;
    let _lock = acquire_lock(&state_path)?;
    let original_config = read_optional(&config_path)?;
    let mut document = parse_config(original_config.as_deref())?;
    let existing_url = managed_string(&document, PROVIDER_BASE_URL)?;
    let effective_url = requested_url
        .clone()
        .or(existing_url)
        .ok_or_else(|| "--set-key requires an existing Aster base_url or --base-url".to_owned())?;
    let previous_state_bytes = read_optional(&state_path)?;
    let previous_state = previous_state_bytes
        .as_deref()
        .map(parse_state)
        .transpose()?;
    if previous_state
        .as_ref()
        .is_some_and(|state| state.config_path != config_path)
    {
        return Err(format!(
            "asterctl previously managed a different Codex config at {}; remove it there before changing CODEX_HOME",
            previous_state
                .as_ref()
                .expect("state was checked")
                .config_path
                .display()
        ));
    }
    let previous_state =
        reconcile_previous_state(&document, original_config.is_some(), previous_state)?;

    let new_key = if set_key {
        let value = rpassword::prompt_password("Aster member API key: ")
            .map_err(|error| format!("could not read API key: {error}"))?;
        if let Some(value) = normalized_api_key(&value) {
            validate_key(&effective_url, value).await?;
            Some(value.to_owned())
        } else {
            if requested_url.is_some() {
                println!("No API key was entered; continuing with the URL-only update.");
                None
            } else {
                println!("No API key was entered; no changes were made.");
                return Ok(());
            }
        }
    } else {
        None
    };

    let model_catalog =
        catalog::prepare(&codex_cli, &document, &config_path, previous_state.as_ref())?;
    let mut desired = desired_values(&effective_url);
    desired.insert(
        catalog::CONFIG_FIELD.to_owned(),
        model_catalog.config_value()?.to_owned(),
    );
    let mut state = build_pending_state(
        &document,
        &config_path,
        original_config.is_some(),
        previous_state.as_ref(),
        &desired,
        new_key.as_deref(),
    )?;
    state.catalog = Some(model_catalog.state.clone());
    aster_codex_config::apply_aster_provider(&mut document, &effective_url)
        .map_err(|error| error.to_string())?;
    set_managed_string(
        &mut document,
        catalog::CONFIG_FIELD,
        model_catalog.config_value()?,
    )?;
    if !configuration_is_complete(&document)? {
        return Err("generated Codex configuration is incomplete".to_owned());
    }
    let rendered = render_config(document)?;
    let previous_key = user_environment::read_api_key();

    atomic_write_json(&state_path, &state)?;
    let verification_key = new_key.as_deref().or(previous_key.as_deref());
    let result = (|| {
        model_catalog.install()?;
        backup_and_write_config(&config_path, &rendered, original_config.as_deref())?;
        if let Some(key) = new_key.as_deref() {
            user_environment::write_api_key(key)
                .map_err(|error| format!("could not set {API_KEY_NAME}: {error}"))?;
        }
        verify_codex_provider(&codex_cli, verification_key)?;
        catalog::verify(&codex_cli, Path::new(model_catalog.config_value()?))?;
        state.status = TransactionStatus::Committed;
        catalog::commit(&mut state);
        atomic_write_json(&state_path, &state)
    })();
    if let Err(error) = result {
        rollback_setup(
            &config_path,
            original_config.as_deref(),
            &state_path,
            previous_state_bytes.as_deref(),
            previous_key.as_deref(),
            &model_catalog,
        )
        .map_err(|rollback| format!("{error}; rollback also failed: {rollback}"))?;
        return Err(error);
    }
    println!("Codex is configured to use Aster Team at {effective_url}.");
    println!(
        "Model catalog: {} ({} models, including Aster Fast entries).",
        model_catalog.config_value()?,
        model_catalog.model_count
    );
    if new_key.is_some() {
        println!("The current-user {API_KEY_NAME} environment variable was updated.");
    } else if user_environment::read_api_key().is_none() {
        println!(
            "Warning: {API_KEY_NAME} is not set; run `asterctl setup codex --set-key` before using Codex."
        );
    }
    if launch {
        println!("Starting Codex...");
        launch_codex(&codex_cli, verification_key)?;
    } else {
        println!("Open Codex and create a new task to use the updated configuration.");
    }
    Ok(())
}

fn launch_codex(cli: &Path, key: Option<&str>) -> Result<()> {
    let mut command = Command::new(cli);
    if let Some(key) = key {
        command.env(API_KEY_NAME, key);
    }
    let status = command.status().map_err(|error| {
        format!(
            "configuration succeeded, but Codex could not be started from {}: {error}",
            cli.display()
        )
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "configuration succeeded, but Codex exited with status {status}"
        ))
    }
}

pub fn status() -> Result<()> {
    let path = codex_config_path()?;
    let bytes = read_optional(&path)?;
    let document = parse_config(bytes.as_deref())?;
    let provider = managed_string(&document, MODEL_PROVIDER)?;
    let base_url = managed_string(&document, PROVIDER_BASE_URL)?;
    let key = user_environment::read_api_key();
    let actor_header = managed_string(&document, PROVIDER_ACTOR_HEADER)?;
    let model_catalog = catalog::inspect(&document, &path);
    let complete = configuration_is_complete(&document)? && key.is_some() && model_catalog.is_ok();
    println!("Codex config: {}", path.display());
    println!(
        "Provider: {}",
        provider.as_deref().unwrap_or("<not configured>")
    );
    println!(
        "Base URL: {}",
        base_url.as_deref().unwrap_or("<not configured>")
    );
    println!(
        "{API_KEY_NAME}: {}",
        key.as_deref()
            .map(mask_key)
            .unwrap_or_else(|| "<not set>".to_owned())
    );
    println!(
        "Actor header: {}",
        actor_header.as_deref().unwrap_or("<not configured>")
    );
    println!("Status: {}", if complete { "ready" } else { "incomplete" });
    match model_catalog {
        Ok((path, count)) => println!("Model catalog: {} ({count} models)", path.display()),
        Err(error) => println!("Model catalog: {error}"),
    }
    if complete {
        Ok(())
    } else {
        Err("Codex Aster configuration is incomplete".to_owned())
    }
}

pub async fn doctor() -> Result<()> {
    require_windows()?;
    let path = codex_config_path()?;
    let bytes = read_optional(&path)?;
    let document = parse_config(bytes.as_deref())?;
    if !configuration_is_complete(&document)? {
        return Err(format!(
            "Aster provider configuration is incomplete in {}",
            path.display()
        ));
    }
    println!("[PASS] Codex configuration: {}", path.display());
    let base_url = managed_string(&document, PROVIDER_BASE_URL)?
        .ok_or_else(|| "Aster base_url is missing".to_owned())?;
    if Url::parse(&base_url).is_ok_and(|parsed| parsed.scheme() == "http") {
        println!("[WARN] Aster API transport is unencrypted HTTP.");
    }
    let stored_key = user_environment::read_api_key()
        .ok_or_else(|| format!("{API_KEY_NAME} is not set for the current user"))?;
    let key = normalized_api_key(&stored_key)
        .ok_or_else(|| format!("{API_KEY_NAME} is empty for the current user"))?;
    println!("[PASS] {API_KEY_NAME}: {}", mask_key(key));
    validate_key(&base_url, key).await?;
    println!("[PASS] Aster models endpoint: authenticated");
    let cli = discover_codex_cli()?;
    verify_codex_provider(&cli, Some(key))?;
    println!("[PASS] Codex CLI provider: {PROVIDER}");
    let (catalog_path, count) = catalog::inspect(&document, &path)?;
    catalog::verify(&cli, &catalog_path)?;
    println!(
        "[PASS] Codex model catalog: {} ({count} models)",
        catalog_path.display()
    );
    println!("Result: diagnostics passed.");
    Ok(())
}

pub fn remove() -> Result<()> {
    require_windows()?;
    ensure_codex_closed()?;
    let state_path = state_path()?;
    let _lock = acquire_lock(&state_path)?;
    let state_bytes = read_optional(&state_path)?.ok_or_else(|| {
        "no asterctl Codex state was found; refusing to guess which fields to remove".to_owned()
    })?;
    let mut state = parse_state(&state_bytes)?;
    let config_bytes = read_optional(&state.config_path)?;
    let mut document = parse_config(config_bytes.as_deref())?;
    let mut conflicts = Vec::new();
    let mut remaining = Vec::new();
    let catalog_conflict = catalog::removal_conflict(&state)?;
    if let Some(error) = &catalog_conflict {
        conflicts.push(error.clone());
    }

    for field in state.fields.iter().rev() {
        if field.path == catalog::CONFIG_FIELD && catalog_conflict.is_some() {
            remaining.push(field.clone());
            continue;
        }
        let current = managed_string(&document, &field.path)?;
        if current.as_deref() == Some(field.installed.as_str()) {
            match field.before.as_deref() {
                Some(value) => set_managed_string(&mut document, &field.path, value)?,
                None => remove_managed_field(&mut document, &field.path)?,
            }
        } else if field_is_already_restored(current.as_deref(), field) {
            // A previous interrupted remove may already have restored this field.
        } else {
            conflicts.push(field.path.clone());
            remaining.push(field.clone());
        }
    }
    prune_empty_aster_tables(&mut document);
    let rendered = render_config(document)?;
    backup_and_write_config(&state.config_path, &rendered, config_bytes.as_deref())?;
    if catalog_conflict.is_none()
        && !remaining
            .iter()
            .any(|field| field.path == catalog::CONFIG_FIELD)
    {
        catalog::remove_file(&mut state)?;
    }

    if conflicts.is_empty() {
        fs::remove_file(&state_path)
            .map_err(|error| format!("could not remove {}: {error}", state_path.display()))?;
        println!("Aster-managed Codex fields were removed. {API_KEY_NAME} was kept.");
        Ok(())
    } else {
        remaining.reverse();
        state.fields = remaining;
        state.status = TransactionStatus::Committed;
        state.updated_at_unix_ms = unix_time_ms()?;
        atomic_write_json(&state_path, &state)?;
        println!("Fields changed after setup were preserved:");
        for path in &conflicts {
            println!("  - {path}");
        }
        Err("some Aster-managed fields had conflicts and were not removed".to_owned())
    }
}

fn field_is_already_restored(current: Option<&str>, field: &ManagedField) -> bool {
    current == field.before.as_deref()
}

fn require_windows() -> Result<()> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Ok(())
    } else {
        Err("Codex setup is currently supported on Windows x64 only".to_owned())
    }
}

fn desired_values(base_url: &str) -> BTreeMap<String, String> {
    aster_codex_config::desired_values(base_url)
}

fn reconcile_previous_state(
    document: &DocumentMut,
    config_exists: bool,
    previous: Option<CodexState>,
) -> Result<Option<CodexState>> {
    let Some(state) = previous else {
        return Ok(None);
    };
    // State is stored outside CODEX_HOME, so it can survive deletion of the
    // entire .codex directory. A newly created config is a new setup baseline.
    let configured_catalog = managed_string(document, catalog::CONFIG_FIELD)?;
    let catalog_is_installed = state
        .fields
        .iter()
        .find(|field| field.path == catalog::CONFIG_FIELD)
        .is_some_and(|field| configured_catalog.as_deref() == Some(field.installed.as_str()));
    if !config_exists
        || (managed_string(document, MODEL_PROVIDER)?.as_deref() != Some(PROVIDER)
            && !catalog_is_installed)
    {
        return Ok(None);
    }
    Ok(Some(state))
}

fn build_pending_state(
    document: &DocumentMut,
    config_path: &Path,
    config_exists: bool,
    previous: Option<&CodexState>,
    desired: &BTreeMap<String, String>,
    new_key: Option<&str>,
) -> Result<CodexState> {
    if let Some(state) = previous
        && state.schema != STATE_SCHEMA
    {
        return Err("the existing asterctl state uses an unsupported schema".to_owned());
    }
    let previous_fields = previous
        .map(|state| {
            state
                .fields
                .iter()
                .map(|field| (field.path.as_str(), field))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let mut fields = Vec::with_capacity(desired.len());
    for (path, installed) in desired {
        let before = if let Some(field) = previous_fields.get(path.as_str()) {
            field.before.clone()
        } else {
            managed_string(document, path)?
        };
        fields.push(ManagedField {
            path: path.to_owned(),
            before,
            installed: installed.clone(),
        });
    }
    Ok(CodexState {
        schema: STATE_SCHEMA.to_owned(),
        status: TransactionStatus::Pending,
        config_path: config_path.to_path_buf(),
        config_existed_before_first_setup: previous
            .map(|state| state.config_existed_before_first_setup)
            .unwrap_or(config_exists),
        fields,
        catalog: None,
        api_key_sha256: new_key
            .map(sha256_text)
            .or_else(|| previous.and_then(|state| state.api_key_sha256.clone())),
        updated_at_unix_ms: unix_time_ms()?,
    })
}

fn configuration_is_complete(document: &DocumentMut) -> Result<bool> {
    let expected = desired_values(
        managed_string(document, PROVIDER_BASE_URL)?
            .as_deref()
            .unwrap_or(""),
    );
    if expected[PROVIDER_BASE_URL].is_empty() {
        return Ok(false);
    }
    for (path, value) in expected {
        if managed_string(document, &path)?.as_deref() != Some(value.as_str()) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn parse_config(bytes: Option<&[u8]>) -> Result<DocumentMut> {
    match bytes {
        None => Ok(DocumentMut::new()),
        Some(bytes) => {
            let text = std::str::from_utf8(bytes)
                .map_err(|_| "Codex config.toml is not valid UTF-8".to_owned())?;
            DocumentMut::from_str(text)
                .map_err(|error| format!("Codex config.toml is invalid: {error}"))
        }
    }
}

fn render_config(mut document: DocumentMut) -> Result<Vec<u8>> {
    if managed_string(&document, MODEL_PROVIDER)?.as_deref() == Some(PROVIDER) {
        document.as_table_mut().sort_values_by(|left, _, right, _| {
            let priority = |key: &str| match key {
                MODEL_PROVIDER => 0,
                catalog::CONFIG_FIELD => 1,
                _ => 2,
            };
            priority(left.get()).cmp(&priority(right.get()))
        });
    }
    aster_codex_config::render_document(document).map_err(|error| error.to_string())
}

fn managed_string(document: &DocumentMut, path: &str) -> Result<Option<String>> {
    aster_codex_config::managed_string(document, path).map_err(|error| error.to_string())
}

fn set_managed_string(document: &mut DocumentMut, path: &str, installed: &str) -> Result<()> {
    aster_codex_config::set_managed_string(document, path, installed)
        .map_err(|error| error.to_string())
}

fn remove_managed_field(document: &mut DocumentMut, path: &str) -> Result<()> {
    let segments = path.split('.').collect::<Vec<_>>();
    let (last, parents) = segments
        .split_last()
        .ok_or_else(|| "managed path is empty".to_owned())?;
    let mut table: &mut dyn toml_edit::TableLike = document.as_table_mut();
    for segment in parents {
        let Some(item) = table.get_mut(segment) else {
            return Ok(());
        };
        let Some(next) = item.as_table_like_mut() else {
            return Ok(());
        };
        table = next;
    }
    table.remove(last);
    Ok(())
}

fn prune_empty_aster_tables(document: &mut DocumentMut) {
    let mut remove_providers = false;
    if let Some(providers) = document
        .as_table_mut()
        .get_mut("model_providers")
        .and_then(Item::as_table_like_mut)
    {
        let mut remove_aster = false;
        if let Some(aster) = providers
            .get_mut(PROVIDER)
            .and_then(Item::as_table_like_mut)
        {
            if aster
                .get("http_headers")
                .and_then(Item::as_table_like)
                .is_some_and(toml_edit::TableLike::is_empty)
            {
                aster.remove("http_headers");
            }
            remove_aster = aster.is_empty();
        }
        if remove_aster {
            providers.remove(PROVIDER);
        }
        remove_providers = providers.is_empty();
    }
    if remove_providers {
        document.as_table_mut().remove("model_providers");
    }
}

fn normalize_base_url(value: &str) -> Result<String> {
    let value = aster_codex_config::normalize_base_url(value).map_err(|error| error.to_string())?;
    if value.starts_with("http://") {
        println!("Warning: the configured API connection uses unencrypted HTTP.");
    }
    Ok(value)
}

async fn validate_key(base_url: &str, key: &str) -> Result<()> {
    let key = normalized_api_key(key).ok_or_else(|| "Aster member API key is empty".to_owned())?;
    let client = Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("could not initialize HTTP client: {error}"))?;
    let response = client
        .get(format!("{}/models", base_url.trim_end_matches('/')))
        .bearer_auth(key)
        .send()
        .await
        .map_err(|error| format!("could not reach the Aster models endpoint: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(validation_response_error(status, &body));
    }
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("Aster models response is invalid JSON: {error}"))?;
    let has_model = body
        .get("data")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|models| !models.is_empty());
    if !has_model {
        return Err("Aster models endpoint returned no available models".to_owned());
    }
    Ok(())
}

fn normalized_api_key(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn validation_response_error(status: StatusCode, body: &str) -> String {
    let detail = aster_error_label(body)
        .map(|value| format!(" ({value})"))
        .unwrap_or_default();
    if status == StatusCode::UNAUTHORIZED {
        return format!(
            "Aster rejected the member API key with HTTP {status}{detail}; use an active API key created by this Aster instance"
        );
    }
    if matches!(
        status,
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
    ) {
        return format!(
            "Aster does not expose a compatible /v1/models endpoint (HTTP {status}{detail}); update the Aster server or verify --base-url"
        );
    }
    format!("Aster models endpoint returned HTTP {status}{detail}")
}

fn aster_error_label(body: &str) -> Option<String> {
    let error = serde_json::from_str::<serde_json::Value>(body)
        .ok()?
        .get("error")?
        .clone();
    let code = error.get("code")?.as_str()?.trim();
    if code.is_empty()
        || code.len() > 64
        || !code
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '_' | '-' | '.'))
    {
        return None;
    }
    match error.get("number").and_then(serde_json::Value::as_i64) {
        Some(number) => Some(format!("{code}/{number}")),
        None => Some(code.to_owned()),
    }
}

fn codex_home() -> Result<PathBuf> {
    if let Some(path) = env::var_os("CODEX_HOME").filter(|value| !value.is_empty()) {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err("CODEX_HOME must be an absolute path".to_owned());
        }
        return Ok(path);
    }
    let profile =
        env::var_os("USERPROFILE").ok_or_else(|| "USERPROFILE is unavailable".to_owned())?;
    Ok(PathBuf::from(profile).join(".codex"))
}

fn codex_config_path() -> Result<PathBuf> {
    Ok(codex_home()?.join("config.toml"))
}

fn state_path() -> Result<PathBuf> {
    let local =
        env::var_os("LOCALAPPDATA").ok_or_else(|| "LOCALAPPDATA is unavailable".to_owned())?;
    Ok(PathBuf::from(local)
        .join("AsterTeam")
        .join("asterctl")
        .join("codex-state.json"))
}

fn acquire_lock(state_path: &Path) -> Result<File> {
    let directory = state_path
        .parent()
        .ok_or_else(|| "asterctl state path has no parent".to_owned())?;
    fs::create_dir_all(directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    let lock_path = directory.join("codex.lock");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| format!("could not open {}: {error}", lock_path.display()))?;
    lock.try_lock()
        .map_err(|error| format!("another asterctl process is modifying Codex: {error}"))?;
    Ok(lock)
}

fn ensure_codex_closed() -> Result<()> {
    if !codex_is_running()? {
        return Ok(());
    }
    if !io::stdin().is_terminal() {
        return Err("Codex is running; fully exit it before setup or remove".to_owned());
    }
    println!("Codex is running. Fully exit Codex, then press Enter to check again.");
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|error| format!("could not read confirmation: {error}"))?;
    if codex_is_running()? {
        Err("Codex is still running; no changes were made".to_owned())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn codex_is_running() -> Result<bool> {
    let output = Command::new("tasklist.exe")
        .args(["/FO", "CSV", "/NH"])
        .output()
        .map_err(|error| format!("could not inspect running processes: {error}"))?;
    if !output.status.success() {
        return Err("tasklist failed while checking whether Codex is running".to_owned());
    }
    let text = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    Ok(text.lines().any(|line| {
        line.starts_with("\"chatgpt.exe\"")
            || line.starts_with("\"codex.exe\"")
            || line.starts_with("\"codex app.exe\"")
    }))
}

#[cfg(not(windows))]
fn codex_is_running() -> Result<bool> {
    Ok(false)
}

fn discover_codex_cli() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        if let Some(local) = env::var_os("LOCALAPPDATA") {
            let bin = PathBuf::from(local)
                .join("OpenAI")
                .join("Codex")
                .join("bin");
            if let Some(path) = newest_codex_cli_in(&bin) {
                return verify_codex_executable(path);
            }
        }
        if let Ok(output) = Command::new("where.exe").arg("codex.exe").output()
            && output.status.success()
            && let Some(path) = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(PathBuf::from)
                .find(|path| path.is_file())
        {
            return verify_codex_executable(path);
        }
    }
    Err("the Codex CLI bundled with Codex Desktop could not be found".to_owned())
}

#[cfg(windows)]
fn newest_codex_cli_in(bin: &Path) -> Option<PathBuf> {
    let mut candidates = fs::read_dir(bin)
        .ok()?
        .filter_map(std::result::Result::ok)
        .map(|entry| {
            entry
                .path()
                .join(if cfg!(windows) { "codex.exe" } else { "codex" })
        })
        .filter(|path| path.is_file())
        .map(|path| {
            let modified = fs::metadata(&path)
                .and_then(|metadata| metadata.modified())
                .unwrap_or(UNIX_EPOCH);
            (modified, path)
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.pop().map(|(_, path)| path)
}

#[cfg(windows)]
fn verify_codex_executable(path: PathBuf) -> Result<PathBuf> {
    let output = Command::new(&path)
        .arg("--version")
        .output()
        .map_err(|error| format!("could not run {}: {error}", path.display()))?;
    if !output.status.success() || !String::from_utf8_lossy(&output.stdout).contains("codex-cli") {
        return Err(format!("{} is not a compatible Codex CLI", path.display()));
    }
    Ok(path)
}

fn verify_codex_provider(cli: &Path, key: Option<&str>) -> Result<()> {
    let mut command = Command::new(cli);
    command.args(["doctor", "--json"]);
    if let Some(key) = key {
        command.env(API_KEY_NAME, key);
    }
    let output = command
        .output()
        .map_err(|error| format!("could not run Codex doctor: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Codex doctor failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let report: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Codex doctor returned invalid JSON: {error}"))?;
    let provider = report
        .pointer("/checks/config.load/details/model provider")
        .and_then(serde_json::Value::as_str);
    if provider != Some(PROVIDER) {
        return Err(format!(
            "Codex loaded provider {}, expected {PROVIDER}",
            provider.unwrap_or("<unknown>")
        ));
    }
    Ok(())
}

fn backup_and_write_config(path: &Path, bytes: &[u8], previous: Option<&[u8]>) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| "Codex config path has no parent".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    if let Some(previous) = previous {
        let timestamp = unix_time_ms()?;
        let backup = (0_u16..1000)
            .map(|sequence| {
                let suffix = if sequence == 0 {
                    String::new()
                } else {
                    format!("-{sequence:03}")
                };
                parent.join(format!("config.toml.asterctl-{timestamp}{suffix}.bak"))
            })
            .find(|candidate| !candidate.exists())
            .ok_or_else(|| "could not allocate a unique Codex config backup".to_owned())?;
        write_new_file(&backup, previous)?;
    }
    atomic_write(path, bytes)
}

fn rollback_setup(
    config_path: &Path,
    previous_config: Option<&[u8]>,
    state_path: &Path,
    previous_state: Option<&[u8]>,
    previous_key: Option<&str>,
    model_catalog: &catalog::PreparedCatalog,
) -> Result<()> {
    restore_optional_file(config_path, previous_config)?;
    model_catalog.rollback()?;
    user_environment::restore_api_key(previous_key).map_err(|error| {
        format!("setup failed and {API_KEY_NAME} rollback also failed: {error}")
    })?;
    restore_optional_file(state_path, previous_state)
}

fn restore_optional_file(path: &Path, bytes: Option<&[u8]>) -> Result<()> {
    if let Some(bytes) = bytes {
        atomic_write(path, bytes)
    } else {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("could not remove {}: {error}", path.display())),
        }
    }
}

fn atomic_write_json(path: &Path, value: &CodexState) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("could not encode asterctl state: {error}"))?;
    bytes.push(b'\n');
    atomic_write(path, &bytes)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    let permissions = fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    let mut temporary = NamedTempFile::new_in(parent).map_err(|error| {
        format!(
            "could not create a temporary file in {}: {error}",
            parent.display()
        )
    })?;
    temporary
        .write_all(bytes)
        .and_then(|()| temporary.flush())
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|error| {
            format!(
                "could not write temporary file for {}: {error}",
                path.display()
            )
        })?;
    if let Some(permissions) = permissions {
        temporary
            .as_file()
            .set_permissions(permissions)
            .map_err(|error| {
                format!(
                    "could not preserve permissions for {}: {error}",
                    path.display()
                )
            })?;
    }
    temporary
        .persist(path)
        .map_err(|error| format!("could not replace {}: {}", path.display(), error.error))?;
    Ok(())
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("could not create {}: {error}", path.display()))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match File::open(path) {
        Ok(mut file) => {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)
                .map_err(|error| format!("could not read {}: {error}", path.display()))?;
            Ok(Some(bytes))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("could not open {}: {error}", path.display())),
    }
}

fn parse_state(bytes: &[u8]) -> Result<CodexState> {
    let state: CodexState = serde_json::from_slice(bytes)
        .map_err(|error| format!("asterctl state is invalid: {error}"))?;
    if state.schema != STATE_SCHEMA {
        return Err("asterctl state uses an unsupported schema".to_owned());
    }
    Ok(state)
}

fn sha256_text(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn unix_time_ms() -> Result<u128> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(|_| "system clock is before the Unix epoch".to_owned())
}

fn mask_key(key: &str) -> String {
    let characters = key.chars().collect::<Vec<_>>();
    if characters.len() <= 10 {
        return "***".to_owned();
    }
    let prefix = characters[..6].iter().collect::<String>();
    let suffix = characters[characters.len() - 4..]
        .iter()
        .collect::<String>();
    format!("{prefix}***{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SANITIZED_DESKTOP_CONFIG: &str =
        include_str!("../tests/fixtures/codex-desktop-sanitized.toml");

    fn configured_document(source: &str, base_url: &str) -> DocumentMut {
        let mut document = DocumentMut::from_str(source).expect("source config");
        for (path, value) in desired_values(base_url) {
            set_managed_string(&mut document, &path, &value).expect("set field");
        }
        let rendered = render_config(document).expect("render config");
        DocumentMut::from_str(std::str::from_utf8(&rendered).expect("UTF-8"))
            .expect("rendered config")
    }

    #[test]
    fn provider_configuration_is_top_level_and_preserves_desktop_fields() {
        let document = configured_document(
            r#"[desktop]
conversationDetailMode = "STEPS_COMMANDS"
notify = ["turn-ended"]
"#,
            "http://10.13.74.140:11080/v1",
        );
        let rendered = document.to_string();
        assert!(rendered.starts_with("model_provider = \"aster\""));
        assert!(rendered.contains("[desktop]"));
        assert!(rendered.contains("notify = [\"turn-ended\"]"));
        assert_eq!(
            managed_string(&document, PROVIDER_ACTOR_HEADER).expect("header"),
            Some(ACTOR_HEADER_VALUE.to_owned())
        );
    }

    #[test]
    fn setup_places_provider_and_catalog_before_existing_root_settings() {
        let mut document = DocumentMut::from_str(SANITIZED_DESKTOP_CONFIG).expect("desktop config");
        aster_codex_config::apply_aster_provider(&mut document, "https://api.example.test/v1")
            .expect("apply provider");
        set_managed_string(
            &mut document,
            catalog::CONFIG_FIELD,
            "C:/Users/member/.codex/aster-models.json",
        )
        .expect("catalog path");

        let rendered = String::from_utf8(render_config(document).expect("render config"))
            .expect("UTF-8 config");
        let mut lines = rendered.lines();
        assert_eq!(lines.next(), Some("model_provider = \"aster\""));
        assert_eq!(
            lines.next(),
            Some("model_catalog_json = \"C:/Users/member/.codex/aster-models.json\"")
        );
        assert!(rendered.contains("service_tier = \"priority\""));
        assert!(rendered.contains("[mcp_servers.codex_apps]"));
        assert_eq!(
            managed_string(
                &DocumentMut::from_str(&rendered).expect("valid TOML"),
                MODEL_PROVIDER
            )
            .expect("provider"),
            Some(PROVIDER.to_owned())
        );
    }

    #[test]
    fn sanitized_desktop_config_survives_setup_update_and_remove() {
        let original = DocumentMut::from_str(SANITIZED_DESKTOP_CONFIG).expect("fixture config");
        let first_desired = desired_values("https://api.example.test/v1");
        let first_state = build_pending_state(
            &original,
            Path::new("C:\\Users\\member\\.codex\\config.toml"),
            true,
            None,
            &first_desired,
            None,
        )
        .expect("capture original values");

        let mut installed = original.clone();
        for (path, value) in first_desired {
            set_managed_string(&mut installed, &path, &value).expect("first setup");
        }
        let updated_desired = desired_values("https://new-api.example.test/v1");
        let state = build_pending_state(
            &installed,
            Path::new("C:\\Users\\member\\.codex\\config.toml"),
            true,
            Some(&first_state),
            &updated_desired,
            None,
        )
        .expect("retain first setup baseline");
        for (path, value) in updated_desired {
            set_managed_string(&mut installed, &path, &value).expect("idempotent URL update");
        }
        let rendered = render_config(installed).expect("render configured fixture");
        let mut installed = DocumentMut::from_str(
            std::str::from_utf8(&rendered).expect("configured fixture is UTF-8"),
        )
        .expect("configured fixture reparses");

        assert_eq!(
            managed_string(&installed, PROVIDER_BASE_URL).expect("base URL"),
            Some("https://new-api.example.test/v1".to_owned())
        );
        assert!(installed["desktop"].get(MODEL_PROVIDER).is_none());
        assert_eq!(installed["model"].as_str(), Some("gpt-example"));
        assert_eq!(
            installed["desktop"]["followUpQueueMode"].as_str(),
            Some("queue")
        );
        assert_eq!(
            installed["mcp_servers"]["codex_apps"]["env"]["CODEX_HOME"].as_str(),
            Some("C:\\Users\\member\\.codex")
        );
        assert_eq!(
            installed["plugins"]["browser@openai-bundled"]["enabled"].as_bool(),
            Some(true)
        );
        assert_eq!(
            installed["notify"].as_array().map(|value| value.len()),
            Some(2)
        );

        // Removal restores only the values captured before the first setup. The
        // fixture's unrelated Desktop-managed and user-managed data must remain.
        for field in state.fields.iter().rev() {
            if managed_string(&installed, &field.path)
                .expect("current managed value")
                .as_deref()
                == Some(field.installed.as_str())
            {
                if let Some(before) = field.before.as_deref() {
                    set_managed_string(&mut installed, &field.path, before).expect("restore field");
                } else {
                    remove_managed_field(&mut installed, &field.path).expect("remove field");
                }
            }
        }
        prune_empty_aster_tables(&mut installed);
        assert_eq!(installed.to_string(), original.to_string());
    }

    #[test]
    fn setup_is_idempotent_and_preserves_unknown_provider_fields() {
        let mut document = configured_document("", "https://api.example.com/v1");
        set_managed_string(
            &mut document,
            "model_providers.aster.request_max_retries",
            "7",
        )
        .expect("unknown field");
        for (path, value) in desired_values("https://new.example.com/v1") {
            set_managed_string(&mut document, &path, &value).expect("repeat setup");
        }
        assert_eq!(
            managed_string(&document, "model_providers.aster.request_max_retries")
                .expect("unknown field"),
            Some("7".to_owned())
        );
        assert_eq!(
            managed_string(&document, PROVIDER_BASE_URL).expect("base URL"),
            Some("https://new.example.com/v1".to_owned())
        );
    }

    #[test]
    fn removal_restores_only_matching_managed_values() {
        let original =
            DocumentMut::from_str("model_provider = \"openai\"\nnotify = [\"turn-ended\"]\n")
                .expect("original");
        let desired = desired_values("https://api.example.com/v1");
        let state = build_pending_state(
            &original,
            Path::new("C:\\Users\\member\\.codex\\config.toml"),
            true,
            None,
            &desired,
            None,
        )
        .expect("state");
        let mut installed = original.clone();
        for (path, value) in desired {
            set_managed_string(&mut installed, &path, &value).expect("install");
        }
        set_managed_string(&mut installed, PROVIDER_NAME, "Member override").expect("override");
        for field in state.fields.iter().rev() {
            if managed_string(&installed, &field.path)
                .expect("current")
                .as_deref()
                == Some(field.installed.as_str())
            {
                if let Some(before) = field.before.as_deref() {
                    set_managed_string(&mut installed, &field.path, before).expect("restore");
                } else {
                    remove_managed_field(&mut installed, &field.path).expect("remove");
                }
            }
        }
        prune_empty_aster_tables(&mut installed);
        assert_eq!(
            managed_string(&installed, MODEL_PROVIDER).expect("provider"),
            Some("openai".to_owned())
        );
        assert_eq!(
            managed_string(&installed, PROVIDER_NAME).expect("name"),
            Some("Member override".to_owned())
        );
        assert!(installed.to_string().contains("notify = [\"turn-ended\"]"));
    }

    #[test]
    fn interrupted_removal_recognizes_fields_that_are_already_restored() {
        let original = DocumentMut::from_str("model_provider = \"openai\"\n").expect("original");
        let desired = desired_values("https://api.example.com/v1");
        let state = build_pending_state(
            &original,
            Path::new("C:\\Users\\member\\.codex\\config.toml"),
            true,
            None,
            &desired,
            None,
        )
        .expect("state");
        let current = original;
        for field in &state.fields {
            let current_value = managed_string(&current, &field.path).expect("current value");
            assert!(field_is_already_restored(current_value.as_deref(), field));
        }
    }

    #[test]
    fn base_url_validation_requires_v1_and_rejects_embedded_credentials() {
        assert_eq!(
            normalize_base_url("https://api.example.com/v1/").expect("valid URL"),
            "https://api.example.com/v1"
        );
        assert!(normalize_base_url("https://api.example.com").is_err());
        assert!(normalize_base_url("https://user:pass@api.example.com/v1").is_err());
        assert!(normalize_base_url("https://api.example.com/v1?x=1").is_err());
    }

    #[test]
    fn key_mask_never_prints_a_short_secret() {
        assert_eq!(mask_key("ask_short"), "***");
        assert_eq!(mask_key("ask_1234567890"), "ask_12***7890");
    }

    #[test]
    fn api_key_normalization_trims_clipboard_whitespace() {
        assert_eq!(normalized_api_key("  ask_example\r\n"), Some("ask_example"));
        assert_eq!(normalized_api_key(" \t\r\n"), None);
    }

    #[test]
    fn validation_errors_distinguish_credentials_from_server_compatibility() {
        let unauthorized = validation_response_error(
            StatusCode::UNAUTHORIZED,
            r#"{"error":{"code":"INVALID_API_KEY","number":31001}}"#,
        );
        assert!(unauthorized.contains("member API key"));
        assert!(unauthorized.contains("INVALID_API_KEY/31001"));

        let missing = validation_response_error(StatusCode::NOT_FOUND, "not found");
        assert!(missing.contains("compatible /v1/models endpoint"));
    }

    #[test]
    fn atomic_write_replaces_an_existing_file() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("config.toml");
        fs::write(&path, b"before").expect("initial file");
        atomic_write(&path, b"after").expect("atomic replacement");
        assert_eq!(fs::read(path).expect("replacement"), b"after");
    }

    #[test]
    fn legacy_state_without_catalog_can_be_upgraded() {
        let document = DocumentMut::new();
        let config = Path::new("C:/Users/member/.codex/config.toml");
        let state = build_pending_state(
            &document,
            config,
            true,
            None,
            &desired_values("https://example.test/v1"),
            None,
        )
        .unwrap();
        let bytes = serde_json::to_vec(&state).unwrap();
        assert!(
            serde_json::from_slice::<serde_json::Value>(&bytes)
                .unwrap()
                .get("catalog")
                .is_none()
        );
        let legacy = parse_state(&bytes).unwrap();
        let mut desired = desired_values("https://example.test/v1");
        desired.insert(
            catalog::CONFIG_FIELD.to_owned(),
            "C:/Users/member/.codex/aster-models.json".to_owned(),
        );
        let updated =
            build_pending_state(&document, config, true, Some(&legacy), &desired, None).unwrap();
        let field = updated
            .fields
            .iter()
            .find(|field| field.path == catalog::CONFIG_FIELD)
            .unwrap();
        assert!(field.before.is_none());
        assert_eq!(updated.fields.len(), legacy.fields.len() + 1);
    }
}
