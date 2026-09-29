use std::collections::BTreeSet;

use serde_json::Value;

use super::*;

pub(super) const CONFIG_FIELD: &str = "model_catalog_json";
const FILE_NAME: &str = "aster-models.json";
const FAST_MODELS: &[u8] = include_bytes!("../../assets/codex-models-aster-fast.json");

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ManagedCatalog {
    path: PathBuf,
    installed_sha256: String,
    // An interrupted update may still have the preceding managed file on disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_sha256: Option<String>,
}

pub(super) struct PreparedCatalog {
    pub state: ManagedCatalog,
    bytes: Vec<u8>,
    previous: Option<Vec<u8>>,
    pub model_count: usize,
}

impl PreparedCatalog {
    pub fn config_value(&self) -> Result<&str> {
        self.state
            .path
            .to_str()
            .ok_or_else(|| "Codex catalog path is not UTF-8".to_owned())
    }

    pub fn install(&self) -> Result<()> {
        atomic_write(&self.state.path, &self.bytes)
    }

    pub fn rollback(&self) -> Result<()> {
        restore_optional_file(&self.state.path, self.previous.as_deref())
    }
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn parse_models(bytes: &[u8]) -> Result<Vec<Value>> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid Codex model catalog JSON: {error}"))?;
    let models = value
        .get("models")
        .and_then(Value::as_array)
        .filter(|models| !models.is_empty())
        .ok_or_else(|| "Codex model catalog must contain a nonempty models array".to_owned())?;
    let mut ids = BTreeSet::new();
    for model in models {
        let id = model
            .get("slug")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| "Codex model catalog contains a model without a slug".to_owned())?;
        if !ids.insert(id) {
            return Err(format!("Codex model catalog contains duplicate model {id}"));
        }
    }
    Ok(models.clone())
}

fn merge_models(mut base: Vec<Value>, additions: Vec<Value>) -> Vec<Value> {
    for model in additions {
        if let Some(existing) = base.iter_mut().find(|item| item["slug"] == model["slug"]) {
            *existing = model;
        } else {
            base.push(model);
        }
    }
    base
}

fn export_models(cli: &Path, override_path: Option<&Path>) -> Result<Vec<Value>> {
    let mut command = Command::new(cli);
    command.args(["debug", "models"]);
    if let Some(path) = override_path {
        let path = path
            .to_str()
            .ok_or_else(|| "Codex catalog path is not UTF-8".to_owned())?;
        // TOML encoding handles Windows paths and quotes without shell interpolation.
        command
            .arg("-c")
            .arg(format!("{CONFIG_FIELD}={}", toml_edit::Value::from(path)));
    } else {
        command.arg("--bundled");
    }
    let output = command
        .output()
        .map_err(|error| format!("could not inspect Codex models: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Codex model catalog check failed; a compatible `codex debug models` is required: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    parse_models(&output.stdout)
}

fn cached_models(home: &Path) -> Option<Vec<Value>> {
    let bytes = read_optional(&home.join("models_cache.json")).ok()??;
    let models = parse_models(&bytes).ok()?;
    // A previous override must not become the only source of native models.
    if models.iter().any(|model| {
        model["slug"]
            .as_str()
            .is_some_and(|id| id.ends_with("-fast"))
    }) {
        return None;
    }
    Some(models)
}

fn original_catalog_path(
    document: &DocumentMut,
    config_path: &Path,
    previous: Option<&CodexState>,
) -> Result<Option<PathBuf>> {
    let configured = managed_string(document, CONFIG_FIELD)?;
    let original = if let Some(field) =
        previous.and_then(|state| state.fields.iter().find(|field| field.path == CONFIG_FIELD))
    {
        if configured.as_deref() != Some(field.installed.as_str())
            && !(previous.is_some_and(|state| state.status == TransactionStatus::Pending)
                && configured == field.before)
        {
            return Err("model_catalog_json changed after setup; remove the previous integration before setting it up again".to_owned());
        }
        field.before.clone()
    } else {
        configured
    };
    Ok(original.map(|path| {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            path
        } else {
            config_path.parent().expect("config parent").join(path)
        }
    }))
}

fn check_owned_file(
    path: &Path,
    bytes: Option<&[u8]>,
    previous: Option<&CodexState>,
) -> Result<()> {
    let Some(bytes) = bytes else {
        return Ok(());
    };
    let owned = previous
        .and_then(|state| state.catalog.as_ref())
        .filter(|catalog| catalog.path == path);
    let hash = digest(bytes);
    if owned.is_some_and(|catalog| {
        catalog.installed_sha256 == hash
            || (previous.is_some_and(|state| state.status == TransactionStatus::Pending)
                && catalog.previous_sha256.as_deref() == Some(hash.as_str()))
    }) {
        return Ok(());
    }
    Err(format!(
        "{} already exists or was changed outside asterctl; preserve or move it before running setup",
        path.display()
    ))
}

pub(super) fn prepare(
    cli: &Path,
    document: &DocumentMut,
    config_path: &Path,
    previous_state: Option<&CodexState>,
) -> Result<PreparedCatalog> {
    prepare_with_exporter(document, config_path, previous_state, |path| {
        export_models(cli, path)
    })
}

fn prepare_with_exporter(
    document: &DocumentMut,
    config_path: &Path,
    previous_state: Option<&CodexState>,
    mut export: impl FnMut(Option<&Path>) -> Result<Vec<Value>>,
) -> Result<PreparedCatalog> {
    let home = config_path
        .parent()
        .ok_or_else(|| "Codex config has no parent".to_owned())?;
    let path = home.join(FILE_NAME);
    let previous = read_optional(&path)?;
    let custom_path = original_catalog_path(document, config_path, previous_state)?;
    if custom_path.as_deref() == Some(path.as_path()) {
        return Err(
            "the existing model_catalog_json points at asterctl's reserved catalog path".to_owned(),
        );
    }
    let mut models = match cached_models(home) {
        Some(models) => models,
        None => export(None)?,
    };
    if let Some(custom_path) = custom_path {
        let bytes = read_optional(&custom_path)?.ok_or_else(|| {
            format!(
                "existing Codex catalog {} is missing",
                custom_path.display()
            )
        })?;
        models = merge_models(models, parse_models(&bytes)?);
    }
    models = merge_models(models, parse_models(FAST_MODELS)?);
    let model_count = models.len();
    let mut bytes = serde_json::to_vec_pretty(&serde_json::json!({"models": models}))
        .map_err(|error| format!("could not encode Codex model catalog: {error}"))?;
    bytes.push(b'\n');
    // Validate using the installed client before changing its config, catalog or key.
    let directory =
        tempfile::tempdir().map_err(|error| format!("could not stage Codex catalog: {error}"))?;
    let candidate = directory.path().join(FILE_NAME);
    atomic_write(&candidate, &bytes)?;
    let loaded = export(Some(&candidate))?;
    verify_model_ids(&models, &loaded)?;
    Ok(PreparedCatalog {
        state: ManagedCatalog {
            path,
            installed_sha256: digest(&bytes),
            previous_sha256: previous.as_deref().map(digest),
        },
        bytes,
        previous,
        model_count,
    })
}

fn verify_model_ids(expected: &[Value], loaded: &[Value]) -> Result<()> {
    let ids = |models: &[Value]| {
        models
            .iter()
            .filter_map(|model| model["slug"].as_str().map(str::to_owned))
            .collect::<BTreeSet<_>>()
    };
    if ids(expected) != ids(loaded) {
        return Err("Codex did not load the complete merged model catalog".to_owned());
    }
    Ok(())
}

pub(super) fn inspect(document: &DocumentMut, config_path: &Path) -> Result<(PathBuf, usize)> {
    let value = managed_string(document, CONFIG_FIELD)?
        .ok_or_else(|| "Codex model catalog is not configured; run setup codex again".to_owned())?;
    let path = PathBuf::from(value);
    let path = if path.is_absolute() {
        path
    } else {
        config_path.parent().expect("config parent").join(path)
    };
    let bytes = read_optional(&path)?
        .ok_or_else(|| format!("Codex model catalog {} is missing", path.display()))?;
    let models = parse_models(&bytes)?;
    for fast in parse_models(FAST_MODELS)? {
        if !models.iter().any(|model| model["slug"] == fast["slug"]) {
            return Err(format!(
                "Codex catalog is missing {}; run setup codex again",
                fast["slug"]
            ));
        }
    }
    Ok((path, models.len()))
}

pub(super) fn verify(cli: &Path, path: &Path) -> Result<()> {
    let bytes = read_optional(path)?.ok_or_else(|| "Codex model catalog is missing".to_owned())?;
    verify_model_ids(&parse_models(&bytes)?, &export_models(cli, Some(path))?)
}

pub(super) fn removal_conflict(state: &CodexState) -> Result<Option<String>> {
    let Some(catalog) = &state.catalog else {
        return Ok(None);
    };
    if state
        .config_path
        .parent()
        .map(|home| home.join(FILE_NAME))
        .as_ref()
        != Some(&catalog.path)
    {
        return Err("asterctl state has an unexpected managed catalog path".to_owned());
    }
    let bytes = read_optional(&catalog.path)?;
    Ok(check_owned_file(&catalog.path, bytes.as_deref(), Some(state)).err())
}

pub(super) fn remove_file(state: &mut CodexState) -> Result<()> {
    if let Some(catalog) = &state.catalog {
        restore_optional_file(&catalog.path, None)?;
    }
    state.catalog = None;
    Ok(())
}

pub(super) fn commit(state: &mut CodexState) {
    if let Some(catalog) = &mut state.catalog {
        catalog.previous_sha256 = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn native_models() -> Vec<Value> {
        vec![
            json!({"slug":"gpt-5.6-luna", "display_name":"Native Luna", "model_messages":{"instructions_template":"native instructions"}, "future_field":42}),
            json!({"slug":"native-hidden", "visibility":"hide", "supported_in_api":false}),
        ]
    }

    fn write_models(path: &Path, models: &[Value]) {
        atomic_write(
            path,
            &serde_json::to_vec(&json!({"models":models})).unwrap(),
        )
        .unwrap();
    }

    fn fake_export(path: Option<&Path>) -> Result<Vec<Value>> {
        match path {
            Some(path) => parse_models(&fs::read(path).unwrap()),
            None => Ok(native_models()),
        }
    }

    fn managed_state(
        config: &Path,
        document: &DocumentMut,
        prepared: &PreparedCatalog,
    ) -> CodexState {
        let mut desired = desired_values("https://example.test/v1");
        desired.insert(
            CONFIG_FIELD.to_owned(),
            prepared.config_value().unwrap().to_owned(),
        );
        let mut state = build_pending_state(document, config, true, None, &desired, None).unwrap();
        state.catalog = Some(prepared.state.clone());
        state.status = TransactionStatus::Committed;
        commit(&mut state);
        state
    }

    #[test]
    fn asset_contains_only_the_four_tested_fast_models_and_required_fields() {
        let models = parse_models(FAST_MODELS).unwrap();
        assert_eq!(
            models
                .iter()
                .map(|model| model["slug"].as_str().unwrap())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "gpt-6-astra-fast",
                "gpt-5.6-sol-fast",
                "gpt-5.6-terra-fast",
                "gpt-5.6-luna-fast"
            ])
        );
        for model in models {
            assert!(
                model["display_name"]
                    .as_str()
                    .unwrap()
                    .ends_with(" Fast Aster")
            );
            assert!(!model["base_instructions"].as_str().unwrap().is_empty());
            assert!(model.get("model_messages").is_none());
            assert_eq!(model["additional_speed_tiers"], json!(["fast"]));
            assert_eq!(model["service_tiers"][0]["id"], "priority");
            assert_eq!(model["experimental_supported_tools"], json!([]));
            let levels = model["supported_reasoning_levels"].as_array().unwrap();
            assert_eq!(
                levels
                    .iter()
                    .map(|level| level["effort"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                ["low", "medium", "high", "xhigh", "max"]
            );
            assert!(
                levels
                    .iter()
                    .any(|level| level["effort"] == model["default_reasoning_level"])
            );
        }
    }

    #[test]
    fn cached_native_and_existing_custom_metadata_survive_merging() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path();
        let mut natives = native_models();
        natives.push(json!({"slug":"new-native", "future_capability":true}));
        write_models(&home.join("models_cache.json"), &natives);
        let custom = vec![
            json!({"slug":"user-model", "user_option":[1,2]}),
            json!({"slug":"gpt-5.6-luna-fast", "obsolete":true}),
        ];
        write_models(&home.join("custom.json"), &custom);
        let mut document = DocumentMut::new();
        set_managed_string(&mut document, CONFIG_FIELD, "custom.json").unwrap();
        let prepared = prepare_with_exporter(&document, &home.join("config.toml"), None, |path| {
            assert!(path.is_some(), "valid cache should avoid bundled export");
            fake_export(path)
        })
        .unwrap();
        let merged = parse_models(&prepared.bytes).unwrap();
        assert_eq!(&merged[..3], natives.as_slice());
        assert_eq!(merged[3], custom[0]);
        assert_eq!(merged.len(), 8);
        assert!(!merged.iter().any(|model| model.get("obsolete").is_some()));
        assert!(
            !home.join(FILE_NAME).exists(),
            "preparation does not install"
        );
        assert_eq!(
            parse_models(&fs::read(home.join("custom.json")).unwrap()).unwrap(),
            custom
        );
    }

    #[test]
    fn missing_invalid_and_override_caches_fall_back_to_bundled_catalog() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("models_cache.json");
        for bytes in [None, Some(b"invalid".as_slice()), Some(FAST_MODELS)] {
            if let Some(bytes) = bytes {
                atomic_write(&path, bytes).unwrap();
            }
            let mut bundled_calls = 0;
            let prepared = prepare_with_exporter(
                &DocumentMut::new(),
                &directory.path().join("config.toml"),
                None,
                |path| {
                    if path.is_none() {
                        bundled_calls += 1;
                    }
                    fake_export(path)
                },
            )
            .unwrap();
            assert_eq!(bundled_calls, 1);
            assert_eq!(prepared.model_count, native_models().len() + 4);
        }
    }

    #[test]
    fn repeat_setup_refreshes_native_models_without_duplicates_and_keeps_original_pointer() {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("config.toml");
        write_models(
            &directory.path().join("custom.json"),
            &[json!({"slug":"user-model"})],
        );
        let mut document = DocumentMut::new();
        set_managed_string(&mut document, CONFIG_FIELD, "custom.json").unwrap();
        let first = prepare_with_exporter(&document, &config, None, fake_export).unwrap();
        first.install().unwrap();
        let state = managed_state(&config, &document, &first);
        set_managed_string(&mut document, CONFIG_FIELD, first.config_value().unwrap()).unwrap();
        let mut natives = native_models();
        natives.push(json!({"slug":"new-model"}));
        write_models(&directory.path().join("models_cache.json"), &natives);
        let update = prepare_with_exporter(&document, &config, Some(&state), fake_export).unwrap();
        assert_eq!(update.model_count, first.model_count + 1);
        let mut desired = desired_values("https://example.test/v1");
        desired.insert(
            CONFIG_FIELD.to_owned(),
            update.config_value().unwrap().to_owned(),
        );
        let next =
            build_pending_state(&document, &config, true, Some(&state), &desired, None).unwrap();
        assert_eq!(
            next.fields
                .iter()
                .find(|field| field.path == CONFIG_FIELD)
                .unwrap()
                .before
                .as_deref(),
            Some("custom.json")
        );
        update.install().unwrap();
        update.rollback().unwrap();
        assert_eq!(fs::read(&first.state.path).unwrap(), first.bytes);
    }

    #[test]
    fn client_rejection_does_not_install_a_catalog() {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("config.toml");
        let result = prepare_with_exporter(&DocumentMut::new(), &config, None, |path| {
            if path.is_some() {
                Err("missing required field".to_owned())
            } else {
                Ok(native_models())
            }
        });
        assert!(result.is_err());
        assert!(!directory.path().join(FILE_NAME).exists());
        assert!(!config.exists());
    }

    #[test]
    fn setup_overwrites_aster_catalog_but_remove_still_detects_external_edits() {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("config.toml");
        let mut document = DocumentMut::new();
        let prepared = prepare_with_exporter(&document, &config, None, fake_export).unwrap();
        prepared.install().unwrap();
        atomic_write(&prepared.state.path, b"old generated contents").unwrap();
        let replacement = prepare_with_exporter(&document, &config, None, fake_export).unwrap();
        replacement.install().unwrap();
        assert_eq!(fs::read(&prepared.state.path).unwrap(), prepared.bytes);
        let state = managed_state(&config, &document, &prepared);
        set_managed_string(
            &mut document,
            CONFIG_FIELD,
            prepared.config_value().unwrap(),
        )
        .unwrap();
        atomic_write(&prepared.state.path, b"user edit").unwrap();
        assert!(removal_conflict(&state).unwrap().is_some());
        let replacement =
            prepare_with_exporter(&document, &config, Some(&state), fake_export).unwrap();
        replacement.install().unwrap();
        assert_eq!(fs::read(&prepared.state.path).unwrap(), prepared.bytes);
    }

    #[test]
    fn setup_after_codex_directory_deletion_ignores_external_state() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join(".codex");
        let config = home.join("config.toml");
        let original = DocumentMut::new();
        let first = prepare_with_exporter(&original, &config, None, fake_export).unwrap();
        first.install().unwrap();
        let state = managed_state(&config, &original, &first);
        fs::write(
            &config,
            format!(
                "model_provider = \"aster\"\nmodel_catalog_json = \"{}\"\n",
                first.config_value().unwrap().replace('\\', "\\\\")
            ),
        )
        .unwrap();
        fs::remove_dir_all(&home).unwrap();

        let fresh = DocumentMut::new();
        let previous = reconcile_previous_state(&fresh, false, Some(state.clone())).unwrap();
        assert!(previous.is_none());
        let recreated = "model = \"gpt-5.6-luna\"\n".parse::<DocumentMut>().unwrap();
        assert!(
            reconcile_previous_state(&recreated, true, Some(state))
                .unwrap()
                .is_none()
        );
        let next = prepare_with_exporter(&fresh, &config, previous.as_ref(), fake_export).unwrap();
        let mut desired = desired_values("https://example.test/v1");
        desired.insert(
            CONFIG_FIELD.to_owned(),
            next.config_value().unwrap().to_owned(),
        );
        let pending =
            build_pending_state(&fresh, &config, false, previous.as_ref(), &desired, None).unwrap();
        assert!(!pending.config_existed_before_first_setup);
        assert!(pending.fields.iter().all(|field| field.before.is_none()));
        next.install().unwrap();
        assert!(config.parent().unwrap().join(FILE_NAME).exists());
    }

    #[test]
    fn install_rollback_and_remove_are_safe_to_retry() {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("config.toml");
        let document = DocumentMut::new();
        let prepared = prepare_with_exporter(&document, &config, None, fake_export).unwrap();
        prepared.install().unwrap();
        prepared.rollback().unwrap();
        prepared.rollback().unwrap();
        assert!(!prepared.state.path.exists());
        prepared.install().unwrap();
        let mut state = managed_state(&config, &document, &prepared);
        assert!(removal_conflict(&state).unwrap().is_none());
        remove_file(&mut state).unwrap();
        remove_file(&mut state).unwrap();
        assert!(!prepared.state.path.exists());
    }

    #[test]
    fn pending_setup_can_resume_before_config_or_catalog_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("config.toml");
        let document = DocumentMut::new();
        let prepared = prepare_with_exporter(&document, &config, None, fake_export).unwrap();
        let mut state = managed_state(&config, &document, &prepared);
        state.status = TransactionStatus::Pending;
        assert!(prepare_with_exporter(&document, &config, Some(&state), fake_export).is_ok());
        let old = b"previous managed contents";
        atomic_write(&prepared.state.path, old).unwrap();
        state.catalog.as_mut().unwrap().previous_sha256 = Some(digest(old));
        assert!(removal_conflict(&state).unwrap().is_none());
        state.status = TransactionStatus::Committed;
        assert!(removal_conflict(&state).unwrap().is_some());
    }

    #[test]
    fn invalid_catalogs_and_missing_models_are_rejected() {
        for bytes in [
            br#"{"models":[]}"#.as_slice(),
            br#"{"models":[{}]}"#,
            br#"{"models":[{"slug":"same"},{"slug":"same"}]}"#,
        ] {
            assert!(parse_models(bytes).is_err());
        }
        assert!(verify_model_ids(&native_models(), &native_models()[..1]).is_err());
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires an installed Codex CLI; run the documented compatibility check"]
    fn installed_codex_catalog_round_trip() {
        let cli = discover_codex_cli().expect("installed Codex CLI");
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("Codex catalog 空格");
        fs::create_dir(&home).unwrap();
        let config = home.join("config.toml");
        let mut document = DocumentMut::new();
        let native = export_models(&cli, None).unwrap();
        let prepared = prepare(&cli, &document, &config, None).unwrap();
        let merged = parse_models(&prepared.bytes).unwrap();
        for original in native {
            assert!(merged.contains(&original), "original metadata must survive");
        }
        prepared.install().unwrap();
        set_managed_string(
            &mut document,
            CONFIG_FIELD,
            prepared.config_value().unwrap(),
        )
        .unwrap();
        let (path, count) = inspect(&document, &config).unwrap();
        assert_eq!(count, merged.len());
        verify(&cli, &path).unwrap();
        prepared.rollback().unwrap();
        assert!(!path.exists());
        assert!(!config.exists(), "the check must not install any config");
    }
}
