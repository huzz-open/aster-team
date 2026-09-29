//! Shared read-only migration evidence. Valid evidence is not permission to
//! run online DDL: the executor must separately enforce its migration policy.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationInspection {
    pub schema: String,
    pub applied_schema_version: u32,
    /// Last migration known by the inspected binary, which may be older than
    /// the database after a compatible upgrade and switchback.
    pub candidate_schema_version: u32,
    pub applied_history_sha256: String,
    pub candidate_history_sha256: String,
    pub pending: Vec<PendingMigration>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PendingMigration {
    pub version: u32,
    pub name: String,
    pub checksum_sha256: String,
    pub compatibility: String,
}

impl MigrationInspection {
    #[must_use]
    pub fn valid(&self) -> bool {
        self.schema == "aster.migration-inspection.v2"
            && self.applied_schema_version > 0
            && self.candidate_schema_version > 0
            && crate::valid_checksum(&self.applied_history_sha256)
            && crate::valid_checksum(&self.candidate_history_sha256)
            && usize::try_from(
                self.candidate_schema_version
                    .saturating_sub(self.applied_schema_version),
            )
            .ok()
                == Some(self.pending.len())
            && self.pending.iter().enumerate().all(|(index, migration)| {
                u32::try_from(index).ok().and_then(|index| {
                    self.applied_schema_version
                        .checked_add(index)?
                        .checked_add(1)
                }) == Some(migration.version)
                    && crate::valid_identifier(&migration.name)
                    && crate::valid_checksum(&migration.checksum_sha256)
                    && migration.compatibility == "rolling_upgrade_safe"
            })
            && (self.applied_schema_version != self.candidate_schema_version
                || self.applied_history_sha256 == self.candidate_history_sha256)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationPreflight {
    pub current: MigrationInspection,
    pub candidate: MigrationInspection,
}

impl MigrationPreflight {
    #[must_use]
    pub fn valid(&self) -> bool {
        self.current.valid()
            && self.candidate.valid()
            && self.current.pending.is_empty()
            && self.candidate.candidate_schema_version >= self.candidate.applied_schema_version
            && self.current.applied_schema_version == self.candidate.applied_schema_version
            && self.current.applied_history_sha256 == self.candidate.applied_history_sha256
    }

    /// The signed candidate's inspector revalidates the actual database prefix.
    /// Resume may observe already-applied migrations; it cannot change the
    /// expected full history, move backwards, or rewrite the original prefix.
    #[must_use]
    pub fn accepts_progress(&self, observed: &MigrationInspection) -> bool {
        self.valid()
            && observed.valid()
            && observed.candidate_schema_version == self.candidate.candidate_schema_version
            && observed.candidate_history_sha256 == self.candidate.candidate_history_sha256
            && observed.applied_schema_version >= self.candidate.applied_schema_version
            && observed.applied_schema_version <= self.candidate.candidate_schema_version
            && (observed.applied_schema_version != self.candidate.applied_schema_version
                || observed.applied_history_sha256 == self.candidate.applied_history_sha256)
            && self.candidate.pending.ends_with(&observed.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> MigrationPreflight {
        let current = MigrationInspection {
            schema: "aster.migration-inspection.v2".into(),
            applied_schema_version: 1,
            candidate_schema_version: 1,
            applied_history_sha256: "a".repeat(64),
            candidate_history_sha256: "a".repeat(64),
            pending: vec![],
        };
        let candidate = MigrationInspection {
            candidate_schema_version: 2,
            candidate_history_sha256: "b".repeat(64),
            pending: vec![PendingMigration {
                version: 2,
                name: "add-column".into(),
                checksum_sha256: "c".repeat(64),
                compatibility: "rolling_upgrade_safe".into(),
            }],
            ..current.clone()
        };
        MigrationPreflight { current, candidate }
    }

    #[test]
    fn reports_require_one_history_and_resume_preserves_the_expected_target() {
        let evidence = fixture();
        assert!(evidence.valid());
        assert!(evidence.accepts_progress(&evidence.candidate));
        let mut completed = evidence.candidate.clone();
        completed.applied_schema_version = 2;
        completed.applied_history_sha256 = completed.candidate_history_sha256.clone();
        completed.pending.clear();
        assert!(evidence.accepts_progress(&completed));
        completed.candidate_history_sha256 = "d".repeat(64);
        completed.applied_history_sha256 = completed.candidate_history_sha256.clone();
        assert!(completed.valid());
        assert!(!evidence.accepts_progress(&completed));
        let mut changed = evidence.clone();
        changed.current.applied_history_sha256 = "d".repeat(64);
        changed.current.candidate_history_sha256 = "d".repeat(64);
        assert!(!changed.valid());
    }

    #[test]
    fn partial_migration_resume_keeps_the_original_remaining_sequence() {
        let mut evidence = fixture();
        evidence.candidate.candidate_schema_version = 3;
        evidence.candidate.pending.push(PendingMigration {
            version: 3,
            name: "add-index".into(),
            checksum_sha256: "d".repeat(64),
            compatibility: "rolling_upgrade_safe".into(),
        });
        assert!(evidence.valid());
        let mut progress = evidence.candidate.clone();
        progress.applied_schema_version = 2;
        progress.applied_history_sha256 = "e".repeat(64);
        progress.pending.remove(0);
        assert!(evidence.accepts_progress(&progress));
        progress.pending[0].name = "other-index".into();
        assert!(progress.valid());
        assert!(!evidence.accepts_progress(&progress));
    }

    #[test]
    fn an_older_current_binary_can_report_a_newer_database_but_candidate_cannot_downgrade_it() {
        let mut evidence = fixture();
        evidence.current.applied_schema_version = 2;
        evidence.current.applied_history_sha256 = "b".repeat(64);
        evidence.candidate.applied_schema_version = 2;
        evidence.candidate.applied_history_sha256 = "b".repeat(64);
        evidence.candidate.pending.clear();
        assert!(evidence.current.valid());
        assert!(evidence.valid());
        assert!(evidence.accepts_progress(&evidence.candidate));
        let mut beyond_target = evidence.candidate.clone();
        beyond_target.applied_schema_version = 3;
        beyond_target.applied_history_sha256 = "d".repeat(64);
        assert!(beyond_target.valid());
        assert!(!evidence.accepts_progress(&beyond_target));
        evidence.candidate = evidence.current.clone();
        assert!(!evidence.valid());
    }

    #[test]
    fn malformed_versions_gaps_declarations_and_digests_are_rejected() {
        let report = fixture().candidate;
        let mut cases = Vec::new();
        let mut bad = report.clone();
        bad.applied_schema_version = 0;
        cases.push(bad);
        let mut bad = report.clone();
        bad.pending[0].version = 3;
        cases.push(bad);
        let mut bad = report.clone();
        bad.pending[0].compatibility = "allow".into();
        cases.push(bad);
        let mut bad = report.clone();
        bad.pending[0].checksum_sha256 = "invalid".into();
        cases.push(bad);
        let mut bad = report.clone();
        bad.pending.clear();
        cases.push(bad);
        let mut bad = report.clone();
        bad.candidate_schema_version = 0;
        cases.push(bad);
        let mut bad = report;
        bad.schema = "aster.migration-inspection.v1".into();
        cases.push(bad);
        for bad in cases {
            assert!(!bad.valid());
        }
    }

    #[test]
    fn current_pending_migrations_and_rewritten_remaining_sql_cannot_be_reused() {
        let mut evidence = fixture();
        let mut changed = evidence.candidate.clone();
        changed.pending[0].checksum_sha256 = "d".repeat(64);
        assert!(changed.valid());
        assert!(!evidence.accepts_progress(&changed));
        evidence.current = evidence.candidate.clone();
        assert!(!evidence.valid());
    }
}
