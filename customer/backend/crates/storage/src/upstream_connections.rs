//! Channel-scoped connections. API keys have an internal identity and no
//! fabricated OAuth subject, email, expiry, or refresh state.
use crate::ModelRecord;
#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
use crate::StorageError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionRecord {
    pub id: String,
    pub provider: String,
    pub channel_id: String,
    pub endpoint_profile: String,
    pub auth_scheme: String,
    pub billing_mode: String,
    pub display_name: String,
    pub legacy_account_id: Option<String>,
    pub status: String,
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionCredential {
    pub id: String,
    pub connection_id: String,
    pub credential_identity_hmac: String,
    pub encrypted_payload: Vec<u8>,
    pub payload_nonce: Vec<u8>,
    pub wrapped_data_key: Vec<u8>,
    pub wrap_nonce: Vec<u8>,
    pub credential_revision: u32,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionModelBinding {
    pub connection_id: String,
    pub model_id: String,
    pub upstream_name: String,
    pub enabled: bool,
    pub capabilities_json: String,
    pub discovered_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionRoute {
    pub connection: ConnectionRecord,
    pub credential: ConnectionCredential,
    pub binding: ConnectionModelBinding,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionModelOrigin {
    pub provider: String,
    pub upstream_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionModelSpec {
    pub model: ModelRecord,
    pub upstream_name: String,
    pub quota_unit: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionMutationOutcome {
    Updated,
    Conflict,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionModelSyncOutcome {
    Applied { added: usize },
    Conflict,
    AlreadyBound,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionModelSync {
    pub connection_id: String,
    pub expected_revision: i64,
    pub models: Vec<ConnectionModelSpec>,
    pub now: String,
}

pub fn valid_connection(record: &ConnectionRecord, credential: &ConnectionCredential) -> bool {
    record.id == credential.connection_id
        && record.legacy_account_id.is_none()
        && record.auth_scheme == "api_key"
        && matches!(record.billing_mode.as_str(), "usage" | "coding_plan")
        && record.status == "active"
        && credential.status == "active"
        && record.revision > 0
        && record.channel_id.len() <= 128
        && record.endpoint_profile.len() <= 128
        && !record.display_name.trim().is_empty()
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
mod sqlite {
    use super::*;
    use crate::SqlCipherStore;
    use crate::{AuditEventRecord, MutationWithAuditOutcome, append_audit_events_sqlite};
    use rusqlite::{OptionalExtension, TransactionBehavior, params};

    impl SqlCipherStore {
        pub fn connection_model_origin(
            &self,
            model_id: &str,
        ) -> Result<Option<ConnectionModelOrigin>, StorageError> {
            self.connection
                .query_row(
                    "SELECT c.provider,b.upstream_name FROM upstream_connection_models b
                 JOIN upstream_connections c ON c.id=b.connection_id WHERE b.model_id=?
                 ORDER BY b.enabled DESC,c.status='active' DESC,c.updated_at DESC,c.id LIMIT 1",
                    [model_id],
                    |row| {
                        Ok(ConnectionModelOrigin {
                            provider: row.get(0)?,
                            upstream_name: row.get(1)?,
                        })
                    },
                )
                .optional()
                .map_err(StorageError::from)
        }

        pub fn insert_api_key_connection(
            &mut self,
            connection: &ConnectionRecord,
            credential: &ConnectionCredential,
            models: &[ConnectionModelSpec],
            expected_audit_sequence: u64,
            expected_audit_hmac: &str,
            audit_event: &AuditEventRecord,
        ) -> Result<MutationWithAuditOutcome<()>, StorageError> {
            if !valid_connection(connection, credential) {
                return Err(StorageError::MigrationIntegrity);
            }
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute(
                "INSERT INTO upstream_connections(id,provider,channel_id,endpoint_profile,auth_scheme,
                 billing_mode,display_name,legacy_account_id,status,revision,created_at,updated_at)
                 VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
                params![connection.id, connection.provider, connection.channel_id,
                    connection.endpoint_profile, connection.auth_scheme, connection.billing_mode,
                    connection.display_name, connection.legacy_account_id, connection.status,
                    connection.revision, connection.created_at, connection.updated_at],
            )?;
            transaction.execute(
                "INSERT INTO upstream_connection_credentials(id,connection_id,credential_identity_hmac,
                 encrypted_payload,payload_nonce,wrapped_data_key,wrap_nonce,credential_revision,
                 status,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
                params![credential.id, credential.connection_id, credential.credential_identity_hmac,
                    credential.encrypted_payload, credential.payload_nonce, credential.wrapped_data_key,
                    credential.wrap_nonce, credential.credential_revision, credential.status,
                    credential.created_at, credential.updated_at],
            )?;
            for spec in models {
                let model = &spec.model;
                let upstream_name = &spec.upstream_name;
                if !matches!(spec.quota_unit.as_str(), "token" | "image") {
                    return Err(StorageError::MigrationIntegrity);
                }
                transaction.execute(
                    "INSERT INTO models(id,public_name,display_name,enabled,discovered_at,created_at)
                     VALUES(?,?,?,0,NULL,?) ON CONFLICT(public_name) DO NOTHING",
                    params![model.id, model.public_name, model.display_name, model.created_at],
                )?;
                let model_id: String = transaction.query_row(
                    "SELECT id FROM models WHERE public_name=?",
                    [&model.public_name],
                    |row| row.get(0),
                )?;
                if spec.quota_unit == "image" {
                    transaction.execute(
                        "INSERT INTO model_quota_units(model_id,quota_unit) VALUES(?,'image')
                         ON CONFLICT(model_id) DO NOTHING",
                        [&model_id],
                    )?;
                } else {
                    let image_unit: bool = transaction.query_row(
                        "SELECT EXISTS(SELECT 1 FROM model_quota_units WHERE model_id=?)",
                        [&model_id],
                        |row| row.get(0),
                    )?;
                    if image_unit {
                        return Err(StorageError::MigrationIntegrity);
                    }
                }
                transaction.execute(
                    "INSERT INTO upstream_connection_models(connection_id,model_id,upstream_name,
                     enabled,capabilities_json,discovered_at) VALUES(?,?,?,1,'{}',?)",
                    params![
                        connection.id,
                        model_id,
                        upstream_name,
                        connection.created_at
                    ],
                )?;
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
            Ok(MutationWithAuditOutcome::Mutation(()))
        }

        pub fn bind_connection_model(
            &self,
            binding: &ConnectionModelBinding,
        ) -> Result<(), StorageError> {
            self.connection.execute(
                "INSERT INTO upstream_connection_models(connection_id,model_id,upstream_name,
                 enabled,capabilities_json,discovered_at) VALUES(?,?,?,?,?,?)
                 ON CONFLICT(connection_id,model_id) DO UPDATE SET upstream_name=excluded.upstream_name,
                 enabled=excluded.enabled,capabilities_json=excluded.capabilities_json,
                 discovered_at=excluded.discovered_at",
                params![binding.connection_id, binding.model_id, binding.upstream_name,
                    binding.enabled, binding.capabilities_json, binding.discovered_at],
            )?;
            Ok(())
        }

        pub fn list_api_key_connections(&self) -> Result<Vec<ConnectionRecord>, StorageError> {
            let mut statement = self.connection.prepare(
                "SELECT id,provider,channel_id,endpoint_profile,auth_scheme,billing_mode,
                 display_name,legacy_account_id,status,revision,created_at,updated_at
                 FROM upstream_connections WHERE auth_scheme='api_key' ORDER BY created_at,id",
            )?;
            statement
                .query_map([], connection_from_sqlite_row)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::from)
        }

        pub fn api_key_connection(
            &self,
            id: &str,
        ) -> Result<Option<(ConnectionRecord, ConnectionCredential)>, StorageError> {
            self.connection.query_row(
                "SELECT c.id,c.provider,c.channel_id,c.endpoint_profile,c.auth_scheme,c.billing_mode,
                 c.display_name,c.legacy_account_id,c.status,c.revision,c.created_at,c.updated_at,
                 k.id,k.connection_id,k.credential_identity_hmac,k.encrypted_payload,k.payload_nonce,
                 k.wrapped_data_key,k.wrap_nonce,k.credential_revision,k.status,k.created_at,k.updated_at
                 FROM upstream_connections c JOIN upstream_connection_credentials k ON k.connection_id=c.id
                 WHERE c.id=? AND c.auth_scheme='api_key'",
                [id],
                |row| Ok((connection_from_sqlite_row(row)?, credential_from_sqlite_row(row, 12)?)),
            ).optional().map_err(StorageError::from)
        }

        pub fn update_api_key_connection(
            &mut self,
            connection: &ConnectionRecord,
            credential: Option<&ConnectionCredential>,
            expected_revision: i64,
            expected_audit_sequence: u64,
            expected_audit_hmac: &str,
            audit_event: &AuditEventRecord,
        ) -> Result<MutationWithAuditOutcome<ConnectionMutationOutcome>, StorageError> {
            if expected_revision.checked_add(1) != Some(connection.revision)
                || !matches!(connection.status.as_str(), "active" | "disabled")
                || connection.display_name.trim().is_empty()
                || credential.is_some_and(|item| {
                    item.connection_id != connection.id || item.credential_revision == 0
                })
            {
                return Err(StorageError::MigrationIntegrity);
            }
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let changed = transaction.execute(
                "UPDATE upstream_connections SET display_name=?,status=?,revision=?,updated_at=?
                 WHERE id=? AND revision=? AND auth_scheme='api_key'",
                params![
                    connection.display_name,
                    connection.status,
                    connection.revision,
                    connection.updated_at,
                    connection.id,
                    expected_revision
                ],
            )?;
            if changed == 0 {
                transaction.rollback()?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    ConnectionMutationOutcome::Conflict,
                ));
            }
            if let Some(credential) = credential {
                let changed = transaction.execute(
                    "UPDATE upstream_connection_credentials SET credential_identity_hmac=?,encrypted_payload=?,
                     payload_nonce=?,wrapped_data_key=?,wrap_nonce=?,credential_revision=?,status='active',updated_at=?
                     WHERE connection_id=? AND id=? AND credential_revision=?",
                    params![credential.credential_identity_hmac, credential.encrypted_payload,
                        credential.payload_nonce, credential.wrapped_data_key, credential.wrap_nonce,
                        credential.credential_revision, credential.updated_at, connection.id,
                        credential.id, credential.credential_revision - 1],
                )?;
                if changed != 1 {
                    transaction.rollback()?;
                    return Ok(MutationWithAuditOutcome::Mutation(
                        ConnectionMutationOutcome::Conflict,
                    ));
                }
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
                ConnectionMutationOutcome::Updated,
            ))
        }

        pub fn sync_api_key_connection_models(
            &mut self,
            input: &ConnectionModelSync,
            expected_audit_sequence: u64,
            expected_audit_hmac: &str,
            audit_event: &AuditEventRecord,
        ) -> Result<MutationWithAuditOutcome<ConnectionModelSyncOutcome>, StorageError> {
            if input.models.len() > 256
                || input.models.iter().any(|model| model.quota_unit != "token")
            {
                return Err(StorageError::MigrationIntegrity);
            }
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let changed = transaction.execute(
                "UPDATE upstream_connections SET revision=revision+1,updated_at=?
                 WHERE id=? AND revision=? AND auth_scheme='api_key' AND status='active'",
                params![input.now, input.connection_id, input.expected_revision],
            )?;
            if changed == 0 {
                transaction.rollback()?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    ConnectionModelSyncOutcome::Conflict,
                ));
            }
            let mut added = 0;
            for spec in &input.models {
                let already_bound: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM upstream_connection_models
                     WHERE connection_id=? AND upstream_name=?)",
                    params![input.connection_id, spec.upstream_name],
                    |row| row.get(0),
                )?;
                if already_bound {
                    continue;
                }
                let model = &spec.model;
                transaction.execute(
                    "INSERT INTO models(id,public_name,display_name,enabled,discovered_at,created_at)
                     VALUES(?,?,?,0,?,?) ON CONFLICT(public_name) DO NOTHING",
                    params![model.id, model.public_name, model.display_name, input.now, input.now],
                )?;
                let model_id: String = transaction.query_row(
                    "SELECT id FROM models WHERE public_name=?",
                    [&model.public_name],
                    |row| row.get(0),
                )?;
                let existing_binding: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM upstream_connection_models
                     WHERE connection_id=? AND model_id=?)",
                    params![input.connection_id, model_id],
                    |row| row.get(0),
                )?;
                if existing_binding {
                    continue;
                }
                transaction.execute(
                    "INSERT INTO upstream_connection_models(connection_id,model_id,upstream_name,
                     enabled,capabilities_json,discovered_at) VALUES(?,?,?,1,'{}',?)",
                    params![input.connection_id, model_id, spec.upstream_name, input.now],
                )?;
                added += 1;
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
                ConnectionModelSyncOutcome::Applied { added },
            ))
        }

        pub fn add_api_key_connection_models(
            &mut self,
            input: &ConnectionModelSync,
            expected_audit_sequence: u64,
            expected_audit_hmac: &str,
            audit_event: &AuditEventRecord,
        ) -> Result<MutationWithAuditOutcome<ConnectionModelSyncOutcome>, StorageError> {
            if !(1..=64).contains(&input.models.len())
                || input
                    .models
                    .iter()
                    .any(|model| !matches!(model.quota_unit.as_str(), "token" | "image"))
            {
                return Err(StorageError::MigrationIntegrity);
            }
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let changed = transaction.execute(
                "UPDATE upstream_connections SET revision=revision+1,updated_at=?
                 WHERE id=? AND revision=? AND auth_scheme='api_key' AND status='active'",
                params![input.now, input.connection_id, input.expected_revision],
            )?;
            if changed == 0 {
                transaction.rollback()?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    ConnectionModelSyncOutcome::Conflict,
                ));
            }
            for spec in &input.models {
                let existing_upstream: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM upstream_connection_models
                     WHERE connection_id=? AND upstream_name=?)",
                    params![input.connection_id, spec.upstream_name],
                    |row| row.get(0),
                )?;
                if existing_upstream {
                    transaction.rollback()?;
                    return Ok(MutationWithAuditOutcome::Mutation(
                        ConnectionModelSyncOutcome::AlreadyBound,
                    ));
                }
                let model = &spec.model;
                let inserted = transaction.execute(
                    "INSERT INTO models(id,public_name,display_name,enabled,discovered_at,created_at)
                     VALUES(?,?,?,0,NULL,?) ON CONFLICT(public_name) DO NOTHING",
                    params![model.id, model.public_name, model.display_name, model.created_at],
                )?;
                let model_id: String = transaction.query_row(
                    "SELECT id FROM models WHERE public_name=?",
                    [&model.public_name],
                    |row| row.get(0),
                )?;
                let existing_binding: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM upstream_connection_models WHERE connection_id=? AND model_id=?)",
                    params![input.connection_id, model_id],
                    |row| row.get(0),
                )?;
                let image_unit: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM model_quota_units WHERE model_id=?)",
                    [&model_id],
                    |row| row.get(0),
                )?;
                if existing_binding
                    || (image_unit && spec.quota_unit != "image")
                    || (inserted == 0 && !image_unit && spec.quota_unit == "image")
                {
                    transaction.rollback()?;
                    return Ok(MutationWithAuditOutcome::Mutation(
                        ConnectionModelSyncOutcome::AlreadyBound,
                    ));
                }
                if spec.quota_unit == "image" {
                    transaction.execute(
                        "INSERT INTO model_quota_units(model_id,quota_unit) VALUES(?,'image')
                         ON CONFLICT(model_id) DO NOTHING",
                        [&model_id],
                    )?;
                }
                transaction.execute(
                    "INSERT INTO upstream_connection_models(connection_id,model_id,upstream_name,
                     enabled,capabilities_json,discovered_at) VALUES(?,?,?,1,'{}',?)",
                    params![input.connection_id, model_id, spec.upstream_name, input.now],
                )?;
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
                ConnectionModelSyncOutcome::Applied {
                    added: input.models.len(),
                },
            ))
        }

        pub fn api_key_connection_route(
            &self,
            public_model: &str,
        ) -> Result<Option<ConnectionRoute>, StorageError> {
            self.connection.query_row(
                "SELECT c.id,c.provider,c.channel_id,c.endpoint_profile,c.auth_scheme,c.billing_mode,
                 c.display_name,c.legacy_account_id,c.status,c.revision,c.created_at,c.updated_at,
                 k.id,k.connection_id,k.credential_identity_hmac,k.encrypted_payload,k.payload_nonce,
                 k.wrapped_data_key,k.wrap_nonce,k.credential_revision,k.status,k.created_at,k.updated_at,
                 b.connection_id,b.model_id,b.upstream_name,b.enabled,b.capabilities_json,b.discovered_at
                 FROM models m JOIN upstream_connection_models b ON b.model_id=m.id
                 JOIN upstream_connections c ON c.id=b.connection_id
                 JOIN upstream_connection_credentials k ON k.connection_id=c.id
                 WHERE m.public_name=? AND m.enabled=1 AND b.enabled=1 AND c.status='active'
                 AND k.status='active' AND c.auth_scheme='api_key'
                 ORDER BY c.updated_at DESC,c.id LIMIT 1",
                [public_model],
                |row| Ok(ConnectionRoute {
                    connection: connection_from_sqlite_row(row)?,
                    credential: credential_from_sqlite_row(row, 12)?,
                    binding: binding_from_sqlite_row(row, 23)?,
                }),
            ).optional().map_err(StorageError::from)
        }
    }

    fn connection_from_sqlite_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConnectionRecord> {
        Ok(ConnectionRecord {
            id: row.get(0)?,
            provider: row.get(1)?,
            channel_id: row.get(2)?,
            endpoint_profile: row.get(3)?,
            auth_scheme: row.get(4)?,
            billing_mode: row.get(5)?,
            display_name: row.get(6)?,
            legacy_account_id: row.get(7)?,
            status: row.get(8)?,
            revision: row.get(9)?,
            created_at: row.get(10)?,
            updated_at: row.get(11)?,
        })
    }

    fn credential_from_sqlite_row(
        row: &rusqlite::Row<'_>,
        offset: usize,
    ) -> rusqlite::Result<ConnectionCredential> {
        Ok(ConnectionCredential {
            id: row.get(offset)?,
            connection_id: row.get(offset + 1)?,
            credential_identity_hmac: row.get(offset + 2)?,
            encrypted_payload: row.get(offset + 3)?,
            payload_nonce: row.get(offset + 4)?,
            wrapped_data_key: row.get(offset + 5)?,
            wrap_nonce: row.get(offset + 6)?,
            credential_revision: row.get(offset + 7)?,
            status: row.get(offset + 8)?,
            created_at: row.get(offset + 9)?,
            updated_at: row.get(offset + 10)?,
        })
    }

    fn binding_from_sqlite_row(
        row: &rusqlite::Row<'_>,
        offset: usize,
    ) -> rusqlite::Result<ConnectionModelBinding> {
        Ok(ConnectionModelBinding {
            connection_id: row.get(offset)?,
            model_id: row.get(offset + 1)?,
            upstream_name: row.get(offset + 2)?,
            enabled: row.get(offset + 3)?,
            capabilities_json: row.get(offset + 4)?,
            discovered_at: row.get(offset + 5)?,
        })
    }
}

#[cfg(feature = "mariadb")]
mod mariadb;

#[cfg(all(test, any(feature = "sqlite-dev", feature = "sqlcipher")))]
mod tests {
    use super::*;
    use crate::{AuditEventRecord, MutationWithAuditOutcome, SqlCipherStore};

    fn audit(sequence: u64, previous: &str) -> AuditEventRecord {
        AuditEventRecord {
            id: format!("audit-connection-{sequence}"),
            sequence,
            actor_identity_id: None,
            actor_role: "system".into(),
            action: "upstream_connection.update".into(),
            target_type: "upstream_connection".into(),
            target_id: Some("connection_test".into()),
            outcome: "succeeded".into(),
            previous_event_hmac: previous.into(),
            integrity_hmac: char::from(b'a' + sequence as u8).to_string().repeat(43),
            created_at: "2026-09-17T00:00:00.000Z".into(),
        }
    }

    #[test]
    fn api_key_connection_revision_guards_status_and_secret_rotation() {
        let directory = tempfile::tempdir().unwrap();
        let mut store =
            SqlCipherStore::initialize(&directory.path().join("customer.db"), &[0x5a; 32]).unwrap();
        let mut connection = ConnectionRecord {
            id: "connection_test".into(),
            provider: "deepseek".into(),
            channel_id: "deepseek.api".into(),
            endpoint_profile: "deepseek.api".into(),
            auth_scheme: "api_key".into(),
            billing_mode: "usage".into(),
            display_name: "Test".into(),
            legacy_account_id: None,
            status: "active".into(),
            revision: 1,
            created_at: "2026-09-17T00:00:00.000Z".into(),
            updated_at: "2026-09-17T00:00:00.000Z".into(),
        };
        let mut credential = ConnectionCredential {
            id: "credential_test".into(),
            connection_id: connection.id.clone(),
            credential_identity_hmac: "a".repeat(64),
            encrypted_payload: vec![1],
            payload_nonce: vec![2],
            wrapped_data_key: vec![3],
            wrap_nonce: vec![4],
            credential_revision: 0,
            status: "active".into(),
            created_at: connection.created_at.clone(),
            updated_at: connection.updated_at.clone(),
        };
        let model = ConnectionModelSpec {
            model: ModelRecord {
                id: "model_test".into(),
                public_name: "deepseek-flash".into(),
                display_name: "DeepSeek Flash".into(),
                enabled: false,
                discovered_at: None,
                created_at: connection.created_at.clone(),
            },
            upstream_name: "deepseek-flash".into(),
            quota_unit: "token".into(),
        };
        assert_eq!(
            store
                .insert_api_key_connection(&connection, &credential, &[model], 0, "", &audit(1, ""))
                .unwrap(),
            MutationWithAuditOutcome::Mutation(())
        );
        connection.status = "disabled".into();
        connection.revision = 2;
        assert_eq!(
            store
                .update_api_key_connection(
                    &connection,
                    None,
                    1,
                    1,
                    &"b".repeat(43),
                    &audit(2, &"b".repeat(43))
                )
                .unwrap(),
            MutationWithAuditOutcome::Mutation(ConnectionMutationOutcome::Updated)
        );
        assert!(
            store
                .api_key_connection_route("deepseek-flash")
                .unwrap()
                .is_none()
        );
        assert_eq!(
            store
                .update_api_key_connection(
                    &connection,
                    None,
                    1,
                    2,
                    &"c".repeat(43),
                    &audit(3, &"c".repeat(43))
                )
                .unwrap(),
            MutationWithAuditOutcome::Mutation(ConnectionMutationOutcome::Conflict)
        );
        connection.status = "active".into();
        connection.revision = 3;
        credential.credential_revision = 1;
        credential.credential_identity_hmac = "d".repeat(64);
        credential.encrypted_payload = vec![9];
        assert_eq!(
            store
                .update_api_key_connection(
                    &connection,
                    Some(&credential),
                    2,
                    2,
                    &"c".repeat(43),
                    &audit(3, &"c".repeat(43))
                )
                .unwrap(),
            MutationWithAuditOutcome::Mutation(ConnectionMutationOutcome::Updated)
        );
        let (stored, secret) = store
            .api_key_connection("connection_test")
            .unwrap()
            .unwrap();
        assert_eq!(stored.revision, 3);
        assert_eq!(secret.credential_revision, 1);
        assert_eq!(secret.encrypted_payload, vec![9]);
        let discovered = ConnectionModelSpec {
            model: ModelRecord {
                id: "model_discovered".into(),
                public_name: "deepseek-v4-pro".into(),
                display_name: "deepseek-v4-pro".into(),
                enabled: false,
                discovered_at: Some(connection.created_at.clone()),
                created_at: connection.created_at.clone(),
            },
            upstream_name: "deepseek-v4-pro".into(),
            quota_unit: "token".into(),
        };
        let sync = ConnectionModelSync {
            connection_id: connection.id.clone(),
            expected_revision: 3,
            models: vec![
                discovered.clone(),
                ConnectionModelSpec {
                    model: ModelRecord {
                        id: "model_duplicate_manual".into(),
                        public_name: "deepseek-flash".into(),
                        display_name: "deepseek-flash".into(),
                        enabled: false,
                        discovered_at: Some(connection.created_at.clone()),
                        created_at: connection.created_at.clone(),
                    },
                    upstream_name: "deepseek-flash".into(),
                    quota_unit: "token".into(),
                },
            ],
            now: connection.created_at.clone(),
        };
        assert_eq!(
            store
                .sync_api_key_connection_models(
                    &sync,
                    3,
                    &"d".repeat(43),
                    &audit(4, &"d".repeat(43))
                )
                .unwrap(),
            MutationWithAuditOutcome::Mutation(ConnectionModelSyncOutcome::Applied { added: 1 })
        );
        assert!(
            store
                .api_key_connection_route("deepseek-v4-pro")
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .update_model_enabled("model_discovered", true)
                .unwrap()
        );
        assert_eq!(
            store.enabled_models().unwrap(),
            vec![ModelRecord {
                enabled: true,
                ..discovered.model.clone()
            }]
        );
        assert_eq!(
            store
                .enabled_model_by_public_name("deepseek-v4-pro")
                .unwrap()
                .map(|model| model.id),
            Some("model_discovered".into())
        );
        assert!(
            store
                .api_key_connection_route("deepseek-v4-pro")
                .unwrap()
                .is_some()
        );
        let stale = store
            .sync_api_key_connection_models(&sync, 4, &"e".repeat(43), &audit(5, &"e".repeat(43)))
            .unwrap();
        assert_eq!(
            stale,
            MutationWithAuditOutcome::Mutation(ConnectionModelSyncOutcome::Conflict)
        );
        let mut repeat = sync;
        repeat.expected_revision = 4;
        assert_eq!(
            store
                .sync_api_key_connection_models(
                    &repeat,
                    4,
                    &"e".repeat(43),
                    &audit(5, &"e".repeat(43))
                )
                .unwrap(),
            MutationWithAuditOutcome::Mutation(ConnectionModelSyncOutcome::Applied { added: 0 })
        );

        let manual = ConnectionModelSync {
            connection_id: connection.id.clone(),
            expected_revision: 5,
            models: vec![ConnectionModelSpec {
                model: ModelRecord {
                    id: "model_manual_later".into(),
                    public_name: "manual-later".into(),
                    display_name: "Manual later".into(),
                    enabled: false,
                    discovered_at: None,
                    created_at: connection.created_at.clone(),
                },
                upstream_name: "manual-later".into(),
                quota_unit: "token".into(),
            }],
            now: connection.created_at.clone(),
        };
        assert_eq!(
            store
                .add_api_key_connection_models(
                    &manual,
                    5,
                    &"f".repeat(43),
                    &audit(6, &"f".repeat(43))
                )
                .unwrap(),
            MutationWithAuditOutcome::Mutation(ConnectionModelSyncOutcome::Applied { added: 1 })
        );
        assert!(
            store
                .api_key_connection_route("manual-later")
                .unwrap()
                .is_none()
        );
        let duplicate = ConnectionModelSync {
            expected_revision: 6,
            ..manual
        };
        assert_eq!(
            store
                .add_api_key_connection_models(
                    &duplicate,
                    6,
                    &"g".repeat(43),
                    &audit(7, &"g".repeat(43))
                )
                .unwrap(),
            MutationWithAuditOutcome::Mutation(ConnectionModelSyncOutcome::AlreadyBound)
        );
        assert_eq!(
            store
                .api_key_connection("connection_test")
                .unwrap()
                .unwrap()
                .0
                .revision,
            6
        );
    }
}
