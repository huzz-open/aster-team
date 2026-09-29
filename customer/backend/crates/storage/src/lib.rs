#![forbid(unsafe_code)]

#[cfg(all(feature = "sqlite-dev", feature = "sqlcipher"))]
compile_error!("sqlite-dev and sqlcipher are mutually exclusive storage features");

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
use std::path::Path;

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
use rusqlite::OptionalExtension;
#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior, params};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
use zeroize::Zeroizing;

#[cfg(feature = "mariadb")]
mod mariadb;
#[cfg(feature = "mariadb")]
pub use mariadb::{
    MariaDbConfig, MariaDbMigrationPolicy, MariaDbStore, MigrationInspection, PendingMigration,
};

pub mod runner_quota;
mod upstream_connections;
pub use upstream_connections::{
    ConnectionCredential, ConnectionModelBinding, ConnectionModelOrigin, ConnectionModelSpec,
    ConnectionModelSync, ConnectionModelSyncOutcome, ConnectionMutationOutcome, ConnectionRecord,
    ConnectionRoute,
};
mod model_access;
pub use model_access::{ModelAccessPolicyRecord, ModelAccessWriteOutcome};
mod image_quota;
mod money_ledger;
pub use image_quota::{
    ImageBalanceRecord, ImageLedgerRecord, ImageQuotaMutationOutcome, ImageReservationRecord,
    valid_image_balance_transition,
};
pub use money_ledger::{
    MoneyBalanceRecord, MoneyLedgerEntry, MoneyMutationOutcome, MoneyStateSnapshot,
    valid_money_transition,
};

#[derive(Clone, Debug)]
pub struct QuotaBatchImageWrite {
    pub expected_balance: Option<ImageBalanceRecord>,
    pub next_balance: ImageBalanceRecord,
    pub ledger: ImageLedgerRecord,
}

#[derive(Clone, Debug)]
pub struct QuotaBatchTokenWrite {
    pub expected_balance: UserBalanceRecord,
    pub next_balance: UserBalanceRecord,
    pub ledger: QuotaLedgerEntry,
}
#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
use runner_quota::{RUNNER_QUOTA_KEY, RunnerQuotaMember, RunnerQuotaPolicy, RunnerQuotaSnapshot};

pub const CURRENT_SCHEMA_VERSION: u32 = 1;
#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
const SQLCIPHER_BASELINE: &str = include_str!("../../../schema/init.sqlcipher.sql");

#[derive(Clone, Copy, Debug)]
struct EmbeddedMigration {
    version: u32,
    name: &'static str,
    sql: &'static str,
    compatibility: MigrationCompatibility,
    /// Non-transactional engines require a trusted query returning 0 (absent),
    /// 1 (complete), or another value (ambiguous). It never authorizes online DDL.
    recovery_query: Option<&'static str>,
    /// Explicit online statement with the same effects; never inferred from a compatibility label.
    online_sql: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MigrationCompatibility {
    Baseline,
    RollingUpgradeSafe,
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
const SQLCIPHER_MIGRATIONS: &[EmbeddedMigration] = &[EmbeddedMigration {
    version: 1,
    name: "baseline-2.2.0",
    sql: SQLCIPHER_BASELINE,
    compatibility: MigrationCompatibility::Baseline,
    online_sql: None,
    recovery_query: None,
}];

#[derive(Debug, Error)]
pub enum StorageError {
    #[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
    #[error("SQLCipher database operation failed: {0}")]
    Database(#[from] rusqlite::Error),
    #[cfg(feature = "mariadb")]
    #[error("MariaDB operation failed: {0}")]
    MariaDb(#[from] sqlx::Error),
    #[error("SQLCipher support is required but unavailable")]
    SqlCipherUnavailable,
    #[error("external database installation identity does not match or is unbound")]
    InstallationMismatch,
    #[error("database server version is outside the supported installation matrix")]
    UnsupportedServer,
    #[error(
        "database migration or validation exceeded its client deadline; inspect effects before retrying"
    )]
    MigrationTimeout,
    #[error("database readiness probe exceeded its deadline")]
    ReadinessTimeout,
    #[error("database is not initialized")]
    Uninitialized,
    #[error("database schema migration {version} is missing or does not match this application")]
    UnsupportedSchema { version: u32 },
    #[error("database initialization requires an empty database")]
    DatabaseNotEmpty,
    #[error("database migration failed integrity validation")]
    MigrationIntegrity,
    #[error("Runner quota binding failed integrity validation")]
    RunnerQuotaIntegrity,
    #[error("credential instance already exists")]
    DuplicateCredentialIdentity,
    #[error("identity email already exists")]
    DuplicateIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageFailureKind {
    Connection,
    Schema,
    Decode,
    Query,
    LocalState,
}

impl StorageError {
    pub fn failure_kind(&self) -> StorageFailureKind {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(error) => match error {
                sqlx::Error::PoolTimedOut
                | sqlx::Error::PoolClosed
                | sqlx::Error::Io(_)
                | sqlx::Error::Tls(_)
                | sqlx::Error::Protocol(_) => StorageFailureKind::Connection,
                sqlx::Error::ColumnDecode { .. } | sqlx::Error::Decode(_) => {
                    StorageFailureKind::Decode
                }
                sqlx::Error::ColumnNotFound(_) | sqlx::Error::TypeNotFound { .. } => {
                    StorageFailureKind::Schema
                }
                sqlx::Error::Database(database) => match database.code().as_deref() {
                    Some("1044" | "1045" | "1049") => StorageFailureKind::Connection,
                    Some("1054" | "1146") => StorageFailureKind::Schema,
                    _ => StorageFailureKind::Query,
                },
                _ => StorageFailureKind::Query,
            },
            #[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
            Self::Database(error) => match error {
                rusqlite::Error::InvalidColumnType(..)
                | rusqlite::Error::FromSqlConversionFailure(..) => StorageFailureKind::Decode,
                rusqlite::Error::InvalidColumnName(..) => StorageFailureKind::Schema,
                rusqlite::Error::SqliteFailure(_, _) => StorageFailureKind::Query,
                _ => StorageFailureKind::LocalState,
            },
            Self::UnsupportedSchema { .. }
            | Self::MigrationIntegrity
            | Self::RunnerQuotaIntegrity
            | Self::UnsupportedServer => StorageFailureKind::Schema,
            Self::SqlCipherUnavailable | Self::Uninitialized | Self::ReadinessTimeout => {
                StorageFailureKind::Connection
            }
            Self::InstallationMismatch | Self::MigrationTimeout | Self::DatabaseNotEmpty => {
                StorageFailureKind::LocalState
            }
            Self::DuplicateCredentialIdentity | Self::DuplicateIdentity => {
                StorageFailureKind::Query
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityRecord {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub password_hash: String,
    pub role: String,
    pub status: String,
    pub can_consume_model: bool,
    pub password_change_required: bool,
    pub revision: u32,
    pub integrity_hmac: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityCreateOutcome {
    Created,
    SeatLimitReached,
    SecurityStateChanged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceCreateOutcome {
    Created,
    LimitReached,
    Conflict,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityInitializeOutcome {
    Created,
    AlreadyInitialized,
    SecurityStateChanged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityStatusUpdateOutcome {
    Updated,
    Conflict,
    SecurityStateChanged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecurityStateRecord {
    pub key: String,
    pub value: Vec<u8>,
    pub revision: u64,
    pub mac: Vec<u8>,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SeatRegistrySnapshot {
    pub state: Option<SecurityStateRecord>,
    pub occupied_identity_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserBalanceRecord {
    pub identity_id: String,
    pub balance_tokens: i64,
    pub reserved_tokens: i64,
    pub granted_tokens: i64,
    pub consumed_tokens: i64,
    pub request_count: i64,
    pub raw_tokens: i64,
    pub billed_tokens: i64,
    pub uncached_input: i64,
    pub cached_input: i64,
    pub cache_write: i64,
    pub output_tokens: i64,
    pub revision: u64,
    pub last_ledger_hmac: String,
    pub integrity_hmac: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuotaReservationRecord {
    pub id: String,
    pub identity_id: String,
    pub api_key_id: String,
    pub request_id: String,
    pub reserved_tokens: i64,
    pub status: String,
    pub revision: u64,
    pub integrity_hmac: String,
    pub created_at: String,
    pub expires_at: String,
    pub settled_at: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuotaLedgerEntry {
    pub id: String,
    pub identity_id: String,
    pub kind: String,
    pub amount_tokens: i64,
    pub uncached_input: i64,
    pub cached_input: i64,
    pub cache_write: i64,
    pub output_tokens: i64,
    pub uncovered_tokens: i64,
    pub raw_tokens: i64,
    pub billed_tokens: i64,
    pub multiplier_micros: i64,
    pub reference_id: String,
    pub client_request_id: Option<String>,
    pub description: String,
    pub protocol: String,
    pub model: String,
    pub requested_model: Option<String>,
    pub processing_tier: Option<String>,
    pub reasoning_effort: Option<String>,
    pub api_key_id: Option<String>,
    pub runner_id: Option<String>,
    pub previous_entry_hmac: String,
    pub integrity_hmac: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuotaStateSnapshot {
    pub balance: UserBalanceRecord,
    pub active_reservations: Vec<QuotaReservationRecord>,
    pub ledger_entries: Vec<QuotaLedgerEntry>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuotaMutationOutcome {
    Applied,
    Conflict,
    Insufficient,
    DuplicateRequest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuotaRequestRecord {
    pub id: String,
    pub identity_id: String,
    pub amount_nanos: i64,
    pub reason: String,
    pub status: String,
    pub review_note: String,
    pub reviewed_by: Option<String>,
    pub reviewed_at: Option<String>,
    pub revision: u32,
    pub integrity_hmac: String,
    pub created_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuotaRequestInsertOutcome {
    Inserted,
    PendingExists,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuotaRequestReviewOutcome {
    Applied,
    Conflict,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoucherRecord {
    pub id: String,
    pub code_hash: String,
    pub code_prefix: String,
    pub name: String,
    pub quota_tokens: i64,
    pub status: String,
    pub max_redemptions: u32,
    pub redeemed_count: u32,
    pub expires_at: Option<String>,
    pub created_by: String,
    pub revision: u32,
    pub integrity_hmac: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoucherDeliveryRecord {
    pub id: String,
    pub voucher_id: String,
    pub identity_id: String,
    pub status: String,
    pub delivered_at: String,
    pub redeemed_at: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoucherRedemptionRecord {
    pub id: String,
    pub voucher_id: String,
    pub identity_id: String,
    pub amount_tokens: i64,
    pub ledger_entry_id: String,
    pub created_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VoucherRedeemOutcome {
    Applied,
    Unavailable,
    AlreadyRedeemed,
    Conflict,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeSettingRecord {
    pub key: String,
    pub value: String,
    pub revision: u64,
    pub integrity_hmac: String,
    pub updated_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeSettingWriteOutcome {
    Applied,
    Conflict,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditEventRecord {
    pub id: String,
    pub sequence: u64,
    pub actor_identity_id: Option<String>,
    pub actor_role: String,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<String>,
    pub outcome: String,
    pub previous_event_hmac: String,
    pub integrity_hmac: String,
    pub created_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditAppendOutcome {
    Applied,
    Conflict,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditedMutationOutcome {
    Applied,
    MutationConflict,
    AuditConflict,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MutationWithAuditOutcome<T> {
    Mutation(T),
    AuditConflict,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionRecord {
    pub id: String,
    pub identity_id: String,
    pub token_hash: String,
    pub expires_at: String,
    pub integrity_hmac: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedSession {
    pub session: SessionRecord,
    pub identity: IdentityRecord,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApiKeyRecord {
    pub id: String,
    pub identity_id: String,
    pub name: String,
    pub key_hash: String,
    pub key_prefix: String,
    pub status: String,
    pub revision: u32,
    pub integrity_hmac: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizedApiKey {
    pub api_key: ApiKeyRecord,
    pub identity: IdentityRecord,
}

fn valid_reservation_subject(
    current: Option<&AuthorizedApiKey>,
    expected: &AuthorizedApiKey,
    reservation: &QuotaReservationRecord,
) -> bool {
    let Some(current) = current else {
        return false;
    };
    let mut current_key = current.api_key.clone();
    // This unsigned usage timestamp is telemetry, not an authorization change.
    current_key
        .last_used_at
        .clone_from(&expected.api_key.last_used_at);
    current_key == expected.api_key
        && current.identity == expected.identity
        && current.identity.id == reservation.identity_id
        && current.api_key.id == reservation.api_key_id
        && current.api_key.identity_id == current.identity.id
        && current.api_key.status == "active"
        && current.identity.status == "active"
        && current.identity.role == "member"
        && current.identity.can_consume_model
        && !current.identity.password_change_required
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelRecord {
    pub id: String,
    pub public_name: String,
    pub display_name: String,
    pub enabled: bool,
    pub discovered_at: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveredModel {
    pub id: String,
    pub public_name: String,
    pub display_name: String,
    pub upstream_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayRouteCandidate {
    pub model_id: String,
    pub public_model: String,
    pub upstream_model: String,
    pub account_id: String,
    pub provider: String,
    pub upstream_subject_id: String,
    pub last_success_runner_id: Option<String>,
    pub credential: EncryptedCredentialInstance,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpstreamAccountRecord {
    pub id: String,
    pub provider: String,
    pub subject_id: String,
    pub email: String,
    pub plan: String,
    pub status: String,
    pub last_success_runner_id: Option<String>,
    pub last_verified_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncryptedCredentialInstance {
    pub id: String,
    pub account_id: String,
    pub credential_identity_hmac: String,
    pub encrypted_payload: Vec<u8>,
    pub payload_nonce: Vec<u8>,
    pub wrapped_data_key: Vec<u8>,
    pub wrap_nonce: Vec<u8>,
    pub credential_revision: u32,
    pub expires_at: String,
    pub status: String,
    pub last_refreshed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialRefreshUpdate {
    pub credential_identity_hmac: String,
    pub encrypted_payload: Vec<u8>,
    pub payload_nonce: Vec<u8>,
    pub wrapped_data_key: Vec<u8>,
    pub wrap_nonce: Vec<u8>,
    pub expires_at: String,
    pub refreshed_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialRefreshLeaseOutcome {
    Acquired,
    Busy,
    RevisionChanged,
    CredentialUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunnerRecord {
    pub id: String,
    pub enabled: bool,
    pub version: String,
    pub protocol_version: u32,
    pub max_inflight: u32,
    pub inflight: u32,
    pub recent_request_count: u32,
    pub recent_error_count: u32,
    pub latency_ms: u32,
    pub last_seen_at: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunnerAdminRecord {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub version: String,
    pub protocol_version: u32,
    pub platform: String,
    pub architecture: String,
    pub max_inflight: u32,
    pub inflight: u32,
    pub recent_request_count: u32,
    pub recent_error_count: u32,
    pub latency_ms: u32,
    pub last_seen_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunnerEnrollmentRecord {
    pub id: String,
    pub token_hash: String,
    pub token_prefix: String,
    pub runner_name: String,
    pub status: String,
    pub expires_at: String,
    pub created_by: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunnerRegistrationRecord {
    pub id: String,
    pub credential_hash: String,
    pub version: String,
    pub protocol_version: u32,
    pub platform: String,
    pub architecture: String,
    pub max_inflight: u32,
    pub created_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunnerEnrollmentConsumeOutcome {
    Registered,
    QuotaChanged,
    LimitReached,
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunnerHeartbeatUpdate {
    pub inflight: u32,
    pub recent_request_count: u32,
    pub recent_error_count: u32,
    pub latency_ms: u32,
}

/// A connection update must still possess the registration credential at the
/// database commit boundary. Protocol transitions are monotonic; the Control
/// handshake selects the one predecessor supported by its current binary.
#[derive(Clone)]
pub struct RunnerConnectionUpdate {
    pub runner_id: String,
    pub credential_hash: String,
    pub previous_protocol_version: u32,
    pub protocol_version: u32,
    pub runner_version: String,
    pub max_inflight: u32,
    pub heartbeat: RunnerHeartbeatUpdate,
    pub observed_at: String,
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
pub struct SqlCipherStore {
    connection: Connection,
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
impl SqlCipherStore {
    pub fn initialize(path: &Path, key: &[u8; 32]) -> Result<Self, StorageError> {
        let store = Self::connect(path, key)?;
        store.apply_migrations(SQLCIPHER_MIGRATIONS)?;
        Ok(store)
    }

    pub fn open(path: &Path, key: &[u8; 32]) -> Result<Self, StorageError> {
        let store = Self::connect(path, key)?;
        store.apply_migrations(SQLCIPHER_MIGRATIONS)?;
        Ok(store)
    }

    fn connect(path: &Path, key: &[u8; 32]) -> Result<Self, StorageError> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let key_hex = Zeroizing::new(hex_key(key));
        connection.execute_batch(&format!(
            "PRAGMA key = \"x'{}'\";\nPRAGMA foreign_keys = ON;\nPRAGMA journal_mode = WAL;\nPRAGMA busy_timeout = 5000;",
            key_hex.as_str()
        ))?;
        #[cfg(feature = "sqlcipher")]
        {
            let cipher_version: Option<String> = connection
                .query_row("PRAGMA cipher_version", [], |row| row.get(0))
                .optional()?;
            if cipher_version.as_deref().is_none_or(str::is_empty) {
                return Err(StorageError::SqlCipherUnavailable);
            }
        }
        connection.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))?;
        Ok(Self { connection })
    }

    pub fn schema_version(&self) -> Result<u32, StorageError> {
        self.connection
            .query_row("SELECT max(version) FROM schema_migrations", [], |row| {
                row.get::<_, Option<u32>>(0)
            })?
            .ok_or(StorageError::Uninitialized)
    }

    pub fn occupied_seats(&self) -> Result<u32, StorageError> {
        let count = self.connection.query_row(
            "SELECT count(*) FROM identities WHERE status='active' AND can_consume_model=1",
            [],
            |row| row.get::<_, u32>(0),
        )?;
        Ok(count)
    }

    pub fn runner_quota_snapshot(&self) -> Result<RunnerQuotaSnapshot, StorageError> {
        let transaction = self.connection.unchecked_transaction()?;
        let result = runner_quota_snapshot_sqlite(&transaction)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn seat_registry_snapshot(&self, key: &str) -> Result<SeatRegistrySnapshot, StorageError> {
        let state = self
            .connection
            .query_row(
                "SELECT key,value,revision,mac,updated_at FROM security_state WHERE key=?",
                [key],
                |row| {
                    Ok(SecurityStateRecord {
                        key: row.get(0)?,
                        value: row.get(1)?,
                        revision: row.get::<_, i64>(2)? as u64,
                        mac: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                },
            )
            .optional()?;
        let occupied_identity_ids = occupied_identity_ids_sqlite(&self.connection)?;
        Ok(SeatRegistrySnapshot {
            state,
            occupied_identity_ids,
        })
    }

    pub fn create_identity_with_seat_limit(
        &mut self,
        identity: &IdentityRecord,
        member_seat_limit: u32,
        expected_state: &SecurityStateRecord,
        next_state: &SecurityStateRecord,
        initial_balance: &UserBalanceRecord,
    ) -> Result<IdentityCreateOutcome, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "UPDATE transaction_gates SET revision=revision+1 WHERE gate_key='member_seat'",
            [],
        )?;
        let current_state = transaction
            .query_row(
                "SELECT key,value,revision,mac,updated_at FROM security_state WHERE key=?",
                [&expected_state.key],
                |row| {
                    Ok(SecurityStateRecord {
                        key: row.get(0)?,
                        value: row.get(1)?,
                        revision: row.get::<_, i64>(2)? as u64,
                        mac: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                },
            )
            .optional()?;
        let current_occupied = occupied_identity_ids_sqlite(&transaction)?;
        if current_state.as_ref() != Some(expected_state)
            || expected_state.value != encode_seat_registry(&current_occupied)
        {
            transaction.rollback()?;
            return Ok(IdentityCreateOutcome::SecurityStateChanged);
        }
        if identity.status == "active" && identity.can_consume_model {
            let occupied = u32::try_from(current_occupied.len())
                .map_err(|_| StorageError::DatabaseNotEmpty)?;
            if occupied >= member_seat_limit {
                transaction.rollback()?;
                return Ok(IdentityCreateOutcome::SeatLimitReached);
            }
        }
        transaction
            .execute(
                "INSERT INTO identities(
                   id,email,display_name,password_hash,role,status,can_consume_model,
                   password_change_required,revision,integrity_hmac,created_at,updated_at
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    identity.id,
                    identity.email,
                    identity.display_name,
                    identity.password_hash,
                    identity.role,
                    identity.status,
                    identity.can_consume_model,
                    identity.password_change_required,
                    identity.revision,
                    identity.integrity_hmac,
                    identity.created_at,
                    identity.updated_at,
                ],
            )
            .map_err(map_sqlite_identity_write_error)?;
        insert_initial_balance_sqlite(&transaction, identity, initial_balance)?;
        let updated_occupied = occupied_identity_ids_sqlite(&transaction)?;
        if next_state.key != expected_state.key
            || next_state.revision != expected_state.revision.saturating_add(1)
            || next_state.value != encode_seat_registry(&updated_occupied)
        {
            transaction.rollback()?;
            return Ok(IdentityCreateOutcome::SecurityStateChanged);
        }
        let changed = transaction.execute(
            "UPDATE security_state SET value=?,revision=?,mac=?,updated_at=?
             WHERE key=? AND value=? AND revision=? AND mac=? AND updated_at=?",
            params![
                next_state.value,
                i64::try_from(next_state.revision).map_err(|_| StorageError::DatabaseNotEmpty)?,
                next_state.mac,
                next_state.updated_at,
                expected_state.key,
                expected_state.value,
                i64::try_from(expected_state.revision)
                    .map_err(|_| StorageError::DatabaseNotEmpty)?,
                expected_state.mac,
                expected_state.updated_at,
            ],
        )?;
        if changed != 1 {
            transaction.rollback()?;
            return Ok(IdentityCreateOutcome::SecurityStateChanged);
        }
        transaction.commit()?;
        Ok(IdentityCreateOutcome::Created)
    }

    pub fn create_identities_with_seat_limit(
        &mut self,
        identities: &[IdentityRecord],
        member_seat_limit: u32,
        expected_state: &SecurityStateRecord,
        next_state: &SecurityStateRecord,
        initial_balances: &[UserBalanceRecord],
    ) -> Result<IdentityCreateOutcome, StorageError> {
        match self.create_identities_with_seat_limit_inner(
            identities,
            member_seat_limit,
            expected_state,
            next_state,
            initial_balances,
            None,
            &[],
        )? {
            MutationWithAuditOutcome::Mutation(outcome) => Ok(outcome),
            MutationWithAuditOutcome::AuditConflict => Err(StorageError::DatabaseNotEmpty),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_identities_with_seat_limit_and_audit(
        &mut self,
        identities: &[IdentityRecord],
        member_seat_limit: u32,
        expected_state: &SecurityStateRecord,
        next_state: &SecurityStateRecord,
        initial_balances: &[UserBalanceRecord],
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_events: &[AuditEventRecord],
        model_policies: &[ModelAccessPolicyRecord],
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
    }

    #[allow(clippy::too_many_arguments)]
    fn create_identities_with_seat_limit_inner(
        &mut self,
        identities: &[IdentityRecord],
        member_seat_limit: u32,
        expected_state: &SecurityStateRecord,
        next_state: &SecurityStateRecord,
        initial_balances: &[UserBalanceRecord],
        audit: Option<(u64, &str, &[AuditEventRecord])>,
        model_policies: &[ModelAccessPolicyRecord],
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
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "UPDATE transaction_gates SET revision=revision+1 WHERE gate_key='member_seat'",
            [],
        )?;
        let current_state = transaction
            .query_row(
                "SELECT key,value,revision,mac,updated_at FROM security_state WHERE key=?",
                [&expected_state.key],
                |row| {
                    Ok(SecurityStateRecord {
                        key: row.get(0)?,
                        value: row.get(1)?,
                        revision: row.get::<_, i64>(2)? as u64,
                        mac: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                },
            )
            .optional()?;
        let current_occupied = occupied_identity_ids_sqlite(&transaction)?;
        if current_state.as_ref() != Some(expected_state)
            || expected_state.value != encode_seat_registry(&current_occupied)
        {
            transaction.rollback()?;
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
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                IdentityCreateOutcome::SeatLimitReached,
            ));
        }
        for (identity, initial_balance) in identities.iter().zip(initial_balances) {
            transaction
                .execute(
                    "INSERT INTO identities(
                       id,email,display_name,password_hash,role,status,can_consume_model,
                       password_change_required,revision,integrity_hmac,created_at,updated_at
                     ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
                    params![
                        identity.id,
                        identity.email,
                        identity.display_name,
                        identity.password_hash,
                        identity.role,
                        identity.status,
                        identity.can_consume_model,
                        identity.password_change_required,
                        identity.revision,
                        identity.integrity_hmac,
                        identity.created_at,
                        identity.updated_at,
                    ],
                )
                .map_err(map_sqlite_identity_write_error)?;
            insert_initial_balance_sqlite(&transaction, identity, initial_balance)?;
        }
        for policy in model_policies {
            transaction.execute(
                "INSERT INTO identity_model_policies(identity_id,mode,revision,grant_set_digest,integrity_hmac,updated_by,updated_at)
                 VALUES(?,?,?,?,?,?,?)",
                params![policy.identity_id,policy.mode,policy.revision,policy.grant_set_digest,
                    policy.integrity_hmac,policy.updated_by,policy.updated_at],
            )?;
        }
        let updated_occupied = occupied_identity_ids_sqlite(&transaction)?;
        if next_state.key != expected_state.key
            || expected_state.revision.checked_add(1) != Some(next_state.revision)
            || next_state.value != encode_seat_registry(&updated_occupied)
        {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                IdentityCreateOutcome::SecurityStateChanged,
            ));
        }
        let changed = transaction.execute(
            "UPDATE security_state SET value=?,revision=?,mac=?,updated_at=?
             WHERE key=? AND value=? AND revision=? AND mac=? AND updated_at=?",
            params![
                next_state.value,
                i64::try_from(next_state.revision).map_err(|_| StorageError::DatabaseNotEmpty)?,
                next_state.mac,
                next_state.updated_at,
                expected_state.key,
                expected_state.value,
                i64::try_from(expected_state.revision)
                    .map_err(|_| StorageError::DatabaseNotEmpty)?,
                expected_state.mac,
                expected_state.updated_at,
            ],
        )?;
        if changed != 1 {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                IdentityCreateOutcome::SecurityStateChanged,
            ));
        }
        if let Some((expected_sequence, expected_hmac, events)) = audit
            && !append_audit_events_sqlite(&transaction, expected_sequence, expected_hmac, events)?
        {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(MutationWithAuditOutcome::Mutation(
            IdentityCreateOutcome::Created,
        ))
    }

    pub fn initialize_first_owner(
        &mut self,
        identity: &IdentityRecord,
        initial_state: &SecurityStateRecord,
        initial_balance: &UserBalanceRecord,
    ) -> Result<IdentityInitializeOutcome, StorageError> {
        match self.initialize_first_owner_inner(identity, initial_state, initial_balance, None)? {
            MutationWithAuditOutcome::Mutation(outcome) => Ok(outcome),
            MutationWithAuditOutcome::AuditConflict => Err(StorageError::DatabaseNotEmpty),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn initialize_first_owner_and_audit(
        &mut self,
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
    }

    fn initialize_first_owner_inner(
        &mut self,
        identity: &IdentityRecord,
        initial_state: &SecurityStateRecord,
        initial_balance: &UserBalanceRecord,
        audit: Option<(u64, &str, &[AuditEventRecord])>,
    ) -> Result<MutationWithAuditOutcome<IdentityInitializeOutcome>, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "UPDATE transaction_gates SET revision=revision+1 WHERE gate_key='member_seat'",
            [],
        )?;
        let identity_count =
            transaction.query_row("SELECT count(*) FROM identities", [], |row| {
                row.get::<_, u32>(0)
            })?;
        if identity_count != 0 {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                IdentityInitializeOutcome::AlreadyInitialized,
            ));
        }
        let state_count = transaction.query_row(
            "SELECT count(*) FROM security_state WHERE key=?",
            [&initial_state.key],
            |row| row.get::<_, u32>(0),
        )?;
        if state_count != 0 {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                IdentityInitializeOutcome::SecurityStateChanged,
            ));
        }
        transaction.execute(
            "INSERT INTO identities(
               id,email,display_name,password_hash,role,status,can_consume_model,
               password_change_required,revision,integrity_hmac,created_at,updated_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
            params![
                identity.id,
                identity.email,
                identity.display_name,
                identity.password_hash,
                identity.role,
                identity.status,
                identity.can_consume_model,
                identity.password_change_required,
                identity.revision,
                identity.integrity_hmac,
                identity.created_at,
                identity.updated_at,
            ],
        )?;
        insert_initial_balance_sqlite(&transaction, identity, initial_balance)?;
        let occupied = occupied_identity_ids_sqlite(&transaction)?;
        if initial_state.revision != 0 || initial_state.value != encode_seat_registry(&occupied) {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                IdentityInitializeOutcome::SecurityStateChanged,
            ));
        }
        transaction.execute(
            "INSERT INTO security_state(key,value,revision,mac,updated_at) VALUES(?,?,?,?,?)",
            params![
                initial_state.key,
                initial_state.value,
                i64::try_from(initial_state.revision)
                    .map_err(|_| StorageError::DatabaseNotEmpty)?,
                initial_state.mac,
                initial_state.updated_at,
            ],
        )?;
        if let Some((expected_sequence, expected_hmac, events)) = audit
            && !append_audit_events_sqlite(&transaction, expected_sequence, expected_hmac, events)?
        {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(MutationWithAuditOutcome::Mutation(
            IdentityInitializeOutcome::Created,
        ))
    }

    pub fn identity_by_email(&self, email: &str) -> Result<Option<IdentityRecord>, StorageError> {
        self.connection
            .query_row(
                "SELECT id,email,display_name,password_hash,role,status,can_consume_model,
                        password_change_required,revision,integrity_hmac,created_at,updated_at
                 FROM identities WHERE email=? AND status!='deleted'",
                [email],
                identity_from_sqlite_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn identity_by_id(&self, id: &str) -> Result<Option<IdentityRecord>, StorageError> {
        self.connection
            .query_row(
                "SELECT id,email,display_name,password_hash,role,status,can_consume_model,
                        password_change_required,revision,integrity_hmac,created_at,updated_at
                 FROM identities WHERE id=?",
                [id],
                identity_from_sqlite_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn list_member_identities(&self) -> Result<Vec<IdentityRecord>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT id,email,display_name,password_hash,role,status,can_consume_model,
                    password_change_required,revision,integrity_hmac,created_at,updated_at
             FROM identities WHERE role='member' AND status!='deleted' ORDER BY created_at,id",
        )?;
        statement
            .query_map([], identity_from_sqlite_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    /// Includes administrators and disabled/deleted identities with unrevoked Keys.
    pub fn list_license_identities(&self) -> Result<Vec<IdentityRecord>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT id,email,display_name,password_hash,role,status,can_consume_model,
                    password_change_required,revision,integrity_hmac,created_at,updated_at
             FROM identities ORDER BY created_at,id",
        )?;
        statement
            .query_map([], identity_from_sqlite_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub fn update_member_identity_status(
        &mut self,
        expected: &IdentityRecord,
        next: &IdentityRecord,
        expected_state: &SecurityStateRecord,
        next_state: &SecurityStateRecord,
    ) -> Result<IdentityStatusUpdateOutcome, StorageError> {
        match self.update_member_identity_status_inner(
            expected,
            next,
            expected_state,
            next_state,
            None,
        )? {
            MutationWithAuditOutcome::Mutation(outcome) => Ok(outcome),
            MutationWithAuditOutcome::AuditConflict => Err(StorageError::DatabaseNotEmpty),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_member_identity_status_and_audit(
        &mut self,
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
    }

    #[allow(clippy::too_many_arguments)]
    fn update_member_identity_status_inner(
        &mut self,
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
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "UPDATE transaction_gates SET revision=revision+1 WHERE gate_key='member_seat'",
            [],
        )?;
        let current_state = transaction
            .query_row(
                "SELECT key,value,revision,mac,updated_at FROM security_state WHERE key=?",
                [&expected_state.key],
                |row| {
                    Ok(SecurityStateRecord {
                        key: row.get(0)?,
                        value: row.get(1)?,
                        revision: row.get::<_, i64>(2)? as u64,
                        mac: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                },
            )
            .optional()?;
        let current_occupied = occupied_identity_ids_sqlite(&transaction)?;
        if current_state.as_ref() != Some(expected_state)
            || expected_state.value != encode_seat_registry(&current_occupied)
        {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                IdentityStatusUpdateOutcome::SecurityStateChanged,
            ));
        }
        let changed = transaction.execute(
            "UPDATE identities SET status=?,revision=?,integrity_hmac=?,updated_at=?
             WHERE id=? AND role='member' AND status=? AND revision=? AND integrity_hmac=?",
            params![
                next.status,
                next.revision,
                next.integrity_hmac,
                next.updated_at,
                expected.id,
                expected.status,
                expected.revision,
                expected.integrity_hmac,
            ],
        )?;
        if changed != 1 {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                IdentityStatusUpdateOutcome::Conflict,
            ));
        }
        if next.status != "active" {
            transaction.execute("DELETE FROM sessions WHERE identity_id=?", [&next.id])?;
        }
        let updated_occupied = occupied_identity_ids_sqlite(&transaction)?;
        if next_state.key != expected_state.key
            || next_state.revision != expected_state.revision.saturating_add(1)
            || next_state.value != encode_seat_registry(&updated_occupied)
        {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                IdentityStatusUpdateOutcome::SecurityStateChanged,
            ));
        }
        let state_changed = transaction.execute(
            "UPDATE security_state SET value=?,revision=?,mac=?,updated_at=?
             WHERE key=? AND value=? AND revision=? AND mac=? AND updated_at=?",
            params![
                next_state.value,
                i64::try_from(next_state.revision).map_err(|_| StorageError::DatabaseNotEmpty)?,
                next_state.mac,
                next_state.updated_at,
                expected_state.key,
                expected_state.value,
                i64::try_from(expected_state.revision)
                    .map_err(|_| StorageError::DatabaseNotEmpty)?,
                expected_state.mac,
                expected_state.updated_at,
            ],
        )?;
        if state_changed != 1 {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                IdentityStatusUpdateOutcome::SecurityStateChanged,
            ));
        }
        if let Some((expected_sequence, expected_hmac, events)) = audit
            && !append_audit_events_sqlite(&transaction, expected_sequence, expected_hmac, events)?
        {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(MutationWithAuditOutcome::Mutation(
            IdentityStatusUpdateOutcome::Updated,
        ))
    }

    pub fn update_identity_password(
        &mut self,
        identity_id: &str,
        expected_revision: u32,
        password_hash: &str,
        password_change_required: bool,
        integrity_hmac: &str,
        updated_at: &str,
    ) -> Result<bool, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE identities
             SET password_hash=?,password_change_required=?,revision=revision+1,
                 integrity_hmac=?,updated_at=?
             WHERE id=? AND revision=? AND status='active'",
            params![
                password_hash,
                u8::from(password_change_required),
                integrity_hmac,
                updated_at,
                identity_id,
                expected_revision,
            ],
        )? == 1;
        if changed {
            transaction.execute("DELETE FROM sessions WHERE identity_id=?", [identity_id])?;
        }
        transaction.commit()?;
        Ok(changed)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_identity_password_and_audit(
        &mut self,
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
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE identities
             SET password_hash=?,password_change_required=?,revision=revision+1,
                 integrity_hmac=?,updated_at=?
             WHERE id=? AND revision=? AND status='active'",
            params![
                password_hash,
                u8::from(password_change_required),
                integrity_hmac,
                updated_at,
                identity_id,
                expected_revision,
            ],
        )? == 1;
        if !changed {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        transaction.execute("DELETE FROM sessions WHERE identity_id=?", [identity_id])?;
        if !append_audit_events_sqlite(
            &transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )? {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(AuditedMutationOutcome::Applied)
    }

    /// Discovery only: callers must authenticate each complete quota snapshot
    /// before deciding whether any reservation may be settled. Read one balance
    /// per identity instead of scanning its historical reservations. Keyset paging
    /// lets a corrupt or busy identity coexist with recovery of later identities.
    pub fn active_quota_identity_page(
        &self,
        after: &str,
        limit: u32,
    ) -> Result<Vec<String>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT identity_id FROM user_balances
             WHERE reserved_tokens>0 AND identity_id>? ORDER BY identity_id ASC LIMIT ?",
        )?;
        Ok(statement
            .query_map(params![after, limit.clamp(1, 128)], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn quota_state_snapshot(
        &self,
        identity_id: &str,
    ) -> Result<Option<QuotaStateSnapshot>, StorageError> {
        let transaction = self.connection.unchecked_transaction()?;
        let Some(balance) = transaction
            .query_row(
                "SELECT identity_id,balance_tokens,reserved_tokens,granted_tokens,
                        consumed_tokens,request_count,raw_tokens,billed_tokens,uncached_input,
                        cached_input,cache_write,output_tokens,revision,last_ledger_hmac,
                        integrity_hmac,updated_at
                 FROM user_balances WHERE identity_id=?",
                [identity_id],
                balance_from_sqlite_row,
            )
            .optional()?
        else {
            return Ok(None);
        };
        let mut statement = transaction.prepare(
            "SELECT id,identity_id,api_key_id,request_id,reserved_tokens,status,revision,
                    integrity_hmac,created_at,expires_at,settled_at
             FROM quota_reservations
             WHERE identity_id=? AND status='active'
             ORDER BY id ASC",
        )?;
        let active_reservations = statement
            .query_map([identity_id], quota_reservation_from_sqlite_row)?
            .collect::<Result<Vec<_>, _>>()?;
        let mut ledger_statement = transaction.prepare(
            "SELECT id,identity_id,kind,amount_tokens,uncached_input,cached_input,cache_write,
                    output_tokens,uncovered_tokens,raw_tokens,billed_tokens,multiplier_micros,
                    reference_id,client_request_id,description,protocol,model,requested_model,processing_tier,
                    reasoning_effort,api_key_id,runner_id,
                    previous_entry_hmac,integrity_hmac,created_at
             FROM ledger_entries WHERE identity_id=? ORDER BY created_at ASC,id ASC",
        )?;
        let ledger_entries = ledger_statement
            .query_map([identity_id], quota_ledger_from_sqlite_row)?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        drop(ledger_statement);
        transaction.commit()?;
        Ok(Some(QuotaStateSnapshot {
            balance,
            active_reservations,
            ledger_entries,
        }))
    }

    pub fn reserve_quota(
        &mut self,
        subject: &AuthorizedApiKey,
        expected_balance: &UserBalanceRecord,
        next_balance: &UserBalanceRecord,
        reservation: &QuotaReservationRecord,
    ) -> Result<QuotaMutationOutcome, StorageError> {
        if !valid_reserve_transition(expected_balance, next_balance, reservation) {
            return Ok(QuotaMutationOutcome::Conflict);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current_subject = transaction
            .query_row(
                "SELECT k.id,k.identity_id,k.name,k.key_hash,k.key_prefix,k.status,k.revision,
                        k.integrity_hmac,k.created_at,k.last_used_at,
                        i.id,i.email,i.display_name,i.password_hash,i.role,i.status,
                        i.can_consume_model,i.password_change_required,i.revision,i.integrity_hmac,
                        i.created_at,i.updated_at
                 FROM api_keys k JOIN identities i ON i.id=k.identity_id
                 WHERE k.key_hash=? AND k.status='active' AND i.status='active'",
                [&subject.api_key.key_hash],
                authorized_api_key_from_sqlite_row,
            )
            .optional()?;
        if !valid_reservation_subject(current_subject.as_ref(), subject, reservation) {
            transaction.rollback()?;
            return Ok(QuotaMutationOutcome::Conflict);
        }
        let current = transaction
            .query_row(
                "SELECT identity_id,balance_tokens,reserved_tokens,granted_tokens,
                        consumed_tokens,request_count,raw_tokens,billed_tokens,uncached_input,
                        cached_input,cache_write,output_tokens,revision,last_ledger_hmac,
                        integrity_hmac,updated_at
                 FROM user_balances WHERE identity_id=?",
                [&expected_balance.identity_id],
                balance_from_sqlite_row,
            )
            .optional()?;
        if current.as_ref() != Some(expected_balance) {
            transaction.rollback()?;
            return Ok(QuotaMutationOutcome::Conflict);
        }
        if transaction
            .query_row(
                "SELECT 1 FROM quota_reservations WHERE request_id=?",
                [&reservation.request_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        {
            transaction.rollback()?;
            return Ok(QuotaMutationOutcome::DuplicateRequest);
        }
        if expected_balance
            .balance_tokens
            .checked_sub(expected_balance.reserved_tokens)
            .is_none_or(|available| available < reservation.reserved_tokens)
        {
            transaction.rollback()?;
            return Ok(QuotaMutationOutcome::Insufficient);
        }
        transaction.execute(
            "INSERT INTO quota_reservations(
               id,identity_id,api_key_id,request_id,reserved_tokens,status,revision,
               integrity_hmac,created_at,expires_at,settled_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
            params![
                reservation.id,
                reservation.identity_id,
                reservation.api_key_id,
                reservation.request_id,
                reservation.reserved_tokens,
                reservation.status,
                i64::try_from(reservation.revision).map_err(|_| StorageError::DatabaseNotEmpty)?,
                reservation.integrity_hmac,
                reservation.created_at,
                reservation.expires_at,
                reservation.settled_at,
            ],
        )?;
        let changed = update_balance_sqlite(&transaction, expected_balance, next_balance)?;
        if !changed {
            transaction.rollback()?;
            return Ok(QuotaMutationOutcome::Conflict);
        }
        transaction.commit()?;
        Ok(QuotaMutationOutcome::Applied)
    }

    pub fn grant_quota(
        &mut self,
        expected_balance: &UserBalanceRecord,
        next_balance: &UserBalanceRecord,
        ledger: &QuotaLedgerEntry,
    ) -> Result<QuotaMutationOutcome, StorageError> {
        match self.grant_quota_inner(expected_balance, next_balance, ledger, None)? {
            MutationWithAuditOutcome::Mutation(outcome) => Ok(outcome),
            MutationWithAuditOutcome::AuditConflict => Err(StorageError::DatabaseNotEmpty),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn grant_quota_and_audit(
        &mut self,
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
    }

    pub fn grant_quota_batch_and_audit(
        &mut self,
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
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(write) = token {
            let current = transaction
                .query_row(
                    "SELECT identity_id,balance_tokens,reserved_tokens,granted_tokens,
                        consumed_tokens,request_count,raw_tokens,billed_tokens,uncached_input,
                        cached_input,cache_write,output_tokens,revision,last_ledger_hmac,
                        integrity_hmac,updated_at FROM user_balances WHERE identity_id=?",
                    [&write.expected_balance.identity_id],
                    balance_from_sqlite_row,
                )
                .optional()?;
            if current.as_ref() != Some(&write.expected_balance) {
                return Ok(MutationWithAuditOutcome::Mutation(
                    QuotaMutationOutcome::Conflict,
                ));
            }
            if let Err(error) = insert_ledger_sqlite(&transaction, &write.ledger) {
                if is_sqlite_unique_violation(&error) {
                    return Ok(MutationWithAuditOutcome::Mutation(
                        QuotaMutationOutcome::DuplicateRequest,
                    ));
                }
                return Err(StorageError::Database(error));
            }
            if !update_balance_sqlite(&transaction, &write.expected_balance, &write.next_balance)? {
                return Ok(MutationWithAuditOutcome::Mutation(
                    QuotaMutationOutcome::Conflict,
                ));
            }
        }
        for write in images {
            let duplicate: Option<String> = transaction
                .query_row(
                    "SELECT id FROM image_quota_ledger WHERE id=?",
                    [&write.ledger.id],
                    |row| row.get(0),
                )
                .optional()?;
            if duplicate.is_some() {
                return Ok(MutationWithAuditOutcome::Mutation(
                    QuotaMutationOutcome::DuplicateRequest,
                ));
            }
            let unit: Option<String> = transaction
                .query_row(
                    "SELECT u.quota_unit FROM model_quota_units u JOIN models m ON m.id=u.model_id
                 WHERE u.model_id=? AND m.enabled=1",
                    [&write.next_balance.public_model_id],
                    |row| row.get(0),
                )
                .optional()?;
            if unit.as_deref() != Some("image") {
                return Ok(MutationWithAuditOutcome::Mutation(
                    QuotaMutationOutcome::Conflict,
                ));
            }
            let current: Option<(i64, String)> = transaction.query_row(
                "SELECT revision,integrity_hmac FROM member_image_balances WHERE identity_id=? AND public_model_id=?",
                params![write.next_balance.identity_id, write.next_balance.public_model_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).optional()?;
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
                let changed = transaction.execute(
                    "UPDATE member_image_balances SET available_images=?,reserved_images=?,consumed_images=?,
                     revision=?,last_ledger_hmac=?,integrity_hmac=? WHERE identity_id=? AND public_model_id=?
                     AND revision=? AND integrity_hmac=?",
                    params![write.next_balance.available_images,write.next_balance.reserved_images,
                        write.next_balance.consumed_images,write.next_balance.revision,
                        write.next_balance.last_ledger_hmac,write.next_balance.integrity_hmac,
                        write.next_balance.identity_id,write.next_balance.public_model_id,
                        expected.revision,expected.integrity_hmac],
                )?;
                if changed != 1 {
                    return Ok(MutationWithAuditOutcome::Mutation(
                        QuotaMutationOutcome::Conflict,
                    ));
                }
            } else {
                transaction.execute(
                    "INSERT INTO member_image_balances(identity_id,public_model_id,available_images,
                     reserved_images,consumed_images,revision,last_ledger_hmac,integrity_hmac)
                     VALUES(?,?,?,?,?,?,?,?)",
                    params![write.next_balance.identity_id,write.next_balance.public_model_id,
                        write.next_balance.available_images,write.next_balance.reserved_images,
                        write.next_balance.consumed_images,write.next_balance.revision,
                        write.next_balance.last_ledger_hmac,write.next_balance.integrity_hmac],
                )?;
            }
            transaction.execute(
                "INSERT INTO image_quota_ledger(id,identity_id,public_model_id,reservation_id,child_id,
                 kind,amount_images,produced_images,delivery_state,actor,reason,previous_hmac,
                 integrity_hmac,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                params![write.ledger.id,write.ledger.identity_id,write.ledger.public_model_id,
                    write.ledger.reservation_id,write.ledger.child_id,write.ledger.kind,
                    write.ledger.amount_images,write.ledger.produced_images,write.ledger.delivery_state,
                    write.ledger.actor,write.ledger.reason,write.ledger.previous_hmac,
                    write.ledger.integrity_hmac,write.ledger.created_at],
            )?;
        }
        if !append_audit_events_sqlite(
            &transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )? {
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(MutationWithAuditOutcome::Mutation(
            QuotaMutationOutcome::Applied,
        ))
    }

    fn grant_quota_inner(
        &mut self,
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
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = transaction
            .query_row(
                "SELECT identity_id,balance_tokens,reserved_tokens,granted_tokens,
                        consumed_tokens,request_count,raw_tokens,billed_tokens,uncached_input,
                        cached_input,cache_write,output_tokens,revision,last_ledger_hmac,
                        integrity_hmac,updated_at
                 FROM user_balances WHERE identity_id=?",
                [&expected_balance.identity_id],
                balance_from_sqlite_row,
            )
            .optional()?;
        if current.as_ref() != Some(expected_balance) {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                QuotaMutationOutcome::Conflict,
            ));
        }
        let inserted = insert_ledger_sqlite(&transaction, ledger);
        if let Err(error) = inserted {
            if is_sqlite_unique_violation(&error) {
                transaction.rollback()?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    QuotaMutationOutcome::DuplicateRequest,
                ));
            }
            return Err(StorageError::Database(error));
        }
        if !update_balance_sqlite(&transaction, expected_balance, next_balance)? {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                QuotaMutationOutcome::Conflict,
            ));
        }
        if let Some((expected_sequence, expected_hmac, events)) = audit
            && !append_audit_events_sqlite(&transaction, expected_sequence, expected_hmac, events)?
        {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(MutationWithAuditOutcome::Mutation(
            QuotaMutationOutcome::Applied,
        ))
    }

    pub fn insert_quota_request(
        &self,
        request: &QuotaRequestRecord,
    ) -> Result<QuotaRequestInsertOutcome, StorageError> {
        let result = self.connection.execute(
            "INSERT INTO quota_requests(
               id,identity_id,amount_nanos,reason,status,review_note,reviewed_by,reviewed_at,
               revision,integrity_hmac,created_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
            params![
                request.id,
                request.identity_id,
                request.amount_nanos,
                request.reason,
                request.status,
                request.review_note,
                request.reviewed_by,
                request.reviewed_at,
                request.revision,
                request.integrity_hmac,
                request.created_at,
            ],
        );
        match result {
            Ok(_) => Ok(QuotaRequestInsertOutcome::Inserted),
            Err(error) if is_sqlite_unique_violation(&error) => {
                Ok(QuotaRequestInsertOutcome::PendingExists)
            }
            Err(error) => Err(StorageError::Database(error)),
        }
    }

    pub fn insert_quota_request_and_audit(
        &mut self,
        request: &QuotaRequestRecord,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<MutationWithAuditOutcome<QuotaRequestInsertOutcome>, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let inserted = transaction.execute(
            "INSERT INTO quota_requests(
               id,identity_id,amount_nanos,reason,status,review_note,reviewed_by,reviewed_at,
               revision,integrity_hmac,created_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
            params![
                request.id,
                request.identity_id,
                request.amount_nanos,
                request.reason,
                request.status,
                request.review_note,
                request.reviewed_by,
                request.reviewed_at,
                request.revision,
                request.integrity_hmac,
                request.created_at,
            ],
        );
        match inserted {
            Ok(1) => {}
            Ok(_) => {
                transaction.rollback()?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    QuotaRequestInsertOutcome::PendingExists,
                ));
            }
            Err(error) if is_sqlite_unique_violation(&error) => {
                transaction.rollback()?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    QuotaRequestInsertOutcome::PendingExists,
                ));
            }
            Err(error) => return Err(StorageError::Database(error)),
        }
        if !append_audit_events_sqlite(
            &transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )? {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(MutationWithAuditOutcome::Mutation(
            QuotaRequestInsertOutcome::Inserted,
        ))
    }

    pub fn quota_request_by_id(
        &self,
        request_id: &str,
    ) -> Result<Option<QuotaRequestRecord>, StorageError> {
        self.connection
            .query_row(
                "SELECT id,identity_id,amount_nanos,reason,status,review_note,
                        reviewed_by,reviewed_at,revision,integrity_hmac,created_at
                 FROM quota_requests WHERE id=?",
                [request_id],
                quota_request_from_sqlite_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn list_quota_requests(
        &self,
        identity_id: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<QuotaRequestRecord>, StorageError> {
        let sql = match (identity_id.is_some(), status.is_some()) {
            (true, true) => {
                "SELECT id,identity_id,amount_nanos,reason,status,review_note,
                        reviewed_by,reviewed_at,revision,integrity_hmac,created_at
                 FROM quota_requests WHERE identity_id=? AND status=? ORDER BY created_at DESC,id"
            }
            (true, false) => {
                "SELECT id,identity_id,amount_nanos,reason,status,review_note,
                        reviewed_by,reviewed_at,revision,integrity_hmac,created_at
                 FROM quota_requests WHERE identity_id=? ORDER BY created_at DESC,id"
            }
            (false, true) => {
                "SELECT id,identity_id,amount_nanos,reason,status,review_note,
                        reviewed_by,reviewed_at,revision,integrity_hmac,created_at
                 FROM quota_requests WHERE status=? ORDER BY created_at DESC,id"
            }
            (false, false) => {
                "SELECT id,identity_id,amount_nanos,reason,status,review_note,
                        reviewed_by,reviewed_at,revision,integrity_hmac,created_at
                 FROM quota_requests ORDER BY created_at DESC,id"
            }
        };
        let mut statement = self.connection.prepare(sql)?;
        let collect = |rows: rusqlite::MappedRows<'_, _>| {
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::from)
        };
        match (identity_id, status) {
            (Some(identity_id), Some(status)) => collect(
                statement.query_map(params![identity_id, status], quota_request_from_sqlite_row)?,
            ),
            (Some(identity_id), None) => {
                collect(statement.query_map([identity_id], quota_request_from_sqlite_row)?)
            }
            (None, Some(status)) => {
                collect(statement.query_map([status], quota_request_from_sqlite_row)?)
            }
            (None, None) => collect(statement.query_map([], quota_request_from_sqlite_row)?),
        }
    }

    pub fn change_pending_quota_request_and_audit(
        &mut self,
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
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = if let Some(next) = next_request {
            transaction.execute(
                "UPDATE quota_requests
                 SET amount_nanos=?,reason=?,revision=?,integrity_hmac=?
                 WHERE id=? AND identity_id=? AND status='pending' AND revision=? AND integrity_hmac=?",
                params![
                    next.amount_nanos,
                    next.reason,
                    next.revision,
                    next.integrity_hmac,
                    expected_request.id,
                    expected_request.identity_id,
                    expected_request.revision,
                    expected_request.integrity_hmac,
                ],
            )?
        } else {
            transaction.execute(
                "DELETE FROM quota_requests
                 WHERE id=? AND identity_id=? AND status='pending' AND revision=? AND integrity_hmac=?",
                params![
                    expected_request.id,
                    expected_request.identity_id,
                    expected_request.revision,
                    expected_request.integrity_hmac,
                ],
            )?
        };
        if changed != 1 {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        if !append_audit_events_sqlite(
            &transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )? {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(AuditedMutationOutcome::Applied)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn review_quota_request(
        &mut self,
        expected_request: &QuotaRequestRecord,
        next_status: &str,
        review_note: &str,
        reviewed_by: &str,
        reviewed_at: &str,
        next_integrity_hmac: &str,
        grant: Option<(&MoneyBalanceRecord, &MoneyBalanceRecord, &MoneyLedgerEntry)>,
    ) -> Result<QuotaRequestReviewOutcome, StorageError> {
        match self.review_quota_request_inner(
            expected_request,
            next_status,
            review_note,
            reviewed_by,
            reviewed_at,
            next_integrity_hmac,
            grant,
            None,
        )? {
            MutationWithAuditOutcome::Mutation(outcome) => Ok(outcome),
            MutationWithAuditOutcome::AuditConflict => Err(StorageError::DatabaseNotEmpty),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn review_quota_request_and_audit(
        &mut self,
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
    }

    #[allow(clippy::too_many_arguments)]
    fn review_quota_request_inner(
        &mut self,
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
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = transaction
            .query_row(
                "SELECT id,identity_id,amount_nanos,reason,status,review_note,
                        reviewed_by,reviewed_at,revision,integrity_hmac,created_at
                 FROM quota_requests WHERE id=?",
                [&expected_request.id],
                quota_request_from_sqlite_row,
            )
            .optional()?;
        if current.as_ref() != Some(expected_request) {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                QuotaRequestReviewOutcome::Conflict,
            ));
        }
        if let Some((expected_balance, next_balance, ledger)) = grant {
            let inserted = transaction.execute(
                "INSERT INTO money_ledger_entries(
                   id,identity_id,currency,kind,amount_nanos,balance_revision,reference_id,billing_status,
                   details_json,previous_entry_hmac,integrity_hmac,created_at
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    ledger.id, ledger.identity_id, ledger.currency, ledger.kind,
                    ledger.amount_nanos, ledger.balance_revision, ledger.reference_id,
                    ledger.billing_status, ledger.details_json, ledger.previous_entry_hmac,
                    ledger.integrity_hmac, ledger.created_at,
                ],
            );
            if let Err(error) = inserted {
                if is_sqlite_unique_violation(&error) {
                    transaction.rollback()?;
                    return Ok(MutationWithAuditOutcome::Mutation(
                        QuotaRequestReviewOutcome::Conflict,
                    ));
                }
                return Err(StorageError::Database(error));
            }
            if transaction.execute(
                "UPDATE money_balances SET balance_nanos=?,credited_nanos=?,debited_nanos=?,
                   revision=?,last_ledger_hmac=?,integrity_hmac=?,updated_at=?
                 WHERE identity_id=? AND revision=? AND integrity_hmac=?",
                params![
                    next_balance.balance_nanos,
                    next_balance.credited_nanos,
                    next_balance.debited_nanos,
                    next_balance.revision,
                    next_balance.last_ledger_hmac,
                    next_balance.integrity_hmac,
                    next_balance.updated_at,
                    expected_balance.identity_id,
                    expected_balance.revision,
                    expected_balance.integrity_hmac,
                ],
            )? != 1
            {
                transaction.rollback()?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    QuotaRequestReviewOutcome::Conflict,
                ));
            }
        }
        if transaction.execute(
            "UPDATE quota_requests
             SET status=?,review_note=?,reviewed_by=?,reviewed_at=?,revision=?,integrity_hmac=?
             WHERE id=? AND status='pending' AND revision=? AND integrity_hmac=?",
            params![
                next_status,
                review_note,
                reviewed_by,
                reviewed_at,
                expected_request.revision.saturating_add(1),
                next_integrity_hmac,
                expected_request.id,
                expected_request.revision,
                expected_request.integrity_hmac,
            ],
        )? != 1
        {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                QuotaRequestReviewOutcome::Conflict,
            ));
        }
        if let Some((expected_sequence, expected_hmac, events)) = audit
            && !append_audit_events_sqlite(&transaction, expected_sequence, expected_hmac, events)?
        {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(MutationWithAuditOutcome::Mutation(
            QuotaRequestReviewOutcome::Applied,
        ))
    }

    pub fn insert_vouchers(
        &mut self,
        records: &[(VoucherRecord, Option<VoucherDeliveryRecord>)],
    ) -> Result<(), StorageError> {
        match self.insert_vouchers_inner(records, None)? {
            AuditedMutationOutcome::Applied => Ok(()),
            AuditedMutationOutcome::MutationConflict | AuditedMutationOutcome::AuditConflict => {
                Err(StorageError::DatabaseNotEmpty)
            }
        }
    }

    pub fn insert_vouchers_and_audit(
        &mut self,
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
    }

    fn insert_vouchers_inner(
        &mut self,
        records: &[(VoucherRecord, Option<VoucherDeliveryRecord>)],
        audit: Option<(u64, &str, &[AuditEventRecord])>,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (voucher, delivery) in records {
            transaction.execute(
                "INSERT INTO vouchers(
                   id,code_hash,code_prefix,name,quota_tokens,status,max_redemptions,
                   redeemed_count,expires_at,created_by,revision,integrity_hmac,created_at
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    voucher.id,
                    voucher.code_hash,
                    voucher.code_prefix,
                    voucher.name,
                    voucher.quota_tokens,
                    voucher.status,
                    voucher.max_redemptions,
                    voucher.redeemed_count,
                    voucher.expires_at,
                    voucher.created_by,
                    voucher.revision,
                    voucher.integrity_hmac,
                    voucher.created_at,
                ],
            )?;
            if let Some(delivery) = delivery {
                transaction.execute(
                    "INSERT INTO voucher_deliveries(
                       id,voucher_id,identity_id,status,delivered_at,redeemed_at
                     ) VALUES(?,?,?,?,?,?)",
                    params![
                        delivery.id,
                        delivery.voucher_id,
                        delivery.identity_id,
                        delivery.status,
                        delivery.delivered_at,
                        delivery.redeemed_at,
                    ],
                )?;
            }
        }
        if let Some((expected_sequence, expected_hmac, events)) = audit
            && !append_audit_events_sqlite(&transaction, expected_sequence, expected_hmac, events)?
        {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(AuditedMutationOutcome::Applied)
    }

    pub fn list_vouchers(&self) -> Result<Vec<VoucherRecord>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT id,code_hash,code_prefix,name,quota_tokens,status,max_redemptions,
                    redeemed_count,expires_at,created_by,revision,integrity_hmac,created_at
             FROM vouchers ORDER BY created_at DESC,id",
        )?;
        statement
            .query_map([], voucher_from_sqlite_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub fn voucher_by_code_hash(
        &self,
        code_hash: &str,
    ) -> Result<Option<VoucherRecord>, StorageError> {
        self.connection
            .query_row(
                "SELECT id,code_hash,code_prefix,name,quota_tokens,status,max_redemptions,
                        redeemed_count,expires_at,created_by,revision,integrity_hmac,created_at
                 FROM vouchers WHERE code_hash=?",
                [code_hash],
                voucher_from_sqlite_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn voucher_delivery_for_identity(
        &self,
        delivery_id: &str,
        identity_id: &str,
    ) -> Result<Option<(VoucherDeliveryRecord, VoucherRecord)>, StorageError> {
        self.connection
            .query_row(
                "SELECT d.id,d.voucher_id,d.identity_id,d.status,d.delivered_at,d.redeemed_at,
                        v.id,v.code_hash,v.code_prefix,v.name,v.quota_tokens,v.status,
                        v.max_redemptions,v.redeemed_count,v.expires_at,v.created_by,
                        v.revision,v.integrity_hmac,v.created_at
                 FROM voucher_deliveries d JOIN vouchers v ON v.id=d.voucher_id
                 WHERE d.id=? AND d.identity_id=?",
                params![delivery_id, identity_id],
                voucher_delivery_pair_from_sqlite_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn list_voucher_deliveries_for_identity(
        &self,
        identity_id: &str,
    ) -> Result<Vec<(VoucherDeliveryRecord, VoucherRecord)>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT d.id,d.voucher_id,d.identity_id,d.status,d.delivered_at,d.redeemed_at,
                    v.id,v.code_hash,v.code_prefix,v.name,v.quota_tokens,v.status,
                    v.max_redemptions,v.redeemed_count,v.expires_at,v.created_by,
                    v.revision,v.integrity_hmac,v.created_at
             FROM voucher_deliveries d JOIN vouchers v ON v.id=d.voucher_id
             WHERE d.identity_id=? ORDER BY d.delivered_at DESC,d.id",
        )?;
        statement
            .query_map([identity_id], voucher_delivery_pair_from_sqlite_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub fn voucher_deliveries(
        &self,
        voucher_id: &str,
    ) -> Result<Vec<VoucherDeliveryRecord>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT id,voucher_id,identity_id,status,delivered_at,redeemed_at
             FROM voucher_deliveries WHERE voucher_id=? ORDER BY delivered_at,id",
        )?;
        statement
            .query_map([voucher_id], voucher_delivery_from_sqlite_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub fn voucher_redemptions(
        &self,
        voucher_id: &str,
    ) -> Result<Vec<VoucherRedemptionRecord>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT id,voucher_id,identity_id,amount_tokens,ledger_entry_id,created_at
             FROM voucher_redemptions WHERE voucher_id=? ORDER BY created_at,id",
        )?;
        statement
            .query_map([voucher_id], voucher_redemption_from_sqlite_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub fn delete_unused_voucher(&self, voucher_id: &str) -> Result<bool, StorageError> {
        Ok(self.connection.execute(
            "DELETE FROM vouchers WHERE id=? AND redeemed_count=0",
            [voucher_id],
        )? == 1)
    }

    pub fn delete_unused_voucher_and_audit(
        &mut self,
        voucher_id: &str,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if transaction.execute(
            "DELETE FROM vouchers WHERE id=? AND redeemed_count=0",
            [voucher_id],
        )? != 1
        {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        if !append_audit_events_sqlite(
            &transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )? {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(AuditedMutationOutcome::Applied)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn redeem_voucher(
        &mut self,
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
        match self.redeem_voucher_inner(
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
        )? {
            MutationWithAuditOutcome::Mutation(outcome) => Ok(outcome),
            MutationWithAuditOutcome::AuditConflict => Err(StorageError::DatabaseNotEmpty),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn redeem_voucher_and_audit(
        &mut self,
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
    }

    #[allow(clippy::too_many_arguments)]
    fn redeem_voucher_inner(
        &mut self,
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
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current_voucher = transaction
            .query_row(
                "SELECT id,code_hash,code_prefix,name,quota_tokens,status,max_redemptions,
                        redeemed_count,expires_at,created_by,revision,integrity_hmac,created_at
                 FROM vouchers WHERE id=?",
                [&expected_voucher.id],
                voucher_from_sqlite_row,
            )
            .optional()?;
        if current_voucher.as_ref() != Some(expected_voucher) {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                VoucherRedeemOutcome::Conflict,
            ));
        }
        if transaction
            .query_row(
                "SELECT 1 FROM voucher_redemptions WHERE voucher_id=? AND identity_id=?",
                params![expected_voucher.id, identity_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        {
            transaction.rollback()?;
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
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                VoucherRedeemOutcome::Unavailable,
            ));
        }
        let delivery_count: u32 = transaction.query_row(
            "SELECT COUNT(*) FROM voucher_deliveries WHERE voucher_id=?",
            [&expected_voucher.id],
            |row| row.get(0),
        )?;
        if let Some(expected_delivery) = expected_delivery {
            let current_delivery = transaction
                .query_row(
                    "SELECT id,voucher_id,identity_id,status,delivered_at,redeemed_at
                     FROM voucher_deliveries WHERE id=? AND identity_id=?",
                    params![expected_delivery.id, identity_id],
                    voucher_delivery_from_sqlite_row,
                )
                .optional()?;
            if current_delivery.as_ref() != Some(expected_delivery)
                || expected_delivery.voucher_id != expected_voucher.id
                || expected_delivery.status != "pending"
            {
                transaction.rollback()?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    VoucherRedeemOutcome::Unavailable,
                ));
            }
        } else if delivery_count != 0 {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                VoucherRedeemOutcome::Unavailable,
            ));
        }
        insert_ledger_sqlite(&transaction, ledger)?;
        if !update_balance_sqlite(&transaction, expected_balance, next_balance)? {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                VoucherRedeemOutcome::Conflict,
            ));
        }
        transaction.execute(
            "INSERT INTO voucher_redemptions(
               id,voucher_id,identity_id,amount_tokens,ledger_entry_id,created_at
             ) VALUES(?,?,?,?,?,?)",
            params![
                redemption.id,
                redemption.voucher_id,
                redemption.identity_id,
                redemption.amount_tokens,
                redemption.ledger_entry_id,
                redemption.created_at,
            ],
        )?;
        let next_redeemed_count = expected_voucher.redeemed_count.saturating_add(1);
        let next_status = if next_redeemed_count >= expected_voucher.max_redemptions {
            "redeemed"
        } else {
            "active"
        };
        if transaction.execute(
            "UPDATE vouchers SET redeemed_count=?,status=?,revision=?,integrity_hmac=?
             WHERE id=? AND status='active' AND redeemed_count=? AND revision=? AND integrity_hmac=?",
            params![
                next_redeemed_count,
                next_status,
                expected_voucher.revision.saturating_add(1),
                next_voucher_integrity_hmac,
                expected_voucher.id,
                expected_voucher.redeemed_count,
                expected_voucher.revision,
                expected_voucher.integrity_hmac,
            ],
        )? != 1
        {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                VoucherRedeemOutcome::Conflict,
            ));
        }
        if let Some(delivery) = expected_delivery
            && transaction.execute(
                "UPDATE voucher_deliveries SET status='redeemed',redeemed_at=?
                 WHERE id=? AND identity_id=? AND status='pending'",
                params![now, delivery.id, identity_id],
            )? != 1
        {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                VoucherRedeemOutcome::Conflict,
            ));
        }
        if let Some((expected_sequence, expected_hmac, event)) = audit
            && !append_audit_events_sqlite(
                &transaction,
                expected_sequence,
                expected_hmac,
                std::slice::from_ref(event),
            )?
        {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(MutationWithAuditOutcome::Mutation(
            VoucherRedeemOutcome::Applied,
        ))
    }

    pub fn runtime_setting(&self, key: &str) -> Result<Option<RuntimeSettingRecord>, StorageError> {
        self.connection
            .query_row(
                "SELECT key,value,revision,integrity_hmac,updated_at
                 FROM runtime_settings WHERE key=?",
                [key],
                runtime_setting_from_sqlite_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn write_runtime_setting(
        &mut self,
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
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = if let Some(expected) = expected {
            transaction.execute(
                "UPDATE runtime_settings
                 SET value=?,revision=?,integrity_hmac=?,updated_at=?
                 WHERE key=? AND revision=? AND integrity_hmac=?",
                params![
                    next.value,
                    i64::try_from(next.revision).map_err(|_| StorageError::DatabaseNotEmpty)?,
                    next.integrity_hmac,
                    next.updated_at,
                    expected.key,
                    i64::try_from(expected.revision).map_err(|_| StorageError::DatabaseNotEmpty)?,
                    expected.integrity_hmac,
                ],
            )? == 1
        } else {
            let result = transaction.execute(
                "INSERT INTO runtime_settings(key,value,revision,integrity_hmac,updated_at)
                 VALUES(?,?,?,?,?)",
                params![
                    next.key,
                    next.value,
                    i64::try_from(next.revision).map_err(|_| StorageError::DatabaseNotEmpty)?,
                    next.integrity_hmac,
                    next.updated_at,
                ],
            );
            match result {
                Ok(changed) => changed == 1,
                Err(error) if is_sqlite_unique_violation(&error) => false,
                Err(error) => return Err(StorageError::Database(error)),
            }
        };
        if !changed {
            transaction.rollback()?;
            return Ok(RuntimeSettingWriteOutcome::Conflict);
        }
        transaction.commit()?;
        Ok(RuntimeSettingWriteOutcome::Applied)
    }

    pub fn write_runtime_setting_with_audit(
        &mut self,
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
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = if let Some(expected) = expected {
            transaction.execute(
                "UPDATE runtime_settings
                 SET value=?,revision=?,integrity_hmac=?,updated_at=?
                 WHERE key=? AND revision=? AND integrity_hmac=?",
                params![
                    next.value,
                    i64::try_from(next.revision).map_err(|_| StorageError::DatabaseNotEmpty)?,
                    next.integrity_hmac,
                    next.updated_at,
                    expected.key,
                    i64::try_from(expected.revision).map_err(|_| StorageError::DatabaseNotEmpty)?,
                    expected.integrity_hmac,
                ],
            )? == 1
        } else {
            match transaction.execute(
                "INSERT INTO runtime_settings(key,value,revision,integrity_hmac,updated_at)
                 VALUES(?,?,?,?,?)",
                params![
                    next.key,
                    next.value,
                    i64::try_from(next.revision).map_err(|_| StorageError::DatabaseNotEmpty)?,
                    next.integrity_hmac,
                    next.updated_at,
                ],
            ) {
                Ok(changed) => changed == 1,
                Err(error) if is_sqlite_unique_violation(&error) => false,
                Err(error) => return Err(StorageError::Database(error)),
            }
        };
        if !changed {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        if transaction.execute(
            "UPDATE transaction_gates SET revision=revision+1 WHERE gate_key='audit_log'",
            [],
        )? != 1
        {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        let current = transaction
            .query_row(
                "SELECT event_sequence,integrity_hmac
                 FROM audit_events ORDER BY event_sequence DESC LIMIT 1",
                [],
                |row| Ok((row.get::<_, i64>(0)? as u64, row.get::<_, String>(1)?)),
            )
            .optional()?
            .unwrap_or((0, String::new()));
        if current.0 != expected_audit_sequence || current.1 != expected_audit_hmac {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        let inserted = transaction.execute(
            "INSERT INTO audit_events(
               id,event_sequence,actor_identity_id,actor_role,action,target_type,target_id,
               outcome,previous_event_hmac,integrity_hmac,created_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
            params![
                audit_event.id,
                i64::try_from(audit_event.sequence).map_err(|_| StorageError::DatabaseNotEmpty)?,
                audit_event.actor_identity_id,
                audit_event.actor_role,
                audit_event.action,
                audit_event.target_type,
                audit_event.target_id,
                audit_event.outcome,
                audit_event.previous_event_hmac,
                audit_event.integrity_hmac,
                audit_event.created_at,
            ],
        );
        match inserted {
            Ok(1) => {
                transaction.commit()?;
                Ok(AuditedMutationOutcome::Applied)
            }
            Ok(_) => {
                transaction.rollback()?;
                Ok(AuditedMutationOutcome::AuditConflict)
            }
            Err(error) if is_sqlite_unique_violation(&error) => {
                transaction.rollback()?;
                Ok(AuditedMutationOutcome::AuditConflict)
            }
            Err(error) => Err(StorageError::Database(error)),
        }
    }

    pub fn audit_tail(&self) -> Result<Option<AuditEventRecord>, StorageError> {
        self.connection
            .query_row(
                "SELECT id,event_sequence,actor_identity_id,actor_role,action,target_type,target_id,
                        outcome,previous_event_hmac,integrity_hmac,created_at
                 FROM audit_events ORDER BY event_sequence DESC LIMIT 1",
                [],
                audit_event_from_sqlite_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn audit_revision(&self) -> Result<u64, StorageError> {
        let revision = self.connection.query_row(
            "SELECT revision FROM transaction_gates WHERE gate_key='audit_log'",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        u64::try_from(revision).map_err(|_| StorageError::DatabaseNotEmpty)
    }

    pub fn audit_events(&self) -> Result<Vec<AuditEventRecord>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT id,event_sequence,actor_identity_id,actor_role,action,target_type,target_id,
                    outcome,previous_event_hmac,integrity_hmac,created_at
             FROM audit_events ORDER BY event_sequence",
        )?;
        let rows = statement.query_map([], audit_event_from_sqlite_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub fn append_audit_event(
        &mut self,
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
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if transaction.execute(
            "UPDATE transaction_gates SET revision=revision+1 WHERE gate_key='audit_log'",
            [],
        )? != 1
        {
            transaction.rollback()?;
            return Ok(AuditAppendOutcome::Conflict);
        }
        let current = transaction
            .query_row(
                "SELECT event_sequence,integrity_hmac
                 FROM audit_events ORDER BY event_sequence DESC LIMIT 1",
                [],
                |row| Ok((row.get::<_, i64>(0)? as u64, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let current = current.unwrap_or((0, String::new()));
        if current.0 != expected_sequence || current.1 != expected_hmac {
            transaction.rollback()?;
            return Ok(AuditAppendOutcome::Conflict);
        }
        let result = transaction.execute(
            "INSERT INTO audit_events(
               id,event_sequence,actor_identity_id,actor_role,action,target_type,target_id,
               outcome,previous_event_hmac,integrity_hmac,created_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
            params![
                event.id,
                i64::try_from(event.sequence).map_err(|_| StorageError::DatabaseNotEmpty)?,
                event.actor_identity_id,
                event.actor_role,
                event.action,
                event.target_type,
                event.target_id,
                event.outcome,
                event.previous_event_hmac,
                event.integrity_hmac,
                event.created_at,
            ],
        );
        match result {
            Ok(1) => {
                transaction.commit()?;
                Ok(AuditAppendOutcome::Applied)
            }
            Ok(_) => {
                transaction.rollback()?;
                Ok(AuditAppendOutcome::Conflict)
            }
            Err(error) if is_sqlite_unique_violation(&error) => {
                transaction.rollback()?;
                Ok(AuditAppendOutcome::Conflict)
            }
            Err(error) => Err(StorageError::Database(error)),
        }
    }

    pub fn release_quota_reservation(
        &mut self,
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
        self.finish_reservation_sqlite(
            expected_balance,
            next_balance,
            expected_reservation,
            next_reservation,
            None,
        )
    }

    pub fn fail_quota_reservation(
        &mut self,
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
        self.finish_reservation_sqlite(
            expected_balance,
            next_balance,
            expected_reservation,
            next_reservation,
            Some(ledger),
        )
    }

    pub fn settle_quota_reservation(
        &mut self,
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
        self.finish_reservation_sqlite(
            expected_balance,
            next_balance,
            expected_reservation,
            next_reservation,
            Some(ledger),
        )
    }

    fn finish_reservation_sqlite(
        &mut self,
        expected_balance: &UserBalanceRecord,
        next_balance: &UserBalanceRecord,
        expected_reservation: &QuotaReservationRecord,
        next_reservation: &QuotaReservationRecord,
        ledger: Option<&QuotaLedgerEntry>,
    ) -> Result<QuotaMutationOutcome, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current_balance = transaction
            .query_row(
                "SELECT identity_id,balance_tokens,reserved_tokens,granted_tokens,
                        consumed_tokens,request_count,raw_tokens,billed_tokens,uncached_input,
                        cached_input,cache_write,output_tokens,revision,last_ledger_hmac,
                        integrity_hmac,updated_at
                 FROM user_balances WHERE identity_id=?",
                [&expected_balance.identity_id],
                balance_from_sqlite_row,
            )
            .optional()?;
        let current_reservation = transaction
            .query_row(
                "SELECT id,identity_id,api_key_id,request_id,reserved_tokens,status,revision,
                        integrity_hmac,created_at,expires_at,settled_at
                 FROM quota_reservations WHERE id=?",
                [&expected_reservation.id],
                quota_reservation_from_sqlite_row,
            )
            .optional()?;
        if current_balance.as_ref() != Some(expected_balance)
            || current_reservation.as_ref() != Some(expected_reservation)
        {
            transaction.rollback()?;
            return Ok(QuotaMutationOutcome::Conflict);
        }
        if let Some(ledger) = ledger {
            let inserted = insert_ledger_sqlite(&transaction, ledger);
            if let Err(error) = inserted {
                if is_sqlite_unique_violation(&error) {
                    transaction.rollback()?;
                    return Ok(QuotaMutationOutcome::DuplicateRequest);
                }
                return Err(StorageError::Database(error));
            }
        }
        let reservation_changed = transaction.execute(
            "UPDATE quota_reservations
             SET status=?,revision=?,integrity_hmac=?,settled_at=?
             WHERE id=? AND status=? AND revision=? AND integrity_hmac=?",
            params![
                next_reservation.status,
                i64::try_from(next_reservation.revision)
                    .map_err(|_| StorageError::DatabaseNotEmpty)?,
                next_reservation.integrity_hmac,
                next_reservation.settled_at,
                expected_reservation.id,
                expected_reservation.status,
                i64::try_from(expected_reservation.revision)
                    .map_err(|_| StorageError::DatabaseNotEmpty)?,
                expected_reservation.integrity_hmac,
            ],
        )? == 1;
        let balance_changed = update_balance_sqlite(&transaction, expected_balance, next_balance)?;
        if !reservation_changed || !balance_changed {
            transaction.rollback()?;
            return Ok(QuotaMutationOutcome::Conflict);
        }
        transaction.commit()?;
        Ok(QuotaMutationOutcome::Applied)
    }

    pub fn insert_session(&self, session: &SessionRecord) -> Result<(), StorageError> {
        self.connection.execute(
            "INSERT INTO sessions(id,identity_id,token_hash,expires_at,integrity_hmac,created_at)
             VALUES(?,?,?,?,?,?)",
            params![
                session.id,
                session.identity_id,
                session.token_hash,
                session.expires_at,
                session.integrity_hmac,
                session.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn authenticated_session_by_token_hash(
        &self,
        token_hash: &str,
        now: &str,
    ) -> Result<Option<AuthenticatedSession>, StorageError> {
        self.connection
            .query_row(
                "SELECT s.id,s.identity_id,s.token_hash,s.expires_at,s.integrity_hmac,s.created_at,
                        i.id,i.email,i.display_name,i.password_hash,i.role,i.status,
                        i.can_consume_model,i.password_change_required,i.revision,i.integrity_hmac,
                        i.created_at,i.updated_at
                 FROM sessions s JOIN identities i ON i.id=s.identity_id
                 WHERE s.token_hash=? AND s.expires_at>? AND i.status='active'",
                params![token_hash, now],
                authenticated_session_from_sqlite_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn delete_session_by_token_hash(&self, token_hash: &str) -> Result<bool, StorageError> {
        Ok(self
            .connection
            .execute("DELETE FROM sessions WHERE token_hash=?", [token_hash])?
            == 1)
    }

    pub fn insert_api_key_unchecked(&self, api_key: &ApiKeyRecord) -> Result<(), StorageError> {
        self.connection.execute(
            "INSERT INTO api_keys(
               id,identity_id,name,key_hash,key_prefix,status,revision,integrity_hmac,created_at,last_used_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?)",
            params![
                api_key.id,
                api_key.identity_id,
                api_key.name,
                api_key.key_hash,
                api_key.key_prefix,
                api_key.status,
                api_key.revision,
                api_key.integrity_hmac,
                api_key.created_at,
                api_key.last_used_at,
            ],
        )?;
        Ok(())
    }

    pub fn insert_api_key_unchecked_and_audit(
        &mut self,
        api_key: &ApiKeyRecord,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        match self.insert_api_key_with_limit_and_audit(
            api_key,
            None,
            expected_audit_sequence,
            expected_audit_hmac,
            audit_event,
        )? {
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

    pub fn insert_api_key_with_limit_and_audit(
        &mut self,
        api_key: &ApiKeyRecord,
        active_limit: Option<u32>,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<MutationWithAuditOutcome<ResourceCreateOutcome>, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "UPDATE transaction_gates SET revision=revision+1 WHERE gate_key='entity_quota'",
            [],
        )?;
        if let Some(limit) = active_limit {
            let occupied = transaction.query_row(
                "SELECT count(*) FROM api_keys WHERE identity_id=? AND status='active'",
                [&api_key.identity_id],
                |row| row.get::<_, i64>(0),
            )?;
            if occupied >= i64::from(limit) {
                transaction.rollback()?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    ResourceCreateOutcome::LimitReached,
                ));
            }
        }
        let inserted = transaction.execute(
            "INSERT INTO api_keys(
               id,identity_id,name,key_hash,key_prefix,status,revision,integrity_hmac,created_at,last_used_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?)",
            params![
                api_key.id,
                api_key.identity_id,
                api_key.name,
                api_key.key_hash,
                api_key.key_prefix,
                api_key.status,
                api_key.revision,
                api_key.integrity_hmac,
                api_key.created_at,
                api_key.last_used_at,
            ],
        );
        match inserted {
            Ok(1) => {}
            Ok(_) => {
                transaction.rollback()?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    ResourceCreateOutcome::Conflict,
                ));
            }
            Err(error) if is_sqlite_unique_violation(&error) => {
                transaction.rollback()?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    ResourceCreateOutcome::Conflict,
                ));
            }
            Err(error) => return Err(StorageError::Database(error)),
        }
        if !append_audit_events_sqlite(
            &transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )? {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(MutationWithAuditOutcome::Mutation(
            ResourceCreateOutcome::Created,
        ))
    }

    pub fn api_keys_for_identity(
        &self,
        identity_id: &str,
    ) -> Result<Vec<ApiKeyRecord>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT id,identity_id,name,key_hash,key_prefix,status,revision,integrity_hmac,
                    created_at,last_used_at
             FROM api_keys WHERE identity_id=? ORDER BY created_at DESC,id",
        )?;
        let rows = statement.query_map([identity_id], api_key_from_sqlite_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub fn api_key_for_identity(
        &self,
        api_key_id: &str,
        identity_id: &str,
    ) -> Result<Option<ApiKeyRecord>, StorageError> {
        self.connection
            .query_row(
                "SELECT id,identity_id,name,key_hash,key_prefix,status,revision,integrity_hmac,
                        created_at,last_used_at
                 FROM api_keys WHERE id=? AND identity_id=?",
                params![api_key_id, identity_id],
                api_key_from_sqlite_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn authorized_api_key_by_hash(
        &self,
        key_hash: &str,
    ) -> Result<Option<AuthorizedApiKey>, StorageError> {
        self.connection
            .query_row(
                "SELECT k.id,k.identity_id,k.name,k.key_hash,k.key_prefix,k.status,k.revision,
                        k.integrity_hmac,k.created_at,k.last_used_at,
                        i.id,i.email,i.display_name,i.password_hash,i.role,i.status,
                        i.can_consume_model,i.password_change_required,i.revision,i.integrity_hmac,
                        i.created_at,i.updated_at
                 FROM api_keys k JOIN identities i ON i.id=k.identity_id
                 WHERE k.key_hash=? AND k.status='active' AND i.status='active'",
                [key_hash],
                authorized_api_key_from_sqlite_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn update_api_key_status(
        &self,
        api_key_id: &str,
        identity_id: &str,
        expected_revision: u32,
        status: &str,
        integrity_hmac: &str,
    ) -> Result<bool, StorageError> {
        Ok(self.connection.execute(
            "UPDATE api_keys SET status=?,revision=revision+1,integrity_hmac=?
             WHERE id=? AND identity_id=? AND revision=?",
            params![
                status,
                integrity_hmac,
                api_key_id,
                identity_id,
                expected_revision
            ],
        )? == 1)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_api_key_status_and_audit(
        &mut self,
        api_key_id: &str,
        identity_id: &str,
        expected_revision: u32,
        status: &str,
        integrity_hmac: &str,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE api_keys SET status=?,revision=revision+1,integrity_hmac=?
             WHERE id=? AND identity_id=? AND revision=?",
            params![
                status,
                integrity_hmac,
                api_key_id,
                identity_id,
                expected_revision
            ],
        )? == 1;
        if !changed {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        if !append_audit_events_sqlite(
            &transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )? {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(AuditedMutationOutcome::Applied)
    }

    pub fn touch_api_key_last_used(
        &self,
        api_key_id: &str,
        last_used_at: &str,
    ) -> Result<bool, StorageError> {
        Ok(self.connection.execute(
            "UPDATE api_keys SET last_used_at=? WHERE id=? AND status='active'",
            params![last_used_at, api_key_id],
        )? == 1)
    }

    pub fn upsert_model(&self, model: &ModelRecord) -> Result<(), StorageError> {
        self.connection.execute(
            "INSERT INTO models(id,public_name,display_name,enabled,discovered_at,created_at)
             VALUES(?,?,?,?,?,?)
             ON CONFLICT(id) DO UPDATE SET
               public_name=excluded.public_name,
               display_name=excluded.display_name,
               discovered_at=excluded.discovered_at",
            params![
                model.id,
                model.public_name,
                model.display_name,
                u8::from(model.enabled),
                model.discovered_at,
                model.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn enabled_models(&self) -> Result<Vec<ModelRecord>, StorageError> {
        let mut statement = self.connection.prepare(
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
        )?;
        let rows = statement.query_map([], model_from_sqlite_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub fn list_models(&self) -> Result<Vec<ModelRecord>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT id,public_name,display_name,enabled,discovered_at,created_at
             FROM models ORDER BY public_name,id",
        )?;
        statement
            .query_map([], model_from_sqlite_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub fn update_model_enabled(
        &self,
        model_id: &str,
        enabled: bool,
    ) -> Result<bool, StorageError> {
        Ok(self.connection.execute(
            "UPDATE models SET enabled=? WHERE id=?",
            params![enabled, model_id],
        )? == 1)
    }

    pub fn update_model_enabled_and_audit(
        &mut self,
        model_id: &str,
        enabled: bool,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if transaction.execute(
            "UPDATE models SET enabled=? WHERE id=?",
            params![enabled, model_id],
        )? != 1
        {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        if !append_audit_events_sqlite(
            &transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )? {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(AuditedMutationOutcome::Applied)
    }

    pub fn enabled_model_by_public_name(
        &self,
        public_name: &str,
    ) -> Result<Option<ModelRecord>, StorageError> {
        self.connection
            .query_row(
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
                [public_name],
                model_from_sqlite_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn replace_account_models(
        &mut self,
        account_id: &str,
        models: &[DiscoveredModel],
        discovered_at: &str,
    ) -> Result<(), StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "DELETE FROM account_models WHERE account_id=?",
            [account_id],
        )?;
        for model in models {
            transaction.execute(
                "INSERT INTO models(id,public_name,display_name,enabled,discovered_at,created_at)
                 VALUES(?,?,?,1,?,?)
                 ON CONFLICT(public_name) DO UPDATE SET
                   display_name=excluded.display_name,discovered_at=excluded.discovered_at",
                params![
                    model.id,
                    model.public_name,
                    model.display_name,
                    discovered_at,
                    discovered_at,
                ],
            )?;
            let model_id: String = transaction.query_row(
                "SELECT id FROM models WHERE public_name=?",
                [&model.public_name],
                |row| row.get(0),
            )?;
            transaction.execute(
                "INSERT INTO account_models(account_id,model_id,upstream_name,discovered_at)
                 VALUES(?,?,?,?)",
                params![account_id, model_id, model.upstream_name, discovered_at],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn replace_account_models_and_record_success_and_audit(
        &mut self,
        account_id: &str,
        models: &[DiscoveredModel],
        discovered_at: &str,
        runner_id: &str,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "DELETE FROM account_models WHERE account_id=?",
            [account_id],
        )?;
        for model in models {
            transaction.execute(
                "INSERT INTO models(id,public_name,display_name,enabled,discovered_at,created_at)
                 VALUES(?,?,?,1,?,?)
                 ON CONFLICT(public_name) DO UPDATE SET
                   display_name=excluded.display_name,discovered_at=excluded.discovered_at",
                params![
                    model.id,
                    model.public_name,
                    model.display_name,
                    discovered_at,
                    discovered_at,
                ],
            )?;
            let model_id: String = transaction.query_row(
                "SELECT id FROM models WHERE public_name=?",
                [&model.public_name],
                |row| row.get(0),
            )?;
            transaction.execute(
                "INSERT INTO account_models(account_id,model_id,upstream_name,discovered_at)
                 VALUES(?,?,?,?)",
                params![account_id, model_id, model.upstream_name, discovered_at],
            )?;
        }
        if transaction.execute(
            "UPDATE upstream_accounts
             SET last_success_runner_id=?,last_verified_at=?,updated_at=?
             WHERE id=? AND status='active'",
            params![runner_id, discovered_at, discovered_at, account_id],
        )? != 1
        {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        if !append_audit_events_sqlite(
            &transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )? {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(AuditedMutationOutcome::Applied)
    }

    pub fn gateway_route_candidates(
        &self,
        public_name: &str,
    ) -> Result<Vec<GatewayRouteCandidate>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT m.id,m.public_name,am.upstream_name,a.id,a.provider,a.subject_id,
                    a.last_success_runner_id,
                    c.id,c.account_id,c.credential_identity_hmac,c.encrypted_payload,
                    c.payload_nonce,c.wrapped_data_key,c.wrap_nonce,c.credential_revision,
                    c.expires_at,c.status,c.last_refreshed_at,c.created_at,c.updated_at
             FROM models m
             JOIN account_models am ON am.model_id=m.id
             JOIN upstream_accounts a ON a.id=am.account_id AND a.status='active'
             JOIN upstream_credential_instances c ON c.account_id=a.id AND c.status='active'
             WHERE m.public_name=? AND m.enabled=1
             ORDER BY a.last_verified_at DESC,c.expires_at DESC,c.updated_at DESC,c.id ASC",
        )?;
        statement
            .query_map([public_name], gateway_route_from_sqlite_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub fn runner_by_credential_hash(
        &self,
        credential_hash: &str,
    ) -> Result<Option<RunnerRecord>, StorageError> {
        self.connection
            .query_row(
                "SELECT id,enabled,version,protocol_version,max_inflight,inflight,
                        recent_request_count,recent_error_count,latency_ms,last_seen_at
                 FROM runners WHERE credential_hash=? AND enabled=1",
                params![credential_hash],
                |row| {
                    Ok(RunnerRecord {
                        id: row.get(0)?,
                        enabled: row.get::<_, u32>(1)? == 1,
                        version: row.get(2)?,
                        protocol_version: row.get(3)?,
                        max_inflight: row.get(4)?,
                        inflight: row.get(5)?,
                        recent_request_count: row.get(6)?,
                        recent_error_count: row.get(7)?,
                        latency_ms: row.get(8)?,
                        last_seen_at: row.get(9)?,
                    })
                },
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn list_enabled_runners(&self) -> Result<Vec<RunnerRecord>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT id,enabled,version,protocol_version,max_inflight,inflight,
                    recent_request_count,recent_error_count,latency_ms,last_seen_at
             FROM runners WHERE enabled=1 ORDER BY id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(RunnerRecord {
                id: row.get(0)?,
                enabled: row.get::<_, u32>(1)? == 1,
                version: row.get(2)?,
                protocol_version: row.get(3)?,
                max_inflight: row.get(4)?,
                inflight: row.get(5)?,
                recent_request_count: row.get(6)?,
                recent_error_count: row.get(7)?,
                latency_ms: row.get(8)?,
                last_seen_at: row.get(9)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub fn list_runners(&self) -> Result<Vec<RunnerAdminRecord>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT id,name,enabled,version,protocol_version,platform,architecture,max_inflight,
                    inflight,recent_request_count,recent_error_count,latency_ms,last_seen_at,
                    created_at,updated_at
             FROM runners ORDER BY created_at,id",
        )?;
        statement
            .query_map([], runner_admin_from_sqlite_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub fn insert_runner_enrollment(
        &self,
        enrollment: &RunnerEnrollmentRecord,
    ) -> Result<(), StorageError> {
        self.connection.execute(
            "INSERT INTO runner_enrollments(
               id,token_hash,token_prefix,runner_name,status,expires_at,created_by,created_at
             ) VALUES(?,?,?,?,?,?,?,?)",
            params![
                enrollment.id,
                enrollment.token_hash,
                enrollment.token_prefix,
                enrollment.runner_name,
                enrollment.status,
                enrollment.expires_at,
                enrollment.created_by,
                enrollment.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn pending_runner_enrollment(
        &self,
        token_hash: &str,
        now: &str,
    ) -> Result<bool, StorageError> {
        self.connection
            .query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM runner_enrollments
                   WHERE token_hash=? AND status='pending' AND expires_at>? AND runner_id IS NULL
                 )",
                params![token_hash, now],
                |row| row.get(0),
            )
            .map_err(StorageError::from)
    }

    pub fn insert_runner_enrollment_and_audit(
        &mut self,
        enrollment: &RunnerEnrollmentRecord,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO runner_enrollments(
               id,token_hash,token_prefix,runner_name,status,expires_at,created_by,created_at
             ) VALUES(?,?,?,?,?,?,?,?)",
            params![
                enrollment.id,
                enrollment.token_hash,
                enrollment.token_prefix,
                enrollment.runner_name,
                enrollment.status,
                enrollment.expires_at,
                enrollment.created_by,
                enrollment.created_at,
            ],
        )?;
        if !append_audit_events_sqlite(
            &transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )? {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(AuditedMutationOutcome::Applied)
    }

    /// Register an installation-owned Runner atomically. The retained enrollment
    /// also prevents a deleted slot identity from being silently recreated.
    pub fn register_local_runner_and_audit(
        &mut self,
        enrollment: &RunnerEnrollmentRecord,
        registration: &RunnerRegistrationRecord,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let seen: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM runner_enrollments WHERE id=? OR runner_id=? OR runner_name=?)",
            params![enrollment.id, registration.id, enrollment.runner_name],
            |row| row.get(0),
        )?;
        if seen {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        transaction.execute(
            "INSERT INTO runner_enrollments(id,token_hash,token_prefix,runner_name,status,expires_at,created_by,created_at)
             VALUES(?,?,?,?,'pending',?,?,?)",
            params![enrollment.id, enrollment.token_hash, enrollment.token_prefix, enrollment.runner_name,
                enrollment.expires_at, enrollment.created_by, enrollment.created_at],
        )?;
        transaction.execute(
            "INSERT INTO runners(id,enrollment_id,name,credential_hash,enabled,version,protocol_version,
               platform,architecture,max_inflight,created_at,updated_at) VALUES(?,?,?,?,1,?,?,?,?,?,?,?)",
            params![registration.id, enrollment.id, enrollment.runner_name, registration.credential_hash,
                registration.version, registration.protocol_version, registration.platform,
                registration.architecture, registration.max_inflight, registration.created_at, registration.created_at],
        )?;
        transaction.execute(
            "UPDATE runner_enrollments SET status='used',used_at=?,runner_id=? WHERE id=?",
            params![registration.created_at, registration.id, enrollment.id],
        )?;
        if !append_audit_events_sqlite(
            &transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )? {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(AuditedMutationOutcome::Applied)
    }

    pub fn consume_runner_enrollment_unchecked(
        &mut self,
        token_hash: &str,
        now: &str,
        registration: &RunnerRegistrationRecord,
    ) -> Result<RunnerEnrollmentConsumeOutcome, StorageError> {
        match self.consume_runner_enrollment_inner(
            token_hash,
            now,
            registration,
            RunnerQuotaPolicy::physical(None),
            None,
        )? {
            MutationWithAuditOutcome::Mutation(outcome) => Ok(outcome),
            MutationWithAuditOutcome::AuditConflict => Err(StorageError::DatabaseNotEmpty),
        }
    }

    pub fn consume_runner_enrollment_unchecked_and_audit(
        &mut self,
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
    }

    #[allow(clippy::too_many_arguments)]
    pub fn consume_runner_enrollment_with_limit_and_audit(
        &mut self,
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
    }

    pub fn consume_runner_enrollment_with_quota_and_audit(
        &mut self,
        token_hash: &str,
        now: &str,
        registration: &RunnerRegistrationRecord,
        quota: RunnerQuotaPolicy,
        audit: (u64, &str, &AuditEventRecord),
    ) -> Result<MutationWithAuditOutcome<RunnerEnrollmentConsumeOutcome>, StorageError> {
        self.consume_runner_enrollment_inner(token_hash, now, registration, quota, Some(audit))
    }

    fn consume_runner_enrollment_inner(
        &mut self,
        token_hash: &str,
        now: &str,
        registration: &RunnerRegistrationRecord,
        quota: RunnerQuotaPolicy,
        audit: Option<(u64, &str, &AuditEventRecord)>,
    ) -> Result<MutationWithAuditOutcome<RunnerEnrollmentConsumeOutcome>, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let enrollment_exists = transaction.query_row(
            "SELECT EXISTS(
               SELECT 1 FROM runner_enrollments
               WHERE token_hash=? AND status='pending' AND expires_at>? AND runner_id IS NULL
             )",
            params![token_hash, now],
            |row| row.get::<_, bool>(0),
        )?;
        if !enrollment_exists {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                RunnerEnrollmentConsumeOutcome::Invalid,
            ));
        }
        transaction.execute(
            "UPDATE transaction_gates SET revision=revision+1 WHERE gate_key='entity_quota'",
            [],
        )?;
        let actual = runner_quota_snapshot_sqlite(&transaction)?;
        let Some(occupied) = quota.occupied(&actual)? else {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                RunnerEnrollmentConsumeOutcome::QuotaChanged,
            ));
        };
        if quota.limit.is_some_and(|limit| occupied >= limit) {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                RunnerEnrollmentConsumeOutcome::LimitReached,
            ));
        }
        let inserted = transaction.execute(
            "INSERT INTO runners(
               id,enrollment_id,name,credential_hash,enabled,version,protocol_version,
               platform,architecture,max_inflight,created_at,updated_at
             )
             SELECT ?,e.id,e.runner_name,?,1,?,?,?,?,?,?,?
             FROM runner_enrollments e
             WHERE e.token_hash=? AND e.status='pending' AND e.expires_at>? AND e.runner_id IS NULL",
            params![
                registration.id,
                registration.credential_hash,
                registration.version,
                registration.protocol_version,
                registration.platform,
                registration.architecture,
                registration.max_inflight,
                registration.created_at,
                registration.created_at,
                token_hash,
                now,
            ],
        )?;
        if inserted != 1 {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                RunnerEnrollmentConsumeOutcome::Invalid,
            ));
        }
        let updated = transaction.execute(
            "UPDATE runner_enrollments
             SET status='used',used_at=?,runner_id=?
             WHERE id=(SELECT enrollment_id FROM runners WHERE id=?)
               AND status='pending' AND runner_id IS NULL",
            params![now, registration.id, registration.id],
        )?;
        if updated != 1 {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                RunnerEnrollmentConsumeOutcome::Invalid,
            ));
        }
        if let Some((expected_sequence, expected_hmac, event)) = audit
            && !append_audit_events_sqlite(
                &transaction,
                expected_sequence,
                expected_hmac,
                std::slice::from_ref(event),
            )?
        {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(MutationWithAuditOutcome::Mutation(
            RunnerEnrollmentConsumeOutcome::Registered,
        ))
    }

    pub fn update_runner_enabled(
        &self,
        runner_id: &str,
        enabled: bool,
        updated_at: &str,
    ) -> Result<bool, StorageError> {
        Ok(self.connection.execute(
            "UPDATE runners SET enabled=?,updated_at=? WHERE id=?",
            params![enabled, updated_at, runner_id],
        )? == 1)
    }

    pub fn update_runner_enabled_and_audit(
        &mut self,
        runner_id: &str,
        enabled: bool,
        updated_at: &str,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if transaction.execute(
            "UPDATE runners SET enabled=?,updated_at=? WHERE id=?",
            params![enabled, updated_at, runner_id],
        )? != 1
        {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        if !append_audit_events_sqlite(
            &transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )? {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(AuditedMutationOutcome::Applied)
    }

    pub fn delete_runner(&self, runner_id: &str) -> Result<bool, StorageError> {
        Ok(self
            .connection
            .execute("DELETE FROM runners WHERE id=?", [runner_id])?
            == 1)
    }

    pub fn delete_runner_and_audit(
        &mut self,
        runner_id: &str,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if transaction.execute("DELETE FROM runners WHERE id=?", [runner_id])? != 1 {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        if !append_audit_events_sqlite(
            &transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )? {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(AuditedMutationOutcome::Applied)
    }

    pub fn record_runner_heartbeat(
        &self,
        runner_id: &str,
        runner_version: &str,
        max_inflight: u32,
        heartbeat: RunnerHeartbeatUpdate,
        observed_at: &str,
    ) -> Result<bool, StorageError> {
        let changed = self.connection.execute(
            "UPDATE runners SET version=?,max_inflight=?,inflight=?,recent_request_count=?,
                    recent_error_count=?,latency_ms=?,last_seen_at=?,updated_at=?
             WHERE id=? AND enabled=1",
            params![
                runner_version,
                max_inflight,
                heartbeat.inflight,
                heartbeat.recent_request_count,
                heartbeat.recent_error_count,
                heartbeat.latency_ms,
                observed_at,
                observed_at,
                runner_id,
            ],
        )?;
        Ok(changed == 1)
    }

    pub fn record_runner_connection(
        &self,
        update: &RunnerConnectionUpdate,
    ) -> Result<bool, StorageError> {
        if update.protocol_version < update.previous_protocol_version {
            return Ok(false);
        }
        let changed = self.connection.execute(
            "UPDATE runners SET protocol_version=?,version=?,max_inflight=?,inflight=?,
                    recent_request_count=?,recent_error_count=?,latency_ms=?,last_seen_at=?,updated_at=?
             WHERE id=? AND credential_hash=? AND enabled=1 AND protocol_version IN (?,?)",
            params![update.protocol_version, update.runner_version, update.max_inflight,
                update.heartbeat.inflight, update.heartbeat.recent_request_count,
                update.heartbeat.recent_error_count, update.heartbeat.latency_ms,
                update.observed_at, update.observed_at, update.runner_id,
                update.credential_hash, update.previous_protocol_version, update.protocol_version],
        )?;
        Ok(changed == 1)
    }

    pub fn insert_upstream_account_unchecked(
        &self,
        id: &str,
        provider: &str,
        subject_id: &str,
        email: &str,
        now: &str,
    ) -> Result<(), StorageError> {
        self.connection.execute(
            "INSERT INTO upstream_accounts(id,provider,subject_id,email,created_at,updated_at) VALUES(?,?,?,?,?,?)",
            params![id, provider, subject_id, email, now, now],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_upstream_account_unchecked_and_audit(
        &mut self,
        id: &str,
        provider: &str,
        subject_id: &str,
        email: &str,
        now: &str,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        match self.insert_upstream_account_with_limit_and_audit(
            id,
            provider,
            subject_id,
            email,
            now,
            None,
            expected_audit_sequence,
            expected_audit_hmac,
            audit_event,
        )? {
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
    pub fn insert_upstream_account_with_limit_and_audit(
        &mut self,
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
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "UPDATE transaction_gates SET revision=revision+1 WHERE gate_key='entity_quota'",
            [],
        )?;
        let account_exists = transaction.query_row(
            "SELECT EXISTS(
               SELECT 1 FROM upstream_accounts
               WHERE id=? OR (provider=? AND subject_id=?)
             )",
            params![id, provider, subject_id],
            |row| row.get::<_, bool>(0),
        )?;
        if account_exists {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::Mutation(
                ResourceCreateOutcome::Conflict,
            ));
        }
        if let Some(limit) = account_limit {
            let occupied =
                transaction.query_row("SELECT count(*) FROM upstream_accounts", [], |row| {
                    row.get::<_, i64>(0)
                })?;
            if occupied >= i64::from(limit) {
                transaction.rollback()?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    ResourceCreateOutcome::LimitReached,
                ));
            }
        }
        let inserted = transaction.execute(
            "INSERT INTO upstream_accounts(id,provider,subject_id,email,created_at,updated_at)
             VALUES(?,?,?,?,?,?)",
            params![id, provider, subject_id, email, now, now],
        );
        match inserted {
            Ok(1) => {}
            Ok(_) => {
                transaction.rollback()?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    ResourceCreateOutcome::Conflict,
                ));
            }
            Err(error) if is_sqlite_unique_violation(&error) => {
                transaction.rollback()?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    ResourceCreateOutcome::Conflict,
                ));
            }
            Err(error) => return Err(StorageError::Database(error)),
        }
        if !append_audit_events_sqlite(
            &transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )? {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(MutationWithAuditOutcome::Mutation(
            ResourceCreateOutcome::Created,
        ))
    }

    pub fn upstream_account_by_provider_subject(
        &self,
        provider: &str,
        subject_id: &str,
    ) -> Result<Option<UpstreamAccountRecord>, StorageError> {
        self.connection
            .query_row(
                "SELECT id,provider,subject_id,email,plan,status,last_success_runner_id,
                        last_verified_at,created_at,updated_at
                 FROM upstream_accounts WHERE provider=? AND subject_id=?",
                params![provider, subject_id],
                upstream_account_from_sqlite_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn upstream_account_by_id(
        &self,
        id: &str,
    ) -> Result<Option<UpstreamAccountRecord>, StorageError> {
        self.connection
            .query_row(
                "SELECT id,provider,subject_id,email,plan,status,last_success_runner_id,
                        last_verified_at,created_at,updated_at
                 FROM upstream_accounts WHERE id=?",
                [id],
                upstream_account_from_sqlite_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn list_upstream_accounts(&self) -> Result<Vec<UpstreamAccountRecord>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT id,provider,subject_id,email,plan,status,last_success_runner_id,
                    last_verified_at,created_at,updated_at
             FROM upstream_accounts ORDER BY created_at,id",
        )?;
        statement
            .query_map([], upstream_account_from_sqlite_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub fn update_upstream_account_metadata(
        &self,
        id: &str,
        email: &str,
        plan: &str,
        verified_at: &str,
    ) -> Result<bool, StorageError> {
        Ok(self.connection.execute(
            "UPDATE upstream_accounts
             SET email=?,plan=?,status='active',last_verified_at=?,updated_at=?
             WHERE id=?",
            params![email, plan, verified_at, verified_at, id],
        )? == 1)
    }

    pub fn update_upstream_account_status(
        &self,
        id: &str,
        status: &str,
        updated_at: &str,
    ) -> Result<bool, StorageError> {
        Ok(self.connection.execute(
            "UPDATE upstream_accounts SET status=?,updated_at=? WHERE id=?",
            params![status, updated_at, id],
        )? == 1)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_upstream_account_status_and_audit(
        &mut self,
        id: &str,
        status: &str,
        updated_at: &str,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if transaction.execute(
            "UPDATE upstream_accounts SET status=?,updated_at=? WHERE id=?",
            params![status, updated_at, id],
        )? != 1
        {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        if !append_audit_events_sqlite(
            &transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )? {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(AuditedMutationOutcome::Applied)
    }

    pub fn delete_upstream_account(&self, id: &str) -> Result<bool, StorageError> {
        Ok(self
            .connection
            .execute("DELETE FROM upstream_accounts WHERE id=?", [id])?
            == 1)
    }

    pub fn delete_upstream_account_and_audit(
        &mut self,
        id: &str,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<AuditedMutationOutcome, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if transaction.execute("DELETE FROM upstream_accounts WHERE id=?", [id])? != 1 {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        if !append_audit_events_sqlite(
            &transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )? {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(AuditedMutationOutcome::Applied)
    }

    pub fn upstream_account_provider(&self, id: &str) -> Result<Option<String>, StorageError> {
        self.connection
            .query_row(
                "SELECT provider FROM upstream_accounts WHERE id=?",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn update_account_last_success_runner(
        &self,
        account_id: &str,
        runner_id: &str,
        updated_at: &str,
    ) -> Result<bool, StorageError> {
        Ok(self.connection.execute(
            "UPDATE upstream_accounts
             SET last_success_runner_id=?,last_verified_at=?,updated_at=?
             WHERE id=? AND status='active'",
            params![runner_id, updated_at, updated_at, account_id],
        )? == 1)
    }

    pub fn credential_instance_by_id(
        &self,
        id: &str,
    ) -> Result<Option<EncryptedCredentialInstance>, StorageError> {
        self.connection
            .query_row(
                "SELECT id,account_id,credential_identity_hmac,encrypted_payload,payload_nonce,
                        wrapped_data_key,wrap_nonce,credential_revision,expires_at,status,
                        last_refreshed_at,created_at,updated_at
                 FROM upstream_credential_instances WHERE id=?",
                [id],
                credential_from_sqlite_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn insert_credential_instance(
        &self,
        credential: &EncryptedCredentialInstance,
    ) -> Result<(), StorageError> {
        self.connection.execute(
            "INSERT INTO upstream_credential_instances(
               id,account_id,credential_identity_hmac,encrypted_payload,payload_nonce,wrapped_data_key,wrap_nonce,
               credential_revision,expires_at,status,last_refreshed_at,created_at,updated_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
            params![
                credential.id,
                credential.account_id,
                credential.credential_identity_hmac,
                credential.encrypted_payload,
                credential.payload_nonce,
                credential.wrapped_data_key,
                credential.wrap_nonce,
                credential.credential_revision,
                credential.expires_at,
                credential.status,
                credential.last_refreshed_at,
                credential.created_at,
                credential.updated_at,
            ],
        )
        .map_err(map_sqlite_credential_write_error)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_account_metadata_and_insert_credential_and_audit(
        &mut self,
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
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if transaction.execute(
            "UPDATE upstream_accounts
             SET email=?,plan=?,status='active',last_verified_at=?,updated_at=?
             WHERE id=?",
            params![email, plan, verified_at, verified_at, account_id],
        )? != 1
        {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::MutationConflict);
        }
        transaction
            .execute(
                "INSERT INTO upstream_credential_instances(
                   id,account_id,credential_identity_hmac,encrypted_payload,payload_nonce,
                   wrapped_data_key,wrap_nonce,credential_revision,expires_at,status,
                   last_refreshed_at,created_at,updated_at
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    credential.id,
                    credential.account_id,
                    credential.credential_identity_hmac,
                    credential.encrypted_payload,
                    credential.payload_nonce,
                    credential.wrapped_data_key,
                    credential.wrap_nonce,
                    credential.credential_revision,
                    credential.expires_at,
                    credential.status,
                    credential.last_refreshed_at,
                    credential.created_at,
                    credential.updated_at,
                ],
            )
            .map_err(map_sqlite_credential_write_error)?;
        if !append_audit_events_sqlite(
            &transaction,
            expected_audit_sequence,
            expected_audit_hmac,
            std::slice::from_ref(audit_event),
        )? {
            transaction.rollback()?;
            return Ok(AuditedMutationOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(AuditedMutationOutcome::Applied)
    }

    pub fn credential_instances_for_account(
        &self,
        account_id: &str,
    ) -> Result<Vec<EncryptedCredentialInstance>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT id,account_id,credential_identity_hmac,encrypted_payload,payload_nonce,wrapped_data_key,wrap_nonce,
                    credential_revision,expires_at,status,last_refreshed_at,created_at,updated_at
             FROM upstream_credential_instances WHERE account_id=? ORDER BY created_at,id",
        )?;
        let rows = statement.query_map([account_id], credential_from_sqlite_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub fn update_credential_after_refresh(
        &mut self,
        id: &str,
        expected_revision: u32,
        lease_token_hash: &str,
        now: &str,
        update: &CredentialRefreshUpdate,
    ) -> Result<bool, StorageError> {
        match self.update_credential_after_refresh_inner(
            id,
            expected_revision,
            lease_token_hash,
            now,
            update,
            None,
        )? {
            MutationWithAuditOutcome::Mutation(changed) => Ok(changed),
            MutationWithAuditOutcome::AuditConflict => Err(StorageError::DatabaseNotEmpty),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_credential_after_refresh_and_audit(
        &mut self,
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
    }

    #[allow(clippy::too_many_arguments)]
    fn update_credential_after_refresh_inner(
        &mut self,
        id: &str,
        expected_revision: u32,
        lease_token_hash: &str,
        now: &str,
        update: &CredentialRefreshUpdate,
        audit: Option<(u64, &str, &AuditEventRecord)>,
    ) -> Result<MutationWithAuditOutcome<bool>, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "DELETE FROM credential_refresh_leases
             WHERE credential_instance_id=? AND expires_at<=?",
            params![id, now],
        )?;
        let changed = transaction
            .execute(
                "UPDATE upstream_credential_instances
                 SET credential_identity_hmac=?,encrypted_payload=?,payload_nonce=?,wrapped_data_key=?,wrap_nonce=?,expires_at=?,
                     credential_revision=credential_revision+1,status='active',last_refreshed_at=?,updated_at=?
                 WHERE id=? AND credential_revision=? AND EXISTS(
                   SELECT 1 FROM credential_refresh_leases l
                   WHERE l.credential_instance_id=upstream_credential_instances.id
                     AND l.lease_token_hash=? AND l.expected_revision=? AND l.expires_at>?
                 )",
                params![
                    update.credential_identity_hmac,
                    update.encrypted_payload,
                    update.payload_nonce,
                    update.wrapped_data_key,
                    update.wrap_nonce,
                    update.expires_at,
                    update.refreshed_at,
                    update.refreshed_at,
                    id,
                    expected_revision,
                    lease_token_hash,
                    expected_revision,
                    now,
                ],
            )
            .map_err(map_sqlite_credential_write_error)?
            == 1;
        transaction.execute(
            "DELETE FROM credential_refresh_leases
             WHERE credential_instance_id=? AND lease_token_hash=?",
            params![id, lease_token_hash],
        )?;
        if changed
            && let Some((expected_sequence, expected_hmac, event)) = audit
            && !append_audit_events_sqlite(
                &transaction,
                expected_sequence,
                expected_hmac,
                std::slice::from_ref(event),
            )?
        {
            transaction.rollback()?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(MutationWithAuditOutcome::Mutation(changed))
    }

    pub fn acquire_credential_refresh_lease(
        &mut self,
        credential_id: &str,
        expected_revision: u32,
        lease_token_hash: &str,
        now: &str,
        expires_at: &str,
    ) -> Result<CredentialRefreshLeaseOutcome, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "DELETE FROM credential_refresh_leases WHERE expires_at<=?",
            [now],
        )?;
        let credential = transaction
            .query_row(
                "SELECT credential_revision,status FROM upstream_credential_instances WHERE id=?",
                [credential_id],
                |row| Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let outcome = match credential {
            None => CredentialRefreshLeaseOutcome::CredentialUnavailable,
            Some((_, status)) if status != "active" => {
                CredentialRefreshLeaseOutcome::CredentialUnavailable
            }
            Some((revision, _)) if revision != expected_revision => {
                CredentialRefreshLeaseOutcome::RevisionChanged
            }
            Some(_) => {
                let occupied = transaction
                    .query_row(
                        "SELECT 1 FROM credential_refresh_leases WHERE credential_instance_id=?",
                        [credential_id],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
                if occupied {
                    CredentialRefreshLeaseOutcome::Busy
                } else {
                    transaction.execute(
                        "INSERT INTO credential_refresh_leases(
                           credential_instance_id,lease_token_hash,expected_revision,expires_at,created_at
                         ) VALUES(?,?,?,?,?)",
                        params![
                            credential_id,
                            lease_token_hash,
                            expected_revision,
                            expires_at,
                            now,
                        ],
                    )?;
                    CredentialRefreshLeaseOutcome::Acquired
                }
            }
        };
        transaction.commit()?;
        Ok(outcome)
    }

    pub fn release_credential_refresh_lease(
        &self,
        credential_id: &str,
        lease_token_hash: &str,
    ) -> Result<bool, StorageError> {
        Ok(self.connection.execute(
            "DELETE FROM credential_refresh_leases
             WHERE credential_instance_id=? AND lease_token_hash=?",
            params![credential_id, lease_token_hash],
        )? == 1)
    }

    fn apply_migrations(&self, migrations: &[EmbeddedMigration]) -> Result<(), StorageError> {
        if migrations.is_empty() {
            return Err(StorageError::Uninitialized);
        }
        validate_migration_sequence(migrations)?;
        if !self.has_user_tables()? {
            let baseline = migrations[0];
            let transaction =
                Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
            transaction.execute_batch(baseline.sql)?;
            record_sqlite_migration(&transaction, baseline)?;
            transaction.commit()?;
        } else if !self.has_table("schema_migrations")? {
            return Err(StorageError::DatabaseNotEmpty);
        }

        self.verify_known_migrations(migrations)?;
        for migration in migrations {
            if self.migration_record(migration.version)?.is_some() {
                continue;
            }
            if migration.version == migrations[0].version {
                return Err(StorageError::MigrationIntegrity);
            }
            let transaction =
                Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
            transaction.execute_batch(migration.sql)?;
            record_sqlite_migration(&transaction, *migration)?;
            transaction.commit()?;
        }
        self.verify_known_migrations(migrations)
    }

    fn verify_known_migrations(
        &self,
        migrations: &[EmbeddedMigration],
    ) -> Result<(), StorageError> {
        for migration in migrations {
            let Some((name, checksum)) = self.migration_record(migration.version)? else {
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

    fn migration_record(&self, version: u32) -> Result<Option<(String, String)>, StorageError> {
        self.connection
            .query_row(
                "SELECT name,checksum_sha256 FROM schema_migrations WHERE version=?",
                [version],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(StorageError::from)
    }

    fn has_table(&self, name: &str) -> Result<bool, StorageError> {
        Ok(self
            .connection
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name=? LIMIT 1",
                [name],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    fn has_user_tables(&self) -> Result<bool, StorageError> {
        let count: u32 = self.connection.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn record_sqlite_migration(
    transaction: &Transaction<'_>,
    migration: EmbeddedMigration,
) -> Result<(), StorageError> {
    transaction.execute(
        "INSERT INTO schema_migrations(version,name,checksum_sha256,applied_at)
         VALUES(?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![
            migration.version,
            migration.name,
            migration_checksum(migration.sql)
        ],
    )?;
    Ok(())
}

fn validate_migration_sequence(migrations: &[EmbeddedMigration]) -> Result<(), StorageError> {
    for (index, migration) in migrations.iter().enumerate() {
        let expected = u32::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .ok_or(StorageError::MigrationIntegrity)?;
        let expected_compatibility = if index == 0 {
            MigrationCompatibility::Baseline
        } else {
            MigrationCompatibility::RollingUpgradeSafe
        };
        if migration.version != expected
            || migration.name.is_empty()
            || migration.sql.is_empty()
            || migration.online_sql.is_some_and(str::is_empty)
            || migration.recovery_query.is_some_and(str::is_empty)
            || migration.compatibility != expected_compatibility
        {
            return Err(StorageError::MigrationIntegrity);
        }
    }
    Ok(())
}

fn migration_checksum(sql: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(sql.as_bytes());
    hex_bytes(&digest.finalize())
}

fn valid_audit_event_batch(
    expected_sequence: u64,
    expected_hmac: &str,
    events: &[AuditEventRecord],
) -> bool {
    if events.is_empty() {
        return false;
    }
    let mut sequence = expected_sequence;
    let mut previous_hmac = expected_hmac;
    for event in events {
        let Some(next_sequence) = sequence.checked_add(1) else {
            return false;
        };
        if event.sequence != next_sequence
            || event.previous_event_hmac != previous_hmac
            || event.integrity_hmac.len() != 43
        {
            return false;
        }
        sequence = event.sequence;
        previous_hmac = &event.integrity_hmac;
    }
    true
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
pub(crate) fn append_audit_events_sqlite(
    transaction: &Transaction<'_>,
    expected_sequence: u64,
    expected_hmac: &str,
    events: &[AuditEventRecord],
) -> Result<bool, StorageError> {
    if !valid_audit_event_batch(expected_sequence, expected_hmac, events) {
        return Ok(false);
    }
    let revision_delta = i64::try_from(events.len()).map_err(|_| StorageError::DatabaseNotEmpty)?;
    if transaction.execute(
        "UPDATE transaction_gates SET revision=revision+? WHERE gate_key='audit_log'",
        [revision_delta],
    )? != 1
    {
        return Ok(false);
    }
    let current = transaction
        .query_row(
            "SELECT event_sequence,integrity_hmac
             FROM audit_events ORDER BY event_sequence DESC LIMIT 1",
            [],
            |row| Ok((row.get::<_, i64>(0)? as u64, row.get::<_, String>(1)?)),
        )
        .optional()?
        .unwrap_or((0, String::new()));
    if current.0 != expected_sequence || current.1 != expected_hmac {
        return Ok(false);
    }
    for event in events {
        let inserted = transaction.execute(
            "INSERT INTO audit_events(
               id,event_sequence,actor_identity_id,actor_role,action,target_type,target_id,
               outcome,previous_event_hmac,integrity_hmac,created_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
            params![
                event.id,
                i64::try_from(event.sequence).map_err(|_| StorageError::DatabaseNotEmpty)?,
                event.actor_identity_id,
                event.actor_role,
                event.action,
                event.target_type,
                event.target_id,
                event.outcome,
                event.previous_event_hmac,
                event.integrity_hmac,
                event.created_at,
            ],
        );
        match inserted {
            Ok(1) => {}
            Ok(_) => return Ok(false),
            Err(error) if is_sqlite_unique_violation(&error) => return Ok(false),
            Err(error) => return Err(StorageError::Database(error)),
        }
    }
    Ok(true)
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn credential_from_sqlite_row(
    row: &rusqlite::Row<'_>,
) -> Result<EncryptedCredentialInstance, rusqlite::Error> {
    Ok(EncryptedCredentialInstance {
        id: row.get(0)?,
        account_id: row.get(1)?,
        credential_identity_hmac: row.get(2)?,
        encrypted_payload: row.get(3)?,
        payload_nonce: row.get(4)?,
        wrapped_data_key: row.get(5)?,
        wrap_nonce: row.get(6)?,
        credential_revision: row.get(7)?,
        expires_at: row.get(8)?,
        status: row.get(9)?,
        last_refreshed_at: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn upstream_account_from_sqlite_row(
    row: &rusqlite::Row<'_>,
) -> Result<UpstreamAccountRecord, rusqlite::Error> {
    Ok(UpstreamAccountRecord {
        id: row.get(0)?,
        provider: row.get(1)?,
        subject_id: row.get(2)?,
        email: row.get(3)?,
        plan: row.get(4)?,
        status: row.get(5)?,
        last_success_runner_id: row.get(6)?,
        last_verified_at: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn runner_admin_from_sqlite_row(
    row: &rusqlite::Row<'_>,
) -> Result<RunnerAdminRecord, rusqlite::Error> {
    Ok(RunnerAdminRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        enabled: row.get::<_, u32>(2)? == 1,
        version: row.get(3)?,
        protocol_version: row.get(4)?,
        platform: row.get(5)?,
        architecture: row.get(6)?,
        max_inflight: row.get(7)?,
        inflight: row.get(8)?,
        recent_request_count: row.get(9)?,
        recent_error_count: row.get(10)?,
        latency_ms: row.get(11)?,
        last_seen_at: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn identity_from_sqlite_row(row: &rusqlite::Row<'_>) -> Result<IdentityRecord, rusqlite::Error> {
    Ok(IdentityRecord {
        id: row.get(0)?,
        email: row.get(1)?,
        display_name: row.get(2)?,
        password_hash: row.get(3)?,
        role: row.get(4)?,
        status: row.get(5)?,
        can_consume_model: row.get::<_, u8>(6)? == 1,
        password_change_required: row.get::<_, u8>(7)? == 1,
        revision: row.get(8)?,
        integrity_hmac: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn balance_from_sqlite_row(row: &rusqlite::Row<'_>) -> Result<UserBalanceRecord, rusqlite::Error> {
    Ok(UserBalanceRecord {
        identity_id: row.get(0)?,
        balance_tokens: row.get(1)?,
        reserved_tokens: row.get(2)?,
        granted_tokens: row.get(3)?,
        consumed_tokens: row.get(4)?,
        request_count: row.get(5)?,
        raw_tokens: row.get(6)?,
        billed_tokens: row.get(7)?,
        uncached_input: row.get(8)?,
        cached_input: row.get(9)?,
        cache_write: row.get(10)?,
        output_tokens: row.get(11)?,
        revision: row.get::<_, i64>(12)? as u64,
        last_ledger_hmac: row.get(13)?,
        integrity_hmac: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn quota_reservation_from_sqlite_row(
    row: &rusqlite::Row<'_>,
) -> Result<QuotaReservationRecord, rusqlite::Error> {
    Ok(QuotaReservationRecord {
        id: row.get(0)?,
        identity_id: row.get(1)?,
        api_key_id: row.get(2)?,
        request_id: row.get(3)?,
        reserved_tokens: row.get(4)?,
        status: row.get(5)?,
        revision: row.get::<_, i64>(6)? as u64,
        integrity_hmac: row.get(7)?,
        created_at: row.get(8)?,
        expires_at: row.get(9)?,
        settled_at: row.get(10)?,
    })
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn quota_request_from_sqlite_row(
    row: &rusqlite::Row<'_>,
) -> Result<QuotaRequestRecord, rusqlite::Error> {
    Ok(QuotaRequestRecord {
        id: row.get(0)?,
        identity_id: row.get(1)?,
        amount_nanos: row.get(2)?,
        reason: row.get(3)?,
        status: row.get(4)?,
        review_note: row.get(5)?,
        reviewed_by: row.get(6)?,
        reviewed_at: row.get(7)?,
        revision: row.get(8)?,
        integrity_hmac: row.get(9)?,
        created_at: row.get(10)?,
    })
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn voucher_from_sqlite_row(row: &rusqlite::Row<'_>) -> Result<VoucherRecord, rusqlite::Error> {
    Ok(VoucherRecord {
        id: row.get(0)?,
        code_hash: row.get(1)?,
        code_prefix: row.get(2)?,
        name: row.get(3)?,
        quota_tokens: row.get(4)?,
        status: row.get(5)?,
        max_redemptions: row.get(6)?,
        redeemed_count: row.get(7)?,
        expires_at: row.get(8)?,
        created_by: row.get(9)?,
        revision: row.get(10)?,
        integrity_hmac: row.get(11)?,
        created_at: row.get(12)?,
    })
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn voucher_delivery_from_sqlite_row(
    row: &rusqlite::Row<'_>,
) -> Result<VoucherDeliveryRecord, rusqlite::Error> {
    Ok(VoucherDeliveryRecord {
        id: row.get(0)?,
        voucher_id: row.get(1)?,
        identity_id: row.get(2)?,
        status: row.get(3)?,
        delivered_at: row.get(4)?,
        redeemed_at: row.get(5)?,
    })
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn voucher_delivery_pair_from_sqlite_row(
    row: &rusqlite::Row<'_>,
) -> Result<(VoucherDeliveryRecord, VoucherRecord), rusqlite::Error> {
    Ok((
        VoucherDeliveryRecord {
            id: row.get(0)?,
            voucher_id: row.get(1)?,
            identity_id: row.get(2)?,
            status: row.get(3)?,
            delivered_at: row.get(4)?,
            redeemed_at: row.get(5)?,
        },
        VoucherRecord {
            id: row.get(6)?,
            code_hash: row.get(7)?,
            code_prefix: row.get(8)?,
            name: row.get(9)?,
            quota_tokens: row.get(10)?,
            status: row.get(11)?,
            max_redemptions: row.get(12)?,
            redeemed_count: row.get(13)?,
            expires_at: row.get(14)?,
            created_by: row.get(15)?,
            revision: row.get(16)?,
            integrity_hmac: row.get(17)?,
            created_at: row.get(18)?,
        },
    ))
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn voucher_redemption_from_sqlite_row(
    row: &rusqlite::Row<'_>,
) -> Result<VoucherRedemptionRecord, rusqlite::Error> {
    Ok(VoucherRedemptionRecord {
        id: row.get(0)?,
        voucher_id: row.get(1)?,
        identity_id: row.get(2)?,
        amount_tokens: row.get(3)?,
        ledger_entry_id: row.get(4)?,
        created_at: row.get(5)?,
    })
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn runtime_setting_from_sqlite_row(
    row: &rusqlite::Row<'_>,
) -> Result<RuntimeSettingRecord, rusqlite::Error> {
    Ok(RuntimeSettingRecord {
        key: row.get(0)?,
        value: row.get(1)?,
        revision: row.get::<_, i64>(2)? as u64,
        integrity_hmac: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn audit_event_from_sqlite_row(
    row: &rusqlite::Row<'_>,
) -> Result<AuditEventRecord, rusqlite::Error> {
    Ok(AuditEventRecord {
        id: row.get(0)?,
        sequence: row.get::<_, i64>(1)? as u64,
        actor_identity_id: row.get(2)?,
        actor_role: row.get(3)?,
        action: row.get(4)?,
        target_type: row.get(5)?,
        target_id: row.get(6)?,
        outcome: row.get(7)?,
        previous_event_hmac: row.get(8)?,
        integrity_hmac: row.get(9)?,
        created_at: row.get(10)?,
    })
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn quota_ledger_from_sqlite_row(
    row: &rusqlite::Row<'_>,
) -> Result<QuotaLedgerEntry, rusqlite::Error> {
    Ok(QuotaLedgerEntry {
        id: row.get(0)?,
        identity_id: row.get(1)?,
        kind: row.get(2)?,
        amount_tokens: row.get(3)?,
        uncached_input: row.get(4)?,
        cached_input: row.get(5)?,
        cache_write: row.get(6)?,
        output_tokens: row.get(7)?,
        uncovered_tokens: row.get(8)?,
        raw_tokens: row.get(9)?,
        billed_tokens: row.get(10)?,
        multiplier_micros: row.get(11)?,
        reference_id: row.get::<_, Option<String>>(12)?.unwrap_or_default(),
        client_request_id: row.get(13)?,
        description: row.get(14)?,
        protocol: row.get(15)?,
        model: row.get(16)?,
        requested_model: row.get(17)?,
        processing_tier: row.get(18)?,
        reasoning_effort: row.get(19)?,
        api_key_id: row.get(20)?,
        runner_id: row.get(21)?,
        previous_entry_hmac: row.get(22)?,
        integrity_hmac: row.get(23)?,
        created_at: row.get(24)?,
    })
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn update_balance_sqlite(
    connection: &Connection,
    expected: &UserBalanceRecord,
    next: &UserBalanceRecord,
) -> Result<bool, StorageError> {
    Ok(connection.execute(
        "UPDATE user_balances SET
           balance_tokens=?,reserved_tokens=?,granted_tokens=?,consumed_tokens=?,
           request_count=?,raw_tokens=?,billed_tokens=?,uncached_input=?,cached_input=?,
           cache_write=?,output_tokens=?,revision=?,last_ledger_hmac=?,integrity_hmac=?,updated_at=?
         WHERE identity_id=? AND revision=? AND integrity_hmac=?",
        params![
            next.balance_tokens,
            next.reserved_tokens,
            next.granted_tokens,
            next.consumed_tokens,
            next.request_count,
            next.raw_tokens,
            next.billed_tokens,
            next.uncached_input,
            next.cached_input,
            next.cache_write,
            next.output_tokens,
            i64::try_from(next.revision).map_err(|_| StorageError::DatabaseNotEmpty)?,
            next.last_ledger_hmac,
            next.integrity_hmac,
            next.updated_at,
            expected.identity_id,
            i64::try_from(expected.revision).map_err(|_| StorageError::DatabaseNotEmpty)?,
            expected.integrity_hmac,
        ],
    )? == 1)
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn insert_ledger_sqlite(
    connection: &Connection,
    ledger: &QuotaLedgerEntry,
) -> Result<usize, rusqlite::Error> {
    connection.execute(
        "INSERT INTO ledger_entries(
           id,identity_id,kind,amount_tokens,uncached_input,cached_input,cache_write,
           output_tokens,uncovered_tokens,raw_tokens,billed_tokens,multiplier_micros,
           reference_id,client_request_id,description,protocol,model,requested_model,processing_tier,
           reasoning_effort,api_key_id,runner_id,
           previous_entry_hmac,integrity_hmac,created_at
         ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        params![
            ledger.id,
            ledger.identity_id,
            ledger.kind,
            ledger.amount_tokens,
            ledger.uncached_input,
            ledger.cached_input,
            ledger.cache_write,
            ledger.output_tokens,
            ledger.uncovered_tokens,
            ledger.raw_tokens,
            ledger.billed_tokens,
            ledger.multiplier_micros,
            ledger.reference_id,
            ledger.client_request_id,
            ledger.description,
            ledger.protocol,
            ledger.model,
            ledger.requested_model,
            ledger.processing_tier,
            ledger.reasoning_effort,
            ledger.api_key_id,
            ledger.runner_id,
            ledger.previous_entry_hmac,
            ledger.integrity_hmac,
            ledger.created_at,
        ],
    )
}

fn valid_balance_shape(balance: &UserBalanceRecord) -> bool {
    balance.balance_tokens >= 0
        && balance.reserved_tokens >= 0
        && balance.granted_tokens >= 0
        && balance.consumed_tokens >= 0
        && balance.request_count >= 0
        && balance.raw_tokens >= 0
        && balance.billed_tokens >= 0
        && balance.uncached_input >= 0
        && balance.cached_input >= 0
        && balance.cache_write >= 0
        && balance.output_tokens >= 0
        && balance.reserved_tokens <= balance.balance_tokens
        && !balance.integrity_hmac.is_empty()
}

fn valid_reserve_transition(
    expected: &UserBalanceRecord,
    next: &UserBalanceRecord,
    reservation: &QuotaReservationRecord,
) -> bool {
    valid_balance_shape(expected)
        && valid_balance_shape(next)
        && reservation.identity_id == expected.identity_id
        && reservation.reserved_tokens > 0
        && reservation.status == "active"
        && reservation.revision == 0
        && reservation.settled_at.is_none()
        && !reservation.integrity_hmac.is_empty()
        && next.identity_id == expected.identity_id
        && next.balance_tokens == expected.balance_tokens
        && next.reserved_tokens
            == expected
                .reserved_tokens
                .checked_add(reservation.reserved_tokens)
                .unwrap_or(-1)
        && unchanged_balance_counters(expected, next)
        && next.revision == expected.revision.saturating_add(1)
        && next.last_ledger_hmac == expected.last_ledger_hmac
}

fn valid_grant_transition(
    expected: &UserBalanceRecord,
    next: &UserBalanceRecord,
    ledger: &QuotaLedgerEntry,
) -> bool {
    valid_balance_shape(expected)
        && valid_balance_shape(next)
        && ledger.kind == "grant"
        && ledger.identity_id == expected.identity_id
        && ledger.amount_tokens > 0
        && ledger.uncached_input == 0
        && ledger.cached_input == 0
        && ledger.cache_write == 0
        && ledger.output_tokens == 0
        && ledger.uncovered_tokens == 0
        && ledger.raw_tokens == 0
        && ledger.billed_tokens == 0
        && ledger.multiplier_micros == 1_000_000
        && !ledger.reference_id.is_empty()
        && !ledger.description.is_empty()
        && ledger.protocol.is_empty()
        && ledger.model.is_empty()
        && ledger.api_key_id.is_none()
        && ledger.runner_id.is_none()
        && ledger.previous_entry_hmac == expected.last_ledger_hmac
        && !ledger.integrity_hmac.is_empty()
        && next.identity_id == expected.identity_id
        && next.balance_tokens
            == expected
                .balance_tokens
                .checked_add(ledger.amount_tokens)
                .unwrap_or(-1)
        && next.reserved_tokens == expected.reserved_tokens
        && next.granted_tokens
            == expected
                .granted_tokens
                .checked_add(ledger.amount_tokens)
                .unwrap_or(-1)
        && next.consumed_tokens == expected.consumed_tokens
        && next.request_count == expected.request_count
        && next.raw_tokens == expected.raw_tokens
        && next.billed_tokens == expected.billed_tokens
        && next.uncached_input == expected.uncached_input
        && next.cached_input == expected.cached_input
        && next.cache_write == expected.cache_write
        && next.output_tokens == expected.output_tokens
        && next.revision == expected.revision.saturating_add(1)
        && next.last_ledger_hmac == ledger.integrity_hmac
}

fn valid_identity_status_transition(expected: &IdentityRecord, next: &IdentityRecord) -> bool {
    expected.role == "member"
        && expected.status != "deleted"
        && matches!(next.status.as_str(), "active" | "disabled" | "deleted")
        && next.status != expected.status
        && next.id == expected.id
        && next.email == expected.email
        && next.display_name == expected.display_name
        && next.password_hash == expected.password_hash
        && next.role == expected.role
        && next.can_consume_model == expected.can_consume_model
        && next.password_change_required == expected.password_change_required
        && next.revision == expected.revision.saturating_add(1)
        && !next.integrity_hmac.is_empty()
        && next.created_at == expected.created_at
        && !next.updated_at.is_empty()
}

fn valid_release_transition(
    expected_balance: &UserBalanceRecord,
    next_balance: &UserBalanceRecord,
    expected_reservation: &QuotaReservationRecord,
    next_reservation: &QuotaReservationRecord,
) -> bool {
    valid_balance_shape(expected_balance)
        && valid_balance_shape(next_balance)
        && expected_reservation.status == "active"
        && next_reservation.status == "released"
        && same_reservation_identity(expected_reservation, next_reservation)
        && next_reservation.revision == expected_reservation.revision.saturating_add(1)
        && next_reservation.settled_at.is_some()
        && !next_reservation.integrity_hmac.is_empty()
        && next_balance.identity_id == expected_balance.identity_id
        && expected_reservation.identity_id == expected_balance.identity_id
        && next_balance.balance_tokens == expected_balance.balance_tokens
        && next_balance.reserved_tokens
            == expected_balance
                .reserved_tokens
                .checked_sub(expected_reservation.reserved_tokens)
                .unwrap_or(-1)
        && unchanged_balance_counters(expected_balance, next_balance)
        && next_balance.revision == expected_balance.revision.saturating_add(1)
        && next_balance.last_ledger_hmac == expected_balance.last_ledger_hmac
}

fn valid_failed_request_transition(
    expected_balance: &UserBalanceRecord,
    next_balance: &UserBalanceRecord,
    expected_reservation: &QuotaReservationRecord,
    next_reservation: &QuotaReservationRecord,
    ledger: &QuotaLedgerEntry,
) -> bool {
    valid_balance_shape(expected_balance)
        && valid_balance_shape(next_balance)
        && expected_reservation.status == "active"
        && next_reservation.status == "released"
        && same_reservation_identity(expected_reservation, next_reservation)
        && next_reservation.revision == expected_reservation.revision.saturating_add(1)
        && next_reservation.settled_at.is_some()
        && !next_reservation.integrity_hmac.is_empty()
        && ledger.kind == "usage_failed"
        && ledger.amount_tokens == 0
        && ledger.uncached_input == 0
        && ledger.cached_input == 0
        && ledger.cache_write == 0
        && ledger.output_tokens == 0
        && ledger.uncovered_tokens == 0
        && ledger.raw_tokens == 0
        && ledger.billed_tokens == 0
        && ledger.multiplier_micros == 1_000_000
        && !ledger.description.is_empty()
        && ledger.identity_id == expected_balance.identity_id
        && ledger.api_key_id.as_deref() == Some(expected_reservation.api_key_id.as_str())
        && ledger.reference_id == expected_reservation.request_id
        && ledger.previous_entry_hmac == expected_balance.last_ledger_hmac
        && !ledger.integrity_hmac.is_empty()
        && next_balance.identity_id == expected_balance.identity_id
        && expected_reservation.identity_id == expected_balance.identity_id
        && next_balance.balance_tokens == expected_balance.balance_tokens
        && next_balance.reserved_tokens
            == expected_balance
                .reserved_tokens
                .checked_sub(expected_reservation.reserved_tokens)
                .unwrap_or(-1)
        && unchanged_balance_counters(expected_balance, next_balance)
        && next_balance.revision == expected_balance.revision.saturating_add(1)
        && next_balance.last_ledger_hmac == ledger.integrity_hmac
}

fn valid_settlement_transition(
    expected_balance: &UserBalanceRecord,
    next_balance: &UserBalanceRecord,
    expected_reservation: &QuotaReservationRecord,
    next_reservation: &QuotaReservationRecord,
    ledger: &QuotaLedgerEntry,
) -> bool {
    let Some(charge) = ledger.amount_tokens.checked_neg() else {
        return false;
    };
    let Some(raw_tokens) = ledger
        .uncached_input
        .checked_add(ledger.cached_input)
        .and_then(|value| value.checked_add(ledger.cache_write))
        .and_then(|value| value.checked_add(ledger.output_tokens))
    else {
        return false;
    };
    let available_for_this = expected_balance.balance_tokens.checked_sub(
        expected_balance
            .reserved_tokens
            .checked_sub(expected_reservation.reserved_tokens)
            .unwrap_or(i64::MAX),
    );
    valid_balance_shape(expected_balance)
        && valid_balance_shape(next_balance)
        && expected_reservation.status == "active"
        && next_reservation.status == "settled"
        && same_reservation_identity(expected_reservation, next_reservation)
        && next_reservation.revision == expected_reservation.revision.saturating_add(1)
        && next_reservation.settled_at.is_some()
        && !next_reservation.integrity_hmac.is_empty()
        && ledger.kind == "usage"
        && !ledger.description.is_empty()
        && ledger.identity_id == expected_balance.identity_id
        && ledger.api_key_id.as_deref() == Some(expected_reservation.api_key_id.as_str())
        && ledger
            .runner_id
            .as_deref()
            .is_some_and(|value| !value.is_empty())
        && ledger.reference_id == expected_reservation.request_id
        && ledger.previous_entry_hmac == expected_balance.last_ledger_hmac
        && !ledger.integrity_hmac.is_empty()
        && ledger.multiplier_micros > 0
        && raw_tokens >= 0
        && ledger.raw_tokens == raw_tokens
        && ledger.billed_tokens >= charge
        && ledger.uncovered_tokens == ledger.billed_tokens - charge
        && available_for_this.is_some_and(|available| charge >= 0 && charge <= available)
        && next_balance.identity_id == expected_balance.identity_id
        && next_balance.balance_tokens == expected_balance.balance_tokens - charge
        && next_balance.reserved_tokens
            == expected_balance.reserved_tokens - expected_reservation.reserved_tokens
        && next_balance.granted_tokens == expected_balance.granted_tokens
        && next_balance.consumed_tokens
            == expected_balance
                .consumed_tokens
                .checked_add(charge)
                .unwrap_or(-1)
        && next_balance.request_count == expected_balance.request_count.checked_add(1).unwrap_or(-1)
        && next_balance.raw_tokens
            == expected_balance
                .raw_tokens
                .checked_add(ledger.raw_tokens)
                .unwrap_or(-1)
        && next_balance.billed_tokens
            == expected_balance
                .billed_tokens
                .checked_add(ledger.billed_tokens)
                .unwrap_or(-1)
        && next_balance.uncached_input
            == expected_balance
                .uncached_input
                .checked_add(ledger.uncached_input)
                .unwrap_or(-1)
        && next_balance.cached_input
            == expected_balance
                .cached_input
                .checked_add(ledger.cached_input)
                .unwrap_or(-1)
        && next_balance.cache_write
            == expected_balance
                .cache_write
                .checked_add(ledger.cache_write)
                .unwrap_or(-1)
        && next_balance.output_tokens
            == expected_balance
                .output_tokens
                .checked_add(ledger.output_tokens)
                .unwrap_or(-1)
        && next_balance.revision == expected_balance.revision.saturating_add(1)
        && next_balance.last_ledger_hmac == ledger.integrity_hmac
}

fn unchanged_balance_counters(expected: &UserBalanceRecord, next: &UserBalanceRecord) -> bool {
    next.granted_tokens == expected.granted_tokens
        && next.consumed_tokens == expected.consumed_tokens
        && next.request_count == expected.request_count
        && next.raw_tokens == expected.raw_tokens
        && next.billed_tokens == expected.billed_tokens
        && next.uncached_input == expected.uncached_input
        && next.cached_input == expected.cached_input
        && next.cache_write == expected.cache_write
        && next.output_tokens == expected.output_tokens
}

fn same_reservation_identity(
    expected: &QuotaReservationRecord,
    next: &QuotaReservationRecord,
) -> bool {
    next.id == expected.id
        && next.identity_id == expected.identity_id
        && next.api_key_id == expected.api_key_id
        && next.request_id == expected.request_id
        && next.reserved_tokens == expected.reserved_tokens
        && next.created_at == expected.created_at
        && next.expires_at == expected.expires_at
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn occupied_identity_ids_sqlite(connection: &Connection) -> Result<Vec<String>, rusqlite::Error> {
    let mut statement = connection.prepare(
        "SELECT id FROM identities
         WHERE status='active' AND can_consume_model=1
         ORDER BY id ASC",
    )?;
    statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect()
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn insert_initial_balance_sqlite(
    connection: &Connection,
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
    connection.execute(
        "INSERT INTO user_balances(
           identity_id,balance_tokens,reserved_tokens,granted_tokens,consumed_tokens,
           request_count,raw_tokens,billed_tokens,uncached_input,cached_input,cache_write,
           output_tokens,revision,last_ledger_hmac,integrity_hmac,updated_at
         ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        params![
            balance.identity_id,
            balance.balance_tokens,
            balance.reserved_tokens,
            balance.granted_tokens,
            balance.consumed_tokens,
            balance.request_count,
            balance.raw_tokens,
            balance.billed_tokens,
            balance.uncached_input,
            balance.cached_input,
            balance.cache_write,
            balance.output_tokens,
            i64::try_from(balance.revision).map_err(|_| StorageError::DatabaseNotEmpty)?,
            balance.last_ledger_hmac,
            balance.integrity_hmac,
            balance.updated_at,
        ],
    )?;
    Ok(())
}

pub fn encode_seat_registry(identity_ids: &[String]) -> Vec<u8> {
    identity_ids.join("\n").into_bytes()
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn authenticated_session_from_sqlite_row(
    row: &rusqlite::Row<'_>,
) -> Result<AuthenticatedSession, rusqlite::Error> {
    Ok(AuthenticatedSession {
        session: SessionRecord {
            id: row.get(0)?,
            identity_id: row.get(1)?,
            token_hash: row.get(2)?,
            expires_at: row.get(3)?,
            integrity_hmac: row.get(4)?,
            created_at: row.get(5)?,
        },
        identity: IdentityRecord {
            id: row.get(6)?,
            email: row.get(7)?,
            display_name: row.get(8)?,
            password_hash: row.get(9)?,
            role: row.get(10)?,
            status: row.get(11)?,
            can_consume_model: row.get::<_, u8>(12)? == 1,
            password_change_required: row.get::<_, u8>(13)? == 1,
            revision: row.get(14)?,
            integrity_hmac: row.get(15)?,
            created_at: row.get(16)?,
            updated_at: row.get(17)?,
        },
    })
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn api_key_from_sqlite_row(row: &rusqlite::Row<'_>) -> Result<ApiKeyRecord, rusqlite::Error> {
    Ok(ApiKeyRecord {
        id: row.get(0)?,
        identity_id: row.get(1)?,
        name: row.get(2)?,
        key_hash: row.get(3)?,
        key_prefix: row.get(4)?,
        status: row.get(5)?,
        revision: row.get(6)?,
        integrity_hmac: row.get(7)?,
        created_at: row.get(8)?,
        last_used_at: row.get(9)?,
    })
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn authorized_api_key_from_sqlite_row(
    row: &rusqlite::Row<'_>,
) -> Result<AuthorizedApiKey, rusqlite::Error> {
    Ok(AuthorizedApiKey {
        api_key: api_key_from_sqlite_row(row)?,
        identity: IdentityRecord {
            id: row.get(10)?,
            email: row.get(11)?,
            display_name: row.get(12)?,
            password_hash: row.get(13)?,
            role: row.get(14)?,
            status: row.get(15)?,
            can_consume_model: row.get::<_, u8>(16)? == 1,
            password_change_required: row.get::<_, u8>(17)? == 1,
            revision: row.get(18)?,
            integrity_hmac: row.get(19)?,
            created_at: row.get(20)?,
            updated_at: row.get(21)?,
        },
    })
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn model_from_sqlite_row(row: &rusqlite::Row<'_>) -> Result<ModelRecord, rusqlite::Error> {
    Ok(ModelRecord {
        id: row.get(0)?,
        public_name: row.get(1)?,
        display_name: row.get(2)?,
        enabled: row.get::<_, u8>(3)? != 0,
        discovered_at: row.get(4)?,
        created_at: row.get(5)?,
    })
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn gateway_route_from_sqlite_row(
    row: &rusqlite::Row<'_>,
) -> Result<GatewayRouteCandidate, rusqlite::Error> {
    Ok(GatewayRouteCandidate {
        model_id: row.get(0)?,
        public_model: row.get(1)?,
        upstream_model: row.get(2)?,
        account_id: row.get(3)?,
        provider: row.get(4)?,
        upstream_subject_id: row.get(5)?,
        last_success_runner_id: row.get(6)?,
        credential: EncryptedCredentialInstance {
            id: row.get(7)?,
            account_id: row.get(8)?,
            credential_identity_hmac: row.get(9)?,
            encrypted_payload: row.get(10)?,
            payload_nonce: row.get(11)?,
            wrapped_data_key: row.get(12)?,
            wrap_nonce: row.get(13)?,
            credential_revision: row.get(14)?,
            expires_at: row.get(15)?,
            status: row.get(16)?,
            last_refreshed_at: row.get(17)?,
            created_at: row.get(18)?,
            updated_at: row.get(19)?,
        },
    })
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn map_sqlite_credential_write_error(error: rusqlite::Error) -> StorageError {
    if error.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation) {
        StorageError::DuplicateCredentialIdentity
    } else {
        StorageError::Database(error)
    }
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn map_sqlite_identity_write_error(error: rusqlite::Error) -> StorageError {
    if error.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation) {
        StorageError::DuplicateIdentity
    } else {
        StorageError::Database(error)
    }
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn is_sqlite_unique_violation(error: &rusqlite::Error) -> bool {
    error.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation)
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn hex_key(key: &[u8; 32]) -> String {
    hex_bytes(key)
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[cfg(all(test, any(feature = "sqlite-dev", feature = "sqlcipher")))]
mod tests {
    mod quota_recovery;
    mod runner_connections;
    #[path = "runner_quota_transactions.rs"]
    mod runner_quota_transactions;
    use std::path::Path;

    #[cfg(feature = "sqlcipher")]
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    const NOW: &str = "2026-08-27T00:00:00.000Z";

    #[cfg(feature = "mariadb")]
    #[test]
    fn mariadb_column_decode_failure_is_distinct_from_connection_failure() {
        let error = StorageError::MariaDb(sqlx::Error::ColumnDecode {
            index: "revision".to_owned(),
            source: Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "BIGINT UNSIGNED cannot decode as i64",
            )),
        });
        assert_eq!(error.failure_kind(), StorageFailureKind::Decode);
        assert_eq!(
            StorageError::MariaDb(sqlx::Error::PoolClosed).failure_kind(),
            StorageFailureKind::Connection,
        );
    }

    fn initialize(path: &Path) -> SqlCipherStore {
        SqlCipherStore::initialize(path, &[0x5a; 32]).expect("initialize SQLCipher")
    }

    fn credential(id: &str, account_id: &str) -> EncryptedCredentialInstance {
        EncryptedCredentialInstance {
            id: id.to_owned(),
            account_id: account_id.to_owned(),
            credential_identity_hmac: format!("private-hmac-{id}"),
            encrypted_payload: vec![1, 2, 3],
            payload_nonce: vec![4; 24],
            wrapped_data_key: vec![5; 48],
            wrap_nonce: vec![6; 24],
            credential_revision: 0,
            expires_at: "2026-08-27T01:00:00.000Z".to_owned(),
            status: "active".to_owned(),
            last_refreshed_at: None,
            created_at: NOW.to_owned(),
            updated_at: NOW.to_owned(),
        }
    }

    fn identity(id: &str, email: &str, can_consume_model: bool) -> IdentityRecord {
        IdentityRecord {
            id: id.to_owned(),
            email: email.to_owned(),
            display_name: id.to_owned(),
            password_hash: "$argon2id$test".to_owned(),
            role: if can_consume_model { "member" } else { "admin" }.to_owned(),
            status: "active".to_owned(),
            can_consume_model,
            password_change_required: true,
            revision: 0,
            integrity_hmac: format!("identity-hmac-{id}"),
            created_at: NOW.to_owned(),
            updated_at: NOW.to_owned(),
        }
    }

    fn seat_state(identity_ids: &[&str], revision: u64) -> SecurityStateRecord {
        SecurityStateRecord {
            key: "member-seat-registry-v1".to_owned(),
            value: identity_ids.join("\n").into_bytes(),
            revision,
            mac: format!("test-seat-mac-{revision}").into_bytes(),
            updated_at: NOW.to_owned(),
        }
    }

    fn bootstrap_empty_seat_state(store: &SqlCipherStore) -> SecurityStateRecord {
        let state = seat_state(&[], 0);
        store
            .connection
            .execute(
                "INSERT INTO security_state(key,value,revision,mac,updated_at) VALUES(?,?,?,?,?)",
                params![
                    state.key,
                    state.value,
                    state.revision as i64,
                    state.mac,
                    state.updated_at,
                ],
            )
            .expect("bootstrap seat state");
        state
    }

    fn initial_balance(identity_id: &str) -> UserBalanceRecord {
        UserBalanceRecord {
            identity_id: identity_id.to_owned(),
            balance_tokens: 0,
            reserved_tokens: 0,
            granted_tokens: 0,
            consumed_tokens: 0,
            request_count: 0,
            raw_tokens: 0,
            billed_tokens: 0,
            uncached_input: 0,
            cached_input: 0,
            cache_write: 0,
            output_tokens: 0,
            revision: 0,
            last_ledger_hmac: String::new(),
            integrity_hmac: format!("balance-hmac-{identity_id}-0"),
            updated_at: NOW.to_owned(),
        }
    }

    fn audit_event(
        id: &str,
        sequence: u64,
        previous_event_hmac: &str,
        integrity_marker: char,
        action: &str,
        target_id: &str,
    ) -> AuditEventRecord {
        AuditEventRecord {
            id: id.to_owned(),
            sequence,
            actor_identity_id: None,
            actor_role: "system".to_owned(),
            action: action.to_owned(),
            target_type: "identity".to_owned(),
            target_id: Some(target_id.to_owned()),
            outcome: "succeeded".to_owned(),
            previous_event_hmac: previous_event_hmac.to_owned(),
            integrity_hmac: integrity_marker.to_string().repeat(43),
            created_at: NOW.to_owned(),
        }
    }

    fn insert_test_identity(store: &SqlCipherStore, record: &IdentityRecord) {
        store
            .connection
            .execute(
                "INSERT INTO identities(
                   id,email,display_name,password_hash,role,status,can_consume_model,
                   password_change_required,revision,integrity_hmac,created_at,updated_at
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    record.id,
                    record.email,
                    record.display_name,
                    record.password_hash,
                    record.role,
                    record.status,
                    record.can_consume_model,
                    record.password_change_required,
                    record.revision,
                    record.integrity_hmac,
                    record.created_at,
                    record.updated_at,
                ],
            )
            .expect("insert identity");
    }

    fn insert_test_balance(store: &SqlCipherStore, balance: &UserBalanceRecord) {
        store
            .connection
            .execute(
                "INSERT INTO user_balances(
                   identity_id,balance_tokens,reserved_tokens,granted_tokens,consumed_tokens,
                   request_count,raw_tokens,billed_tokens,uncached_input,cached_input,
                   cache_write,output_tokens,revision,last_ledger_hmac,integrity_hmac,updated_at
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    balance.identity_id,
                    balance.balance_tokens,
                    balance.reserved_tokens,
                    balance.granted_tokens,
                    balance.consumed_tokens,
                    balance.request_count,
                    balance.raw_tokens,
                    balance.billed_tokens,
                    balance.uncached_input,
                    balance.cached_input,
                    balance.cache_write,
                    balance.output_tokens,
                    i64::try_from(balance.revision).expect("balance revision"),
                    balance.last_ledger_hmac,
                    balance.integrity_hmac,
                    balance.updated_at,
                ],
            )
            .expect("insert balance");
    }

    #[test]
    fn voucher_redemption_atomically_updates_voucher_balance_and_ledger() {
        let directory = tempdir().expect("temp directory");
        let mut store = initialize(&directory.path().join("customer.db"));
        let admin = identity("identity_admin", "admin@example.com", false);
        let member = identity("identity_member", "member@example.com", true);
        insert_test_identity(&store, &admin);
        insert_test_identity(&store, &member);
        let expected_balance = initial_balance(&member.id);
        insert_test_balance(&store, &expected_balance);
        let voucher = VoucherRecord {
            id: "voucher_00000000000000000000000000000001".to_owned(),
            code_hash: "code-hash".to_owned(),
            code_prefix: "av_example".to_owned(),
            name: "trial".to_owned(),
            quota_tokens: 2_000,
            status: "active".to_owned(),
            max_redemptions: 1,
            redeemed_count: 0,
            expires_at: None,
            created_by: admin.id,
            revision: 0,
            integrity_hmac: "voucher-integrity-0".to_owned(),
            created_at: NOW.to_owned(),
        };
        store
            .insert_vouchers(&[(voucher.clone(), None)])
            .expect("insert voucher");
        let ledger = QuotaLedgerEntry {
            id: "ledger_voucher".to_owned(),
            identity_id: member.id.clone(),
            kind: "grant".to_owned(),
            amount_tokens: voucher.quota_tokens,
            uncached_input: 0,
            cached_input: 0,
            cache_write: 0,
            output_tokens: 0,
            uncovered_tokens: 0,
            raw_tokens: 0,
            billed_tokens: 0,
            multiplier_micros: 1_000_000,
            reference_id: format!("voucher:{}:{}", voucher.id, member.id),
            client_request_id: None,
            description: "voucher grant".to_owned(),
            protocol: String::new(),
            model: String::new(),
            requested_model: None,
            processing_tier: None,
            reasoning_effort: None,
            api_key_id: None,
            runner_id: None,
            previous_entry_hmac: expected_balance.last_ledger_hmac.clone(),
            integrity_hmac: "voucher-ledger-hmac".to_owned(),
            created_at: NOW.to_owned(),
        };
        let mut next_balance = expected_balance.clone();
        next_balance.balance_tokens = voucher.quota_tokens;
        next_balance.granted_tokens = voucher.quota_tokens;
        next_balance.revision = 1;
        next_balance.last_ledger_hmac = ledger.integrity_hmac.clone();
        next_balance.integrity_hmac = "voucher-balance-hmac".to_owned();
        let redemption = VoucherRedemptionRecord {
            id: "voucher_redemption_00000000000000000000000000000001".to_owned(),
            voucher_id: voucher.id.clone(),
            identity_id: member.id.clone(),
            amount_tokens: voucher.quota_tokens,
            ledger_entry_id: ledger.id.clone(),
            created_at: NOW.to_owned(),
        };
        let stale_audit = audit_event(
            "audit_voucher_stale",
            2,
            &"X".repeat(43),
            'Y',
            "voucher.redeem",
            &voucher.id,
        );
        assert_eq!(
            store
                .redeem_voucher_and_audit(
                    &voucher,
                    None,
                    &member.id,
                    NOW,
                    &expected_balance,
                    &next_balance,
                    &ledger,
                    &redemption,
                    "voucher-integrity-1",
                    0,
                    "",
                    &stale_audit,
                )
                .expect("reject stale voucher audit tail"),
            MutationWithAuditOutcome::AuditConflict
        );
        assert_eq!(
            store
                .voucher_by_code_hash(&voucher.code_hash)
                .expect("load rolled-back voucher"),
            Some(voucher.clone())
        );
        assert_eq!(
            store
                .quota_state_snapshot(&member.id)
                .expect("rolled-back quota snapshot")
                .expect("balance exists")
                .balance,
            expected_balance
        );
        assert!(store.voucher_redemptions(&voucher.id).unwrap().is_empty());
        let redeem_audit = audit_event(
            "audit_voucher_redeem",
            1,
            "",
            'V',
            "voucher.redeem",
            &voucher.id,
        );
        assert_eq!(
            store
                .redeem_voucher_and_audit(
                    &voucher,
                    None,
                    &member.id,
                    NOW,
                    &expected_balance,
                    &next_balance,
                    &ledger,
                    &redemption,
                    "voucher-integrity-1",
                    0,
                    "",
                    &redeem_audit,
                )
                .expect("redeem voucher with audit"),
            MutationWithAuditOutcome::Mutation(VoucherRedeemOutcome::Applied)
        );
        let stored_voucher = store
            .voucher_by_code_hash(&voucher.code_hash)
            .expect("load voucher")
            .expect("voucher exists");
        assert_eq!(stored_voucher.status, "redeemed");
        assert_eq!(stored_voucher.redeemed_count, 1);
        let snapshot = store
            .quota_state_snapshot(&member.id)
            .expect("quota snapshot")
            .expect("balance exists");
        assert_eq!(snapshot.balance.balance_tokens, 2_000);
        assert_eq!(snapshot.ledger_entries.len(), 1);
        assert_eq!(store.voucher_redemptions(&voucher.id).unwrap().len(), 1);
        assert_eq!(store.audit_events().unwrap(), vec![redeem_audit]);
    }

    #[test]
    fn quota_grant_and_audit_commit_or_rollback_together() {
        let directory = tempdir().expect("temp directory");
        let mut store = initialize(&directory.path().join("customer.db"));
        let member = identity("identity_member", "member@example.com", true);
        insert_test_identity(&store, &member);
        let expected_balance = initial_balance(&member.id);
        insert_test_balance(&store, &expected_balance);
        let ledger = QuotaLedgerEntry {
            id: "ledger_admin_grant".to_owned(),
            identity_id: member.id.clone(),
            kind: "grant".to_owned(),
            amount_tokens: 1_000,
            uncached_input: 0,
            cached_input: 0,
            cache_write: 0,
            output_tokens: 0,
            uncovered_tokens: 0,
            raw_tokens: 0,
            billed_tokens: 0,
            multiplier_micros: 1_000_000,
            reference_id: "admin_grant:test".to_owned(),
            client_request_id: None,
            description: "initial grant".to_owned(),
            protocol: String::new(),
            model: String::new(),
            requested_model: None,
            processing_tier: None,
            reasoning_effort: None,
            api_key_id: None,
            runner_id: None,
            previous_entry_hmac: String::new(),
            integrity_hmac: "ledger-integrity-1".to_owned(),
            created_at: NOW.to_owned(),
        };
        let mut next_balance = expected_balance.clone();
        next_balance.balance_tokens = 1_000;
        next_balance.granted_tokens = 1_000;
        next_balance.revision = 1;
        next_balance.last_ledger_hmac = ledger.integrity_hmac.clone();
        next_balance.integrity_hmac = "balance-integrity-1".to_owned();
        let stale_audit = audit_event(
            "audit_stale_grant",
            2,
            &"X".repeat(43),
            'Y',
            "member.quota.grant",
            &member.id,
        );
        assert_eq!(
            store
                .grant_quota_and_audit(
                    &expected_balance,
                    &next_balance,
                    &ledger,
                    1,
                    &"X".repeat(43),
                    &stale_audit,
                )
                .expect("reject stale grant audit tail"),
            MutationWithAuditOutcome::AuditConflict
        );
        let rolled_back = store
            .quota_state_snapshot(&member.id)
            .expect("rolled-back quota snapshot")
            .expect("balance exists");
        assert_eq!(rolled_back.balance, expected_balance);
        assert!(rolled_back.ledger_entries.is_empty());

        let audit = audit_event("audit_grant", 1, "", 'A', "member.quota.grant", &member.id);
        assert_eq!(
            store
                .grant_quota_and_audit(&expected_balance, &next_balance, &ledger, 0, "", &audit,)
                .expect("grant quota with audit"),
            MutationWithAuditOutcome::Mutation(QuotaMutationOutcome::Applied)
        );
        let applied = store
            .quota_state_snapshot(&member.id)
            .expect("applied quota snapshot")
            .expect("balance exists");
        assert_eq!(applied.balance, next_balance);
        assert_eq!(applied.ledger_entries, vec![ledger]);
        assert_eq!(store.audit_events().expect("grant audit"), vec![audit]);
    }

    #[test]
    fn mixed_quota_grant_is_atomic_across_tokens_images_and_audit() {
        let directory = tempdir().expect("temp directory");
        let mut store = initialize(&directory.path().join("customer.db"));
        let member = identity("identity_member", "member@example.com", true);
        insert_test_identity(&store, &member);
        let expected_balance = initial_balance(&member.id);
        insert_test_balance(&store, &expected_balance);
        let model = ModelRecord {
            id: "model_image".to_owned(),
            public_name: "image-test".to_owned(),
            display_name: "Image Test".to_owned(),
            enabled: true,
            discovered_at: Some(NOW.to_owned()),
            created_at: NOW.to_owned(),
        };
        store.upsert_model(&model).expect("image model");
        store
            .connection
            .execute(
                "INSERT INTO model_quota_units(model_id,quota_unit) VALUES(?,?)",
                params![model.id, "image"],
            )
            .expect("image quota unit");
        let ledger = QuotaLedgerEntry {
            id: "ledger_mixed_grant".to_owned(),
            identity_id: member.id.clone(),
            kind: "grant".to_owned(),
            amount_tokens: 1_000,
            uncached_input: 0,
            cached_input: 0,
            cache_write: 0,
            output_tokens: 0,
            uncovered_tokens: 0,
            raw_tokens: 0,
            billed_tokens: 0,
            multiplier_micros: 1_000_000,
            reference_id: "admin_grant:mixed".to_owned(),
            client_request_id: None,
            description: "mixed grant".to_owned(),
            protocol: String::new(),
            model: String::new(),
            requested_model: None,
            processing_tier: None,
            reasoning_effort: None,
            api_key_id: None,
            runner_id: None,
            previous_entry_hmac: String::new(),
            integrity_hmac: "token-hmac".to_owned(),
            created_at: NOW.to_owned(),
        };
        let mut next_balance = expected_balance.clone();
        next_balance.balance_tokens = 1_000;
        next_balance.granted_tokens = 1_000;
        next_balance.revision = 1;
        next_balance.last_ledger_hmac = ledger.integrity_hmac.clone();
        next_balance.integrity_hmac = "balance-hmac".to_owned();
        let token = QuotaBatchTokenWrite {
            expected_balance: expected_balance.clone(),
            next_balance: next_balance.clone(),
            ledger,
        };
        let image_ledger = ImageLedgerRecord {
            id: "image_adjust_mixed".to_owned(),
            identity_id: member.id.clone(),
            public_model_id: model.id.clone(),
            reservation_id: None,
            child_id: None,
            kind: "adjust".to_owned(),
            amount_images: 3,
            produced_images: 0,
            delivery_state: "none".to_owned(),
            actor: member.id.clone(),
            reason: "mixed grant".to_owned(),
            previous_hmac: String::new(),
            integrity_hmac: "image-ledger-hmac".to_owned(),
            created_at: NOW.to_owned(),
        };
        let image_balance = ImageBalanceRecord {
            identity_id: member.id.clone(),
            public_model_id: model.id.clone(),
            available_images: 3,
            reserved_images: 0,
            consumed_images: 0,
            revision: 0,
            last_ledger_hmac: image_ledger.integrity_hmac.clone(),
            integrity_hmac: "image-balance-hmac".to_owned(),
        };
        let image = QuotaBatchImageWrite {
            expected_balance: None,
            next_balance: image_balance.clone(),
            ledger: image_ledger,
        };
        let stale_audit = audit_event(
            "audit_stale_mixed",
            2,
            &"X".repeat(43),
            'Y',
            "member.quota.grant",
            &member.id,
        );
        assert_eq!(
            store
                .grant_quota_batch_and_audit(
                    Some(&token),
                    std::slice::from_ref(&image),
                    1,
                    &"X".repeat(43),
                    &stale_audit,
                )
                .expect("stale audit"),
            MutationWithAuditOutcome::AuditConflict
        );
        assert_eq!(
            store
                .quota_state_snapshot(&member.id)
                .unwrap()
                .unwrap()
                .balance,
            expected_balance
        );
        assert!(
            store
                .image_balance(&member.id, &model.id)
                .unwrap()
                .is_none()
        );
        let audit = audit_event("audit_mixed", 1, "", 'M', "member.quota.grant", &member.id);
        assert_eq!(
            store
                .grant_quota_batch_and_audit(
                    Some(&token),
                    std::slice::from_ref(&image),
                    0,
                    "",
                    &audit,
                )
                .expect("mixed grant"),
            MutationWithAuditOutcome::Mutation(QuotaMutationOutcome::Applied)
        );
        assert_eq!(
            store
                .quota_state_snapshot(&member.id)
                .unwrap()
                .unwrap()
                .balance,
            next_balance
        );
        assert_eq!(
            store.image_balance(&member.id, &model.id).unwrap(),
            Some(image_balance)
        );
        assert_eq!(store.audit_events().unwrap(), vec![audit]);
    }

    #[test]
    fn member_can_update_and_withdraw_pending_quota_request_atomically() {
        let directory = tempdir().expect("temp directory");
        let mut store = initialize(&directory.path().join("customer.db"));
        let member = identity("identity_member", "member@example.com", true);
        insert_test_identity(&store, &member);
        let request = QuotaRequestRecord {
            id: "quota_request_00000000000000000000000000000001".to_owned(),
            identity_id: member.id.clone(),
            amount_nanos: 1_000,
            reason: "initial project quota".to_owned(),
            status: "pending".to_owned(),
            review_note: String::new(),
            reviewed_by: None,
            reviewed_at: None,
            revision: 0,
            integrity_hmac: "quota-request-integrity-0".to_owned(),
            created_at: NOW.to_owned(),
        };
        assert_eq!(
            store
                .insert_quota_request(&request)
                .expect("insert pending request"),
            QuotaRequestInsertOutcome::Inserted
        );

        let mut updated = request.clone();
        updated.amount_nanos = 2_000;
        updated.reason = "updated project quota".to_owned();
        updated.revision = 1;
        updated.integrity_hmac = "quota-request-integrity-1".to_owned();
        let update_audit = audit_event(
            "audit_quota_request_update",
            1,
            "",
            'A',
            "quota_request.update",
            &request.id,
        );
        assert_eq!(
            store
                .change_pending_quota_request_and_audit(
                    &request,
                    Some(&updated),
                    0,
                    "",
                    &update_audit,
                )
                .expect("update pending request"),
            AuditedMutationOutcome::Applied
        );
        assert_eq!(
            store
                .quota_request_by_id(&request.id)
                .expect("load updated request"),
            Some(updated.clone())
        );

        let withdraw_audit = audit_event(
            "audit_quota_request_withdraw",
            2,
            &update_audit.integrity_hmac,
            'B',
            "quota_request.withdraw",
            &request.id,
        );
        assert_eq!(
            store
                .change_pending_quota_request_and_audit(
                    &updated,
                    None,
                    1,
                    &update_audit.integrity_hmac,
                    &withdraw_audit,
                )
                .expect("withdraw pending request"),
            AuditedMutationOutcome::Applied
        );
        assert!(
            store
                .quota_request_by_id(&request.id)
                .expect("load withdrawn request")
                .is_none()
        );
        assert_eq!(
            store.audit_events().expect("member request audit"),
            vec![update_audit, withdraw_audit]
        );
    }

    #[test]
    fn quota_request_approval_and_grant_commit_in_one_transaction() {
        let directory = tempdir().expect("temp directory");
        let mut store = initialize(&directory.path().join("customer.db"));
        for record in [
            identity("identity_admin", "admin@example.com", false),
            identity("identity_member", "member@example.com", true),
        ] {
            insert_test_identity(&store, &record);
        }
        let expected_balance = MoneyBalanceRecord {
            identity_id: "identity_member".to_owned(),
            currency: "CNY".to_owned(),
            balance_nanos: 0,
            credited_nanos: 0,
            debited_nanos: 0,
            revision: 0,
            last_ledger_hmac: String::new(),
            integrity_hmac: "money-balance-hmac-0".to_owned(),
            updated_at: NOW.to_owned(),
        };
        assert!(store.initialize_money_balance(&expected_balance).unwrap());
        let request = QuotaRequestRecord {
            id: "quota_request_00000000000000000000000000000001".to_owned(),
            identity_id: "identity_member".to_owned(),
            amount_nanos: 1_000,
            reason: "project quota".to_owned(),
            status: "pending".to_owned(),
            review_note: String::new(),
            reviewed_by: None,
            reviewed_at: None,
            revision: 0,
            integrity_hmac: "quota-request-integrity-0".to_owned(),
            created_at: NOW.to_owned(),
        };
        let stale_create_audit = audit_event(
            "audit_stale_quota_request_create",
            2,
            &"X".repeat(43),
            'Y',
            "quota_request.create",
            &request.id,
        );
        assert_eq!(
            store
                .insert_quota_request_and_audit(&request, 1, &"X".repeat(43), &stale_create_audit,)
                .expect("reject stale quota request audit tail"),
            MutationWithAuditOutcome::AuditConflict
        );
        assert!(
            store
                .quota_request_by_id(&request.id)
                .expect("load rolled-back request")
                .is_none()
        );
        let create_audit = audit_event(
            "audit_quota_request_create",
            1,
            "",
            'A',
            "quota_request.create",
            &request.id,
        );
        assert_eq!(
            store
                .insert_quota_request_and_audit(&request, 0, "", &create_audit)
                .expect("insert request with audit"),
            MutationWithAuditOutcome::Mutation(QuotaRequestInsertOutcome::Inserted)
        );
        let mut ledger = MoneyLedgerEntry {
            id: "money_quota_request".to_owned(),
            identity_id: request.identity_id.clone(),
            currency: "CNY".to_owned(),
            kind: "grant".to_owned(),
            amount_nanos: request.amount_nanos,
            balance_revision: 1,
            reference_id: format!("money-request:{}", request.id),
            billing_status: "not_applicable".to_owned(),
            details_json: "{}".to_owned(),
            previous_entry_hmac: expected_balance.last_ledger_hmac.clone(),
            integrity_hmac: "money-ledger-hmac".to_owned(),
            created_at: NOW.to_owned(),
        };
        let mut next_balance = expected_balance.clone();
        next_balance.balance_nanos = request.amount_nanos;
        next_balance.credited_nanos = request.amount_nanos;
        next_balance.revision = 1;
        next_balance.last_ledger_hmac = ledger.integrity_hmac.clone();
        next_balance.integrity_hmac = "money-balance-hmac-1".to_owned();
        let stale_audit = audit_event(
            "audit_stale_review",
            2,
            &"X".repeat(43),
            'Y',
            "quota_request.review",
            &request.id,
        );
        assert_eq!(
            store
                .review_quota_request_and_audit(
                    &request,
                    "approved",
                    "approved",
                    "identity_admin",
                    NOW,
                    "quota-request-integrity-1",
                    Some((&expected_balance, &next_balance, &ledger)),
                    1,
                    &"X".repeat(43),
                    &stale_audit,
                )
                .expect("reject stale review audit tail"),
            MutationWithAuditOutcome::AuditConflict
        );
        assert_eq!(
            store
                .quota_request_by_id(&request.id)
                .expect("load rolled-back request")
                .expect("request exists")
                .status,
            "pending"
        );
        assert_eq!(
            store
                .money_state_snapshot("identity_member")
                .expect("rolled-back quota snapshot")
                .expect("balance exists")
                .balance,
            expected_balance
        );
        let review_audit = audit_event(
            "audit_review",
            2,
            &"A".repeat(43),
            'B',
            "quota_request.review",
            &request.id,
        );
        assert_eq!(
            store
                .review_quota_request_and_audit(
                    &request,
                    "approved",
                    "approved",
                    "identity_admin",
                    NOW,
                    "quota-request-integrity-1",
                    Some((&expected_balance, &next_balance, &ledger)),
                    1,
                    &"A".repeat(43),
                    &review_audit,
                )
                .expect("approve request with audit"),
            MutationWithAuditOutcome::Mutation(QuotaRequestReviewOutcome::Applied)
        );
        assert_eq!(
            store
                .review_quota_request(
                    &request,
                    "approved",
                    "duplicate",
                    "identity_admin",
                    NOW,
                    "quota-request-integrity-1",
                    Some((&expected_balance, &next_balance, &ledger)),
                )
                .expect("repeat approval"),
            QuotaRequestReviewOutcome::Conflict
        );
        ledger.integrity_hmac.clear();
        let stored = store
            .quota_request_by_id(&request.id)
            .expect("load request")
            .expect("request exists");
        assert_eq!(stored.status, "approved");
        let snapshot = store
            .money_state_snapshot("identity_member")
            .expect("quota snapshot")
            .expect("balance exists");
        assert_eq!(snapshot.balance.balance_nanos, 1_000);
        assert_eq!(snapshot.ledger_entries.len(), 1);
    }

    #[cfg(feature = "sqlcipher")]
    #[test]
    fn initializes_an_encrypted_final_schema() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("customer.db");
        let store = initialize(&path);
        assert_eq!(
            store.schema_version().expect("schema version"),
            CURRENT_SCHEMA_VERSION
        );
        let enhanced_memory_security: String = store
            .connection
            .query_row("PRAGMA cipher_memory_security", [], |row| row.get(0))
            .expect("read enhanced memory security setting");
        assert_eq!(enhanced_memory_security, "0");
        drop(store);

        let bytes = fs::read(&path).expect("read encrypted database");
        assert!(!bytes.starts_with(b"SQLite format 3\0"));
        assert!(SqlCipherStore::initialize(&path, &[0x5a; 32]).is_ok());
        assert!(SqlCipherStore::open(&path, &[0x5a; 32]).is_ok());
        assert!(SqlCipherStore::open(&path, &[0x6b; 32]).is_err());
    }

    #[cfg(feature = "sqlite-dev")]
    #[test]
    fn database_initialization_is_idempotent_without_replacing_data() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("customer.db");
        let store = initialize(&path);
        store
            .connection
            .execute(
                "INSERT INTO runtime_settings(key,value,revision,integrity_hmac,updated_at) VALUES(?,?,?,?,?)",
                params!["retry-test", "preserved", 0, "mac", NOW],
            )
            .expect("insert preserved row");
        drop(store);

        let retried =
            SqlCipherStore::initialize(&path, &[0x5a; 32]).expect("retry database initialization");
        assert_eq!(
            retried
                .runtime_setting("retry-test")
                .expect("read preserved row")
                .expect("preserved row exists")
                .value,
            "preserved"
        );
    }

    #[test]
    fn baseline_reuses_only_a_deleted_identity_email() {
        let directory = tempdir().expect("temp directory");
        let migrated = initialize(&directory.path().join("customer.db"));
        let mut deleted = identity("identity_deleted", "reuse@example.com", true);
        deleted.status = "deleted".to_owned();
        insert_test_identity(&migrated, &deleted);
        let replacement = identity("identity_replacement", "reuse@example.com", true);
        insert_test_identity(&migrated, &replacement);
        let duplicate = identity("identity_duplicate", "reuse@example.com", true);
        assert!(
            migrated
                .connection
                .execute(
                    "INSERT INTO identities(
                       id,email,display_name,password_hash,role,status,can_consume_model,
                       password_change_required,revision,integrity_hmac,created_at,updated_at
                     ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
                    params![
                        duplicate.id,
                        duplicate.email,
                        duplicate.display_name,
                        duplicate.password_hash,
                        duplicate.role,
                        duplicate.status,
                        duplicate.can_consume_model,
                        duplicate.password_change_required,
                        duplicate.revision,
                        duplicate.integrity_hmac,
                        duplicate.created_at,
                        duplicate.updated_at,
                    ],
                )
                .is_err()
        );
        assert_eq!(
            migrated
                .identity_by_email("reuse@example.com")
                .expect("look up reusable email")
                .expect("replacement identity")
                .id,
            replacement.id
        );
    }

    #[test]
    fn startup_applies_each_missing_embedded_migration_once() {
        const TEST_MIGRATION: &str = "CREATE TABLE migration_probe(id INTEGER PRIMARY KEY) STRICT;";
        let migrations = [
            SQLCIPHER_MIGRATIONS[0],
            EmbeddedMigration {
                version: 2,
                name: "migration-probe",
                sql: TEST_MIGRATION,
                compatibility: MigrationCompatibility::RollingUpgradeSafe,
                online_sql: None,
                recovery_query: None,
            },
        ];
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("customer.db");
        let store = SqlCipherStore::connect(&path, &[0x5a; 32]).expect("connect database");
        store
            .apply_migrations(&migrations)
            .expect("apply migrations");
        store
            .apply_migrations(&migrations)
            .expect("reapply migrations");
        assert_eq!(store.schema_version().expect("schema version"), 2);
        assert!(store.has_table("migration_probe").expect("probe table"));
        drop(store);

        let older_application =
            SqlCipherStore::connect(&path, &[0x5a; 32]).expect("open newer compatible schema");
        older_application
            .apply_migrations(SQLCIPHER_MIGRATIONS)
            .expect("older application verifies known migrations");
        assert_eq!(
            older_application.schema_version().expect("future version"),
            2
        );
        let written_by_older = identity(
            "identity-written-by-older-app",
            "older-app@example.com",
            false,
        );
        insert_test_identity(&older_application, &written_by_older);
        assert_eq!(
            older_application
                .identity_by_email(&written_by_older.email)
                .expect("older application read")
                .expect("identity written by older application")
                .id,
            written_by_older.id
        );
        drop(older_application);

        let newer_application =
            SqlCipherStore::connect(&path, &[0x5a; 32]).expect("reopen newer application");
        newer_application
            .apply_migrations(&migrations)
            .expect("newer application rechecks migrations");
        assert_eq!(
            newer_application
                .identity_by_email(&written_by_older.email)
                .expect("newer application read")
                .expect("identity remains visible to newer application")
                .id,
            written_by_older.id
        );
    }

    #[test]
    fn migration_sequence_requires_explicit_rolling_upgrade_compatibility() {
        let invalid = [
            SQLCIPHER_MIGRATIONS[0],
            EmbeddedMigration {
                version: 2,
                name: "unclassified-migration",
                sql: "CREATE TABLE migration_probe(id INTEGER PRIMARY KEY) STRICT;",
                compatibility: MigrationCompatibility::Baseline,
                online_sql: None,
                recovery_query: None,
            },
        ];
        assert!(matches!(
            validate_migration_sequence(&invalid),
            Err(StorageError::MigrationIntegrity)
        ));
    }

    #[test]
    fn startup_rejects_a_changed_applied_migration() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("customer.db");
        let store = initialize(&path);
        store
            .connection
            .execute(
                "UPDATE schema_migrations SET checksum_sha256=? WHERE version=1",
                ["0".repeat(64)],
            )
            .expect("change checksum");
        drop(store);

        assert!(matches!(
            SqlCipherStore::open(&path, &[0x5a; 32]),
            Err(StorageError::UnsupportedSchema { version: 1 })
        ));
    }

    #[test]
    fn counts_consuming_identities_including_admin_roles() {
        let directory = tempdir().expect("temp directory");
        let store = initialize(&directory.path().join("customer.db"));
        for (id, role, can_consume) in [
            ("identity_owner", "owner", 0),
            ("identity_admin", "admin", 1),
            ("identity_member", "member", 1),
        ] {
            store.connection.execute(
                "INSERT INTO identities(id,email,display_name,password_hash,role,can_consume_model,integrity_hmac,created_at,updated_at)
                 VALUES(?,?,?,?,?,?,?,?,?)",
                params![id, format!("{id}@example.com"), id, "hash", role, can_consume, format!("mac-{id}"), NOW, NOW],
            ).expect("insert identity");
        }
        assert_eq!(store.occupied_seats().expect("occupied seats"), 2);
    }

    #[test]
    fn identity_creation_serializes_the_seat_gate_and_creates_balance_state() {
        let directory = tempdir().expect("temp directory");
        let mut store = initialize(&directory.path().join("customer.db"));
        let empty = bootstrap_empty_seat_state(&store);
        let one_member = seat_state(&["identity_1"], 1);
        assert_eq!(
            store
                .create_identity_with_seat_limit(
                    &identity("identity_1", "one@example.com", true),
                    1,
                    &empty,
                    &one_member,
                    &initial_balance("identity_1"),
                )
                .expect("create first member"),
            IdentityCreateOutcome::Created
        );
        assert_eq!(
            store
                .create_identity_with_seat_limit(
                    &identity("identity_2", "two@example.com", true),
                    1,
                    &one_member,
                    &seat_state(&["identity_1", "identity_2"], 2),
                    &initial_balance("identity_2"),
                )
                .expect("reject second member"),
            IdentityCreateOutcome::SeatLimitReached
        );
        assert_eq!(
            store
                .create_identity_with_seat_limit(
                    &identity("identity_3", "admin@example.com", false),
                    1,
                    &one_member,
                    &seat_state(&["identity_1"], 2),
                    &initial_balance("identity_3"),
                )
                .expect("create non-consuming admin"),
            IdentityCreateOutcome::Created
        );
        assert_eq!(store.occupied_seats().expect("occupied seats"), 1);
        assert_eq!(
            store
                .identity_by_email("one@example.com")
                .expect("load identity")
                .expect("identity exists")
                .integrity_hmac,
            "identity-hmac-identity_1"
        );
        let balances = store
            .connection
            .query_row("SELECT count(*) FROM user_balances", [], |row| {
                row.get::<_, u32>(0)
            })
            .expect("count balances");
        assert_eq!(balances, 2);
    }

    #[test]
    fn batch_identity_creation_is_atomic_and_advances_the_seat_registry_once() {
        let directory = tempdir().expect("temp directory");
        let mut store = initialize(&directory.path().join("customer.db"));
        let empty = bootstrap_empty_seat_state(&store);
        let identities = vec![
            identity("identity_1", "one@example.com", true),
            identity("identity_2", "two@example.com", true),
        ];
        let balances = vec![initial_balance("identity_1"), initial_balance("identity_2")];
        let two_members = seat_state(&["identity_1", "identity_2"], 1);
        assert_eq!(
            store
                .create_identities_with_seat_limit(&identities, 1, &empty, &two_members, &balances,)
                .expect("reject whole over-limit batch"),
            IdentityCreateOutcome::SeatLimitReached
        );
        assert_eq!(store.occupied_seats().expect("no partial seats"), 0);
        assert_eq!(
            store
                .connection
                .query_row("SELECT count(*) FROM identities", [], |row| row
                    .get::<_, u32>(0))
                .expect("count identities"),
            0
        );
        assert_eq!(
            store
                .create_identities_with_seat_limit(&identities, 2, &empty, &two_members, &balances,)
                .expect("create whole batch"),
            IdentityCreateOutcome::Created
        );
        let snapshot = store
            .seat_registry_snapshot("member-seat-registry-v1")
            .expect("load seat registry");
        assert_eq!(snapshot.state.expect("seat state").revision, 1);
        assert_eq!(
            snapshot.occupied_identity_ids,
            vec!["identity_1".to_owned(), "identity_2".to_owned()]
        );
        assert_eq!(
            store
                .connection
                .query_row("SELECT count(*) FROM user_balances", [], |row| row
                    .get::<_, u32>(0))
                .expect("count balances"),
            2
        );
    }

    #[test]
    fn first_owner_initialization_and_session_lookup_are_fail_closed() {
        let directory = tempdir().expect("temp directory");
        let mut store = initialize(&directory.path().join("customer.db"));
        let owner = identity("identity_owner", "owner@example.com", false);
        let initial_seat_state = seat_state(&[], 0);
        assert_eq!(
            store
                .initialize_first_owner(&owner, &initial_seat_state, &initial_balance(&owner.id),)
                .expect("initialize owner"),
            IdentityInitializeOutcome::Created
        );
        assert_eq!(
            store
                .initialize_first_owner(
                    &identity("identity_other", "other@example.com", false,),
                    &initial_seat_state,
                    &initial_balance("identity_other"),
                )
                .expect("reject second owner"),
            IdentityInitializeOutcome::AlreadyInitialized
        );
        let session = SessionRecord {
            id: "session_001".to_owned(),
            identity_id: owner.id.clone(),
            token_hash: "A".repeat(43),
            expires_at: "2026-09-04T00:00:00.000Z".to_owned(),
            integrity_hmac: "session-integrity-tag".to_owned(),
            created_at: NOW.to_owned(),
        };
        store.insert_session(&session).expect("insert session");
        let authenticated = store
            .authenticated_session_by_token_hash(&session.token_hash, "2026-08-28T00:00:00.000Z")
            .expect("load active session")
            .expect("active session exists");
        assert_eq!(authenticated.session, session);
        assert_eq!(authenticated.identity.id, owner.id);
        assert!(
            store
                .authenticated_session_by_token_hash(
                    &authenticated.session.token_hash,
                    "2026-09-05T00:00:00.000Z",
                )
                .expect("expired lookup")
                .is_none()
        );
        assert!(
            store
                .delete_session_by_token_hash(&authenticated.session.token_hash)
                .expect("delete session")
        );
        let replacement_session = SessionRecord {
            id: "session_002".to_owned(),
            token_hash: "B".repeat(43),
            ..session
        };
        store
            .insert_session(&replacement_session)
            .expect("insert replacement session");
        let stale_password_audit = audit_event(
            "audit_stale_password",
            2,
            &"X".repeat(43),
            'Y',
            "identity.password.update",
            &owner.id,
        );
        assert_eq!(
            store
                .update_identity_password_and_audit(
                    &owner.id,
                    0,
                    "$argon2id$updated",
                    false,
                    "identity-hmac-updated",
                    "2026-08-28T00:01:00.000Z",
                    1,
                    &"X".repeat(43),
                    &stale_password_audit,
                )
                .expect("reject stale password audit tail"),
            AuditedMutationOutcome::AuditConflict
        );
        assert!(
            store
                .authenticated_session_by_token_hash(
                    &replacement_session.token_hash,
                    "2026-08-28T00:02:00.000Z",
                )
                .expect("session preserved after audit rollback")
                .is_some()
        );
        let password_audit = audit_event(
            "audit_password",
            1,
            "",
            'A',
            "identity.password.update",
            &owner.id,
        );
        assert_eq!(
            store
                .update_identity_password_and_audit(
                    &owner.id,
                    0,
                    "$argon2id$updated",
                    false,
                    "identity-hmac-updated",
                    "2026-08-28T00:01:00.000Z",
                    0,
                    "",
                    &password_audit,
                )
                .expect("update password with audit"),
            AuditedMutationOutcome::Applied
        );
        assert!(
            store
                .authenticated_session_by_token_hash(
                    &replacement_session.token_hash,
                    "2026-08-28T00:02:00.000Z",
                )
                .expect("session revoked by password update")
                .is_none()
        );
        let stale_revision_audit = audit_event(
            "audit_stale_revision",
            2,
            &"A".repeat(43),
            'B',
            "identity.password.update",
            &owner.id,
        );
        assert_eq!(
            store
                .update_identity_password_and_audit(
                    &owner.id,
                    0,
                    "$argon2id$stale",
                    false,
                    "stale-integrity",
                    "2026-08-28T00:02:00.000Z",
                    1,
                    &"A".repeat(43),
                    &stale_revision_audit,
                )
                .expect("reject stale password update"),
            AuditedMutationOutcome::MutationConflict
        );
    }

    #[test]
    fn api_key_lookup_is_owner_scoped_and_status_update_uses_revision_cas() {
        let directory = tempdir().expect("temp directory");
        let mut store = initialize(&directory.path().join("customer.db"));
        let member = identity("identity_member", "member@example.com", true);
        let empty = bootstrap_empty_seat_state(&store);
        assert_eq!(
            store
                .create_identity_with_seat_limit(
                    &member,
                    1,
                    &empty,
                    &seat_state(&["identity_member"], 1),
                    &initial_balance("identity_member"),
                )
                .expect("create member"),
            IdentityCreateOutcome::Created
        );
        let api_key = ApiKeyRecord {
            id: "key_00000000000000000000000000000001".to_owned(),
            identity_id: member.id.clone(),
            name: "desktop agent".to_owned(),
            key_hash: "K".repeat(43),
            key_prefix: "ask_example".to_owned(),
            status: "active".to_owned(),
            revision: 0,
            integrity_hmac: "api-key-integrity-v0".to_owned(),
            created_at: NOW.to_owned(),
            last_used_at: None,
        };
        let stale_create_audit = audit_event(
            "audit_stale_api_key_create",
            2,
            &"X".repeat(43),
            'Y',
            "api_key.create",
            &api_key.id,
        );
        assert_eq!(
            store
                .insert_api_key_unchecked_and_audit(
                    &api_key,
                    1,
                    &"X".repeat(43),
                    &stale_create_audit,
                )
                .expect("reject stale API key create audit tail"),
            AuditedMutationOutcome::AuditConflict
        );
        assert!(
            store
                .api_key_for_identity(&api_key.id, &member.id)
                .expect("load rolled-back API key")
                .is_none()
        );
        let create_audit = audit_event(
            "audit_api_key_create",
            1,
            "",
            'A',
            "api_key.create",
            &api_key.id,
        );
        assert_eq!(
            store
                .insert_api_key_unchecked_and_audit(&api_key, 0, "", &create_audit)
                .expect("insert API key with audit"),
            AuditedMutationOutcome::Applied
        );

        assert_eq!(
            store
                .api_keys_for_identity(&member.id)
                .expect("list API keys"),
            vec![api_key.clone()]
        );
        assert!(
            store
                .api_key_for_identity(&api_key.id, "identity_other")
                .expect("owner-scoped lookup")
                .is_none()
        );
        let authorized = store
            .authorized_api_key_by_hash(&api_key.key_hash)
            .expect("authorized lookup")
            .expect("active API key");
        assert_eq!(authorized.api_key, api_key);
        assert_eq!(authorized.identity.id, member.id);

        let stale_revoke_audit = audit_event(
            "audit_stale_api_key_revoke",
            1,
            "",
            'Z',
            "api_key.revoke",
            &api_key.id,
        );
        assert_eq!(
            store
                .update_api_key_status_and_audit(
                    &authorized.api_key.id,
                    &authorized.identity.id,
                    0,
                    "revoked",
                    "api-key-integrity-v1",
                    0,
                    "",
                    &stale_revoke_audit,
                )
                .expect("reject stale API key revoke audit tail"),
            AuditedMutationOutcome::AuditConflict
        );
        assert!(
            store
                .authorized_api_key_by_hash(&authorized.api_key.key_hash)
                .expect("active lookup after rollback")
                .is_some()
        );
        let revoke_audit = audit_event(
            "audit_api_key_revoke",
            2,
            &"A".repeat(43),
            'B',
            "api_key.revoke",
            &api_key.id,
        );
        assert_eq!(
            store
                .update_api_key_status_and_audit(
                    &authorized.api_key.id,
                    &authorized.identity.id,
                    0,
                    "revoked",
                    "api-key-integrity-v1",
                    1,
                    &"A".repeat(43),
                    &revoke_audit,
                )
                .expect("revoke API key with audit"),
            AuditedMutationOutcome::Applied
        );
        let stale_revision_audit = audit_event(
            "audit_api_key_stale_revision",
            3,
            &"B".repeat(43),
            'C',
            "api_key.revoke",
            &api_key.id,
        );
        assert_eq!(
            store
                .update_api_key_status_and_audit(
                    &authorized.api_key.id,
                    &authorized.identity.id,
                    0,
                    "active",
                    "stale-integrity",
                    2,
                    &"B".repeat(43),
                    &stale_revision_audit,
                )
                .expect("reject stale API key update"),
            AuditedMutationOutcome::MutationConflict
        );
        assert!(
            store
                .authorized_api_key_by_hash(&authorized.api_key.key_hash)
                .expect("revoked lookup")
                .is_none()
        );
        let revoked = store
            .api_key_for_identity(&authorized.api_key.id, &authorized.identity.id)
            .expect("load revoked API key")
            .expect("revoked API key exists");
        assert_eq!(revoked.status, "revoked");
        assert_eq!(revoked.revision, 1);
        assert_eq!(revoked.integrity_hmac, "api-key-integrity-v1");
    }

    #[test]
    fn active_api_key_limit_is_atomic_and_revoke_releases_capacity() {
        let directory = tempdir().expect("temp directory");
        let mut store = initialize(&directory.path().join("customer.db"));
        let member = identity("identity_member", "member@example.com", true);
        let empty = bootstrap_empty_seat_state(&store);
        assert_eq!(
            store
                .create_identity_with_seat_limit(
                    &member,
                    1,
                    &empty,
                    &seat_state(&["identity_member"], 1),
                    &initial_balance("identity_member"),
                )
                .expect("create member"),
            IdentityCreateOutcome::Created
        );
        let key = |suffix: &str| ApiKeyRecord {
            id: format!("key_{suffix:0>32}"),
            identity_id: member.id.clone(),
            name: format!("key {suffix}"),
            key_hash: format!("{suffix:K<43}"),
            key_prefix: format!("ask_{suffix}"),
            status: "active".to_owned(),
            revision: 0,
            integrity_hmac: format!("api-key-{suffix}"),
            created_at: NOW.to_owned(),
            last_used_at: None,
        };
        let first = key("one");
        let first_audit = audit_event("audit_key_one", 1, "", 'A', "api_key.create", &first.id);
        assert_eq!(
            store
                .insert_api_key_with_limit_and_audit(&first, Some(1), 0, "", &first_audit)
                .expect("create first key"),
            MutationWithAuditOutcome::Mutation(ResourceCreateOutcome::Created)
        );

        let second = key("two");
        let second_audit = audit_event(
            "audit_key_two",
            2,
            &"A".repeat(43),
            'B',
            "api_key.create",
            &second.id,
        );
        assert_eq!(
            store
                .insert_api_key_with_limit_and_audit(
                    &second,
                    Some(1),
                    1,
                    &"A".repeat(43),
                    &second_audit,
                )
                .expect("enforce key limit"),
            MutationWithAuditOutcome::Mutation(ResourceCreateOutcome::LimitReached)
        );
        assert!(
            store
                .update_api_key_status(&first.id, &member.id, 0, "revoked", "revoked-hmac")
                .expect("revoke first key")
        );
        assert_eq!(
            store
                .insert_api_key_with_limit_and_audit(
                    &second,
                    Some(1),
                    1,
                    &"A".repeat(43),
                    &second_audit,
                )
                .expect("reuse released key capacity"),
            MutationWithAuditOutcome::Mutation(ResourceCreateOutcome::Created)
        );
    }

    #[test]
    fn one_account_has_multiple_credentials_without_runner_binding() {
        let directory = tempdir().expect("temp directory");
        let store = initialize(&directory.path().join("customer.db"));
        store
            .insert_upstream_account_unchecked(
                "account_1",
                "openai",
                "subject_1",
                "owner@example.com",
                NOW,
            )
            .expect("insert account");
        store
            .insert_credential_instance(&credential("credential_1", "account_1"))
            .expect("insert first credential");
        store
            .insert_credential_instance(&credential("credential_2", "account_1"))
            .expect("insert second credential");
        let instances = store
            .credential_instances_for_account("account_1")
            .expect("list credentials");
        assert_eq!(instances.len(), 2);

        let columns: Vec<String> = store
            .connection
            .prepare("PRAGMA table_info(upstream_credential_instances)")
            .expect("prepare table info")
            .query_map([], |row| row.get(1))
            .expect("query table info")
            .collect::<Result<_, _>>()
            .expect("collect columns");
        assert!(!columns.iter().any(|column| column == "runner_id"));

        let lease_columns: Vec<String> = store
            .connection
            .prepare("PRAGMA table_info(credential_refresh_leases)")
            .expect("prepare refresh lease table info")
            .query_map([], |row| row.get(1))
            .expect("query refresh lease table info")
            .collect::<Result<_, _>>()
            .expect("collect refresh lease columns");
        assert!(!lease_columns.iter().any(|column| column == "runner_id"));

        let oauth_session_table_count: u32 = store
            .connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='upstream_oauth_sessions'",
                [],
                |row| row.get(0),
            )
            .expect("query OAuth session table absence");
        assert_eq!(oauth_session_table_count, 0);
    }

    #[test]
    fn copied_stable_credential_cannot_be_counted_as_an_independent_instance() {
        let directory = tempdir().expect("temp directory");
        let store = initialize(&directory.path().join("customer.db"));
        store
            .insert_upstream_account_unchecked(
                "account_1",
                "openai",
                "subject_1",
                "owner@example.com",
                NOW,
            )
            .expect("insert account");
        let first = credential("credential_1", "account_1");
        let mut copied = credential("credential_2", "account_1");
        copied.credential_identity_hmac = first.credential_identity_hmac.clone();
        store
            .insert_credential_instance(&first)
            .expect("insert first credential");
        assert!(store.insert_credential_instance(&copied).is_err());
    }

    #[test]
    fn gateway_model_catalog_requires_an_active_account_and_credential_instance() {
        let directory = tempdir().expect("temp directory");
        let store = initialize(&directory.path().join("customer.db"));
        let model = ModelRecord {
            id: "model_1".to_owned(),
            public_name: "gpt-test".to_owned(),
            display_name: "GPT Test".to_owned(),
            enabled: true,
            discovered_at: Some(NOW.to_owned()),
            created_at: NOW.to_owned(),
        };
        store.upsert_model(&model).expect("upsert model");
        assert!(
            store
                .enabled_models()
                .expect("no account models")
                .is_empty()
        );
        store
            .insert_upstream_account_unchecked(
                "account_1",
                "openai",
                "subject_1",
                "owner@example.com",
                NOW,
            )
            .expect("insert account");
        store
            .connection
            .execute(
                "INSERT INTO account_models(account_id,model_id,upstream_name,discovered_at)
                 VALUES(?,?,?,?)",
                params!["account_1", model.id, "gpt-upstream", NOW],
            )
            .expect("map model to account");
        assert!(
            store
                .enabled_models()
                .expect("no credential models")
                .is_empty()
        );
        store
            .insert_credential_instance(&credential("credential_1", "account_1"))
            .expect("insert active credential");
        assert_eq!(
            store.enabled_models().expect("enabled models"),
            vec![model.clone()]
        );
        assert_eq!(
            store
                .enabled_model_by_public_name(&model.public_name)
                .expect("model lookup"),
            Some(model.clone())
        );
        let routes = store
            .gateway_route_candidates(&model.public_name)
            .expect("gateway routes");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].account_id, "account_1");
        assert_eq!(routes[0].upstream_subject_id, "subject_1");
        assert_eq!(routes[0].upstream_model, "gpt-upstream");
        assert_eq!(routes[0].credential.id, "credential_1");
        assert_eq!(routes[0].last_success_runner_id, None);
    }

    #[test]
    fn independent_credentials_refresh_separately_and_stale_results_cannot_overwrite() {
        let directory = tempdir().expect("temp directory");
        let mut store = initialize(&directory.path().join("customer.db"));
        store
            .insert_upstream_account_unchecked(
                "account_1",
                "openai",
                "subject_1",
                "owner@example.com",
                NOW,
            )
            .expect("insert account");
        store
            .insert_credential_instance(&credential("credential_1", "account_1"))
            .expect("insert credential");
        store
            .insert_credential_instance(&credential("credential_2", "account_1"))
            .expect("insert second credential");
        assert_eq!(
            store
                .acquire_credential_refresh_lease(
                    "credential_1",
                    0,
                    "lease-hash-first",
                    NOW,
                    "2026-08-27T00:02:00.000Z",
                )
                .expect("acquire first-instance lease"),
            CredentialRefreshLeaseOutcome::Acquired
        );
        assert_eq!(
            store
                .acquire_credential_refresh_lease(
                    "credential_1",
                    0,
                    "lease-hash-concurrent",
                    NOW,
                    "2026-08-27T00:02:00.000Z",
                )
                .expect("same-instance lease is busy"),
            CredentialRefreshLeaseOutcome::Busy
        );
        assert_eq!(
            store
                .acquire_credential_refresh_lease(
                    "credential_2",
                    0,
                    "lease-hash-second-instance",
                    NOW,
                    "2026-08-27T00:02:00.000Z",
                )
                .expect("different instance refreshes independently"),
            CredentialRefreshLeaseOutcome::Acquired
        );
        let first_update = CredentialRefreshUpdate {
            credential_identity_hmac: "private-hmac-refreshed".to_owned(),
            encrypted_payload: vec![9],
            payload_nonce: vec![8; 24],
            wrapped_data_key: vec![7; 48],
            wrap_nonce: vec![6; 24],
            expires_at: "2026-08-27T02:00:00.000Z".to_owned(),
            refreshed_at: "2026-08-27T00:30:00.000Z".to_owned(),
        };
        let stale_refresh_audit = audit_event(
            "audit_credential_refresh_stale",
            2,
            &"X".repeat(43),
            'Y',
            "upstream_credential.refresh",
            "credential_1",
        );
        assert_eq!(
            store
                .update_credential_after_refresh_and_audit(
                    "credential_1",
                    0,
                    "lease-hash-first",
                    "2026-08-27T00:01:00.000Z",
                    &first_update,
                    0,
                    "",
                    &stale_refresh_audit,
                )
                .expect("reject stale credential refresh audit tail"),
            MutationWithAuditOutcome::AuditConflict
        );
        assert_eq!(
            store
                .credential_instance_by_id("credential_1")
                .unwrap()
                .unwrap()
                .credential_revision,
            0
        );
        assert_eq!(
            store
                .acquire_credential_refresh_lease(
                    "credential_1",
                    0,
                    "lease-hash-still-busy",
                    "2026-08-27T00:01:00.000Z",
                    "2026-08-27T00:03:00.000Z",
                )
                .expect("rolled-back refresh preserves the original lease"),
            CredentialRefreshLeaseOutcome::Busy
        );
        let refresh_audit = audit_event(
            "audit_credential_refresh",
            1,
            "",
            'R',
            "upstream_credential.refresh",
            "credential_1",
        );
        assert_eq!(
            store
                .update_credential_after_refresh_and_audit(
                    "credential_1",
                    0,
                    "lease-hash-first",
                    "2026-08-27T00:01:00.000Z",
                    &first_update,
                    0,
                    "",
                    &refresh_audit,
                )
                .expect("commit first refresh and audit together"),
            MutationWithAuditOutcome::Mutation(true)
        );
        assert!(
            store
                .update_credential_after_refresh(
                    "credential_2",
                    0,
                    "lease-hash-second-instance",
                    "2026-08-27T00:01:00.000Z",
                    &CredentialRefreshUpdate {
                        credential_identity_hmac: "private-hmac-second-refreshed".to_owned(),
                        encrypted_payload: vec![19],
                        payload_nonce: vec![18; 24],
                        wrapped_data_key: vec![17; 48],
                        wrap_nonce: vec![16; 24],
                        expires_at: "2026-08-27T04:00:00.000Z".to_owned(),
                        refreshed_at: "2026-08-27T00:30:30.000Z".to_owned(),
                    },
                )
                .expect("commit second credential refresh independently")
        );
        assert_eq!(
            store
                .acquire_credential_refresh_lease(
                    "credential_1",
                    1,
                    "lease-hash-next",
                    "2026-08-27T00:01:30.000Z",
                    "2026-08-27T00:03:30.000Z",
                )
                .expect("acquire next revision lease"),
            CredentialRefreshLeaseOutcome::Acquired
        );
        assert!(
            !store
                .update_credential_after_refresh(
                    "credential_1",
                    0,
                    "lease-hash-next",
                    "2026-08-27T00:02:00.000Z",
                    &CredentialRefreshUpdate {
                        credential_identity_hmac: "private-hmac-stale".to_owned(),
                        encrypted_payload: vec![4],
                        payload_nonce: vec![3; 24],
                        wrapped_data_key: vec![2; 48],
                        wrap_nonce: vec![1; 24],
                        expires_at: "2026-08-27T03:00:00.000Z".to_owned(),
                        refreshed_at: "2026-08-27T00:31:00.000Z".to_owned(),
                    },
                )
                .expect("reject stale refresh")
        );
        let instances = store
            .credential_instances_for_account("account_1")
            .expect("load credentials");
        let first = instances
            .iter()
            .find(|instance| instance.id == "credential_1")
            .expect("first credential exists");
        assert_eq!(first.credential_revision, 1);
        assert_eq!(first.encrypted_payload, vec![9]);
        let second = instances
            .iter()
            .find(|instance| instance.id == "credential_2")
            .expect("second credential exists");
        assert_eq!(second.credential_revision, 1);
        assert_eq!(second.encrypted_payload, vec![19]);
        assert_eq!(store.audit_events().unwrap(), vec![refresh_audit]);
    }

    #[test]
    fn local_runner_registration_rolls_back_on_audit_conflict_and_retains_deletion_tombstone() {
        let directory = tempdir().unwrap();
        let mut store = initialize(&directory.path().join("customer.db"));
        let owner = identity("identity_owner", "owner@example.com", false);
        store
            .initialize_first_owner(&owner, &seat_state(&[], 0), &initial_balance(&owner.id))
            .unwrap();
        let enrollment = RunnerEnrollmentRecord {
            id: "enrollment_local_blue".into(),
            token_hash: "a".repeat(64),
            token_prefix: "local-slot".into(),
            runner_name: "local-runner-blue".into(),
            status: "pending".into(),
            expires_at: NOW.into(),
            created_by: owner.id.clone(),
            created_at: NOW.into(),
        };
        let registration = RunnerRegistrationRecord {
            id: "runner_local_blue".into(),
            credential_hash: "b".repeat(64),
            version: "2.0.1".into(),
            protocol_version: 3,
            platform: "linux".into(),
            architecture: "x86_64".into(),
            max_inflight: 4,
            created_at: NOW.into(),
        };
        let event = audit_event(
            "audit_local_blue",
            1,
            "",
            'a',
            "runner.register",
            &registration.id,
        );
        assert_eq!(
            store
                .register_local_runner_and_audit(&enrollment, &registration, 9, "wrong", &event)
                .unwrap(),
            AuditedMutationOutcome::AuditConflict
        );
        assert!(store.list_runners().unwrap().is_empty());
        let count: i64 = store
            .connection
            .query_row("SELECT COUNT(*) FROM runner_enrollments", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(
            store
                .register_local_runner_and_audit(&enrollment, &registration, 0, "", &event)
                .unwrap(),
            AuditedMutationOutcome::Applied
        );
        assert_eq!(store.audit_events().unwrap().len(), 1);
        store.delete_runner(&registration.id).unwrap();
        assert_eq!(
            store
                .register_local_runner_and_audit(&enrollment, &registration, 0, "", &event)
                .unwrap(),
            AuditedMutationOutcome::MutationConflict
        );
        let mut other_enrollment = enrollment.clone();
        other_enrollment.id = "replacement_enrollment".into();
        other_enrollment.token_hash = "c".repeat(64);
        let mut other_registration = registration;
        other_registration.id = "replacement_runner".into();
        assert_eq!(
            store
                .register_local_runner_and_audit(
                    &other_enrollment,
                    &other_registration,
                    0,
                    "",
                    &event
                )
                .unwrap(),
            AuditedMutationOutcome::MutationConflict
        );
        assert!(store.list_runners().unwrap().is_empty());
    }

    #[test]
    fn legacy_runner_registration_path_remains_unlimited() {
        let directory = tempdir().expect("temp directory");
        let mut store = initialize(&directory.path().join("customer.db"));
        let owner = identity("identity_owner", "owner@example.com", false);
        assert_eq!(
            store
                .initialize_first_owner(&owner, &seat_state(&[], 0), &initial_balance(&owner.id),)
                .expect("initialize owner"),
            IdentityInitializeOutcome::Created
        );

        for index in 0..64_u32 {
            let enrollment_id = format!("enrollment_{index:032x}");
            let runner_id = format!("runner_{index:032x}");
            let token_hash = format!("{index:064x}");
            store
                .insert_runner_enrollment(&RunnerEnrollmentRecord {
                    id: enrollment_id,
                    token_hash: token_hash.clone(),
                    token_prefix: format!("aren_{index:011x}"),
                    runner_name: format!("Runner {index}"),
                    status: "pending".to_owned(),
                    expires_at: "2026-08-28T00:00:00.000Z".to_owned(),
                    created_by: owner.id.clone(),
                    created_at: NOW.to_owned(),
                })
                .expect("insert enrollment without runner count gate");
            assert!(
                store
                    .pending_runner_enrollment(&token_hash, NOW)
                    .expect("pending enrollment")
            );
            assert!(
                !store
                    .pending_runner_enrollment(&token_hash, "2026-08-29T00:00:00.000Z")
                    .expect("expired enrollment")
            );
            assert_eq!(
                store
                    .consume_runner_enrollment_unchecked(
                        &token_hash,
                        NOW,
                        &RunnerRegistrationRecord {
                            id: runner_id,
                            credential_hash: format!("{:064x}", index + 1_000),
                            version: "0.1.0".to_owned(),
                            protocol_version: 2,
                            platform: "linux".to_owned(),
                            architecture: "x86_64".to_owned(),
                            max_inflight: 8,
                            created_at: NOW.to_owned(),
                        },
                    )
                    .expect("consume enrollment"),
                RunnerEnrollmentConsumeOutcome::Registered
            );
            assert!(
                !store
                    .pending_runner_enrollment(&token_hash, NOW)
                    .expect("consumed enrollment")
            );
        }
        assert_eq!(store.list_runners().expect("list runners").len(), 64);

        assert_eq!(
            store
                .consume_runner_enrollment_unchecked(
                    &format!("{:064x}", 0),
                    NOW,
                    &RunnerRegistrationRecord {
                        id: "runner_ffffffffffffffffffffffffffffffff".to_owned(),
                        credential_hash: "f".repeat(64),
                        version: "0.1.0".to_owned(),
                        protocol_version: 2,
                        platform: "linux".to_owned(),
                        architecture: "x86_64".to_owned(),
                        max_inflight: 8,
                        created_at: NOW.to_owned(),
                    },
                )
                .expect("reject replayed enrollment"),
            RunnerEnrollmentConsumeOutcome::Invalid
        );
        assert_eq!(store.list_runners().expect("list runners").len(), 64);
    }

    #[test]
    fn runner_limit_counts_disabled_records_and_delete_releases_capacity() {
        let directory = tempdir().expect("temp directory");
        let mut store = initialize(&directory.path().join("customer.db"));
        let owner = identity("identity_owner", "owner@example.com", false);
        assert_eq!(
            store
                .initialize_first_owner(&owner, &seat_state(&[], 0), &initial_balance(&owner.id))
                .expect("initialize owner"),
            IdentityInitializeOutcome::Created
        );
        let enrollment = |id: &str, token_hash: &str| RunnerEnrollmentRecord {
            id: id.to_owned(),
            token_hash: token_hash.to_owned(),
            token_prefix: format!("aren_{id}"),
            runner_name: id.to_owned(),
            status: "pending".to_owned(),
            expires_at: "2026-08-28T00:00:00.000Z".to_owned(),
            created_by: owner.id.clone(),
            created_at: NOW.to_owned(),
        };
        let registration = |id: &str| RunnerRegistrationRecord {
            id: id.to_owned(),
            credential_hash: format!("credential-{id}"),
            version: "0.1.0".to_owned(),
            protocol_version: 2,
            platform: "linux".to_owned(),
            architecture: "x86_64".to_owned(),
            max_inflight: 8,
            created_at: NOW.to_owned(),
        };
        store
            .insert_runner_enrollment(&enrollment("enrollment_one", "token_one"))
            .expect("insert first enrollment");
        let first_audit = audit_event(
            "audit_runner_one",
            1,
            "",
            'A',
            "runner.register",
            "runner_one",
        );
        assert_eq!(
            store
                .consume_runner_enrollment_with_limit_and_audit(
                    "token_one",
                    NOW,
                    &registration("runner_one"),
                    Some(1),
                    0,
                    "",
                    &first_audit,
                )
                .expect("register first Runner"),
            MutationWithAuditOutcome::Mutation(RunnerEnrollmentConsumeOutcome::Registered)
        );
        assert!(
            store
                .update_runner_enabled("runner_one", false, NOW)
                .expect("disable first Runner")
        );
        store
            .insert_runner_enrollment(&enrollment("enrollment_two", "token_two"))
            .expect("insert second enrollment");
        let second_audit = audit_event(
            "audit_runner_two",
            2,
            &"A".repeat(43),
            'B',
            "runner.register",
            "runner_two",
        );
        assert_eq!(
            store
                .consume_runner_enrollment_with_limit_and_audit(
                    "missing_token",
                    NOW,
                    &registration("runner_missing"),
                    Some(1),
                    1,
                    &"A".repeat(43),
                    &second_audit,
                )
                .expect("reject invalid enrollment before quota check"),
            MutationWithAuditOutcome::Mutation(RunnerEnrollmentConsumeOutcome::Invalid)
        );
        assert_eq!(
            store
                .consume_runner_enrollment_with_limit_and_audit(
                    "token_two",
                    NOW,
                    &registration("runner_two"),
                    Some(1),
                    1,
                    &"A".repeat(43),
                    &second_audit,
                )
                .expect("disabled Runner still consumes capacity"),
            MutationWithAuditOutcome::Mutation(RunnerEnrollmentConsumeOutcome::LimitReached)
        );
        assert!(
            store
                .delete_runner("runner_one")
                .expect("delete first Runner")
        );
        assert_eq!(
            store
                .consume_runner_enrollment_with_limit_and_audit(
                    "token_two",
                    NOW,
                    &registration("runner_two"),
                    Some(1),
                    1,
                    &"A".repeat(43),
                    &second_audit,
                )
                .expect("reuse released Runner capacity"),
            MutationWithAuditOutcome::Mutation(RunnerEnrollmentConsumeOutcome::Registered)
        );
    }

    #[test]
    fn upstream_account_limit_counts_disabled_records_and_delete_releases_capacity() {
        let directory = tempdir().expect("temp directory");
        let mut store = initialize(&directory.path().join("customer.db"));
        let first_audit = audit_event(
            "audit_account_one",
            1,
            "",
            'A',
            "upstream_account.create",
            "account_one",
        );
        assert_eq!(
            store
                .insert_upstream_account_with_limit_and_audit(
                    "account_one",
                    "openai",
                    "subject_one",
                    "one@example.com",
                    NOW,
                    Some(1),
                    0,
                    "",
                    &first_audit,
                )
                .expect("create first account"),
            MutationWithAuditOutcome::Mutation(ResourceCreateOutcome::Created)
        );
        let duplicate_audit = audit_event(
            "audit_account_duplicate",
            2,
            &"A".repeat(43),
            'B',
            "upstream_account.create",
            "account_duplicate",
        );
        assert_eq!(
            store
                .insert_upstream_account_with_limit_and_audit(
                    "account_duplicate",
                    "openai",
                    "subject_one",
                    "one@example.com",
                    NOW,
                    Some(1),
                    1,
                    &"A".repeat(43),
                    &duplicate_audit,
                )
                .expect("recognize an existing account before quota"),
            MutationWithAuditOutcome::Mutation(ResourceCreateOutcome::Conflict)
        );
        assert!(
            store
                .update_upstream_account_status("account_one", "disabled", NOW)
                .expect("disable first account")
        );
        let second_audit = audit_event(
            "audit_account_two",
            2,
            &"A".repeat(43),
            'B',
            "upstream_account.create",
            "account_two",
        );
        assert_eq!(
            store
                .insert_upstream_account_with_limit_and_audit(
                    "account_two",
                    "openai",
                    "subject_two",
                    "two@example.com",
                    NOW,
                    Some(1),
                    1,
                    &"A".repeat(43),
                    &second_audit,
                )
                .expect("disabled account still consumes capacity"),
            MutationWithAuditOutcome::Mutation(ResourceCreateOutcome::LimitReached)
        );
        assert!(
            store
                .delete_upstream_account("account_one")
                .expect("delete first account")
        );
        assert_eq!(
            store
                .insert_upstream_account_with_limit_and_audit(
                    "account_two",
                    "openai",
                    "subject_two",
                    "two@example.com",
                    NOW,
                    Some(1),
                    1,
                    &"A".repeat(43),
                    &second_audit,
                )
                .expect("reuse released account capacity"),
            MutationWithAuditOutcome::Mutation(ResourceCreateOutcome::Created)
        );
    }

    #[test]
    fn member_status_and_signed_seat_registry_change_in_one_transaction() {
        let directory = tempdir().expect("temp directory");
        let mut store = initialize(&directory.path().join("customer.db"));
        let owner = identity("identity_owner", "owner@example.com", false);
        let initial_state = seat_state(&[], 0);
        assert_eq!(
            store
                .initialize_first_owner(&owner, &initial_state, &initial_balance(&owner.id))
                .expect("initialize owner"),
            IdentityInitializeOutcome::Created
        );
        let member = identity("identity_member", "member@example.com", true);
        let active_state = seat_state(&["identity_member"], 1);
        assert_eq!(
            store
                .create_identity_with_seat_limit(
                    &member,
                    1,
                    &initial_state,
                    &active_state,
                    &initial_balance(&member.id),
                )
                .expect("create member"),
            IdentityCreateOutcome::Created
        );

        let mut disabled = member.clone();
        disabled.status = "disabled".to_owned();
        disabled.revision = 1;
        disabled.integrity_hmac = "identity-hmac-disabled".to_owned();
        disabled.updated_at = "2026-08-27T00:01:00.000Z".to_owned();
        let disabled_state = SecurityStateRecord {
            key: active_state.key.clone(),
            value: encode_seat_registry(&[]),
            revision: 2,
            mac: b"seat-state-disabled".to_vec(),
            updated_at: disabled.updated_at.clone(),
        };
        assert_eq!(
            store
                .update_member_identity_status(&member, &disabled, &active_state, &disabled_state,)
                .expect("disable member"),
            IdentityStatusUpdateOutcome::Updated
        );
        assert!(
            store
                .seat_registry_snapshot(&active_state.key)
                .expect("disabled seat registry")
                .occupied_identity_ids
                .is_empty()
        );

        let mut reenabled = disabled.clone();
        reenabled.status = "active".to_owned();
        reenabled.revision = 2;
        reenabled.integrity_hmac = "identity-hmac-reenabled".to_owned();
        reenabled.updated_at = "2026-08-27T00:02:00.000Z".to_owned();
        let reenabled_state = SecurityStateRecord {
            key: active_state.key.clone(),
            value: encode_seat_registry(std::slice::from_ref(&member.id)),
            revision: 3,
            mac: b"seat-state-reenabled".to_vec(),
            updated_at: reenabled.updated_at.clone(),
        };
        assert_eq!(
            store
                .update_member_identity_status(
                    &disabled,
                    &reenabled,
                    &disabled_state,
                    &reenabled_state,
                )
                .expect("reenable member"),
            IdentityStatusUpdateOutcome::Updated
        );
        assert_eq!(
            store
                .seat_registry_snapshot(&active_state.key)
                .expect("reenabled seat registry")
                .occupied_identity_ids,
            vec![member.id]
        );
    }

    #[test]
    fn member_seat_mutations_and_audit_chain_commit_or_rollback_together() {
        let directory = tempdir().expect("temp directory");
        let mut store = initialize(&directory.path().join("customer.db"));
        let owner = identity("identity_owner", "owner@example.com", false);
        let initial_state = seat_state(&[], 0);
        let stale_owner_audit = audit_event(
            "audit_stale_owner",
            2,
            &"X".repeat(43),
            'Y',
            "owner.initialize",
            &owner.id,
        );
        assert_eq!(
            store
                .initialize_first_owner_and_audit(
                    &owner,
                    &initial_state,
                    &initial_balance(&owner.id),
                    1,
                    &"X".repeat(43),
                    &stale_owner_audit,
                )
                .expect("reject stale owner audit tail"),
            MutationWithAuditOutcome::AuditConflict
        );
        assert!(
            store
                .identity_by_id(&owner.id)
                .expect("load rolled-back owner")
                .is_none()
        );
        assert_eq!(store.audit_events().expect("empty audit log"), Vec::new());

        let owner_audit = audit_event("audit_owner", 1, "", 'A', "owner.initialize", &owner.id);
        assert_eq!(
            store
                .initialize_first_owner_and_audit(
                    &owner,
                    &initial_state,
                    &initial_balance(&owner.id),
                    0,
                    "",
                    &owner_audit,
                )
                .expect("initialize owner with audit"),
            MutationWithAuditOutcome::Mutation(IdentityInitializeOutcome::Created)
        );

        let first = identity("identity_member_1", "one@example.com", true);
        let second = identity("identity_member_2", "two@example.com", true);
        let identities = vec![first.clone(), second.clone()];
        let balances = vec![initial_balance(&first.id), initial_balance(&second.id)];
        let active_state = seat_state(&["identity_member_1", "identity_member_2"], 1);
        let stale_batch_events = vec![
            audit_event(
                "audit_stale_member_1",
                1,
                "",
                'P',
                "member.create",
                &first.id,
            ),
            audit_event(
                "audit_stale_member_2",
                2,
                &"P".repeat(43),
                'Q',
                "member.create",
                &second.id,
            ),
        ];
        assert_eq!(
            store
                .create_identities_with_seat_limit_and_audit(
                    &identities,
                    2,
                    &initial_state,
                    &active_state,
                    &balances,
                    0,
                    "",
                    &stale_batch_events,
                    &[],
                )
                .expect("reject stale batch audit tail"),
            MutationWithAuditOutcome::AuditConflict
        );
        assert!(
            store
                .identity_by_id(&first.id)
                .expect("load rolled-back first member")
                .is_none()
        );
        assert_eq!(
            store
                .seat_registry_snapshot(&initial_state.key)
                .expect("seat state after rollback")
                .state,
            Some(initial_state.clone())
        );

        let member_events = vec![
            audit_event(
                "audit_member_1",
                2,
                &"A".repeat(43),
                'B',
                "member.create",
                &first.id,
            ),
            audit_event(
                "audit_member_2",
                3,
                &"B".repeat(43),
                'C',
                "member.create",
                &second.id,
            ),
        ];
        assert_eq!(
            store
                .create_identities_with_seat_limit_and_audit(
                    &identities,
                    2,
                    &initial_state,
                    &active_state,
                    &balances,
                    1,
                    &"A".repeat(43),
                    &member_events,
                    &[],
                )
                .expect("create members with audit"),
            MutationWithAuditOutcome::Mutation(IdentityCreateOutcome::Created)
        );

        let mut disabled = first.clone();
        disabled.status = "disabled".to_owned();
        disabled.revision = 1;
        disabled.integrity_hmac = "identity-hmac-disabled".to_owned();
        disabled.updated_at = "2026-08-27T00:01:00.000Z".to_owned();
        let disabled_state = SecurityStateRecord {
            key: active_state.key.clone(),
            value: encode_seat_registry(std::slice::from_ref(&second.id)),
            revision: 2,
            mac: b"seat-state-disabled".to_vec(),
            updated_at: disabled.updated_at.clone(),
        };
        let stale_status_audit = audit_event(
            "audit_stale_status",
            2,
            &"A".repeat(43),
            'R',
            "member.status.update",
            &first.id,
        );
        assert_eq!(
            store
                .update_member_identity_status_and_audit(
                    &first,
                    &disabled,
                    &active_state,
                    &disabled_state,
                    1,
                    &"A".repeat(43),
                    &stale_status_audit,
                )
                .expect("reject stale status audit tail"),
            MutationWithAuditOutcome::AuditConflict
        );
        assert_eq!(
            store
                .identity_by_id(&first.id)
                .expect("load active member")
                .expect("member exists")
                .status,
            "active"
        );
        let status_audit = audit_event(
            "audit_status",
            4,
            &"C".repeat(43),
            'D',
            "member.status.update",
            &first.id,
        );
        assert_eq!(
            store
                .update_member_identity_status_and_audit(
                    &first,
                    &disabled,
                    &active_state,
                    &disabled_state,
                    3,
                    &"C".repeat(43),
                    &status_audit,
                )
                .expect("disable member with audit"),
            MutationWithAuditOutcome::Mutation(IdentityStatusUpdateOutcome::Updated)
        );
        assert_eq!(store.audit_events().expect("audit events").len(), 4);
        assert_eq!(store.audit_revision().expect("audit revision"), 4);
    }

    #[test]
    fn audit_log_appends_one_global_ordered_chain_with_compare_and_swap() {
        let directory = tempdir().expect("temp directory");
        let mut store = initialize(&directory.path().join("customer.db"));
        assert_eq!(store.audit_tail().expect("empty audit tail"), None);
        assert_eq!(store.audit_revision().expect("empty audit revision"), 0);
        let first = AuditEventRecord {
            id: "audit_00000000000000000000000000000001".to_owned(),
            sequence: 1,
            actor_identity_id: Some("identity_admin".to_owned()),
            actor_role: "admin".to_owned(),
            action: "member.create".to_owned(),
            target_type: "identity".to_owned(),
            target_id: Some("identity_member".to_owned()),
            outcome: "succeeded".to_owned(),
            previous_event_hmac: String::new(),
            integrity_hmac: "A".repeat(43),
            created_at: NOW.to_owned(),
        };
        assert_eq!(
            store
                .append_audit_event(0, "", &first)
                .expect("append first audit event"),
            AuditAppendOutcome::Applied
        );
        let competing = AuditEventRecord {
            id: "audit_00000000000000000000000000000002".to_owned(),
            integrity_hmac: "B".repeat(43),
            ..first.clone()
        };
        assert_eq!(
            store
                .append_audit_event(0, "", &competing)
                .expect("reject stale audit append"),
            AuditAppendOutcome::Conflict
        );
        let second = AuditEventRecord {
            id: competing.id,
            sequence: 2,
            action: "member.disable".to_owned(),
            previous_event_hmac: first.integrity_hmac.clone(),
            integrity_hmac: competing.integrity_hmac,
            ..first.clone()
        };
        assert_eq!(
            store
                .append_audit_event(1, &first.integrity_hmac, &second)
                .expect("append second audit event"),
            AuditAppendOutcome::Applied
        );
        assert_eq!(
            store.audit_tail().expect("audit tail"),
            Some(second.clone())
        );
        assert_eq!(
            store.audit_events().expect("ordered audit events"),
            vec![first, second]
        );
        assert_eq!(store.audit_revision().expect("audit revision"), 2);
    }

    #[test]
    fn runtime_setting_and_audit_event_commit_or_rollback_together() {
        let directory = tempdir().expect("temp directory");
        let mut store = initialize(&directory.path().join("customer.db"));
        let setting = RuntimeSettingRecord {
            key: "runtime-configuration-v1".to_owned(),
            value: "first".to_owned(),
            revision: 0,
            integrity_hmac: "setting-hmac-1".to_owned(),
            updated_at: NOW.to_owned(),
        };
        let first_audit = AuditEventRecord {
            id: "audit_00000000000000000000000000000011".to_owned(),
            sequence: 1,
            actor_identity_id: Some("identity_admin".to_owned()),
            actor_role: "admin".to_owned(),
            action: "settings.update".to_owned(),
            target_type: "runtime_setting".to_owned(),
            target_id: Some(setting.key.clone()),
            outcome: "succeeded".to_owned(),
            previous_event_hmac: String::new(),
            integrity_hmac: "C".repeat(43),
            created_at: NOW.to_owned(),
        };
        assert_eq!(
            store
                .write_runtime_setting_with_audit(None, &setting, 0, "", &first_audit)
                .expect("write setting with audit"),
            AuditedMutationOutcome::Applied
        );

        let updated = RuntimeSettingRecord {
            value: "second".to_owned(),
            revision: 1,
            integrity_hmac: "setting-hmac-2".to_owned(),
            ..setting.clone()
        };
        let stale_audit = AuditEventRecord {
            id: "audit_00000000000000000000000000000012".to_owned(),
            integrity_hmac: "D".repeat(43),
            ..first_audit.clone()
        };
        assert_eq!(
            store
                .write_runtime_setting_with_audit(Some(&setting), &updated, 0, "", &stale_audit,)
                .expect("reject stale audit tail"),
            AuditedMutationOutcome::AuditConflict
        );
        assert_eq!(
            store
                .runtime_setting(&setting.key)
                .expect("setting after rollback"),
            Some(setting.clone())
        );
        assert_eq!(store.audit_revision().expect("revision after rollback"), 1);

        let second_audit = AuditEventRecord {
            sequence: 2,
            previous_event_hmac: first_audit.integrity_hmac.clone(),
            ..stale_audit
        };
        assert_eq!(
            store
                .write_runtime_setting_with_audit(
                    Some(&setting),
                    &updated,
                    1,
                    &first_audit.integrity_hmac,
                    &second_audit,
                )
                .expect("commit updated setting and audit"),
            AuditedMutationOutcome::Applied
        );
        assert_eq!(
            store
                .runtime_setting(&setting.key)
                .expect("updated setting"),
            Some(updated)
        );
        assert_eq!(store.audit_revision().expect("updated revision"), 2);
    }

    #[test]
    fn final_schema_has_no_runner_placement_or_seat_limit_table() {
        let directory = tempdir().expect("temp directory");
        let store = initialize(&directory.path().join("customer.db"));
        let names: Vec<String> = store
            .connection
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .expect("prepare table query")
            .query_map([], |row| row.get(0))
            .expect("query tables")
            .collect::<Result<_, _>>()
            .expect("collect tables");
        assert!(!names.iter().any(|name| name == "runner_account_placements"));
        assert!(!names.iter().any(|name| name == "runner_limits"));
        assert!(!names.iter().any(|name| name == "admins"));
        assert!(names.iter().any(|name| name == "audit_events"));
    }
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
fn runner_quota_snapshot_sqlite(
    connection: &Connection,
) -> Result<RunnerQuotaSnapshot, StorageError> {
    let state = connection
        .query_row(
            "SELECT key,value,revision,mac,updated_at FROM security_state WHERE key=?",
            [RUNNER_QUOTA_KEY],
            |row| {
                Ok(SecurityStateRecord {
                    key: row.get(0)?,
                    value: row.get(1)?,
                    revision: {
                        let value: i64 = row.get(2)?;
                        u64::try_from(value)
                            .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(2, value))?
                    },
                    mac: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            },
        )
        .optional()?;
    let members = connection
        .prepare("SELECT id,credential_hash FROM runners ORDER BY id")?
        .query_map([], |row| {
            Ok(RunnerQuotaMember {
                id: row.get(0)?,
                credential_hash: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RunnerQuotaSnapshot { state, members })
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
impl SqlCipherStore {
    pub fn apply_runner_quota_change(
        &mut self,
        write: runner_quota::RunnerQuotaWrite<'_>,
        runner_limit: Option<u32>,
        audit: (u64, &str, &AuditEventRecord),
    ) -> Result<runner_quota::RunnerQuotaWriteOutcome, StorageError> {
        use runner_quota::{RunnerQuotaAction, RunnerQuotaWriteOutcome as Outcome};
        if !write.valid(audit.2) {
            return Err(StorageError::RunnerQuotaIntegrity);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "UPDATE transaction_gates SET revision=revision+1 WHERE gate_key='entity_quota'",
            [],
        )?;
        if !write
            .change
            .matches_before(&runner_quota_snapshot_sqlite(&transaction)?)
        {
            transaction.rollback()?;
            return Ok(Outcome::QuotaChanged);
        }
        match write.change.action() {
            RunnerQuotaAction::Register(_) => {
                if runner_limit.is_some_and(|limit| write.change.occupied_before() > limit) {
                    transaction.rollback()?;
                    return Ok(Outcome::LimitReached);
                }
                let enrollment = write.enrollment.ok_or(StorageError::RunnerQuotaIntegrity)?;
                let registration = write
                    .registration
                    .ok_or(StorageError::RunnerQuotaIntegrity)?;
                let seen: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM runner_enrollments WHERE id=? OR runner_id=?)",
                    params![enrollment.id, registration.id],
                    |row| row.get(0),
                )?;
                if seen {
                    transaction.rollback()?;
                    return Ok(Outcome::Conflict);
                }
                transaction.execute(
                    "INSERT INTO runner_enrollments(id,token_hash,token_prefix,runner_name,status,expires_at,created_by,created_at)
                     VALUES(?,?,?,?,'pending',?,?,?)",
                    params![enrollment.id,enrollment.token_hash,enrollment.token_prefix,enrollment.runner_name,enrollment.expires_at,enrollment.created_by,enrollment.created_at],
                )?;
                transaction.execute(
                    "INSERT INTO runners(id,enrollment_id,name,credential_hash,enabled,version,protocol_version,platform,architecture,max_inflight,created_at,updated_at)
                     VALUES(?,?,?,?,1,?,?,?,?,?,?,?)",
                    params![registration.id,enrollment.id,enrollment.runner_name,registration.credential_hash,registration.version,registration.protocol_version,registration.platform,registration.architecture,registration.max_inflight,registration.created_at,registration.created_at],
                )?;
                transaction.execute(
                    "UPDATE runner_enrollments SET status='used',used_at=?,runner_id=? WHERE id=?",
                    params![registration.created_at, registration.id, enrollment.id],
                )?;
            }
            RunnerQuotaAction::Delete(member) => {
                if transaction.execute(
                    "DELETE FROM runners WHERE id=? AND credential_hash=?",
                    params![member.id, member.credential_hash],
                )? != 1
                {
                    transaction.rollback()?;
                    return Ok(Outcome::Conflict);
                }
            }
        }
        if !write
            .change
            .matches_after(&runner_quota_snapshot_sqlite(&transaction)?.members)
        {
            return Err(StorageError::RunnerQuotaIntegrity);
        }
        let next = write.next_state;
        let revision =
            i64::try_from(next.revision).map_err(|_| StorageError::RunnerQuotaIntegrity)?;
        transaction.execute(
            "INSERT INTO security_state(key,value,revision,mac,updated_at) VALUES(?,?,?,?,?)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value,revision=excluded.revision,mac=excluded.mac,updated_at=excluded.updated_at",
            params![next.key,next.value,revision,next.mac,next.updated_at],
        )?;
        if !append_audit_events_sqlite(
            &transaction,
            audit.0,
            audit.1,
            std::slice::from_ref(audit.2),
        )? {
            transaction.rollback()?;
            return Ok(Outcome::AuditConflict);
        }
        transaction.commit()?;
        Ok(Outcome::Applied)
    }
}
