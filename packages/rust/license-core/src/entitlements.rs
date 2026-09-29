//! Shared entitlement payload used by immutable plans and the next license protocol.
//! Validation establishes structural consistency, not signature authenticity or user access.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::catalog::{CAPABILITIES, CATALOG_VERSION, CapabilityId, FeatureSetId, QuotaId};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum QuotaLimit {
    Limited { value: u32 },
    Unlimited {},
}

impl QuotaLimit {
    pub const fn permits(self, occupied: u32) -> bool {
        match self {
            Self::Limited { value } => occupied <= value,
            Self::Unlimited {} => true,
        }
    }

    /// A free issuer cannot turn a finite ceiling into an unlimited grant.
    pub const fn is_within(self, ceiling: Self) -> bool {
        match (self, ceiling) {
            (_, Self::Unlimited {}) => true,
            (Self::Unlimited {}, Self::Limited { .. }) => false,
            (Self::Limited { value }, Self::Limited { value: maximum }) => value <= maximum,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuotaGrant {
    pub id: QuotaId,
    pub limit: QuotaLimit,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entitlements {
    pub catalog_version: u32,
    pub features: Vec<CapabilityId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub feature_sets: Vec<FeatureSetId>,
    pub quotas: Vec<QuotaGrant>,
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum EntitlementError {
    #[error("unsupported capability catalog version")]
    CatalogVersion,
    #[error("duplicate capability")]
    DuplicateCapability,
    #[error("missing capability dependency")]
    MissingDependency,
    #[error("every quota must be explicitly declared exactly once")]
    IncompleteQuotas,
    #[error("entitlements exceed the issuer's allowed scope")]
    IssuerScope,
}

impl Entitlements {
    pub fn validate(&self) -> Result<(), EntitlementError> {
        if self.catalog_version != CATALOG_VERSION {
            return Err(EntitlementError::CatalogVersion);
        }
        let features: BTreeSet<_> = self.features.iter().copied().collect();
        if features.len() != self.features.len() {
            return Err(EntitlementError::DuplicateCapability);
        }
        if self
            .feature_sets
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len()
            != self.feature_sets.len()
        {
            return Err(EntitlementError::DuplicateCapability);
        }
        let features: BTreeSet<_> = self.effective_features().into_iter().collect();
        for entry in CAPABILITIES {
            if features.contains(&entry.id)
                && entry
                    .requires
                    .iter()
                    .any(|required| !features.contains(required))
            {
                return Err(EntitlementError::MissingDependency);
            }
        }
        let quotas: BTreeSet<_> = self.quotas.iter().map(|grant| grant.id).collect();
        if quotas.len() != QuotaId::ALL.len()
            || quotas.len() != self.quotas.len()
            || QuotaId::ALL.iter().any(|id| !quotas.contains(id))
        {
            return Err(EntitlementError::IncompleteQuotas);
        }
        Ok(())
    }

    /// Resolve signed symbolic grants against compiled metadata only. Never
    /// replace the signed payload with this projection.
    pub fn effective_features(&self) -> Vec<CapabilityId> {
        CAPABILITIES
            .iter()
            .filter(|entry| {
                self.features.contains(&entry.id) || self.feature_sets.contains(&entry.feature_set)
            })
            .map(|entry| entry.id)
            .collect()
    }

    pub fn quota(&self, id: QuotaId) -> Option<QuotaLimit> {
        self.quotas
            .iter()
            .find(|grant| grant.id == id)
            .map(|grant| grant.limit)
    }

    pub fn ensure_within(&self, ceiling: &Self) -> Result<(), EntitlementError> {
        self.validate()?;
        ceiling.validate()?;
        if self
            .features
            .iter()
            .any(|id| !ceiling.effective_features().contains(id))
            || self
                .feature_sets
                .iter()
                .any(|id| !ceiling.feature_sets.contains(id))
            || self.quotas.iter().any(|grant| {
                !ceiling
                    .quota(grant.id)
                    .is_some_and(|limit| grant.limit.is_within(limit))
            })
        {
            return Err(EntitlementError::IssuerScope);
        }
        Ok(())
    }
}
