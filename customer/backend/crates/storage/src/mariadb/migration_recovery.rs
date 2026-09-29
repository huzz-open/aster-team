//! DDL and its history INSERT are separate commits. Reconcile only an exact
//! trusted postcondition, never a duplicate-column error or a missing record alone.
use super::{EmbeddedMigration, MySqlConnection, StorageError};

#[derive(Debug, Eq, PartialEq)]
enum Effects {
    Absent,
    Complete,
}

async fn inspect(
    migration: EmbeddedMigration,
    connection: &mut MySqlConnection,
) -> Result<Effects, StorageError> {
    let query = migration
        .recovery_query
        .ok_or(StorageError::MigrationIntegrity)?;
    match sqlx::query_scalar::<_, i64>(query)
        .fetch_one(connection)
        .await?
    {
        0 => Ok(Effects::Absent),
        1 => Ok(Effects::Complete),
        _ => Err(StorageError::MigrationIntegrity),
    }
}

/// The caller owns the installation's schema advisory lock, has validated the
/// full recorded prefix, and records completion only after this succeeds.
pub(super) async fn apply_or_reconcile(
    migration: EmbeddedMigration,
    sql: &'static str,
    connection: &mut MySqlConnection,
) -> Result<(), StorageError> {
    if inspect(migration, connection).await? == Effects::Absent {
        sqlx::raw_sql(sql).execute(&mut *connection).await?;
        if inspect(migration, connection).await? != Effects::Complete {
            return Err(StorageError::MigrationIntegrity);
        }
    }
    Ok(())
}
