#![forbid(unsafe_code)]

pub mod billing;

use aster_error_catalog::{ErrorDescriptor, license as license_errors, member as member_errors};
use aster_license_core::{VerifiedLicenseRef, catalog::QuotaId};
use semver::Version;
use thiserror::Error;
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdentityAccess {
    pub active: bool,
    pub can_consume_model: bool,
}

impl IdentityAccess {
    pub const fn occupies_seat(self) -> bool {
        self.active && self.can_consume_model
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SeatSnapshot {
    pub occupied: u32,
    pub over_limit_first_observed_at: Option<OffsetDateTime>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SeatStatus {
    WithinLimit,
    Grace { expires_at: OffsetDateTime },
    Blocked,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum PolicyError {
    #[error("member seat limit reached")]
    SeatLimitReached,
    #[error("member seat overage grace period ended")]
    SeatOverageExpired,
    #[error("feature is not licensed")]
    FeatureNotLicensed,
    #[error("license is not active yet")]
    NotBefore,
    #[error("license is expired")]
    Expired,
    #[error("current version is below the licensed minimum")]
    VersionTooOld,
    #[error("license contains an invalid semantic version")]
    InvalidMinimumVersion,
    #[error("license contains an invalid timestamp")]
    InvalidLicenseTime,
}

impl PolicyError {
    pub const fn descriptor(&self) -> ErrorDescriptor {
        match self {
            Self::SeatLimitReached => member_errors::SEAT_LIMIT_REACHED,
            Self::SeatOverageExpired => license_errors::SEAT_OVERAGE_EXPIRED,
            Self::FeatureNotLicensed => license_errors::FEATURE_NOT_LICENSED,
            Self::NotBefore | Self::InvalidLicenseTime => license_errors::TIME_INVALID,
            Self::Expired => license_errors::EXPIRED,
            Self::VersionTooOld | Self::InvalidMinimumVersion => license_errors::VERSION_INVALID,
        }
    }
}

pub fn authorize_new_seat<'a>(
    license: impl Into<VerifiedLicenseRef<'a>>,
    before: IdentityAccess,
    after: IdentityAccess,
    currently_occupied: u32,
) -> Result<(), PolicyError> {
    let license = license.into();
    let adds_seat = !before.occupies_seat() && after.occupies_seat();
    let occupied_after = currently_occupied.checked_add(u32::from(adds_seat));
    if adds_seat
        && occupied_after
            .is_none_or(|occupied| !license.quota(QuotaId::MemberSeats).permits(occupied))
    {
        return Err(PolicyError::SeatLimitReached);
    }
    Ok(())
}

pub fn evaluate_existing_seats<'a>(
    license: impl Into<VerifiedLicenseRef<'a>>,
    snapshot: SeatSnapshot,
    now: OffsetDateTime,
) -> SeatStatus {
    let license = license.into();
    if license
        .quota(QuotaId::MemberSeats)
        .permits(snapshot.occupied)
    {
        return SeatStatus::WithinLimit;
    }
    let Some(first_observed) = snapshot.over_limit_first_observed_at else {
        return SeatStatus::Grace {
            expires_at: now + Duration::days(i64::from(license.seat_over_limit_grace_days())),
        };
    };
    let expires_at =
        first_observed + Duration::days(i64::from(license.seat_over_limit_grace_days()));
    if now < expires_at {
        SeatStatus::Grace { expires_at }
    } else {
        SeatStatus::Blocked
    }
}

pub fn authorize_request<'a>(
    license: impl Into<VerifiedLicenseRef<'a>>,
    required_feature: &str,
    current_version: &str,
    now: OffsetDateTime,
    seats: SeatSnapshot,
) -> Result<(), PolicyError> {
    let license = license.into();
    authorize_feature(license, required_feature, current_version, now)?;
    if evaluate_existing_seats(license, seats, now) == SeatStatus::Blocked {
        return Err(PolicyError::SeatOverageExpired);
    }
    Ok(())
}

/// Checks current time and version only. Success is not a business permission;
/// signature/scope verification precedes this check, and installation history,
/// binding, identities, capabilities and quotas remain separate requirements.
pub fn check_license_current<'a>(
    license: impl Into<VerifiedLicenseRef<'a>>,
    current_version: &str,
    now: OffsetDateTime,
) -> Result<(), PolicyError> {
    let license = license.into();
    check_license_retained(license, current_version, now)?;
    if let Some(expires_at) = license.expires_at()
        && now >= parse_license_time(expires_at)?
    {
        return Err(PolicyError::Expired);
    }
    Ok(())
}

/// Only for explicitly retained data access and recovery operations. Expiry
/// does not revoke existing data; future activation and incompatible versions
/// still fail closed. Signature, binding and history remain caller checks.
pub fn check_license_retained<'a>(
    license: impl Into<VerifiedLicenseRef<'a>>,
    current_version: &str,
    now: OffsetDateTime,
) -> Result<(), PolicyError> {
    let license = license.into();
    let not_before = parse_license_time(license.not_before())?;
    if now < not_before {
        return Err(PolicyError::NotBefore);
    }
    let minimum = Version::parse(license.minimum_version())
        .map_err(|_| PolicyError::InvalidMinimumVersion)?;
    let current = Version::parse(current_version).map_err(|_| PolicyError::VersionTooOld)?;
    if current.cmp_precedence(&minimum).is_lt() {
        return Err(PolicyError::VersionTooOld);
    }
    Ok(())
}

pub fn authorize_feature<'a>(
    license: impl Into<VerifiedLicenseRef<'a>>,
    required_feature: &str,
    current_version: &str,
    now: OffsetDateTime,
) -> Result<(), PolicyError> {
    let license = license.into();
    check_license_current(license, current_version, now)?;
    if !license.has_feature(required_feature) {
        return Err(PolicyError::FeatureNotLicensed);
    }
    Ok(())
}

/// This does not authorize new resources, new tasks or model consumption.
pub fn authorize_retained_feature<'a>(
    license: impl Into<VerifiedLicenseRef<'a>>,
    required_feature: &str,
    current_version: &str,
    now: OffsetDateTime,
) -> Result<(), PolicyError> {
    let license = license.into();
    check_license_retained(license, current_version, now)?;
    if !license.has_feature(required_feature) {
        return Err(PolicyError::FeatureNotLicensed);
    }
    Ok(())
}

fn parse_license_time(value: &str) -> Result<OffsetDateTime, PolicyError> {
    OffsetDateTime::parse(value, &Rfc3339).map_err(|_| PolicyError::InvalidLicenseTime)
}

#[cfg(test)]
mod tests {
    use aster_license_core::{
        LICENSE_SCHEMA, LicenseClaims, LicenseLimits, TrustedLicenseKeys, VerifiedLicense, sign,
        verify,
    };
    use ed25519_dalek::SigningKey;
    use time::macros::datetime;

    use super::*;

    fn license(member_seats: u32, grace_days: u16) -> VerifiedLicense {
        license_with_features(member_seats, grace_days, &["member"])
    }

    fn license_with_features(
        member_seats: u32,
        grace_days: u16,
        features: &[&str],
    ) -> VerifiedLicense {
        let signing_key = SigningKey::from_bytes(&[21_u8; 32]);
        let claims = LicenseClaims {
            schema: LICENSE_SCHEMA.to_owned(),
            key_id: "license-test-01".to_owned(),
            license_id: "license_test_001".to_owned(),
            serial: "AT-TEST-001".to_owned(),
            request_id: "request_test_001".to_owned(),
            customer_ref: "customer_test_001".to_owned(),
            product: "aster-team".to_owned(),
            edition: "enterprise".to_owned(),
            features: features
                .iter()
                .map(|feature| (*feature).to_owned())
                .collect(),
            limits: LicenseLimits {
                member_seats,
                seat_over_limit_grace_days: grace_days,
            },
            minimum_version: "1.0.0".to_owned(),
            installation_id: "installation_test_001".to_owned(),
            machine_fingerprint_sha256: "A".repeat(43),
            transfer_sequence: 0,
            issued_at: "2026-08-26T00:00:00.000Z".to_owned(),
            not_before: "2026-08-26T00:00:00.000Z".to_owned(),
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
    }

    #[test]
    fn currentness_is_independent_of_member_but_business_permission_is_not() {
        let license = license_with_features(1, 0, &["runner"]);
        let now = datetime!(2026-08-27 0:00 UTC);
        assert_eq!(check_license_current(&license, "1.0.0", now), Ok(()));
        assert_eq!(authorize_feature(&license, "runner", "1.0.0", now), Ok(()));
        assert_eq!(
            authorize_feature(&license, "member", "1.0.0", now),
            Err(PolicyError::FeatureNotLicensed)
        );
        assert_eq!(
            check_license_current(&license, "1.0.0", datetime!(2026-08-25 23:59:59 UTC)),
            Err(PolicyError::NotBefore)
        );
        assert_eq!(
            check_license_current(&license, "1.0.0", datetime!(2026-08-26 0:00 UTC)),
            Ok(())
        );
        assert_eq!(
            check_license_current(&license, "1.0.0", datetime!(2027-08-26 0:00 UTC)),
            Err(PolicyError::Expired)
        );
        assert_eq!(
            check_license_current(&license, "0.9.9", now),
            Err(PolicyError::VersionTooOld)
        );
        assert_eq!(
            check_license_current(&license, "invalid", now),
            Err(PolicyError::VersionTooOld)
        );
    }

    #[test]
    fn verified_v2_uses_explicit_time_modes_and_feature_permissions() {
        use aster_license_core::{catalog::CapabilityId, v2};
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
        // Fixed and no-expiry are protocol fixtures, not a choice of free-plan defaults.
        for index in [0, 1] {
            let mut document: v2::Document =
                serde_json::from_value(fixture["cases"][index]["document"].clone()).unwrap();
            document.claims.entitlements.features = vec![CapabilityId::Runner];
            let document = v2::sign(document.claims, &SigningKey::from_bytes(&[42; 32])).unwrap();
            let license = v2::verify(&serde_json::to_vec(&document).unwrap(), &keys).unwrap();
            let now = datetime!(2026-09-01 0:00 UTC);
            assert_eq!(check_license_current(&license, "1.2.3", now), Ok(()));
            assert_eq!(
                check_license_current(&license, "1.2.2", now),
                Err(PolicyError::VersionTooOld)
            );
            assert_eq!(
                check_license_current(&license, "1.2.3", now - Duration::seconds(1)),
                Err(PolicyError::NotBefore)
            );
            assert_eq!(authorize_feature(&license, "runner", "1.2.3", now), Ok(()));
            assert_eq!(
                authorize_feature(&license, "member", "1.2.3", now),
                Err(PolicyError::FeatureNotLicensed)
            );
            assert_eq!(
                check_license_current(&license, "1.2.3", datetime!(2027-09-01 0:00 UTC)),
                if index == 0 {
                    Err(PolicyError::Expired)
                } else {
                    Ok(())
                }
            );
        }
    }

    #[test]
    fn symbolic_subscription_still_enforces_time_and_seats() {
        use aster_license_core::v2;
        let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../../contracts/test-vectors/license.v2.json"
        ))
        .unwrap();
        let document: v2::Document = serde_json::from_value(
            fixture["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|case| case["name"] == "commercial_standard_feature_set")
                .unwrap()["document"]
                .clone(),
        )
        .unwrap();
        let mut keys = TrustedLicenseKeys::new();
        keys.insert_scoped_spki_base64url(
            &document.claims.key_id,
            fixture["public_key_spki"].as_str().unwrap(),
            v2::IssuerPolicy {
                sources: vec![v2::SourceKind::CommercialOrder],
                bindings: vec![v2::BindingKind::Installation],
                expiries: vec![v2::ExpiryKind::Fixed],
                entitlement_ceiling: document.claims.entitlements.clone(),
            },
        )
        .unwrap();
        let license = v2::verify(&serde_json::to_vec(&document).unwrap(), &keys).unwrap();
        let begins = parse_license_time(&document.claims.validity.not_before).unwrap();
        let v2::Expiry::Fixed { ref expires_at } = document.claims.validity.expiry else {
            panic!("fixed subscription")
        };
        let ends = parse_license_time(expires_at).unwrap();
        assert_eq!(
            authorize_feature(&license, "member", "99.0.0", begins),
            Ok(())
        );
        assert_eq!(
            authorize_feature(&license, "member", "99.0.0", begins - Duration::seconds(1)),
            Err(PolicyError::NotBefore)
        );
        assert_eq!(
            authorize_feature(&license, "member", "99.0.0", ends),
            Err(PolicyError::Expired)
        );
        let aster_license_core::entitlements::QuotaLimit::Limited { value: seats } =
            VerifiedLicenseRef::from(&license).quota(QuotaId::MemberSeats)
        else {
            panic!("finite seats")
        };
        assert_eq!(
            authorize_new_seat(
                &license,
                IdentityAccess {
                    active: false,
                    can_consume_model: true
                },
                IdentityAccess {
                    active: true,
                    can_consume_model: true
                },
                seats
            ),
            Err(PolicyError::SeatLimitReached)
        );
    }

    #[test]
    fn verified_v1_and_v2_use_semver_precedence_without_build_metadata() {
        use aster_license_core::v2;
        let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../../contracts/test-vectors/license.v2.json"
        ))
        .unwrap();
        let trust: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../../contracts/test-vectors/license-trust.v1.json"
        ))
        .unwrap();
        let v2_keys =
            TrustedLicenseKeys::from_json(&serde_json::to_vec(&trust["keyring"]).unwrap()).unwrap();
        let v1_key = SigningKey::from_bytes(&[21; 32]);
        let mut v1_keys = TrustedLicenseKeys::new();
        v1_keys
            .insert("license-test-01", v1_key.verifying_key())
            .unwrap();
        for (minimum, current, expected) in [
            ("1.2.3+z", "1.2.3+a", true),
            ("1.2.3+a", "1.2.3+z", true),
            ("1.2.3+z", "1.2.3", true),
            ("1.2.3", "1.2.3-rc.1+z", false),
            ("1.2.3-rc.1", "1.2.3", true),
            ("1.2.3-rc.10", "1.2.3-rc.9", false),
            ("1.2.3", "invalid", false),
        ] {
            let mut claims = license(1, 0).claims().clone();
            claims.minimum_version = minimum.to_owned();
            let doc = sign(claims, &v1_key).unwrap();
            let v1 = verify(&serde_json::to_vec(&doc).unwrap(), &v1_keys).unwrap();
            let mut doc: v2::Document =
                serde_json::from_value(fixture["cases"][0]["document"].clone()).unwrap();
            doc.claims.minimum_version = minimum.to_owned();
            let doc = v2::sign(doc.claims, &SigningKey::from_bytes(&[42; 32])).unwrap();
            let v2 = v2::verify(&serde_json::to_vec(&doc).unwrap(), &v2_keys).unwrap();
            let result = if expected {
                Ok(())
            } else {
                Err(PolicyError::VersionTooOld)
            };
            for verified in [VerifiedLicenseRef::from(&v1), VerifiedLicenseRef::from(&v2)] {
                assert_eq!(
                    check_license_current(verified, current, datetime!(2026-09-06 0:00 UTC)),
                    result,
                    "{current} >= {minimum}"
                );
            }
        }
    }

    #[test]
    fn retained_access_ignores_only_expiry_and_never_authorizes_execution() {
        let license = license(1, 0);
        let expired = datetime!(2027-08-26 0:00 UTC);
        assert_eq!(
            authorize_retained_feature(&license, "member", "1.0.0", expired),
            Ok(())
        );
        assert_eq!(
            authorize_feature(&license, "member", "1.0.0", expired),
            Err(PolicyError::Expired)
        );
        assert_eq!(
            authorize_retained_feature(&license, "runner", "1.0.0", expired),
            Err(PolicyError::FeatureNotLicensed)
        );
        assert_eq!(
            authorize_retained_feature(&license, "member", "0.9.0", expired),
            Err(PolicyError::VersionTooOld)
        );
        assert_eq!(
            authorize_retained_feature(&license, "member", "1.0.0", datetime!(2026-08-25 0:00 UTC)),
            Err(PolicyError::NotBefore)
        );
    }

    #[test]
    fn model_capability_occupies_a_seat_regardless_of_role() {
        let license = license(1, 0);
        let management_only = IdentityAccess {
            active: true,
            can_consume_model: false,
        };
        let model_enabled_admin = IdentityAccess {
            active: true,
            can_consume_model: true,
        };
        assert_eq!(
            authorize_new_seat(&license, management_only, model_enabled_admin, 1),
            Err(PolicyError::SeatLimitReached)
        );
    }

    #[test]
    fn unlimited_runner_count_does_not_exist_in_seat_policy() {
        let license = license(2, 0);
        assert!(
            authorize_new_seat(
                &license,
                IdentityAccess {
                    active: false,
                    can_consume_model: false
                },
                IdentityAccess {
                    active: true,
                    can_consume_model: true
                },
                1,
            )
            .is_ok()
        );
    }

    #[test]
    fn existing_overage_blocks_after_grace_period() {
        let license = license(1, 7);
        let first_observed = datetime!(2026-08-01 0:00 UTC);
        assert_eq!(
            evaluate_existing_seats(
                &license,
                SeatSnapshot {
                    occupied: 2,
                    over_limit_first_observed_at: Some(first_observed),
                },
                datetime!(2026-08-08 0:00 UTC),
            ),
            SeatStatus::Blocked
        );
    }

    #[test]
    fn request_checks_time_feature_version_and_seats() {
        let license = license(1, 0);
        assert_eq!(
            authorize_request(
                &license,
                "member",
                "1.0.0",
                datetime!(2026-08-27 0:00 UTC),
                SeatSnapshot {
                    occupied: 2,
                    over_limit_first_observed_at: Some(datetime!(2026-08-26 0:00 UTC)),
                },
            ),
            Err(PolicyError::SeatOverageExpired)
        );
    }

    #[test]
    fn non_consuming_feature_check_does_not_apply_member_seat_state() {
        let license = license(1, 0);
        assert!(
            authorize_feature(&license, "member", "1.0.0", datetime!(2026-08-27 0:00 UTC),).is_ok()
        );
    }
}
