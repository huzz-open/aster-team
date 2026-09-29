use super::{EmbeddedMigration, MySqlConnection, StorageError};
use std::{future::Future, time::Duration};

/// Selected by trusted installation/upgrade context, never by License bypass flags.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MariaDbMigrationPolicy {
    Maintenance,
    Online,
    /// Candidate startup/finalization must not silently apply missing migrations.
    Existing,
}

impl MariaDbMigrationPolicy {
    pub(super) async fn bounded<T>(
        self,
        operation: impl Future<Output = Result<T, StorageError>>,
    ) -> Result<T, StorageError> {
        let seconds = match self {
            Self::Maintenance => return operation.await,
            Self::Online => 30,
            Self::Existing => 10,
        };
        tokio::time::timeout(Duration::from_secs(seconds), operation)
            .await
            .map_err(|_| StorageError::MigrationTimeout)?
    }

    pub(super) const fn lock_wait_seconds(self) -> u32 {
        match self {
            Self::Maintenance => 30,
            Self::Online | Self::Existing => 2,
        }
    }

    pub(super) fn validate_pending(
        self,
        pending: &[EmbeddedMigration],
    ) -> Result<(), StorageError> {
        for migration in pending {
            self.sql(*migration)?;
            if migration.recovery_query.is_none() {
                return Err(StorageError::MigrationIntegrity);
            }
        }
        Ok(())
    }

    pub(super) fn sql(self, migration: EmbeddedMigration) -> Result<&'static str, StorageError> {
        match self {
            Self::Maintenance => Ok(migration.sql),
            Self::Online => migration.online_sql.ok_or(StorageError::MigrationIntegrity),
            Self::Existing => Err(StorageError::MigrationIntegrity),
        }
    }

    pub(super) async fn configure(
        self,
        connection: &mut MySqlConnection,
    ) -> Result<(), StorageError> {
        if self == Self::Online {
            // Server interruption is cooperative, not a hard execution guarantee.
            // The client budget closes the owned detached session on cancellation;
            // the next attempt still acquires the schema lock and checks effects.
            sqlx::query("SET SESSION lock_wait_timeout=0")
                .execute(&mut *connection)
                .await?;
            sqlx::query("SET SESSION innodb_lock_wait_timeout=1")
                .execute(&mut *connection)
                .await?;
            sqlx::query("SET SESSION max_statement_time=20")
                .execute(&mut *connection)
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
pub(super) mod tests;
