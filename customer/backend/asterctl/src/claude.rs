use std::{
    fs::{self, File},
    io::{self, IsTerminal as _, Read as _, Write as _},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use aster_claude_config::{
    ClaudeCliSettingsResponse, MINIMUM_VERSION, SETTINGS_SCHEMA, parse_claude_version,
    render_project_settings, supports_version,
};
use reqwest::{Client, StatusCode, redirect::Policy};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;
use url::Url;

type Result<T> = std::result::Result<T, String>;

const STATE_SCHEMA: &str = "asterctl.claude-state/v1";
const SETTINGS_RELATIVE_PATH: &str = ".claude/settings.local.json";
const STATE_FILE_NAME: &str = ".asterctl-state.json";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ClaudeState {
    schema: String,
    config_path: PathBuf,
    installed_sha256: String,
    backup_path: Option<PathBuf>,
    created_config: bool,
    claude_version: String,
    updated_at_unix_ms: u128,
}

pub async fn setup(base_url: &str, project: &Path, set_key: bool, launch: bool) -> Result<()> {
    if !set_key {
        return Err("setup claude requires --set-key so the member API key is not stored in command history".to_owned());
    }
    let base_url = normalize_base_url(base_url)?;
    let (version, _) = discover_claude_version()?;
    if !supports_version(&version) {
        return Err(format!(
            "Claude Code {version} is unsupported. Run `claude update` and retry after upgrading to {MINIMUM_VERSION} or later; no files were changed."
        ));
    }
    println!("Detected Claude Code {version}.");
    let project = prepare_project(project)?;
    let config_path = project.join(SETTINGS_RELATIVE_PATH);
    let state_path = project.join(".claude").join(STATE_FILE_NAME);
    let original = read_optional(&config_path)?;
    if original.is_some() && !confirm_overwrite(&config_path)? {
        println!("No changes were made.");
        println!(
            "Configure {} manually if you want to keep the existing file.",
            config_path.display()
        );
        return Ok(());
    }

    let key = rpassword::prompt_password("Aster member API key: ")
        .map_err(|error| format!("could not read API key: {error}"))?;
    let key = normalized_api_key(&key)
        .ok_or_else(|| "no API key was entered; no changes were made".to_owned())?;
    let settings = fetch_settings(&base_url, key, &version).await?;
    let rendered = render_project_settings(
        base_url.as_str().trim_end_matches('/'),
        key,
        &settings.model_mappings,
        &version,
    )?;

    let directory = config_path
        .parent()
        .ok_or_else(|| "Claude settings path has no parent directory".to_owned())?;
    fs::create_dir_all(directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    let backup_path = original
        .as_deref()
        .map(|bytes| write_backup(&config_path, bytes))
        .transpose()?;
    if let Err(error) = atomic_write(&config_path, &rendered) {
        if let Some(path) = backup_path.as_deref() {
            let _ = fs::remove_file(path);
        }
        return Err(error);
    }
    let state = ClaudeState {
        schema: STATE_SCHEMA.to_owned(),
        config_path: config_path.clone(),
        installed_sha256: sha256_hex(&rendered),
        backup_path: backup_path.clone(),
        created_config: original.is_none(),
        claude_version: version.to_string(),
        updated_at_unix_ms: now_unix_ms()?,
    };
    if let Err(error) = atomic_write_json(&state_path, &state) {
        restore_after_failed_setup(&config_path, original.as_deref());
        if let Some(path) = backup_path.as_deref() {
            let _ = fs::remove_file(path);
        }
        return Err(error);
    }

    println!("Configured Claude CLI for {}.", user_facing_path(&project));
    println!("Settings: {}", user_facing_path(&config_path));
    for (alias, model) in settings.model_mappings.entries() {
        println!("  {alias} -> {model}");
    }
    if launch {
        println!("Starting Claude in the configured project...");
        launch_claude(&project)?;
    } else {
        println!("Run claude from that project directory to start a new session.");
    }
    Ok(())
}

pub fn status(project: &Path) -> Result<()> {
    let project = canonical_project(project)?;
    let config_path = project.join(SETTINGS_RELATIVE_PATH);
    let state_path = project.join(".claude").join(STATE_FILE_NAME);
    let Some(state_bytes) = read_optional(&state_path)? else {
        return Err(format!(
            "asterctl has not configured Claude CLI for {}; run setup claude first",
            project.display()
        ));
    };
    let state = parse_state(&state_bytes)?;
    if state.config_path != config_path {
        return Err("Claude asterctl state points to a different settings file".to_owned());
    }
    let current = fs::read(&config_path)
        .map_err(|error| format!("could not read {}: {error}", config_path.display()))?;
    if sha256_hex(&current) != state.installed_sha256 {
        return Err(format!(
            "{} changed after asterctl setup; inspect it manually",
            config_path.display()
        ));
    }
    println!("Claude CLI project configuration is managed and unchanged.");
    println!("Project: {}", user_facing_path(&project));
    println!("Claude Code version at setup: {}", state.claude_version);
    Ok(())
}

pub async fn doctor(project: &Path) -> Result<()> {
    status(project)?;
    let project = canonical_project(project)?;
    let config_path = project.join(SETTINGS_RELATIVE_PATH);
    let bytes = fs::read(&config_path)
        .map_err(|error| format!("could not read {}: {error}", config_path.display()))?;
    let document: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("{} is not valid JSON: {error}", config_path.display()))?;
    let environment = document
        .get("env")
        .and_then(Value::as_object)
        .ok_or_else(|| "Claude settings do not contain an env object".to_owned())?;
    let base_url = environment
        .get("ANTHROPIC_BASE_URL")
        .and_then(Value::as_str)
        .ok_or_else(|| "Claude settings do not contain ANTHROPIC_BASE_URL".to_owned())?;
    let api_key = environment
        .get("ANTHROPIC_API_KEY")
        .and_then(Value::as_str)
        .and_then(normalized_api_key)
        .ok_or_else(|| "Claude settings do not contain a valid ANTHROPIC_API_KEY".to_owned())?;
    let base_url = normalize_base_url(base_url)?;
    let (version, _) = discover_claude_version()?;
    let remote = fetch_settings(&base_url, api_key, &version).await?;
    let expected = render_project_settings(
        base_url.as_str().trim_end_matches('/'),
        api_key,
        &remote.model_mappings,
        &version,
    )?;
    if expected != bytes {
        return Err(
            "the saved settings differ from the current Claude version or platform model mapping; rerun setup claude"
                .to_owned(),
        );
    }
    println!("Authentication, connectivity, Claude version, and model mappings are healthy.");
    Ok(())
}

pub fn remove(project: &Path) -> Result<()> {
    let project = canonical_project(project)?;
    let config_path = project.join(SETTINGS_RELATIVE_PATH);
    let state_path = project.join(".claude").join(STATE_FILE_NAME);
    let state_bytes = read_optional(&state_path)?.ok_or_else(|| {
        format!(
            "asterctl has not configured Claude CLI for {}",
            project.display()
        )
    })?;
    let state = parse_state(&state_bytes)?;
    let current = fs::read(&config_path)
        .map_err(|error| format!("could not read {}: {error}", config_path.display()))?;
    if sha256_hex(&current) != state.installed_sha256 {
        return Err(format!(
            "{} changed after setup; refusing to overwrite user changes",
            config_path.display()
        ));
    }
    if state.created_config {
        fs::remove_file(&config_path)
            .map_err(|error| format!("could not remove {}: {error}", config_path.display()))?;
    } else {
        let backup_path = state.backup_path.as_deref().ok_or_else(|| {
            "the original Claude settings backup is missing from asterctl state".to_owned()
        })?;
        let original = fs::read(backup_path)
            .map_err(|error| format!("could not read backup {}: {error}", backup_path.display()))?;
        atomic_write(&config_path, &original)?;
        fs::remove_file(backup_path).map_err(|error| {
            format!("could not remove backup {}: {error}", backup_path.display())
        })?;
    }
    fs::remove_file(&state_path)
        .map_err(|error| format!("could not remove {}: {error}", state_path.display()))?;
    println!("Removed the asterctl-managed Claude CLI configuration.");
    if !state.created_config {
        println!("Restored the settings file that existed before setup.");
    }
    Ok(())
}

fn canonical_project(project: &Path) -> Result<PathBuf> {
    let path = fs::canonicalize(project).map_err(|error| {
        format!(
            "could not open project directory {}: {error}",
            project.display()
        )
    })?;
    if !path.is_dir() {
        return Err(format!(
            "project path is not a directory: {}",
            path.display()
        ));
    }
    Ok(path)
}

fn prepare_project(project: &Path) -> Result<PathBuf> {
    match fs::metadata(project) {
        Ok(metadata) if !metadata.is_dir() => {
            return Err(format!(
                "project path is not a directory: {}",
                project.display()
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(project).map_err(|create_error| {
                format!(
                    "could not create project directory {}: {create_error}",
                    project.display()
                )
            })?;
        }
        Err(error) => {
            return Err(format!(
                "could not inspect project directory {}: {error}",
                project.display()
            ));
        }
    }
    canonical_project(project)
}

fn user_facing_path(path: &Path) -> String {
    let value = path.display().to_string();
    if let Some(path) = value.strip_prefix("\\\\?\\UNC\\") {
        return format!("\\\\{path}");
    }
    value.strip_prefix("\\\\?\\").unwrap_or(&value).to_owned()
}

fn launch_claude(project: &Path) -> Result<()> {
    let status = Command::new("claude")
        .current_dir(project)
        .status()
        .map_err(|error| {
            format!(
                "configuration succeeded, but Claude could not be started in {}: {error}",
                user_facing_path(project)
            )
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "configuration succeeded, but Claude exited with status {status}"
        ))
    }
}

fn confirm_overwrite(path: &Path) -> Result<bool> {
    eprintln!("warning: {} already exists.", path.display());
    if !io::stdin().is_terminal() {
        return Err(
            "existing Claude settings require interactive overwrite confirmation; configure the file manually or rerun in a terminal"
                .to_owned(),
        );
    }
    eprint!("Overwrite it after creating a backup? [y/N] ");
    io::stderr()
        .flush()
        .map_err(|error| format!("could not display overwrite prompt: {error}"))?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|error| format!("could not read overwrite confirmation: {error}"))?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn discover_claude_version() -> Result<(Version, String)> {
    let output = Command::new("claude")
        .arg("--version")
        .output()
        .map_err(|error| format!("could not run `claude --version`: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "`claude --version` exited with status {}; install or repair Claude Code first",
            output.status
        ));
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|_| "`claude --version` returned non-UTF-8 output".to_owned())?;
    let text = text.trim().to_owned();
    let version = parse_claude_version(&text)
        .ok_or_else(|| format!("could not parse Claude Code version from `{text}`"))?;
    Ok((version, text))
}

async fn fetch_settings(
    base_url: &Url,
    api_key: &str,
    version: &Version,
) -> Result<ClaudeCliSettingsResponse> {
    let mut endpoint = base_url.clone();
    endpoint.set_path("/v1/claude-cli/settings");
    endpoint.set_query(None);
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .redirect(Policy::none())
        .build()
        .map_err(|error| format!("could not create HTTP client: {error}"))?;
    let response = client
        .get(endpoint)
        .header("x-api-key", api_key)
        .query(&[("version", version.to_string())])
        .send()
        .await
        .map_err(|error| format!("could not reach Aster: {error}"))?;
    let status = response.status();
    if status != StatusCode::OK {
        let body = response.text().await.unwrap_or_default();
        let message = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|value| {
                value
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| status.to_string());
        return Err(format!("Aster rejected Claude CLI setup: {message}"));
    }
    let settings = response
        .json::<ClaudeCliSettingsResponse>()
        .await
        .map_err(|error| format!("Aster returned invalid Claude CLI settings: {error}"))?;
    if settings.schema != SETTINGS_SCHEMA
        || settings.claude_version != version.to_string()
        || settings.minimum_version != MINIMUM_VERSION
        || !settings.model_mappings.is_complete()
    {
        return Err("Aster returned incompatible Claude CLI settings".to_owned());
    }
    Ok(settings)
}

fn normalize_base_url(value: &str) -> Result<Url> {
    let mut url =
        Url::parse(value.trim()).map_err(|error| format!("invalid Anthropic base URL: {error}"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Anthropic base URL must be an absolute HTTP(S) origin".to_owned());
    }
    let path = url.path().trim_end_matches('/');
    if !path.is_empty() {
        return Err("Anthropic base URL must not include /v1 or another path".to_owned());
    }
    url.set_path("");
    Ok(url)
}

fn normalized_api_key(value: &str) -> Option<&str> {
    let value = value.trim();
    (value.starts_with("ask_") && value.len() >= 16 && !value.chars().any(char::is_whitespace))
        .then_some(value)
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

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    let mut temporary = NamedTempFile::new_in(parent).map_err(|error| {
        format!(
            "could not create temporary file in {}: {error}",
            parent.display()
        )
    })?;
    temporary
        .write_all(bytes)
        .and_then(|()| temporary.flush())
        .map_err(|error| {
            format!(
                "could not write temporary file for {}: {error}",
                path.display()
            )
        })?;
    temporary
        .persist(path)
        .map_err(|error| format!("could not replace {}: {}", path.display(), error.error))?;
    Ok(())
}

fn atomic_write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("could not serialize {}: {error}", path.display()))?;
    bytes.push(b'\n');
    atomic_write(path, &bytes)
}

fn write_backup(config_path: &Path, bytes: &[u8]) -> Result<PathBuf> {
    let stamp = now_unix_ms()?;
    let file_name = format!("settings.local.json.asterctl-{stamp}.bak");
    let path = config_path
        .parent()
        .ok_or_else(|| "Claude settings path has no parent directory".to_owned())?
        .join(file_name);
    atomic_write(&path, bytes)?;
    Ok(path)
}

fn restore_after_failed_setup(path: &Path, original: Option<&[u8]>) {
    if let Some(bytes) = original {
        let _ = atomic_write(path, bytes);
    } else {
        let _ = fs::remove_file(path);
    }
}

fn parse_state(bytes: &[u8]) -> Result<ClaudeState> {
    let state: ClaudeState = serde_json::from_slice(bytes)
        .map_err(|error| format!("Claude asterctl state is invalid: {error}"))?;
    if state.schema != STATE_SCHEMA {
        return Err("Claude asterctl state uses an unsupported schema".to_owned());
    }
    Ok(state)
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn now_unix_ms() -> Result<u128> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(|error| format!("system clock is before Unix epoch: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_an_origin_for_the_anthropic_base_url() {
        assert!(normalize_base_url("https://api.example.com").is_ok());
        assert!(normalize_base_url("https://api.example.com/v1").is_err());
    }

    #[test]
    fn state_never_contains_the_api_key() {
        let state = ClaudeState {
            schema: STATE_SCHEMA.to_owned(),
            config_path: PathBuf::from("project/.claude/settings.local.json"),
            installed_sha256: "hash".to_owned(),
            backup_path: None,
            created_config: true,
            claude_version: "2.1.255".to_owned(),
            updated_at_unix_ms: 1,
        };
        let rendered = serde_json::to_string(&state).expect("state JSON");
        assert!(!rendered.contains("ask_"));
    }

    #[test]
    fn prepare_project_reuses_an_existing_directory() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let project = temporary.path().join("existing");
        fs::create_dir(&project).expect("create project");

        let resolved = prepare_project(&project).expect("resolve existing project");

        assert_eq!(
            resolved,
            fs::canonicalize(&project).expect("canonical project")
        );
    }

    #[test]
    fn prepare_project_creates_a_missing_directory() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let project = temporary.path().join("new").join("nested-project");

        let resolved = prepare_project(&project).expect("create project");

        assert!(project.is_dir());
        assert_eq!(
            resolved,
            fs::canonicalize(&project).expect("canonical project")
        );
    }

    #[test]
    fn prepare_project_rejects_an_existing_file() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let project = temporary.path().join("not-a-directory");
        fs::write(&project, b"file").expect("create file");

        let error = prepare_project(&project).expect_err("reject file");

        assert!(error.contains("not a directory"));
    }

    #[test]
    fn user_facing_windows_paths_hide_the_verbatim_prefix() {
        assert_eq!(
            user_facing_path(Path::new(r"\\?\D:\code\project-demo\.claude")),
            r"D:\code\project-demo\.claude"
        );
        assert_eq!(
            user_facing_path(Path::new(r"\\?\UNC\server\share\project-demo")),
            r"\\server\share\project-demo"
        );
    }
}
