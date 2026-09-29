//! Immutable build-time v2 trust configuration. Every issuer requires a scope;
//! legacy, missing or malformed policies never create trust.

use std::collections::BTreeSet;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{
    VerifyingKey,
    pkcs8::{DecodePublicKey, EncodePublicKey},
};
use serde::Deserialize;

use crate::{LicenseError, TrustedLicenseKeys, decode_exact, v2};

const MAX_KEYRING_BYTES: usize = 1 << 20;
const MAX_KEYS: usize = 8;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyProfile {
    key_id: String,
    public_key_spki: String,
    policy: v2::IssuerPolicy,
}

impl TrustedLicenseKeys {
    /// Parses the same profiles emitted by Operations and validated at build time.
    /// Empty trust is allowed only by explicitly constructing `new()` for callers
    /// that need an unlicensed test state, not by accepting empty configuration.
    pub fn from_json(data: &[u8]) -> Result<Self, v2::Error> {
        if data.len() > MAX_KEYRING_BYTES {
            return Err(LicenseError::InvalidField("trusted_keys").into());
        }
        let entries: Vec<KeyProfile> = decode_exact(data)?;
        if entries.is_empty() || entries.len() > MAX_KEYS {
            return Err(LicenseError::InvalidField("trusted_keys").into());
        }
        let mut keys = Self::new();
        let mut public_keys = BTreeSet::new();
        for entry in entries {
            let der = URL_SAFE_NO_PAD
                .decode(&entry.public_key_spki)
                .map_err(|_| LicenseError::PublicKeyEncoding)?;
            let key = VerifyingKey::from_public_key_der(&der)
                .map_err(|_| LicenseError::PublicKeyEncoding)?;
            let canonical = key
                .to_public_key_der()
                .map_err(|_| LicenseError::PublicKeyEncoding)?;
            if URL_SAFE_NO_PAD.encode(canonical.as_bytes()) != entry.public_key_spki {
                return Err(LicenseError::PublicKeyEncoding.into());
            }
            if !public_keys.insert(key.to_bytes()) {
                return Err(LicenseError::ConflictingTrustedKey.into());
            }
            keys.insert_scoped(entry.key_id, key, entry.policy)?;
        }
        Ok(keys)
    }
}
