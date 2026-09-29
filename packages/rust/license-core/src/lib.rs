#![forbid(unsafe_code)]

pub mod catalog;
pub mod entitlements;
pub mod request_v2;
mod trust;
pub mod v2;
mod verified_ref;

pub use verified_ref::{VerifiedBinding, VerifiedLicenseRef};

use std::{cmp::Ordering, collections::BTreeMap};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{
    Signature, Signer, SigningKey, Verifier, VerifyingKey, pkcs8::DecodePublicKey,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

pub const LICENSE_SCHEMA: &str = "aster.license.v1";
pub const LICENSE_REQUEST_SCHEMA: &str = "aster.license-request.v1";
pub const PRODUCT: &str = "aster-team";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LicenseLimits {
    pub member_seats: u32,
    pub seat_over_limit_grace_days: u16,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LicenseClaims {
    pub schema: String,
    pub key_id: String,
    pub license_id: String,
    pub serial: String,
    pub request_id: String,
    pub customer_ref: String,
    pub product: String,
    pub edition: String,
    pub features: Vec<String>,
    pub limits: LicenseLimits,
    pub minimum_version: String,
    pub installation_id: String,
    pub machine_fingerprint_sha256: String,
    pub transfer_sequence: u32,
    pub issued_at: String,
    pub not_before: String,
    pub expires_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LicenseDocument {
    #[serde(flatten)]
    pub claims: LicenseClaims,
    pub signature: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LicenseWire {
    schema: String,
    key_id: String,
    license_id: String,
    serial: String,
    request_id: String,
    customer_ref: String,
    product: String,
    edition: String,
    features: Vec<String>,
    limits: LicenseLimits,
    minimum_version: String,
    installation_id: String,
    machine_fingerprint_sha256: String,
    transfer_sequence: u32,
    issued_at: String,
    not_before: String,
    expires_at: String,
    signature: String,
}

impl LicenseWire {
    fn into_document(self) -> LicenseDocument {
        LicenseDocument {
            claims: LicenseClaims {
                schema: self.schema,
                key_id: self.key_id,
                license_id: self.license_id,
                serial: self.serial,
                request_id: self.request_id,
                customer_ref: self.customer_ref,
                product: self.product,
                edition: self.edition,
                features: self.features,
                limits: self.limits,
                minimum_version: self.minimum_version,
                installation_id: self.installation_id,
                machine_fingerprint_sha256: self.machine_fingerprint_sha256,
                transfer_sequence: self.transfer_sequence,
                issued_at: self.issued_at,
                not_before: self.not_before,
                expires_at: self.expires_at,
            },
            signature: self.signature,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineFactor {
    pub kind: MachineFactorKind,
    pub sha256: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineFactorKind {
    DmiProductUuid,
    MachineId,
}

impl<'de> Deserialize<'de> for MachineFactorKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match String::deserialize(deserializer)?.as_str() {
            "dmi_product_uuid" => Ok(Self::DmiProductUuid),
            "machine_id" => Ok(Self::MachineId),
            _ => Err(serde::de::Error::custom("unknown machine factor kind")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LicenseRequest {
    pub schema: String,
    pub request_id: String,
    pub product: String,
    pub product_version: String,
    pub platform: String,
    pub architecture: String,
    pub installation_id: String,
    pub machine_fingerprint_sha256: String,
    pub machine_factors: Vec<MachineFactor>,
    pub generated_at: String,
}

#[derive(Clone, Debug)]
pub struct VerifiedLicense {
    document: LicenseDocument,
    source: Vec<u8>,
}

impl VerifiedLicense {
    pub fn document(&self) -> &LicenseDocument {
        &self.document
    }

    pub fn claims(&self) -> &LicenseClaims {
        &self.document.claims
    }

    pub fn into_document(self) -> LicenseDocument {
        self.document
    }

    /// Exact bytes whose strict structure and signature were verified together.
    pub fn source(&self) -> &[u8] {
        &self.source
    }
}

/// One complete license document accepted by the product runtime. Keeping the
/// protocol variants inside this enum prevents callers from combining an
/// authenticated v2 entitlement payload with fields from a different file.
#[derive(Clone, Debug)]
pub enum VerifiedProductLicense {
    V1(VerifiedLicense),
    V2(v2::Verified),
}

impl From<VerifiedLicense> for VerifiedProductLicense {
    fn from(value: VerifiedLicense) -> Self {
        Self::V1(value)
    }
}

impl From<v2::Verified> for VerifiedProductLicense {
    fn from(value: v2::Verified) -> Self {
        Self::V2(value)
    }
}

impl VerifiedProductLicense {
    pub fn as_ref(&self) -> VerifiedLicenseRef<'_> {
        self.into()
    }

    pub fn source(&self) -> &[u8] {
        match self {
            Self::V1(value) => value.source(),
            Self::V2(value) => value.source(),
        }
    }
}

#[derive(Default)]
pub struct TrustedLicenseKeys {
    keys: BTreeMap<String, VerifyingKey>,
    scoped_keys: BTreeMap<String, v2::ScopedKey>,
}

impl TrustedLicenseKeys {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_spki_base64url(
        &mut self,
        key_id: impl Into<String>,
        public_key_spki: &str,
    ) -> Result<(), LicenseError> {
        let key_id = key_id.into();
        validate_identifier("key_id", &key_id, 1, 128)?;
        let der = URL_SAFE_NO_PAD
            .decode(public_key_spki)
            .map_err(|_| LicenseError::PublicKeyEncoding)?;
        let key =
            VerifyingKey::from_public_key_der(&der).map_err(|_| LicenseError::PublicKeyEncoding)?;
        self.insert_legacy(key_id, key)
    }

    pub fn insert(
        &mut self,
        key_id: impl Into<String>,
        key: VerifyingKey,
    ) -> Result<(), LicenseError> {
        let key_id = key_id.into();
        validate_identifier("key_id", &key_id, 1, 128)?;
        self.insert_legacy(key_id, key)
    }

    fn insert_legacy(&mut self, key_id: String, key: VerifyingKey) -> Result<(), LicenseError> {
        if self.keys.contains_key(&key_id) || self.scoped_keys.contains_key(&key_id) {
            return Err(LicenseError::DuplicateTrustedKey(key_id));
        }
        if self.scoped_keys.values().any(|record| record.key == key) {
            return Err(LicenseError::ConflictingTrustedKey);
        }
        self.keys.insert(key_id, key);
        Ok(())
    }

    fn get(&self, key_id: &str) -> Result<&VerifyingKey, LicenseError> {
        self.keys
            .get(key_id)
            .ok_or_else(|| LicenseError::UntrustedKey(key_id.to_owned()))
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum LicenseError {
    #[error("license JSON is invalid: {0}")]
    InvalidJson(String),
    #[error("license contains trailing JSON")]
    TrailingJson,
    #[error("{0} is invalid")]
    InvalidField(&'static str),
    #[error("license time range is invalid")]
    InvalidTimeRange,
    #[error("license feature is duplicated")]
    DuplicateFeature,
    #[error("license machine factor is duplicated")]
    DuplicateMachineFactor,
    #[error("license machine factors are incomplete")]
    IncompleteMachineFactors,
    #[error("license signature encoding is invalid")]
    SignatureEncoding,
    #[error("license signature is invalid")]
    InvalidSignature,
    #[error("license public key encoding is invalid")]
    PublicKeyEncoding,
    #[error("license key id is not trusted: {0}")]
    UntrustedKey(String),
    #[error("trusted license key id is duplicated: {0}")]
    DuplicateTrustedKey(String),
    #[error("scoped signing material cannot be registered under another key id or protocol")]
    ConflictingTrustedKey,
    #[error("canonical JSON contains an unsupported number")]
    UnsupportedNumber,
    #[error("canonical JSON serialization failed")]
    CanonicalSerialization,
}

pub fn parse_request(data: &[u8]) -> Result<LicenseRequest, LicenseError> {
    let request: LicenseRequest = decode_exact(data)?;
    validate_request(&request)?;
    Ok(request)
}

pub fn sign(
    claims: LicenseClaims,
    signing_key: &SigningKey,
) -> Result<LicenseDocument, LicenseError> {
    validate_claims(&claims)?;
    let payload = canonical_claims(&claims)?;
    let signature = signing_key.sign(&payload);
    Ok(LicenseDocument {
        claims,
        signature: URL_SAFE_NO_PAD.encode(signature.to_bytes()),
    })
}

pub fn verify(
    data: &[u8],
    trusted_keys: &TrustedLicenseKeys,
) -> Result<VerifiedLicense, LicenseError> {
    let wire: LicenseWire = decode_exact(data)?;
    let document = wire.into_document();
    validate_claims(&document.claims)?;
    let signature_bytes = URL_SAFE_NO_PAD
        .decode(&document.signature)
        .map_err(|_| LicenseError::SignatureEncoding)?;
    let signature =
        Signature::from_slice(&signature_bytes).map_err(|_| LicenseError::SignatureEncoding)?;
    let payload = canonical_claims(&document.claims)?;
    trusted_keys
        .get(&document.claims.key_id)?
        .verify(&payload, &signature)
        .map_err(|_| LicenseError::InvalidSignature)?;
    Ok(VerifiedLicense {
        document,
        source: data.to_vec(),
    })
}

/// Verifies the v2 product baseline. Legacy internal documents are deliberately
/// rejected, even if their signatures are valid. The v2 verifier rejects
/// duplicate, missing and unknown fields and authenticates the original bytes.
pub fn verify_product_license(
    data: &[u8],
    trusted_keys: &TrustedLicenseKeys,
) -> Result<VerifiedProductLicense, v2::Error> {
    if data.len() > 1 << 20 {
        return Err(LicenseError::InvalidField("document_size").into());
    }
    let value: Value = decode_exact(data)?;
    if value
        .get("claims")
        .and_then(|claims| claims.get("schema"))
        .and_then(Value::as_str)
        == Some(v2::SCHEMA)
    {
        return v2::verify(data, trusted_keys).map(VerifiedProductLicense::V2);
    }
    Err(LicenseError::InvalidField("schema").into())
}

pub fn canonical_claims(claims: &LicenseClaims) -> Result<Vec<u8>, LicenseError> {
    let value = serde_json::to_value(claims).map_err(|_| LicenseError::CanonicalSerialization)?;
    canonicalize(&value)
}

pub fn canonicalize(value: &Value) -> Result<Vec<u8>, LicenseError> {
    let mut output = Vec::new();
    append_canonical(&mut output, value)?;
    Ok(output)
}

fn decode_exact<'de, T: Deserialize<'de>>(data: &'de [u8]) -> Result<T, LicenseError> {
    let mut deserializer = serde_json::Deserializer::from_slice(data);
    let value = T::deserialize(&mut deserializer)
        .map_err(|error| LicenseError::InvalidJson(error.to_string()))?;
    deserializer.end().map_err(|_| LicenseError::TrailingJson)?;
    Ok(value)
}

fn validate_request(request: &LicenseRequest) -> Result<(), LicenseError> {
    if request.schema != LICENSE_REQUEST_SCHEMA {
        return Err(LicenseError::InvalidField("schema"));
    }
    if request.product != PRODUCT {
        return Err(LicenseError::InvalidField("product"));
    }
    if !matches!(request.platform.as_str(), "linux" | "windows" | "macos") {
        return Err(LicenseError::InvalidField("platform"));
    }
    if request.architecture != "amd64" && request.architecture != "arm64" {
        return Err(LicenseError::InvalidField("architecture"));
    }
    validate_identifier("request_id", &request.request_id, 8, 128)?;
    validate_identifier("product_version", &request.product_version, 1, 64)?;
    validate_identifier("installation_id", &request.installation_id, 8, 128)?;
    validate_digest(
        "machine_fingerprint_sha256",
        &request.machine_fingerprint_sha256,
    )?;
    validate_exact_time("generated_at", &request.generated_at)?;
    if request.machine_factors.len() != 2 {
        return Err(LicenseError::IncompleteMachineFactors);
    }
    let mut dmi = false;
    let mut machine_id = false;
    for factor in &request.machine_factors {
        validate_digest("machine factor sha256", &factor.sha256)?;
        match factor.kind {
            MachineFactorKind::DmiProductUuid if dmi => {
                return Err(LicenseError::DuplicateMachineFactor);
            }
            MachineFactorKind::DmiProductUuid => dmi = true,
            MachineFactorKind::MachineId if machine_id => {
                return Err(LicenseError::DuplicateMachineFactor);
            }
            MachineFactorKind::MachineId => machine_id = true,
        }
    }
    if !dmi || !machine_id {
        return Err(LicenseError::IncompleteMachineFactors);
    }
    Ok(())
}

fn validate_claims(claims: &LicenseClaims) -> Result<(), LicenseError> {
    if claims.schema != LICENSE_SCHEMA {
        return Err(LicenseError::InvalidField("schema"));
    }
    if claims.product != PRODUCT {
        return Err(LicenseError::InvalidField("product"));
    }
    for (name, value) in [
        ("key_id", claims.key_id.as_str()),
        ("license_id", claims.license_id.as_str()),
        ("serial", claims.serial.as_str()),
        ("request_id", claims.request_id.as_str()),
        ("customer_ref", claims.customer_ref.as_str()),
        ("edition", claims.edition.as_str()),
        ("minimum_version", claims.minimum_version.as_str()),
        ("installation_id", claims.installation_id.as_str()),
    ] {
        validate_identifier(name, value, 1, 128)?;
    }
    validate_digest(
        "machine_fingerprint_sha256",
        &claims.machine_fingerprint_sha256,
    )?;
    if claims.transfer_sequence > 10_000 {
        return Err(LicenseError::InvalidField("transfer_sequence"));
    }
    if claims.features.is_empty() || claims.features.len() > 64 {
        return Err(LicenseError::InvalidField("features"));
    }
    let mut features = claims.features.clone();
    features.sort();
    if features.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(LicenseError::DuplicateFeature);
    }
    for feature in &claims.features {
        validate_identifier("feature", feature, 1, 64)?;
    }
    if claims.limits.member_seats > 1_000_000 || claims.limits.seat_over_limit_grace_days > 90 {
        return Err(LicenseError::InvalidField("limits"));
    }
    let issued_at = validate_exact_time("issued_at", &claims.issued_at)?;
    let not_before = validate_exact_time("not_before", &claims.not_before)?;
    let expires_at = validate_exact_time("expires_at", &claims.expires_at)?;
    if issued_at > not_before || not_before >= expires_at {
        return Err(LicenseError::InvalidTimeRange);
    }
    Ok(())
}

fn validate_identifier(
    name: &'static str,
    value: &str,
    minimum: usize,
    maximum: usize,
) -> Result<(), LicenseError> {
    if value.len() < minimum
        || value.len() > maximum
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'.' | b'_' | b':' | b'@' | b'+' | b'/' | b'-')
        })
    {
        return Err(LicenseError::InvalidField(name));
    }
    Ok(())
}

fn validate_digest(name: &'static str, value: &str) -> Result<(), LicenseError> {
    if value.len() != 43
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(LicenseError::InvalidField(name));
    }
    Ok(())
}

fn validate_exact_time(name: &'static str, value: &str) -> Result<OffsetDateTime, LicenseError> {
    if value.len() != 24 || value.as_bytes().get(19) != Some(&b'.') || !value.ends_with('Z') {
        return Err(LicenseError::InvalidField(name));
    }
    let parsed =
        OffsetDateTime::parse(value, &Rfc3339).map_err(|_| LicenseError::InvalidField(name))?;
    if parsed.offset() != UtcOffset::UTC {
        return Err(LicenseError::InvalidField(name));
    }
    Ok(parsed)
}

fn append_canonical(output: &mut Vec<u8>, value: &Value) -> Result<(), LicenseError> {
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(value) => output.extend_from_slice(if *value { b"true" } else { b"false" }),
        Value::String(value) => output.extend_from_slice(
            serde_json::to_string(value)
                .map_err(|_| LicenseError::CanonicalSerialization)?
                .as_bytes(),
        ),
        Value::Number(value) => {
            if value.as_i64().is_none() && value.as_u64().is_none() {
                return Err(LicenseError::UnsupportedNumber);
            }
            output.extend_from_slice(value.to_string().as_bytes());
        }
        Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                append_canonical(output, value)?;
            }
            output.push(b']');
        }
        Value::Object(values) => {
            let mut keys: Vec<_> = values.keys().collect();
            keys.sort_by(|left, right| compare_utf16(left, right));
            output.push(b'{');
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                output.extend_from_slice(
                    serde_json::to_string(key)
                        .map_err(|_| LicenseError::CanonicalSerialization)?
                        .as_bytes(),
                );
                output.push(b':');
                append_canonical(output, &values[*key])?;
            }
            output.push(b'}');
        }
    }
    Ok(())
}

fn compare_utf16(left: &str, right: &str) -> Ordering {
    left.encode_utf16().cmp(right.encode_utf16())
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{SigningKey, pkcs8::EncodePublicKey};
    use serde_json::json;

    use super::*;

    fn claims() -> LicenseClaims {
        LicenseClaims {
            schema: LICENSE_SCHEMA.to_owned(),
            key_id: "license-test-01".to_owned(),
            license_id: "license_test_001".to_owned(),
            serial: "AT-TEST-001".to_owned(),
            request_id: "request_test_001".to_owned(),
            customer_ref: "customer_test_001".to_owned(),
            product: PRODUCT.to_owned(),
            edition: "enterprise".to_owned(),
            features: vec!["member".to_owned(), "runner".to_owned()],
            limits: LicenseLimits {
                member_seats: 10,
                seat_over_limit_grace_days: 7,
            },
            minimum_version: "1.0.0".to_owned(),
            installation_id: "installation_test_001".to_owned(),
            machine_fingerprint_sha256: "A".repeat(43),
            transfer_sequence: 0,
            issued_at: "2026-08-26T12:00:00.000Z".to_owned(),
            not_before: "2026-08-26T12:00:00.000Z".to_owned(),
            expires_at: "2027-08-26T12:00:00.000Z".to_owned(),
        }
    }

    fn keyring(signing_key: &SigningKey) -> TrustedLicenseKeys {
        let der = signing_key
            .verifying_key()
            .to_public_key_der()
            .expect("encode SPKI");
        let mut keys = TrustedLicenseKeys::new();
        keys.insert_spki_base64url("license-test-01", &URL_SAFE_NO_PAD.encode(der.as_bytes()))
            .expect("insert key");
        keys
    }

    #[test]
    fn signs_and_verifies_the_final_schema() {
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let document = sign(claims(), &signing_key).expect("sign license");
        let encoded = serde_json::to_vec(&document).expect("serialize license");
        let verified = verify(&encoded, &keyring(&signing_key)).expect("verify license");
        assert_eq!(verified.claims().limits.member_seats, 10);
        assert_eq!(verified.claims().key_id, "license-test-01");
        assert_eq!(verified.source(), encoded);
        let mut formatted = serde_json::to_vec_pretty(&document).unwrap();
        formatted.push(b'\n');
        let formatted_verified = verify(&formatted, &keyring(&signing_key)).unwrap();
        assert_eq!(formatted_verified.source(), formatted);
        assert_eq!(formatted_verified.claims(), verified.claims());

        assert!(verify_product_license(&formatted, &keyring(&signing_key)).is_err());
        let product = VerifiedProductLicense::from(formatted_verified);
        assert!(matches!(product, VerifiedProductLicense::V1(_)));
        assert_eq!(product.source(), formatted);
        assert_eq!(product.as_ref().protocol_schema(), LICENSE_SCHEMA);
        assert!(product.as_ref().has_feature("member"));
        assert!(product.as_ref().has_feature("runner"));
        assert!(!product.as_ref().has_feature("gateway"));
        assert_eq!(
            product.as_ref().quota(catalog::QuotaId::MemberSeats),
            entitlements::QuotaLimit::Limited { value: 10 }
        );
        assert_eq!(
            product.as_ref().quota(catalog::QuotaId::Runners),
            entitlements::QuotaLimit::Unlimited {}
        );
        assert_eq!(
            product.as_ref().quota(catalog::QuotaId::UpstreamAccounts),
            entitlements::QuotaLimit::Unlimited {}
        );
        assert_eq!(
            product.as_ref().quota(catalog::QuotaId::ApiKeysPerMember),
            entitlements::QuotaLimit::Unlimited {}
        );
        assert_eq!(product.as_ref().seat_over_limit_grace_days(), 7);
    }

    #[test]
    fn bridge_keyring_accepts_old_and_new_license_signatures_during_rotation() {
        let old_key = SigningKey::from_bytes(&[17_u8; 32]);
        let new_key = SigningKey::from_bytes(&[18_u8; 32]);
        let mut old_claims = claims();
        old_claims.key_id = "license-2026-01".to_owned();
        let mut new_claims = claims();
        new_claims.key_id = "license-2027-01".to_owned();
        new_claims.license_id = "license_test_002".to_owned();
        new_claims.serial = "AT-TEST-002".to_owned();
        new_claims.request_id = "request_test_002".to_owned();
        let old_document = sign(old_claims, &old_key).expect("sign old license");
        let new_document = sign(new_claims, &new_key).expect("sign new license");
        let mut bridge = TrustedLicenseKeys::new();
        bridge
            .insert("license-2026-01", old_key.verifying_key())
            .expect("insert old public key");
        bridge
            .insert("license-2027-01", new_key.verifying_key())
            .expect("insert new public key");
        assert!(
            verify(
                &serde_json::to_vec(&old_document).expect("encode old license"),
                &bridge,
            )
            .is_ok()
        );
        assert!(
            verify(
                &serde_json::to_vec(&new_document).expect("encode new license"),
                &bridge,
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_removed_runner_and_admin_limits() {
        let signing_key = SigningKey::from_bytes(&[8_u8; 32]);
        let document = sign(claims(), &signing_key).expect("sign license");
        let mut value = serde_json::to_value(document).expect("serialize license");
        let limits = value["limits"].as_object_mut().expect("limits object");
        limits.insert("runners".to_owned(), json!(5));
        limits.insert("admin_seats".to_owned(), json!(2));
        let error = verify(
            &serde_json::to_vec(&value).expect("encode modified document"),
            &keyring(&signing_key),
        )
        .expect_err("removed fields must be rejected");
        assert!(matches!(error, LicenseError::InvalidJson(_)));
    }

    #[test]
    fn rejects_unknown_key_id_before_signature_verification() {
        let signing_key = SigningKey::from_bytes(&[9_u8; 32]);
        let mut document = sign(claims(), &signing_key).expect("sign license");
        document.claims.key_id = "license-unknown".to_owned();
        let error = verify(
            &serde_json::to_vec(&document).expect("serialize license"),
            &TrustedLicenseKeys::new(),
        )
        .expect_err("unknown key must fail");
        assert_eq!(
            error,
            LicenseError::UntrustedKey("license-unknown".to_owned())
        );
    }

    #[test]
    fn rejects_duplicate_and_trailing_fields() {
        let signing_key = SigningKey::from_bytes(&[10_u8; 32]);
        let document = sign(claims(), &signing_key).expect("sign license");
        let encoded = serde_json::to_string(&document).expect("serialize license");
        let duplicated = encoded.replacen(
            "\"schema\":\"aster.license.v1\"",
            "\"schema\":\"aster.license.v1\",\"schema\":\"aster.license.v1\"",
            1,
        );
        assert!(matches!(
            verify(duplicated.as_bytes(), &keyring(&signing_key)),
            Err(LicenseError::InvalidJson(_))
        ));
        assert!(matches!(
            verify(format!("{encoded} {{}}").as_bytes(), &keyring(&signing_key)),
            Err(LicenseError::TrailingJson)
        ));
    }

    #[test]
    fn canonical_json_orders_keys_by_utf16() {
        let value = json!({"\u{e000}": 1, "\u{10000}": 2});
        assert_eq!(
            String::from_utf8(canonicalize(&value).expect("canonicalize")).expect("UTF-8"),
            "{\"𐀀\":2,\"\":1}"
        );
    }

    #[test]
    fn parses_exact_machine_request() {
        let mut request = json!({
            "schema": LICENSE_REQUEST_SCHEMA,
            "request_id": "request_test_001",
            "product": PRODUCT,
            "product_version": "1.0.0",
            "platform": "linux",
            "architecture": "amd64",
            "installation_id": "installation_test_001",
            "machine_fingerprint_sha256": "A".repeat(43),
            "machine_factors": [
                {"kind": "dmi_product_uuid", "sha256": "B".repeat(43)},
                {"kind": "machine_id", "sha256": "C".repeat(43)}
            ],
            "generated_at": "2026-08-26T12:00:00.000Z"
        });
        for platform in ["linux", "windows", "macos"] {
            request["platform"] = json!(platform);
            let parsed = parse_request(&serde_json::to_vec(&request).expect("serialize request"))
                .expect("parse request");
            assert_eq!(parsed.platform, platform);
            assert_eq!(parsed.architecture, "amd64");
        }
        request["platform"] = json!("other");
        assert!(parse_request(&serde_json::to_vec(&request).expect("serialize request")).is_err());
    }

    #[test]
    fn verifies_go_signed_cross_language_fixture() {
        let fixture: Value = serde_json::from_slice(include_bytes!(
            "../../../../contracts/test-vectors/license.v1.json"
        ))
        .expect("decode shared fixture");
        let public_key = fixture["public_key_spki"]
            .as_str()
            .expect("fixture public key");
        let document = serde_json::to_vec(&fixture["document"]).expect("encode fixture document");
        let mut keys = TrustedLicenseKeys::new();
        keys.insert_spki_base64url("license-cross-language-v1", public_key)
            .expect("load shared public key");
        assert!(verify_product_license(&document, &keys).is_err());
        let verified = VerifiedProductLicense::from(
            verify(&document, &keys).expect("verify historical cryptographic fixture"),
        );
        assert!(matches!(verified, VerifiedProductLicense::V1(_)));
        assert_eq!(verified.as_ref().license_id(), "license_cross_language_001");
        assert_eq!(
            verified.as_ref().quota(catalog::QuotaId::MemberSeats),
            entitlements::QuotaLimit::Limited { value: 25 }
        );
        for quota in [
            catalog::QuotaId::Runners,
            catalog::QuotaId::UpstreamAccounts,
            catalog::QuotaId::ApiKeysPerMember,
        ] {
            assert_eq!(
                verified.as_ref().quota(quota),
                entitlements::QuotaLimit::Unlimited {}
            );
        }
        assert_eq!(verified.as_ref().seat_over_limit_grace_days(), 7);
        assert!(verified.as_ref().has_feature("gateway"));
        assert!(verified.as_ref().has_feature("member"));
        assert!(verified.as_ref().has_feature("runner"));
        assert!(!verified.as_ref().has_feature("future-capability"));
        assert!(matches!(
            verified.as_ref().binding(),
            VerifiedBinding::Installation {
                installation_id: "installation_cross_language_001",
                machine_fingerprint_sha256: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                transfer_sequence: 0,
            }
        ));
    }
}
