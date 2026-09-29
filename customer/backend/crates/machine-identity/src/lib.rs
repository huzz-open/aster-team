#![forbid(unsafe_code)]

use std::path::Path;

#[cfg(any(target_os = "windows", target_os = "macos"))]
use std::process::Command;

use aster_license_core::{
    LicenseClaims, MachineFactor, MachineFactorKind, VerifiedBinding, VerifiedLicenseRef,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

pub const INSTALLATION_SCHEMA: &str = "aster.installation.v1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationProfile {
    pub schema: String,
    pub installation_id: String,
    pub machine_fingerprint_sha256: String,
    pub machine_factors: Vec<MachineFactor>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawMachineFactors {
    pub dmi_product_uuid: String,
    pub machine_id: String,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MachineIdentityError {
    #[error("installation profile could not be read")]
    Io,
    #[error("installation profile JSON is invalid: {0}")]
    InvalidJson(String),
    #[error("installation profile contains trailing JSON")]
    TrailingJson,
    #[error("installation profile is invalid")]
    InvalidProfile,
    #[error("machine factor is unavailable")]
    FactorUnavailable,
    #[error("installation profile does not match this machine")]
    MachineMismatch,
    #[error("license does not match the installation profile")]
    LicenseMismatch,
    #[error("machine identity is unavailable on this operating system")]
    UnsupportedPlatform,
}

pub fn create_profile(
    installation_id: impl Into<String>,
    raw: &RawMachineFactors,
) -> Result<InstallationProfile, MachineIdentityError> {
    let installation_id = installation_id.into();
    validate_identifier(&installation_id)?;
    let machine_factors = normalized_factor_hashes(raw)?;
    let machine_fingerprint_sha256 = fingerprint(&installation_id, &machine_factors);
    Ok(InstallationProfile {
        schema: INSTALLATION_SCHEMA.to_owned(),
        installation_id,
        machine_fingerprint_sha256,
        machine_factors,
    })
}

pub fn parse_and_verify_profile(
    data: &[u8],
    current: &RawMachineFactors,
) -> Result<InstallationProfile, MachineIdentityError> {
    let profile = parse_profile(data)?;
    let current_factors = normalized_factor_hashes(current)?;
    if !same_factors(&profile.machine_factors, &current_factors)
        || profile.machine_fingerprint_sha256
            != fingerprint(&profile.installation_id, &current_factors)
    {
        return Err(MachineIdentityError::MachineMismatch);
    }
    Ok(profile)
}

pub fn parse_profile(data: &[u8]) -> Result<InstallationProfile, MachineIdentityError> {
    let mut deserializer = serde_json::Deserializer::from_slice(data);
    let profile = InstallationProfile::deserialize(&mut deserializer)
        .map_err(|error| MachineIdentityError::InvalidJson(error.to_string()))?;
    deserializer
        .end()
        .map_err(|_| MachineIdentityError::TrailingJson)?;
    validate_profile(&profile)?;
    Ok(profile)
}

pub fn verify_license_binding(
    profile: &InstallationProfile,
    license: &LicenseClaims,
) -> Result<(), MachineIdentityError> {
    if license.installation_id != profile.installation_id
        || license.machine_fingerprint_sha256 != profile.machine_fingerprint_sha256
    {
        return Err(MachineIdentityError::LicenseMismatch);
    }
    Ok(())
}

/// Only an explicitly unbound, verified document skips License-to-installation
/// matching. The local installation profile must still be structurally valid;
/// entry points separately verify it against this machine before using its keys.
pub fn verify_authorization_binding<'a>(
    profile: &InstallationProfile,
    license: impl Into<VerifiedLicenseRef<'a>>,
) -> Result<(), MachineIdentityError> {
    validate_profile(profile)?;
    match license.into().binding() {
        VerifiedBinding::Unbound => Ok(()),
        VerifiedBinding::Installation {
            installation_id,
            machine_fingerprint_sha256,
            ..
        } if installation_id == profile.installation_id
            && machine_fingerprint_sha256 == profile.machine_fingerprint_sha256 =>
        {
            Ok(())
        }
        _ => Err(MachineIdentityError::LicenseMismatch),
    }
}

pub fn load_and_verify_current_profile(
    profile_path: &Path,
) -> Result<InstallationProfile, MachineIdentityError> {
    let profile = std::fs::read(profile_path).map_err(|_| MachineIdentityError::Io)?;
    parse_and_verify_profile(&profile, &current_machine_factors()?)
}

pub fn current_machine_factors() -> Result<RawMachineFactors, MachineIdentityError> {
    #[cfg(target_os = "linux")]
    {
        Ok(RawMachineFactors {
            dmi_product_uuid: std::fs::read_to_string("/sys/class/dmi/id/product_uuid")
                .map_err(|_| MachineIdentityError::FactorUnavailable)?,
            machine_id: std::fs::read_to_string("/etc/machine-id")
                .map_err(|_| MachineIdentityError::FactorUnavailable)?,
        })
    }
    #[cfg(target_os = "windows")]
    {
        current_windows_machine_factors()
    }
    #[cfg(target_os = "macos")]
    {
        current_macos_machine_factors()
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        Err(MachineIdentityError::UnsupportedPlatform)
    }
}

#[cfg(target_os = "windows")]
fn current_windows_machine_factors() -> Result<RawMachineFactors, MachineIdentityError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WindowsFactors {
        system_uuid: String,
        machine_id: String,
    }

    let script = concat!(
        "$ErrorActionPreference='Stop';",
        "$f=[ordered]@{",
        "system_uuid=[string](Get-CimInstance -ClassName Win32_ComputerSystemProduct -ErrorAction Stop).UUID;",
        "machine_id=[string](Get-ItemPropertyValue -LiteralPath 'Registry::HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Cryptography' -Name MachineGuid -ErrorAction Stop)",
        "};",
        "$f|ConvertTo-Json -Compress"
    );
    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .output()
        .map_err(|_| MachineIdentityError::FactorUnavailable)?;
    if !output.status.success() {
        return Err(MachineIdentityError::FactorUnavailable);
    }
    let factors: WindowsFactors = serde_json::from_slice(&output.stdout)
        .map_err(|_| MachineIdentityError::FactorUnavailable)?;
    Ok(RawMachineFactors {
        dmi_product_uuid: factors.system_uuid,
        machine_id: factors.machine_id,
    })
}

#[cfg(target_os = "macos")]
fn current_macos_machine_factors() -> Result<RawMachineFactors, MachineIdentityError> {
    let output = Command::new("/usr/sbin/ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .map_err(|_| MachineIdentityError::FactorUnavailable)?;
    if !output.status.success() {
        return Err(MachineIdentityError::FactorUnavailable);
    }
    let output =
        String::from_utf8(output.stdout).map_err(|_| MachineIdentityError::FactorUnavailable)?;
    Ok(RawMachineFactors {
        dmi_product_uuid: ioreg_value(&output, "IOPlatformUUID")?,
        machine_id: ioreg_value(&output, "IOPlatformSerialNumber")?,
    })
}

#[cfg(target_os = "macos")]
fn ioreg_value(output: &str, key: &str) -> Result<String, MachineIdentityError> {
    let prefix = format!("\"{key}\" = \"");
    output
        .lines()
        .map(str::trim)
        .find_map(|line| {
            line.strip_prefix(&prefix)
                .and_then(|value| value.strip_suffix('"'))
                .map(str::to_owned)
        })
        .ok_or(MachineIdentityError::FactorUnavailable)
}

fn validate_profile(profile: &InstallationProfile) -> Result<(), MachineIdentityError> {
    if profile.schema != INSTALLATION_SCHEMA
        || validate_identifier(&profile.installation_id).is_err()
        || !valid_digest(&profile.machine_fingerprint_sha256)
        || profile.machine_factors.len() != 2
    {
        return Err(MachineIdentityError::InvalidProfile);
    }
    let mut dmi = false;
    let mut machine_id = false;
    for factor in &profile.machine_factors {
        if !valid_digest(&factor.sha256) {
            return Err(MachineIdentityError::InvalidProfile);
        }
        match factor.kind {
            MachineFactorKind::DmiProductUuid if dmi => {
                return Err(MachineIdentityError::InvalidProfile);
            }
            MachineFactorKind::DmiProductUuid => dmi = true,
            MachineFactorKind::MachineId if machine_id => {
                return Err(MachineIdentityError::InvalidProfile);
            }
            MachineFactorKind::MachineId => machine_id = true,
        }
    }
    if !dmi
        || !machine_id
        || profile.machine_fingerprint_sha256
            != fingerprint(&profile.installation_id, &profile.machine_factors)
    {
        return Err(MachineIdentityError::InvalidProfile);
    }
    Ok(())
}

fn normalized_factor_hashes(
    raw: &RawMachineFactors,
) -> Result<Vec<MachineFactor>, MachineIdentityError> {
    Ok(vec![
        MachineFactor {
            kind: MachineFactorKind::DmiProductUuid,
            sha256: sha256(&normalize(&raw.dmi_product_uuid)?),
        },
        MachineFactor {
            kind: MachineFactorKind::MachineId,
            sha256: sha256(&normalize(&raw.machine_id)?),
        },
    ])
}

fn normalize(value: &str) -> Result<String, MachineIdentityError> {
    let normalized = value.trim().to_lowercase();
    if normalized.is_empty() {
        return Err(MachineIdentityError::FactorUnavailable);
    }
    Ok(normalized)
}

fn same_factors(left: &[MachineFactor], right: &[MachineFactor]) -> bool {
    left.len() == right.len()
        && left.iter().all(|factor| {
            right
                .iter()
                .any(|candidate| candidate.kind == factor.kind && candidate.sha256 == factor.sha256)
        })
}

fn fingerprint(installation_id: &str, factors: &[MachineFactor]) -> String {
    let mut lines: Vec<_> = factors
        .iter()
        .map(|factor| {
            let kind = match factor.kind {
                MachineFactorKind::DmiProductUuid => "dmi_product_uuid",
                MachineFactorKind::MachineId => "machine_id",
            };
            format!("{kind}={}", factor.sha256)
        })
        .collect();
    lines.sort();
    sha256(&format!(
        "aster-team\n{installation_id}\n{}\n",
        lines.join("\n")
    ))
}

fn sha256(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(value.as_bytes()))
}

fn validate_identifier(value: &str) -> Result<(), MachineIdentityError> {
    if value.len() < 8
        || value.len() > 128
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'.' | b'_' | b':' | b'@' | b'+' | b'/' | b'-')
        })
    {
        return Err(MachineIdentityError::InvalidProfile);
    }
    Ok(())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use aster_license_core::{LICENSE_SCHEMA, LicenseClaims, LicenseLimits};

    use super::*;

    fn raw() -> RawMachineFactors {
        RawMachineFactors {
            dmi_product_uuid: " ABCD-EF01 \n".to_owned(),
            machine_id: "Machine-ID-01\n".to_owned(),
        }
    }

    #[test]
    fn verified_unbound_license_still_requires_a_valid_local_profile() {
        use aster_license_core::{TrustedLicenseKeys, v2};
        let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../../contracts/test-vectors/license.v2.json"
        ))
        .unwrap();
        let trust: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../../contracts/test-vectors/license-trust.v1.json"
        ))
        .unwrap();
        let keys =
            TrustedLicenseKeys::from_json(&serde_json::to_vec(&trust["keyring"]).unwrap()).unwrap();
        let license = v2::verify(
            &serde_json::to_vec(&fixture["cases"][0]["document"]).unwrap(),
            &keys,
        )
        .unwrap();
        let first = create_profile("installation_free_first", &raw()).unwrap();
        let second = create_profile(
            "installation_free_second",
            &RawMachineFactors {
                machine_id: "another-machine".to_owned(),
                ..raw()
            },
        )
        .unwrap();
        assert_eq!(verify_authorization_binding(&first, &license), Ok(()));
        assert_eq!(verify_authorization_binding(&second, &license), Ok(()));
        let mut invalid = first.clone();
        invalid.machine_fingerprint_sha256 = second.machine_fingerprint_sha256;
        assert_eq!(
            verify_authorization_binding(&invalid, &license),
            Err(MachineIdentityError::InvalidProfile)
        );
        // Free matching does not make a copied local profile valid on another host.
        assert_eq!(
            parse_and_verify_profile(
                &serde_json::to_vec(&first).unwrap(),
                &RawMachineFactors {
                    machine_id: "another-machine".to_owned(),
                    ..raw()
                }
            ),
            Err(MachineIdentityError::MachineMismatch)
        );
    }

    #[test]
    fn verified_paid_v2_binding_matches_both_installation_and_machine() {
        use aster_license_core::{TrustedLicenseKeys, v2};
        let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../../contracts/test-vectors/license.v2.json"
        ))
        .unwrap();
        let mut document: v2::Document =
            serde_json::from_value(fixture["cases"][2]["document"].clone()).unwrap();
        let first = create_profile("installation_paid", &raw()).unwrap();
        document.claims.binding = v2::Binding::Installation {
            installation_id: first.installation_id.clone(),
            machine_fingerprint_sha256: first.machine_fingerprint_sha256.clone(),
            transfer_sequence: 2,
        };
        let policy = v2::IssuerPolicy {
            sources: vec![v2::SourceKind::CommercialOrder],
            bindings: vec![v2::BindingKind::Installation],
            expiries: vec![v2::ExpiryKind::Fixed],
            entitlement_ceiling: document.claims.entitlements.clone(),
        };
        let mut keys = TrustedLicenseKeys::new();
        keys.insert_scoped_spki_base64url(
            "test-only-v2",
            fixture["public_key_spki"].as_str().unwrap(),
            policy,
        )
        .unwrap();
        let document = v2::sign(
            document.claims,
            &ed25519_dalek::SigningKey::from_bytes(&[42; 32]),
        )
        .unwrap();
        let license = v2::verify(&serde_json::to_vec(&document).unwrap(), &keys).unwrap();
        assert_eq!(verify_authorization_binding(&first, &license), Ok(()));
        let another_installation = create_profile("installation_other", &raw()).unwrap();
        let another_machine = create_profile(
            "installation_paid",
            &RawMachineFactors {
                machine_id: "another-machine".to_owned(),
                ..raw()
            },
        )
        .unwrap();
        for other in [another_installation, another_machine] {
            assert_eq!(
                verify_authorization_binding(&other, &license),
                Err(MachineIdentityError::LicenseMismatch)
            );
        }
    }

    #[test]
    fn creates_and_verifies_a_stable_profile() {
        let profile = create_profile("installation_test_001", &raw()).expect("create profile");
        let encoded = serde_json::to_vec(&profile).expect("serialize profile");
        assert_eq!(parse_profile(&encoded).expect("parse profile"), profile);
        assert_eq!(
            parse_and_verify_profile(&encoded, &raw()).expect("verify profile"),
            profile
        );
    }

    #[test]
    fn rejects_profile_copied_to_another_machine() {
        let profile = create_profile("installation_test_001", &raw()).expect("create profile");
        let encoded = serde_json::to_vec(&profile).expect("serialize profile");
        let other = RawMachineFactors {
            dmi_product_uuid: "different-machine".to_owned(),
            ..raw()
        };
        assert_eq!(
            parse_and_verify_profile(&encoded, &other),
            Err(MachineIdentityError::MachineMismatch)
        );
    }

    #[test]
    fn rejects_unknown_and_duplicate_profile_fields() {
        let profile = create_profile("installation_test_001", &raw()).expect("create profile");
        let encoded = serde_json::to_string(&profile).expect("serialize profile");
        let unknown = encoded.replacen("{", "{\"unexpected\":true,", 1);
        assert!(matches!(
            parse_and_verify_profile(unknown.as_bytes(), &raw()),
            Err(MachineIdentityError::InvalidJson(_))
        ));
        let duplicate = encoded.replacen(
            "\"schema\":\"aster.installation.v1\"",
            "\"schema\":\"aster.installation.v1\",\"schema\":\"aster.installation.v1\"",
            1,
        );
        assert!(matches!(
            parse_and_verify_profile(duplicate.as_bytes(), &raw()),
            Err(MachineIdentityError::InvalidJson(_))
        ));
    }

    #[test]
    fn license_must_match_both_installation_id_and_fingerprint() {
        let profile = create_profile("installation_test_001", &raw()).expect("create profile");
        let mut license = LicenseClaims {
            schema: LICENSE_SCHEMA.to_owned(),
            key_id: "license-test-01".to_owned(),
            license_id: "license_test_001".to_owned(),
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
            installation_id: profile.installation_id.clone(),
            machine_fingerprint_sha256: profile.machine_fingerprint_sha256.clone(),
            transfer_sequence: 0,
            issued_at: "2026-08-26T00:00:00.000Z".to_owned(),
            not_before: "2026-08-26T00:00:00.000Z".to_owned(),
            expires_at: "2027-08-26T00:00:00.000Z".to_owned(),
        };
        assert!(verify_license_binding(&profile, &license).is_ok());
        license.installation_id = "installation_other_001".to_owned();
        assert_eq!(
            verify_license_binding(&profile, &license),
            Err(MachineIdentityError::LicenseMismatch)
        );
    }
}
