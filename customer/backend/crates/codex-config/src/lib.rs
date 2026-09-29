#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use thiserror::Error;
use toml_edit::{DocumentMut, Item, Table, value};
use url::Url;

pub const PROVIDER: &str = "aster";
pub const API_KEY_ENVIRONMENT_VARIABLE: &str = "ASTER_API_KEY";
pub const ACTOR_HEADER_VALUE: &str = "aster-proxy";

pub const MODEL_PROVIDER: &str = "model_provider";
pub const PROVIDER_NAME: &str = "model_providers.aster.name";
pub const PROVIDER_BASE_URL: &str = "model_providers.aster.base_url";
pub const PROVIDER_WIRE_API: &str = "model_providers.aster.wire_api";
pub const PROVIDER_ENV_KEY: &str = "model_providers.aster.env_key";
pub const PROVIDER_ACTOR_HEADER: &str =
    "model_providers.aster.http_headers.x-openai-actor-authorization";

pub const MANAGED_PATHS: [&str; 6] = [
    MODEL_PROVIDER,
    PROVIDER_NAME,
    PROVIDER_BASE_URL,
    PROVIDER_WIRE_API,
    PROVIDER_ENV_KEY,
    PROVIDER_ACTOR_HEADER,
];

#[derive(Debug, Error)]
pub enum CodexConfigError {
    #[error(
        "Codex provider base URL must be an absolute HTTP(S) URL ending in /v1 without credentials, query, or fragment"
    )]
    InvalidBaseUrl,
    #[error("managed Codex parent {0} is not a table")]
    ParentIsNotTable(String),
    #[error("managed Codex field {0} exists but is not a string")]
    FieldIsNotString(String),
    #[error("generated Codex configuration is invalid: {0}")]
    InvalidGeneratedConfig(String),
}

pub fn normalize_base_url(value: &str) -> Result<String, CodexConfigError> {
    let value = value.trim().trim_end_matches('/');
    let parsed = Url::parse(value).map_err(|_| CodexConfigError::InvalidBaseUrl)?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !parsed.path().trim_end_matches('/').ends_with("/v1")
    {
        return Err(CodexConfigError::InvalidBaseUrl);
    }
    Ok(value.to_owned())
}

#[must_use]
pub fn desired_values(base_url: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        (MODEL_PROVIDER.to_owned(), PROVIDER.to_owned()),
        (PROVIDER_NAME.to_owned(), "Aster Team".to_owned()),
        (PROVIDER_BASE_URL.to_owned(), base_url.to_owned()),
        (PROVIDER_WIRE_API.to_owned(), "responses".to_owned()),
        (
            PROVIDER_ENV_KEY.to_owned(),
            API_KEY_ENVIRONMENT_VARIABLE.to_owned(),
        ),
        (
            PROVIDER_ACTOR_HEADER.to_owned(),
            ACTOR_HEADER_VALUE.to_owned(),
        ),
    ])
}

pub fn apply_aster_provider(
    document: &mut DocumentMut,
    base_url: &str,
) -> Result<(), CodexConfigError> {
    let base_url = normalize_base_url(base_url)?;
    for (path, installed) in desired_values(&base_url) {
        set_managed_string(document, &path, &installed)?;
    }
    Ok(())
}

pub fn render_aster_provider_snippet(base_url: &str) -> Result<String, CodexConfigError> {
    let mut document = DocumentMut::new();
    apply_aster_provider(&mut document, base_url)?;
    render_document(document).and_then(|bytes| {
        String::from_utf8(bytes)
            .map_err(|error| CodexConfigError::InvalidGeneratedConfig(error.to_string()))
    })
}

pub fn render_document(mut document: DocumentMut) -> Result<Vec<u8>, CodexConfigError> {
    let rendered = document.to_string();
    document = rendered
        .parse::<DocumentMut>()
        .map_err(|error| CodexConfigError::InvalidGeneratedConfig(error.to_string()))?;
    Ok(document.to_string().into_bytes())
}

pub fn managed_string(
    document: &DocumentMut,
    path: &str,
) -> Result<Option<String>, CodexConfigError> {
    let segments = path.split('.').collect::<Vec<_>>();
    let Some(mut item) = document.as_table().get(segments[0]) else {
        return Ok(None);
    };
    for segment in &segments[1..] {
        let Some(next) = item.get(*segment) else {
            return Ok(None);
        };
        item = next;
    }
    item.as_str()
        .map(|value| Some(value.to_owned()))
        .ok_or_else(|| CodexConfigError::FieldIsNotString(path.to_owned()))
}

pub fn set_managed_string(
    document: &mut DocumentMut,
    path: &str,
    installed: &str,
) -> Result<(), CodexConfigError> {
    let segments = path.split('.').collect::<Vec<_>>();
    let Some((last, parents)) = segments.split_last() else {
        return Err(CodexConfigError::ParentIsNotTable(String::new()));
    };
    let mut table: &mut dyn toml_edit::TableLike = document.as_table_mut();
    for segment in parents {
        let item = table
            .entry(segment)
            .or_insert_with(|| Item::Table(Table::new()));
        table = item
            .as_table_like_mut()
            .ok_or_else(|| CodexConfigError::ParentIsNotTable(parents.join(".")))?;
    }
    table.insert(last, value(installed));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_the_canonical_aster_provider_configuration() {
        let rendered = render_aster_provider_snippet("http://10.13.74.140:11080/v1")
            .expect("canonical configuration");
        assert_eq!(
            rendered,
            concat!(
                "model_provider = \"aster\"\n",
                "\n",
                "[model_providers]\n",
                "\n",
                "[model_providers.aster]\n",
                "base_url = \"http://10.13.74.140:11080/v1\"\n",
                "env_key = \"ASTER_API_KEY\"\n",
                "name = \"Aster Team\"\n",
                "wire_api = \"responses\"\n",
                "\n",
                "[model_providers.aster.http_headers]\n",
                "x-openai-actor-authorization = \"aster-proxy\"\n",
            )
        );
    }

    #[test]
    fn rejects_a_provider_url_without_the_v1_path() {
        assert!(matches!(
            render_aster_provider_snippet("https://api.example.com"),
            Err(CodexConfigError::InvalidBaseUrl)
        ));
    }

    #[test]
    fn preserves_unmanaged_configuration_when_applying_provider() {
        let mut document = "notify = [\"turn-ended\"]\n"
            .parse::<DocumentMut>()
            .expect("existing config");
        apply_aster_provider(&mut document, "https://api.example.com/v1").expect("apply provider");
        let rendered =
            String::from_utf8(render_document(document).expect("render document")).expect("UTF-8");
        assert!(rendered.contains("notify = [\"turn-ended\"]"));
        assert!(rendered.contains("base_url = \"https://api.example.com/v1\""));
    }
}
