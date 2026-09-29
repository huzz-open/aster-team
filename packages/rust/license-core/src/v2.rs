//! Signed v2 documents for both free distribution and installation-bound sales.
//! Verification authenticates the complete document and issuer scope. Runtime
//! time, installation, history, role and consumption checks remain separate.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{
    Signature, Signer, SigningKey, Verifier, VerifyingKey, pkcs8::DecodePublicKey,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::entitlements::{EntitlementError, Entitlements};
use crate::{
    LicenseError, TrustedLicenseKeys, canonicalize, decode_exact, validate_digest,
    validate_exact_time, validate_identifier,
};

pub const SCHEMA: &str = "aster.license.v2";
pub const QUOTA_POLICY_VERSION: u32 = 1;

// Deserialize strings explicitly: Serde's default unit-enum representation also
// accepts objects such as {"unbound": null}, outside the shared JSON contract.
macro_rules! policy_kind {
    ($name:ident { $($variant:ident => $wire:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
        pub enum $name { $(#[serde(rename = $wire)] $variant),+ }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                match String::deserialize(deserializer)?.as_str() {
                    $($wire => Ok(Self::$variant),)+
                    _ => Err(serde::de::Error::custom(concat!("unknown ", stringify!($name)))),
                }
            }
        }
    };
}
policy_kind!(SourceKind {
    FreeDistribution => "free_distribution",
    CommercialOrder => "commercial_order",
    ApprovedTrial => "approved_trial",
});
policy_kind!(BindingKind { Unbound => "unbound", Installation => "installation" });
policy_kind!(ExpiryKind { Fixed => "fixed", None => "none" });

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Source {
    FreeDistribution {
        distribution_id: String,
    },
    CommercialOrder {
        order_id: String,
        customer_ref: String,
        request_id: String,
    },
    ApprovedTrial {
        trial_id: String,
        customer_ref: String,
        request_id: String,
    },
}

impl Source {
    pub const fn kind(&self) -> SourceKind {
        match self {
            Self::FreeDistribution { .. } => SourceKind::FreeDistribution,
            Self::CommercialOrder { .. } => SourceKind::CommercialOrder,
            Self::ApprovedTrial { .. } => SourceKind::ApprovedTrial,
        }
    }

    fn validate(&self) -> Result<(), Error> {
        let identifiers: Vec<(&'static str, &str)> = match self {
            Self::FreeDistribution { distribution_id } => {
                vec![("distribution_id", distribution_id)]
            }
            Self::CommercialOrder {
                order_id,
                customer_ref,
                request_id,
            } => vec![
                ("order_id", order_id),
                ("customer_ref", customer_ref),
                ("request_id", request_id),
            ],
            Self::ApprovedTrial {
                trial_id,
                customer_ref,
                request_id,
            } => vec![
                ("trial_id", trial_id),
                ("customer_ref", customer_ref),
                ("request_id", request_id),
            ],
        };
        for (name, value) in identifiers {
            validate_identifier(name, value, 1, 128)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum Binding {
    Unbound {},
    Installation {
        installation_id: String,
        machine_fingerprint_sha256: String,
        transfer_sequence: u32,
    },
}

impl Binding {
    pub const fn kind(&self) -> BindingKind {
        match self {
            Self::Unbound {} => BindingKind::Unbound,
            Self::Installation { .. } => BindingKind::Installation,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum Expiry {
    Fixed { expires_at: String },
    None {},
}
impl Expiry {
    pub const fn kind(&self) -> ExpiryKind {
        match self {
            Self::Fixed { .. } => ExpiryKind::Fixed,
            Self::None {} => ExpiryKind::None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Validity {
    pub not_before: String,
    pub expiry: Expiry,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claims {
    pub schema: String,
    pub key_id: String,
    pub license_id: String,
    pub serial: String,
    pub product: String,
    pub edition: String,
    pub plan_id: String,
    pub plan_version: u32,
    pub source: Source,
    pub entitlements: Entitlements,
    pub quota_policy_version: u32,
    pub binding: Binding,
    pub validity: Validity,
    pub issued_at: String,
    pub minimum_version: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub claims: Claims,
    pub signature: String,
}

#[derive(Clone, Debug)]
pub struct Verified {
    document: Document,
    source: Vec<u8>,
}
impl Verified {
    pub fn claims(&self) -> &Claims {
        &self.document.claims
    }
    pub fn document(&self) -> &Document {
        &self.document
    }
    pub fn into_document(self) -> Document {
        self.document
    }
    /// Exact bytes whose strict structure, signature and issuer scope were
    /// verified together.
    pub fn source(&self) -> &[u8] {
        &self.source
    }
}

/// Trusted configuration, never accepted from a customer License or local plan table.
/// There is no implicit unrestricted v2 key. Legacy registrations accept v1 only.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssuerPolicy {
    pub sources: Vec<SourceKind>,
    pub bindings: Vec<BindingKind>,
    pub expiries: Vec<ExpiryKind>,
    pub entitlement_ceiling: Entitlements,
}

impl IssuerPolicy {
    pub fn validate(&self) -> Result<(), Error> {
        fn nonempty_unique<T: PartialEq>(items: &[T]) -> bool {
            !items.is_empty()
                && items
                    .iter()
                    .enumerate()
                    .all(|(i, value)| !items[..i].contains(value))
        }
        if !nonempty_unique(&self.sources)
            || !nonempty_unique(&self.bindings)
            || !nonempty_unique(&self.expiries)
            || (self.sources.contains(&SourceKind::FreeDistribution) && self.sources.len() != 1)
        {
            return Err(Error::IssuerPolicy);
        }
        self.entitlement_ceiling.validate()?;
        if !self.entitlement_ceiling.feature_sets.is_empty()
            && self.sources != [SourceKind::CommercialOrder]
        {
            return Err(Error::IssuerPolicy);
        }
        Ok(())
    }

    pub fn authorize(&self, claims: &Claims) -> Result<(), Error> {
        self.validate()?;
        claims.validate()?;
        if !self.sources.contains(&claims.source.kind())
            || !self.bindings.contains(&claims.binding.kind())
            || !self.expiries.contains(&claims.validity.expiry.kind())
        {
            return Err(Error::IssuerScope);
        }
        claims
            .entitlements
            .ensure_within(&self.entitlement_ceiling)?;
        Ok(())
    }
}

pub(crate) struct ScopedKey {
    pub(super) key: VerifyingKey,
    policy: IssuerPolicy,
}
impl TrustedLicenseKeys {
    pub fn insert_scoped(
        &mut self,
        key_id: impl Into<String>,
        key: VerifyingKey,
        policy: IssuerPolicy,
    ) -> Result<(), Error> {
        let key_id = key_id.into();
        validate_identifier("key_id", &key_id, 1, 128)?;
        policy.validate()?;
        if self.keys.contains_key(&key_id) || self.scoped_keys.contains_key(&key_id) {
            return Err(LicenseError::DuplicateTrustedKey(key_id).into());
        }
        if self.keys.values().any(|existing| *existing == key)
            || self.scoped_keys.values().any(|record| record.key == key)
        {
            return Err(LicenseError::ConflictingTrustedKey.into());
        }
        self.scoped_keys.insert(key_id, ScopedKey { key, policy });
        Ok(())
    }

    pub fn insert_scoped_spki_base64url(
        &mut self,
        key_id: impl Into<String>,
        public_key_spki: &str,
        policy: IssuerPolicy,
    ) -> Result<(), Error> {
        let der = URL_SAFE_NO_PAD
            .decode(public_key_spki)
            .map_err(|_| LicenseError::PublicKeyEncoding)?;
        let key =
            VerifyingKey::from_public_key_der(&der).map_err(|_| LicenseError::PublicKeyEncoding)?;
        self.insert_scoped(key_id, key, policy)
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum Error {
    #[error(transparent)]
    License(#[from] LicenseError),
    #[error(transparent)]
    Entitlements(#[from] EntitlementError),
    #[error("issuer policy must explicitly declare its allowed scope")]
    IssuerPolicy,
    #[error("license exceeds issuer policy")]
    IssuerScope,
    #[error("license source, binding and expiry are inconsistent")]
    SourcePolicy,
}

impl Claims {
    pub fn validate(&self) -> Result<(), Error> {
        if self.schema != SCHEMA {
            return Err(LicenseError::InvalidField("schema").into());
        }
        if self.product != crate::PRODUCT {
            return Err(LicenseError::InvalidField("product").into());
        }
        for (name, value) in [
            ("key_id", self.key_id.as_str()),
            ("license_id", self.license_id.as_str()),
            ("serial", self.serial.as_str()),
            ("edition", self.edition.as_str()),
            ("plan_id", self.plan_id.as_str()),
        ] {
            validate_identifier(name, value, 1, 128)?;
        }
        if self.plan_version == 0 {
            return Err(LicenseError::InvalidField("plan_version").into());
        }
        if self.quota_policy_version != QUOTA_POLICY_VERSION {
            return Err(LicenseError::InvalidField("quota_policy_version").into());
        }
        if self.minimum_version.len() > 64 || semver::Version::parse(&self.minimum_version).is_err()
        {
            return Err(LicenseError::InvalidField("minimum_version").into());
        }
        self.entitlements.validate()?;
        if !self.entitlements.feature_sets.is_empty()
            && self.source.kind() != SourceKind::CommercialOrder
        {
            return Err(Error::IssuerPolicy);
        }
        self.source.validate()?;
        match (&self.source, &self.binding, &self.validity.expiry) {
            (Source::FreeDistribution { .. }, Binding::Unbound {}, _) => {}
            (
                Source::CommercialOrder { .. } | Source::ApprovedTrial { .. },
                Binding::Installation { .. },
                Expiry::Fixed { .. },
            ) => {}
            _ => return Err(Error::SourcePolicy),
        }
        if let Binding::Installation {
            installation_id,
            machine_fingerprint_sha256,
            transfer_sequence,
        } = &self.binding
        {
            validate_identifier("installation_id", installation_id, 8, 128)?;
            validate_digest("machine_fingerprint_sha256", machine_fingerprint_sha256)?;
            if *transfer_sequence > 10_000 {
                return Err(LicenseError::InvalidField("transfer_sequence").into());
            }
        }
        let issued = exact_time("issued_at", &self.issued_at)?;
        let begins = exact_time("not_before", &self.validity.not_before)?;
        if let Expiry::Fixed { expires_at } = &self.validity.expiry {
            let ends = exact_time("expires_at", expires_at)?;
            if begins >= ends || issued >= ends {
                return Err(LicenseError::InvalidTimeRange.into());
            }
        }
        Ok(())
    }
}

fn exact_time(name: &'static str, value: &str) -> Result<time::OffsetDateTime, LicenseError> {
    let parsed = validate_exact_time(name, value)?;
    let format = time::macros::format_description!(
        "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
    );
    if parsed
        .format(format)
        .map_err(|_| LicenseError::InvalidField(name))?
        != value
    {
        return Err(LicenseError::InvalidField(name));
    }
    Ok(parsed)
}

pub fn canonical_claims(claims: &Claims) -> Result<Vec<u8>, Error> {
    let value = serde_json::to_value(claims).map_err(|_| LicenseError::CanonicalSerialization)?;
    Ok(canonicalize(&value)?)
}

pub fn sign(claims: Claims, signing_key: &SigningKey) -> Result<Document, Error> {
    claims.validate()?;
    let signature = signing_key.sign(&canonical_claims(&claims)?);
    Ok(Document {
        claims,
        signature: URL_SAFE_NO_PAD.encode(signature.to_bytes()),
    })
}

pub fn verify(data: &[u8], trusted_keys: &TrustedLicenseKeys) -> Result<Verified, Error> {
    if data.len() > 1 << 20 {
        return Err(LicenseError::InvalidField("document_size").into());
    }
    let document: Document = decode_exact(data)?;
    document.claims.validate()?;
    let key_id = &document.claims.key_id;
    let scoped = trusted_keys
        .scoped_keys
        .get(key_id)
        .ok_or_else(|| LicenseError::UntrustedKey(key_id.clone()))?;
    let bytes = URL_SAFE_NO_PAD
        .decode(&document.signature)
        .map_err(|_| LicenseError::SignatureEncoding)?;
    let signature = Signature::from_slice(&bytes).map_err(|_| LicenseError::SignatureEncoding)?;
    scoped
        .key
        .verify(&canonical_claims(&document.claims)?, &signature)
        .map_err(|_| LicenseError::InvalidSignature)?;
    scoped.policy.authorize(&document.claims)?;
    Ok(Verified {
        document,
        source: data.to_vec(),
    })
}
