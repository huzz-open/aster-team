//! Candidate preflight must not use Store::open: opening applies migrations.
use super::*;
pub use aster_upgrade_core::migration::{MigrationInspection, PendingMigration};
use sha2::{Digest as _, Sha256};

pub(super) type AppliedMigration = (u32, String, String);

/// Validate the complete known prefix without treating later metadata as
/// permission to mutate schema or as proof of arbitrary business compatibility.
pub(super) fn verify_applied_history(
    rows: &[AppliedMigration],
    migrations: &[EmbeddedMigration],
) -> Result<(), StorageError> {
    validate_migration_sequence(migrations)?;
    if migrations.is_empty() || rows.len() > 1024 {
        return Err(StorageError::MigrationIntegrity);
    }
    if rows.is_empty() {
        return Err(StorageError::Uninitialized);
    }
    for (index, (version, name, checksum)) in rows.iter().enumerate() {
        if usize::try_from(*version).ok() != index.checked_add(1)
            || !aster_upgrade_core::valid_identifier(name)
            || checksum.len() != 64
            || !checksum
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(StorageError::MigrationIntegrity);
        }
        if let Some(expected) = migrations.get(index)
            && (name != expected.name || checksum != &migration_checksum(expected.sql))
        {
            return Err(StorageError::UnsupportedSchema { version: *version });
        }
    }
    Ok(())
}

fn inspect_history(rows: &[AppliedMigration]) -> Result<MigrationInspection, StorageError> {
    inspect_history_for(rows, MARIADB_MIGRATIONS)
}

fn inspect_history_for(
    rows: &[AppliedMigration],
    migrations: &[EmbeddedMigration],
) -> Result<MigrationInspection, StorageError> {
    verify_applied_history(rows, migrations)?;
    let pending = migrations[rows.len().min(migrations.len())..]
        .iter()
        .map(|migration| {
            let compatibility = match migration.compatibility {
                MigrationCompatibility::Baseline => return Err(StorageError::MigrationIntegrity),
                MigrationCompatibility::RollingUpgradeSafe => "rolling_upgrade_safe",
            };
            Ok(PendingMigration {
                version: migration.version,
                name: migration.name.into(),
                checksum_sha256: migration_checksum(migration.sql),
                compatibility: compatibility.into(),
            })
        })
        .collect::<Result<Vec<_>, StorageError>>()?;
    Ok(MigrationInspection {
        schema: "aster.migration-inspection.v2".into(),
        applied_schema_version: rows.last().ok_or(StorageError::Uninitialized)?.0,
        candidate_schema_version: migrations
            .last()
            .ok_or(StorageError::MigrationIntegrity)?
            .version,
        applied_history_sha256: history_digest(rows),
        candidate_history_sha256: history_digest(
            &migrations
                .iter()
                .map(|migration| {
                    (
                        migration.version,
                        migration.name.into(),
                        migration_checksum(migration.sql),
                    )
                })
                .collect::<Vec<_>>(),
        ),
        pending,
    })
}

fn history_digest(rows: &[AppliedMigration]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"aster.migration-history.v1\0");
    for (version, name, checksum) in rows {
        digest.update(version.to_be_bytes());
        digest.update(name.as_bytes());
        digest.update([0]);
        digest.update(checksum.as_bytes());
        digest.update([0]);
    }
    crate::hex_bytes(&digest.finalize())
}

impl MariaDbStore {
    /// Bounded, read-only snapshot of a previously initialized installation.
    /// No migration lock, schema mutation, binding initialization or gate update.
    /// Cancellation drops the detached connection and its read transaction.
    pub async fn inspect_installed_migrations(
        config: &MariaDbConfig,
        installation_id: &str,
        key_fingerprint: &[u8; 32],
    ) -> Result<MigrationInspection, StorageError> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let store = Self::connect(config).await?;
            store.verify_supported_server().await?;
            let mut connection = store.pool.acquire().await?.detach();
            sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
                .execute(&mut connection).await?;
            sqlx::query("START TRANSACTION READ ONLY")
                .execute(&mut connection).await?;
            let binding: Option<(Vec<u8>, Vec<u8>)> = sqlx::query_as(
                "SELECT state_value,mac FROM security_state WHERE state_key='installation.binding.v1'",
            ).fetch_optional(&mut connection).await?;
            if !binding.is_some_and(|(id, fingerprint)| {
                id == installation_id.as_bytes() && fingerprint == key_fingerprint
            }) {
                return Err(StorageError::InstallationMismatch);
            }
            let rows: Vec<AppliedMigration> = sqlx::query_as(
                "SELECT version,name,checksum_sha256 FROM schema_migrations ORDER BY version LIMIT 1025",
            )
                .fetch_all(&mut connection).await?;
            let report = inspect_history(&rows)?;
            sqlx::query("ROLLBACK").execute(&mut connection).await?;
            sqlx::Connection::close(connection).await?;
            store.close().await;
            Ok(report)
        }).await.map_err(|_| StorageError::ReadinessTimeout)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history() -> Vec<AppliedMigration> {
        MARIADB_MIGRATIONS
            .iter()
            .map(|migration| {
                (
                    migration.version,
                    migration.name.into(),
                    migration_checksum(migration.sql),
                )
            })
            .collect()
    }

    #[test]
    fn inspection_reports_each_exact_prefix_without_treating_a_declaration_as_permission() {
        let rows = history();
        for applied in 1..=rows.len() {
            let report = inspect_history(&rows[..applied]).unwrap();
            assert!(report.valid());
            let decoded: MigrationInspection =
                serde_json::from_slice(&serde_json::to_vec(&report).unwrap()).unwrap();
            assert_eq!(decoded, report);
            assert_eq!(report.candidate_history_sha256, history_digest(&rows));
            assert_eq!(report.applied_schema_version as usize, applied);
            assert_eq!(
                report.candidate_schema_version,
                crate::CURRENT_SCHEMA_VERSION
            );
            assert_eq!(report.pending.len(), rows.len() - applied);
            assert_eq!(report.applied_history_sha256.len(), 64);
            for (pending, expected) in report.pending.iter().zip(&MARIADB_MIGRATIONS[applied..]) {
                assert_eq!(pending.version, expected.version);
                assert_eq!(pending.checksum_sha256, migration_checksum(expected.sql));
            }
        }
    }

    #[test]
    fn missing_gapped_reordered_unknown_or_changed_history_fails_closed() {
        let rows = history();
        let mut faults = vec![Vec::new()];
        let mut unknown = rows.clone();
        unknown.push((999, "unknown".into(), "a".repeat(64)));
        faults.push(unknown);
        for field in 0..3 {
            let mut changed = rows.clone();
            match field {
                0 => changed[0].0 = 0,
                1 => changed[0].1 = "changed".into(),
                _ => changed[0].2 = "a".repeat(64),
            }
            faults.push(changed);
        }
        for fault in faults {
            assert!(inspect_history(&fault).is_err());
        }
    }
}
