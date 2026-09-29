//! Read-only view of one completely verified document. Callers cannot create
//! this view from unsigned claims or combine fields from different licenses.

use crate::{
    VerifiedLicense, VerifiedProductLicense, catalog::QuotaId, entitlements::QuotaLimit, v2,
};

#[derive(Clone, Copy, Debug)]
pub enum VerifiedLicenseRef<'a> {
    V1(&'a VerifiedLicense),
    V2(&'a v2::Verified),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerifiedBinding<'a> {
    Unbound,
    Installation {
        installation_id: &'a str,
        machine_fingerprint_sha256: &'a str,
        transfer_sequence: u32,
    },
}

impl<'a> From<&'a VerifiedLicense> for VerifiedLicenseRef<'a> {
    fn from(value: &'a VerifiedLicense) -> Self {
        Self::V1(value)
    }
}

impl<'a> From<&'a v2::Verified> for VerifiedLicenseRef<'a> {
    fn from(value: &'a v2::Verified) -> Self {
        Self::V2(value)
    }
}

impl<'a> From<&'a VerifiedProductLicense> for VerifiedLicenseRef<'a> {
    fn from(value: &'a VerifiedProductLicense) -> Self {
        match value {
            VerifiedProductLicense::V1(value) => Self::V1(value),
            VerifiedProductLicense::V2(value) => Self::V2(value),
        }
    }
}

impl<'a> VerifiedLicenseRef<'a> {
    pub fn protocol_schema(self) -> &'static str {
        match self {
            Self::V1(_) => crate::LICENSE_SCHEMA,
            Self::V2(_) => v2::SCHEMA,
        }
    }

    pub fn source(self) -> &'a [u8] {
        match self {
            Self::V1(value) => value.source(),
            Self::V2(value) => value.source(),
        }
    }

    pub fn key_id(self) -> &'a str {
        match self {
            Self::V1(value) => &value.claims().key_id,
            Self::V2(value) => &value.claims().key_id,
        }
    }

    pub fn license_id(self) -> &'a str {
        match self {
            Self::V1(value) => &value.claims().license_id,
            Self::V2(value) => &value.claims().license_id,
        }
    }

    pub fn edition(self) -> &'a str {
        match self {
            Self::V1(value) => &value.claims().edition,
            Self::V2(value) => &value.claims().edition,
        }
    }

    pub fn issued_at(self) -> &'a str {
        match self {
            Self::V1(value) => &value.claims().issued_at,
            Self::V2(value) => &value.claims().issued_at,
        }
    }

    pub fn transfer_sequence(self) -> u32 {
        match self.binding() {
            VerifiedBinding::Unbound => 0,
            VerifiedBinding::Installation {
                transfer_sequence, ..
            } => transfer_sequence,
        }
    }

    pub fn features(self) -> Vec<&'a str> {
        match self {
            Self::V1(value) => value.claims().features.iter().map(String::as_str).collect(),
            Self::V2(value) => value
                .claims()
                .entitlements
                .effective_features()
                .into_iter()
                .map(|id| id.as_str())
                .collect(),
        }
    }

    /// Legacy v1 explicitly constrained only seats. Its other current-product
    /// quotas remain unlimited so upgrading the binary does not silently reduce
    /// an already issued v1 license.
    pub fn quota(self, id: QuotaId) -> QuotaLimit {
        match self {
            Self::V1(value) => match id {
                QuotaId::MemberSeats => QuotaLimit::Limited {
                    value: value.claims().limits.member_seats,
                },
                QuotaId::Runners | QuotaId::UpstreamAccounts | QuotaId::ApiKeysPerMember => {
                    QuotaLimit::Unlimited {}
                }
            },
            Self::V2(value) => value
                .claims()
                .entitlements
                .quota(id)
                .expect("verified v2 entitlements declare every quota"),
        }
    }

    pub fn seat_over_limit_grace_days(self) -> u16 {
        match self {
            Self::V1(value) => value.claims().limits.seat_over_limit_grace_days,
            Self::V2(_) => 0,
        }
    }

    pub fn not_before(self) -> &'a str {
        match self {
            Self::V1(value) => &value.claims().not_before,
            Self::V2(value) => &value.claims().validity.not_before,
        }
    }

    pub fn expires_at(self) -> Option<&'a str> {
        match self {
            Self::V1(value) => Some(&value.claims().expires_at),
            Self::V2(value) => match &value.claims().validity.expiry {
                v2::Expiry::Fixed { expires_at } => Some(expires_at),
                v2::Expiry::None {} => None,
            },
        }
    }

    pub fn minimum_version(self) -> &'a str {
        match self {
            Self::V1(value) => &value.claims().minimum_version,
            Self::V2(value) => &value.claims().minimum_version,
        }
    }

    pub fn has_feature(self, required: &str) -> bool {
        match self {
            Self::V1(value) => value.claims().features.iter().any(|id| id == required),
            Self::V2(value) => value
                .claims()
                .entitlements
                .effective_features()
                .into_iter()
                .any(|id| id.as_str() == required),
        }
    }

    pub fn binding(self) -> VerifiedBinding<'a> {
        match self {
            Self::V1(value) => VerifiedBinding::Installation {
                installation_id: &value.claims().installation_id,
                machine_fingerprint_sha256: &value.claims().machine_fingerprint_sha256,
                transfer_sequence: value.claims().transfer_sequence,
            },
            Self::V2(value) => match &value.claims().binding {
                v2::Binding::Unbound {} => VerifiedBinding::Unbound,
                v2::Binding::Installation {
                    installation_id,
                    machine_fingerprint_sha256,
                    transfer_sequence,
                } => VerifiedBinding::Installation {
                    installation_id,
                    machine_fingerprint_sha256,
                    transfer_sequence: *transfer_sequence,
                },
            },
        }
    }
}
