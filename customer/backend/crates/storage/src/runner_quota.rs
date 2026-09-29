//! Logical quota is derived from authenticated upgrade bindings, never names.
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{SecurityStateRecord, StorageError};

pub const RUNNER_QUOTA_KEY: &str = "runner-upgrade-quota-v1";
const SCHEMA: &str = "aster.runner-upgrade-quota.v1";
const MAX_BINDINGS: usize = 128;
const MAX_STATE_BYTES: usize = 128 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerQuotaMember {
    pub id: String,
    pub credential_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerUpgradeBinding {
    pub phase: RunnerUpgradePhase,
    pub installation_id: String,
    pub job_id: String,
    pub logical_runner_id: String,
    pub previous: RunnerQuotaMember,
    pub candidate: RunnerQuotaMember,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerUpgradePhase {
    Prepared,
    Committed,
    RolledBack,
}

impl RunnerUpgradeBinding {
    fn current(&self) -> Option<&RunnerQuotaMember> {
        match self.phase {
            RunnerUpgradePhase::Prepared => None,
            RunnerUpgradePhase::Committed => Some(&self.candidate),
            RunnerUpgradePhase::RolledBack => Some(&self.previous),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerQuotaBindings {
    schema: String,
    bindings: Vec<RunnerUpgradeBinding>,
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl RunnerQuotaBindings {
    pub fn new(bindings: Vec<RunnerUpgradeBinding>) -> Result<Self, StorageError> {
        let value = Self {
            schema: SCHEMA.into(),
            bindings,
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), StorageError> {
        let mut members = BTreeSet::new();
        let mut logical = BTreeSet::new();
        let mut jobs = BTreeSet::new();
        if self.schema != SCHEMA || self.bindings.len() > MAX_BINDINGS {
            return Err(StorageError::RunnerQuotaIntegrity);
        }
        for binding in &self.bindings {
            if !identifier(&binding.installation_id)
                || !identifier(&binding.job_id)
                || !identifier(&binding.logical_runner_id)
                || !logical.insert((&binding.installation_id, &binding.logical_runner_id))
                || !jobs.insert((&binding.installation_id, &binding.job_id))
            {
                return Err(StorageError::RunnerQuotaIntegrity);
            }
            for member in [&binding.previous, &binding.candidate] {
                if !identifier(&member.id)
                    || !member.id.starts_with("runner_")
                    || member.credential_hash.len() != 64
                    || !member
                        .credential_hash
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    || !members.insert(&member.id)
                {
                    return Err(StorageError::RunnerQuotaIntegrity);
                }
            }
            if binding.previous.credential_hash == binding.candidate.credential_hash {
                return Err(StorageError::RunnerQuotaIntegrity);
            }
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, StorageError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| StorageError::RunnerQuotaIntegrity)?;
        if bytes.len() > MAX_STATE_BYTES {
            return Err(StorageError::RunnerQuotaIntegrity);
        }
        Ok(bytes)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunnerQuotaSnapshot {
    pub state: Option<SecurityStateRecord>,
    pub members: Vec<RunnerQuotaMember>,
}

/// Only constructed after the Control verifies its installation-scoped MAC.
/// Transactions compare the complete snapshot again after acquiring quota lock.
#[derive(Clone, Debug)]
pub struct VerifiedRunnerQuota {
    snapshot: RunnerQuotaSnapshot,
    occupied: u32,
    upgrade_members: BTreeSet<String>,
    bindings: RunnerQuotaBindings,
    installation_id: String,
}

impl RunnerQuotaSnapshot {
    pub fn verify(
        self,
        installation_id: &str,
        authenticate: impl FnOnce(&SecurityStateRecord) -> bool,
    ) -> Result<VerifiedRunnerQuota, StorageError> {
        let mut ids = BTreeSet::new();
        if self.members.iter().any(|member| !ids.insert(&member.id)) {
            return Err(StorageError::RunnerQuotaIntegrity);
        }
        let bindings = match &self.state {
            None => RunnerQuotaBindings::new(Vec::new())?,
            Some(state) => {
                if state.key != RUNNER_QUOTA_KEY
                    || state.value.len() > MAX_STATE_BYTES
                    || !authenticate(state)
                {
                    return Err(StorageError::RunnerQuotaIntegrity);
                }
                let value: RunnerQuotaBindings = serde_json::from_slice(&state.value)
                    .map_err(|_| StorageError::RunnerQuotaIntegrity)?;
                value.validate()?;
                // Canonical encoding avoids alternate encodings of one binding.
                if value.encode()? != state.value {
                    return Err(StorageError::RunnerQuotaIntegrity);
                }
                value
            }
        };
        for binding in &bindings.bindings {
            let present = |member: &RunnerQuotaMember| self.members.contains(member);
            let absent = |member: &RunnerQuotaMember| {
                self.members.iter().all(|actual| actual.id != member.id)
            };
            let matches_phase = match binding.phase {
                RunnerUpgradePhase::Prepared => {
                    present(&binding.previous) && present(&binding.candidate)
                }
                RunnerUpgradePhase::Committed => {
                    absent(&binding.previous) && present(&binding.candidate)
                }
                RunnerUpgradePhase::RolledBack => {
                    present(&binding.previous) && absent(&binding.candidate)
                }
            };
            if binding.installation_id != installation_id || !matches_phase {
                return Err(StorageError::RunnerQuotaIntegrity);
            }
        }
        let occupied = self
            .members
            .len()
            .checked_sub(
                bindings
                    .bindings
                    .iter()
                    .filter(|binding| binding.phase == RunnerUpgradePhase::Prepared)
                    .count(),
            )
            .and_then(|count| u32::try_from(count).ok())
            .ok_or(StorageError::RunnerQuotaIntegrity)?;
        let upgrade_members = bindings
            .bindings
            .iter()
            .filter(|binding| binding.phase == RunnerUpgradePhase::Prepared)
            .flat_map(|binding| [binding.previous.id.clone(), binding.candidate.id.clone()])
            .collect();
        Ok(VerifiedRunnerQuota {
            snapshot: self,
            occupied,
            upgrade_members,
            bindings,
            installation_id: installation_id.into(),
        })
    }
}

impl VerifiedRunnerQuota {
    #[must_use]
    pub fn upgrade_binding(&self, job_id: &str) -> Option<&RunnerUpgradeBinding> {
        self.bindings
            .bindings
            .iter()
            .find(|binding| binding.job_id == job_id)
    }

    /// Preserve a logical identity across successive, fully retired upgrades.
    /// Pending bindings are returned too so callers cannot create a second job
    /// for either physical member of an unfinished upgrade.
    pub fn logical_id_for<'a>(
        &'a self,
        member: &'a RunnerQuotaMember,
    ) -> Result<&'a str, StorageError> {
        if !self.snapshot.members.contains(member) {
            return Err(StorageError::RunnerQuotaIntegrity);
        }
        Ok(self
            .bindings
            .bindings
            .iter()
            .find(|binding| &binding.previous == member || &binding.candidate == member)
            .map_or(member.id.as_str(), |binding| {
                binding.logical_runner_id.as_str()
            }))
    }

    /// Prepare registration and its signed ownership update together. An
    /// existing unfinished job cannot be replaced by a new job or candidate.
    pub fn begin_upgrade(
        &self,
        binding: RunnerUpgradeBinding,
    ) -> Result<RunnerQuotaChange, StorageError> {
        if binding.phase != RunnerUpgradePhase::Prepared
            || binding.installation_id != self.installation_id
            || !self.snapshot.members.contains(&binding.previous)
            || self
                .snapshot
                .members
                .iter()
                .any(|member| member.id == binding.candidate.id)
        {
            return Err(StorageError::RunnerQuotaIntegrity);
        }
        let mut next = self.bindings.clone();
        if let Some(index) = next
            .bindings
            .iter()
            .position(|old| old.logical_runner_id == binding.logical_runner_id)
        {
            let old = &next.bindings[index];
            if old.current() != Some(&binding.previous) || old.job_id == binding.job_id {
                return Err(StorageError::RunnerQuotaIntegrity);
            }
            next.bindings[index] = binding.clone();
        } else {
            next.bindings.push(binding.clone());
        }
        next.validate()?;
        let mut members = self.snapshot.members.clone();
        members.push(binding.candidate.clone());
        self.change(
            next,
            members,
            RunnerQuotaAction::Register(binding.candidate),
        )
    }

    /// A matching completed receipt is an idempotent success after a lost reply.
    /// Changing the selected survivor after finalization is never allowed.
    pub fn finish_upgrade(
        &self,
        job_id: &str,
        keep_candidate: bool,
    ) -> Result<Option<RunnerQuotaChange>, StorageError> {
        let mut next = self.bindings.clone();
        let binding = next
            .bindings
            .iter_mut()
            .find(|binding| binding.job_id == job_id)
            .ok_or(StorageError::RunnerQuotaIntegrity)?;
        let phase = if keep_candidate {
            RunnerUpgradePhase::Committed
        } else {
            RunnerUpgradePhase::RolledBack
        };
        if binding.phase == phase {
            return Ok(None);
        }
        if binding.phase != RunnerUpgradePhase::Prepared {
            return Err(StorageError::RunnerQuotaIntegrity);
        }
        let retired = if keep_candidate {
            binding.previous.clone()
        } else {
            binding.candidate.clone()
        };
        binding.phase = phase;
        let members = self
            .snapshot
            .members
            .iter()
            .filter(|member| member.id != retired.id)
            .cloned()
            .collect();
        self.change(next, members, RunnerQuotaAction::Delete(retired))
            .map(Some)
    }

    /// Ordinary deletion can release a completed logical identity, while an
    /// in-progress pair is reserved for the installation recovery workflow.
    pub fn delete_runner(&self, id: &str) -> Result<RunnerQuotaChange, StorageError> {
        if self.is_upgrade_member(id) {
            return Err(StorageError::RunnerQuotaIntegrity);
        }
        let retired = self
            .snapshot
            .members
            .iter()
            .find(|member| member.id == id)
            .cloned()
            .ok_or(StorageError::RunnerQuotaIntegrity)?;
        let mut next = self.bindings.clone();
        next.bindings
            .retain(|binding| binding.current().is_none_or(|member| member.id != id));
        let members = self
            .snapshot
            .members
            .iter()
            .filter(|member| member.id != id)
            .cloned()
            .collect();
        self.change(next, members, RunnerQuotaAction::Delete(retired))
    }

    fn change(
        &self,
        next: RunnerQuotaBindings,
        mut members: Vec<RunnerQuotaMember>,
        action: RunnerQuotaAction,
    ) -> Result<RunnerQuotaChange, StorageError> {
        members.sort_by(|left, right| left.id.cmp(&right.id));
        let revision = self
            .snapshot
            .state
            .as_ref()
            .map_or(0, |state| state.revision)
            .checked_add(1)
            .filter(|revision| i64::try_from(*revision).is_ok())
            .ok_or(StorageError::RunnerQuotaIntegrity)?;
        let value = next.encode()?;
        Ok(RunnerQuotaChange {
            expected: self.clone(),
            next_members: members,
            value,
            revision,
            action,
        })
    }

    #[must_use]
    pub fn is_upgrade_member(&self, id: &str) -> bool {
        self.upgrade_members.contains(id)
    }

    #[must_use]
    pub const fn occupied(&self) -> u32 {
        self.occupied
    }

    #[must_use]
    pub fn matches(&self, snapshot: &RunnerQuotaSnapshot) -> bool {
        self.snapshot == *snapshot
    }
}

/// No public fields: storage accepts only a transition derived from a verified
/// snapshot, not arbitrary client-provided quota reductions or retire targets.
#[derive(Clone, Debug)]
pub struct RunnerQuotaChange {
    expected: VerifiedRunnerQuota,
    next_members: Vec<RunnerQuotaMember>,
    value: Vec<u8>,
    revision: u64,
    action: RunnerQuotaAction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunnerQuotaAction {
    Register(RunnerQuotaMember),
    Delete(RunnerQuotaMember),
}

impl RunnerQuotaChange {
    #[must_use]
    pub fn action(&self) -> &RunnerQuotaAction {
        &self.action
    }
    #[must_use]
    pub fn value(&self) -> &[u8] {
        &self.value
    }
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    #[must_use]
    pub const fn occupied_before(&self) -> u32 {
        self.expected.occupied
    }
    #[must_use]
    pub fn matches_before(&self, actual: &RunnerQuotaSnapshot) -> bool {
        self.expected.matches(actual)
    }
    #[must_use]
    pub fn validates_next(&self, record: &SecurityStateRecord) -> bool {
        record.key == RUNNER_QUOTA_KEY
            && record.value == self.value
            && record.revision == self.revision
            && !record.mac.is_empty()
            && !record.updated_at.is_empty()
    }
    #[must_use]
    pub fn matches_after(&self, members: &[RunnerQuotaMember]) -> bool {
        self.next_members == members
    }
}

pub struct RunnerQuotaWrite<'a> {
    pub change: &'a RunnerQuotaChange,
    pub next_state: &'a SecurityStateRecord,
    pub enrollment: Option<&'a crate::RunnerEnrollmentRecord>,
    pub registration: Option<&'a crate::RunnerRegistrationRecord>,
}

impl RunnerQuotaWrite<'_> {
    pub(crate) fn valid(&self, event: &crate::AuditEventRecord) -> bool {
        if !self.change.validates_next(self.next_state) {
            return false;
        }
        match (self.change.action(), self.enrollment, self.registration) {
            (RunnerQuotaAction::Register(member), Some(enrollment), Some(registration)) => {
                member.id == registration.id
                    && member.credential_hash == registration.credential_hash
                    && enrollment.status == "pending"
                    && event.action == "runner.register"
                    && event.target_id.as_deref() == Some(member.id.as_str())
            }
            (RunnerQuotaAction::Delete(member), None, None) => {
                event.action == "runner.delete"
                    && event.target_id.as_deref() == Some(member.id.as_str())
            }
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunnerQuotaWriteOutcome {
    Applied,
    QuotaChanged,
    LimitReached,
    Conflict,
    AuditConflict,
}

#[derive(Clone, Debug)]
pub struct RunnerQuotaPolicy {
    pub limit: Option<u32>,
    pub verified: Option<VerifiedRunnerQuota>,
}

impl RunnerQuotaPolicy {
    #[must_use]
    pub const fn physical(limit: Option<u32>) -> Self {
        Self {
            limit,
            verified: None,
        }
    }

    pub(crate) fn occupied(
        &self,
        actual: &RunnerQuotaSnapshot,
    ) -> Result<Option<u32>, StorageError> {
        match &self.verified {
            Some(verified) if verified.matches(actual) => Ok(Some(verified.occupied())),
            Some(_) => Ok(None),
            // Legacy/internal callers may count physical rows only when there
            // is no logical binding. They cannot silently bypass its MAC check.
            None if actual.state.is_none() => u32::try_from(actual.members.len())
                .map(Some)
                .map_err(|_| StorageError::RunnerQuotaIntegrity),
            None => Err(StorageError::RunnerQuotaIntegrity),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn member(id: &str, digit: &str) -> RunnerQuotaMember {
        RunnerQuotaMember {
            id: id.into(),
            credential_hash: digit.repeat(64),
        }
    }

    fn snapshot() -> RunnerQuotaSnapshot {
        let previous = member("runner_old", "a");
        let candidate = member("runner_new", "b");
        let value = RunnerQuotaBindings::new(vec![RunnerUpgradeBinding {
            phase: RunnerUpgradePhase::Prepared,
            installation_id: "installation".into(),
            job_id: "upgrade-one".into(),
            logical_runner_id: "runner_old".into(),
            previous: previous.clone(),
            candidate: candidate.clone(),
        }])
        .unwrap()
        .encode()
        .unwrap();
        RunnerQuotaSnapshot {
            state: Some(SecurityStateRecord {
                key: RUNNER_QUOTA_KEY.into(),
                value,
                revision: 1,
                mac: b"authenticated".to_vec(),
                updated_at: "now".into(),
            }),
            members: vec![candidate, previous],
        }
    }

    #[test]
    fn signed_pair_counts_once_and_unrelated_runner_counts_separately() {
        let mut actual = snapshot();
        let verified = actual.clone().verify("installation", |_| true).unwrap();
        assert_eq!(verified.occupied(), 1);
        assert!(verified.is_upgrade_member("runner_old"));
        assert!(verified.is_upgrade_member("runner_new"));
        assert!(!verified.is_upgrade_member("runner_other"));
        actual.members.push(member("runner_other", "c"));
        assert!(!verified.matches(&actual));
        assert_eq!(
            actual.verify("installation", |_| true).unwrap().occupied(),
            2
        );
    }

    #[test]
    fn absent_binding_counts_every_instance_and_never_requires_authentication() {
        let mut actual = snapshot();
        actual.state = None;
        assert_eq!(
            actual
                .verify("installation", |_| panic!("no record to verify"))
                .unwrap()
                .occupied(),
            2
        );
    }

    #[test]
    fn broken_or_foreign_binding_never_earns_quota_credit() {
        assert!(snapshot().verify("installation", |_| false).is_err());
        assert!(snapshot().verify("foreign-installation", |_| true).is_err());
        let mut missing = snapshot();
        missing.members.pop();
        assert!(missing.verify("installation", |_| true).is_err());
        let mut changed = snapshot();
        changed.members[0].credential_hash = "c".repeat(64);
        assert!(changed.verify("installation", |_| true).is_err());
        let mut corrupt = snapshot();
        corrupt.state.as_mut().unwrap().value.push(b' ');
        assert!(corrupt.verify("installation", |_| true).is_err());
        let mut duplicate = snapshot();
        duplicate.members.push(duplicate.members[0].clone());
        assert!(duplicate.verify("installation", |_| true).is_err());
        assert!(
            RunnerQuotaPolicy::physical(Some(3))
                .occupied(&snapshot())
                .is_err()
        );
    }

    #[test]
    fn duplicate_job_logical_identity_member_or_secret_is_rejected() {
        let encoded = snapshot().state.unwrap().value;
        let original: RunnerQuotaBindings = serde_json::from_slice(&encoded).unwrap();
        let binding = original.bindings[0].clone();
        assert!(RunnerQuotaBindings::new(vec![binding.clone(), binding.clone()]).is_err());
        let mut same_member = binding.clone();
        same_member.candidate = same_member.previous.clone();
        assert!(RunnerQuotaBindings::new(vec![same_member]).is_err());
        let mut same_secret = binding.clone();
        same_secret.candidate.credential_hash = same_secret.previous.credential_hash.clone();
        assert!(RunnerQuotaBindings::new(vec![same_secret]).is_err());
        let mut empty_job = binding;
        empty_job.job_id.clear();
        assert!(RunnerQuotaBindings::new(vec![empty_job]).is_err());
    }
}
