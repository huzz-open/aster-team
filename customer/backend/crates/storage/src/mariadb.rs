mod migration_inspection;
mod migration_policy;
mod migration_recovery;
pub use migration_inspection::{MigrationInspection, PendingMigration};
pub use migration_policy::MariaDbMigrationPolicy;

use crate::runner_quota::{
    RUNNER_QUOTA_KEY, RunnerQuotaMember, RunnerQuotaPolicy, RunnerQuotaSnapshot,
};
use sqlx::{
    ConnectOptions, MySql, MySqlConnection, MySqlPool, Row,
    mysql::{MySqlConnectOptions, MySqlPoolOptions, MySqlSslMode},
};

use crate::money_ledger::valid_initial_money_balance;
use crate::{
    ApiKeyRecord, AuditAppendOutcome, AuditEventRecord, AuditedMutationOutcome,
    AuthenticatedSession, AuthorizedApiKey, CredentialRefreshLeaseOutcome, CredentialRefreshUpdate,
    DiscoveredModel, EmbeddedMigration, EncryptedCredentialInstance, GatewayRouteCandidate,
    IdentityCreateOutcome, IdentityInitializeOutcome, IdentityRecord, IdentityStatusUpdateOutcome,
    MigrationCompatibility, ModelRecord, MoneyBalanceRecord, MoneyLedgerEntry,
    MoneyMutationOutcome, MoneyStateSnapshot, MutationWithAuditOutcome, QuotaBatchImageWrite,
    QuotaBatchTokenWrite, QuotaLedgerEntry, QuotaMutationOutcome, QuotaRequestInsertOutcome,
    QuotaRequestRecord, QuotaRequestReviewOutcome, QuotaReservationRecord, QuotaStateSnapshot,
    ResourceCreateOutcome, RunnerAdminRecord, RunnerConnectionUpdate,
    RunnerEnrollmentConsumeOutcome, RunnerEnrollmentRecord, RunnerHeartbeatUpdate, RunnerRecord,
    RunnerRegistrationRecord, RuntimeSettingRecord, RuntimeSettingWriteOutcome,
    SeatRegistrySnapshot, SecurityStateRecord, SessionRecord, StorageError, UpstreamAccountRecord,
    UserBalanceRecord, VoucherDeliveryRecord, VoucherRecord, VoucherRedeemOutcome,
    VoucherRedemptionRecord, encode_seat_registry, migration_checksum, valid_audit_event_batch,
    valid_failed_request_transition, valid_grant_transition, valid_identity_status_transition,
    valid_image_balance_transition, valid_money_transition, valid_release_transition,
    valid_reservation_subject, valid_reserve_transition, valid_settlement_transition,
    validate_migration_sequence,
};

const MARIADB_BASELINE: &str = include_str!("../../../schema/init.mariadb.sql");
const MARIADB_MIGRATIONS: &[EmbeddedMigration] = &[EmbeddedMigration {
    version: 1,
    name: "baseline-2.2.0",
    sql: MARIADB_BASELINE,
    compatibility: MigrationCompatibility::Baseline,
    online_sql: None,
    recovery_query: None,
}];

#[derive(Clone, Eq, PartialEq)]
pub struct MariaDbConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
    pub tls: bool,
    pub ca_certificate: Option<std::path::PathBuf>,
    pub max_connections: u32,
}

impl std::fmt::Debug for MariaDbConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MariaDbConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .field("tls", &self.tls)
            .field("ca_certificate", &self.ca_certificate)
            .field("max_connections", &self.max_connections)
            .finish()
    }
}

#[derive(Clone)]
pub struct MariaDbStore {
    pool: MySqlPool,
}

impl MariaDbStore {
    pub async fn initialize(config: &MariaDbConfig) -> Result<Self, StorageError> {
        let store = Self::connect(config).await?;
        store.apply_migrations(MARIADB_MIGRATIONS).await?;
        Ok(store)
    }

    pub async fn open(config: &MariaDbConfig) -> Result<Self, StorageError> {
        let store = Self::connect(config).await?;
        store.apply_migrations(MARIADB_MIGRATIONS).await?;
        Ok(store)
    }

    pub async fn open_installed(
        config: &MariaDbConfig,
        installation_id: &str,
        key_fingerprint: &[u8; 32],
        initialize: bool,
    ) -> Result<Self, StorageError> {
        Self::open_installed_with_policy(
            config,
            installation_id,
            key_fingerprint,
            initialize,
            MariaDbMigrationPolicy::Maintenance,
        )
        .await
    }

    pub async fn open_installed_with_policy(
        config: &MariaDbConfig,
        installation_id: &str,
        key_fingerprint: &[u8; 32],
        initialize: bool,
        policy: MariaDbMigrationPolicy,
    ) -> Result<Self, StorageError> {
        if initialize && policy != MariaDbMigrationPolicy::Maintenance {
            return Err(StorageError::MigrationIntegrity);
        }
        policy
            .bounded(Self::open_installed_policy(
                config,
                installation_id,
                key_fingerprint,
                initialize,
                policy,
            ))
            .await
    }

    async fn open_installed_policy(
        config: &MariaDbConfig,
        installation_id: &str,
        key_fingerprint: &[u8; 32],
        initialize: bool,
        policy: MariaDbMigrationPolicy,
    ) -> Result<Self, StorageError> {
        let store = Self::connect(config).await?;
        store.verify_supported_server().await?;
        let empty = {
            let mut connection = store.pool.acquire().await?;
            !has_user_tables_on(&mut connection).await?
        };
        if empty && !initialize {
            return Err(StorageError::Uninitialized);
        }
        if !empty {
            store
                .verify_installation(installation_id, key_fingerprint, false)
                .await?;
        }
        store
            .apply_migrations_with_policy(MARIADB_MIGRATIONS, policy)
            .await?;
        store
            .verify_installation(installation_id, key_fingerprint, initialize && empty)
            .await?;
        Ok(store)
    }

    async fn connect(config: &MariaDbConfig) -> Result<Self, StorageError> {
        let ssl_mode = if config.tls {
            MySqlSslMode::VerifyIdentity
        } else {
            MySqlSslMode::Disabled
        };
        let mut options = MySqlConnectOptions::new()
            .host(&config.host)
            .port(config.port)
            .database(&config.database)
            .username(&config.username)
            .password(&config.password)
            .ssl_mode(ssl_mode)
            .disable_statement_logging();
        if let Some(certificate) = &config.ca_certificate {
            options = options.ssl_ca(certificate);
        }
        let pool = MySqlPoolOptions::new()
            .max_connections(config.max_connections.max(1))
            .acquire_timeout(std::time::Duration::from_secs(10))
            .connect_with(options)
            .await?;
        Ok(Self { pool })
    }

    /// Production external installations are deliberately narrower than the
    /// development driver. MySQL compatibility is not a tested release promise.
    pub async fn verify_supported_server(&self) -> Result<(), StorageError> {
        let version: String = sqlx::query_scalar("SELECT VERSION()")
            .fetch_one(&self.pool)
            .await?;
        if !version.starts_with("11.8.6-") || !version.contains("MariaDB") {
            return Err(StorageError::UnsupportedServer);
        }
        Ok(())
    }

    /// Reserve one immutable security-state key for the installation/key pair.
    /// Only explicit initialization may claim an empty Customer database.
    pub async fn verify_installation(
        &self,
        installation_id: &str,
        key_fingerprint: &[u8; 32],
        initialize: bool,
    ) -> Result<(), StorageError> {
        const KEY: &str = "installation.binding.v1";
        let mut transaction = self.pool.begin().await?;
        if initialize {
            sqlx::query(
                "INSERT INTO security_state(state_key,state_value,revision,mac,updated_at)
                SELECT ?,?,1,?,'1970-01-01T00:00:00.000Z' FROM DUAL
                WHERE NOT EXISTS (SELECT 1 FROM identities)
                ON DUPLICATE KEY UPDATE state_key=state_key",
            )
            .bind(KEY)
            .bind(installation_id.as_bytes())
            .bind(key_fingerprint.as_slice())
            .execute(&mut *transaction)
            .await?;
        }
        let row: Option<(Vec<u8>, Vec<u8>)> = sqlx::query_as(
            "SELECT state_value,mac FROM security_state WHERE state_key=? FOR UPDATE",
        )
        .bind(KEY)
        .fetch_optional(&mut *transaction)
        .await?;
        if !row.is_some_and(|(id, fingerprint)| {
            id == installation_id.as_bytes() && fingerprint == key_fingerprint
        }) {
            return Err(StorageError::InstallationMismatch);
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Exercise real SELECT/UPDATE privileges without persisting any mutation.
    /// Detachment ensures cancellation closes the session instead of returning
    /// an open transaction to the pool. This probe never runs schema migrations.
    pub async fn verify_read_write(&self) -> Result<(), StorageError> {
        self.verify_read_write_for(MARIADB_MIGRATIONS).await
    }

    async fn verify_read_write_for(
        &self,
        migrations: &[EmbeddedMigration],
    ) -> Result<(), StorageError> {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            let mut connection = self.pool.acquire().await?.detach();
            let history: Vec<migration_inspection::AppliedMigration> = sqlx::query_as(
                "SELECT version,name,checksum_sha256 FROM schema_migrations ORDER BY version LIMIT 1025",
            ).fetch_all(&mut connection).await?;
            migration_inspection::verify_applied_history(&history, migrations)?;
            if history.len() < migrations.len() {
                return Err(StorageError::UnsupportedSchema { version: migrations[history.len()].version });
            }
            // Probe one gate at a time so readiness adds no lock-ordering
            // dependency to normal seat, audit, or entity-quota transactions.
            for gate in ["member_seat", "audit_log", "entity_quota"] {
                sqlx::query("START TRANSACTION")
                    .execute(&mut connection)
                    .await?;
                let changed = sqlx::query(
                    "UPDATE transaction_gates SET revision=revision+1 WHERE gate_key=?",
                )
                .bind(gate)
                .execute(&mut connection)
                .await?
                .rows_affected();
                if changed != 1 {
                    return Err(StorageError::Uninitialized);
                }
                sqlx::query("ROLLBACK").execute(&mut connection).await?;
            }
            sqlx::Connection::close(connection).await?;
            Ok(())
        })
        .await
        .map_err(|_| StorageError::ReadinessTimeout)?
    }

    pub fn pool(&self) -> &MySqlPool {
        &self.pool
    }

    pub async fn close(self) {
        self.pool.close().await;
    }

    pub async fn schema_version(&self) -> Result<u32, StorageError> {
        let version = sqlx::query_scalar::<MySql, u32>(
            "SELECT version FROM schema_migrations ORDER BY version DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StorageError::Uninitialized)?;
        Ok(version)
    }

    pub async fn occupied_seats(&self) -> Result<u32, StorageError> {
        let count = sqlx::query_scalar::<MySql, i64>(
            "SELECT count(*) FROM identities WHERE status='active' AND can_consume_model=1",
        )
        .fetch_one(&self.pool)
        .await?;
        u32::try_from(count).map_err(|_| StorageError::DatabaseNotEmpty)
    }

    pub async fn runner_quota_snapshot(&self) -> Result<RunnerQuotaSnapshot, StorageError> {
        let mut transaction = self.pool.begin().await?;
        lock_runner_quota_mariadb(&mut transaction).await?;
        let result = runner_quota_snapshot_mariadb(&mut transaction).await?;
        transaction.commit().await?;
        Ok(result)
    }

    pub async fn seat_registry_snapshot(
        &self,
        key: &str,
    ) -> Result<SeatRegistrySnapshot, StorageError> {
        let state = sqlx::query(
            "SELECT state_key,state_value,revision,mac,updated_at
             FROM security_state WHERE state_key=?",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await?
        .map(security_state_from_mariadb_row)
        .transpose()?;
        let occupied_identity_ids = sqlx::query_scalar::<MySql, String>(
            "SELECT id FROM identities
             WHERE status='active' AND can_consume_model=1
             ORDER BY id ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(SeatRegistrySnapshot {
            state,
            occupied_identity_ids,
        })
    }

    pub async fn create_identity_with_seat_limit(
        &self,
        identity: &IdentityRecord,
        member_seat_limit: u32,
        expected_state: &SecurityStateRecord,
        next_state: &SecurityStateRecord,
        initial_balance: &UserBalanceRecord,
    ) -> Result<IdentityCreateOutcome, StorageError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query_scalar::<MySql, u64>(
            "SELECT revision FROM transaction_gates WHERE gate_key='member_seat' FOR UPDATE",
        )
        .fetch_one(&mut *transaction)
        .await?;
        let current_state = sqlx::query(
            "SELECT state_key,state_value,revision,mac,updated_at
             FROM security_state WHERE state_key=? FOR UPDATE",
        )
        .bind(&expected_state.key)
        .fetch_optional(&mut *transaction)
        .await?
        .map(security_state_from_mariadb_row)
        .transpose()?;
        let current_occupied = sqlx::query_scalar::<MySql, String>(
            "SELECT id FROM identities
             WHERE status='active' AND can_consume_model=1
             ORDER BY id ASC",
        )
        .fetch_all(&mut *transaction)
        .await?;
        if current_state.as_ref() != Some(expected_state)
            || expected_state.value != encode_seat_registry(&current_occupied)
        {
            transaction.rollback().await?;
            return Ok(IdentityCreateOutcome::SecurityStateChanged);
        }
        if identity.status == "active" && identity.can_consume_model {
            let occupied = u32::try_from(current_occupied.len())
                .map_err(|_| StorageError::DatabaseNotEmpty)?;
            if occupied >= member_seat_limit {
                transaction.rollback().await?;
                return Ok(IdentityCreateOutcome::SeatLimitReached);
            }
        }
        sqlx::query(
            "INSERT INTO identities(
               id,email,display_name,password_hash,role,status,can_consume_model,
               password_change_required,revision,integrity_hmac,created_at,updated_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&identity.id)
        .bind(&identity.email)
        .bind(&identity.display_name)
        .bind(&identity.password_hash)
        .bind(&identity.role)
        .bind(&identity.status)
        .bind(identity.can_consume_model)
        .bind(identity.password_change_required)
        .bind(identity.revision)
        .bind(&identity.integrity_hmac)
        .bind(&identity.created_at)
        .bind(&identity.updated_at)
        .execute(&mut *transaction)
        .await
        .map_err(map_mariadb_identity_write_error)?;
        insert_initial_balance_mariadb(&mut transaction, identity, initial_balance).await?;
        let updated_occupied = sqlx::query_scalar::<MySql, String>(
            "SELECT id FROM identities
             WHERE status='active' AND can_consume_model=1
             ORDER BY id ASC",
        )
        .fetch_all(&mut *transaction)
        .await?;
        if next_state.key != expected_state.key
            || next_state.revision != expected_state.revision.saturating_add(1)
            || next_state.value != encode_seat_registry(&updated_occupied)
        {
            transaction.rollback().await?;
            return Ok(IdentityCreateOutcome::SecurityStateChanged);
        }
        let changed = sqlx::query(
            "UPDATE security_state
             SET state_value=?,revision=?,mac=?,updated_at=?
             WHERE state_key=? AND state_value=? AND revision=? AND mac=? AND updated_at=?",
        )
        .bind(&next_state.value)
        .bind(next_state.revision)
        .bind(&next_state.mac)
        .bind(&next_state.updated_at)
        .bind(&expected_state.key)
        .bind(&expected_state.value)
        .bind(expected_state.revision)
        .bind(&expected_state.mac)
        .bind(&expected_state.updated_at)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if changed != 1 {
            transaction.rollback().await?;
            return Ok(IdentityCreateOutcome::SecurityStateChanged);
        }
        transaction.commit().await?;
        Ok(IdentityCreateOutcome::Created)
    }

    pub async fn create_identities_with_seat_limit(
        &self,
        identities: &[IdentityRecord],
        member_seat_limit: u32,
        expected_state: &SecurityStateRecord,
        next_state: &SecurityStateRecord,
        initial_balances: &[UserBalanceRecord],
    ) -> Result<IdentityCreateOutcome, StorageError> {
        match self
            .create_identities_with_seat_limit_inner(
                identities,
                member_seat_limit,
                expected_state,
                next_state,
                initial_balances,
                None,
                &[],
            )
            .await?
        {
            MutationWithAuditOutcome::Mutation(outcome) => Ok(outcome),
            MutationWithAuditOutcome::AuditConflict => Err(StorageError::DatabaseNotEmpty),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_identities_with_seat_limit_and_audit(
        &self,
        identities: &[IdentityRecord],
        member_seat_limit: u32,
        expected_state: &SecurityStateRecord,
        next_state: &SecurityStateRecord,
        initial_balances: &[UserBalanceRecord],
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_events: &[AuditEventRecord],
        model_policies: &[crate::ModelAccessPolicyRecord],
    ) -> Result<MutationWithAuditOutcome<IdentityCreateOutcome>, StorageError> {
        self.create_identities_with_seat_limit_inner(
            identities,
            member_seat_limit,
            expected_state,
            next_state,
            initial_balances,
            Some((expected_audit_sequence, expected_audit_hmac, audit_events)),
            model_policies,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_identities_with_seat_limit_inner(
        &self,
        identities: &[IdentityRecord],
        member_seat_limit: u32,
        expected_state: &SecurityStateRecord,
        next_state: &SecurityStateRecord,
        initial_balances: &[UserBalanceRecord],
        audit: Option<(u64, &str, &[AuditEventRecord])>,
        model_policies: &[crate::ModelAccessPolicyRecord],
    ) -> Result<MutationWithAuditOutcome<IdentityCreateOutcome>, StorageError> {
        if identities.is_empty() || identities.len() != initial_balances.len() {
            return Ok(MutationWithAuditOutcome::Mutation(
                IdentityCreateOutcome::SecurityStateChanged,
            ));
        }
        if !model_policies.is_empty()
            && (model_policies.len() != identities.len()
                || model_policies
                    .iter()
                    .zip(identities)
                    .any(|(policy, identity)| {
                        policy.identity_id != identity.id
                            || policy.mode != "selected"
                            || policy.revision != 0
                            || !policy.grants.is_empty()
                    }))
        {
            return Err(StorageError::MigrationIntegrity);
        }
        let mut transaction = self.pool.begin().await?;
        sqlx::query_scalar::<MySql, u64>(
            "SELECT revision FROM transaction_gates WHERE gate_key='member_seat' FOR UPDATE",
        )
        .fetch_one(&mut *transaction)
        .await?;
        let current_state = sqlx::query(
            "SELECT state_key,state_value,revision,mac,updated_at
             FROM security_state WHERE state_key=? FOR UPDATE",
        )
        .bind(&expected_state.key)
        .fetch_optional(&mut *transaction)
        .await?
        .map(security_state_from_mariadb_row)
        .transpose()?;
        let current_occupied = sqlx::query_scalar::<MySql, String>(
            "SELECT id FROM identities
             WHERE status='active' AND can_consume_model=1
             ORDER BY id ASC",
        )
        .fetch_all(&mut *transaction)
        .await?;
        if current_state.as_ref() != Some(expected_state)
            || expected_state.value != encode_seat_registry(&current_occupied)
        {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                IdentityCreateOutcome::SecurityStateChanged,
            ));
        }
        let added_seats = identities
            .iter()
            .filter(|identity| identity.status == "active" && identity.can_consume_model)
            .count();
        let occupied_after = current_occupied
            .len()
            .checked_add(added_seats)
            .ok_or(StorageError::DatabaseNotEmpty)?;
        if occupied_after > member_seat_limit as usize {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                IdentityCreateOutcome::SeatLimitReached,
            ));
        }
        for (identity, initial_balance) in identities.iter().zip(initial_balances) {
            sqlx::query(
                "INSERT INTO identities(
                   id,email,display_name,password_hash,role,status,can_consume_model,
                   password_change_required,revision,integrity_hmac,created_at,updated_at
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(&identity.id)
            .bind(&identity.email)
            .bind(&identity.display_name)
            .bind(&identity.password_hash)
            .bind(&identity.role)
            .bind(&identity.status)
            .bind(identity.can_consume_model)
            .bind(identity.password_change_required)
            .bind(identity.revision)
            .bind(&identity.integrity_hmac)
            .bind(&identity.created_at)
            .bind(&identity.updated_at)
            .execute(&mut *transaction)
            .await
            .map_err(map_mariadb_identity_write_error)?;
            insert_initial_balance_mariadb(&mut transaction, identity, initial_balance).await?;
        }
        for policy in model_policies {
            sqlx::query(
                "INSERT INTO identity_model_policies(identity_id,mode,revision,grant_set_digest,integrity_hmac,updated_by,updated_at)
                 VALUES(?,?,?,?,?,?,?)",
            )
            .bind(&policy.identity_id).bind(&policy.mode).bind(policy.revision)
            .bind(&policy.grant_set_digest).bind(&policy.integrity_hmac)
            .bind(&policy.updated_by).bind(&policy.updated_at)
            .execute(&mut *transaction).await?;
        }
        let updated_occupied = sqlx::query_scalar::<MySql, String>(
            "SELECT id FROM identities
             WHERE status='active' AND can_consume_model=1
             ORDER BY id ASC",
        )
        .fetch_all(&mut *transaction)
        .await?;
        if next_state.key != expected_state.key
            || expected_state.revision.checked_add(1) != Some(next_state.revision)
            || next_state.value != encode_seat_registry(&updated_occupied)
        {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                IdentityCreateOutcome::SecurityStateChanged,
            ));
        }
        let changed = sqlx::query(
            "UPDATE security_state
             SET state_value=?,revision=?,mac=?,updated_at=?
             WHERE state_key=? AND state_value=? AND revision=? AND mac=? AND updated_at=?",
        )
        .bind(&next_state.value)
        .bind(next_state.revision)
        .bind(&next_state.mac)
        .bind(&next_state.updated_at)
        .bind(&expected_state.key)
        .bind(&expected_state.value)
        .bind(expected_state.revision)
        .bind(&expected_state.mac)
        .bind(&expected_state.updated_at)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if changed != 1 {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                IdentityCreateOutcome::SecurityStateChanged,
            ));
        }
        if let Some((expected_sequence, expected_hmac, events)) = audit
            && !append_audit_events_mariadb(
                &mut transaction,
                expected_sequence,
                expected_hmac,
                events,
            )
            .await?
        {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(MutationWithAuditOutcome::Mutation(
            IdentityCreateOutcome::Created,
        ))
    }

    pub async fn initialize_first_owner(
        &self,
        identity: &IdentityRecord,
        initial_state: &SecurityStateRecord,
        initial_balance: &UserBalanceRecord,
    ) -> Result<IdentityInitializeOutcome, StorageError> {
        match self
            .initialize_first_owner_inner(identity, initial_state, initial_balance, None)
            .await?
        {
            MutationWithAuditOutcome::Mutation(outcome) => Ok(outcome),
            MutationWithAuditOutcome::AuditConflict => Err(StorageError::DatabaseNotEmpty),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn initialize_first_owner_and_audit(
        &self,
        identity: &IdentityRecord,
        initial_state: &SecurityStateRecord,
        initial_balance: &UserBalanceRecord,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<MutationWithAuditOutcome<IdentityInitializeOutcome>, StorageError> {
        self.initialize_first_owner_inner(
            identity,
            initial_state,
            initial_balance,
            Some((
                expected_audit_sequence,
                expected_audit_hmac,
                std::slice::from_ref(audit_event),
            )),
        )
        .await
    }

    async fn initialize_first_owner_inner(
        &self,
        identity: &IdentityRecord,
        initial_state: &SecurityStateRecord,
        initial_balance: &UserBalanceRecord,
        audit: Option<(u64, &str, &[AuditEventRecord])>,
    ) -> Result<MutationWithAuditOutcome<IdentityInitializeOutcome>, StorageError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query_scalar::<MySql, u64>(
            "SELECT revision FROM transaction_gates WHERE gate_key='member_seat' FOR UPDATE",
        )
        .fetch_one(&mut *transaction)
        .await?;
        let identity_count = sqlx::query_scalar::<MySql, i64>("SELECT count(*) FROM identities")
            .fetch_one(&mut *transaction)
            .await?;
        if identity_count != 0 {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                IdentityInitializeOutcome::AlreadyInitialized,
            ));
        }
        let state_count = sqlx::query_scalar::<MySql, i64>(
            "SELECT count(*) FROM security_state WHERE state_key=?",
        )
        .bind(&initial_state.key)
        .fetch_one(&mut *transaction)
        .await?;
        if state_count != 0 {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                IdentityInitializeOutcome::SecurityStateChanged,
            ));
        }
        sqlx::query(
            "INSERT INTO identities(
               id,email,display_name,password_hash,role,status,can_consume_model,
               password_change_required,revision,integrity_hmac,created_at,updated_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&identity.id)
        .bind(&identity.email)
        .bind(&identity.display_name)
        .bind(&identity.password_hash)
        .bind(&identity.role)
        .bind(&identity.status)
        .bind(identity.can_consume_model)
        .bind(identity.password_change_required)
        .bind(identity.revision)
        .bind(&identity.integrity_hmac)
        .bind(&identity.created_at)
        .bind(&identity.updated_at)
        .execute(&mut *transaction)
        .await?;
        insert_initial_balance_mariadb(&mut transaction, identity, initial_balance).await?;
        let occupied = sqlx::query_scalar::<MySql, String>(
            "SELECT id FROM identities
             WHERE status='active' AND can_consume_model=1
             ORDER BY id ASC",
        )
        .fetch_all(&mut *transaction)
        .await?;
        if initial_state.revision != 0 || initial_state.value != encode_seat_registry(&occupied) {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                IdentityInitializeOutcome::SecurityStateChanged,
            ));
        }
        sqlx::query(
            "INSERT INTO security_state(state_key,state_value,revision,mac,updated_at)
             VALUES(?,?,?,?,?)",
        )
        .bind(&initial_state.key)
        .bind(&initial_state.value)
        .bind(initial_state.revision)
        .bind(&initial_state.mac)
        .bind(&initial_state.updated_at)
        .execute(&mut *transaction)
        .await?;
        if let Some((expected_sequence, expected_hmac, events)) = audit
            && !append_audit_events_mariadb(
                &mut transaction,
                expected_sequence,
                expected_hmac,
                events,
            )
            .await?
        {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(MutationWithAuditOutcome::Mutation(
            IdentityInitializeOutcome::Created,
        ))
    }

    pub async fn identity_by_email(
        &self,
        email: &str,
    ) -> Result<Option<IdentityRecord>, StorageError> {
        sqlx::query(
            "SELECT id,email,display_name,password_hash,role,status,can_consume_model,
                    password_change_required,revision,integrity_hmac,created_at,updated_at
             FROM identities WHERE email=? AND status!='deleted'",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?
        .map(identity_from_mariadb_row)
        .transpose()
        .map_err(StorageError::from)
    }

    pub async fn identity_by_id(&self, id: &str) -> Result<Option<IdentityRecord>, StorageError> {
        sqlx::query(
            "SELECT id,email,display_name,password_hash,role,status,can_consume_model,
                    password_change_required,revision,integrity_hmac,created_at,updated_at
             FROM identities WHERE id=?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .map(identity_from_mariadb_row)
        .transpose()
        .map_err(StorageError::from)
    }

    pub async fn list_member_identities(&self) -> Result<Vec<IdentityRecord>, StorageError> {
        sqlx::query(
            "SELECT id,email,display_name,password_hash,role,status,can_consume_model,
                    password_change_required,revision,integrity_hmac,created_at,updated_at
             FROM identities WHERE role='member' AND status!='deleted' ORDER BY created_at,id",
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(identity_from_mariadb_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(StorageError::from)
    }

    pub async fn list_license_identities(&self) -> Result<Vec<IdentityRecord>, StorageError> {
        sqlx::query(
            "SELECT id,email,display_name,password_hash,role,status,can_consume_model,
                    password_change_required,revision,integrity_hmac,created_at,updated_at
             FROM identities ORDER BY created_at,id",
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(identity_from_mariadb_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(StorageError::from)
    }

    pub async fn update_member_identity_status(
        &self,
        expected: &IdentityRecord,
        next: &IdentityRecord,
        expected_state: &SecurityStateRecord,
        next_state: &SecurityStateRecord,
    ) -> Result<IdentityStatusUpdateOutcome, StorageError> {
        match self
            .update_member_identity_status_inner(expected, next, expected_state, next_state, None)
            .await?
        {
            MutationWithAuditOutcome::Mutation(outcome) => Ok(outcome),
            MutationWithAuditOutcome::AuditConflict => Err(StorageError::DatabaseNotEmpty),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_member_identity_status_and_audit(
        &self,
        expected: &IdentityRecord,
        next: &IdentityRecord,
        expected_state: &SecurityStateRecord,
        next_state: &SecurityStateRecord,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<MutationWithAuditOutcome<IdentityStatusUpdateOutcome>, StorageError> {
        self.update_member_identity_status_inner(
            expected,
            next,
            expected_state,
            next_state,
            Some((
                expected_audit_sequence,
                expected_audit_hmac,
                std::slice::from_ref(audit_event),
            )),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn update_member_identity_status_inner(
        &self,
        expected: &IdentityRecord,
        next: &IdentityRecord,
        expected_state: &SecurityStateRecord,
        next_state: &SecurityStateRecord,
        audit: Option<(u64, &str, &[AuditEventRecord])>,
    ) -> Result<MutationWithAuditOutcome<IdentityStatusUpdateOutcome>, StorageError> {
        if !valid_identity_status_transition(expected, next) {
            return Ok(MutationWithAuditOutcome::Mutation(
                IdentityStatusUpdateOutcome::Conflict,
            ));
        }
        let mut transaction = self.pool.begin().await?;
        sqlx::query_scalar::<MySql, u64>(
            "SELECT revision FROM transaction_gates WHERE gate_key='member_seat' FOR UPDATE",
        )
        .fetch_one(&mut *transaction)
        .await?;
        let current_state = sqlx::query(
            "SELECT state_key,state_value,revision,mac,updated_at
             FROM security_state WHERE state_key=? FOR UPDATE",
        )
        .bind(&expected_state.key)
        .fetch_optional(&mut *transaction)
        .await?
        .map(security_state_from_mariadb_row)
        .transpose()?;
        let current_occupied = sqlx::query_scalar::<MySql, String>(
            "SELECT id FROM identities
             WHERE status='active' AND can_consume_model=1
             ORDER BY id ASC",
        )
        .fetch_all(&mut *transaction)
        .await?;
        if current_state.as_ref() != Some(expected_state)
            || expected_state.value != encode_seat_registry(&current_occupied)
        {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                IdentityStatusUpdateOutcome::SecurityStateChanged,
            ));
        }
        let changed = sqlx::query(
            "UPDATE identities SET status=?,revision=?,integrity_hmac=?,updated_at=?
             WHERE id=? AND role='member' AND status=? AND revision=? AND integrity_hmac=?",
        )
        .bind(&next.status)
        .bind(next.revision)
        .bind(&next.integrity_hmac)
        .bind(&next.updated_at)
        .bind(&expected.id)
        .bind(&expected.status)
        .bind(expected.revision)
        .bind(&expected.integrity_hmac)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if changed != 1 {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                IdentityStatusUpdateOutcome::Conflict,
            ));
        }
        if next.status != "active" {
            sqlx::query("DELETE FROM sessions WHERE identity_id=?")
                .bind(&next.id)
                .execute(&mut *transaction)
                .await?;
        }
        let updated_occupied = sqlx::query_scalar::<MySql, String>(
            "SELECT id FROM identities
             WHERE status='active' AND can_consume_model=1
             ORDER BY id ASC",
        )
        .fetch_all(&mut *transaction)
        .await?;
        if next_state.key != expected_state.key
            || next_state.revision != expected_state.revision.saturating_add(1)
            || next_state.value != encode_seat_registry(&updated_occupied)
        {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                IdentityStatusUpdateOutcome::SecurityStateChanged,
            ));
        }
        let state_changed = sqlx::query(
            "UPDATE security_state
             SET state_value=?,revision=?,mac=?,updated_at=?
             WHERE state_key=? AND state_value=? AND revision=? AND mac=? AND updated_at=?",
        )
        .bind(&next_state.value)
        .bind(next_state.revision)
        .bind(&next_state.mac)
        .bind(&next_state.updated_at)
        .bind(&expected_state.key)
        .bind(&expected_state.value)
        .bind(expected_state.revision)
        .bind(&expected_state.mac)
        .bind(&expected_state.updated_at)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if state_changed != 1 {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                IdentityStatusUpdateOutcome::SecurityStateChanged,
            ));
        }
        if let Some((expected_sequence, expected_hmac, events)) = audit
            && !append_audit_events_mariadb(
                &mut transaction,
                expected_sequence,
                expected_hmac,
                events,
            )
            .await?
        {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(MutationWithAuditOutcome::Mutation(
            IdentityStatusUpdateOutcome::Updated,
        ))
    }

    pub async fn update_identity_password(
        &self,
        identity_id: &str,
        expected_revision: u32,
        password_hash: &str,
        password_change_required: bool,
        integrity_hmac: &str,
        updated_at: &str,
    ) -> Result<bool, StorageError> {
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE identities
             SET password_hash=?,password_change_required=?,revision=revision+1,
                 integrity_hmac=?,updated_at=?
             WHERE id=? AND revision=? AND status='active'",
        )
        .bind(password_hash)
        .bind(password_change_required)
        .bind(integrity_hmac)
        .bind(updated_at)
        .bind(identity_id)
        .bind(expected_revision)
        .execute(&mut *transaction)
        .await?;
        let changed = result.rows_affected() == 1;
        if changed {
            sqlx::query("DELETE FROM sessions WHERE identity_id=?")
                .bind(identity_id)
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(changed)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_identity_password_and_audit(
        &self,
        identity_id: &str,
        expected_revision: u32,
        password_hash: &str,
        password_change_required: bool,
        integrity_hmac: &str,
        updated_at: &str,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let mut transaction = self.pool.begin().await?;
        let changed = sqlx::query(
            "UPDATE identities
             SET password_hash=?,password_change_required=?,revision=revision+1,
                 integrity_hmac=?,updated_at=?
             WHERE id=? AND revision=? AND status='active'",
        )
        .bind(password_hash)
        .bind(password_change_required)
        .bind(integrity_hmac)
        .bind(updated_at)
        .bind(identity_id)
        .bind(expected_revision)
        .execute(&mut *transaction)
        .await?
        .rows_affected()
            == 1;
        if !changed {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        sqlx::query("DELETE FROM sessions WHERE identity_id=?")
            .bind(identity_id)
            .execute(&mut *transaction)
            .await?;
        if !append_audit_events_mariadb(
            &mut transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )
        .await?
        {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(AuditedMutationOutcome::Applied)
    }

    /// Untrusted discovery hints only; signed snapshots authorize recovery.
    pub async fn active_quota_identity_page(
        &self,
        after: &str,
        limit: u32,
    ) -> Result<Vec<String>, StorageError> {
        Ok(sqlx::query_scalar(
            "SELECT identity_id FROM user_balances
             WHERE reserved_tokens>0 AND identity_id>? ORDER BY identity_id ASC LIMIT ?",
        )
        .bind(after)
        .bind(limit.clamp(1, 128))
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn quota_state_snapshot(
        &self,
        identity_id: &str,
    ) -> Result<Option<QuotaStateSnapshot>, StorageError> {
        // Every committed quota mutation locks/updates this balance in the same
        // transaction, even when its ledger writes precede the balance update.
        // Keep a shared lock and one connection across the complete read.
        let mut transaction = self.pool.begin().await?;
        let Some(balance) = sqlx::query(
            "SELECT identity_id,balance_tokens,reserved_tokens,granted_tokens,
                    consumed_tokens,request_count,raw_tokens,billed_tokens,uncached_input,
                    cached_input,cache_write,output_tokens,revision,last_ledger_hmac,
                    integrity_hmac,updated_at
             FROM user_balances WHERE identity_id=? LOCK IN SHARE MODE",
        )
        .bind(identity_id)
        .fetch_optional(&mut *transaction)
        .await?
        .map(balance_from_mariadb_row)
        .transpose()?
        else {
            return Ok(None);
        };
        let active_reservations = sqlx::query(
            "SELECT id,identity_id,api_key_id,request_id,reserved_tokens,status,revision,
                    integrity_hmac,created_at,expires_at,settled_at
             FROM quota_reservations
             WHERE identity_id=? AND status='active'
             ORDER BY id ASC",
        )
        .bind(identity_id)
        .fetch_all(&mut *transaction)
        .await?
        .into_iter()
        .map(quota_reservation_from_mariadb_row)
        .collect::<Result<Vec<_>, _>>()?;
        let ledger_entries = sqlx::query(
            "SELECT id,identity_id,kind,amount_tokens,uncached_input,cached_input,cache_write,
                    output_tokens,uncovered_tokens,raw_tokens,billed_tokens,multiplier_micros,
                    reference_id,client_request_id,description,protocol,model,requested_model,processing_tier,
                    reasoning_effort,api_key_id,runner_id,
                    previous_entry_hmac,integrity_hmac,created_at
             FROM ledger_entries WHERE identity_id=? ORDER BY created_at ASC,id ASC",
        )
        .bind(identity_id)
        .fetch_all(&mut *transaction)
        .await?
        .into_iter()
        .map(quota_ledger_from_mariadb_row)
        .collect::<Result<Vec<_>, _>>()?;
        transaction.commit().await?;
        Ok(Some(QuotaStateSnapshot {
            balance,
            active_reservations,
            ledger_entries,
        }))
    }

    pub async fn reserve_quota(
        &self,
        subject: &AuthorizedApiKey,
        expected_balance: &UserBalanceRecord,
        next_balance: &UserBalanceRecord,
        reservation: &QuotaReservationRecord,
    ) -> Result<QuotaMutationOutcome, StorageError> {
        if !valid_reserve_transition(expected_balance, next_balance, reservation) {
            return Ok(QuotaMutationOutcome::Conflict);
        }
        let mut transaction = self.pool.begin().await?;
        let current_subject = sqlx::query(
            "SELECT k.id,k.identity_id,k.name,k.key_hash,k.key_prefix,k.status,k.revision,
                    k.integrity_hmac,k.created_at,k.last_used_at,
                    i.id AS identity_id_value,i.email,i.display_name,i.password_hash,i.role,
                    i.status AS identity_status,i.can_consume_model,i.password_change_required,
                    i.revision AS identity_revision,i.integrity_hmac AS identity_integrity_hmac,
                    i.created_at AS identity_created_at,i.updated_at AS identity_updated_at
             FROM api_keys k JOIN identities i ON i.id=k.identity_id
             WHERE k.key_hash=? AND k.status='active' AND i.status='active' FOR UPDATE",
        )
        .bind(&subject.api_key.key_hash)
        .fetch_optional(&mut *transaction)
        .await?
        .map(authorized_api_key_from_mariadb_row)
        .transpose()?;
        if !valid_reservation_subject(current_subject.as_ref(), subject, reservation) {
            transaction.rollback().await?;
            return Ok(QuotaMutationOutcome::Conflict);
        }
        let current = sqlx::query(
            "SELECT identity_id,balance_tokens,reserved_tokens,granted_tokens,
                    consumed_tokens,request_count,raw_tokens,billed_tokens,uncached_input,
                    cached_input,cache_write,output_tokens,revision,last_ledger_hmac,
                    integrity_hmac,updated_at
             FROM user_balances WHERE identity_id=? FOR UPDATE",
        )
        .bind(&expected_balance.identity_id)
        .fetch_optional(&mut *transaction)
        .await?
        .map(balance_from_mariadb_row)
        .transpose()?;
        if current.as_ref() != Some(expected_balance) {
            transaction.rollback().await?;
            return Ok(QuotaMutationOutcome::Conflict);
        }
        if sqlx::query("SELECT 1 FROM quota_reservations WHERE request_id=? FOR UPDATE")
            .bind(&reservation.request_id)
            .fetch_optional(&mut *transaction)
            .await?
            .is_some()
        {
            transaction.rollback().await?;
            return Ok(QuotaMutationOutcome::DuplicateRequest);
        }
        if expected_balance
            .balance_tokens
            .checked_sub(expected_balance.reserved_tokens)
            .is_none_or(|available| available < reservation.reserved_tokens)
        {
            transaction.rollback().await?;
            return Ok(QuotaMutationOutcome::Insufficient);
        }
        sqlx::query(
            "INSERT INTO quota_reservations(
               id,identity_id,api_key_id,request_id,reserved_tokens,status,revision,
               integrity_hmac,created_at,expires_at,settled_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&reservation.id)
        .bind(&reservation.identity_id)
        .bind(&reservation.api_key_id)
        .bind(&reservation.request_id)
        .bind(reservation.reserved_tokens)
        .bind(&reservation.status)
        .bind(reservation.revision)
        .bind(&reservation.integrity_hmac)
        .bind(&reservation.created_at)
        .bind(&reservation.expires_at)
        .bind(&reservation.settled_at)
        .execute(&mut *transaction)
        .await?;
        if !update_balance_mariadb(&mut transaction, expected_balance, next_balance).await? {
            transaction.rollback().await?;
            return Ok(QuotaMutationOutcome::Conflict);
        }
        transaction.commit().await?;
        Ok(QuotaMutationOutcome::Applied)
    }

    pub async fn grant_quota(
        &self,
        expected_balance: &UserBalanceRecord,
        next_balance: &UserBalanceRecord,
        ledger: &QuotaLedgerEntry,
    ) -> Result<QuotaMutationOutcome, StorageError> {
        match self
            .grant_quota_inner(expected_balance, next_balance, ledger, None)
            .await?
        {
            MutationWithAuditOutcome::Mutation(outcome) => Ok(outcome),
            MutationWithAuditOutcome::AuditConflict => Err(StorageError::DatabaseNotEmpty),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn grant_quota_and_audit(
        &self,
        expected_balance: &UserBalanceRecord,
        next_balance: &UserBalanceRecord,
        ledger: &QuotaLedgerEntry,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<MutationWithAuditOutcome<QuotaMutationOutcome>, StorageError> {
        self.grant_quota_inner(
            expected_balance,
            next_balance,
            ledger,
            Some((
                expected_audit_sequence,
                expected_audit_hmac,
                std::slice::from_ref(audit_event),
            )),
        )
        .await
    }

    pub async fn grant_quota_batch_and_audit(
        &self,
        token: Option<&QuotaBatchTokenWrite>,
        images: &[QuotaBatchImageWrite],
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<MutationWithAuditOutcome<QuotaMutationOutcome>, StorageError> {
        if token.is_none() && images.is_empty() {
            return Ok(MutationWithAuditOutcome::Mutation(
                QuotaMutationOutcome::Conflict,
            ));
        }
        if token.is_some_and(|write| {
            !valid_grant_transition(&write.expected_balance, &write.next_balance, &write.ledger)
        }) || images.iter().any(|write| {
            write.ledger.kind != "adjust"
                || !valid_image_balance_transition(
                    write.expected_balance.as_ref(),
                    &write.next_balance,
                    &write.ledger,
                )
        }) {
            return Ok(MutationWithAuditOutcome::Mutation(
                QuotaMutationOutcome::Conflict,
            ));
        }
        let identity_id = token
            .map(|write| write.next_balance.identity_id.as_str())
            .or_else(|| {
                images
                    .first()
                    .map(|write| write.next_balance.identity_id.as_str())
            })
            .ok_or(StorageError::MigrationIntegrity)?;
        if images
            .iter()
            .any(|write| write.next_balance.identity_id != identity_id)
        {
            return Ok(MutationWithAuditOutcome::Mutation(
                QuotaMutationOutcome::Conflict,
            ));
        }
        let mut model_ids = std::collections::HashSet::new();
        if images
            .iter()
            .any(|write| !model_ids.insert(&write.next_balance.public_model_id))
        {
            return Ok(MutationWithAuditOutcome::Mutation(
                QuotaMutationOutcome::Conflict,
            ));
        }
        let mut transaction = self.pool.begin().await?;
        if let Some(write) = token {
            let current = sqlx::query(
                "SELECT identity_id,balance_tokens,reserved_tokens,granted_tokens,
                        consumed_tokens,request_count,raw_tokens,billed_tokens,uncached_input,
                        cached_input,cache_write,output_tokens,revision,last_ledger_hmac,
                        integrity_hmac,updated_at FROM user_balances WHERE identity_id=? FOR UPDATE",
            ).bind(&write.expected_balance.identity_id).fetch_optional(&mut *transaction).await?
                .map(balance_from_mariadb_row).transpose()?;
            if current.as_ref() != Some(&write.expected_balance) {
                return Ok(MutationWithAuditOutcome::Mutation(
                    QuotaMutationOutcome::Conflict,
                ));
            }
            if let Err(error) = insert_ledger_mariadb(&mut transaction, &write.ledger).await {
                if is_mariadb_duplicate(&error) {
                    return Ok(MutationWithAuditOutcome::Mutation(
                        QuotaMutationOutcome::DuplicateRequest,
                    ));
                }
                return Err(StorageError::MariaDb(error));
            }
            if !update_balance_mariadb(
                &mut transaction,
                &write.expected_balance,
                &write.next_balance,
            )
            .await?
            {
                return Ok(MutationWithAuditOutcome::Mutation(
                    QuotaMutationOutcome::Conflict,
                ));
            }
        }
        for write in images {
            let duplicate: Option<String> =
                sqlx::query_scalar("SELECT id FROM image_quota_ledger WHERE id=? FOR UPDATE")
                    .bind(&write.ledger.id)
                    .fetch_optional(&mut *transaction)
                    .await?;
            if duplicate.is_some() {
                return Ok(MutationWithAuditOutcome::Mutation(
                    QuotaMutationOutcome::DuplicateRequest,
                ));
            }
            let unit: Option<String> = sqlx::query_scalar(
                "SELECT u.quota_unit FROM model_quota_units u JOIN models m ON m.id=u.model_id
                 WHERE u.model_id=? AND m.enabled=1",
            )
            .bind(&write.next_balance.public_model_id)
            .fetch_optional(&mut *transaction)
            .await?;
            if unit.as_deref() != Some("image") {
                return Ok(MutationWithAuditOutcome::Mutation(
                    QuotaMutationOutcome::Conflict,
                ));
            }
            let current = sqlx::query(
                "SELECT revision,integrity_hmac FROM member_image_balances
                 WHERE identity_id=? AND public_model_id=? FOR UPDATE",
            )
            .bind(&write.next_balance.identity_id)
            .bind(&write.next_balance.public_model_id)
            .fetch_optional(&mut *transaction)
            .await?
            .map(|row| {
                Ok::<_, sqlx::Error>((
                    checked_i64(row.try_get::<u64, _>("revision")?)?,
                    row.try_get::<String, _>("integrity_hmac")?,
                ))
            })
            .transpose()?;
            if current
                != write
                    .expected_balance
                    .as_ref()
                    .map(|value| (value.revision, value.integrity_hmac.clone()))
            {
                return Ok(MutationWithAuditOutcome::Mutation(
                    QuotaMutationOutcome::Conflict,
                ));
            }
            if let Some(expected) = &write.expected_balance {
                let changed = sqlx::query(
                    "UPDATE member_image_balances SET available_images=?,reserved_images=?,consumed_images=?,
                     revision=?,last_ledger_hmac=?,integrity_hmac=? WHERE identity_id=? AND public_model_id=?
                     AND revision=? AND integrity_hmac=?",
                ).bind(write.next_balance.available_images).bind(write.next_balance.reserved_images)
                    .bind(write.next_balance.consumed_images).bind(write.next_balance.revision)
                    .bind(&write.next_balance.last_ledger_hmac).bind(&write.next_balance.integrity_hmac)
                    .bind(&write.next_balance.identity_id).bind(&write.next_balance.public_model_id)
                    .bind(expected.revision).bind(&expected.integrity_hmac)
                    .execute(&mut *transaction).await?.rows_affected();
                if changed != 1 {
                    return Ok(MutationWithAuditOutcome::Mutation(
                        QuotaMutationOutcome::Conflict,
                    ));
                }
            } else {
                let inserted = sqlx::query(
                    "INSERT INTO member_image_balances(identity_id,public_model_id,available_images,
                     reserved_images,consumed_images,revision,last_ledger_hmac,integrity_hmac)
                     VALUES(?,?,?,?,?,?,?,?)",
                ).bind(&write.next_balance.identity_id).bind(&write.next_balance.public_model_id)
                    .bind(write.next_balance.available_images).bind(write.next_balance.reserved_images)
                    .bind(write.next_balance.consumed_images).bind(write.next_balance.revision)
                    .bind(&write.next_balance.last_ledger_hmac).bind(&write.next_balance.integrity_hmac)
                    .execute(&mut *transaction).await;
                if let Err(error) = inserted {
                    if is_mariadb_duplicate(&error) {
                        return Ok(MutationWithAuditOutcome::Mutation(
                            QuotaMutationOutcome::Conflict,
                        ));
                    }
                    return Err(StorageError::MariaDb(error));
                }
            }
            if let Err(error) = sqlx::query(
                "INSERT INTO image_quota_ledger(id,identity_id,public_model_id,reservation_id,child_id,
                 kind,amount_images,produced_images,delivery_state,actor,reason,previous_hmac,
                 integrity_hmac,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            ).bind(&write.ledger.id).bind(&write.ledger.identity_id).bind(&write.ledger.public_model_id)
                .bind(&write.ledger.reservation_id).bind(&write.ledger.child_id).bind(&write.ledger.kind)
                .bind(write.ledger.amount_images).bind(write.ledger.produced_images)
                .bind(&write.ledger.delivery_state).bind(&write.ledger.actor).bind(&write.ledger.reason)
                .bind(&write.ledger.previous_hmac).bind(&write.ledger.integrity_hmac)
                .bind(&write.ledger.created_at).execute(&mut *transaction).await {
                if is_mariadb_duplicate(&error) {
                    return Ok(MutationWithAuditOutcome::Mutation(QuotaMutationOutcome::DuplicateRequest));
                }
                return Err(StorageError::MariaDb(error));
            }
        }
        if !append_audit_events_mariadb(
            &mut transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )
        .await?
        {
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(MutationWithAuditOutcome::Mutation(
            QuotaMutationOutcome::Applied,
        ))
    }

    async fn grant_quota_inner(
        &self,
        expected_balance: &UserBalanceRecord,
        next_balance: &UserBalanceRecord,
        ledger: &QuotaLedgerEntry,
        audit: Option<(u64, &str, &[AuditEventRecord])>,
    ) -> Result<MutationWithAuditOutcome<QuotaMutationOutcome>, StorageError> {
        if !valid_grant_transition(expected_balance, next_balance, ledger) {
            return Ok(MutationWithAuditOutcome::Mutation(
                QuotaMutationOutcome::Conflict,
            ));
        }
        let mut transaction = self.pool.begin().await?;
        let current = sqlx::query(
            "SELECT identity_id,balance_tokens,reserved_tokens,granted_tokens,
                    consumed_tokens,request_count,raw_tokens,billed_tokens,uncached_input,
                    cached_input,cache_write,output_tokens,revision,last_ledger_hmac,
                    integrity_hmac,updated_at
             FROM user_balances WHERE identity_id=? FOR UPDATE",
        )
        .bind(&expected_balance.identity_id)
        .fetch_optional(&mut *transaction)
        .await?
        .map(balance_from_mariadb_row)
        .transpose()?;
        if current.as_ref() != Some(expected_balance) {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                QuotaMutationOutcome::Conflict,
            ));
        }
        if let Err(error) = insert_ledger_mariadb(&mut transaction, ledger).await {
            if is_mariadb_duplicate(&error) {
                transaction.rollback().await?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    QuotaMutationOutcome::DuplicateRequest,
                ));
            }
            return Err(StorageError::MariaDb(error));
        }
        if !update_balance_mariadb(&mut transaction, expected_balance, next_balance).await? {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                QuotaMutationOutcome::Conflict,
            ));
        }
        if let Some((expected_sequence, expected_hmac, events)) = audit
            && !append_audit_events_mariadb(
                &mut transaction,
                expected_sequence,
                expected_hmac,
                events,
            )
            .await?
        {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(MutationWithAuditOutcome::Mutation(
            QuotaMutationOutcome::Applied,
        ))
    }

    pub async fn insert_quota_request(
        &self,
        request: &QuotaRequestRecord,
    ) -> Result<QuotaRequestInsertOutcome, StorageError> {
        let result = sqlx::query(
            "INSERT INTO quota_requests(
               id,identity_id,amount_nanos,reason,status,review_note,reviewed_by,reviewed_at,
               revision,integrity_hmac,created_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&request.id)
        .bind(&request.identity_id)
        .bind(request.amount_nanos)
        .bind(&request.reason)
        .bind(&request.status)
        .bind(&request.review_note)
        .bind(&request.reviewed_by)
        .bind(&request.reviewed_at)
        .bind(request.revision)
        .bind(&request.integrity_hmac)
        .bind(&request.created_at)
        .execute(&self.pool)
        .await;
        match result {
            Ok(_) => Ok(QuotaRequestInsertOutcome::Inserted),
            Err(error) if is_mariadb_duplicate(&error) => {
                Ok(QuotaRequestInsertOutcome::PendingExists)
            }
            Err(error) => Err(StorageError::MariaDb(error)),
        }
    }

    pub async fn insert_quota_request_and_audit(
        &self,
        request: &QuotaRequestRecord,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<MutationWithAuditOutcome<QuotaRequestInsertOutcome>, StorageError> {
        let mut transaction = self.pool.begin().await?;
        let inserted = sqlx::query(
            "INSERT INTO quota_requests(
               id,identity_id,amount_nanos,reason,status,review_note,reviewed_by,reviewed_at,
               revision,integrity_hmac,created_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&request.id)
        .bind(&request.identity_id)
        .bind(request.amount_nanos)
        .bind(&request.reason)
        .bind(&request.status)
        .bind(&request.review_note)
        .bind(&request.reviewed_by)
        .bind(&request.reviewed_at)
        .bind(request.revision)
        .bind(&request.integrity_hmac)
        .bind(&request.created_at)
        .execute(&mut *transaction)
        .await;
        match inserted {
            Ok(result) if result.rows_affected() == 1 => {}
            Ok(_) => {
                transaction.rollback().await?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    QuotaRequestInsertOutcome::PendingExists,
                ));
            }
            Err(error) if is_mariadb_duplicate(&error) => {
                transaction.rollback().await?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    QuotaRequestInsertOutcome::PendingExists,
                ));
            }
            Err(error) => return Err(StorageError::MariaDb(error)),
        }
        if !append_audit_events_mariadb(
            &mut transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )
        .await?
        {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(MutationWithAuditOutcome::Mutation(
            QuotaRequestInsertOutcome::Inserted,
        ))
    }

    pub async fn quota_request_by_id(
        &self,
        request_id: &str,
    ) -> Result<Option<QuotaRequestRecord>, StorageError> {
        sqlx::query(
            "SELECT id,identity_id,amount_nanos,reason,status,review_note,
                    reviewed_by,reviewed_at,revision,integrity_hmac,created_at
             FROM quota_requests WHERE id=?",
        )
        .bind(request_id)
        .fetch_optional(&self.pool)
        .await?
        .map(quota_request_from_mariadb_row)
        .transpose()
        .map_err(StorageError::from)
    }

    pub async fn list_quota_requests(
        &self,
        identity_id: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<QuotaRequestRecord>, StorageError> {
        let rows = match (identity_id, status) {
            (Some(identity_id), Some(status)) => {
                sqlx::query(
                    "SELECT id,identity_id,amount_nanos,reason,status,review_note,
                        reviewed_by,reviewed_at,revision,integrity_hmac,created_at
                 FROM quota_requests WHERE identity_id=? AND status=? ORDER BY created_at DESC,id",
                )
                .bind(identity_id)
                .bind(status)
                .fetch_all(&self.pool)
                .await?
            }
            (Some(identity_id), None) => {
                sqlx::query(
                    "SELECT id,identity_id,amount_nanos,reason,status,review_note,
                        reviewed_by,reviewed_at,revision,integrity_hmac,created_at
                 FROM quota_requests WHERE identity_id=? ORDER BY created_at DESC,id",
                )
                .bind(identity_id)
                .fetch_all(&self.pool)
                .await?
            }
            (None, Some(status)) => {
                sqlx::query(
                    "SELECT id,identity_id,amount_nanos,reason,status,review_note,
                        reviewed_by,reviewed_at,revision,integrity_hmac,created_at
                 FROM quota_requests WHERE status=? ORDER BY created_at DESC,id",
                )
                .bind(status)
                .fetch_all(&self.pool)
                .await?
            }
            (None, None) => {
                sqlx::query(
                    "SELECT id,identity_id,amount_nanos,reason,status,review_note,
                        reviewed_by,reviewed_at,revision,integrity_hmac,created_at
                 FROM quota_requests ORDER BY created_at DESC,id",
                )
                .fetch_all(&self.pool)
                .await?
            }
        };
        rows.into_iter()
            .map(quota_request_from_mariadb_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub async fn change_pending_quota_request_and_audit(
        &self,
        expected_request: &QuotaRequestRecord,
        next_request: Option<&QuotaRequestRecord>,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let valid_next = next_request.is_none_or(|next| {
            next.id == expected_request.id
                && next.identity_id == expected_request.identity_id
                && next.status == "pending"
                && next.review_note.is_empty()
                && next.reviewed_by.is_none()
                && next.reviewed_at.is_none()
                && next.created_at == expected_request.created_at
                && expected_request
                    .revision
                    .checked_add(1)
                    .is_some_and(|revision| next.revision == revision)
                && !next.integrity_hmac.is_empty()
        });
        if expected_request.status != "pending" || !valid_next {
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        let mut transaction = self.pool.begin().await?;
        let changed = if let Some(next) = next_request {
            sqlx::query(
                "UPDATE quota_requests
                 SET amount_nanos=?,reason=?,revision=?,integrity_hmac=?
                 WHERE id=? AND identity_id=? AND status='pending' AND revision=? AND integrity_hmac=?",
            )
            .bind(next.amount_nanos)
            .bind(&next.reason)
            .bind(next.revision)
            .bind(&next.integrity_hmac)
            .bind(&expected_request.id)
            .bind(&expected_request.identity_id)
            .bind(expected_request.revision)
            .bind(&expected_request.integrity_hmac)
            .execute(&mut *transaction)
            .await?
            .rows_affected()
        } else {
            sqlx::query(
                "DELETE FROM quota_requests
                 WHERE id=? AND identity_id=? AND status='pending' AND revision=? AND integrity_hmac=?",
            )
            .bind(&expected_request.id)
            .bind(&expected_request.identity_id)
            .bind(expected_request.revision)
            .bind(&expected_request.integrity_hmac)
            .execute(&mut *transaction)
            .await?
            .rows_affected()
        };
        if changed != 1 {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        if !append_audit_events_mariadb(
            &mut transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )
        .await?
        {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(AuditedMutationOutcome::Applied)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn review_quota_request(
        &self,
        expected_request: &QuotaRequestRecord,
        next_status: &str,
        review_note: &str,
        reviewed_by: &str,
        reviewed_at: &str,
        next_integrity_hmac: &str,
        grant: Option<(&MoneyBalanceRecord, &MoneyBalanceRecord, &MoneyLedgerEntry)>,
    ) -> Result<QuotaRequestReviewOutcome, StorageError> {
        match self
            .review_quota_request_inner(
                expected_request,
                next_status,
                review_note,
                reviewed_by,
                reviewed_at,
                next_integrity_hmac,
                grant,
                None,
            )
            .await?
        {
            MutationWithAuditOutcome::Mutation(outcome) => Ok(outcome),
            MutationWithAuditOutcome::AuditConflict => Err(StorageError::DatabaseNotEmpty),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn review_quota_request_and_audit(
        &self,
        expected_request: &QuotaRequestRecord,
        next_status: &str,
        review_note: &str,
        reviewed_by: &str,
        reviewed_at: &str,
        next_integrity_hmac: &str,
        grant: Option<(&MoneyBalanceRecord, &MoneyBalanceRecord, &MoneyLedgerEntry)>,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<MutationWithAuditOutcome<QuotaRequestReviewOutcome>, StorageError> {
        self.review_quota_request_inner(
            expected_request,
            next_status,
            review_note,
            reviewed_by,
            reviewed_at,
            next_integrity_hmac,
            grant,
            Some((
                expected_audit_sequence,
                expected_audit_hmac,
                std::slice::from_ref(audit_event),
            )),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn review_quota_request_inner(
        &self,
        expected_request: &QuotaRequestRecord,
        next_status: &str,
        review_note: &str,
        reviewed_by: &str,
        reviewed_at: &str,
        next_integrity_hmac: &str,
        grant: Option<(&MoneyBalanceRecord, &MoneyBalanceRecord, &MoneyLedgerEntry)>,
        audit: Option<(u64, &str, &[AuditEventRecord])>,
    ) -> Result<MutationWithAuditOutcome<QuotaRequestReviewOutcome>, StorageError> {
        if expected_request.status != "pending"
            || !matches!(next_status, "approved" | "rejected")
            || (next_status == "approved") != grant.is_some()
            || next_integrity_hmac.is_empty()
        {
            return Ok(MutationWithAuditOutcome::Mutation(
                QuotaRequestReviewOutcome::Conflict,
            ));
        }
        if let Some((expected_balance, next_balance, ledger)) = grant
            && (!valid_money_transition(expected_balance, next_balance, ledger)
                || ledger.identity_id != expected_request.identity_id
                || ledger.amount_nanos != expected_request.amount_nanos)
        {
            return Ok(MutationWithAuditOutcome::Mutation(
                QuotaRequestReviewOutcome::Conflict,
            ));
        }
        let mut transaction = self.pool.begin().await?;
        let current = sqlx::query(
            "SELECT id,identity_id,amount_nanos,reason,status,review_note,
                    reviewed_by,reviewed_at,revision,integrity_hmac,created_at
             FROM quota_requests WHERE id=? FOR UPDATE",
        )
        .bind(&expected_request.id)
        .fetch_optional(&mut *transaction)
        .await?
        .map(quota_request_from_mariadb_row)
        .transpose()?;
        if current.as_ref() != Some(expected_request) {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                QuotaRequestReviewOutcome::Conflict,
            ));
        }
        if let Some((expected_balance, next_balance, ledger)) = grant {
            let inserted = sqlx::query(
                "INSERT INTO money_ledger_entries(
                   id,identity_id,currency,kind,amount_nanos,balance_revision,reference_id,billing_status,
                   details_json,previous_entry_hmac,integrity_hmac,created_at
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(&ledger.id).bind(&ledger.identity_id).bind(&ledger.currency)
            .bind(&ledger.kind).bind(ledger.amount_nanos).bind(ledger.balance_revision)
            .bind(&ledger.reference_id).bind(&ledger.billing_status)
            .bind(&ledger.details_json).bind(&ledger.previous_entry_hmac)
            .bind(&ledger.integrity_hmac).bind(&ledger.created_at)
            .execute(&mut *transaction).await;
            if let Err(error) = inserted {
                if is_mariadb_duplicate(&error) {
                    transaction.rollback().await?;
                    return Ok(MutationWithAuditOutcome::Mutation(
                        QuotaRequestReviewOutcome::Conflict,
                    ));
                }
                return Err(StorageError::MariaDb(error));
            }
            if sqlx::query(
                "UPDATE money_balances SET balance_nanos=?,credited_nanos=?,debited_nanos=?,
                   revision=?,last_ledger_hmac=?,integrity_hmac=?,updated_at=?
                 WHERE identity_id=? AND revision=? AND integrity_hmac=?",
            )
            .bind(next_balance.balance_nanos)
            .bind(next_balance.credited_nanos)
            .bind(next_balance.debited_nanos)
            .bind(next_balance.revision)
            .bind(&next_balance.last_ledger_hmac)
            .bind(&next_balance.integrity_hmac)
            .bind(&next_balance.updated_at)
            .bind(&expected_balance.identity_id)
            .bind(expected_balance.revision)
            .bind(&expected_balance.integrity_hmac)
            .execute(&mut *transaction)
            .await?
            .rows_affected()
                != 1
            {
                transaction.rollback().await?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    QuotaRequestReviewOutcome::Conflict,
                ));
            }
        }
        let updated = sqlx::query(
            "UPDATE quota_requests
             SET status=?,review_note=?,reviewed_by=?,reviewed_at=?,revision=?,integrity_hmac=?
             WHERE id=? AND status='pending' AND revision=? AND integrity_hmac=?",
        )
        .bind(next_status)
        .bind(review_note)
        .bind(reviewed_by)
        .bind(reviewed_at)
        .bind(expected_request.revision.saturating_add(1))
        .bind(next_integrity_hmac)
        .bind(&expected_request.id)
        .bind(expected_request.revision)
        .bind(&expected_request.integrity_hmac)
        .execute(&mut *transaction)
        .await?
        .rows_affected()
            == 1;
        if !updated {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                QuotaRequestReviewOutcome::Conflict,
            ));
        }
        if let Some((expected_sequence, expected_hmac, events)) = audit
            && !append_audit_events_mariadb(
                &mut transaction,
                expected_sequence,
                expected_hmac,
                events,
            )
            .await?
        {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(MutationWithAuditOutcome::Mutation(
            QuotaRequestReviewOutcome::Applied,
        ))
    }

    pub async fn insert_vouchers(
        &self,
        records: &[(VoucherRecord, Option<VoucherDeliveryRecord>)],
    ) -> Result<(), StorageError> {
        match self.insert_vouchers_inner(records, None).await? {
            AuditedMutationOutcome::Applied => Ok(()),
            AuditedMutationOutcome::MutationConflict | AuditedMutationOutcome::AuditConflict => {
                Err(StorageError::DatabaseNotEmpty)
            }
        }
    }

    pub async fn insert_vouchers_and_audit(
        &self,
        records: &[(VoucherRecord, Option<VoucherDeliveryRecord>)],
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_events: &[AuditEventRecord],
    ) -> Result<AuditedMutationOutcome, StorageError> {
        if records.len() != audit_events.len() {
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        self.insert_vouchers_inner(
            records,
            Some((expected_audit_sequence, expected_audit_hmac, audit_events)),
        )
        .await
    }

    async fn insert_vouchers_inner(
        &self,
        records: &[(VoucherRecord, Option<VoucherDeliveryRecord>)],
        audit: Option<(u64, &str, &[AuditEventRecord])>,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let mut transaction = self.pool.begin().await?;
        for (voucher, delivery) in records {
            sqlx::query(
                "INSERT INTO vouchers(
                   id,code_hash,code_prefix,name,quota_tokens,status,max_redemptions,
                   redeemed_count,expires_at,created_by,revision,integrity_hmac,created_at
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(&voucher.id)
            .bind(&voucher.code_hash)
            .bind(&voucher.code_prefix)
            .bind(&voucher.name)
            .bind(voucher.quota_tokens)
            .bind(&voucher.status)
            .bind(voucher.max_redemptions)
            .bind(voucher.redeemed_count)
            .bind(&voucher.expires_at)
            .bind(&voucher.created_by)
            .bind(voucher.revision)
            .bind(&voucher.integrity_hmac)
            .bind(&voucher.created_at)
            .execute(&mut *transaction)
            .await?;
            if let Some(delivery) = delivery {
                sqlx::query(
                    "INSERT INTO voucher_deliveries(
                       id,voucher_id,identity_id,status,delivered_at,redeemed_at
                     ) VALUES(?,?,?,?,?,?)",
                )
                .bind(&delivery.id)
                .bind(&delivery.voucher_id)
                .bind(&delivery.identity_id)
                .bind(&delivery.status)
                .bind(&delivery.delivered_at)
                .bind(&delivery.redeemed_at)
                .execute(&mut *transaction)
                .await?;
            }
        }
        if let Some((expected_sequence, expected_hmac, events)) = audit
            && !append_audit_events_mariadb(
                &mut transaction,
                expected_sequence,
                expected_hmac,
                events,
            )
            .await?
        {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(AuditedMutationOutcome::Applied)
    }

    pub async fn list_vouchers(&self) -> Result<Vec<VoucherRecord>, StorageError> {
        sqlx::query(
            "SELECT id,code_hash,code_prefix,name,quota_tokens,status,max_redemptions,
                    redeemed_count,expires_at,created_by,revision,integrity_hmac,created_at
             FROM vouchers ORDER BY created_at DESC,id",
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(voucher_from_mariadb_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(StorageError::from)
    }

    pub async fn voucher_by_code_hash(
        &self,
        code_hash: &str,
    ) -> Result<Option<VoucherRecord>, StorageError> {
        sqlx::query(
            "SELECT id,code_hash,code_prefix,name,quota_tokens,status,max_redemptions,
                    redeemed_count,expires_at,created_by,revision,integrity_hmac,created_at
             FROM vouchers WHERE code_hash=?",
        )
        .bind(code_hash)
        .fetch_optional(&self.pool)
        .await?
        .map(voucher_from_mariadb_row)
        .transpose()
        .map_err(StorageError::from)
    }

    pub async fn voucher_delivery_for_identity(
        &self,
        delivery_id: &str,
        identity_id: &str,
    ) -> Result<Option<(VoucherDeliveryRecord, VoucherRecord)>, StorageError> {
        sqlx::query(
            "SELECT d.id AS delivery_id,d.voucher_id,d.identity_id,d.status AS delivery_status,
                    d.delivered_at,d.redeemed_at,
                    v.id AS voucher_id_value,v.code_hash,v.code_prefix,v.name,v.quota_tokens,
                    v.status AS voucher_status,v.max_redemptions,v.redeemed_count,v.expires_at,
                    v.created_by,v.revision,v.integrity_hmac,v.created_at
             FROM voucher_deliveries d JOIN vouchers v ON v.id=d.voucher_id
             WHERE d.id=? AND d.identity_id=?",
        )
        .bind(delivery_id)
        .bind(identity_id)
        .fetch_optional(&self.pool)
        .await?
        .map(voucher_delivery_pair_from_mariadb_row)
        .transpose()
        .map_err(StorageError::from)
    }

    pub async fn list_voucher_deliveries_for_identity(
        &self,
        identity_id: &str,
    ) -> Result<Vec<(VoucherDeliveryRecord, VoucherRecord)>, StorageError> {
        sqlx::query(
            "SELECT d.id AS delivery_id,d.voucher_id,d.identity_id,d.status AS delivery_status,
                    d.delivered_at,d.redeemed_at,
                    v.id AS voucher_id_value,v.code_hash,v.code_prefix,v.name,v.quota_tokens,
                    v.status AS voucher_status,v.max_redemptions,v.redeemed_count,v.expires_at,
                    v.created_by,v.revision,v.integrity_hmac,v.created_at
             FROM voucher_deliveries d JOIN vouchers v ON v.id=d.voucher_id
             WHERE d.identity_id=? ORDER BY d.delivered_at DESC,d.id",
        )
        .bind(identity_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(voucher_delivery_pair_from_mariadb_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(StorageError::from)
    }

    pub async fn voucher_deliveries(
        &self,
        voucher_id: &str,
    ) -> Result<Vec<VoucherDeliveryRecord>, StorageError> {
        sqlx::query(
            "SELECT id,voucher_id,identity_id,status,delivered_at,redeemed_at
             FROM voucher_deliveries WHERE voucher_id=? ORDER BY delivered_at,id",
        )
        .bind(voucher_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(voucher_delivery_from_mariadb_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(StorageError::from)
    }

    pub async fn voucher_redemptions(
        &self,
        voucher_id: &str,
    ) -> Result<Vec<VoucherRedemptionRecord>, StorageError> {
        sqlx::query(
            "SELECT id,voucher_id,identity_id,amount_tokens,ledger_entry_id,created_at
             FROM voucher_redemptions WHERE voucher_id=? ORDER BY created_at,id",
        )
        .bind(voucher_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(voucher_redemption_from_mariadb_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(StorageError::from)
    }

    pub async fn delete_unused_voucher(&self, voucher_id: &str) -> Result<bool, StorageError> {
        Ok(
            sqlx::query("DELETE FROM vouchers WHERE id=? AND redeemed_count=0")
                .bind(voucher_id)
                .execute(&self.pool)
                .await?
                .rows_affected()
                == 1,
        )
    }

    pub async fn delete_unused_voucher_and_audit(
        &self,
        voucher_id: &str,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let mut transaction = self.pool.begin().await?;
        if sqlx::query("DELETE FROM vouchers WHERE id=? AND redeemed_count=0")
            .bind(voucher_id)
            .execute(&mut *transaction)
            .await?
            .rows_affected()
            != 1
        {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        if !append_audit_events_mariadb(
            &mut transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )
        .await?
        {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(AuditedMutationOutcome::Applied)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn redeem_voucher(
        &self,
        expected_voucher: &VoucherRecord,
        expected_delivery: Option<&VoucherDeliveryRecord>,
        identity_id: &str,
        now: &str,
        expected_balance: &UserBalanceRecord,
        next_balance: &UserBalanceRecord,
        ledger: &QuotaLedgerEntry,
        redemption: &VoucherRedemptionRecord,
        next_voucher_integrity_hmac: &str,
    ) -> Result<VoucherRedeemOutcome, StorageError> {
        match self
            .redeem_voucher_inner(
                expected_voucher,
                expected_delivery,
                identity_id,
                now,
                expected_balance,
                next_balance,
                ledger,
                redemption,
                next_voucher_integrity_hmac,
                None,
            )
            .await?
        {
            MutationWithAuditOutcome::Mutation(outcome) => Ok(outcome),
            MutationWithAuditOutcome::AuditConflict => Err(StorageError::DatabaseNotEmpty),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn redeem_voucher_and_audit(
        &self,
        expected_voucher: &VoucherRecord,
        expected_delivery: Option<&VoucherDeliveryRecord>,
        identity_id: &str,
        now: &str,
        expected_balance: &UserBalanceRecord,
        next_balance: &UserBalanceRecord,
        ledger: &QuotaLedgerEntry,
        redemption: &VoucherRedemptionRecord,
        next_voucher_integrity_hmac: &str,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<MutationWithAuditOutcome<VoucherRedeemOutcome>, StorageError> {
        self.redeem_voucher_inner(
            expected_voucher,
            expected_delivery,
            identity_id,
            now,
            expected_balance,
            next_balance,
            ledger,
            redemption,
            next_voucher_integrity_hmac,
            Some((expected_audit_sequence, expected_audit_hmac, audit_event)),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn redeem_voucher_inner(
        &self,
        expected_voucher: &VoucherRecord,
        expected_delivery: Option<&VoucherDeliveryRecord>,
        identity_id: &str,
        now: &str,
        expected_balance: &UserBalanceRecord,
        next_balance: &UserBalanceRecord,
        ledger: &QuotaLedgerEntry,
        redemption: &VoucherRedemptionRecord,
        next_voucher_integrity_hmac: &str,
        audit: Option<(u64, &str, &AuditEventRecord)>,
    ) -> Result<MutationWithAuditOutcome<VoucherRedeemOutcome>, StorageError> {
        if !valid_grant_transition(expected_balance, next_balance, ledger)
            || expected_voucher.quota_tokens != ledger.amount_tokens
            || expected_voucher.quota_tokens != redemption.amount_tokens
            || expected_voucher.id != redemption.voucher_id
            || expected_balance.identity_id != identity_id
            || ledger.identity_id != identity_id
            || redemption.identity_id != identity_id
            || redemption.ledger_entry_id != ledger.id
            || expected_voucher.integrity_hmac.is_empty()
            || next_voucher_integrity_hmac.is_empty()
        {
            return Ok(MutationWithAuditOutcome::Mutation(
                VoucherRedeemOutcome::Conflict,
            ));
        }
        let mut transaction = self.pool.begin().await?;
        let current_voucher = sqlx::query(
            "SELECT id,code_hash,code_prefix,name,quota_tokens,status,max_redemptions,
                    redeemed_count,expires_at,created_by,revision,integrity_hmac,created_at
             FROM vouchers WHERE id=? FOR UPDATE",
        )
        .bind(&expected_voucher.id)
        .fetch_optional(&mut *transaction)
        .await?
        .map(voucher_from_mariadb_row)
        .transpose()?;
        if current_voucher.as_ref() != Some(expected_voucher) {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                VoucherRedeemOutcome::Conflict,
            ));
        }
        if sqlx::query("SELECT 1 FROM voucher_redemptions WHERE voucher_id=? AND identity_id=?")
            .bind(&expected_voucher.id)
            .bind(identity_id)
            .fetch_optional(&mut *transaction)
            .await?
            .is_some()
        {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                VoucherRedeemOutcome::AlreadyRedeemed,
            ));
        }
        if expected_voucher.status != "active"
            || expected_voucher.redeemed_count >= expected_voucher.max_redemptions
            || expected_voucher
                .expires_at
                .as_deref()
                .is_some_and(|expires_at| expires_at <= now)
        {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                VoucherRedeemOutcome::Unavailable,
            ));
        }
        let delivery_count: i64 = sqlx::query(
            "SELECT COUNT(*) AS delivery_count FROM voucher_deliveries WHERE voucher_id=?",
        )
        .bind(&expected_voucher.id)
        .fetch_one(&mut *transaction)
        .await?
        .try_get("delivery_count")?;
        if let Some(expected_delivery) = expected_delivery {
            let current_delivery = sqlx::query(
                "SELECT id,voucher_id,identity_id,status,delivered_at,redeemed_at
                 FROM voucher_deliveries WHERE id=? AND identity_id=? FOR UPDATE",
            )
            .bind(&expected_delivery.id)
            .bind(identity_id)
            .fetch_optional(&mut *transaction)
            .await?
            .map(voucher_delivery_from_mariadb_row)
            .transpose()?;
            if current_delivery.as_ref() != Some(expected_delivery)
                || expected_delivery.voucher_id != expected_voucher.id
                || expected_delivery.status != "pending"
            {
                transaction.rollback().await?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    VoucherRedeemOutcome::Unavailable,
                ));
            }
        } else if delivery_count != 0 {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                VoucherRedeemOutcome::Unavailable,
            ));
        }
        insert_ledger_mariadb(&mut transaction, ledger).await?;
        if !update_balance_mariadb(&mut transaction, expected_balance, next_balance).await? {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                VoucherRedeemOutcome::Conflict,
            ));
        }
        sqlx::query(
            "INSERT INTO voucher_redemptions(
               id,voucher_id,identity_id,amount_tokens,ledger_entry_id,created_at
             ) VALUES(?,?,?,?,?,?)",
        )
        .bind(&redemption.id)
        .bind(&redemption.voucher_id)
        .bind(&redemption.identity_id)
        .bind(redemption.amount_tokens)
        .bind(&redemption.ledger_entry_id)
        .bind(&redemption.created_at)
        .execute(&mut *transaction)
        .await?;
        let next_redeemed_count = expected_voucher.redeemed_count.saturating_add(1);
        let next_status = if next_redeemed_count >= expected_voucher.max_redemptions {
            "redeemed"
        } else {
            "active"
        };
        if sqlx::query(
            "UPDATE vouchers SET redeemed_count=?,status=?,revision=?,integrity_hmac=?
             WHERE id=? AND status='active' AND redeemed_count=? AND revision=? AND integrity_hmac=?",
        )
        .bind(next_redeemed_count)
        .bind(next_status)
        .bind(expected_voucher.revision.saturating_add(1))
        .bind(next_voucher_integrity_hmac)
        .bind(&expected_voucher.id)
        .bind(expected_voucher.redeemed_count)
        .bind(expected_voucher.revision)
        .bind(&expected_voucher.integrity_hmac)
        .execute(&mut *transaction)
        .await?
        .rows_affected()
            != 1
        {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                VoucherRedeemOutcome::Conflict,
            ));
        }
        if let Some(delivery) = expected_delivery
            && sqlx::query(
                "UPDATE voucher_deliveries SET status='redeemed',redeemed_at=?
                 WHERE id=? AND identity_id=? AND status='pending'",
            )
            .bind(now)
            .bind(&delivery.id)
            .bind(identity_id)
            .execute(&mut *transaction)
            .await?
            .rows_affected()
                != 1
        {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                VoucherRedeemOutcome::Conflict,
            ));
        }
        if let Some((expected_sequence, expected_hmac, event)) = audit
            && !append_audit_events_mariadb(
                &mut transaction,
                expected_sequence,
                expected_hmac,
                std::slice::from_ref(event),
            )
            .await?
        {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(MutationWithAuditOutcome::Mutation(
            VoucherRedeemOutcome::Applied,
        ))
    }

    pub async fn initialize_money_balance(
        &self,
        balance: &MoneyBalanceRecord,
    ) -> Result<bool, StorageError> {
        if !valid_initial_money_balance(balance) {
            return Ok(false);
        }
        let inserted = sqlx::query(
            "INSERT INTO money_balances(
               identity_id,currency,balance_nanos,credited_nanos,debited_nanos,
               revision,last_ledger_hmac,integrity_hmac,updated_at
             ) VALUES(?,?,?,?,?,?,?,?,?)",
        )
        .bind(&balance.identity_id)
        .bind(&balance.currency)
        .bind(balance.balance_nanos)
        .bind(balance.credited_nanos)
        .bind(balance.debited_nanos)
        .bind(balance.revision)
        .bind(&balance.last_ledger_hmac)
        .bind(&balance.integrity_hmac)
        .bind(&balance.updated_at)
        .execute(&self.pool)
        .await;
        match inserted {
            Ok(result) => Ok(result.rows_affected() == 1),
            Err(error) if is_mariadb_duplicate(&error) => Ok(false),
            Err(error) => Err(StorageError::MariaDb(error)),
        }
    }

    pub async fn money_state_snapshot(
        &self,
        identity_id: &str,
    ) -> Result<Option<MoneyStateSnapshot>, StorageError> {
        let mut transaction = self.pool.begin().await?;
        let balance = sqlx::query(
            "SELECT identity_id,currency,balance_nanos,credited_nanos,debited_nanos,
                    revision,last_ledger_hmac,integrity_hmac,updated_at
             FROM money_balances WHERE identity_id=?",
        )
        .bind(identity_id)
        .fetch_optional(&mut *transaction)
        .await?
        .map(money_balance_from_mariadb_row)
        .transpose()?;
        let Some(balance) = balance else {
            return Ok(None);
        };
        let ledger_entries = sqlx::query(
            "SELECT id,identity_id,currency,kind,amount_nanos,balance_revision,reference_id,
                    billing_status,details_json,previous_entry_hmac,integrity_hmac,created_at
             FROM money_ledger_entries WHERE identity_id=? ORDER BY balance_revision ASC",
        )
        .bind(identity_id)
        .fetch_all(&mut *transaction)
        .await?
        .into_iter()
        .map(money_ledger_from_mariadb_row)
        .collect::<Result<Vec<_>, _>>()?;
        transaction.commit().await?;
        Ok(Some(MoneyStateSnapshot {
            balance,
            ledger_entries,
        }))
    }

    pub async fn write_money_entry_with_audit(
        &self,
        expected: &MoneyBalanceRecord,
        next: &MoneyBalanceRecord,
        entry: &MoneyLedgerEntry,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<MutationWithAuditOutcome<MoneyMutationOutcome>, StorageError> {
        if !valid_money_transition(expected, next, entry) {
            return Ok(MutationWithAuditOutcome::Mutation(
                MoneyMutationOutcome::Conflict,
            ));
        }
        let mut transaction = self.pool.begin().await?;
        let current = sqlx::query(
            "SELECT identity_id,currency,balance_nanos,credited_nanos,debited_nanos,
                    revision,last_ledger_hmac,integrity_hmac,updated_at
             FROM money_balances WHERE identity_id=? FOR UPDATE",
        )
        .bind(&expected.identity_id)
        .fetch_optional(&mut *transaction)
        .await?
        .map(money_balance_from_mariadb_row)
        .transpose()?;
        if current.as_ref() != Some(expected) {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                MoneyMutationOutcome::Conflict,
            ));
        }
        let inserted = sqlx::query(
            "INSERT INTO money_ledger_entries(
               id,identity_id,currency,kind,amount_nanos,balance_revision,reference_id,
               billing_status,details_json,previous_entry_hmac,integrity_hmac,created_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&entry.id)
        .bind(&entry.identity_id)
        .bind(&entry.currency)
        .bind(&entry.kind)
        .bind(entry.amount_nanos)
        .bind(entry.balance_revision)
        .bind(&entry.reference_id)
        .bind(&entry.billing_status)
        .bind(&entry.details_json)
        .bind(&entry.previous_entry_hmac)
        .bind(&entry.integrity_hmac)
        .bind(&entry.created_at)
        .execute(&mut *transaction)
        .await;
        match inserted {
            Ok(result) if result.rows_affected() == 1 => {}
            Ok(_) => {
                transaction.rollback().await?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    MoneyMutationOutcome::Conflict,
                ));
            }
            Err(error) if is_mariadb_duplicate(&error) => {
                transaction.rollback().await?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    MoneyMutationOutcome::DuplicateReference,
                ));
            }
            Err(error) => return Err(StorageError::MariaDb(error)),
        }
        let updated = sqlx::query(
            "UPDATE money_balances SET balance_nanos=?,credited_nanos=?,debited_nanos=?,
               revision=?,last_ledger_hmac=?,integrity_hmac=?,updated_at=?
             WHERE identity_id=? AND revision=? AND integrity_hmac=?",
        )
        .bind(next.balance_nanos)
        .bind(next.credited_nanos)
        .bind(next.debited_nanos)
        .bind(next.revision)
        .bind(&next.last_ledger_hmac)
        .bind(&next.integrity_hmac)
        .bind(&next.updated_at)
        .bind(&expected.identity_id)
        .bind(expected.revision)
        .bind(&expected.integrity_hmac)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                MoneyMutationOutcome::Conflict,
            ));
        }
        if !append_audit_events_mariadb(
            &mut transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )
        .await?
        {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(MutationWithAuditOutcome::Mutation(
            MoneyMutationOutcome::Applied,
        ))
    }

    pub async fn runtime_setting(
        &self,
        key: &str,
    ) -> Result<Option<RuntimeSettingRecord>, StorageError> {
        sqlx::query(
            "SELECT setting_key,setting_value,revision,integrity_hmac,updated_at
             FROM runtime_settings WHERE setting_key=?",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await?
        .map(runtime_setting_from_mariadb_row)
        .transpose()
        .map_err(StorageError::from)
    }

    pub async fn write_runtime_setting(
        &self,
        expected: Option<&RuntimeSettingRecord>,
        next: &RuntimeSettingRecord,
    ) -> Result<RuntimeSettingWriteOutcome, StorageError> {
        let valid_transition = match expected {
            Some(expected) => {
                next.key == expected.key && next.revision == expected.revision.saturating_add(1)
            }
            None => next.revision == 0,
        } && !next.key.is_empty()
            && !next.integrity_hmac.is_empty();
        if !valid_transition {
            return Ok(RuntimeSettingWriteOutcome::Conflict);
        }
        let result = if let Some(expected) = expected {
            sqlx::query(
                "UPDATE runtime_settings
                 SET setting_value=?,revision=?,integrity_hmac=?,updated_at=?
                 WHERE setting_key=? AND revision=? AND integrity_hmac=?",
            )
            .bind(&next.value)
            .bind(next.revision)
            .bind(&next.integrity_hmac)
            .bind(&next.updated_at)
            .bind(&expected.key)
            .bind(expected.revision)
            .bind(&expected.integrity_hmac)
            .execute(&self.pool)
            .await
        } else {
            sqlx::query(
                "INSERT INTO runtime_settings(
                   setting_key,setting_value,revision,integrity_hmac,updated_at
                 ) VALUES(?,?,?,?,?)",
            )
            .bind(&next.key)
            .bind(&next.value)
            .bind(next.revision)
            .bind(&next.integrity_hmac)
            .bind(&next.updated_at)
            .execute(&self.pool)
            .await
        };
        match result {
            Ok(result) if result.rows_affected() == 1 => Ok(RuntimeSettingWriteOutcome::Applied),
            Ok(_) => Ok(RuntimeSettingWriteOutcome::Conflict),
            Err(error) if is_mariadb_duplicate(&error) => Ok(RuntimeSettingWriteOutcome::Conflict),
            Err(error) => Err(StorageError::MariaDb(error)),
        }
    }

    pub async fn write_runtime_setting_with_audit(
        &self,
        expected: Option<&RuntimeSettingRecord>,
        next: &RuntimeSettingRecord,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let valid_transition = match expected {
            Some(expected) => {
                next.key == expected.key && next.revision == expected.revision.saturating_add(1)
            }
            None => next.revision == 0,
        } && !next.key.is_empty()
            && !next.integrity_hmac.is_empty();
        let valid_audit = audit_event.sequence == expected_audit_sequence.saturating_add(1)
            && audit_event.previous_event_hmac == expected_audit_hmac
            && audit_event.integrity_hmac.len() == 43;
        if !valid_transition {
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        if !valid_audit {
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        let mut transaction = self.pool.begin().await?;
        let changed = if let Some(expected) = expected {
            sqlx::query(
                "UPDATE runtime_settings
                 SET setting_value=?,revision=?,integrity_hmac=?,updated_at=?
                 WHERE setting_key=? AND revision=? AND integrity_hmac=?",
            )
            .bind(&next.value)
            .bind(next.revision)
            .bind(&next.integrity_hmac)
            .bind(&next.updated_at)
            .bind(&expected.key)
            .bind(expected.revision)
            .bind(&expected.integrity_hmac)
            .execute(&mut *transaction)
            .await
        } else {
            sqlx::query(
                "INSERT INTO runtime_settings(
                   setting_key,setting_value,revision,integrity_hmac,updated_at
                 ) VALUES(?,?,?,?,?)",
            )
            .bind(&next.key)
            .bind(&next.value)
            .bind(next.revision)
            .bind(&next.integrity_hmac)
            .bind(&next.updated_at)
            .execute(&mut *transaction)
            .await
        };
        let changed = match changed {
            Ok(result) => result.rows_affected() == 1,
            Err(error) if is_mariadb_duplicate(&error) => false,
            Err(error) => return Err(StorageError::MariaDb(error)),
        };
        if !changed {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        sqlx::query_scalar::<MySql, u64>(
            "SELECT revision FROM transaction_gates WHERE gate_key='audit_log' FOR UPDATE",
        )
        .fetch_one(&mut *transaction)
        .await?;
        let current = sqlx::query(
            "SELECT event_sequence,integrity_hmac
             FROM audit_events ORDER BY event_sequence DESC LIMIT 1",
        )
        .fetch_optional(&mut *transaction)
        .await?
        .map(|row| {
            Ok::<_, sqlx::Error>((
                row.try_get("event_sequence")?,
                row.try_get("integrity_hmac")?,
            ))
        })
        .transpose()?
        .unwrap_or((0, String::new()));
        if current.0 != expected_audit_sequence || current.1 != expected_audit_hmac {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        let inserted = sqlx::query(
            "INSERT INTO audit_events(
               id,event_sequence,actor_identity_id,actor_role,action,target_type,target_id,
               outcome,previous_event_hmac,integrity_hmac,created_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&audit_event.id)
        .bind(audit_event.sequence)
        .bind(&audit_event.actor_identity_id)
        .bind(&audit_event.actor_role)
        .bind(&audit_event.action)
        .bind(&audit_event.target_type)
        .bind(&audit_event.target_id)
        .bind(&audit_event.outcome)
        .bind(&audit_event.previous_event_hmac)
        .bind(&audit_event.integrity_hmac)
        .bind(&audit_event.created_at)
        .execute(&mut *transaction)
        .await;
        match inserted {
            Ok(result) if result.rows_affected() == 1 => {
                sqlx::query(
                    "UPDATE transaction_gates SET revision=revision+1 WHERE gate_key='audit_log'",
                )
                .execute(&mut *transaction)
                .await?;
                transaction.commit().await?;
                Ok(AuditedMutationOutcome::Applied)
            }
            Ok(_) => {
                transaction.rollback().await?;
                Ok(AuditedMutationOutcome::AuditConflict)
            }
            Err(error) if is_mariadb_duplicate(&error) => {
                transaction.rollback().await?;
                Ok(AuditedMutationOutcome::AuditConflict)
            }
            Err(error) => Err(StorageError::MariaDb(error)),
        }
    }

    pub async fn audit_tail(&self) -> Result<Option<AuditEventRecord>, StorageError> {
        sqlx::query(
            "SELECT id,event_sequence,actor_identity_id,actor_role,action,target_type,target_id,
                    outcome,previous_event_hmac,integrity_hmac,created_at
             FROM audit_events ORDER BY event_sequence DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?
        .map(audit_event_from_mariadb_row)
        .transpose()
        .map_err(StorageError::from)
    }

    pub async fn audit_revision(&self) -> Result<u64, StorageError> {
        sqlx::query_scalar::<MySql, u64>(
            "SELECT revision FROM transaction_gates WHERE gate_key='audit_log'",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(StorageError::from)
    }

    pub async fn audit_events(&self) -> Result<Vec<AuditEventRecord>, StorageError> {
        sqlx::query(
            "SELECT id,event_sequence,actor_identity_id,actor_role,action,target_type,target_id,
                    outcome,previous_event_hmac,integrity_hmac,created_at
             FROM audit_events ORDER BY event_sequence",
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(audit_event_from_mariadb_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(StorageError::from)
    }

    pub async fn append_audit_event(
        &self,
        expected_sequence: u64,
        expected_hmac: &str,
        event: &AuditEventRecord,
    ) -> Result<AuditAppendOutcome, StorageError> {
        if event.sequence != expected_sequence.saturating_add(1)
            || event.previous_event_hmac != expected_hmac
            || event.integrity_hmac.len() != 43
        {
            return Ok(AuditAppendOutcome::Conflict);
        }
        let mut transaction = self.pool.begin().await?;
        sqlx::query_scalar::<MySql, u64>(
            "SELECT revision FROM transaction_gates WHERE gate_key='audit_log' FOR UPDATE",
        )
        .fetch_one(&mut *transaction)
        .await?;
        let current = sqlx::query(
            "SELECT event_sequence,integrity_hmac
             FROM audit_events ORDER BY event_sequence DESC LIMIT 1",
        )
        .fetch_optional(&mut *transaction)
        .await?
        .map(|row| {
            Ok::<_, sqlx::Error>((
                row.try_get("event_sequence")?,
                row.try_get("integrity_hmac")?,
            ))
        })
        .transpose()?
        .unwrap_or((0, String::new()));
        if current.0 != expected_sequence || current.1 != expected_hmac {
            transaction.rollback().await?;
            return Ok(AuditAppendOutcome::Conflict);
        }
        let inserted = sqlx::query(
            "INSERT INTO audit_events(
               id,event_sequence,actor_identity_id,actor_role,action,target_type,target_id,
               outcome,previous_event_hmac,integrity_hmac,created_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&event.id)
        .bind(event.sequence)
        .bind(&event.actor_identity_id)
        .bind(&event.actor_role)
        .bind(&event.action)
        .bind(&event.target_type)
        .bind(&event.target_id)
        .bind(&event.outcome)
        .bind(&event.previous_event_hmac)
        .bind(&event.integrity_hmac)
        .bind(&event.created_at)
        .execute(&mut *transaction)
        .await;
        match inserted {
            Ok(result) if result.rows_affected() == 1 => {
                sqlx::query(
                    "UPDATE transaction_gates SET revision=revision+1 WHERE gate_key='audit_log'",
                )
                .execute(&mut *transaction)
                .await?;
                transaction.commit().await?;
                Ok(AuditAppendOutcome::Applied)
            }
            Ok(_) => {
                transaction.rollback().await?;
                Ok(AuditAppendOutcome::Conflict)
            }
            Err(error) if is_mariadb_duplicate(&error) => {
                transaction.rollback().await?;
                Ok(AuditAppendOutcome::Conflict)
            }
            Err(error) => Err(StorageError::MariaDb(error)),
        }
    }

    pub async fn release_quota_reservation(
        &self,
        expected_balance: &UserBalanceRecord,
        next_balance: &UserBalanceRecord,
        expected_reservation: &QuotaReservationRecord,
        next_reservation: &QuotaReservationRecord,
    ) -> Result<QuotaMutationOutcome, StorageError> {
        if !valid_release_transition(
            expected_balance,
            next_balance,
            expected_reservation,
            next_reservation,
        ) {
            return Ok(QuotaMutationOutcome::Conflict);
        }
        self.finish_reservation_mariadb(
            expected_balance,
            next_balance,
            expected_reservation,
            next_reservation,
            None,
        )
        .await
    }

    pub async fn fail_quota_reservation(
        &self,
        expected_balance: &UserBalanceRecord,
        next_balance: &UserBalanceRecord,
        expected_reservation: &QuotaReservationRecord,
        next_reservation: &QuotaReservationRecord,
        ledger: &QuotaLedgerEntry,
    ) -> Result<QuotaMutationOutcome, StorageError> {
        if !valid_failed_request_transition(
            expected_balance,
            next_balance,
            expected_reservation,
            next_reservation,
            ledger,
        ) {
            return Ok(QuotaMutationOutcome::Conflict);
        }
        self.finish_reservation_mariadb(
            expected_balance,
            next_balance,
            expected_reservation,
            next_reservation,
            Some(ledger),
        )
        .await
    }

    pub async fn settle_quota_reservation(
        &self,
        expected_balance: &UserBalanceRecord,
        next_balance: &UserBalanceRecord,
        expected_reservation: &QuotaReservationRecord,
        next_reservation: &QuotaReservationRecord,
        ledger: &QuotaLedgerEntry,
    ) -> Result<QuotaMutationOutcome, StorageError> {
        if !valid_settlement_transition(
            expected_balance,
            next_balance,
            expected_reservation,
            next_reservation,
            ledger,
        ) {
            return Ok(QuotaMutationOutcome::Conflict);
        }
        self.finish_reservation_mariadb(
            expected_balance,
            next_balance,
            expected_reservation,
            next_reservation,
            Some(ledger),
        )
        .await
    }

    async fn finish_reservation_mariadb(
        &self,
        expected_balance: &UserBalanceRecord,
        next_balance: &UserBalanceRecord,
        expected_reservation: &QuotaReservationRecord,
        next_reservation: &QuotaReservationRecord,
        ledger: Option<&QuotaLedgerEntry>,
    ) -> Result<QuotaMutationOutcome, StorageError> {
        let mut transaction = self.pool.begin().await?;
        let current_balance = sqlx::query(
            "SELECT identity_id,balance_tokens,reserved_tokens,granted_tokens,
                    consumed_tokens,request_count,raw_tokens,billed_tokens,uncached_input,
                    cached_input,cache_write,output_tokens,revision,last_ledger_hmac,
                    integrity_hmac,updated_at
             FROM user_balances WHERE identity_id=? FOR UPDATE",
        )
        .bind(&expected_balance.identity_id)
        .fetch_optional(&mut *transaction)
        .await?
        .map(balance_from_mariadb_row)
        .transpose()?;
        let current_reservation = sqlx::query(
            "SELECT id,identity_id,api_key_id,request_id,reserved_tokens,status,revision,
                    integrity_hmac,created_at,expires_at,settled_at
             FROM quota_reservations WHERE id=? FOR UPDATE",
        )
        .bind(&expected_reservation.id)
        .fetch_optional(&mut *transaction)
        .await?
        .map(quota_reservation_from_mariadb_row)
        .transpose()?;
        if current_balance.as_ref() != Some(expected_balance)
            || current_reservation.as_ref() != Some(expected_reservation)
        {
            transaction.rollback().await?;
            return Ok(QuotaMutationOutcome::Conflict);
        }
        if let Some(ledger) = ledger {
            let inserted = insert_ledger_mariadb(&mut transaction, ledger).await;
            if let Err(error) = inserted {
                if is_mariadb_duplicate(&error) {
                    transaction.rollback().await?;
                    return Ok(QuotaMutationOutcome::DuplicateRequest);
                }
                return Err(StorageError::MariaDb(error));
            }
        }
        let reservation_changed = sqlx::query(
            "UPDATE quota_reservations
             SET status=?,revision=?,integrity_hmac=?,settled_at=?
             WHERE id=? AND status=? AND revision=? AND integrity_hmac=?",
        )
        .bind(&next_reservation.status)
        .bind(next_reservation.revision)
        .bind(&next_reservation.integrity_hmac)
        .bind(&next_reservation.settled_at)
        .bind(&expected_reservation.id)
        .bind(&expected_reservation.status)
        .bind(expected_reservation.revision)
        .bind(&expected_reservation.integrity_hmac)
        .execute(&mut *transaction)
        .await?
        .rows_affected()
            == 1;
        let balance_changed =
            update_balance_mariadb(&mut transaction, expected_balance, next_balance).await?;
        if !reservation_changed || !balance_changed {
            transaction.rollback().await?;
            return Ok(QuotaMutationOutcome::Conflict);
        }
        transaction.commit().await?;
        Ok(QuotaMutationOutcome::Applied)
    }

    pub async fn runner_by_credential_hash(
        &self,
        credential_hash: &str,
    ) -> Result<Option<RunnerRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT id,enabled,version,protocol_version,max_inflight,inflight,
                    recent_request_count,recent_error_count,latency_ms,last_seen_at
             FROM runners WHERE credential_hash=? AND enabled=1",
        )
        .bind(credential_hash)
        .fetch_optional(&self.pool)
        .await?;
        let record = match row {
            Some(row) => Some(RunnerRecord {
                id: row.try_get("id")?,
                enabled: row.try_get::<u8, _>("enabled")? == 1,
                version: row.try_get("version")?,
                protocol_version: row.try_get("protocol_version")?,
                max_inflight: row.try_get("max_inflight")?,
                inflight: row.try_get("inflight")?,
                recent_request_count: row.try_get("recent_request_count")?,
                recent_error_count: row.try_get("recent_error_count")?,
                latency_ms: row.try_get("latency_ms")?,
                last_seen_at: row.try_get("last_seen_at")?,
            }),
            None => None,
        };
        Ok(record)
    }

    pub async fn list_enabled_runners(&self) -> Result<Vec<RunnerRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT id,enabled,version,protocol_version,max_inflight,inflight,
                    recent_request_count,recent_error_count,latency_ms,last_seen_at
             FROM runners WHERE enabled=1 ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(RunnerRecord {
                    id: row.try_get("id")?,
                    enabled: row.try_get::<u8, _>("enabled")? == 1,
                    version: row.try_get("version")?,
                    protocol_version: row.try_get("protocol_version")?,
                    max_inflight: row.try_get("max_inflight")?,
                    inflight: row.try_get("inflight")?,
                    recent_request_count: row.try_get("recent_request_count")?,
                    recent_error_count: row.try_get("recent_error_count")?,
                    latency_ms: row.try_get("latency_ms")?,
                    last_seen_at: row.try_get("last_seen_at")?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()
            .map_err(StorageError::from)
    }

    pub async fn list_runners(&self) -> Result<Vec<RunnerAdminRecord>, StorageError> {
        sqlx::query(
            "SELECT id,name,enabled,version,protocol_version,platform,architecture,max_inflight,
                    inflight,recent_request_count,recent_error_count,latency_ms,last_seen_at,
                    created_at,updated_at
             FROM runners ORDER BY created_at,id",
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(runner_admin_from_mariadb_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(StorageError::from)
    }

    pub async fn insert_runner_enrollment(
        &self,
        enrollment: &RunnerEnrollmentRecord,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO runner_enrollments(
               id,token_hash,token_prefix,runner_name,status,expires_at,created_by,created_at
             ) VALUES(?,?,?,?,?,?,?,?)",
        )
        .bind(&enrollment.id)
        .bind(&enrollment.token_hash)
        .bind(&enrollment.token_prefix)
        .bind(&enrollment.runner_name)
        .bind(&enrollment.status)
        .bind(&enrollment.expires_at)
        .bind(&enrollment.created_by)
        .bind(&enrollment.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn pending_runner_enrollment(
        &self,
        token_hash: &str,
        now: &str,
    ) -> Result<bool, StorageError> {
        let enrollment = sqlx::query(
            "SELECT id FROM runner_enrollments
             WHERE token_hash=? AND status='pending' AND expires_at>? AND runner_id IS NULL
             LIMIT 1",
        )
        .bind(token_hash)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;
        Ok(enrollment.is_some())
    }

    pub async fn insert_runner_enrollment_and_audit(
        &self,
        enrollment: &RunnerEnrollmentRecord,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO runner_enrollments(
               id,token_hash,token_prefix,runner_name,status,expires_at,created_by,created_at
             ) VALUES(?,?,?,?,?,?,?,?)",
        )
        .bind(&enrollment.id)
        .bind(&enrollment.token_hash)
        .bind(&enrollment.token_prefix)
        .bind(&enrollment.runner_name)
        .bind(&enrollment.status)
        .bind(&enrollment.expires_at)
        .bind(&enrollment.created_by)
        .bind(&enrollment.created_at)
        .execute(&mut *transaction)
        .await?;
        if !append_audit_events_mariadb(
            &mut transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )
        .await?
        {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(AuditedMutationOutcome::Applied)
    }

    pub async fn register_local_runner_and_audit(
        &self,
        enrollment: &RunnerEnrollmentRecord,
        registration: &RunnerRegistrationRecord,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let mut transaction = self.pool.begin().await?;
        let seen = sqlx::query(
            "SELECT id FROM runner_enrollments WHERE id=? OR runner_id=? OR runner_name=? LIMIT 1 FOR UPDATE",
        )
        .bind(&enrollment.id).bind(&registration.id).bind(&enrollment.runner_name)
        .fetch_optional(&mut *transaction).await?;
        if seen.is_some() {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        sqlx::query(
            "INSERT INTO runner_enrollments(id,token_hash,token_prefix,runner_name,status,expires_at,created_by,created_at)
             VALUES(?,?,?,?,'pending',?,?,?)",
        )
        .bind(&enrollment.id).bind(&enrollment.token_hash).bind(&enrollment.token_prefix)
        .bind(&enrollment.runner_name).bind(&enrollment.expires_at).bind(&enrollment.created_by)
        .bind(&enrollment.created_at).execute(&mut *transaction).await?;
        sqlx::query(
            "INSERT INTO runners(id,enrollment_id,name,credential_hash,enabled,version,protocol_version,
               platform,architecture,max_inflight,created_at,updated_at) VALUES(?,?,?,?,1,?,?,?,?,?,?,?)",
        )
        .bind(&registration.id).bind(&enrollment.id).bind(&enrollment.runner_name)
        .bind(&registration.credential_hash).bind(&registration.version).bind(registration.protocol_version)
        .bind(&registration.platform).bind(&registration.architecture).bind(registration.max_inflight)
        .bind(&registration.created_at).bind(&registration.created_at).execute(&mut *transaction).await?;
        sqlx::query("UPDATE runner_enrollments SET status='used',used_at=?,runner_id=? WHERE id=?")
            .bind(&registration.created_at)
            .bind(&registration.id)
            .bind(&enrollment.id)
            .execute(&mut *transaction)
            .await?;
        if !append_audit_events_mariadb(
            &mut transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )
        .await?
        {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(AuditedMutationOutcome::Applied)
    }

    pub async fn consume_runner_enrollment_unchecked(
        &self,
        token_hash: &str,
        now: &str,
        registration: &RunnerRegistrationRecord,
    ) -> Result<RunnerEnrollmentConsumeOutcome, StorageError> {
        match self
            .consume_runner_enrollment_inner(
                token_hash,
                now,
                registration,
                RunnerQuotaPolicy::physical(None),
                None,
            )
            .await?
        {
            MutationWithAuditOutcome::Mutation(outcome) => Ok(outcome),
            MutationWithAuditOutcome::AuditConflict => Err(StorageError::DatabaseNotEmpty),
        }
    }

    pub async fn consume_runner_enrollment_unchecked_and_audit(
        &self,
        token_hash: &str,
        now: &str,
        registration: &RunnerRegistrationRecord,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<MutationWithAuditOutcome<RunnerEnrollmentConsumeOutcome>, StorageError> {
        self.consume_runner_enrollment_inner(
            token_hash,
            now,
            registration,
            RunnerQuotaPolicy::physical(None),
            Some((expected_audit_sequence, expected_audit_hmac, audit_event)),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn consume_runner_enrollment_with_limit_and_audit(
        &self,
        token_hash: &str,
        now: &str,
        registration: &RunnerRegistrationRecord,
        runner_limit: Option<u32>,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<MutationWithAuditOutcome<RunnerEnrollmentConsumeOutcome>, StorageError> {
        self.consume_runner_enrollment_with_quota_and_audit(
            token_hash,
            now,
            registration,
            RunnerQuotaPolicy::physical(runner_limit),
            (expected_audit_sequence, expected_audit_hmac, audit_event),
        )
        .await
    }

    pub async fn consume_runner_enrollment_with_quota_and_audit(
        &self,
        token_hash: &str,
        now: &str,
        registration: &RunnerRegistrationRecord,
        quota: RunnerQuotaPolicy,
        audit: (u64, &str, &AuditEventRecord),
    ) -> Result<MutationWithAuditOutcome<RunnerEnrollmentConsumeOutcome>, StorageError> {
        self.consume_runner_enrollment_inner(token_hash, now, registration, quota, Some(audit))
            .await
    }

    async fn consume_runner_enrollment_inner(
        &self,
        token_hash: &str,
        now: &str,
        registration: &RunnerRegistrationRecord,
        quota: RunnerQuotaPolicy,
        audit: Option<(u64, &str, &AuditEventRecord)>,
    ) -> Result<MutationWithAuditOutcome<RunnerEnrollmentConsumeOutcome>, StorageError> {
        let mut transaction = self.pool.begin().await?;
        lock_runner_quota_mariadb(&mut transaction).await?;
        let enrollment = sqlx::query(
            "SELECT id,runner_name FROM runner_enrollments
             WHERE token_hash=? AND status='pending' AND expires_at>? AND runner_id IS NULL
             FOR UPDATE",
        )
        .bind(token_hash)
        .bind(now)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(enrollment) = enrollment else {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                RunnerEnrollmentConsumeOutcome::Invalid,
            ));
        };
        let enrollment_id: String = enrollment.try_get("id")?;
        let runner_name: String = enrollment.try_get("runner_name")?;
        let actual = runner_quota_snapshot_mariadb(&mut transaction).await?;
        let Some(occupied) = quota.occupied(&actual)? else {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                RunnerEnrollmentConsumeOutcome::QuotaChanged,
            ));
        };
        if quota.limit.is_some_and(|limit| occupied >= limit) {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                RunnerEnrollmentConsumeOutcome::LimitReached,
            ));
        }
        sqlx::query(
            "INSERT INTO runners(
               id,enrollment_id,name,credential_hash,enabled,version,protocol_version,
               platform,architecture,max_inflight,created_at,updated_at
             ) VALUES(?,?,?,?,1,?,?,?,?,?,?,?)",
        )
        .bind(&registration.id)
        .bind(&enrollment_id)
        .bind(runner_name)
        .bind(&registration.credential_hash)
        .bind(&registration.version)
        .bind(registration.protocol_version)
        .bind(&registration.platform)
        .bind(&registration.architecture)
        .bind(registration.max_inflight)
        .bind(&registration.created_at)
        .bind(&registration.created_at)
        .execute(&mut *transaction)
        .await?;
        let updated = sqlx::query(
            "UPDATE runner_enrollments SET status='used',used_at=?,runner_id=?
             WHERE id=? AND status='pending' AND runner_id IS NULL",
        )
        .bind(now)
        .bind(&registration.id)
        .bind(&enrollment_id)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                RunnerEnrollmentConsumeOutcome::Invalid,
            ));
        }
        if let Some((expected_sequence, expected_hmac, event)) = audit
            && !append_audit_events_mariadb(
                &mut transaction,
                expected_sequence,
                expected_hmac,
                std::slice::from_ref(event),
            )
            .await?
        {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(MutationWithAuditOutcome::Mutation(
            RunnerEnrollmentConsumeOutcome::Registered,
        ))
    }

    pub async fn update_runner_enabled(
        &self,
        runner_id: &str,
        enabled: bool,
        updated_at: &str,
    ) -> Result<bool, StorageError> {
        Ok(
            sqlx::query("UPDATE runners SET enabled=?,updated_at=? WHERE id=?")
                .bind(enabled)
                .bind(updated_at)
                .bind(runner_id)
                .execute(&self.pool)
                .await?
                .rows_affected()
                == 1,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_runner_enabled_and_audit(
        &self,
        runner_id: &str,
        enabled: bool,
        updated_at: &str,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let mut transaction = self.pool.begin().await?;
        if sqlx::query("UPDATE runners SET enabled=?,updated_at=? WHERE id=?")
            .bind(enabled)
            .bind(updated_at)
            .bind(runner_id)
            .execute(&mut *transaction)
            .await?
            .rows_affected()
            != 1
        {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        if !append_audit_events_mariadb(
            &mut transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )
        .await?
        {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(AuditedMutationOutcome::Applied)
    }

    pub async fn delete_runner(&self, runner_id: &str) -> Result<bool, StorageError> {
        Ok(sqlx::query("DELETE FROM runners WHERE id=?")
            .bind(runner_id)
            .execute(&self.pool)
            .await?
            .rows_affected()
            == 1)
    }

    pub async fn delete_runner_and_audit(
        &self,
        runner_id: &str,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let mut transaction = self.pool.begin().await?;
        if sqlx::query("DELETE FROM runners WHERE id=?")
            .bind(runner_id)
            .execute(&mut *transaction)
            .await?
            .rows_affected()
            != 1
        {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        if !append_audit_events_mariadb(
            &mut transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )
        .await?
        {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(AuditedMutationOutcome::Applied)
    }

    pub async fn insert_session(&self, session: &SessionRecord) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO sessions(id,identity_id,token_hash,expires_at,integrity_hmac,created_at)
             VALUES(?,?,?,?,?,?)",
        )
        .bind(&session.id)
        .bind(&session.identity_id)
        .bind(&session.token_hash)
        .bind(&session.expires_at)
        .bind(&session.integrity_hmac)
        .bind(&session.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn authenticated_session_by_token_hash(
        &self,
        token_hash: &str,
        now: &str,
    ) -> Result<Option<AuthenticatedSession>, StorageError> {
        sqlx::query(
            "SELECT s.id AS session_id,s.identity_id,s.token_hash,s.expires_at,
                    s.integrity_hmac AS session_integrity_hmac,s.created_at AS session_created_at,
                    i.id AS identity_id_value,i.email,i.display_name,i.password_hash,i.role,i.status,
                    i.can_consume_model,i.password_change_required,i.revision,
                    i.integrity_hmac AS identity_integrity_hmac,
                    i.created_at AS identity_created_at,i.updated_at AS identity_updated_at
             FROM sessions s JOIN identities i ON i.id=s.identity_id
             WHERE s.token_hash=? AND s.expires_at>? AND i.status='active'",
        )
        .bind(token_hash)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?
        .map(authenticated_session_from_mariadb_row)
        .transpose()
        .map_err(StorageError::from)
    }

    pub async fn delete_session_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<bool, StorageError> {
        let result = sqlx::query("DELETE FROM sessions WHERE token_hash=?")
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn insert_api_key_unchecked(
        &self,
        api_key: &ApiKeyRecord,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO api_keys(
               id,identity_id,name,key_hash,key_prefix,status,revision,integrity_hmac,created_at,last_used_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&api_key.id)
        .bind(&api_key.identity_id)
        .bind(&api_key.name)
        .bind(&api_key.key_hash)
        .bind(&api_key.key_prefix)
        .bind(&api_key.status)
        .bind(api_key.revision)
        .bind(&api_key.integrity_hmac)
        .bind(&api_key.created_at)
        .bind(&api_key.last_used_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn insert_api_key_unchecked_and_audit(
        &self,
        api_key: &ApiKeyRecord,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        match self
            .insert_api_key_with_limit_and_audit(
                api_key,
                None,
                expected_audit_sequence,
                expected_audit_hmac,
                audit_event,
            )
            .await?
        {
            MutationWithAuditOutcome::Mutation(ResourceCreateOutcome::Created) => {
                Ok(AuditedMutationOutcome::Applied)
            }
            MutationWithAuditOutcome::Mutation(ResourceCreateOutcome::Conflict) => {
                Ok(AuditedMutationOutcome::MutationConflict)
            }
            MutationWithAuditOutcome::Mutation(ResourceCreateOutcome::LimitReached) => {
                Err(StorageError::DatabaseNotEmpty)
            }
            MutationWithAuditOutcome::AuditConflict => Ok(AuditedMutationOutcome::AuditConflict),
        }
    }

    pub async fn insert_api_key_with_limit_and_audit(
        &self,
        api_key: &ApiKeyRecord,
        active_limit: Option<u32>,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<MutationWithAuditOutcome<ResourceCreateOutcome>, StorageError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query_scalar::<MySql, u64>(
            "SELECT revision FROM transaction_gates WHERE gate_key='entity_quota' FOR UPDATE",
        )
        .fetch_one(&mut *transaction)
        .await?;
        if let Some(limit) = active_limit {
            let occupied = sqlx::query_scalar::<MySql, i64>(
                "SELECT count(*) FROM api_keys WHERE identity_id=? AND status='active'",
            )
            .bind(&api_key.identity_id)
            .fetch_one(&mut *transaction)
            .await?;
            if occupied >= i64::from(limit) {
                transaction.rollback().await?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    ResourceCreateOutcome::LimitReached,
                ));
            }
        }
        let inserted = sqlx::query(
            "INSERT INTO api_keys(
               id,identity_id,name,key_hash,key_prefix,status,revision,integrity_hmac,created_at,last_used_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&api_key.id)
        .bind(&api_key.identity_id)
        .bind(&api_key.name)
        .bind(&api_key.key_hash)
        .bind(&api_key.key_prefix)
        .bind(&api_key.status)
        .bind(api_key.revision)
        .bind(&api_key.integrity_hmac)
        .bind(&api_key.created_at)
        .bind(&api_key.last_used_at)
        .execute(&mut *transaction)
        .await;
        match inserted {
            Ok(result) if result.rows_affected() == 1 => {}
            Ok(_) => {
                transaction.rollback().await?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    ResourceCreateOutcome::Conflict,
                ));
            }
            Err(error) if is_mariadb_duplicate(&error) => {
                transaction.rollback().await?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    ResourceCreateOutcome::Conflict,
                ));
            }
            Err(error) => return Err(StorageError::MariaDb(error)),
        }
        if !append_audit_events_mariadb(
            &mut transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )
        .await?
        {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(MutationWithAuditOutcome::Mutation(
            ResourceCreateOutcome::Created,
        ))
    }

    pub async fn api_keys_for_identity(
        &self,
        identity_id: &str,
    ) -> Result<Vec<ApiKeyRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT id,identity_id,name,key_hash,key_prefix,status,revision,integrity_hmac,
                    created_at,last_used_at
             FROM api_keys WHERE identity_id=? ORDER BY created_at DESC,id",
        )
        .bind(identity_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(api_key_from_mariadb_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub async fn api_key_for_identity(
        &self,
        api_key_id: &str,
        identity_id: &str,
    ) -> Result<Option<ApiKeyRecord>, StorageError> {
        sqlx::query(
            "SELECT id,identity_id,name,key_hash,key_prefix,status,revision,integrity_hmac,
                    created_at,last_used_at
             FROM api_keys WHERE id=? AND identity_id=?",
        )
        .bind(api_key_id)
        .bind(identity_id)
        .fetch_optional(&self.pool)
        .await?
        .map(api_key_from_mariadb_row)
        .transpose()
        .map_err(StorageError::from)
    }

    pub async fn authorized_api_key_by_hash(
        &self,
        key_hash: &str,
    ) -> Result<Option<AuthorizedApiKey>, StorageError> {
        sqlx::query(
            "SELECT k.id,k.identity_id,k.name,k.key_hash,k.key_prefix,k.status,k.revision,
                    k.integrity_hmac,k.created_at,k.last_used_at,
                    i.id AS identity_id_value,i.email,i.display_name,i.password_hash,i.role,
                    i.status AS identity_status,i.can_consume_model,i.password_change_required,
                    i.revision AS identity_revision,i.integrity_hmac AS identity_integrity_hmac,
                    i.created_at AS identity_created_at,i.updated_at AS identity_updated_at
             FROM api_keys k JOIN identities i ON i.id=k.identity_id
             WHERE k.key_hash=? AND k.status='active' AND i.status='active'",
        )
        .bind(key_hash)
        .fetch_optional(&self.pool)
        .await?
        .map(authorized_api_key_from_mariadb_row)
        .transpose()
        .map_err(StorageError::from)
    }

    pub async fn update_api_key_status(
        &self,
        api_key_id: &str,
        identity_id: &str,
        expected_revision: u32,
        status: &str,
        integrity_hmac: &str,
    ) -> Result<bool, StorageError> {
        let result = sqlx::query(
            "UPDATE api_keys SET status=?,revision=revision+1,integrity_hmac=?
             WHERE id=? AND identity_id=? AND revision=?",
        )
        .bind(status)
        .bind(integrity_hmac)
        .bind(api_key_id)
        .bind(identity_id)
        .bind(expected_revision)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_api_key_status_and_audit(
        &self,
        api_key_id: &str,
        identity_id: &str,
        expected_revision: u32,
        status: &str,
        integrity_hmac: &str,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let mut transaction = self.pool.begin().await?;
        let changed = sqlx::query(
            "UPDATE api_keys SET status=?,revision=revision+1,integrity_hmac=?
             WHERE id=? AND identity_id=? AND revision=?",
        )
        .bind(status)
        .bind(integrity_hmac)
        .bind(api_key_id)
        .bind(identity_id)
        .bind(expected_revision)
        .execute(&mut *transaction)
        .await?
        .rows_affected()
            == 1;
        if !changed {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        if !append_audit_events_mariadb(
            &mut transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )
        .await?
        {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(AuditedMutationOutcome::Applied)
    }

    pub async fn touch_api_key_last_used(
        &self,
        api_key_id: &str,
        last_used_at: &str,
    ) -> Result<bool, StorageError> {
        let result =
            sqlx::query("UPDATE api_keys SET last_used_at=? WHERE id=? AND status='active'")
                .bind(last_used_at)
                .bind(api_key_id)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn upsert_model(&self, model: &ModelRecord) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO models(id,public_name,display_name,enabled,discovered_at,created_at)
             VALUES(?,?,?,?,?,?)
             ON DUPLICATE KEY UPDATE
               public_name=VALUES(public_name),
               display_name=VALUES(display_name),
               discovered_at=VALUES(discovered_at)",
        )
        .bind(&model.id)
        .bind(&model.public_name)
        .bind(&model.display_name)
        .bind(model.enabled)
        .bind(&model.discovered_at)
        .bind(&model.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn enabled_models(&self) -> Result<Vec<ModelRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT m.id,m.public_name,m.display_name,m.enabled,m.discovered_at,m.created_at
             FROM models m
             WHERE m.enabled=1 AND (EXISTS(
               SELECT 1 FROM account_models am
               JOIN upstream_accounts a ON a.id=am.account_id AND a.status='active'
               JOIN upstream_credential_instances c ON c.account_id=a.id AND c.status='active'
               WHERE am.model_id=m.id
             ) OR EXISTS(
               SELECT 1 FROM upstream_connection_models cm
               JOIN upstream_connections uc ON uc.id=cm.connection_id
                 AND uc.status='active' AND uc.auth_scheme='api_key'
               JOIN upstream_connection_credentials cc ON cc.connection_id=uc.id
                 AND cc.status='active'
               WHERE cm.model_id=m.id AND cm.enabled=1
             ))
             ORDER BY m.public_name",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(model_from_mariadb_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub async fn list_models(&self) -> Result<Vec<ModelRecord>, StorageError> {
        sqlx::query(
            "SELECT id,public_name,display_name,enabled,discovered_at,created_at
             FROM models ORDER BY public_name,id",
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(model_from_mariadb_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(StorageError::from)
    }

    pub async fn update_model_enabled(
        &self,
        model_id: &str,
        enabled: bool,
    ) -> Result<bool, StorageError> {
        Ok(sqlx::query("UPDATE models SET enabled=? WHERE id=?")
            .bind(enabled)
            .bind(model_id)
            .execute(&self.pool)
            .await?
            .rows_affected()
            == 1)
    }

    pub async fn update_model_enabled_and_audit(
        &self,
        model_id: &str,
        enabled: bool,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let mut transaction = self.pool.begin().await?;
        if sqlx::query("UPDATE models SET enabled=? WHERE id=?")
            .bind(enabled)
            .bind(model_id)
            .execute(&mut *transaction)
            .await?
            .rows_affected()
            != 1
        {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        if !append_audit_events_mariadb(
            &mut transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )
        .await?
        {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(AuditedMutationOutcome::Applied)
    }

    pub async fn enabled_model_by_public_name(
        &self,
        public_name: &str,
    ) -> Result<Option<ModelRecord>, StorageError> {
        sqlx::query(
            "SELECT m.id,m.public_name,m.display_name,m.enabled,m.discovered_at,m.created_at
             FROM models m
             WHERE m.public_name=? AND m.enabled=1 AND (EXISTS(
               SELECT 1 FROM account_models am
               JOIN upstream_accounts a ON a.id=am.account_id AND a.status='active'
               JOIN upstream_credential_instances c ON c.account_id=a.id AND c.status='active'
               WHERE am.model_id=m.id
             ) OR EXISTS(
               SELECT 1 FROM upstream_connection_models cm
               JOIN upstream_connections uc ON uc.id=cm.connection_id
                 AND uc.status='active' AND uc.auth_scheme='api_key'
               JOIN upstream_connection_credentials cc ON cc.connection_id=uc.id
                 AND cc.status='active'
               WHERE cm.model_id=m.id AND cm.enabled=1
             ))",
        )
        .bind(public_name)
        .fetch_optional(&self.pool)
        .await?
        .map(model_from_mariadb_row)
        .transpose()
        .map_err(StorageError::from)
    }

    pub async fn gateway_route_candidates(
        &self,
        public_name: &str,
    ) -> Result<Vec<GatewayRouteCandidate>, StorageError> {
        sqlx::query(
            "SELECT m.id AS model_id,m.public_name,am.upstream_name,a.id AS account_id,
                    a.provider,a.subject_id AS upstream_subject_id,a.last_success_runner_id,
                    c.id AS credential_id,c.account_id AS credential_account_id,
                    c.credential_identity_hmac,c.encrypted_payload,c.payload_nonce,
                    c.wrapped_data_key,c.wrap_nonce,c.credential_revision,c.expires_at,
                    c.status AS credential_status,c.last_refreshed_at,
                    c.created_at AS credential_created_at,c.updated_at AS credential_updated_at
             FROM models m
             JOIN account_models am ON am.model_id=m.id
             JOIN upstream_accounts a ON a.id=am.account_id AND a.status='active'
             JOIN upstream_credential_instances c ON c.account_id=a.id AND c.status='active'
             WHERE m.public_name=? AND m.enabled=1
             ORDER BY a.last_verified_at DESC,c.expires_at DESC,c.updated_at DESC,c.id ASC",
        )
        .bind(public_name)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(gateway_route_from_mariadb_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(StorageError::from)
    }

    pub async fn record_runner_heartbeat(
        &self,
        runner_id: &str,
        runner_version: &str,
        max_inflight: u32,
        heartbeat: RunnerHeartbeatUpdate,
        observed_at: &str,
    ) -> Result<bool, StorageError> {
        let result = sqlx::query(
            "UPDATE runners SET version=?,max_inflight=?,inflight=?,recent_request_count=?,
                    recent_error_count=?,latency_ms=?,last_seen_at=?,updated_at=?
             WHERE id=? AND enabled=1",
        )
        .bind(runner_version)
        .bind(max_inflight)
        .bind(heartbeat.inflight)
        .bind(heartbeat.recent_request_count)
        .bind(heartbeat.recent_error_count)
        .bind(heartbeat.latency_ms)
        .bind(observed_at)
        .bind(observed_at)
        .bind(runner_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn record_runner_connection(
        &self,
        update: &RunnerConnectionUpdate,
    ) -> Result<bool, StorageError> {
        if update.protocol_version < update.previous_protocol_version {
            return Ok(false);
        }
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query("SELECT protocol_version FROM runners WHERE id=? AND credential_hash=? AND enabled=1 FOR UPDATE")
            .bind(&update.runner_id).bind(&update.credential_hash).fetch_optional(&mut *transaction).await?;
        let Some(row) = row else {
            transaction.rollback().await?;
            return Ok(false);
        };
        let current: u32 = row.try_get("protocol_version")?;
        if current != update.previous_protocol_version && current != update.protocol_version {
            transaction.rollback().await?;
            return Ok(false);
        }
        // Return the locked authorization result, not MySQL's changed-row
        // count: a repeated valid hello/heartbeat may leave every value equal.
        sqlx::query("UPDATE runners SET protocol_version=?,version=?,max_inflight=?,inflight=?,recent_request_count=?,recent_error_count=?,latency_ms=?,last_seen_at=?,updated_at=? WHERE id=?")
            .bind(update.protocol_version).bind(&update.runner_version).bind(update.max_inflight)
            .bind(update.heartbeat.inflight).bind(update.heartbeat.recent_request_count)
            .bind(update.heartbeat.recent_error_count).bind(update.heartbeat.latency_ms)
            .bind(&update.observed_at).bind(&update.observed_at).bind(&update.runner_id)
            .execute(&mut *transaction).await?;
        transaction.commit().await?;
        Ok(true)
    }

    pub async fn insert_upstream_account_unchecked(
        &self,
        id: &str,
        provider: &str,
        subject_id: &str,
        email: &str,
        now: &str,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO upstream_accounts(id,provider,subject_id,email,created_at,updated_at) VALUES(?,?,?,?,?,?)",
        )
        .bind(id)
        .bind(provider)
        .bind(subject_id)
        .bind(email)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_upstream_account_unchecked_and_audit(
        &self,
        id: &str,
        provider: &str,
        subject_id: &str,
        email: &str,
        now: &str,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        match self
            .insert_upstream_account_with_limit_and_audit(
                id,
                provider,
                subject_id,
                email,
                now,
                None,
                expected_audit_sequence,
                expected_audit_hmac,
                audit_event,
            )
            .await?
        {
            MutationWithAuditOutcome::Mutation(ResourceCreateOutcome::Created) => {
                Ok(AuditedMutationOutcome::Applied)
            }
            MutationWithAuditOutcome::Mutation(ResourceCreateOutcome::Conflict) => {
                Ok(AuditedMutationOutcome::MutationConflict)
            }
            MutationWithAuditOutcome::Mutation(ResourceCreateOutcome::LimitReached) => {
                Err(StorageError::DatabaseNotEmpty)
            }
            MutationWithAuditOutcome::AuditConflict => Ok(AuditedMutationOutcome::AuditConflict),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_upstream_account_with_limit_and_audit(
        &self,
        id: &str,
        provider: &str,
        subject_id: &str,
        email: &str,
        now: &str,
        account_limit: Option<u32>,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<MutationWithAuditOutcome<ResourceCreateOutcome>, StorageError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query_scalar::<MySql, u64>(
            "SELECT revision FROM transaction_gates WHERE gate_key='entity_quota' FOR UPDATE",
        )
        .fetch_one(&mut *transaction)
        .await?;
        let account_exists = sqlx::query_scalar::<MySql, i64>(
            "SELECT EXISTS(
               SELECT 1 FROM upstream_accounts
               WHERE id=? OR (provider=? AND subject_id=?)
             )",
        )
        .bind(id)
        .bind(provider)
        .bind(subject_id)
        .fetch_one(&mut *transaction)
        .await?;
        if account_exists == 1 {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                ResourceCreateOutcome::Conflict,
            ));
        }
        if let Some(limit) = account_limit {
            let occupied =
                sqlx::query_scalar::<MySql, i64>("SELECT count(*) FROM upstream_accounts")
                    .fetch_one(&mut *transaction)
                    .await?;
            if occupied >= i64::from(limit) {
                transaction.rollback().await?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    ResourceCreateOutcome::LimitReached,
                ));
            }
        }
        let inserted = sqlx::query(
            "INSERT INTO upstream_accounts(id,provider,subject_id,email,created_at,updated_at)
             VALUES(?,?,?,?,?,?)",
        )
        .bind(id)
        .bind(provider)
        .bind(subject_id)
        .bind(email)
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await;
        match inserted {
            Ok(result) if result.rows_affected() == 1 => {}
            Ok(_) => {
                transaction.rollback().await?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    ResourceCreateOutcome::Conflict,
                ));
            }
            Err(error) if is_mariadb_duplicate(&error) => {
                transaction.rollback().await?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    ResourceCreateOutcome::Conflict,
                ));
            }
            Err(error) => return Err(StorageError::MariaDb(error)),
        }
        if !append_audit_events_mariadb(
            &mut transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )
        .await?
        {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(MutationWithAuditOutcome::Mutation(
            ResourceCreateOutcome::Created,
        ))
    }

    pub async fn upstream_account_by_provider_subject(
        &self,
        provider: &str,
        subject_id: &str,
    ) -> Result<Option<UpstreamAccountRecord>, StorageError> {
        sqlx::query(
            "SELECT id,provider,subject_id,email,plan,status,last_success_runner_id,
                    last_verified_at,created_at,updated_at
             FROM upstream_accounts WHERE provider=? AND subject_id=?",
        )
        .bind(provider)
        .bind(subject_id)
        .fetch_optional(&self.pool)
        .await?
        .map(upstream_account_from_mariadb_row)
        .transpose()
        .map_err(StorageError::from)
    }

    pub async fn upstream_account_by_id(
        &self,
        id: &str,
    ) -> Result<Option<UpstreamAccountRecord>, StorageError> {
        sqlx::query(
            "SELECT id,provider,subject_id,email,plan,status,last_success_runner_id,
                    last_verified_at,created_at,updated_at
             FROM upstream_accounts WHERE id=?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .map(upstream_account_from_mariadb_row)
        .transpose()
        .map_err(StorageError::from)
    }

    pub async fn list_upstream_accounts(&self) -> Result<Vec<UpstreamAccountRecord>, StorageError> {
        sqlx::query(
            "SELECT id,provider,subject_id,email,plan,status,last_success_runner_id,
                    last_verified_at,created_at,updated_at
             FROM upstream_accounts ORDER BY created_at,id",
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(upstream_account_from_mariadb_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(StorageError::from)
    }

    pub async fn update_upstream_account_metadata(
        &self,
        id: &str,
        email: &str,
        plan: &str,
        verified_at: &str,
    ) -> Result<bool, StorageError> {
        Ok(sqlx::query(
            "UPDATE upstream_accounts
             SET email=?,plan=?,status='active',last_verified_at=?,updated_at=?
             WHERE id=?",
        )
        .bind(email)
        .bind(plan)
        .bind(verified_at)
        .bind(verified_at)
        .bind(id)
        .execute(&self.pool)
        .await?
        .rows_affected()
            == 1)
    }

    pub async fn update_upstream_account_status(
        &self,
        id: &str,
        status: &str,
        updated_at: &str,
    ) -> Result<bool, StorageError> {
        Ok(
            sqlx::query("UPDATE upstream_accounts SET status=?,updated_at=? WHERE id=?")
                .bind(status)
                .bind(updated_at)
                .bind(id)
                .execute(&self.pool)
                .await?
                .rows_affected()
                == 1,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_upstream_account_status_and_audit(
        &self,
        id: &str,
        status: &str,
        updated_at: &str,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let mut transaction = self.pool.begin().await?;
        if sqlx::query("UPDATE upstream_accounts SET status=?,updated_at=? WHERE id=?")
            .bind(status)
            .bind(updated_at)
            .bind(id)
            .execute(&mut *transaction)
            .await?
            .rows_affected()
            != 1
        {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        if !append_audit_events_mariadb(
            &mut transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )
        .await?
        {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(AuditedMutationOutcome::Applied)
    }

    pub async fn delete_upstream_account(&self, id: &str) -> Result<bool, StorageError> {
        Ok(sqlx::query("DELETE FROM upstream_accounts WHERE id=?")
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected()
            == 1)
    }

    pub async fn delete_upstream_account_and_audit(
        &self,
        id: &str,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let mut transaction = self.pool.begin().await?;
        if sqlx::query("DELETE FROM upstream_accounts WHERE id=?")
            .bind(id)
            .execute(&mut *transaction)
            .await?
            .rows_affected()
            != 1
        {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        if !append_audit_events_mariadb(
            &mut transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )
        .await?
        {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(AuditedMutationOutcome::Applied)
    }

    pub async fn upstream_account_provider(
        &self,
        id: &str,
    ) -> Result<Option<String>, StorageError> {
        sqlx::query_scalar::<MySql, String>("SELECT provider FROM upstream_accounts WHERE id=?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(StorageError::from)
    }

    pub async fn replace_account_models(
        &self,
        account_id: &str,
        models: &[DiscoveredModel],
        discovered_at: &str,
    ) -> Result<(), StorageError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM account_models WHERE account_id=?")
            .bind(account_id)
            .execute(&mut *transaction)
            .await?;
        for model in models {
            sqlx::query(
                "INSERT INTO models(id,public_name,display_name,enabled,discovered_at,created_at)
                 VALUES(?,?,?,1,?,?)
                 ON DUPLICATE KEY UPDATE
                   display_name=VALUES(display_name),discovered_at=VALUES(discovered_at)",
            )
            .bind(&model.id)
            .bind(&model.public_name)
            .bind(&model.display_name)
            .bind(discovered_at)
            .bind(discovered_at)
            .execute(&mut *transaction)
            .await?;
            let model_id =
                sqlx::query_scalar::<MySql, String>("SELECT id FROM models WHERE public_name=?")
                    .bind(&model.public_name)
                    .fetch_one(&mut *transaction)
                    .await?;
            sqlx::query(
                "INSERT INTO account_models(account_id,model_id,upstream_name,discovered_at)
                 VALUES(?,?,?,?)",
            )
            .bind(account_id)
            .bind(model_id)
            .bind(&model.upstream_name)
            .bind(discovered_at)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn replace_account_models_and_record_success_and_audit(
        &self,
        account_id: &str,
        models: &[DiscoveredModel],
        discovered_at: &str,
        runner_id: &str,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM account_models WHERE account_id=?")
            .bind(account_id)
            .execute(&mut *transaction)
            .await?;
        for model in models {
            sqlx::query(
                "INSERT INTO models(id,public_name,display_name,enabled,discovered_at,created_at)
                 VALUES(?,?,?,1,?,?)
                 ON DUPLICATE KEY UPDATE
                   display_name=VALUES(display_name),discovered_at=VALUES(discovered_at)",
            )
            .bind(&model.id)
            .bind(&model.public_name)
            .bind(&model.display_name)
            .bind(discovered_at)
            .bind(discovered_at)
            .execute(&mut *transaction)
            .await?;
            let model_id =
                sqlx::query_scalar::<MySql, String>("SELECT id FROM models WHERE public_name=?")
                    .bind(&model.public_name)
                    .fetch_one(&mut *transaction)
                    .await?;
            sqlx::query(
                "INSERT INTO account_models(account_id,model_id,upstream_name,discovered_at)
                 VALUES(?,?,?,?)",
            )
            .bind(account_id)
            .bind(model_id)
            .bind(&model.upstream_name)
            .bind(discovered_at)
            .execute(&mut *transaction)
            .await?;
        }
        if sqlx::query(
            "UPDATE upstream_accounts
             SET last_success_runner_id=?,last_verified_at=?,updated_at=?
             WHERE id=? AND status='active'",
        )
        .bind(runner_id)
        .bind(discovered_at)
        .bind(discovered_at)
        .bind(account_id)
        .execute(&mut *transaction)
        .await?
        .rows_affected()
            != 1
        {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        if !append_audit_events_mariadb(
            &mut transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )
        .await?
        {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(AuditedMutationOutcome::Applied)
    }

    pub async fn update_account_last_success_runner(
        &self,
        account_id: &str,
        runner_id: &str,
        updated_at: &str,
    ) -> Result<bool, StorageError> {
        Ok(sqlx::query(
            "UPDATE upstream_accounts
             SET last_success_runner_id=?,last_verified_at=?,updated_at=?
             WHERE id=? AND status='active'",
        )
        .bind(runner_id)
        .bind(updated_at)
        .bind(updated_at)
        .bind(account_id)
        .execute(&self.pool)
        .await?
        .rows_affected()
            == 1)
    }

    pub async fn insert_credential_instance(
        &self,
        credential: &EncryptedCredentialInstance,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO upstream_credential_instances(
               id,account_id,credential_identity_hmac,encrypted_payload,payload_nonce,wrapped_data_key,wrap_nonce,
               credential_revision,expires_at,status,last_refreshed_at,created_at,updated_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&credential.id)
        .bind(&credential.account_id)
        .bind(&credential.credential_identity_hmac)
        .bind(&credential.encrypted_payload)
        .bind(&credential.payload_nonce)
        .bind(&credential.wrapped_data_key)
        .bind(&credential.wrap_nonce)
        .bind(credential.credential_revision)
        .bind(&credential.expires_at)
        .bind(&credential.status)
        .bind(&credential.last_refreshed_at)
        .bind(&credential.created_at)
        .bind(&credential.updated_at)
        .execute(&self.pool)
        .await
        .map_err(map_mariadb_credential_write_error)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_account_metadata_and_insert_credential_and_audit(
        &self,
        account_id: &str,
        email: &str,
        plan: &str,
        verified_at: &str,
        credential: &EncryptedCredentialInstance,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        if credential.account_id != account_id {
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        let mut transaction = self.pool.begin().await?;
        if sqlx::query(
            "UPDATE upstream_accounts
             SET email=?,plan=?,status='active',last_verified_at=?,updated_at=?
             WHERE id=?",
        )
        .bind(email)
        .bind(plan)
        .bind(verified_at)
        .bind(verified_at)
        .bind(account_id)
        .execute(&mut *transaction)
        .await?
        .rows_affected()
            != 1
        {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        sqlx::query(
            "INSERT INTO upstream_credential_instances(
               id,account_id,credential_identity_hmac,encrypted_payload,payload_nonce,
               wrapped_data_key,wrap_nonce,credential_revision,expires_at,status,
               last_refreshed_at,created_at,updated_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&credential.id)
        .bind(&credential.account_id)
        .bind(&credential.credential_identity_hmac)
        .bind(&credential.encrypted_payload)
        .bind(&credential.payload_nonce)
        .bind(&credential.wrapped_data_key)
        .bind(&credential.wrap_nonce)
        .bind(credential.credential_revision)
        .bind(&credential.expires_at)
        .bind(&credential.status)
        .bind(&credential.last_refreshed_at)
        .bind(&credential.created_at)
        .bind(&credential.updated_at)
        .execute(&mut *transaction)
        .await
        .map_err(map_mariadb_credential_write_error)?;
        if !append_audit_events_mariadb(
            &mut transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )
        .await?
        {
            transaction.rollback().await?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(AuditedMutationOutcome::Applied)
    }

    pub async fn credential_instance_by_id(
        &self,
        id: &str,
    ) -> Result<Option<EncryptedCredentialInstance>, StorageError> {
        let row = sqlx::query(
            "SELECT id,account_id,credential_identity_hmac,encrypted_payload,payload_nonce,
                    wrapped_data_key,wrap_nonce,credential_revision,expires_at,status,
                    last_refreshed_at,created_at,updated_at
             FROM upstream_credential_instances WHERE id=?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(credential_from_mariadb_row)
            .transpose()
            .map_err(StorageError::from)
    }

    pub async fn credential_instances_for_account(
        &self,
        account_id: &str,
    ) -> Result<Vec<EncryptedCredentialInstance>, StorageError> {
        let rows = sqlx::query(
            "SELECT id,account_id,credential_identity_hmac,encrypted_payload,payload_nonce,wrapped_data_key,wrap_nonce,
                    credential_revision,expires_at,status,last_refreshed_at,created_at,updated_at
             FROM upstream_credential_instances WHERE account_id=? ORDER BY created_at,id",
        )
        .bind(account_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(credential_from_mariadb_row)
            .collect::<Result<Vec<_>, sqlx::Error>>()
            .map_err(StorageError::from)
    }

    pub async fn update_credential_after_refresh(
        &self,
        id: &str,
        expected_revision: u32,
        lease_token_hash: &str,
        now: &str,
        update: &CredentialRefreshUpdate,
    ) -> Result<bool, StorageError> {
        match self
            .update_credential_after_refresh_inner(
                id,
                expected_revision,
                lease_token_hash,
                now,
                update,
                None,
            )
            .await?
        {
            MutationWithAuditOutcome::Mutation(changed) => Ok(changed),
            MutationWithAuditOutcome::AuditConflict => Err(StorageError::DatabaseNotEmpty),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_credential_after_refresh_and_audit(
        &self,
        id: &str,
        expected_revision: u32,
        lease_token_hash: &str,
        now: &str,
        update: &CredentialRefreshUpdate,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<MutationWithAuditOutcome<bool>, StorageError> {
        self.update_credential_after_refresh_inner(
            id,
            expected_revision,
            lease_token_hash,
            now,
            update,
            Some((expected_audit_sequence, expected_audit_hmac, audit_event)),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn update_credential_after_refresh_inner(
        &self,
        id: &str,
        expected_revision: u32,
        lease_token_hash: &str,
        now: &str,
        update: &CredentialRefreshUpdate,
        audit: Option<(u64, &str, &AuditEventRecord)>,
    ) -> Result<MutationWithAuditOutcome<bool>, StorageError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "DELETE FROM credential_refresh_leases
             WHERE credential_instance_id=? AND expires_at<=?",
        )
        .bind(id)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        let result = sqlx::query(
            "UPDATE upstream_credential_instances
             SET credential_identity_hmac=?,encrypted_payload=?,payload_nonce=?,wrapped_data_key=?,wrap_nonce=?,expires_at=?,
                 credential_revision=credential_revision+1,status='active',last_refreshed_at=?,updated_at=?
             WHERE id=? AND credential_revision=? AND EXISTS(
               SELECT 1 FROM credential_refresh_leases l
               WHERE l.credential_instance_id=upstream_credential_instances.id
                 AND l.lease_token_hash=? AND l.expected_revision=? AND l.expires_at>?
             )",
        )
        .bind(&update.credential_identity_hmac)
        .bind(&update.encrypted_payload)
        .bind(&update.payload_nonce)
        .bind(&update.wrapped_data_key)
        .bind(&update.wrap_nonce)
        .bind(&update.expires_at)
        .bind(&update.refreshed_at)
        .bind(&update.refreshed_at)
        .bind(id)
        .bind(expected_revision)
        .bind(lease_token_hash)
        .bind(expected_revision)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(map_mariadb_credential_write_error)?;
        sqlx::query(
            "DELETE FROM credential_refresh_leases
             WHERE credential_instance_id=? AND lease_token_hash=?",
        )
        .bind(id)
        .bind(lease_token_hash)
        .execute(&mut *transaction)
        .await?;
        let changed = result.rows_affected() == 1;
        if changed
            && let Some((expected_sequence, expected_hmac, event)) = audit
            && !append_audit_events_mariadb(
                &mut transaction,
                expected_sequence,
                expected_hmac,
                std::slice::from_ref(event),
            )
            .await?
        {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(MutationWithAuditOutcome::Mutation(changed))
    }

    pub async fn acquire_credential_refresh_lease(
        &self,
        credential_id: &str,
        expected_revision: u32,
        lease_token_hash: &str,
        now: &str,
        expires_at: &str,
    ) -> Result<CredentialRefreshLeaseOutcome, StorageError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM credential_refresh_leases WHERE expires_at<=?")
            .bind(now)
            .execute(&mut *transaction)
            .await?;
        let credential = sqlx::query(
            "SELECT credential_revision,status FROM upstream_credential_instances WHERE id=? FOR UPDATE",
        )
        .bind(credential_id)
        .fetch_optional(&mut *transaction)
        .await?;
        let outcome = match credential {
            None => CredentialRefreshLeaseOutcome::CredentialUnavailable,
            Some(row) => {
                let revision: u32 = row.try_get("credential_revision")?;
                let status: String = row.try_get("status")?;
                if status != "active" {
                    CredentialRefreshLeaseOutcome::CredentialUnavailable
                } else if revision != expected_revision {
                    CredentialRefreshLeaseOutcome::RevisionChanged
                } else {
                    let occupied = sqlx::query(
                        "SELECT 1 FROM credential_refresh_leases
                         WHERE credential_instance_id=? FOR UPDATE",
                    )
                    .bind(credential_id)
                    .fetch_optional(&mut *transaction)
                    .await?
                    .is_some();
                    if occupied {
                        CredentialRefreshLeaseOutcome::Busy
                    } else {
                        sqlx::query(
                            "INSERT INTO credential_refresh_leases(
                               credential_instance_id,lease_token_hash,expected_revision,expires_at,created_at
                             ) VALUES(?,?,?,?,?)",
                        )
                        .bind(credential_id)
                        .bind(lease_token_hash)
                        .bind(expected_revision)
                        .bind(expires_at)
                        .bind(now)
                        .execute(&mut *transaction)
                        .await?;
                        CredentialRefreshLeaseOutcome::Acquired
                    }
                }
            }
        };
        transaction.commit().await?;
        Ok(outcome)
    }

    pub async fn release_credential_refresh_lease(
        &self,
        credential_id: &str,
        lease_token_hash: &str,
    ) -> Result<bool, StorageError> {
        let result = sqlx::query(
            "DELETE FROM credential_refresh_leases
             WHERE credential_instance_id=? AND lease_token_hash=?",
        )
        .bind(credential_id)
        .bind(lease_token_hash)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    async fn apply_migrations(&self, migrations: &[EmbeddedMigration]) -> Result<(), StorageError> {
        self.apply_migrations_with_policy(migrations, MariaDbMigrationPolicy::Maintenance)
            .await
    }

    async fn apply_migrations_with_policy(
        &self,
        migrations: &[EmbeddedMigration],
        policy: MariaDbMigrationPolicy,
    ) -> Result<(), StorageError> {
        policy
            .bounded(self.apply_migrations_policy(migrations, policy))
            .await
    }

    async fn apply_migrations_policy(
        &self,
        migrations: &[EmbeddedMigration],
        policy: MariaDbMigrationPolicy,
    ) -> Result<(), StorageError> {
        validate_migration_sequence(migrations)?;
        if migrations.is_empty() {
            return Err(StorageError::MigrationIntegrity);
        }
        // A cancelled migration must close its session, not return an advisory-locked
        // connection to the pool. MariaDB releases GET_LOCK when this connection closes.
        let mut connection = self.pool.acquire().await?.detach();
        let acquired = sqlx::query_scalar::<MySql, Option<i64>>(
            "SELECT GET_LOCK('aster_team_schema_migrations', ?)",
        )
        .bind(policy.lock_wait_seconds())
        .fetch_one(&mut connection)
        .await?
        .unwrap_or_default();
        if acquired != 1 {
            return Err(StorageError::MigrationIntegrity);
        }
        let result = self
            .apply_migrations_locked(migrations, policy, &mut connection)
            .await;
        let released = sqlx::query_scalar::<MySql, Option<i64>>(
            "SELECT RELEASE_LOCK('aster_team_schema_migrations')",
        )
        .fetch_one(&mut connection)
        .await;
        result?;
        if released?.unwrap_or_default() != 1 {
            return Err(StorageError::MigrationIntegrity);
        }
        Ok(())
    }

    async fn apply_migrations_locked(
        &self,
        migrations: &[EmbeddedMigration],
        policy: MariaDbMigrationPolicy,
        connection: &mut MySqlConnection,
    ) -> Result<(), StorageError> {
        if !has_user_tables_on(connection).await? {
            if policy != MariaDbMigrationPolicy::Maintenance {
                return Err(StorageError::Uninitialized);
            }
            let baseline = migrations[0];
            sqlx::raw_sql(baseline.sql)
                .execute(&mut *connection)
                .await?;
            self.record_migration(baseline, connection).await?;
        } else if !self.has_table("schema_migrations", connection).await? {
            return Err(StorageError::DatabaseNotEmpty);
        }

        let history: Vec<migration_inspection::AppliedMigration> = sqlx::query_as(
            "SELECT version,name,checksum_sha256 FROM schema_migrations ORDER BY version LIMIT 1025",
        ).fetch_all(&mut *connection).await?;
        migration_inspection::verify_applied_history(&history, migrations)?;
        self.verify_known_migrations(migrations, connection).await?;
        let pending = &migrations[history.len().min(migrations.len())..];
        if policy == MariaDbMigrationPolicy::Existing {
            return if let Some(migration) = pending.first() {
                Err(StorageError::UnsupportedSchema {
                    version: migration.version,
                })
            } else {
                Ok(())
            };
        }
        // Reject the whole pending plan before applying even its first statement.
        policy.validate_pending(pending)?;
        policy.configure(connection).await?;
        for migration in migrations {
            if self
                .migration_record(migration.version, connection)
                .await?
                .is_some()
            {
                continue;
            }
            if migration.version == migrations[0].version {
                return Err(StorageError::MigrationIntegrity);
            }
            migration_recovery::apply_or_reconcile(*migration, policy.sql(*migration)?, connection)
                .await?;
            self.record_migration(*migration, connection).await?;
        }
        self.verify_known_migrations(migrations, connection).await
    }

    async fn verify_known_migrations(
        &self,
        migrations: &[EmbeddedMigration],
        connection: &mut MySqlConnection,
    ) -> Result<(), StorageError> {
        for migration in migrations {
            let Some((name, checksum)) =
                self.migration_record(migration.version, connection).await?
            else {
                if migration.version == migrations[0].version {
                    return Err(StorageError::UnsupportedSchema {
                        version: migration.version,
                    });
                }
                continue;
            };
            if name != migration.name || checksum != migration_checksum(migration.sql) {
                return Err(StorageError::UnsupportedSchema {
                    version: migration.version,
                });
            }
        }
        Ok(())
    }

    async fn migration_record(
        &self,
        version: u32,
        connection: &mut MySqlConnection,
    ) -> Result<Option<(String, String)>, StorageError> {
        let record =
            sqlx::query("SELECT name,checksum_sha256 FROM schema_migrations WHERE version=?")
                .bind(version)
                .fetch_optional(&mut *connection)
                .await?
                .map(|row| Ok::<_, sqlx::Error>((row.try_get(0)?, row.try_get(1)?)))
                .transpose()?;
        Ok(record)
    }

    async fn record_migration(
        &self,
        migration: EmbeddedMigration,
        connection: &mut MySqlConnection,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO schema_migrations(version,name,checksum_sha256,applied_at)
             VALUES(?,?,?,DATE_FORMAT(UTC_TIMESTAMP(), '%Y-%m-%dT%H:%i:%s.000Z'))",
        )
        .bind(migration.version)
        .bind(migration.name)
        .bind(migration_checksum(migration.sql))
        .execute(&mut *connection)
        .await?;
        Ok(())
    }

    async fn has_table(
        &self,
        name: &str,
        connection: &mut MySqlConnection,
    ) -> Result<bool, StorageError> {
        let count = sqlx::query_scalar::<MySql, i64>(
            "SELECT count(*) FROM information_schema.tables
             WHERE table_schema=DATABASE() AND table_name=?",
        )
        .bind(name)
        .fetch_one(&mut *connection)
        .await?;
        Ok(count == 1)
    }
}

async fn has_user_tables_on(connection: &mut MySqlConnection) -> Result<bool, StorageError> {
    let count = sqlx::query_scalar::<MySql, i64>(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema=DATABASE()",
    )
    .fetch_one(connection)
    .await?;
    Ok(count > 0)
}

pub(crate) async fn append_audit_events_mariadb(
    transaction: &mut sqlx::Transaction<'_, MySql>,
    expected_sequence: u64,
    expected_hmac: &str,
    events: &[AuditEventRecord],
) -> Result<bool, StorageError> {
    if !valid_audit_event_batch(expected_sequence, expected_hmac, events) {
        return Ok(false);
    }
    sqlx::query_scalar::<MySql, u64>(
        "SELECT revision FROM transaction_gates WHERE gate_key='audit_log' FOR UPDATE",
    )
    .fetch_one(&mut **transaction)
    .await?;
    let current = sqlx::query(
        "SELECT event_sequence,integrity_hmac
         FROM audit_events ORDER BY event_sequence DESC LIMIT 1",
    )
    .fetch_optional(&mut **transaction)
    .await?
    .map(|row| {
        Ok::<_, sqlx::Error>((
            row.try_get("event_sequence")?,
            row.try_get("integrity_hmac")?,
        ))
    })
    .transpose()?
    .unwrap_or((0, String::new()));
    if current.0 != expected_sequence || current.1 != expected_hmac {
        return Ok(false);
    }
    for event in events {
        let inserted = sqlx::query(
            "INSERT INTO audit_events(
               id,event_sequence,actor_identity_id,actor_role,action,target_type,target_id,
               outcome,previous_event_hmac,integrity_hmac,created_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&event.id)
        .bind(event.sequence)
        .bind(&event.actor_identity_id)
        .bind(&event.actor_role)
        .bind(&event.action)
        .bind(&event.target_type)
        .bind(&event.target_id)
        .bind(&event.outcome)
        .bind(&event.previous_event_hmac)
        .bind(&event.integrity_hmac)
        .bind(&event.created_at)
        .execute(&mut **transaction)
        .await;
        match inserted {
            Ok(result) if result.rows_affected() == 1 => {}
            Ok(_) => return Ok(false),
            Err(error) if is_mariadb_duplicate(&error) => return Ok(false),
            Err(error) => return Err(StorageError::MariaDb(error)),
        }
    }
    let revision_delta = u64::try_from(events.len()).map_err(|_| StorageError::DatabaseNotEmpty)?;
    let changed =
        sqlx::query("UPDATE transaction_gates SET revision=revision+? WHERE gate_key='audit_log'")
            .bind(revision_delta)
            .execute(&mut **transaction)
            .await?
            .rows_affected();
    Ok(changed == 1)
}

pub(crate) fn checked_i64(value: u64) -> Result<i64, sqlx::Error> {
    i64::try_from(value).map_err(|error| sqlx::Error::Decode(Box::new(error)))
}

fn money_balance_from_mariadb_row(
    row: sqlx::mysql::MySqlRow,
) -> Result<MoneyBalanceRecord, sqlx::Error> {
    Ok(MoneyBalanceRecord {
        identity_id: row.try_get("identity_id")?,
        currency: row.try_get("currency")?,
        balance_nanos: row.try_get("balance_nanos")?,
        credited_nanos: row.try_get("credited_nanos")?,
        debited_nanos: row.try_get("debited_nanos")?,
        revision: row.try_get("revision")?,
        last_ledger_hmac: row.try_get("last_ledger_hmac")?,
        integrity_hmac: row.try_get("integrity_hmac")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn money_ledger_from_mariadb_row(
    row: sqlx::mysql::MySqlRow,
) -> Result<MoneyLedgerEntry, sqlx::Error> {
    Ok(MoneyLedgerEntry {
        id: row.try_get("id")?,
        identity_id: row.try_get("identity_id")?,
        currency: row.try_get("currency")?,
        kind: row.try_get("kind")?,
        amount_nanos: row.try_get("amount_nanos")?,
        balance_revision: row.try_get("balance_revision")?,
        reference_id: row.try_get("reference_id")?,
        billing_status: row.try_get("billing_status")?,
        details_json: row.try_get("details_json")?,
        previous_entry_hmac: row.try_get("previous_entry_hmac")?,
        integrity_hmac: row.try_get("integrity_hmac")?,
        created_at: row.try_get("created_at")?,
    })
}

fn balance_from_mariadb_row(row: sqlx::mysql::MySqlRow) -> Result<UserBalanceRecord, sqlx::Error> {
    Ok(UserBalanceRecord {
        identity_id: row.try_get("identity_id")?,
        balance_tokens: row.try_get("balance_tokens")?,
        reserved_tokens: checked_i64(row.try_get::<u64, _>("reserved_tokens")?)?,
        granted_tokens: checked_i64(row.try_get::<u64, _>("granted_tokens")?)?,
        consumed_tokens: checked_i64(row.try_get::<u64, _>("consumed_tokens")?)?,
        request_count: checked_i64(row.try_get::<u64, _>("request_count")?)?,
        raw_tokens: checked_i64(row.try_get::<u64, _>("raw_tokens")?)?,
        billed_tokens: checked_i64(row.try_get::<u64, _>("billed_tokens")?)?,
        uncached_input: checked_i64(row.try_get::<u64, _>("uncached_input")?)?,
        cached_input: checked_i64(row.try_get::<u64, _>("cached_input")?)?,
        cache_write: checked_i64(row.try_get::<u64, _>("cache_write")?)?,
        output_tokens: checked_i64(row.try_get::<u64, _>("output_tokens")?)?,
        revision: row.try_get("revision")?,
        last_ledger_hmac: row.try_get("last_ledger_hmac")?,
        integrity_hmac: row.try_get("integrity_hmac")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn quota_reservation_from_mariadb_row(
    row: sqlx::mysql::MySqlRow,
) -> Result<QuotaReservationRecord, sqlx::Error> {
    Ok(QuotaReservationRecord {
        id: row.try_get("id")?,
        identity_id: row.try_get("identity_id")?,
        api_key_id: row.try_get("api_key_id")?,
        request_id: row.try_get("request_id")?,
        reserved_tokens: checked_i64(row.try_get::<u64, _>("reserved_tokens")?)?,
        status: row.try_get("status")?,
        revision: row.try_get("revision")?,
        integrity_hmac: row.try_get("integrity_hmac")?,
        created_at: row.try_get("created_at")?,
        expires_at: row.try_get("expires_at")?,
        settled_at: row.try_get("settled_at")?,
    })
}

fn quota_ledger_from_mariadb_row(
    row: sqlx::mysql::MySqlRow,
) -> Result<QuotaLedgerEntry, sqlx::Error> {
    Ok(QuotaLedgerEntry {
        id: row.try_get("id")?,
        identity_id: row.try_get("identity_id")?,
        kind: row.try_get("kind")?,
        amount_tokens: row.try_get("amount_tokens")?,
        uncached_input: checked_i64(row.try_get::<u64, _>("uncached_input")?)?,
        cached_input: checked_i64(row.try_get::<u64, _>("cached_input")?)?,
        cache_write: checked_i64(row.try_get::<u64, _>("cache_write")?)?,
        output_tokens: checked_i64(row.try_get::<u64, _>("output_tokens")?)?,
        uncovered_tokens: checked_i64(row.try_get::<u64, _>("uncovered_tokens")?)?,
        raw_tokens: checked_i64(row.try_get::<u64, _>("raw_tokens")?)?,
        billed_tokens: checked_i64(row.try_get::<u64, _>("billed_tokens")?)?,
        multiplier_micros: checked_i64(row.try_get::<u64, _>("multiplier_micros")?)?,
        reference_id: row
            .try_get::<Option<String>, _>("reference_id")?
            .unwrap_or_default(),
        client_request_id: row.try_get("client_request_id")?,
        description: row.try_get("description")?,
        protocol: row.try_get("protocol")?,
        model: row.try_get("model")?,
        requested_model: row.try_get("requested_model")?,
        processing_tier: row.try_get("processing_tier")?,
        reasoning_effort: row.try_get("reasoning_effort")?,
        api_key_id: row.try_get("api_key_id")?,
        runner_id: row.try_get("runner_id")?,
        previous_entry_hmac: row.try_get("previous_entry_hmac")?,
        integrity_hmac: row.try_get("integrity_hmac")?,
        created_at: row.try_get("created_at")?,
    })
}

async fn insert_ledger_mariadb(
    transaction: &mut sqlx::Transaction<'_, MySql>,
    ledger: &QuotaLedgerEntry,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO ledger_entries(
           id,identity_id,kind,amount_tokens,uncached_input,cached_input,cache_write,
           output_tokens,uncovered_tokens,raw_tokens,billed_tokens,multiplier_micros,
           reference_id,client_request_id,description,protocol,model,requested_model,processing_tier,
           reasoning_effort,api_key_id,runner_id,
           previous_entry_hmac,integrity_hmac,created_at
         ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&ledger.id)
    .bind(&ledger.identity_id)
    .bind(&ledger.kind)
    .bind(ledger.amount_tokens)
    .bind(ledger.uncached_input)
    .bind(ledger.cached_input)
    .bind(ledger.cache_write)
    .bind(ledger.output_tokens)
    .bind(ledger.uncovered_tokens)
    .bind(ledger.raw_tokens)
    .bind(ledger.billed_tokens)
    .bind(ledger.multiplier_micros)
    .bind(&ledger.reference_id)
    .bind(&ledger.client_request_id)
    .bind(&ledger.description)
    .bind(&ledger.protocol)
    .bind(&ledger.model)
    .bind(&ledger.requested_model)
    .bind(&ledger.processing_tier)
    .bind(&ledger.reasoning_effort)
    .bind(&ledger.api_key_id)
    .bind(&ledger.runner_id)
    .bind(&ledger.previous_entry_hmac)
    .bind(&ledger.integrity_hmac)
    .bind(&ledger.created_at)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn update_balance_mariadb(
    transaction: &mut sqlx::Transaction<'_, MySql>,
    expected: &UserBalanceRecord,
    next: &UserBalanceRecord,
) -> Result<bool, StorageError> {
    let changed = sqlx::query(
        "UPDATE user_balances SET
           balance_tokens=?,reserved_tokens=?,granted_tokens=?,consumed_tokens=?,
           request_count=?,raw_tokens=?,billed_tokens=?,uncached_input=?,cached_input=?,
           cache_write=?,output_tokens=?,revision=?,last_ledger_hmac=?,integrity_hmac=?,updated_at=?
         WHERE identity_id=? AND revision=? AND integrity_hmac=?",
    )
    .bind(next.balance_tokens)
    .bind(next.reserved_tokens)
    .bind(next.granted_tokens)
    .bind(next.consumed_tokens)
    .bind(next.request_count)
    .bind(next.raw_tokens)
    .bind(next.billed_tokens)
    .bind(next.uncached_input)
    .bind(next.cached_input)
    .bind(next.cache_write)
    .bind(next.output_tokens)
    .bind(next.revision)
    .bind(&next.last_ledger_hmac)
    .bind(&next.integrity_hmac)
    .bind(&next.updated_at)
    .bind(&expected.identity_id)
    .bind(expected.revision)
    .bind(&expected.integrity_hmac)
    .execute(&mut **transaction)
    .await?
    .rows_affected();
    Ok(changed == 1)
}

fn is_mariadb_duplicate(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|error| error.code())
        .is_some_and(|code| code == "1062")
}

async fn insert_initial_balance_mariadb(
    transaction: &mut sqlx::Transaction<'_, MySql>,
    identity: &IdentityRecord,
    balance: &UserBalanceRecord,
) -> Result<(), StorageError> {
    if balance.identity_id != identity.id
        || balance.balance_tokens != 0
        || balance.reserved_tokens != 0
        || balance.granted_tokens != 0
        || balance.consumed_tokens != 0
        || balance.request_count != 0
        || balance.raw_tokens != 0
        || balance.billed_tokens != 0
        || balance.uncached_input != 0
        || balance.cached_input != 0
        || balance.cache_write != 0
        || balance.output_tokens != 0
        || balance.revision != 0
        || !balance.last_ledger_hmac.is_empty()
        || balance.integrity_hmac.is_empty()
        || balance.updated_at != identity.created_at
    {
        return Err(StorageError::DatabaseNotEmpty);
    }
    sqlx::query(
        "INSERT INTO user_balances(
           identity_id,balance_tokens,reserved_tokens,granted_tokens,consumed_tokens,
           request_count,raw_tokens,billed_tokens,uncached_input,cached_input,cache_write,
           output_tokens,revision,last_ledger_hmac,integrity_hmac,updated_at
         ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&balance.identity_id)
    .bind(balance.balance_tokens)
    .bind(balance.reserved_tokens)
    .bind(balance.granted_tokens)
    .bind(balance.consumed_tokens)
    .bind(balance.request_count)
    .bind(balance.raw_tokens)
    .bind(balance.billed_tokens)
    .bind(balance.uncached_input)
    .bind(balance.cached_input)
    .bind(balance.cache_write)
    .bind(balance.output_tokens)
    .bind(balance.revision)
    .bind(&balance.last_ledger_hmac)
    .bind(&balance.integrity_hmac)
    .bind(&balance.updated_at)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn credential_from_mariadb_row(
    row: sqlx::mysql::MySqlRow,
) -> Result<EncryptedCredentialInstance, sqlx::Error> {
    Ok(EncryptedCredentialInstance {
        id: row.try_get("id")?,
        account_id: row.try_get("account_id")?,
        credential_identity_hmac: row.try_get("credential_identity_hmac")?,
        encrypted_payload: row.try_get("encrypted_payload")?,
        payload_nonce: row.try_get("payload_nonce")?,
        wrapped_data_key: row.try_get("wrapped_data_key")?,
        wrap_nonce: row.try_get("wrap_nonce")?,
        credential_revision: row.try_get("credential_revision")?,
        expires_at: row.try_get("expires_at")?,
        status: row.try_get("status")?,
        last_refreshed_at: row.try_get("last_refreshed_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn upstream_account_from_mariadb_row(
    row: sqlx::mysql::MySqlRow,
) -> Result<UpstreamAccountRecord, sqlx::Error> {
    Ok(UpstreamAccountRecord {
        id: row.try_get("id")?,
        provider: row.try_get("provider")?,
        subject_id: row.try_get("subject_id")?,
        email: row.try_get("email")?,
        plan: row.try_get("plan")?,
        status: row.try_get("status")?,
        last_success_runner_id: row.try_get("last_success_runner_id")?,
        last_verified_at: row.try_get("last_verified_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn runner_admin_from_mariadb_row(
    row: sqlx::mysql::MySqlRow,
) -> Result<RunnerAdminRecord, sqlx::Error> {
    Ok(RunnerAdminRecord {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        enabled: row.try_get::<u8, _>("enabled")? == 1,
        version: row.try_get("version")?,
        protocol_version: row.try_get("protocol_version")?,
        platform: row.try_get("platform")?,
        architecture: row.try_get("architecture")?,
        max_inflight: row.try_get("max_inflight")?,
        inflight: row.try_get("inflight")?,
        recent_request_count: row.try_get("recent_request_count")?,
        recent_error_count: row.try_get("recent_error_count")?,
        latency_ms: row.try_get("latency_ms")?,
        last_seen_at: row.try_get("last_seen_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn identity_from_mariadb_row(row: sqlx::mysql::MySqlRow) -> Result<IdentityRecord, sqlx::Error> {
    Ok(IdentityRecord {
        id: row.try_get("id")?,
        email: row.try_get("email")?,
        display_name: row.try_get("display_name")?,
        password_hash: row.try_get("password_hash")?,
        role: row.try_get("role")?,
        status: row.try_get("status")?,
        can_consume_model: row.try_get::<u8, _>("can_consume_model")? == 1,
        password_change_required: row.try_get::<u8, _>("password_change_required")? == 1,
        revision: row.try_get("revision")?,
        integrity_hmac: row.try_get("integrity_hmac")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn security_state_from_mariadb_row(
    row: sqlx::mysql::MySqlRow,
) -> Result<SecurityStateRecord, sqlx::Error> {
    Ok(SecurityStateRecord {
        key: row.try_get("state_key")?,
        value: row.try_get("state_value")?,
        revision: row.try_get("revision")?,
        mac: row.try_get("mac")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn authenticated_session_from_mariadb_row(
    row: sqlx::mysql::MySqlRow,
) -> Result<AuthenticatedSession, sqlx::Error> {
    Ok(AuthenticatedSession {
        session: SessionRecord {
            id: row.try_get("session_id")?,
            identity_id: row.try_get("identity_id")?,
            token_hash: row.try_get("token_hash")?,
            expires_at: row.try_get("expires_at")?,
            integrity_hmac: row.try_get("session_integrity_hmac")?,
            created_at: row.try_get("session_created_at")?,
        },
        identity: IdentityRecord {
            id: row.try_get("identity_id_value")?,
            email: row.try_get("email")?,
            display_name: row.try_get("display_name")?,
            password_hash: row.try_get("password_hash")?,
            role: row.try_get("role")?,
            status: row.try_get("status")?,
            can_consume_model: row.try_get::<u8, _>("can_consume_model")? == 1,
            password_change_required: row.try_get::<u8, _>("password_change_required")? == 1,
            revision: row.try_get("revision")?,
            integrity_hmac: row.try_get("identity_integrity_hmac")?,
            created_at: row.try_get("identity_created_at")?,
            updated_at: row.try_get("identity_updated_at")?,
        },
    })
}

fn api_key_from_mariadb_row(row: sqlx::mysql::MySqlRow) -> Result<ApiKeyRecord, sqlx::Error> {
    Ok(ApiKeyRecord {
        id: row.try_get("id")?,
        identity_id: row.try_get("identity_id")?,
        name: row.try_get("name")?,
        key_hash: row.try_get("key_hash")?,
        key_prefix: row.try_get("key_prefix")?,
        status: row.try_get("status")?,
        revision: row.try_get("revision")?,
        integrity_hmac: row.try_get("integrity_hmac")?,
        created_at: row.try_get("created_at")?,
        last_used_at: row.try_get("last_used_at")?,
    })
}

fn quota_request_from_mariadb_row(
    row: sqlx::mysql::MySqlRow,
) -> Result<QuotaRequestRecord, sqlx::Error> {
    Ok(QuotaRequestRecord {
        id: row.try_get("id")?,
        identity_id: row.try_get("identity_id")?,
        amount_nanos: checked_i64(row.try_get::<u64, _>("amount_nanos")?)?,
        reason: row.try_get("reason")?,
        status: row.try_get("status")?,
        review_note: row.try_get("review_note")?,
        reviewed_by: row.try_get("reviewed_by")?,
        reviewed_at: row.try_get("reviewed_at")?,
        revision: row.try_get("revision")?,
        integrity_hmac: row.try_get("integrity_hmac")?,
        created_at: row.try_get("created_at")?,
    })
}

fn voucher_from_mariadb_row(row: sqlx::mysql::MySqlRow) -> Result<VoucherRecord, sqlx::Error> {
    Ok(VoucherRecord {
        id: row.try_get("id")?,
        code_hash: row.try_get("code_hash")?,
        code_prefix: row.try_get("code_prefix")?,
        name: row.try_get("name")?,
        quota_tokens: checked_i64(row.try_get::<u64, _>("quota_tokens")?)?,
        status: row.try_get("status")?,
        max_redemptions: row.try_get("max_redemptions")?,
        redeemed_count: row.try_get("redeemed_count")?,
        expires_at: row.try_get("expires_at")?,
        created_by: row.try_get("created_by")?,
        revision: row.try_get("revision")?,
        integrity_hmac: row.try_get("integrity_hmac")?,
        created_at: row.try_get("created_at")?,
    })
}

fn voucher_delivery_from_mariadb_row(
    row: sqlx::mysql::MySqlRow,
) -> Result<VoucherDeliveryRecord, sqlx::Error> {
    Ok(VoucherDeliveryRecord {
        id: row.try_get("id")?,
        voucher_id: row.try_get("voucher_id")?,
        identity_id: row.try_get("identity_id")?,
        status: row.try_get("status")?,
        delivered_at: row.try_get("delivered_at")?,
        redeemed_at: row.try_get("redeemed_at")?,
    })
}

fn voucher_delivery_pair_from_mariadb_row(
    row: sqlx::mysql::MySqlRow,
) -> Result<(VoucherDeliveryRecord, VoucherRecord), sqlx::Error> {
    Ok((
        VoucherDeliveryRecord {
            id: row.try_get("delivery_id")?,
            voucher_id: row.try_get("voucher_id")?,
            identity_id: row.try_get("identity_id")?,
            status: row.try_get("delivery_status")?,
            delivered_at: row.try_get("delivered_at")?,
            redeemed_at: row.try_get("redeemed_at")?,
        },
        VoucherRecord {
            id: row.try_get("voucher_id_value")?,
            code_hash: row.try_get("code_hash")?,
            code_prefix: row.try_get("code_prefix")?,
            name: row.try_get("name")?,
            quota_tokens: checked_i64(row.try_get::<u64, _>("quota_tokens")?)?,
            status: row.try_get("voucher_status")?,
            max_redemptions: row.try_get("max_redemptions")?,
            redeemed_count: row.try_get("redeemed_count")?,
            expires_at: row.try_get("expires_at")?,
            created_by: row.try_get("created_by")?,
            revision: row.try_get("revision")?,
            integrity_hmac: row.try_get("integrity_hmac")?,
            created_at: row.try_get("created_at")?,
        },
    ))
}

fn voucher_redemption_from_mariadb_row(
    row: sqlx::mysql::MySqlRow,
) -> Result<VoucherRedemptionRecord, sqlx::Error> {
    Ok(VoucherRedemptionRecord {
        id: row.try_get("id")?,
        voucher_id: row.try_get("voucher_id")?,
        identity_id: row.try_get("identity_id")?,
        amount_tokens: checked_i64(row.try_get::<u64, _>("amount_tokens")?)?,
        ledger_entry_id: row.try_get("ledger_entry_id")?,
        created_at: row.try_get("created_at")?,
    })
}

fn runtime_setting_from_mariadb_row(
    row: sqlx::mysql::MySqlRow,
) -> Result<RuntimeSettingRecord, sqlx::Error> {
    Ok(RuntimeSettingRecord {
        key: row.try_get("setting_key")?,
        value: row.try_get("setting_value")?,
        revision: row.try_get("revision")?,
        integrity_hmac: row.try_get("integrity_hmac")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn audit_event_from_mariadb_row(
    row: sqlx::mysql::MySqlRow,
) -> Result<AuditEventRecord, sqlx::Error> {
    Ok(AuditEventRecord {
        id: row.try_get("id")?,
        sequence: row.try_get("event_sequence")?,
        actor_identity_id: row.try_get("actor_identity_id")?,
        actor_role: row.try_get("actor_role")?,
        action: row.try_get("action")?,
        target_type: row.try_get("target_type")?,
        target_id: row.try_get("target_id")?,
        outcome: row.try_get("outcome")?,
        previous_event_hmac: row.try_get("previous_event_hmac")?,
        integrity_hmac: row.try_get("integrity_hmac")?,
        created_at: row.try_get("created_at")?,
    })
}

fn authorized_api_key_from_mariadb_row(
    row: sqlx::mysql::MySqlRow,
) -> Result<AuthorizedApiKey, sqlx::Error> {
    Ok(AuthorizedApiKey {
        api_key: ApiKeyRecord {
            id: row.try_get("id")?,
            identity_id: row.try_get("identity_id")?,
            name: row.try_get("name")?,
            key_hash: row.try_get("key_hash")?,
            key_prefix: row.try_get("key_prefix")?,
            status: row.try_get("status")?,
            revision: row.try_get("revision")?,
            integrity_hmac: row.try_get("integrity_hmac")?,
            created_at: row.try_get("created_at")?,
            last_used_at: row.try_get("last_used_at")?,
        },
        identity: IdentityRecord {
            id: row.try_get("identity_id_value")?,
            email: row.try_get("email")?,
            display_name: row.try_get("display_name")?,
            password_hash: row.try_get("password_hash")?,
            role: row.try_get("role")?,
            status: row.try_get("identity_status")?,
            can_consume_model: row.try_get::<u8, _>("can_consume_model")? == 1,
            password_change_required: row.try_get::<u8, _>("password_change_required")? == 1,
            revision: row.try_get("identity_revision")?,
            integrity_hmac: row.try_get("identity_integrity_hmac")?,
            created_at: row.try_get("identity_created_at")?,
            updated_at: row.try_get("identity_updated_at")?,
        },
    })
}

fn model_from_mariadb_row(row: sqlx::mysql::MySqlRow) -> Result<ModelRecord, sqlx::Error> {
    Ok(ModelRecord {
        id: row.try_get("id")?,
        public_name: row.try_get("public_name")?,
        display_name: row.try_get("display_name")?,
        enabled: row.try_get("enabled")?,
        discovered_at: row.try_get("discovered_at")?,
        created_at: row.try_get("created_at")?,
    })
}

fn gateway_route_from_mariadb_row(
    row: sqlx::mysql::MySqlRow,
) -> Result<GatewayRouteCandidate, sqlx::Error> {
    Ok(GatewayRouteCandidate {
        model_id: row.try_get("model_id")?,
        public_model: row.try_get("public_name")?,
        upstream_model: row.try_get("upstream_name")?,
        account_id: row.try_get("account_id")?,
        provider: row.try_get("provider")?,
        upstream_subject_id: row.try_get("upstream_subject_id")?,
        last_success_runner_id: row.try_get("last_success_runner_id")?,
        credential: EncryptedCredentialInstance {
            id: row.try_get("credential_id")?,
            account_id: row.try_get("credential_account_id")?,
            credential_identity_hmac: row.try_get("credential_identity_hmac")?,
            encrypted_payload: row.try_get("encrypted_payload")?,
            payload_nonce: row.try_get("payload_nonce")?,
            wrapped_data_key: row.try_get("wrapped_data_key")?,
            wrap_nonce: row.try_get("wrap_nonce")?,
            credential_revision: row.try_get("credential_revision")?,
            expires_at: row.try_get("expires_at")?,
            status: row.try_get("credential_status")?,
            last_refreshed_at: row.try_get("last_refreshed_at")?,
            created_at: row.try_get("credential_created_at")?,
            updated_at: row.try_get("credential_updated_at")?,
        },
    })
}

fn map_mariadb_credential_write_error(error: sqlx::Error) -> StorageError {
    if matches!(&error, sqlx::Error::Database(database) if database.is_unique_violation()) {
        StorageError::DuplicateCredentialIdentity
    } else {
        StorageError::MariaDb(error)
    }
}

fn map_mariadb_identity_write_error(error: sqlx::Error) -> StorageError {
    if matches!(&error, sqlx::Error::Database(database) if database.is_unique_violation()) {
        StorageError::DuplicateIdentity
    } else {
        StorageError::MariaDb(error)
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    fn test_config() -> MariaDbConfig {
        let database = std::env::var("ASTER_TEST_MARIADB_DATABASE")
            .expect("ASTER_TEST_MARIADB_DATABASE is required");
        assert!(
            database.starts_with("aster_entity_quota_test_"),
            "integration test database must use the dedicated prefix"
        );
        MariaDbConfig {
            host: std::env::var("ASTER_TEST_MARIADB_HOST")
                .unwrap_or_else(|_| "127.0.0.1".to_owned()),
            port: std::env::var("ASTER_TEST_MARIADB_PORT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(3306),
            database,
            username: std::env::var("ASTER_TEST_MARIADB_USER")
                .expect("ASTER_TEST_MARIADB_USER is required"),
            password: std::env::var("ASTER_TEST_MARIADB_PASSWORD")
                .expect("ASTER_TEST_MARIADB_PASSWORD is required"),
            tls: false,
            ca_certificate: None,
            max_connections: 4,
        }
    }

    #[tokio::test]
    #[ignore = "requires an isolated MariaDB database"]
    async fn active_quota_discovery_pages_distinct_identities() {
        let store = MariaDbStore::initialize(&test_config()).await.unwrap();
        let now = "2026-09-07T00:00:00.000Z";
        for (id, identity, status) in [
            ("discovery_a1", "identity_discovery_a", "active"),
            ("discovery_a2", "identity_discovery_a", "active"),
            ("discovery_b", "identity_discovery_b", "released"),
            ("discovery_c", "identity_discovery_c", "active"),
            ("discovery_d", "identity_discovery_d", "active"),
        ] {
            sqlx::query("INSERT IGNORE INTO identities(id,email,display_name,password_hash,role,status,can_consume_model,password_change_required,revision,integrity_hmac,created_at,updated_at) VALUES(?,?,'Recovery','hash','member','active',1,0,0,'untrusted',?,?)")
                .bind(identity).bind(format!("{identity}@example.test")).bind(now).bind(now)
                .execute(store.pool()).await.unwrap();
            sqlx::query("INSERT IGNORE INTO user_balances(identity_id,balance_tokens,reserved_tokens,granted_tokens,integrity_hmac,updated_at) VALUES(?,10,?,10,'untrusted',?)")
                .bind(identity).bind(i64::from(status == "active")).bind(now)
                .execute(store.pool()).await.unwrap();
            sqlx::query("INSERT INTO api_keys(id,identity_id,name,key_hash,key_prefix,status,revision,integrity_hmac,created_at) VALUES(?,?,'Recovery',?,'ask_test','active',0,'untrusted',?)")
                .bind(id).bind(identity).bind(format!("hash_{id}")).bind(now)
                .execute(store.pool()).await.unwrap();
            sqlx::query("INSERT INTO quota_reservations(id,identity_id,api_key_id,request_id,reserved_tokens,status,revision,integrity_hmac,created_at,expires_at) VALUES(?,?,?,?,1,?,0,'untrusted',?,'untrusted-expiry')")
                .bind(id).bind(identity).bind(id).bind(id).bind(status).bind(now)
                .execute(store.pool()).await.unwrap();
        }
        assert_eq!(
            store.active_quota_identity_page("", 2).await.unwrap(),
            ["identity_discovery_a", "identity_discovery_c"]
        );
        assert_eq!(
            store
                .active_quota_identity_page("identity_discovery_c", 2)
                .await
                .unwrap(),
            ["identity_discovery_d"]
        );
        assert!(
            store
                .active_quota_identity_page("identity_discovery_d", 2)
                .await
                .unwrap()
                .is_empty()
        );
        sqlx::query(
            "UPDATE user_balances SET reserved_tokens=0 WHERE identity_id='identity_discovery_a'",
        )
        .execute(store.pool())
        .await
        .unwrap();
        assert_eq!(
            store.active_quota_identity_page("", 2).await.unwrap(),
            ["identity_discovery_c", "identity_discovery_d"]
        );
        assert_eq!(
            store
                .active_quota_identity_page("identity_discovery_a", 2)
                .await
                .unwrap(),
            ["identity_discovery_c", "identity_discovery_d"]
        );
    }

    fn api_key(id: &str, marker: char) -> ApiKeyRecord {
        ApiKeyRecord {
            id: id.to_owned(),
            identity_id: "identity_quota_member".to_owned(),
            name: id.to_owned(),
            key_hash: marker.to_string().repeat(43),
            key_prefix: format!("ask_{marker}"),
            status: "active".to_owned(),
            revision: 0,
            integrity_hmac: format!("key-hmac-{marker}"),
            created_at: "2026-09-07T00:00:00.000Z".to_owned(),
            last_used_at: None,
        }
    }

    #[tokio::test]
    #[ignore = "requires an isolated MariaDB database"]
    async fn runner_connection_upgrade_serializes_credential_changes_and_recovers_failure() {
        let store = MariaDbStore::initialize(&test_config()).await.unwrap();
        let now = "2026-09-07T00:00:00.000Z";
        sqlx::query("INSERT INTO identities(id,email,display_name,password_hash,role,status,can_consume_model,password_change_required,revision,integrity_hmac,created_at,updated_at) VALUES('identity_connection_owner','connection@example.test','Owner','hash','owner','active',0,0,0,'identity-hmac',?,?)").bind(now).bind(now).execute(store.pool()).await.unwrap();
        store
            .insert_runner_enrollment(&RunnerEnrollmentRecord {
                id: "enrollment_connection".into(),
                token_hash: "enrollment_token".into(),
                token_prefix: "test".into(),
                runner_name: "runner_connection".into(),
                status: "pending".into(),
                expires_at: "2026-09-08T00:00:00.000Z".into(),
                created_by: "identity_connection_owner".into(),
                created_at: now.into(),
            })
            .await
            .unwrap();
        assert!(
            store
                .pending_runner_enrollment("enrollment_token", now)
                .await
                .unwrap()
        );
        assert!(
            !store
                .pending_runner_enrollment("enrollment_token", "2026-09-09T00:00:00.000Z")
                .await
                .unwrap()
        );
        let credential_hash = "a".repeat(64);
        assert_eq!(
            store
                .consume_runner_enrollment_unchecked(
                    "enrollment_token",
                    now,
                    &RunnerRegistrationRecord {
                        id: "runner_connection".into(),
                        credential_hash: credential_hash.clone(),
                        version: "2.0.0".into(),
                        protocol_version: 2,
                        platform: "linux".into(),
                        architecture: "x86_64".into(),
                        max_inflight: 1,
                        created_at: now.into(),
                    }
                )
                .await
                .unwrap(),
            RunnerEnrollmentConsumeOutcome::Registered
        );
        assert!(
            !store
                .pending_runner_enrollment("enrollment_token", now)
                .await
                .unwrap()
        );
        let update = RunnerConnectionUpdate {
            runner_id: "runner_connection".into(),
            credential_hash: credential_hash.clone(),
            previous_protocol_version: 2,
            protocol_version: 3,
            runner_version: "2.1.0".into(),
            max_inflight: 3,
            heartbeat: RunnerHeartbeatUpdate {
                inflight: 0,
                recent_request_count: 0,
                recent_error_count: 0,
                latency_ms: 0,
            },
            observed_at: now.into(),
        };
        for mutation in [
            "UPDATE runners SET credential_hash=REPEAT('b',64) WHERE id='runner_connection'",
            "UPDATE runners SET enabled=0 WHERE id='runner_connection'",
        ] {
            for commit in [false, true] {
                sqlx::query("UPDATE runners SET credential_hash=?,enabled=1,protocol_version=2,version='2.0.0',max_inflight=1,last_seen_at=NULL WHERE id='runner_connection'").bind(&credential_hash).execute(store.pool()).await.unwrap();
                let mut writer = store.pool().begin().await.unwrap();
                let connection_id: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()")
                    .fetch_one(&mut *writer)
                    .await
                    .unwrap();
                sqlx::query(mutation).execute(&mut *writer).await.unwrap();
                let updating_store = store.clone();
                let queued_update = update.clone();
                let updater = tokio::spawn(async move {
                    updating_store
                        .record_runner_connection(&queued_update)
                        .await
                });
                wait_for_row_lock(&store, connection_id).await;
                assert!(
                    !updater.is_finished(),
                    "connection write must wait for the real registration row lock"
                );
                if commit {
                    writer.commit().await.unwrap();
                } else {
                    writer.rollback().await.unwrap();
                }
                let accepted = tokio::time::timeout(std::time::Duration::from_secs(5), updater)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                assert_eq!(accepted, !commit);
                let row = sqlx::query("SELECT protocol_version,version,max_inflight FROM runners WHERE id='runner_connection'").fetch_one(store.pool()).await.unwrap();
                assert_eq!(
                    row.get::<u32, _>("protocol_version"),
                    if commit { 2 } else { 3 }
                );
                assert_eq!(
                    row.get::<String, _>("version"),
                    if commit { "2.0.0" } else { "2.1.0" }
                );
                assert_eq!(
                    row.get::<u32, _>("max_inflight"),
                    if commit { 1 } else { 3 }
                );
            }
        }
        sqlx::query("UPDATE runners SET credential_hash=?,enabled=1,protocol_version=2,version='2.0.0',max_inflight=1 WHERE id='runner_connection'").bind(&credential_hash).execute(store.pool()).await.unwrap();
        sqlx::query("CREATE TRIGGER fail_runner_connection AFTER UPDATE ON runners FOR EACH ROW SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT='injected connection write failure'").execute(store.pool()).await.unwrap();
        assert!(store.record_runner_connection(&update).await.is_err());
        let record = store
            .runner_by_credential_hash(&credential_hash)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.protocol_version, 2);
        assert_eq!(record.version, "2.0.0");
        sqlx::query("DROP TRIGGER fail_runner_connection")
            .execute(store.pool())
            .await
            .unwrap();
        assert!(store.record_runner_connection(&update).await.unwrap());
        assert!(store.record_runner_connection(&update).await.unwrap());
        let mut downgrade = update.clone();
        downgrade.previous_protocol_version = 3;
        downgrade.protocol_version = 2;
        assert!(!store.record_runner_connection(&downgrade).await.unwrap());
        assert_eq!(
            store
                .runner_by_credential_hash(&credential_hash)
                .await
                .unwrap()
                .unwrap()
                .protocol_version,
            3
        );
    }

    fn audit(id: &str, marker: char) -> AuditEventRecord {
        AuditEventRecord {
            id: id.to_owned(),
            sequence: 1,
            actor_identity_id: None,
            actor_role: "system".to_owned(),
            action: "api_key.create".to_owned(),
            target_type: "api_key".to_owned(),
            target_id: Some(id.to_owned()),
            outcome: "succeeded".to_owned(),
            previous_event_hmac: String::new(),
            integrity_hmac: marker.to_string().repeat(43),
            created_at: "2026-09-07T00:00:00.000Z".to_owned(),
        }
    }

    #[tokio::test]
    #[ignore = "requires an isolated MariaDB database"]
    async fn quota_reservation_subject_locks_serialize_revocation_and_disable() {
        let store = MariaDbStore::initialize(&test_config()).await.unwrap();
        sqlx::query("INSERT INTO identities(id,email,display_name,password_hash,role,status,can_consume_model,password_change_required,revision,integrity_hmac,created_at,updated_at) VALUES('identity_quota_member','quota@example.test','Quota','hash','member','active',1,0,0,'identity-hmac','2026-09-07T00:00:00.000Z','2026-09-07T00:00:00.000Z')")
            .execute(store.pool()).await.unwrap();
        sqlx::query("INSERT INTO user_balances(identity_id,balance_tokens,granted_tokens,integrity_hmac,updated_at) VALUES('identity_quota_member',1000,1000,'balance-hmac','2026-09-07T00:00:00.000Z')")
            .execute(store.pool()).await.unwrap();
        let key = api_key("key_reservation", 'R');
        store.insert_api_key_unchecked(&key).await.unwrap();

        // These are transaction/locking fixtures. Real HMAC and License
        // verification is covered by the Control model_authorization tests.
        for (table, id, disabled) in [
            ("api_keys", key.id.as_str(), "revoked"),
            ("identities", "identity_quota_member", "disabled"),
        ] {
            let subject = store
                .authorized_api_key_by_hash(&key.key_hash)
                .await
                .unwrap()
                .unwrap();
            let expected = store
                .quota_state_snapshot(&subject.identity.id)
                .await
                .unwrap()
                .unwrap()
                .balance;
            let (next, record) =
                reservation_transition(&subject, &expected, &format!("{table}-revoke-first"));
            let mut revoke = store.pool().begin().await.unwrap();
            let revoke_id: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()")
                .fetch_one(&mut *revoke)
                .await
                .unwrap();
            sqlx::query(subject_status_update(table))
                .bind(disabled)
                .bind("revoked-hmac")
                .bind(id)
                .execute(&mut *revoke)
                .await
                .unwrap();
            let reserve_store = store.clone();
            let expected_before = expected.reserved_tokens;
            let mut reserve = tokio::spawn(async move {
                reserve_store
                    .reserve_quota(&subject, &expected, &next, &record)
                    .await
            });
            tokio::select! {
                _ = wait_for_row_lock(&store, revoke_id) => {},
                result = &mut reserve => panic!("reservation completed before the revocation lock wait: {result:?}"),
            }
            revoke.commit().await.unwrap();
            assert_eq!(
                reserve.await.unwrap().unwrap(),
                QuotaMutationOutcome::Conflict
            );
            assert_eq!(
                store
                    .quota_state_snapshot("identity_quota_member")
                    .await
                    .unwrap()
                    .unwrap()
                    .balance
                    .reserved_tokens,
                expected_before
            );

            sqlx::query(subject_status_update(table))
                .bind("active")
                .bind("restored-hmac")
                .bind(id)
                .execute(store.pool())
                .await
                .unwrap();
            let subject = store
                .authorized_api_key_by_hash(&key.key_hash)
                .await
                .unwrap()
                .unwrap();
            let expected = store
                .quota_state_snapshot(&subject.identity.id)
                .await
                .unwrap()
                .unwrap()
                .balance;
            let (next, record) =
                reservation_transition(&subject, &expected, &format!("{table}-reserve-first"));
            let mut balance_lock = store.pool().begin().await.unwrap();
            let balance_id: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()")
                .fetch_one(&mut *balance_lock)
                .await
                .unwrap();
            sqlx::query("SELECT identity_id FROM user_balances WHERE identity_id='identity_quota_member' FOR UPDATE")
                .fetch_one(&mut *balance_lock).await.unwrap();
            let reserve_store = store.clone();
            let reserve = tokio::spawn(async move {
                reserve_store
                    .reserve_quota(&subject, &expected, &next, &record)
                    .await
            });
            // A real DB lock wait proves reserve has already locked its subject
            // and reached the balance. No scheduling delay is assumed.
            let reserve_id = wait_for_row_lock(&store, balance_id).await;
            let revoke_store = store.clone();
            let id = id.to_owned();
            let revoke = tokio::spawn(async move {
                sqlx::query(subject_status_update(table))
                    .bind(disabled)
                    .bind("revoked-hmac")
                    .bind(id)
                    .execute(revoke_store.pool())
                    .await
            });
            wait_for_row_lock(&store, reserve_id).await;
            balance_lock.commit().await.unwrap();
            assert_eq!(
                reserve.await.unwrap().unwrap(),
                QuotaMutationOutcome::Applied
            );
            assert_eq!(revoke.await.unwrap().unwrap().rows_affected(), 1);
            assert_eq!(
                store
                    .quota_state_snapshot("identity_quota_member")
                    .await
                    .unwrap()
                    .unwrap()
                    .balance
                    .reserved_tokens,
                expected_before + 100
            );
            assert!(
                store
                    .authorized_api_key_by_hash(&key.key_hash)
                    .await
                    .unwrap()
                    .is_none()
            );
            sqlx::query(subject_status_update(table))
                .bind("active")
                .bind("restored-hmac")
                .bind(if table == "api_keys" {
                    key.id.as_str()
                } else {
                    "identity_quota_member"
                })
                .execute(store.pool())
                .await
                .unwrap();
        }
    }

    fn subject_status_update(table: &str) -> &'static str {
        match table {
            "api_keys" => {
                "UPDATE api_keys SET status=?,revision=revision+1,integrity_hmac=? WHERE id=?"
            }
            "identities" => {
                "UPDATE identities SET status=?,revision=revision+1,integrity_hmac=? WHERE id=?"
            }
            _ => panic!("unknown fixture table"),
        }
    }

    #[tokio::test]
    #[ignore = "requires an isolated MariaDB database"]
    async fn quota_snapshot_waits_for_complete_reservation_transaction() {
        let store = MariaDbStore::initialize(&test_config()).await.unwrap();
        let identity = "identity_snapshot";
        sqlx::query("INSERT INTO identities(id,email,display_name,password_hash,role,status,can_consume_model,password_change_required,revision,integrity_hmac,created_at,updated_at) VALUES(?,'snapshot@example.test','Snapshot','hash','member','active',1,0,0,'identity-hmac','2026-09-07T00:00:00.000Z','2026-09-07T00:00:00.000Z')")
            .bind(identity).execute(store.pool()).await.unwrap();
        sqlx::query("INSERT INTO user_balances(identity_id,balance_tokens,granted_tokens,integrity_hmac,updated_at) VALUES(?,1000,1000,'balance-hmac','2026-09-07T00:00:00.000Z')")
            .bind(identity).execute(store.pool()).await.unwrap();
        let mut key = api_key("key_snapshot", 'S');
        key.identity_id = identity.to_owned();
        store.insert_api_key_unchecked(&key).await.unwrap();

        for commit in [false, true] {
            let before = store.quota_state_snapshot(identity).await.unwrap().unwrap();
            let mut writer = store.pool().begin().await.unwrap();
            let writer_id: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()")
                .fetch_one(&mut *writer)
                .await
                .unwrap();
            // Exercise ledger-first writes as well as reservation writes after
            // the balance lock. This fixture checks transaction shape, not HMAC.
            sqlx::query("INSERT INTO ledger_entries(id,identity_id,kind,amount_tokens,reference_id,description,previous_entry_hmac,integrity_hmac,created_at) VALUES('snapshot_ledger',?,'grant',100,'snapshot-grant','test','','snapshot-ledger-hmac','2026-09-07T00:00:00.000Z')")
                .bind(identity).execute(&mut *writer).await.unwrap();
            sqlx::query("UPDATE user_balances SET balance_tokens=1100,granted_tokens=1100,reserved_tokens=100,last_ledger_hmac='snapshot-ledger-hmac',revision=revision+1 WHERE identity_id=?")
                .bind(identity).execute(&mut *writer).await.unwrap();
            let reader_store = store.clone();
            let reader = tokio::spawn(async move {
                reader_store
                    .quota_state_snapshot(identity)
                    .await
                    .unwrap()
                    .unwrap()
            });
            // The snapshot must wait at the balance lock; a normal uncoordinated
            // balance SELECT returns immediately and cannot pass this assertion.
            wait_for_row_lock(&store, writer_id).await;
            assert!(!reader.is_finished());
            sqlx::query("INSERT INTO quota_reservations(id,identity_id,api_key_id,request_id,reserved_tokens,status,revision,integrity_hmac,created_at,expires_at) VALUES('snapshot_reservation',?,'key_snapshot','snapshot_request',100,'active',0,'reservation-hmac','2026-09-07T00:00:00.000Z','2026-09-07T00:05:00.000Z')")
                .bind(identity).execute(&mut *writer).await.unwrap();
            if commit {
                writer.commit().await.unwrap();
            } else {
                writer.rollback().await.unwrap();
            }
            let snapshot = tokio::time::timeout(std::time::Duration::from_secs(5), reader)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                snapshot.balance.reserved_tokens,
                if commit { 100 } else { 0 }
            );
            assert_eq!(snapshot.active_reservations.len(), usize::from(commit));
            assert_eq!(
                snapshot
                    .active_reservations
                    .iter()
                    .map(|record| record.reserved_tokens)
                    .sum::<i64>(),
                snapshot.balance.reserved_tokens
            );
            if commit {
                assert_eq!(snapshot.ledger_entries.len(), 1);
                assert_eq!(
                    snapshot.ledger_entries[0].integrity_hmac,
                    snapshot.balance.last_ledger_hmac
                );
                assert_eq!(snapshot.balance.balance_tokens, 1100);
            } else {
                assert_eq!(snapshot.balance, before.balance);
                assert_eq!(snapshot.ledger_entries, before.ledger_entries);
            }
        }
    }

    async fn wait_for_row_lock(store: &MariaDbStore, blocking_id: u64) -> u64 {
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let waiter: Option<u64> = sqlx::query_scalar("SELECT r.trx_mysql_thread_id FROM information_schema.INNODB_LOCK_WAITS w JOIN information_schema.INNODB_TRX b ON b.trx_id=w.blocking_trx_id JOIN information_schema.INNODB_TRX r ON r.trx_id=w.requesting_trx_id WHERE b.trx_mysql_thread_id=? LIMIT 1")
                    .bind(blocking_id).fetch_optional(store.pool()).await.unwrap();
                if let Some(waiter) = waiter { return waiter; }
                // Leave the InnoDB transaction/lock snapshot cache idle between
                // reads. Tight polling can keep returning the same snapshot.
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }).await;
        if let Ok(waiter) = outcome {
            return waiter;
        }
        let transactions =
            sqlx::query("SELECT trx_mysql_thread_id,trx_state FROM information_schema.INNODB_TRX")
                .fetch_all(store.pool())
                .await
                .unwrap()
                .into_iter()
                .map(|row| {
                    (
                        row.get::<u64, _>("trx_mysql_thread_id"),
                        row.get::<String, _>("trx_state"),
                    )
                })
                .collect::<Vec<_>>();
        panic!(
            "no row lock wait for blocker {blocking_id}; current transactions: {transactions:?}"
        );
    }

    fn reservation_transition(
        subject: &AuthorizedApiKey,
        expected: &UserBalanceRecord,
        id: &str,
    ) -> (UserBalanceRecord, QuotaReservationRecord) {
        let mut next = expected.clone();
        next.reserved_tokens += 100;
        next.revision += 1;
        next.integrity_hmac = format!("balance-{id}");
        let record = QuotaReservationRecord {
            id: id.to_owned(),
            identity_id: subject.identity.id.clone(),
            api_key_id: subject.api_key.id.clone(),
            request_id: id.to_owned(),
            reserved_tokens: 100,
            status: "active".to_owned(),
            revision: 0,
            integrity_hmac: format!("reservation-{id}"),
            created_at: "2026-09-07T00:00:00.000Z".to_owned(),
            expires_at: "2026-09-07T00:05:00.000Z".to_owned(),
            settled_at: None,
        };
        (next, record)
    }

    #[tokio::test]
    #[ignore = "requires an isolated MariaDB database"]
    async fn entity_quota_gate_recovers_migration_and_serializes_concurrent_creates() {
        let config = test_config();
        let store = MariaDbStore::initialize(&config)
            .await
            .expect("initialize isolated database");

        sqlx::query("DELETE FROM schema_migrations WHERE version=4")
            .execute(store.pool())
            .await
            .expect("simulate crash after migration effect");
        let store = MariaDbStore::open(&config)
            .await
            .expect("recover idempotent migration");
        assert_eq!(
            sqlx::query_scalar::<MySql, i64>(
                "SELECT count(*) FROM schema_migrations WHERE version=4",
            )
            .fetch_one(store.pool())
            .await
            .expect("read recovered migration"),
            1
        );

        sqlx::query(
            "INSERT INTO identities(
               id,email,display_name,password_hash,role,status,can_consume_model,
               password_change_required,revision,integrity_hmac,created_at,updated_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind("identity_quota_member")
        .bind("quota-member@example.com")
        .bind("Quota Member")
        .bind("password-hash")
        .bind("member")
        .bind("active")
        .bind(true)
        .bind(false)
        .bind(0_u32)
        .bind("identity-hmac")
        .bind("2026-09-07T00:00:00.000Z")
        .bind("2026-09-07T00:00:00.000Z")
        .execute(store.pool())
        .await
        .expect("seed member");

        let first_store = store.clone();
        let first = tokio::spawn(async move {
            first_store
                .insert_api_key_with_limit_and_audit(
                    &api_key("key_concurrent_one", 'K'),
                    Some(1),
                    0,
                    "",
                    &audit("audit_concurrent_one", 'A'),
                )
                .await
        });
        let second_store = store.clone();
        let second = tokio::spawn(async move {
            second_store
                .insert_api_key_with_limit_and_audit(
                    &api_key("key_concurrent_two", 'L'),
                    Some(1),
                    0,
                    "",
                    &audit("audit_concurrent_two", 'B'),
                )
                .await
        });
        let outcomes = [
            first.await.expect("first task").expect("first create"),
            second.await.expect("second task").expect("second create"),
        ];
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| {
                    **outcome == MutationWithAuditOutcome::Mutation(ResourceCreateOutcome::Created)
                })
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| {
                    **outcome
                        == MutationWithAuditOutcome::Mutation(ResourceCreateOutcome::LimitReached)
                })
                .count(),
            1
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EmbeddedMigration, MARIADB_BASELINE, MARIADB_MIGRATIONS, MariaDbConfig, MariaDbStore,
        MigrationCompatibility, StorageError, checked_i64,
    };

    #[test]
    fn unsigned_database_counters_are_converted_without_wrapping() {
        assert_eq!(checked_i64(0).expect("zero"), 0);
        assert_eq!(checked_i64(i64::MAX as u64).expect("maximum"), i64::MAX);
        assert!(checked_i64(i64::MAX as u64 + 1).is_err());
    }

    #[test]
    fn baseline_has_multi_credential_model_without_placements_or_runner_limits() {
        assert!(MARIADB_BASELINE.contains("CREATE TABLE upstream_credential_instances"));
        assert!(MARIADB_BASELINE.contains("credential_revision BIGINT UNSIGNED"));
        assert!(MARIADB_BASELINE.contains("credential_instances_identity_uq"));
        let refresh_lease = MARIADB_BASELINE
            .split("CREATE TABLE credential_refresh_leases")
            .nth(1)
            .and_then(|tail| tail.split(") ENGINE=InnoDB").next())
            .expect("credential refresh lease table");
        assert!(!refresh_lease.contains("runner_id"));
        assert!(!MARIADB_BASELINE.contains("CREATE TABLE upstream_oauth_sessions"));
        assert!(!MARIADB_BASELINE.contains("runner_account_placements"));
        assert!(!MARIADB_BASELINE.contains("runner_limit"));
        assert!(!MARIADB_BASELINE.contains("admin_seats"));
    }

    #[test]
    fn baseline_counts_seats_by_capability_not_role() {
        assert!(MARIADB_BASELINE.contains("can_consume_model"));
        assert!(!MARIADB_BASELINE.contains("CREATE TABLE admins"));
    }

    #[test]
    fn active_identity_email_is_unique_but_deleted_email_is_reusable() {
        assert!(MARIADB_BASELINE.contains("active_email VARCHAR(320) GENERATED ALWAYS AS"));
        assert!(MARIADB_BASELINE.contains("identities_active_email_uq(active_email)"));
        assert!(!MARIADB_BASELINE.contains("email VARCHAR(320) NOT NULL UNIQUE"));
    }

    async fn verify_atomic_local_runner_registration(store: &MariaDbStore) {
        use crate::{
            AuditEventRecord, AuditedMutationOutcome, RunnerEnrollmentRecord,
            RunnerRegistrationRecord,
        };
        const NOW: &str = "2026-09-08T00:00:00.000Z";
        sqlx::query("INSERT INTO identities(id,email,display_name,password_hash,role,status,integrity_hmac,created_at,updated_at)
            VALUES('slot-owner','slot-owner@example.test','Slot owner','fixture','owner','active','fixture',?,?)")
            .bind(NOW).bind(NOW).execute(store.pool()).await.unwrap();
        let enrollment = RunnerEnrollmentRecord {
            id: "slot-enrollment-blue".into(),
            token_hash: "a".repeat(64),
            token_prefix: "local-slot".into(),
            runner_name: "local-runner-blue".into(),
            status: "pending".into(),
            expires_at: NOW.into(),
            created_by: "slot-owner".into(),
            created_at: NOW.into(),
        };
        let registration = RunnerRegistrationRecord {
            id: "slot-runner-blue".into(),
            credential_hash: "b".repeat(64),
            version: "2.0.1".into(),
            protocol_version: 3,
            platform: "linux".into(),
            architecture: "x86_64".into(),
            max_inflight: 4,
            created_at: NOW.into(),
        };
        let event = AuditEventRecord {
            id: "slot-audit-blue".into(),
            sequence: 1,
            actor_identity_id: Some("slot-owner".into()),
            actor_role: "owner".into(),
            action: "runner.register".into(),
            target_type: "runner".into(),
            target_id: Some(registration.id.clone()),
            outcome: "succeeded".into(),
            previous_event_hmac: String::new(),
            integrity_hmac: "a".repeat(43),
            created_at: NOW.into(),
        };
        assert_eq!(
            store
                .register_local_runner_and_audit(&enrollment, &registration, 9, "wrong", &event)
                .await
                .unwrap(),
            AuditedMutationOutcome::AuditConflict
        );
        assert!(store.list_runners().await.unwrap().is_empty());
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runner_enrollments")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(
            store
                .register_local_runner_and_audit(&enrollment, &registration, 0, "", &event)
                .await
                .unwrap(),
            AuditedMutationOutcome::Applied
        );
        let used: (String, String) =
            sqlx::query_as("SELECT status,runner_id FROM runner_enrollments WHERE id=?")
                .bind(&enrollment.id)
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_eq!(used, ("used".into(), registration.id.clone()));
        assert_eq!(store.audit_events().await.unwrap().len(), 1);
        assert_eq!(
            store
                .register_local_runner_and_audit(&enrollment, &registration, 0, "", &event)
                .await
                .unwrap(),
            AuditedMutationOutcome::MutationConflict
        );
        sqlx::query("DELETE FROM runners WHERE id=?")
            .bind(&registration.id)
            .execute(store.pool())
            .await
            .unwrap();
        assert_eq!(
            store
                .register_local_runner_and_audit(&enrollment, &registration, 0, "", &event)
                .await
                .unwrap(),
            AuditedMutationOutcome::MutationConflict
        );
        let mut replacement = enrollment.clone();
        replacement.id = "slot-enrollment-replacement".into();
        replacement.token_hash = "c".repeat(64);
        let mut replaced_runner = registration;
        replaced_runner.id = "slot-runner-replacement".into();
        assert_eq!(
            store
                .register_local_runner_and_audit(&replacement, &replaced_runner, 0, "", &event)
                .await
                .unwrap(),
            AuditedMutationOutcome::MutationConflict
        );
        assert!(store.list_runners().await.unwrap().is_empty());
        assert_eq!(store.audit_events().await.unwrap().len(), 1);
    }

    async fn verify_logical_runner_quota_transactions(store: &MariaDbStore) {
        use crate::runner_quota::{
            RUNNER_QUOTA_KEY, RunnerQuotaBindings, RunnerQuotaMember, RunnerQuotaPolicy,
            RunnerUpgradeBinding,
        };
        use crate::{
            AuditEventRecord, MutationWithAuditOutcome, RunnerEnrollmentConsumeOutcome,
            RunnerEnrollmentRecord, RunnerRegistrationRecord,
        };
        const NOW: &str = "2026-09-08T00:00:00.000Z";
        let registration = |id: &str, digit: &str| RunnerRegistrationRecord {
            id: id.into(),
            credential_hash: digit.repeat(64),
            version: "2.1.0".into(),
            protocol_version: 3,
            platform: "linux".into(),
            architecture: "x86_64".into(),
            max_inflight: 4,
            created_at: NOW.into(),
        };
        for (id, digit) in [
            ("runner_quota_old", "1"),
            ("runner_quota_new", "2"),
            ("runner_quota_third", "3"),
            ("runner_quota_fourth", "4"),
        ] {
            store
                .insert_runner_enrollment(&RunnerEnrollmentRecord {
                    id: format!("enrollment_{id}"),
                    token_hash: id.into(),
                    token_prefix: id.into(),
                    runner_name: id.into(),
                    status: "pending".into(),
                    expires_at: "2026-12-31T00:00:00.000Z".into(),
                    created_by: "slot-owner".into(),
                    created_at: NOW.into(),
                })
                .await
                .unwrap();
            if matches!(digit, "1" | "2") {
                assert_eq!(
                    store
                        .consume_runner_enrollment_unchecked(id, NOW, &registration(id, digit))
                        .await
                        .unwrap(),
                    RunnerEnrollmentConsumeOutcome::Registered
                );
            }
        }
        let value = RunnerQuotaBindings::new(vec![RunnerUpgradeBinding {
            phase: crate::runner_quota::RunnerUpgradePhase::Prepared,
            installation_id: "installation_a".into(),
            job_id: "quota-upgrade-one".into(),
            logical_runner_id: "runner_quota_old".into(),
            previous: RunnerQuotaMember {
                id: "runner_quota_old".into(),
                credential_hash: "1".repeat(64),
            },
            candidate: RunnerQuotaMember {
                id: "runner_quota_new".into(),
                credential_hash: "2".repeat(64),
            },
        }])
        .unwrap()
        .encode()
        .unwrap();
        sqlx::query("INSERT INTO security_state(state_key,state_value,revision,mac,updated_at) VALUES(?,?,1,?,?)")
            .bind(RUNNER_QUOTA_KEY).bind(value).bind(b"fixture".as_slice()).bind(NOW).execute(store.pool()).await.unwrap();
        let proof = store
            .runner_quota_snapshot()
            .await
            .unwrap()
            .verify("installation_a", |_| true)
            .unwrap();
        assert_eq!(proof.occupied(), 1);
        let events = store.audit_events().await.unwrap();
        let last = events.last().unwrap();
        let event = |id: &str| AuditEventRecord {
            id: format!("quota-audit-{id}"),
            sequence: last.sequence + 1,
            actor_identity_id: Some("slot-owner".into()),
            actor_role: "owner".into(),
            action: "runner.register".into(),
            target_type: "runner".into(),
            target_id: Some(id.into()),
            outcome: "succeeded".into(),
            previous_event_hmac: last.integrity_hmac.clone(),
            integrity_hmac: "b".repeat(43),
            created_at: NOW.into(),
        };
        let third = registration("runner_quota_third", "3");
        let fourth = registration("runner_quota_fourth", "4");
        let third_event = event(&third.id);
        let fourth_event = event(&fourth.id);
        let policy = RunnerQuotaPolicy {
            limit: Some(2),
            verified: Some(proof.clone()),
        };
        let (first, second) = tokio::join!(
            store.consume_runner_enrollment_with_quota_and_audit(
                &third.id,
                NOW,
                &third,
                policy.clone(),
                (last.sequence, &last.integrity_hmac, &third_event)
            ),
            store.consume_runner_enrollment_with_quota_and_audit(
                &fourth.id,
                NOW,
                &fourth,
                policy,
                (last.sequence, &last.integrity_hmac, &fourth_event)
            ),
        );
        let outcomes = [first.unwrap(), second.unwrap()];
        assert_eq!(
            outcomes
                .iter()
                .filter(|x| **x
                    == MutationWithAuditOutcome::Mutation(
                        RunnerEnrollmentConsumeOutcome::Registered
                    ))
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|x| **x
                    == MutationWithAuditOutcome::Mutation(
                        RunnerEnrollmentConsumeOutcome::QuotaChanged
                    ))
                .count(),
            1
        );
        let current = store
            .runner_quota_snapshot()
            .await
            .unwrap()
            .verify("installation_a", |_| true)
            .unwrap();
        assert_eq!(current.occupied(), 2);
        assert_eq!(store.audit_events().await.unwrap().len(), events.len() + 1);
        assert_eq!(store.list_runners().await.unwrap().len(), 3);
        verify_logical_runner_quota_lifecycle(store).await;
    }

    async fn verify_logical_runner_quota_lifecycle(store: &MariaDbStore) {
        use crate::runner_quota::{
            RUNNER_QUOTA_KEY, RunnerQuotaMember, RunnerQuotaWrite,
            RunnerQuotaWriteOutcome as Outcome, RunnerUpgradeBinding, RunnerUpgradePhase,
        };
        use crate::{
            AuditEventRecord, RunnerEnrollmentRecord, RunnerRegistrationRecord, SecurityStateRecord,
        };
        const NOW: &str = "2026-09-08T00:00:00.000Z";
        let signed = |change: &crate::runner_quota::RunnerQuotaChange| SecurityStateRecord {
            key: RUNNER_QUOTA_KEY.into(),
            value: change.value().to_vec(),
            revision: change.revision(),
            mac: b"fixture".to_vec(),
            updated_at: NOW.into(),
        };
        let event =
            |previous: &AuditEventRecord, id: &str, action: &str, target: &str| AuditEventRecord {
                id: id.into(),
                sequence: previous.sequence + 1,
                actor_identity_id: Some("slot-owner".into()),
                actor_role: "owner".into(),
                action: action.into(),
                target_type: "runner".into(),
                target_id: Some(target.into()),
                outcome: "succeeded".into(),
                previous_event_hmac: previous.integrity_hmac.clone(),
                integrity_hmac: "c".repeat(43),
                created_at: NOW.into(),
            };
        let before = store.audit_events().await.unwrap().last().unwrap().clone();
        let proof = store
            .runner_quota_snapshot()
            .await
            .unwrap()
            .verify("installation_a", |_| true)
            .unwrap();
        let finish = proof
            .finish_upgrade("quota-upgrade-one", true)
            .unwrap()
            .unwrap();
        let next = signed(&finish);
        let retired = event(
            &before,
            "quota-retired",
            "runner.delete",
            "runner_quota_old",
        );
        let write = || RunnerQuotaWrite {
            change: &finish,
            next_state: &next,
            enrollment: None,
            registration: None,
        };
        assert_eq!(
            store
                .apply_runner_quota_change(write(), None, (0, "", &retired))
                .await
                .unwrap(),
            Outcome::AuditConflict
        );
        assert_eq!(store.list_runners().await.unwrap().len(), 3);
        assert_eq!(
            store
                .apply_runner_quota_change(
                    write(),
                    None,
                    (before.sequence, &before.integrity_hmac, &retired)
                )
                .await
                .unwrap(),
            Outcome::Applied
        );
        let committed = store
            .runner_quota_snapshot()
            .await
            .unwrap()
            .verify("installation_a", |_| true)
            .unwrap();
        assert!(
            committed
                .finish_upgrade("quota-upgrade-one", true)
                .unwrap()
                .is_none()
        );
        assert!(
            committed
                .finish_upgrade("quota-upgrade-one", false)
                .is_err()
        );
        assert_eq!(committed.occupied(), 2);
        let prepare = committed
            .begin_upgrade(RunnerUpgradeBinding {
                phase: RunnerUpgradePhase::Prepared,
                installation_id: "installation_a".into(),
                job_id: "quota-upgrade-two".into(),
                logical_runner_id: "runner_quota_old".into(),
                previous: RunnerQuotaMember {
                    id: "runner_quota_new".into(),
                    credential_hash: "2".repeat(64),
                },
                candidate: RunnerQuotaMember {
                    id: "runner_quota_final".into(),
                    credential_hash: "5".repeat(64),
                },
            })
            .unwrap();
        let next = signed(&prepare);
        let enrollment = RunnerEnrollmentRecord {
            id: "quota-final-enrollment".into(),
            token_hash: "quota-final-token".into(),
            token_prefix: "local-slot".into(),
            runner_name: "runner_quota_old".into(),
            status: "pending".into(),
            expires_at: NOW.into(),
            created_by: "slot-owner".into(),
            created_at: NOW.into(),
        };
        let registration = RunnerRegistrationRecord {
            id: "runner_quota_final".into(),
            credential_hash: "5".repeat(64),
            version: "2.1.0".into(),
            protocol_version: 3,
            platform: "linux".into(),
            architecture: "x86_64".into(),
            max_inflight: 4,
            created_at: NOW.into(),
        };
        let prepared = event(
            &retired,
            "quota-prepared-again",
            "runner.register",
            "runner_quota_final",
        );
        let write = || RunnerQuotaWrite {
            change: &prepare,
            next_state: &next,
            enrollment: Some(&enrollment),
            registration: Some(&registration),
        };
        assert_eq!(
            store
                .apply_runner_quota_change(
                    write(),
                    Some(1),
                    (retired.sequence, &retired.integrity_hmac, &prepared)
                )
                .await
                .unwrap(),
            Outcome::LimitReached
        );
        assert_eq!(
            store
                .apply_runner_quota_change(write(), Some(2), (0, "", &prepared))
                .await
                .unwrap(),
            Outcome::AuditConflict
        );
        assert_eq!(
            store
                .apply_runner_quota_change(
                    write(),
                    Some(2),
                    (retired.sequence, &retired.integrity_hmac, &prepared)
                )
                .await
                .unwrap(),
            Outcome::Applied
        );
        assert_eq!(
            store
                .apply_runner_quota_change(
                    write(),
                    Some(2),
                    (retired.sequence, &retired.integrity_hmac, &prepared)
                )
                .await
                .unwrap(),
            Outcome::QuotaChanged
        );
        let proof = store
            .runner_quota_snapshot()
            .await
            .unwrap()
            .verify("installation_a", |_| true)
            .unwrap();
        assert_eq!(proof.occupied(), 2);
        assert!(proof.delete_runner("runner_quota_final").is_err());
        let rollback = proof
            .finish_upgrade("quota-upgrade-two", false)
            .unwrap()
            .unwrap();
        let next = signed(&rollback);
        let undone = event(
            &prepared,
            "quota-undone",
            "runner.delete",
            "runner_quota_final",
        );
        assert_eq!(
            store
                .apply_runner_quota_change(
                    RunnerQuotaWrite {
                        change: &rollback,
                        next_state: &next,
                        enrollment: None,
                        registration: None
                    },
                    None,
                    (prepared.sequence, &prepared.integrity_hmac, &undone)
                )
                .await
                .unwrap(),
            Outcome::Applied
        );
        let proof = store
            .runner_quota_snapshot()
            .await
            .unwrap()
            .verify("installation_a", |_| true)
            .unwrap();
        assert!(
            proof
                .finish_upgrade("quota-upgrade-two", false)
                .unwrap()
                .is_none()
        );
        let delete = proof.delete_runner("runner_quota_new").unwrap();
        let next = signed(&delete);
        let deleted = event(
            &undone,
            "quota-deleted",
            "runner.delete",
            "runner_quota_new",
        );
        assert_eq!(
            store
                .apply_runner_quota_change(
                    RunnerQuotaWrite {
                        change: &delete,
                        next_state: &next,
                        enrollment: None,
                        registration: None
                    },
                    None,
                    (undone.sequence, &undone.integrity_hmac, &deleted)
                )
                .await
                .unwrap(),
            Outcome::Applied
        );
        assert_eq!(
            store
                .runner_quota_snapshot()
                .await
                .unwrap()
                .verify("installation_a", |_| true)
                .unwrap()
                .occupied(),
            1
        );
        assert_eq!(
            store.audit_events().await.unwrap().len() as u64,
            before.sequence + 4
        );
    }

    async fn verify_readonly_migration_inspection(store: &MariaDbStore, config: &MariaDbConfig) {
        let before: Vec<(u32, String, String, String)> = sqlx::query_as(
            "SELECT version,name,checksum_sha256,applied_at FROM schema_migrations ORDER BY version",
        ).fetch_all(store.pool()).await.unwrap();
        let report = MariaDbStore::inspect_installed_migrations(config, "installation_a", &[7; 32])
            .await
            .unwrap();
        assert!(report.pending.is_empty());
        for (id, key) in [("installation_b", [7; 32]), ("installation_a", [8; 32])] {
            assert!(matches!(
                MariaDbStore::inspect_installed_migrations(config, id, &key).await,
                Err(StorageError::InstallationMismatch)
            ));
        }
        let last = before.last().unwrap();
        sqlx::query("DELETE FROM schema_migrations WHERE version=?")
            .bind(last.0)
            .execute(store.pool())
            .await
            .unwrap();
        let pending =
            MariaDbStore::inspect_installed_migrations(config, "installation_a", &[7; 32])
                .await
                .unwrap();
        assert_eq!(pending.pending.len(), 1);
        assert_eq!(pending.pending[0].version, last.0);
        assert_eq!(
            store.schema_version().await.unwrap(),
            last.0 - 1,
            "inspection must not apply the missing migration"
        );
        sqlx::query("INSERT INTO schema_migrations(version,name,checksum_sha256,applied_at) VALUES(?,?,?,?)")
            .bind(last.0).bind(&last.1).bind(&last.2).bind(&last.3)
            .execute(store.pool()).await.unwrap();
        let after: Vec<(u32, String, String, String)> = sqlx::query_as(
            "SELECT version,name,checksum_sha256,applied_at FROM schema_migrations ORDER BY version",
        ).fetch_all(store.pool()).await.unwrap();
        assert_eq!(before, after);
    }

    async fn verify_readiness_after_additive_migration(
        store: &MariaDbStore,
        config: &MariaDbConfig,
    ) {
        let mut extended = MARIADB_MIGRATIONS.to_vec();
        extended.push(EmbeddedMigration {
            version: crate::CURRENT_SCHEMA_VERSION + 1,
            name: "readiness-additive-fixture",
            sql: "ALTER TABLE transaction_gates ADD COLUMN readiness_probe_fixture INT NOT NULL DEFAULT 0",
            compatibility: MigrationCompatibility::RollingUpgradeSafe,
                online_sql: None,
                recovery_query: Some("SELECT CASE WHEN COUNT(*)=0 THEN 0 WHEN COUNT(*)=1 AND SUM(DATA_TYPE='int' AND IS_NULLABLE='NO' AND COLUMN_DEFAULT='0' AND EXTRA='')=1 THEN 1 ELSE 2 END FROM information_schema.COLUMNS WHERE TABLE_SCHEMA=DATABASE() AND TABLE_NAME='transaction_gates' AND COLUMN_NAME='readiness_probe_fixture'"),
        });
        store.apply_migrations(&extended).await.unwrap();
        let before: Vec<(String, u64)> =
            sqlx::query_as("SELECT gate_key,revision FROM transaction_gates ORDER BY gate_key")
                .fetch_all(store.pool())
                .await
                .unwrap();
        let (older, newer) = tokio::join!(
            store.verify_read_write(),
            store.verify_read_write_for(&extended),
        );
        older.unwrap();
        newer.unwrap();
        let report = MariaDbStore::inspect_installed_migrations(config, "installation_a", &[7; 32])
            .await
            .unwrap();
        assert!(report.valid());
        assert_eq!(
            report.applied_schema_version,
            crate::CURRENT_SCHEMA_VERSION + 1
        );
        assert_eq!(
            report.candidate_schema_version,
            crate::CURRENT_SCHEMA_VERSION
        );
        assert!(report.pending.is_empty());
        sqlx::query("UPDATE schema_migrations SET checksum_sha256=? WHERE version=1")
            .bind("a".repeat(64))
            .execute(store.pool())
            .await
            .unwrap();
        assert!(matches!(
            store.verify_read_write().await,
            Err(StorageError::UnsupportedSchema { version: 1 })
        ));
        sqlx::query("UPDATE schema_migrations SET checksum_sha256=? WHERE version=1")
            .bind(crate::migration_checksum(MARIADB_MIGRATIONS[0].sql))
            .execute(store.pool())
            .await
            .unwrap();
        sqlx::query("UPDATE schema_migrations SET version=version+2 WHERE version=?")
            .bind(crate::CURRENT_SCHEMA_VERSION + 1)
            .execute(store.pool())
            .await
            .unwrap();
        assert!(matches!(
            store.verify_read_write().await,
            Err(StorageError::MigrationIntegrity)
        ));
        sqlx::query("UPDATE schema_migrations SET version=version-2 WHERE version=?")
            .bind(crate::CURRENT_SCHEMA_VERSION + 3)
            .execute(store.pool())
            .await
            .unwrap();
        store.verify_read_write().await.unwrap();
        let after: Vec<(String, u64)> =
            sqlx::query_as("SELECT gate_key,revision FROM transaction_gates ORDER BY gate_key")
                .fetch_all(store.pool())
                .await
                .unwrap();
        assert_eq!(
            before, after,
            "both catalog probes must roll back their writes"
        );
    }

    #[tokio::test]
    async fn installed_database_is_bound_and_migration_cancellation_releases_session_lock() {
        let Ok(port) = std::env::var("ASTER_CUSTOMER_TEST_DB_PORT") else {
            return;
        };
        let config = MariaDbConfig {
            host: "127.0.0.1".to_owned(),
            port: port.parse().unwrap(),
            database: "aster_customer_fixture".to_owned(),
            username: "aster_team".to_owned(),
            password: std::env::var("ASTER_CUSTOMER_TEST_DB_PASSWORD").unwrap(),
            tls: false,
            ca_certificate: None,
            max_connections: 4,
        };
        assert!(!format!("{config:?}").contains(&config.password));
        assert!(matches!(
            MariaDbStore::open_installed(&config, "installation_a", &[7; 32], false).await,
            Err(StorageError::Uninitialized)
        ));
        let store = MariaDbStore::open_installed(&config, "installation_a", &[7; 32], true)
            .await
            .unwrap();
        assert_eq!(
            store.schema_version().await.unwrap(),
            crate::CURRENT_SCHEMA_VERSION
        );
        assert!(matches!(
            MariaDbStore::open_installed(&config, "installation_b", &[7; 32], false).await,
            Err(StorageError::InstallationMismatch)
        ));
        assert!(matches!(
            MariaDbStore::open_installed(&config, "installation_a", &[8; 32], true).await,
            Err(StorageError::InstallationMismatch)
        ));
        let (blue, green) = tokio::join!(
            MariaDbStore::open_installed(&config, "installation_a", &[7; 32], false),
            MariaDbStore::open_installed(&config, "installation_a", &[7; 32], false)
        );
        blue.unwrap().close().await;
        green.unwrap().close().await;
        verify_atomic_local_runner_registration(&store).await;
        verify_logical_runner_quota_transactions(&store).await;
        let gates_before: Vec<(String, u64)> =
            sqlx::query_as("SELECT gate_key,revision FROM transaction_gates ORDER BY gate_key")
                .fetch_all(store.pool())
                .await
                .unwrap();
        let (first_probe, second_probe) =
            tokio::join!(store.verify_read_write(), store.verify_read_write());
        first_probe.unwrap();
        second_probe.unwrap();
        let gates_after: Vec<(String, u64)> =
            sqlx::query_as("SELECT gate_key,revision FROM transaction_gates ORDER BY gate_key")
                .fetch_all(store.pool())
                .await
                .unwrap();
        assert_eq!(
            gates_before, gates_after,
            "readiness probes must roll back all gates"
        );
        for (gate, revision) in &gates_before {
            sqlx::query("DELETE FROM transaction_gates WHERE gate_key=?")
                .bind(gate)
                .execute(store.pool())
                .await
                .unwrap();
            assert!(
                matches!(
                    store.verify_read_write().await,
                    Err(StorageError::Uninitialized)
                ),
                "missing {gate} must not pass business readiness"
            );
            sqlx::query("INSERT INTO transaction_gates(gate_key,revision) VALUES(?,?)")
                .bind(gate)
                .bind(revision)
                .execute(store.pool())
                .await
                .unwrap();
        }
        let mut readonly_config = config.clone();
        readonly_config.username = std::env::var("ASTER_CUSTOMER_TEST_DB_READONLY_USER").unwrap();
        verify_readonly_migration_inspection(&store, &readonly_config).await;
        let readonly = MariaDbStore::connect(&readonly_config).await.unwrap();
        assert_eq!(
            readonly.schema_version().await.unwrap(),
            crate::CURRENT_SCHEMA_VERSION
        );
        assert!(
            readonly.verify_read_write().await.is_err(),
            "SELECT success cannot hide missing UPDATE permission"
        );
        readonly.close().await;
        let mut locked = store.pool.begin().await.unwrap();
        sqlx::query(
            "UPDATE transaction_gates SET revision=revision+1 WHERE gate_key='member_seat'",
        )
        .execute(&mut *locked)
        .await
        .unwrap();
        assert!(matches!(
            store.verify_read_write().await,
            Err(StorageError::ReadinessTimeout)
        ));
        locked.rollback().await.unwrap();
        store.verify_read_write().await.unwrap();
        let gates_after_timeout: Vec<(String, u64)> =
            sqlx::query_as("SELECT gate_key,revision FROM transaction_gates ORDER BY gate_key")
                .fetch_all(store.pool())
                .await
                .unwrap();
        assert_eq!(gates_before, gates_after_timeout);
        let worker_store = store.clone();
        let worker = tokio::spawn(async move {
            let mut migrations = MARIADB_MIGRATIONS.to_vec();
            migrations.push(EmbeddedMigration {
                version: crate::CURRENT_SCHEMA_VERSION + 1,
                name: "cancelled-test",
                sql: "SELECT SLEEP(10)",
                compatibility: MigrationCompatibility::RollingUpgradeSafe,
                online_sql: None,
                recovery_query: Some("SELECT 0"),
            });
            worker_store.apply_migrations(&migrations).await
        });
        let mut observer = store.pool.acquire().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let owner: Option<i64> =
                    sqlx::query_scalar("SELECT IS_USED_LOCK('aster_team_schema_migrations')")
                        .fetch_one(&mut *observer)
                        .await
                        .unwrap();
                if owner.is_some() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        worker.abort();
        assert!(worker.await.unwrap_err().is_cancelled());
        let acquired: Option<i64> =
            sqlx::query_scalar("SELECT GET_LOCK('aster_team_schema_migrations', 15)")
                .fetch_one(&mut *observer)
                .await
                .unwrap();
        assert_eq!(
            acquired,
            Some(1),
            "a cancelled migration must not leave a pooled session holding the lock"
        );
        let _: Option<i64> =
            sqlx::query_scalar("SELECT RELEASE_LOCK('aster_team_schema_migrations')")
                .fetch_one(&mut *observer)
                .await
                .unwrap();
        drop(observer);
        assert_eq!(
            store.schema_version().await.unwrap(),
            crate::CURRENT_SCHEMA_VERSION
        );
        if let Ok(certificate) = std::env::var("ASTER_CUSTOMER_TEST_DB_CA") {
            let mut secure = config.clone();
            secure.tls = true;
            secure.ca_certificate = Some(certificate.into());
            let secure_store =
                MariaDbStore::open_installed(&secure, "installation_a", &[7; 32], false)
                    .await
                    .unwrap();
            secure_store.close().await;
            secure.ca_certificate = None;
            assert!(
                MariaDbStore::open_installed(&secure, "installation_a", &[7; 32], false)
                    .await
                    .is_err(),
                "an untrusted database certificate must fail closed"
            );
        }
        super::migration_policy::tests::verify_online_policy(&config).await;
        verify_readiness_after_additive_migration(&store, &config).await;
        store.close().await;
    }

    #[test]
    fn baseline_has_serialized_integrity_chained_audit_events() {
        assert!(MARIADB_BASELINE.contains("('audit_log', 0)"));
        assert!(MARIADB_BASELINE.contains("CREATE TABLE audit_events"));
        assert!(MARIADB_BASELINE.contains("event_sequence BIGINT UNSIGNED NOT NULL UNIQUE"));
        assert!(MARIADB_BASELINE.contains("previous_event_hmac CHAR(43) NOT NULL"));
        let audit_table = MARIADB_BASELINE
            .split_once("CREATE TABLE audit_events")
            .expect("audit table exists")
            .1
            .split_once(") ENGINE=InnoDB")
            .expect("audit table closes")
            .0;
        assert!(!audit_table.contains("details_json"));
    }
}

async fn lock_runner_quota_mariadb(
    transaction: &mut sqlx::Transaction<'_, MySql>,
) -> Result<(), StorageError> {
    sqlx::query_scalar::<MySql, u64>(
        "SELECT revision FROM transaction_gates WHERE gate_key='entity_quota' FOR UPDATE",
    )
    .fetch_one(&mut **transaction)
    .await?;
    Ok(())
}

async fn runner_quota_snapshot_mariadb(
    transaction: &mut sqlx::Transaction<'_, MySql>,
) -> Result<RunnerQuotaSnapshot, StorageError> {
    let state = sqlx::query(
        "SELECT state_key,state_value,revision,mac,updated_at FROM security_state WHERE state_key=? FOR UPDATE",
    ).bind(RUNNER_QUOTA_KEY).fetch_optional(&mut **transaction).await?
        .map(security_state_from_mariadb_row).transpose()?;
    let members = sqlx::query("SELECT id,credential_hash FROM runners ORDER BY id FOR UPDATE")
        .fetch_all(&mut **transaction)
        .await?
        .into_iter()
        .map(|row| {
            Ok(RunnerQuotaMember {
                id: row.try_get("id")?,
                credential_hash: row.try_get("credential_hash")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    Ok(RunnerQuotaSnapshot { state, members })
}

impl MariaDbStore {
    pub async fn apply_runner_quota_change(
        &self,
        write: crate::runner_quota::RunnerQuotaWrite<'_>,
        runner_limit: Option<u32>,
        audit: (u64, &str, &AuditEventRecord),
    ) -> Result<crate::runner_quota::RunnerQuotaWriteOutcome, StorageError> {
        use crate::runner_quota::{RunnerQuotaAction, RunnerQuotaWriteOutcome as Outcome};
        if !write.valid(audit.2) {
            return Err(StorageError::RunnerQuotaIntegrity);
        }
        let mut transaction = self.pool.begin().await?;
        lock_runner_quota_mariadb(&mut transaction).await?;
        if !write
            .change
            .matches_before(&runner_quota_snapshot_mariadb(&mut transaction).await?)
        {
            transaction.rollback().await?;
            return Ok(Outcome::QuotaChanged);
        }
        match write.change.action() {
            RunnerQuotaAction::Register(_) => {
                if runner_limit.is_some_and(|limit| write.change.occupied_before() > limit) {
                    transaction.rollback().await?;
                    return Ok(Outcome::LimitReached);
                }
                let enrollment = write.enrollment.ok_or(StorageError::RunnerQuotaIntegrity)?;
                let registration = write
                    .registration
                    .ok_or(StorageError::RunnerQuotaIntegrity)?;
                let seen: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM runner_enrollments WHERE id=? OR runner_id=?",
                )
                .bind(&enrollment.id)
                .bind(&registration.id)
                .fetch_one(&mut *transaction)
                .await?;
                if seen != 0 {
                    transaction.rollback().await?;
                    return Ok(Outcome::Conflict);
                }
                sqlx::query("INSERT INTO runner_enrollments(id,token_hash,token_prefix,runner_name,status,expires_at,created_by,created_at) VALUES(?,?,?,?,'pending',?,?,?)")
                    .bind(&enrollment.id).bind(&enrollment.token_hash).bind(&enrollment.token_prefix).bind(&enrollment.runner_name)
                    .bind(&enrollment.expires_at).bind(&enrollment.created_by).bind(&enrollment.created_at).execute(&mut *transaction).await?;
                sqlx::query("INSERT INTO runners(id,enrollment_id,name,credential_hash,enabled,version,protocol_version,platform,architecture,max_inflight,created_at,updated_at) VALUES(?,?,?,?,1,?,?,?,?,?,?,?)")
                    .bind(&registration.id).bind(&enrollment.id).bind(&enrollment.runner_name).bind(&registration.credential_hash)
                    .bind(&registration.version).bind(registration.protocol_version).bind(&registration.platform).bind(&registration.architecture)
                    .bind(registration.max_inflight).bind(&registration.created_at).bind(&registration.created_at).execute(&mut *transaction).await?;
                sqlx::query(
                    "UPDATE runner_enrollments SET status='used',used_at=?,runner_id=? WHERE id=?",
                )
                .bind(&registration.created_at)
                .bind(&registration.id)
                .bind(&enrollment.id)
                .execute(&mut *transaction)
                .await?;
            }
            RunnerQuotaAction::Delete(member) => {
                if sqlx::query("DELETE FROM runners WHERE id=? AND credential_hash=?")
                    .bind(&member.id)
                    .bind(&member.credential_hash)
                    .execute(&mut *transaction)
                    .await?
                    .rows_affected()
                    != 1
                {
                    transaction.rollback().await?;
                    return Ok(Outcome::Conflict);
                }
            }
        }
        if !write.change.matches_after(
            &runner_quota_snapshot_mariadb(&mut transaction)
                .await?
                .members,
        ) {
            return Err(StorageError::RunnerQuotaIntegrity);
        }
        let next = write.next_state;
        sqlx::query("INSERT INTO security_state(state_key,state_value,revision,mac,updated_at) VALUES(?,?,?,?,?) ON DUPLICATE KEY UPDATE state_value=VALUES(state_value),revision=VALUES(revision),mac=VALUES(mac),updated_at=VALUES(updated_at)")
            .bind(&next.key).bind(&next.value).bind(next.revision).bind(&next.mac).bind(&next.updated_at).execute(&mut *transaction).await?;
        if !append_audit_events_mariadb(
            &mut transaction,
            audit.0,
            audit.1,
            std::slice::from_ref(audit.2),
        )
        .await?
        {
            transaction.rollback().await?;
            return Ok(Outcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(Outcome::Applied)
    }
}
