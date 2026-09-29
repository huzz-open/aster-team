#![forbid(unsafe_code)]

use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const SETTINGS_SCHEMA: &str = "aster.claude-cli-settings/v1";
pub const COMPATIBILITY_SOURCE: &str = "https://code.claude.com/docs/en/model-config";
pub const COMPATIBILITY_CHECKED_AT: &str = "2026-09-03";
pub const MINIMUM_VERSION: &str = "2.1.255";

pub const FABLE_MODEL_ID: &str = "claude-fable-5-1";
pub const OPUS_MODEL_ID: &str = "claude-opus-5";
pub const SONNET_MODEL_ID: &str = "claude-sonnet-5";
pub const HAIKU_MODEL_ID: &str = "claude-haiku-4-5-20251001";

const FABLE_CANDIDATES: &[&str] = &[
    "gpt-5.6-sol",
    "gpt-5.5",
    "gpt-5.4",
    "gpt-5.6-terra",
    "gpt-5.4-mini",
    "gpt-5.3-codex",
    "gpt-5.6-luna",
];
const OPUS_CANDIDATES: &[&str] = &[
    "gpt-5.6-terra",
    "gpt-5.6-sol",
    "gpt-5.5",
    "gpt-5.4",
    "gpt-5.4-mini",
    "gpt-5.3-codex",
    "gpt-5.6-luna",
];
const SONNET_CANDIDATES: &[&str] = &[
    "gpt-5.4-mini",
    "gpt-5.3-codex",
    "gpt-5.6-terra",
    "gpt-5.6-sol",
    "gpt-5.5",
    "gpt-5.4",
    "gpt-5.6-luna",
];
const HAIKU_CANDIDATES: &[&str] = &[
    "gpt-5.6-luna",
    "gpt-5.4-mini",
    "gpt-5.3-codex",
    "gpt-5.6-terra",
    "gpt-5.6-sol",
    "gpt-5.5",
    "gpt-5.4",
];

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClaudeModelMappings {
    #[serde(default)]
    pub fable: String,
    #[serde(default)]
    pub opus: String,
    #[serde(default)]
    pub sonnet: String,
    #[serde(default)]
    pub haiku: String,
}

impl ClaudeModelMappings {
    pub fn is_complete(&self) -> bool {
        [&self.fable, &self.opus, &self.sonnet, &self.haiku]
            .into_iter()
            .all(|value| !value.trim().is_empty())
    }

    pub fn entries(&self) -> [(&'static str, &str); 4] {
        [
            ("fable", &self.fable),
            ("opus", &self.opus),
            ("sonnet", &self.sonnet),
            ("haiku", &self.haiku),
        ]
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClaudeCliSettingsResponse {
    pub schema: String,
    pub claude_version: String,
    pub minimum_version: String,
    pub model_mappings: ClaudeModelMappings,
}

pub fn parse_claude_version(output: &str) -> Option<Version> {
    output
        .split(|character: char| !(character.is_ascii_digit() || character == '.'))
        .filter(|candidate| candidate.matches('.').count() >= 2)
        .find_map(|candidate| Version::parse(candidate.trim_matches('.')).ok())
}

pub fn supports_version(version: &Version) -> bool {
    version >= &Version::new(2, 1, 255)
}

pub fn suggested_mappings(
    available_models: impl IntoIterator<Item = impl AsRef<str>>,
) -> ClaudeModelMappings {
    let mut models = available_models
        .into_iter()
        .map(|value| value.as_ref().trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    models.sort();
    models.dedup();
    let Some(fallback) = models.first().cloned() else {
        return ClaudeModelMappings::default();
    };

    ClaudeModelMappings {
        fable: select_model(&models, FABLE_CANDIDATES).unwrap_or_else(|| fallback.clone()),
        opus: select_model(&models, OPUS_CANDIDATES).unwrap_or_else(|| fallback.clone()),
        sonnet: select_model(&models, SONNET_CANDIDATES).unwrap_or_else(|| fallback.clone()),
        haiku: select_model(&models, HAIKU_CANDIDATES).unwrap_or(fallback),
    }
}

fn select_model(models: &[String], candidates: &[&str]) -> Option<String> {
    candidates
        .iter()
        .find_map(|candidate| models.iter().find(|model| model == candidate).cloned())
}

pub fn render_project_settings(
    base_url: &str,
    api_key: &str,
    mappings: &ClaudeModelMappings,
    version: &Version,
) -> Result<Vec<u8>, String> {
    if !supports_version(version) {
        return Err(format!(
            "Claude Code {version} is unsupported; version {MINIMUM_VERSION} or later is required"
        ));
    }
    if base_url.trim().is_empty() || api_key.trim().is_empty() || !mappings.is_complete() {
        return Err("Claude project settings are incomplete".to_owned());
    }

    serde_json::to_vec_pretty(&json!({
        "env": {
            "ANTHROPIC_API_KEY": api_key,
            "ANTHROPIC_BASE_URL": base_url,
            "ANTHROPIC_DEFAULT_FABLE_MODEL": FABLE_MODEL_ID,
            "ANTHROPIC_DEFAULT_HAIKU_MODEL": HAIKU_MODEL_ID,
            "ANTHROPIC_DEFAULT_OPUS_MODEL": OPUS_MODEL_ID,
            "ANTHROPIC_DEFAULT_SONNET_MODEL": SONNET_MODEL_ID,
        },
        "model": "opus",
        "modelOverrides": {
            FABLE_MODEL_ID: mappings.fable,
            HAIKU_MODEL_ID: mappings.haiku,
            OPUS_MODEL_ID: mappings.opus,
            SONNET_MODEL_ID: mappings.sonnet,
        },
    }))
    .map(|mut bytes| {
        bytes.push(b'\n');
        bytes
    })
    .map_err(|error| format!("could not render Claude project settings: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_standard_claude_version_output() {
        assert_eq!(
            parse_claude_version("2.1.255 (Claude Code)"),
            Some(Version::new(2, 1, 255))
        );
    }

    #[test]
    fn enforces_the_minimum_supported_version() {
        assert!(!supports_version(&Version::new(2, 1, 254)));
        assert!(supports_version(&Version::new(2, 1, 255)));
        assert!(supports_version(&Version::new(2, 2, 0)));
    }

    #[test]
    fn maps_the_default_aster_models_by_explicit_role() {
        let mappings = suggested_mappings([
            "gpt-5.6-luna",
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.4-mini",
        ]);
        assert_eq!(mappings.fable, "gpt-5.6-sol");
        assert_eq!(mappings.opus, "gpt-5.6-terra");
        assert_eq!(mappings.sonnet, "gpt-5.4-mini");
        assert_eq!(mappings.haiku, "gpt-5.6-luna");
    }

    #[test]
    fn falls_back_to_the_only_enabled_model() {
        let mappings = suggested_mappings(["private-model"]);
        assert_eq!(mappings.fable, "private-model");
        assert_eq!(mappings.opus, "private-model");
        assert_eq!(mappings.sonnet, "private-model");
        assert_eq!(mappings.haiku, "private-model");
    }

    #[test]
    fn renders_pinned_claude_ids_and_model_overrides() {
        let mappings = ClaudeModelMappings {
            fable: "gpt-5.6-sol".to_owned(),
            opus: "gpt-5.6-terra".to_owned(),
            sonnet: "gpt-5.4-mini".to_owned(),
            haiku: "gpt-5.6-luna".to_owned(),
        };
        let rendered = render_project_settings(
            "https://api.example.com",
            "ask_secret",
            &mappings,
            &Version::new(2, 1, 255),
        )
        .expect("settings");
        let document: serde_json::Value = serde_json::from_slice(&rendered).expect("settings JSON");

        assert_eq!(document["model"], "opus");
        assert_eq!(
            document["env"]["ANTHROPIC_DEFAULT_FABLE_MODEL"],
            FABLE_MODEL_ID
        );
        assert_eq!(document["modelOverrides"][FABLE_MODEL_ID], "gpt-5.6-sol");
        assert_eq!(document["modelOverrides"][OPUS_MODEL_ID], "gpt-5.6-terra");
        assert_eq!(document["modelOverrides"][SONNET_MODEL_ID], "gpt-5.4-mini");
        assert_eq!(document["modelOverrides"][HAIKU_MODEL_ID], "gpt-5.6-luna");
    }

    #[test]
    fn refuses_to_render_for_an_unsupported_version() {
        let mappings = suggested_mappings(["gpt-5.6-sol"]);
        let error = render_project_settings(
            "https://api.example.com",
            "ask_secret",
            &mappings,
            &Version::new(2, 1, 254),
        )
        .expect_err("unsupported version");
        assert!(error.contains(MINIMUM_VERSION));
    }
}
