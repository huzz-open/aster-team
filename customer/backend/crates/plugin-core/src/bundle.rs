use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::{Cursor, Read},
    sync::Arc,
};

use crate::sha256_hex;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::Value;
use thiserror::Error;
use zip::ZipArchive;

const DOMAIN: &[u8] = b"ASTER-LUA-BUNDLE-V1\0";
const MAX_ARCHIVE: usize = 16 * 1024 * 1024;
const MAX_UNPACKED: u64 = 64 * 1024 * 1024;
const MAX_FILE: u64 = 8 * 1024 * 1024;
const MAX_FILES: usize = 512;
const MAX_MANIFEST: u64 = 1024 * 1024;

#[derive(Debug, Error)]
pub enum BundleError {
    #[error("plugin archive exceeds the size limit")]
    ArchiveTooLarge,
    #[error("invalid plugin archive: {0}")]
    Archive(String),
    #[error("invalid plugin manifest: {0}")]
    Manifest(String),
    #[error("invalid plugin signature")]
    Signature,
    #[error("plugin publisher is not trusted for this bundle")]
    Publisher,
    #[error("plugin file mismatch: {0}")]
    File(String),
}

/// A publisher key can sign only its explicitly assigned bundle identity.
#[derive(Clone)]
pub struct TrustedPublisher {
    pub key_id: String,
    pub bundle_id: String,
    pub public_key: VerifyingKey,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BundleFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BundleDisplay {
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BundleManifest {
    pub format_version: u32,
    pub bundle_id: String,
    pub bundle_version: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub display: BTreeMap<String, BundleDisplay>,
    pub host_api: u32,
    pub canonical_schema: u32,
    pub min_host_version: String,
    pub entrypoints: BTreeMap<String, String>,
    pub files: Vec<BundleFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignatureDocument {
    algorithm: String,
    key_id: String,
    signature_base64: String,
}

#[derive(Clone)]
pub struct Bundle {
    pub digest: String,
    pub manifest: BundleManifest,
    pub files: Arc<HashMap<String, Vec<u8>>>,
}

#[cfg(test)]
pub(crate) fn signed_fixture(source: &[u8]) -> (Vec<u8>, TrustedPublisher) {
    signed_fixture_with_files(source, &[])
}

#[cfg(test)]
pub(crate) fn signed_fixture_with_files(
    source: &[u8],
    extras: &[(&str, &[u8])],
) -> (Vec<u8>, TrustedPublisher) {
    use std::io::Write;

    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    let signing_key = SigningKey::from_bytes(&[17; 32]);
    let manifest = json!({
        "format_version": 1,
        "bundle_id": "aster.test",
        "bundle_version": "1.0.0",
        "host_api": 1,
        "canonical_schema": 2,
        "min_host_version": "2.1.1",
        "entrypoints": {"public": "main.lua"},
        "files": std::iter::once(json!({
            "path": "main.lua",
            "size": source.len(),
            "sha256": sha256_hex(source),
        })).chain(extras.iter().map(|(path, data)| json!({
            "path": path,
            "size": data.len(),
            "sha256": sha256_hex(data),
        }))).collect::<Vec<_>>(),
    });
    let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    let mut signed = DOMAIN.to_vec();
    signed.extend_from_slice(&manifest_bytes);
    let signature = json!({
        "algorithm": "ed25519",
        "key_id": "fixture",
        "signature_base64": STANDARD.encode(signing_key.sign(&signed).to_bytes()),
    });
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    writer.start_file("manifest.json", options).unwrap();
    writer.write_all(&manifest_bytes).unwrap();
    writer.start_file("signature.json", options).unwrap();
    writer
        .write_all(&serde_json::to_vec(&signature).unwrap())
        .unwrap();
    writer.start_file("main.lua", options).unwrap();
    writer.write_all(source).unwrap();
    for (path, data) in extras {
        writer.start_file(*path, options).unwrap();
        writer.write_all(data).unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    (
        bytes,
        TrustedPublisher {
            key_id: "fixture".into(),
            bundle_id: "aster.test".into(),
            public_key: signing_key.verifying_key(),
        },
    )
}

/// Verifies a complete archive before exposing any Lua source to the runtime.
/// The returned file map owns the validated snapshot and never reads the
/// original submission path during execution.
pub fn verify_bundle(
    bytes: &[u8],
    publishers: &[TrustedPublisher],
    host_version: &str,
    canonical_schema: u32,
) -> Result<Bundle, BundleError> {
    if bytes.len() > MAX_ARCHIVE {
        return Err(BundleError::ArchiveTooLarge);
    }
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| BundleError::Archive(error.to_string()))?;
    if archive.len() > MAX_FILES + 2 {
        return Err(BundleError::Archive("too many entries".into()));
    }
    let mut contents = HashMap::new();
    let mut folded = HashSet::new();
    let mut total = 0_u64;
    let mut actual_total = 0_u64;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| BundleError::Archive(error.to_string()))?;
        let path = entry.name().to_owned();
        validate_path(&path)?;
        if !folded.insert(path.to_lowercase()) {
            return Err(BundleError::Archive(format!("duplicate path: {path}")));
        }
        if entry.is_dir() || !is_regular_file(entry.unix_mode()) {
            return Err(BundleError::Archive(format!("invalid entry type: {path}")));
        }
        let file_limit = if path == "manifest.json" || path == "signature.json" {
            MAX_MANIFEST
        } else {
            MAX_FILE
        };
        if entry.size() > file_limit {
            return Err(BundleError::Archive(format!("entry too large: {path}")));
        }
        total = total
            .checked_add(entry.size())
            .ok_or_else(|| BundleError::Archive("size overflow".into()))?;
        if total > MAX_UNPACKED {
            return Err(BundleError::Archive("unpacked size limit exceeded".into()));
        }
        let declared_size = entry.size();
        let mut content = Vec::with_capacity(declared_size as usize);
        entry
            .take(file_limit + 1)
            .read_to_end(&mut content)
            .map_err(|error| BundleError::Archive(error.to_string()))?;
        if content.len() as u64 > file_limit {
            return Err(BundleError::Archive(format!("entry too large: {path}")));
        }
        actual_total = actual_total
            .checked_add(content.len() as u64)
            .ok_or_else(|| BundleError::Archive("size overflow".into()))?;
        if actual_total > MAX_UNPACKED || content.len() as u64 != declared_size {
            return Err(BundleError::Archive("unpacked size mismatch".into()));
        }
        contents.insert(path, content);
    }

    let manifest_bytes = contents
        .remove("manifest.json")
        .ok_or_else(|| BundleError::Manifest("manifest.json is missing".into()))?;
    let signature_bytes = contents
        .remove("signature.json")
        .ok_or(BundleError::Signature)?;
    let signature: SignatureDocument = parse_strict(&signature_bytes)
        .and_then(serde_json::from_value)
        .map_err(|_| BundleError::Signature)?;
    if signature.algorithm != "ed25519" {
        return Err(BundleError::Signature);
    }
    let publisher = publishers
        .iter()
        .find(|key| key.key_id == signature.key_id)
        .ok_or(BundleError::Publisher)?;
    let raw_signature = STANDARD
        .decode(signature.signature_base64)
        .map_err(|_| BundleError::Signature)?;
    let signature = Signature::from_slice(&raw_signature).map_err(|_| BundleError::Signature)?;
    let mut signed = Vec::with_capacity(DOMAIN.len() + manifest_bytes.len());
    signed.extend_from_slice(DOMAIN);
    signed.extend_from_slice(&manifest_bytes);
    publisher
        .public_key
        .verify_strict(&signed, &signature)
        .map_err(|_| BundleError::Signature)?;
    let manifest: BundleManifest = parse_strict(&manifest_bytes)
        .and_then(serde_json::from_value)
        .map_err(|error| BundleError::Manifest(error.to_string()))?;
    if manifest.bundle_id != publisher.bundle_id {
        return Err(BundleError::Publisher);
    }
    if manifest.format_version != 1 || manifest.host_api != 1 {
        return Err(BundleError::Manifest(
            "unsupported bundle or host API".into(),
        ));
    }
    if manifest.canonical_schema != canonical_schema {
        return Err(BundleError::Manifest("canonical schema mismatch".into()));
    }
    let minimum = semver::Version::parse(&manifest.min_host_version)
        .map_err(|error| BundleError::Manifest(error.to_string()))?;
    let current = semver::Version::parse(host_version)
        .map_err(|error| BundleError::Manifest(error.to_string()))?;
    if current < minimum {
        return Err(BundleError::Manifest("host version is too old".into()));
    }
    semver::Version::parse(&manifest.bundle_version)
        .map_err(|error| BundleError::Manifest(error.to_string()))?;
    if manifest.bundle_id.is_empty() || manifest.entrypoints.is_empty() {
        return Err(BundleError::Manifest(
            "bundle identity or entrypoints missing".into(),
        ));
    }
    if manifest.display.len() > 2
        || manifest.display.iter().any(|(locale, display)| {
            !matches!(locale.as_str(), "zh-CN" | "en-US")
                || display.name.trim().is_empty()
                || display.name.trim() != display.name
                || display.name.chars().count() > 80
                || display.name.chars().any(char::is_control)
                || display.description.trim().is_empty()
                || display.description.trim() != display.description
                || display.description.chars().count() > 200
                || display.description.chars().any(char::is_control)
        })
    {
        return Err(BundleError::Manifest(
            "invalid plugin display metadata".into(),
        ));
    }

    if contents.len() != manifest.files.len() || manifest.files.len() > MAX_FILES {
        return Err(BundleError::Manifest(
            "file list does not match archive".into(),
        ));
    }
    let mut declared = HashSet::new();
    for file in &manifest.files {
        validate_path(&file.path)?;
        if file.path == "manifest.json" || file.path == "signature.json" {
            return Err(BundleError::Manifest("reserved path in file list".into()));
        }
        if !declared.insert(file.path.as_str()) {
            return Err(BundleError::Manifest("duplicate file declaration".into()));
        }
        let content = contents
            .get(&file.path)
            .ok_or_else(|| BundleError::File(file.path.clone()))?;
        if content.len() as u64 != file.size || sha256_hex(content) != file.sha256 {
            return Err(BundleError::File(file.path.clone()));
        }
        if !file.path.ends_with(".lua") && !file.path.ends_with(".json") {
            return Err(BundleError::Manifest("unsupported file type".into()));
        }
        if std::str::from_utf8(content).is_err() || content.contains(&0) {
            return Err(BundleError::File(file.path.clone()));
        }
    }
    for path in manifest.entrypoints.values() {
        if !path.ends_with(".lua") || !contents.contains_key(path) {
            return Err(BundleError::Manifest(format!(
                "missing Lua entrypoint: {path}"
            )));
        }
    }
    Ok(Bundle {
        digest: sha256_hex(bytes),
        manifest,
        files: Arc::new(contents),
    })
}

fn validate_path(path: &str) -> Result<(), BundleError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(BundleError::Archive(format!("invalid path: {path}")));
    }
    Ok(())
}

fn is_regular_file(mode: Option<u32>) -> bool {
    mode.is_none_or(|value| {
        let file_type = value & 0o170000;
        file_type == 0 || file_type == 0o100000
    })
}

// serde_json::Value normally accepts duplicate object keys. A signed manifest
// must have only one interpretation across release and runtime tools.
fn parse_strict(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = StrictValue::deserialize(&mut deserializer)?.0;
    deserializer.end()?;
    Ok(value)
}

struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StrictVisitor;
        impl<'de> Visitor<'de> for StrictVisitor {
            type Value = StrictValue;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("JSON value without duplicate keys")
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Bool(value)))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::from(value)))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::from(value)))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(value)
                    .map(Value::Number)
                    .map(StrictValue)
                    .ok_or_else(|| E::custom("non-finite number"))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::String(value.to_owned())))
            }
            fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::String(value)))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element::<StrictValue>()? {
                    values.push(value.0);
                }
                Ok(StrictValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some((key, value)) = map.next_entry::<String, StrictValue>()? {
                    if values.insert(key.clone(), value.0).is_some() {
                        return Err(de::Error::custom(format!("duplicate key: {key}")));
                    }
                }
                Ok(StrictValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(StrictVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_snapshot_rejects_mutated_source_and_untrusted_key() {
        let (archive, publisher) =
            signed_fixture(b"return { self_test = function() return true end }");
        let verified =
            verify_bundle(&archive, std::slice::from_ref(&publisher), "2.1.1", 2).unwrap();
        assert_eq!(verified.files.len(), 1);
        assert!(matches!(
            verify_bundle(&archive, &[], "2.1.1", 2),
            Err(BundleError::Publisher)
        ));
        let mut mutated = archive;
        let marker = b"self_test";
        let offset = mutated
            .windows(marker.len())
            .position(|bytes| bytes == marker)
            .unwrap();
        mutated[offset] = b'X';
        assert!(verify_bundle(&mutated, &[publisher], "2.1.1", 2).is_err());
    }

    #[test]
    fn strict_json_rejects_duplicate_keys_at_any_depth() {
        assert!(parse_strict(br#"{"files":[{"path":"a","path":"b"}]}"#).is_err());
        assert!(parse_strict(br#"{"a":1,"b":2}"#).is_ok());
    }
}
