#![forbid(unsafe_code)]

mod transaction;
pub use transaction::{FreeSwitchGuard, LicenseMutationGuard};

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use aster_license_core::{VerifiedProductLicense, canonicalize, catalog::QuotaId};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hkdf::Hkdf;
use hmac::{Hmac, KeyInit as _, Mac as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use time::{
    Duration, OffsetDateTime,
    format_description::{FormatItem, well_known::Rfc3339},
    macros::format_description,
};
use zeroize::Zeroizing;

pub const LICENSE_STATE_SCHEMA: &str = "aster.license-state.v2";
pub const MAX_LICENSE_DOCUMENT_BYTES: usize = 64 * 1024;
const CLOCK_ROLLBACK_TOLERANCE: Duration = Duration::minutes(5);
const OBSERVATION_PERSIST_INTERVAL: Duration = Duration::minutes(1);
const STATE_TIME_FORMAT: &[FormatItem<'static>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z");

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct StateClaims {
    schema: String,
    last_license_sha256: Option<String>,
    last_license_id: Option<String>,
    last_issued_at: String,
    last_transfer_sequence: u32,
    last_seen_at: String,
    seat_over_limit_first_observed_at: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateWire {
    schema: String,
    last_license_sha256: String,
    last_license_id: String,
    last_issued_at: String,
    last_transfer_sequence: u32,
    last_seen_at: String,
    seat_over_limit_first_observed_at: Option<String>,
    mac: String,
}

impl StateWire {
    fn claims(&self) -> StateClaims {
        StateClaims {
            schema: self.schema.clone(),
            last_license_sha256: Some(self.last_license_sha256.clone()),
            last_license_id: Some(self.last_license_id.clone()),
            last_issued_at: self.last_issued_at.clone(),
            last_transfer_sequence: self.last_transfer_sequence,
            last_seen_at: self.last_seen_at.clone(),
            seat_over_limit_first_observed_at: self.seat_over_limit_first_observed_at.clone(),
        }
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum LicenseStateError {
    #[error("installation key must contain exactly 32 bytes")]
    InvalidInstallationKey,
    #[error("license state is missing")]
    Missing,
    #[error("license state could not be read or written")]
    Io,
    #[error("license state JSON is invalid")]
    InvalidJson,
    #[error("license state integrity check failed")]
    Integrity,
    #[error("license state contains an invalid timestamp")]
    InvalidTime,
    #[error("system time is earlier than the last trusted observation")]
    ClockRollback,
    #[error("license transfer sequence or issue time was rolled back")]
    LicenseRollback,
    #[error("license state already exists")]
    AlreadyInitialized,
    #[error("license state is busy in another process")]
    Busy,
    #[error("license state cryptographic operation failed")]
    Crypto,
    #[error("a license update must be recovered before observing history")]
    RecoveryRequired,
    #[error("license changed or a licensed mutation is still committing; retry the operation")]
    MutationConflict,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StagedActivationOutcome {
    Activated { staged_cleared: bool },
    AlreadyActive { staged_cleared: bool },
    Superseded { staged_cleared: bool },
    Changed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingLicenseActivation {
    license_id: String,
    license_sha256: String,
    activated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    import: Option<LicenseImportAudit>,
}

/// Captured before a verified import is accepted. CLI imports have no web actor.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LicenseImportAudit {
    pub event_id: String,
    pub actor: LicenseImportActor,
    pub action: LicenseImportAction,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LicenseImportActor {
    Administrator { identity_id: String, role: String },
    LocalCommand {},
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenseImportAction {
    Install,
    Schedule,
    SwitchFree,
}

impl LicenseImportAudit {
    pub(crate) fn valid(&self) -> bool {
        !self.event_id.is_empty()
            && self.event_id.len() <= 96
            && self.event_id.starts_with("license_import_")
            && self
                .event_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            && match &self.actor {
                LicenseImportActor::Administrator { identity_id, role } => {
                    !identity_id.is_empty()
                        && identity_id.len() <= 128
                        && matches!(role.as_str(), "owner" | "admin")
                }
                LicenseImportActor::LocalCommand {} => true,
            }
    }
}

impl PendingLicenseActivation {
    pub fn import(&self) -> Option<&LicenseImportAudit> {
        self.import.as_ref()
    }

    pub fn event_id(&self) -> String {
        self.import.as_ref().map_or_else(
            || format!("license_activate_{}", self.license_sha256),
            |import| import.event_id.clone(),
        )
    }

    pub fn action(&self) -> &'static str {
        match self.import.as_ref().map(|import| import.action) {
            None => "license.activate",
            Some(LicenseImportAction::Install) => "license.install",
            Some(LicenseImportAction::Schedule) => "license.schedule",
            Some(LicenseImportAction::SwitchFree) => "license.switch_free",
        }
    }

    pub fn license_id(&self) -> &str {
        &self.license_id
    }

    pub fn license_sha256(&self) -> &str {
        &self.license_sha256
    }

    pub fn activated_at(&self) -> &str {
        &self.activated_at
    }

    pub fn audit_target_id(&self) -> String {
        if self.import.is_some() {
            self.license_id.clone()
        } else {
            format!("license_sha256:{}", self.license_sha256)
        }
    }
}

#[derive(Clone)]
pub struct LicenseStateStore {
    path: PathBuf,
    state_key: Zeroizing<[u8; 32]>,
    gate: Arc<Mutex<()>>,
}

impl LicenseStateStore {
    pub fn new(
        path: impl Into<PathBuf>,
        installation_key: &[u8],
    ) -> Result<Self, LicenseStateError> {
        if installation_key.len() != 32 {
            return Err(LicenseStateError::InvalidInstallationKey);
        }
        let hkdf = Hkdf::<Sha256>::new(Some(b"aster-team-license-state-v1"), installation_key);
        let mut state_key = Zeroizing::new([0_u8; 32]);
        hkdf.expand(b"license-state-hmac", state_key.as_mut())
            .map_err(|_| LicenseStateError::Crypto)?;
        Ok(Self {
            path: path.into(),
            state_key,
            gate: Arc::new(Mutex::new(())),
        })
    }

    pub fn initialize(
        &self,
        license: &VerifiedProductLicense,
        now: OffsetDateTime,
    ) -> Result<(), LicenseStateError> {
        let _guard = self.lock_state()?;
        let _mutation = self.lock_mutations(false)?;
        self.require_no_pending_update()?;
        if self.path.exists() {
            return Err(LicenseStateError::AlreadyInitialized);
        }
        let claims = state_from_license(license, now)?;
        self.write(&claims)
    }

    pub fn is_initialized(&self) -> bool {
        self.path.is_file()
    }

    /// Check a loaded document against committed history without requiring it
    /// to be currently usable. Expired licenses remain inspectable for recovery.
    pub fn validate_document_progress(
        &self,
        license: &VerifiedProductLicense,
    ) -> Result<(), LicenseStateError> {
        let _guard = self.lock_state()?;
        self.require_no_pending_update()?;
        validate_license_progress(&self.load()?, license)
    }

    pub fn validate_replacement(
        &self,
        license: &VerifiedProductLicense,
        now: OffsetDateTime,
    ) -> Result<(), LicenseStateError> {
        let _guard = self.lock_state()?;
        self.require_no_pending_update()?;
        let current = self.load()?;
        validate_observation(&current, license, now)
    }

    pub fn check_and_observe(
        &self,
        license: &VerifiedProductLicense,
        now: OffsetDateTime,
    ) -> Result<(), LicenseStateError> {
        let _guard = self.lock_state()?;
        self.require_no_pending_update()?;
        let current = self.load()?;
        let updated = observe(current.clone(), license, now)?;
        if should_persist_observation(&current, &updated, now)? {
            let _mutation = self.guard_observation_change(&current, &updated)?;
            self.write(&updated)?;
        }
        Ok(())
    }

    pub fn check_observe_and_track_seats(
        &self,
        license: &VerifiedProductLicense,
        occupied_seats: u32,
        now: OffsetDateTime,
    ) -> Result<Option<OffsetDateTime>, LicenseStateError> {
        let _guard = self.lock_state()?;
        self.require_no_pending_update()?;
        let current = self.load()?;
        let mut updated = observe(current.clone(), license, now)?;
        let is_over_limit = !license
            .as_ref()
            .quota(QuotaId::MemberSeats)
            .permits(occupied_seats);
        if is_over_limit && updated.seat_over_limit_first_observed_at.is_none() {
            updated.seat_over_limit_first_observed_at = Some(format_time(now)?);
        } else if !is_over_limit {
            updated.seat_over_limit_first_observed_at = None;
        }
        let first_observed = updated
            .seat_over_limit_first_observed_at
            .as_deref()
            .map(parse_time)
            .transpose()?;
        if should_persist_observation(&current, &updated, now)? {
            let _mutation = self.guard_observation_change(&current, &updated)?;
            self.write(&updated)?;
        }
        Ok(first_observed)
    }

    pub fn accept_replacement(
        &self,
        license: &VerifiedProductLicense,
        now: OffsetDateTime,
    ) -> Result<(), LicenseStateError> {
        let _guard = self.lock_state()?;
        let _mutation = self.lock_mutations(false)?;
        self.require_no_pending_update()?;
        let current = self.load()?;
        let updated = observe(current, license, now)?;
        self.write(&updated)
    }

    fn guard_observation_change(
        &self,
        current: &StateClaims,
        updated: &StateClaims,
    ) -> Result<Option<LicenseMutationGuard>, LicenseStateError> {
        if current.last_license_sha256 != updated.last_license_sha256 {
            self.lock_mutations(false).map(Some)
        } else {
            Ok(None)
        }
    }

    fn load(&self) -> Result<StateClaims, LicenseStateError> {
        let data = transaction::read_optional(&self.path, transaction::MAX_STATE_BYTES)?
            .ok_or(LicenseStateError::Missing)?;
        self.decode_state(&data)
    }

    fn decode_state(&self, data: &[u8]) -> Result<StateClaims, LicenseStateError> {
        let mut deserializer = serde_json::Deserializer::from_slice(data);
        let wire = StateWire::deserialize(&mut deserializer)
            .map_err(|_| LicenseStateError::InvalidJson)?;
        deserializer
            .end()
            .map_err(|_| LicenseStateError::InvalidJson)?;
        if wire.schema != LICENSE_STATE_SCHEMA
            || !valid_digest(&wire.last_license_sha256)
            || wire.last_license_id.is_empty()
            || wire.last_license_id.len() > 128
            || wire.last_transfer_sequence > 10_000
            || parse_time(&wire.last_issued_at).is_err()
            || parse_time(&wire.last_seen_at).is_err()
            || wire
                .seat_over_limit_first_observed_at
                .as_deref()
                .is_some_and(|value| parse_time(value).is_err())
        {
            return Err(LicenseStateError::InvalidJson);
        }
        self.verify_mac(&wire.claims(), &wire.mac)?;
        Ok(wire.claims())
    }

    fn write(&self, claims: &StateClaims) -> Result<(), LicenseStateError> {
        transaction::write_atomic(&self.path, &self.encode_state(claims)?)
    }

    fn encode_state(&self, claims: &StateClaims) -> Result<Vec<u8>, LicenseStateError> {
        let mac = self.calculate_mac(claims)?;
        let value = serde_json::json!({
            "schema": claims.schema,
            "last_license_sha256": claims.last_license_sha256,
            "last_license_id": claims.last_license_id,
            "last_issued_at": claims.last_issued_at,
            "last_transfer_sequence": claims.last_transfer_sequence,
            "last_seen_at": claims.last_seen_at,
            "seat_over_limit_first_observed_at": claims.seat_over_limit_first_observed_at,
            "mac": mac,
        });
        let mut encoded = serde_json::to_vec_pretty(&value).map_err(|_| LicenseStateError::Io)?;
        encoded.push(b'\n');
        Ok(encoded)
    }

    fn calculate_mac(&self, claims: &StateClaims) -> Result<String, LicenseStateError> {
        self.calculate_mac_value(claims)
    }

    fn calculate_mac_value<T: Serialize>(&self, claims: &T) -> Result<String, LicenseStateError> {
        let value = serde_json::to_value(claims).map_err(|_| LicenseStateError::Crypto)?;
        let canonical = canonicalize(&value).map_err(|_| LicenseStateError::Crypto)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(self.state_key.as_ref())
            .map_err(|_| LicenseStateError::Crypto)?;
        mac.update(&canonical);
        Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
    }

    fn verify_mac(&self, claims: &StateClaims, encoded: &str) -> Result<(), LicenseStateError> {
        self.verify_mac_value(claims, encoded)
    }

    fn verify_mac_value<T: Serialize>(
        &self,
        claims: &T,
        encoded: &str,
    ) -> Result<(), LicenseStateError> {
        let actual = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| LicenseStateError::Integrity)?;
        let value = serde_json::to_value(claims).map_err(|_| LicenseStateError::Crypto)?;
        let canonical = canonicalize(&value).map_err(|_| LicenseStateError::Crypto)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(self.state_key.as_ref())
            .map_err(|_| LicenseStateError::Crypto)?;
        mac.update(&canonical);
        mac.verify_slice(&actual)
            .map_err(|_| LicenseStateError::Integrity)
    }
}

fn state_from_license(
    license: &VerifiedProductLicense,
    now: OffsetDateTime,
) -> Result<StateClaims, LicenseStateError> {
    parse_time(license.as_ref().issued_at())?;
    Ok(StateClaims {
        schema: LICENSE_STATE_SCHEMA.to_owned(),
        last_license_sha256: Some(license_digest(license)),
        last_license_id: Some(license.as_ref().license_id().to_owned()),
        last_issued_at: license.as_ref().issued_at().to_owned(),
        last_transfer_sequence: license.as_ref().transfer_sequence(),
        last_seen_at: format_time(now)?,
        seat_over_limit_first_observed_at: None,
    })
}

fn validate_license_progress(
    current: &StateClaims,
    license: &VerifiedProductLicense,
) -> Result<(), LicenseStateError> {
    // A later public free signature is still a downgrade, not an ordinary
    // renewal. It must use the explicit capacity-checked switch even when its
    // issue time is newer than the installed paid document. This also covers
    // staging and startup file replacement, not just the HTTP import endpoint.
    if matches!(license, VerifiedProductLicense::V2(verified)
        if matches!(verified.claims().source, aster_license_core::v2::Source::FreeDistribution { .. }))
        && current.last_license_sha256.as_deref() != Some(license_digest(license).as_str())
    {
        return Err(LicenseStateError::LicenseRollback);
    }
    // Only the exact free document committed by the explicit switch can sit
    // below the issue-time watermark. Its signature and issuer scope have
    // already been verified. The HMAC history, never a caller flag, names it.
    if is_recorded_free(current, license) {
        return Ok(());
    }
    let candidate = license.as_ref();
    let candidate_issued_at = parse_time(candidate.issued_at())?;
    let current_issued_at = parse_time(&current.last_issued_at)?;
    match current.last_license_id.as_deref() {
        Some(current_id) if current_id != candidate.license_id() => {
            // Renewal, upgrade, and free-to-paid conversion create a new
            // license identity whose transfer sequence starts at zero. A new
            // identity may therefore reset that sequence, but it must advance
            // the signed issue-time history strictly so an older license
            // identity cannot be replayed after the switch.
            if candidate_issued_at <= current_issued_at {
                return Err(LicenseStateError::LicenseRollback);
            }
        }
        _ => {
            // A migrated v1 history has no authenticated license ID yet, so it
            // retains the legacy global sequence rule for its first update.
            if candidate_issued_at < current_issued_at
                || candidate.transfer_sequence() < current.last_transfer_sequence
            {
                return Err(LicenseStateError::LicenseRollback);
            }
            if candidate.transfer_sequence() == current.last_transfer_sequence
                && candidate_issued_at == current_issued_at
                && current
                    .last_license_sha256
                    .as_deref()
                    .is_some_and(|digest| digest != license_digest(license))
            {
                return Err(LicenseStateError::LicenseRollback);
            }
        }
    }
    Ok(())
}

fn is_public_free(license: &VerifiedProductLicense) -> bool {
    use aster_license_core::v2::{Binding, Expiry, Source};
    matches!(license, VerifiedProductLicense::V2(verified)
        if matches!(verified.claims().source, Source::FreeDistribution { .. })
            && matches!(verified.claims().binding, Binding::Unbound {})
            && matches!(verified.claims().validity.expiry, Expiry::None {}))
}

fn is_recorded_free(current: &StateClaims, license: &VerifiedProductLicense) -> bool {
    is_public_free(license)
        && current.last_license_id.as_deref() == Some(license.as_ref().license_id())
        && current.last_license_sha256.as_deref() == Some(license_digest(license).as_str())
}

fn validate_observation(
    current: &StateClaims,
    license: &VerifiedProductLicense,
    now: OffsetDateTime,
) -> Result<(), LicenseStateError> {
    validate_license_progress(current, license)?;
    let last_seen = parse_time(&current.last_seen_at)?;
    if now + CLOCK_ROLLBACK_TOLERANCE < last_seen {
        return Err(LicenseStateError::ClockRollback);
    }
    Ok(())
}

fn observe(
    mut current: StateClaims,
    license: &VerifiedProductLicense,
    now: OffsetDateTime,
) -> Result<StateClaims, LicenseStateError> {
    validate_observation(&current, license, now)?;
    let last_seen = parse_time(&current.last_seen_at)?;
    if now > last_seen {
        current.last_seen_at = format_time(now)?;
    }
    if !is_recorded_free(&current, license)
        && (license.as_ref().transfer_sequence() > current.last_transfer_sequence
            || parse_time(license.as_ref().issued_at())? > parse_time(&current.last_issued_at)?)
    {
        current.last_transfer_sequence = license.as_ref().transfer_sequence();
        current.last_issued_at = license.as_ref().issued_at().to_owned();
    }
    current.schema = LICENSE_STATE_SCHEMA.to_owned();
    current.last_license_sha256 = Some(license_digest(license));
    current.last_license_id = Some(license.as_ref().license_id().to_owned());
    Ok(current)
}

fn should_persist_observation(
    current: &StateClaims,
    updated: &StateClaims,
    now: OffsetDateTime,
) -> Result<bool, LicenseStateError> {
    let last_seen = parse_time(&current.last_seen_at)?;
    Ok(updated.last_issued_at != current.last_issued_at
        || updated.last_transfer_sequence != current.last_transfer_sequence
        || updated.last_license_sha256 != current.last_license_sha256
        || updated.last_license_id != current.last_license_id
        || updated.schema != current.schema
        || updated.seat_over_limit_first_observed_at != current.seat_over_limit_first_observed_at
        || now >= last_seen + OBSERVATION_PERSIST_INTERVAL)
}

fn parse_time(value: &str) -> Result<OffsetDateTime, LicenseStateError> {
    if value.len() != 24 || !value.ends_with('Z') {
        return Err(LicenseStateError::InvalidTime);
    }
    OffsetDateTime::parse(value, &Rfc3339).map_err(|_| LicenseStateError::InvalidTime)
}

fn format_time(value: OffsetDateTime) -> Result<String, LicenseStateError> {
    value
        .format(STATE_TIME_FORMAT)
        .map_err(|_| LicenseStateError::InvalidTime)
}

fn license_digest(license: &VerifiedProductLicense) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(license.source()))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use aster_license_core::{
        LICENSE_SCHEMA, LicenseClaims, LicenseLimits, TrustedLicenseKeys, VerifiedProductLicense,
        sign,
        v2::{self, BindingKind, ExpiryKind, IssuerPolicy, SourceKind},
        verify,
    };
    use ed25519_dalek::SigningKey;
    use std::fs;
    use tempfile::tempdir;
    use time::macros::datetime;

    use super::*;

    pub(super) fn license(sequence: u32, issued_at: &str) -> VerifiedProductLicense {
        license_with_id(sequence, issued_at, "license_test_001")
    }

    pub(super) fn v2_license(sequence: u32, issued_at: &str) -> VerifiedProductLicense {
        let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../../contracts/test-vectors/license.v2.json"
        ))
        .expect("parse v2 fixture");
        let mut document: v2::Document =
            serde_json::from_value(fixture["cases"][2]["document"].clone())
                .expect("parse commercial v2 fixture");
        document.claims.license_id = format!("license_v2_{sequence}");
        document.claims.serial = format!("V2-{sequence}");
        document.claims.issued_at = issued_at.to_owned();
        document.claims.validity.not_before = issued_at.to_owned();
        document.claims.binding = v2::Binding::Installation {
            installation_id: "installation_test_001".to_owned(),
            machine_fingerprint_sha256: "A".repeat(43),
            transfer_sequence: sequence,
        };
        let signing_key = SigningKey::from_bytes(&[42_u8; 32]);
        let document = v2::sign(document.claims, &signing_key).expect("sign v2 license");
        let mut keys = TrustedLicenseKeys::new();
        keys.insert_scoped(
            "test-only-v2",
            signing_key.verifying_key(),
            IssuerPolicy {
                sources: vec![SourceKind::CommercialOrder],
                bindings: vec![BindingKind::Installation],
                expiries: vec![ExpiryKind::Fixed],
                entitlement_ceiling: document.claims.entitlements.clone(),
            },
        )
        .expect("insert v2 test key");
        v2::verify(&serde_json::to_vec(&document).unwrap(), &keys)
            .expect("verify v2 license")
            .into()
    }

    fn license_with_id(sequence: u32, issued_at: &str, license_id: &str) -> VerifiedProductLicense {
        let signing_key = SigningKey::from_bytes(&[41_u8; 32]);
        let claims = LicenseClaims {
            schema: LICENSE_SCHEMA.to_owned(),
            key_id: "license-test-01".to_owned(),
            license_id: license_id.to_owned(),
            serial: "AT-TEST-001".to_owned(),
            request_id: "request_test_001".to_owned(),
            customer_ref: "customer_test_001".to_owned(),
            product: "aster-team".to_owned(),
            edition: "enterprise".to_owned(),
            features: vec!["member".to_owned()],
            limits: LicenseLimits {
                member_seats: 10,
                seat_over_limit_grace_days: 0,
            },
            minimum_version: "1.0.0".to_owned(),
            installation_id: "installation_test_001".to_owned(),
            machine_fingerprint_sha256: "A".repeat(43),
            transfer_sequence: sequence,
            issued_at: issued_at.to_owned(),
            not_before: issued_at.to_owned(),
            expires_at: "2027-08-26T00:00:00.000Z".to_owned(),
        };
        let document = sign(claims, &signing_key).expect("sign license");
        let mut keys = TrustedLicenseKeys::new();
        keys.insert("license-test-01", signing_key.verifying_key())
            .expect("insert key");
        verify(
            &serde_json::to_vec(&document).expect("serialize license"),
            &keys,
        )
        .expect("verify license")
        .into()
    }

    #[test]
    fn initializes_and_observes_monotonic_state() {
        let directory = tempdir().expect("temp directory");
        let store = LicenseStateStore::new(directory.path().join("state.json"), &[7_u8; 32])
            .expect("create store");
        let license = license(0, "2026-08-26T00:00:00.000Z");
        store
            .initialize(&license, datetime!(2026-08-27 0:00 UTC))
            .expect("initialize state");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(directory.path().join("state.json"))
                    .expect("state metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o640
            );
        }
        store
            .check_and_observe(&license, datetime!(2026-08-28 0:00 UTC))
            .expect("observe state");
    }

    #[test]
    fn independent_stores_serialize_initialization_and_preserve_monotonic_observations() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("state.json");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
        let workers = (0..4)
            .map(|index| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let store = LicenseStateStore::new(path, &[7_u8; 32]).unwrap();
                    let license = license(0, "2026-08-26T00:00:00.000Z");
                    barrier.wait();
                    let result = store.initialize(&license, datetime!(2026-08-27 0:00 UTC));
                    assert!(result.is_ok() || result == Err(LicenseStateError::AlreadyInitialized));
                    store
                        .check_and_observe(
                            &license,
                            datetime!(2026-08-27 0:00 UTC) + Duration::seconds(60 + index * 60),
                        )
                        .unwrap();
                    result.is_ok()
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            workers
                .into_iter()
                .map(|worker| usize::from(worker.join().unwrap()))
                .sum::<usize>(),
            1
        );
        let store = LicenseStateStore::new(path, &[7_u8; 32]).unwrap();
        assert_eq!(
            store.load().unwrap().last_seen_at,
            "2026-08-27T00:04:00.000Z"
        );
    }

    #[test]
    fn coalesces_frequent_time_observations() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("state.json");
        let store = LicenseStateStore::new(&path, &[7_u8; 32]).expect("create store");
        let license = license(0, "2026-08-26T00:00:00.000Z");
        store
            .initialize(&license, datetime!(2026-08-27 0:00 UTC))
            .expect("initialize state");
        let initial = fs::read(&path).expect("read initial state");

        store
            .check_and_observe(&license, datetime!(2026-08-27 0:00:30 UTC))
            .expect("observe within persist interval");
        assert_eq!(
            fs::read(&path).expect("read coalesced state"),
            initial,
            "an observation inside the persist interval must not rewrite the state file"
        );

        store
            .check_and_observe(&license, datetime!(2026-08-27 0:01 UTC))
            .expect("observe at persist interval");
        assert_eq!(
            store.load().expect("load persisted state").last_seen_at,
            "2026-08-27T00:01:00.000Z"
        );
    }

    #[test]
    fn detects_state_tampering() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("state.json");
        let store = LicenseStateStore::new(&path, &[7_u8; 32]).expect("create store");
        let license = license(0, "2026-08-26T00:00:00.000Z");
        store
            .initialize(&license, datetime!(2026-08-27 0:00 UTC))
            .expect("initialize state");
        let source = fs::read_to_string(&path).expect("read state");
        fs::write(&path, source.replace("2026-08-27", "2026-08-20")).expect("tamper state");
        assert_eq!(
            store.check_and_observe(&license, datetime!(2026-08-28 0:00 UTC)),
            Err(LicenseStateError::Integrity)
        );
    }

    #[test]
    fn rejects_clock_and_license_rollbacks() {
        let directory = tempdir().expect("temp directory");
        let store = LicenseStateStore::new(directory.path().join("state.json"), &[7_u8; 32])
            .expect("create store");
        let current = license(1, "2026-08-27T00:00:00.000Z");
        store
            .initialize(&current, datetime!(2026-08-28 0:00 UTC))
            .expect("initialize state");
        assert_eq!(
            store.check_and_observe(&current, datetime!(2026-08-27 23:00 UTC)),
            Err(LicenseStateError::ClockRollback)
        );
        let older = license(0, "2026-08-26T00:00:00.000Z");
        assert_eq!(
            store.check_and_observe(&older, datetime!(2026-08-28 0:01 UTC)),
            Err(LicenseStateError::LicenseRollback)
        );
    }

    #[test]
    fn rejects_a_different_signed_document_at_the_same_history_position() {
        let directory = tempdir().expect("temp directory");
        let store = LicenseStateStore::new(directory.path().join("state.json"), &[7_u8; 32])
            .expect("create store");
        let current = license_with_id(1, "2026-08-27T00:00:00.000Z", "license_current");
        store
            .initialize(&current, datetime!(2026-08-28 0:00 UTC))
            .expect("initialize state");
        let conflicting = license_with_id(1, "2026-08-27T00:00:00.000Z", "license_conflicting");
        assert_eq!(
            store.check_and_observe(&conflicting, datetime!(2026-08-28 0:01 UTC)),
            Err(LicenseStateError::LicenseRollback)
        );
    }

    #[test]
    fn a_newer_license_identity_can_restart_its_transfer_sequence() {
        let directory = tempdir().expect("temp directory");
        let store = LicenseStateStore::new(directory.path().join("state.json"), &[7_u8; 32])
            .expect("create store");
        let transferred =
            license_with_id(7, "2026-08-27T00:00:00.000Z", "license_original_contract");
        store
            .initialize(&transferred, datetime!(2026-08-28 0:00 UTC))
            .expect("initialize transferred license history");

        let renewal = license_with_id(0, "2026-08-27T00:00:01.000Z", "license_renewal_contract");
        store
            .accept_replacement(&renewal, datetime!(2026-08-28 0:01 UTC))
            .expect("accept newer renewal identity at sequence zero");
        let updated = store.load().expect("load renewal history");
        assert_eq!(
            updated.last_license_id.as_deref(),
            Some("license_renewal_contract")
        );
        assert_eq!(updated.last_transfer_sequence, 0);

        assert_eq!(
            store.validate_replacement(&transferred, datetime!(2026-08-28 0:02 UTC)),
            Err(LicenseStateError::LicenseRollback)
        );
        let older_new_identity =
            license_with_id(10, "2026-08-27T00:00:00.500Z", "license_older_contract");
        assert_eq!(
            store.validate_replacement(&older_new_identity, datetime!(2026-08-28 0:02 UTC)),
            Err(LicenseStateError::LicenseRollback)
        );
    }

    #[test]
    fn a_higher_transfer_sequence_cannot_move_issue_time_back_for_identity_replay() {
        let directory = tempdir().expect("temp directory");
        let store = LicenseStateStore::new(directory.path().join("state.json"), &[7_u8; 32])
            .expect("create store");
        let current = license_with_id(7, "2026-08-27T10:00:00.000Z", "license_current_contract");
        store
            .initialize(&current, datetime!(2026-08-28 0:00 UTC))
            .expect("initialize current identity");

        let time_rollback =
            license_with_id(8, "2026-08-27T08:00:00.000Z", "license_current_contract");
        assert_eq!(
            store.accept_replacement(&time_rollback, datetime!(2026-08-28 0:01 UTC)),
            Err(LicenseStateError::LicenseRollback)
        );
        let unchanged = store.load().expect("load unchanged history");
        assert_eq!(unchanged.last_issued_at, "2026-08-27T10:00:00.000Z");
        assert_eq!(unchanged.last_transfer_sequence, 7);

        let older_identity =
            license_with_id(0, "2026-08-27T09:00:00.000Z", "license_previous_contract");
        assert_eq!(
            store.accept_replacement(&older_identity, datetime!(2026-08-28 0:02 UTC)),
            Err(LicenseStateError::LicenseRollback)
        );
    }

    #[test]
    fn rejects_authenticated_legacy_history_without_rewriting_it() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("state.json");
        let store = LicenseStateStore::new(&path, &[7_u8; 32]).expect("create store");
        let mut legacy = serde_json::json!({
            "schema": "aster.license-state.v1",
            "last_issued_at": "2026-08-26T00:00:00.000Z",
            "last_transfer_sequence": 0,
            "last_seen_at": "2026-08-27T00:00:00.000Z",
            "seat_over_limit_first_observed_at": null,
        });
        let mac = store.calculate_mac_value(&legacy).unwrap();
        legacy["mac"] = serde_json::json!(mac);
        let encoded = serde_json::to_vec_pretty(&legacy).unwrap();
        fs::write(&path, &encoded).unwrap();
        let current = license(0, "2026-08-26T00:00:00.000Z");
        assert_eq!(
            store.check_and_observe(&current, datetime!(2026-08-27 0:00:30 UTC)),
            Err(LicenseStateError::InvalidJson)
        );
        assert_eq!(fs::read(&path).unwrap(), encoded);
        assert_eq!(
            store.initialize(&current, datetime!(2026-08-27 0:00:30 UTC)),
            Err(LicenseStateError::AlreadyInitialized)
        );
        assert_eq!(fs::read(&path).unwrap(), encoded);
    }

    #[test]
    fn a_different_installation_key_cannot_verify_state() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("state.json");
        let license = license(0, "2026-08-26T00:00:00.000Z");
        LicenseStateStore::new(&path, &[7_u8; 32])
            .expect("create first store")
            .initialize(&license, datetime!(2026-08-27 0:00 UTC))
            .expect("initialize state");
        let other = LicenseStateStore::new(&path, &[8_u8; 32]).expect("create second store");
        assert_eq!(
            other.check_and_observe(&license, datetime!(2026-08-27 0:01 UTC)),
            Err(LicenseStateError::Integrity)
        );
    }

    #[test]
    fn persists_and_clears_the_first_seat_overage_observation() {
        let directory = tempdir().expect("temp directory");
        let store = LicenseStateStore::new(directory.path().join("state.json"), &[9_u8; 32])
            .expect("create store");
        let license = license(0, "2026-08-26T00:00:00.000Z");
        store
            .initialize(&license, datetime!(2026-08-27 0:00 UTC))
            .expect("initialize state");
        assert_eq!(
            store
                .check_observe_and_track_seats(&license, 11, datetime!(2026-08-28 0:00 UTC))
                .expect("observe overage"),
            Some(datetime!(2026-08-28 0:00 UTC))
        );
        assert_eq!(
            store
                .check_observe_and_track_seats(&license, 12, datetime!(2026-08-29 0:00 UTC))
                .expect("retain overage"),
            Some(datetime!(2026-08-28 0:00 UTC))
        );
        assert_eq!(
            store
                .check_observe_and_track_seats(&license, 10, datetime!(2026-08-29 0:01 UTC))
                .expect("clear overage"),
            None
        );
    }
}
