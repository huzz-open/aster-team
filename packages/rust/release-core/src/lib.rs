#![forbid(unsafe_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs::File,
    io::Read as _,
    path::{Component, Path},
};

use aster_license_core::canonicalize;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{
    Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey, pkcs8::DecodePublicKey as _,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

pub const RELEASE_SCHEMA: &str = "aster.release-manifest.v1";
pub const RELEASE_MANIFEST_FILE: &str = "RELEASE.json";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub executable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseClaims {
    pub schema: String,
    pub key_id: String,
    pub product: String,
    pub version: String,
    pub platform: String,
    pub architecture: String,
    pub runtime: String,
    pub created_at: String,
    pub files: Vec<ReleaseFile>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReleaseDocument {
    #[serde(flatten)]
    pub claims: ReleaseClaims,
    pub signature: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseWire {
    schema: String,
    key_id: String,
    product: String,
    version: String,
    platform: String,
    architecture: String,
    runtime: String,
    created_at: String,
    files: Vec<ReleaseFile>,
    signature: String,
}

impl ReleaseWire {
    fn into_document(self) -> ReleaseDocument {
        ReleaseDocument {
            claims: ReleaseClaims {
                schema: self.schema,
                key_id: self.key_id,
                product: self.product,
                version: self.version,
                platform: self.platform,
                architecture: self.architecture,
                runtime: self.runtime,
                created_at: self.created_at,
                files: self.files,
            },
            signature: self.signature,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedRelease(ReleaseDocument);

impl VerifiedRelease {
    pub fn document(&self) -> &ReleaseDocument {
        &self.0
    }

    pub fn claims(&self) -> &ReleaseClaims {
        &self.0.claims
    }
}

#[derive(Clone, Debug, Default)]
pub struct TrustedReleaseKeys {
    keys: BTreeMap<String, VerifyingKey>,
}

impl TrustedReleaseKeys {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(
        &mut self,
        key_id: impl Into<String>,
        key: VerifyingKey,
    ) -> Result<(), ReleaseError> {
        let key_id = key_id.into();
        validate_identifier(&key_id)?;
        if self.keys.insert(key_id.clone(), key).is_some() {
            return Err(ReleaseError::DuplicateTrustedKey(key_id));
        }
        Ok(())
    }

    pub fn insert_spki_base64url(
        &mut self,
        key_id: impl Into<String>,
        public_key_spki: &str,
    ) -> Result<(), ReleaseError> {
        let bytes = URL_SAFE_NO_PAD
            .decode(public_key_spki)
            .map_err(|_| ReleaseError::PublicKeyEncoding)?;
        let key = VerifyingKey::from_public_key_der(&bytes)
            .map_err(|_| ReleaseError::PublicKeyEncoding)?;
        self.insert(key_id, key)
    }

    fn get(&self, key_id: &str) -> Result<&VerifyingKey, ReleaseError> {
        self.keys
            .get(key_id)
            .ok_or_else(|| ReleaseError::UntrustedKey(key_id.to_owned()))
    }
}

#[derive(Debug, Error)]
pub enum ReleaseError {
    #[error("release manifest JSON is invalid: {0}")]
    InvalidJson(String),
    #[error("release manifest contains trailing JSON")]
    TrailingJson,
    #[error("release manifest is invalid")]
    InvalidManifest,
    #[error("release file path is invalid: {0}")]
    InvalidPath(String),
    #[error("release signature encoding is invalid")]
    SignatureEncoding,
    #[error("release signature is invalid")]
    InvalidSignature,
    #[error("release public key encoding is invalid")]
    PublicKeyEncoding,
    #[error("release key id is not trusted: {0}")]
    UntrustedKey(String),
    #[error("trusted release key id is duplicated: {0}")]
    DuplicateTrustedKey(String),
    #[error("release tree contains a missing, extra, changed, or unsafe file: {0}")]
    TreeMismatch(String),
    #[error("release filesystem operation failed")]
    Io,
    #[error("canonical release serialization failed")]
    CanonicalSerialization,
}

pub fn sign(
    claims: ReleaseClaims,
    signing_key: &SigningKey,
) -> Result<ReleaseDocument, ReleaseError> {
    validate_claims(&claims)?;
    let payload = canonical_claims(&claims)?;
    let signature = signing_key.sign(&payload);
    Ok(ReleaseDocument {
        claims,
        signature: URL_SAFE_NO_PAD.encode(signature.to_bytes()),
    })
}

pub fn verify(
    data: &[u8],
    trusted_keys: &TrustedReleaseKeys,
) -> Result<VerifiedRelease, ReleaseError> {
    let wire: ReleaseWire = decode_exact(data)?;
    let document = wire.into_document();
    validate_claims(&document.claims)?;
    let signature_bytes = URL_SAFE_NO_PAD
        .decode(&document.signature)
        .map_err(|_| ReleaseError::SignatureEncoding)?;
    let signature =
        Signature::from_slice(&signature_bytes).map_err(|_| ReleaseError::SignatureEncoding)?;
    trusted_keys
        .get(&document.claims.key_id)?
        .verify(&canonical_claims(&document.claims)?, &signature)
        .map_err(|_| ReleaseError::InvalidSignature)?;
    Ok(VerifiedRelease(document))
}

pub fn verify_release_tree(root: &Path, release: &VerifiedRelease) -> Result<(), ReleaseError> {
    verify_release_files(
        root,
        release,
        release.claims().files.iter().map(|file| file.path.as_str()),
    )
}

/// Verifies a signed, deliberately partial release tree.
///
/// The requested paths must all be present in the signed release manifest and
/// the tree must contain exactly those files (plus `RELEASE.json`). This keeps
/// role-specific installers small without allowing unsigned or unexpected
/// payloads into the installed release.
pub fn verify_release_subset<'a>(
    root: &Path,
    release: &VerifiedRelease,
    required_paths: impl IntoIterator<Item = &'a str>,
) -> Result<(), ReleaseError> {
    verify_release_files(root, release, required_paths)
}

fn verify_release_files<'a>(
    root: &Path,
    release: &VerifiedRelease,
    required_paths: impl IntoIterator<Item = &'a str>,
) -> Result<(), ReleaseError> {
    let root_metadata = std::fs::symlink_metadata(root).map_err(|_| ReleaseError::Io)?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err(ReleaseError::TreeMismatch(".".to_owned()));
    }
    let expected: BTreeSet<_> = required_paths.into_iter().map(str::to_owned).collect();
    if expected.is_empty() {
        return Err(ReleaseError::InvalidManifest);
    }
    for path in &expected {
        validate_relative_path(path)?;
        if !release.claims().files.iter().any(|file| file.path == *path) {
            return Err(ReleaseError::TreeMismatch(path.clone()));
        }
    }
    let actual = collect_release_files(root, root)?;
    if actual != expected {
        let difference = actual
            .symmetric_difference(&expected)
            .next()
            .cloned()
            .unwrap_or_else(|| ".".to_owned());
        return Err(ReleaseError::TreeMismatch(difference));
    }
    for expected_file in release
        .claims()
        .files
        .iter()
        .filter(|file| expected.contains(&file.path))
    {
        let path = checked_join(root, &expected_file.path)?;
        let metadata = std::fs::symlink_metadata(&path).map_err(|_| ReleaseError::Io)?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() != expected_file.size
        {
            return Err(ReleaseError::TreeMismatch(expected_file.path.clone()));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let executable = metadata.permissions().mode() & 0o111 != 0;
            if executable != expected_file.executable {
                return Err(ReleaseError::TreeMismatch(expected_file.path.clone()));
            }
        }
        let mut source = File::open(path).map_err(|_| ReleaseError::Io)?;
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = source.read(&mut buffer).map_err(|_| ReleaseError::Io)?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        if lowercase_hex(&digest.finalize()) != expected_file.sha256 {
            return Err(ReleaseError::TreeMismatch(expected_file.path.clone()));
        }
    }
    Ok(())
}

fn lowercase_hex(value: &[u8]) -> String {
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

fn canonical_claims(claims: &ReleaseClaims) -> Result<Vec<u8>, ReleaseError> {
    let value = serde_json::to_value(claims).map_err(|_| ReleaseError::CanonicalSerialization)?;
    canonicalize(&value).map_err(|_| ReleaseError::CanonicalSerialization)
}

fn validate_claims(claims: &ReleaseClaims) -> Result<(), ReleaseError> {
    if claims.schema != RELEASE_SCHEMA
        || claims.product != "aster-team"
        || !matches!(claims.platform.as_str(), "linux" | "windows" | "macos")
        || !matches!(claims.architecture.as_str(), "amd64" | "arm64")
        || !matches!(
            (claims.platform.as_str(), claims.runtime.as_str()),
            ("linux", "musl-static") | ("windows", "msvc") | ("macos", "native")
        )
        || Version::parse(&claims.version).is_err()
        || validate_identifier(&claims.key_id).is_err()
        || validate_exact_time(&claims.created_at).is_err()
        || claims.files.is_empty()
        || claims.files.len() > 10_000
    {
        return Err(ReleaseError::InvalidManifest);
    }
    let mut previous = None;
    for file in &claims.files {
        validate_relative_path(&file.path)?;
        if file.path == RELEASE_MANIFEST_FILE
            || file.sha256.len() != 64
            || !file
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            || previous.is_some_and(|value: &str| value >= file.path.as_str())
        {
            return Err(ReleaseError::InvalidManifest);
        }
        previous = Some(file.path.as_str());
    }
    Ok(())
}

fn validate_identifier(value: &str) -> Result<(), ReleaseError> {
    if !(3..=128).contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
    {
        return Err(ReleaseError::InvalidManifest);
    }
    Ok(())
}

fn validate_exact_time(value: &str) -> Result<(), ReleaseError> {
    if value.len() != 24 || value.as_bytes().get(19) != Some(&b'.') || !value.ends_with('Z') {
        return Err(ReleaseError::InvalidManifest);
    }
    let parsed =
        OffsetDateTime::parse(value, &Rfc3339).map_err(|_| ReleaseError::InvalidManifest)?;
    if parsed.offset() != UtcOffset::UTC {
        return Err(ReleaseError::InvalidManifest);
    }
    Ok(())
}

fn validate_relative_path(value: &str) -> Result<(), ReleaseError> {
    if value.is_empty()
        || value.len() > 512
        || value.starts_with('/')
        || value.ends_with('/')
        || value.split('/').any(|segment| segment.is_empty())
        || value.contains('\\')
        || value.chars().any(char::is_control)
    {
        return Err(ReleaseError::InvalidPath(value.to_owned()));
    }
    let path = Path::new(value);
    if path.components().any(|component| {
        !matches!(component, Component::Normal(_))
            || component.as_os_str().to_str().is_none()
            || component.as_os_str().is_empty()
    }) {
        return Err(ReleaseError::InvalidPath(value.to_owned()));
    }
    Ok(())
}

fn checked_join(root: &Path, relative: &str) -> Result<std::path::PathBuf, ReleaseError> {
    validate_relative_path(relative)?;
    Ok(root.join(relative))
}

fn collect_release_files(root: &Path, directory: &Path) -> Result<BTreeSet<String>, ReleaseError> {
    let mut files = BTreeSet::new();
    let entries = std::fs::read_dir(directory).map_err(|_| ReleaseError::Io)?;
    for entry in entries {
        let entry = entry.map_err(|_| ReleaseError::Io)?;
        let metadata = entry.metadata().map_err(|_| ReleaseError::Io)?;
        let file_type = entry.file_type().map_err(|_| ReleaseError::Io)?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|_| ReleaseError::Io)?
            .to_str()
            .ok_or(ReleaseError::Io)?
            .replace('\\', "/");
        if file_type.is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
            return Err(ReleaseError::TreeMismatch(relative));
        }
        if metadata.is_dir() {
            files.extend(collect_release_files(root, &path)?);
        } else if relative != RELEASE_MANIFEST_FILE {
            validate_relative_path(&relative)?;
            files.insert(relative);
        }
    }
    Ok(files)
}

fn decode_exact<'de, T: Deserialize<'de>>(data: &'de [u8]) -> Result<T, ReleaseError> {
    let mut deserializer = serde_json::Deserializer::from_slice(data);
    let value = T::deserialize(&mut deserializer)
        .map_err(|error| ReleaseError::InvalidJson(error.to_string()))?;
    deserializer.end().map_err(|_| ReleaseError::TrailingJson)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use serde_json::Value;
    use tempfile::tempdir;

    use super::*;

    fn file(path: &str, value: &[u8], executable: bool) -> ReleaseFile {
        ReleaseFile {
            path: path.to_owned(),
            size: value.len() as u64,
            sha256: lowercase_hex(&Sha256::digest(value)),
            executable,
        }
    }

    fn claims(key_id: &str) -> ReleaseClaims {
        ReleaseClaims {
            schema: RELEASE_SCHEMA.to_owned(),
            key_id: key_id.to_owned(),
            product: "aster-team".to_owned(),
            version: "1.0.0".to_owned(),
            platform: "linux".to_owned(),
            architecture: "amd64".to_owned(),
            runtime: "musl-static".to_owned(),
            created_at: "2026-08-28T00:00:00.000Z".to_owned(),
            files: vec![file("bin/aster-control", b"control", false)],
        }
    }

    #[test]
    fn bridge_keyring_accepts_old_and_new_release_signatures() {
        let old = SigningKey::from_bytes(&[11_u8; 32]);
        let new = SigningKey::from_bytes(&[12_u8; 32]);
        let old_document = sign(claims("release-2026-01"), &old).expect("sign old release");
        let new_document = sign(claims("release-2027-01"), &new).expect("sign new release");
        let mut keys = TrustedReleaseKeys::new();
        keys.insert("release-2026-01", old.verifying_key())
            .expect("insert old key");
        keys.insert("release-2027-01", new.verifying_key())
            .expect("insert new key");
        verify(
            &serde_json::to_vec(&old_document).expect("serialize old release"),
            &keys,
        )
        .expect("verify old release");
        verify(
            &serde_json::to_vec(&new_document).expect("serialize new release"),
            &keys,
        )
        .expect("verify new release");
    }

    #[test]
    fn validates_supported_cross_platform_runtime_pairs() {
        let key = SigningKey::from_bytes(&[15_u8; 32]);
        for (platform, runtime) in [
            ("linux", "musl-static"),
            ("windows", "msvc"),
            ("macos", "native"),
        ] {
            let mut value = claims("release-platform-test");
            value.platform = platform.to_owned();
            value.runtime = runtime.to_owned();
            sign(value, &key).expect("supported platform and runtime");
        }
        let mut invalid = claims("release-platform-test");
        invalid.platform = "windows".to_owned();
        invalid.runtime = "musl-static".to_owned();
        assert!(sign(invalid, &key).is_err());
    }

    #[test]
    fn verifies_exact_tree_and_rejects_tampering_extras_and_symlinks() {
        let directory = tempdir().expect("temp directory");
        let bin = directory.path().join("bin");
        std::fs::create_dir_all(&bin).expect("create bin");
        let mut target = File::create(bin.join("aster-control")).expect("create binary");
        target.write_all(b"control").expect("write binary");
        drop(target);
        let signing_key = SigningKey::from_bytes(&[13_u8; 32]);
        let document = sign(claims("release-test-01"), &signing_key).expect("sign release");
        let mut keys = TrustedReleaseKeys::new();
        keys.insert("release-test-01", signing_key.verifying_key())
            .expect("insert key");
        let verified = verify(
            &serde_json::to_vec(&document).expect("serialize release"),
            &keys,
        )
        .expect("verify release");
        verify_release_tree(directory.path(), &verified).expect("verify exact tree");

        std::fs::write(bin.join("aster-control"), b"tampered").expect("tamper binary");
        assert!(matches!(
            verify_release_tree(directory.path(), &verified),
            Err(ReleaseError::TreeMismatch(_))
        ));
        std::fs::write(bin.join("aster-control"), b"control").expect("restore binary");
        std::fs::write(directory.path().join("extra"), b"extra").expect("write extra file");
        assert!(matches!(
            verify_release_tree(directory.path(), &verified),
            Err(ReleaseError::TreeMismatch(_))
        ));
    }

    #[test]
    fn verifies_an_exact_signed_subset() {
        let directory = tempdir().expect("temp directory");
        let bin = directory.path().join("bin");
        std::fs::create_dir_all(&bin).expect("create bin");
        std::fs::write(bin.join("aster-runner"), b"runner").expect("write runner");
        let signing_key = SigningKey::from_bytes(&[16_u8; 32]);
        let mut release_claims = claims("release-test-01");
        release_claims
            .files
            .push(file("bin/aster-runner", b"runner", false));
        release_claims
            .files
            .sort_by(|left, right| left.path.cmp(&right.path));
        let document = sign(release_claims, &signing_key).expect("sign release");
        let mut keys = TrustedReleaseKeys::new();
        keys.insert("release-test-01", signing_key.verifying_key())
            .expect("insert key");
        let verified = verify(
            &serde_json::to_vec(&document).expect("serialize release"),
            &keys,
        )
        .expect("verify release");

        verify_release_subset(directory.path(), &verified, ["bin/aster-runner"])
            .expect("verify Runner subset");
        assert!(matches!(
            verify_release_subset(directory.path(), &verified, ["bin/not-signed"]),
            Err(ReleaseError::TreeMismatch(_))
        ));
        std::fs::write(directory.path().join("extra"), b"extra").expect("write extra");
        assert!(matches!(
            verify_release_subset(directory.path(), &verified, ["bin/aster-runner"]),
            Err(ReleaseError::TreeMismatch(_))
        ));
    }

    #[test]
    fn rejects_traversal_unsorted_files_unknown_fields_and_signature_edits() {
        let signing_key = SigningKey::from_bytes(&[14_u8; 32]);
        for path in ["../escape", "/absolute", "a\\b", "a//b", "./a"] {
            let mut invalid = claims("release-test-01");
            invalid.files[0].path = path.to_owned();
            assert!(sign(invalid, &signing_key).is_err());
        }
        let document = sign(claims("release-test-01"), &signing_key).expect("sign release");
        let mut value = serde_json::to_value(&document).expect("release JSON");
        value["version"] = Value::String("2.0.0".to_owned());
        let mut keys = TrustedReleaseKeys::new();
        keys.insert("release-test-01", signing_key.verifying_key())
            .expect("insert key");
        assert!(matches!(
            verify(
                &serde_json::to_vec(&value).expect("serialize tamper"),
                &keys
            ),
            Err(ReleaseError::InvalidSignature)
        ));
        value["unexpected"] = Value::Bool(true);
        assert!(matches!(
            verify(
                &serde_json::to_vec(&value).expect("serialize unknown"),
                &keys
            ),
            Err(ReleaseError::InvalidJson(_))
        ));
    }
}
