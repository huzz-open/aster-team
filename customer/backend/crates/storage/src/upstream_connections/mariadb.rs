use super::{
    ConnectionCredential, ConnectionModelBinding, ConnectionModelOrigin, ConnectionModelSpec,
    ConnectionModelSync, ConnectionModelSyncOutcome, ConnectionMutationOutcome, ConnectionRecord,
    ConnectionRoute, valid_connection,
};
use crate::mariadb::append_audit_events_mariadb;
use crate::{AuditEventRecord, MariaDbStore, MutationWithAuditOutcome, StorageError};
use sqlx::{Row, mysql::MySqlRow};

impl MariaDbStore {
    pub async fn connection_model_origin(
        &self,
        model_id: &str,
    ) -> Result<Option<ConnectionModelOrigin>, StorageError> {
        let row = sqlx::query(
            "SELECT c.provider,b.upstream_name FROM upstream_connection_models b
             JOIN upstream_connections c ON c.id=b.connection_id WHERE b.model_id=?
             ORDER BY b.enabled DESC,(c.status='active') DESC,c.updated_at DESC,c.id LIMIT 1",
        )
        .bind(model_id)
        .fetch_optional(self.pool())
        .await?;
        match row {
            Some(row) => Ok(Some(ConnectionModelOrigin {
                provider: row.try_get("provider")?,
                upstream_name: row.try_get("upstream_name")?,
            })),
            None => Ok(None),
        }
    }

    pub async fn insert_api_key_connection(
        &self,
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
        let mut transaction = self.pool().begin().await?;
        sqlx::query(
            "INSERT INTO upstream_connections(id,provider,channel_id,endpoint_profile,auth_scheme,
             billing_mode,display_name,legacy_account_id,status,revision,created_at,updated_at)
             VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&connection.id)
        .bind(&connection.provider)
        .bind(&connection.channel_id)
        .bind(&connection.endpoint_profile)
        .bind(&connection.auth_scheme)
        .bind(&connection.billing_mode)
        .bind(&connection.display_name)
        .bind(&connection.legacy_account_id)
        .bind(&connection.status)
        .bind(connection.revision)
        .bind(&connection.created_at)
        .bind(&connection.updated_at)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO upstream_connection_credentials(id,connection_id,credential_identity_hmac,
             encrypted_payload,payload_nonce,wrapped_data_key,wrap_nonce,credential_revision,
             status,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&credential.id)
        .bind(&credential.connection_id)
        .bind(&credential.credential_identity_hmac)
        .bind(&credential.encrypted_payload)
        .bind(&credential.payload_nonce)
        .bind(&credential.wrapped_data_key)
        .bind(&credential.wrap_nonce)
        .bind(credential.credential_revision)
        .bind(&credential.status)
        .bind(&credential.created_at)
        .bind(&credential.updated_at)
        .execute(&mut *transaction)
        .await?;
        for spec in models {
            let model = &spec.model;
            let upstream_name = &spec.upstream_name;
            if !matches!(spec.quota_unit.as_str(), "token" | "image") {
                return Err(StorageError::MigrationIntegrity);
            }
            sqlx::query(
                "INSERT INTO models(id,public_name,display_name,enabled,discovered_at,created_at)
                 VALUES(?,?,?,0,NULL,?) ON DUPLICATE KEY UPDATE id=id",
            )
            .bind(&model.id)
            .bind(&model.public_name)
            .bind(&model.display_name)
            .bind(&model.created_at)
            .execute(&mut *transaction)
            .await?;
            let model_id: String = sqlx::query_scalar("SELECT id FROM models WHERE public_name=?")
                .bind(&model.public_name)
                .fetch_one(&mut *transaction)
                .await?;
            if spec.quota_unit == "image" {
                sqlx::query(
                    "INSERT INTO model_quota_units(model_id,quota_unit) VALUES(?,'image')
                    ON DUPLICATE KEY UPDATE model_id=model_id",
                )
                .bind(&model_id)
                .execute(&mut *transaction)
                .await?;
            } else {
                let image_unit: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM model_quota_units WHERE model_id=?")
                        .bind(&model_id)
                        .fetch_one(&mut *transaction)
                        .await?;
                if image_unit != 0 {
                    return Err(StorageError::MigrationIntegrity);
                }
            }
            sqlx::query(
                "INSERT INTO upstream_connection_models(connection_id,model_id,upstream_name,
                 enabled,capabilities_json,discovered_at) VALUES(?,?,?,1,'{}',?)",
            )
            .bind(&connection.id)
            .bind(&model_id)
            .bind(upstream_name)
            .bind(&connection.created_at)
            .execute(&mut *transaction)
            .await?;
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
        Ok(MutationWithAuditOutcome::Mutation(()))
    }

    pub async fn bind_connection_model(
        &self,
        binding: &ConnectionModelBinding,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO upstream_connection_models(connection_id,model_id,upstream_name,
             enabled,capabilities_json,discovered_at) VALUES(?,?,?,?,?,?)
             ON DUPLICATE KEY UPDATE upstream_name=VALUES(upstream_name),enabled=VALUES(enabled),
             capabilities_json=VALUES(capabilities_json),discovered_at=VALUES(discovered_at)",
        )
        .bind(&binding.connection_id)
        .bind(&binding.model_id)
        .bind(&binding.upstream_name)
        .bind(binding.enabled)
        .bind(&binding.capabilities_json)
        .bind(&binding.discovered_at)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn list_api_key_connections(&self) -> Result<Vec<ConnectionRecord>, StorageError> {
        sqlx::query(
            "SELECT id,provider,channel_id,endpoint_profile,auth_scheme,billing_mode,
             display_name,legacy_account_id,status,revision,created_at,updated_at
             FROM upstream_connections WHERE auth_scheme='api_key' ORDER BY created_at,id",
        )
        .fetch_all(self.pool())
        .await?
        .iter()
        .map(connection_from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(StorageError::from)
    }

    pub async fn api_key_connection(
        &self,
        id: &str,
    ) -> Result<Option<(ConnectionRecord, ConnectionCredential)>, StorageError> {
        let row = sqlx::query(
            "SELECT c.id,c.provider,c.channel_id,c.endpoint_profile,c.auth_scheme,c.billing_mode,
             c.display_name,c.legacy_account_id,c.status,c.revision,c.created_at,c.updated_at,
             k.id,k.connection_id,k.credential_identity_hmac,k.encrypted_payload,k.payload_nonce,
             k.wrapped_data_key,k.wrap_nonce,k.credential_revision,k.status,k.created_at,k.updated_at
             FROM upstream_connections c JOIN upstream_connection_credentials k ON k.connection_id=c.id
             WHERE c.id=? AND c.auth_scheme='api_key'",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        row.map(|row| {
            Ok::<_, sqlx::Error>((connection_from_row(&row)?, credential_from_row(&row)?))
        })
        .transpose()
        .map_err(StorageError::from)
    }

    pub async fn update_api_key_connection(
        &self,
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
        let mut transaction = self.pool().begin().await?;
        let changed = sqlx::query(
            "UPDATE upstream_connections SET display_name=?,status=?,revision=?,updated_at=?
             WHERE id=? AND revision=? AND auth_scheme='api_key'",
        )
        .bind(&connection.display_name)
        .bind(&connection.status)
        .bind(connection.revision)
        .bind(&connection.updated_at)
        .bind(&connection.id)
        .bind(expected_revision)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if changed == 0 {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                ConnectionMutationOutcome::Conflict,
            ));
        }
        if let Some(credential) = credential {
            let changed = sqlx::query(
                "UPDATE upstream_connection_credentials SET credential_identity_hmac=?,encrypted_payload=?,
                 payload_nonce=?,wrapped_data_key=?,wrap_nonce=?,credential_revision=?,status='active',updated_at=?
                 WHERE connection_id=? AND id=? AND credential_revision=?",
            )
            .bind(&credential.credential_identity_hmac)
            .bind(&credential.encrypted_payload)
            .bind(&credential.payload_nonce)
            .bind(&credential.wrapped_data_key)
            .bind(&credential.wrap_nonce)
            .bind(credential.credential_revision)
            .bind(&credential.updated_at)
            .bind(&connection.id)
            .bind(&credential.id)
            .bind(credential.credential_revision - 1)
            .execute(&mut *transaction)
            .await?
            .rows_affected();
            if changed != 1 {
                transaction.rollback().await?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    ConnectionMutationOutcome::Conflict,
                ));
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
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::AuditConflict);
        }
        transaction.commit().await?;
        Ok(MutationWithAuditOutcome::Mutation(
            ConnectionMutationOutcome::Updated,
        ))
    }

    pub async fn sync_api_key_connection_models(
        &self,
        input: &ConnectionModelSync,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<MutationWithAuditOutcome<ConnectionModelSyncOutcome>, StorageError> {
        if input.models.len() > 256 || input.models.iter().any(|model| model.quota_unit != "token")
        {
            return Err(StorageError::MigrationIntegrity);
        }
        let mut transaction = self.pool().begin().await?;
        let changed = sqlx::query(
            "UPDATE upstream_connections SET revision=revision+1,updated_at=?
             WHERE id=? AND revision=? AND auth_scheme='api_key' AND status='active'",
        )
        .bind(&input.now)
        .bind(&input.connection_id)
        .bind(input.expected_revision)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if changed == 0 {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                ConnectionModelSyncOutcome::Conflict,
            ));
        }
        let mut added = 0;
        for spec in &input.models {
            let already_bound: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM upstream_connection_models
                 WHERE connection_id=? AND upstream_name=?",
            )
            .bind(&input.connection_id)
            .bind(&spec.upstream_name)
            .fetch_one(&mut *transaction)
            .await?;
            if already_bound != 0 {
                continue;
            }
            let model = &spec.model;
            sqlx::query(
                "INSERT INTO models(id,public_name,display_name,enabled,discovered_at,created_at)
                 VALUES(?,?,?,0,?,?) ON DUPLICATE KEY UPDATE id=id",
            )
            .bind(&model.id)
            .bind(&model.public_name)
            .bind(&model.display_name)
            .bind(&input.now)
            .bind(&input.now)
            .execute(&mut *transaction)
            .await?;
            let model_id: String = sqlx::query_scalar("SELECT id FROM models WHERE public_name=?")
                .bind(&model.public_name)
                .fetch_one(&mut *transaction)
                .await?;
            let existing_binding: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM upstream_connection_models
                 WHERE connection_id=? AND model_id=?",
            )
            .bind(&input.connection_id)
            .bind(&model_id)
            .fetch_one(&mut *transaction)
            .await?;
            if existing_binding != 0 {
                continue;
            }
            sqlx::query(
                "INSERT INTO upstream_connection_models(connection_id,model_id,upstream_name,
                 enabled,capabilities_json,discovered_at) VALUES(?,?,?,1,'{}',?)",
            )
            .bind(&input.connection_id)
            .bind(&model_id)
            .bind(&spec.upstream_name)
            .bind(&input.now)
            .execute(&mut *transaction)
            .await?;
            added += 1;
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
            ConnectionModelSyncOutcome::Applied { added },
        ))
    }

    pub async fn add_api_key_connection_models(
        &self,
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
        let mut transaction = self.pool().begin().await?;
        let changed = sqlx::query(
            "UPDATE upstream_connections SET revision=revision+1,updated_at=?
             WHERE id=? AND revision=? AND auth_scheme='api_key' AND status='active'",
        )
        .bind(&input.now)
        .bind(&input.connection_id)
        .bind(input.expected_revision)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if changed == 0 {
            transaction.rollback().await?;
            return Ok(MutationWithAuditOutcome::Mutation(
                ConnectionModelSyncOutcome::Conflict,
            ));
        }
        for spec in &input.models {
            let existing_upstream: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM upstream_connection_models WHERE connection_id=? AND upstream_name=?",
            )
            .bind(&input.connection_id)
            .bind(&spec.upstream_name)
            .fetch_one(&mut *transaction)
            .await?;
            if existing_upstream != 0 {
                transaction.rollback().await?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    ConnectionModelSyncOutcome::AlreadyBound,
                ));
            }
            let model = &spec.model;
            let inserted = sqlx::query(
                "INSERT INTO models(id,public_name,display_name,enabled,discovered_at,created_at)
                 VALUES(?,?,?,0,NULL,?) ON DUPLICATE KEY UPDATE id=id",
            )
            .bind(&model.id)
            .bind(&model.public_name)
            .bind(&model.display_name)
            .bind(&model.created_at)
            .execute(&mut *transaction)
            .await?
            .rows_affected()
                == 1;
            let model_id: String = sqlx::query_scalar("SELECT id FROM models WHERE public_name=?")
                .bind(&model.public_name)
                .fetch_one(&mut *transaction)
                .await?;
            let existing_binding: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM upstream_connection_models WHERE connection_id=? AND model_id=?",
            )
            .bind(&input.connection_id)
            .bind(&model_id)
            .fetch_one(&mut *transaction)
            .await?;
            let image_unit: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM model_quota_units WHERE model_id=?")
                    .bind(&model_id)
                    .fetch_one(&mut *transaction)
                    .await?;
            if existing_binding != 0
                || (image_unit != 0 && spec.quota_unit != "image")
                || (!inserted && image_unit == 0 && spec.quota_unit == "image")
            {
                transaction.rollback().await?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    ConnectionModelSyncOutcome::AlreadyBound,
                ));
            }
            if spec.quota_unit == "image" {
                sqlx::query(
                    "INSERT INTO model_quota_units(model_id,quota_unit) VALUES(?,'image')
                     ON DUPLICATE KEY UPDATE model_id=model_id",
                )
                .bind(&model_id)
                .execute(&mut *transaction)
                .await?;
            }
            sqlx::query(
                "INSERT INTO upstream_connection_models(connection_id,model_id,upstream_name,
                 enabled,capabilities_json,discovered_at) VALUES(?,?,?,1,'{}',?)",
            )
            .bind(&input.connection_id)
            .bind(&model_id)
            .bind(&spec.upstream_name)
            .bind(&input.now)
            .execute(&mut *transaction)
            .await?;
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
            ConnectionModelSyncOutcome::Applied {
                added: input.models.len(),
            },
        ))
    }

    pub async fn api_key_connection_route(
        &self,
        public_model: &str,
    ) -> Result<Option<ConnectionRoute>, StorageError> {
        let row = sqlx::query(
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
        )
        .bind(public_model)
        .fetch_optional(self.pool())
        .await?;
        row.map(|row| {
            Ok::<ConnectionRoute, sqlx::Error>(ConnectionRoute {
                connection: connection_from_row(&row)?,
                credential: credential_from_row(&row)?,
                binding: binding_from_row(&row)?,
            })
        })
        .transpose()
        .map_err(StorageError::from)
    }
}

fn connection_from_row(row: &MySqlRow) -> Result<ConnectionRecord, sqlx::Error> {
    Ok(ConnectionRecord {
        id: row.try_get(0)?,
        provider: row.try_get(1)?,
        channel_id: row.try_get(2)?,
        endpoint_profile: row.try_get(3)?,
        auth_scheme: row.try_get(4)?,
        billing_mode: row.try_get(5)?,
        display_name: row.try_get(6)?,
        legacy_account_id: row.try_get(7)?,
        status: row.try_get(8)?,
        revision: row.try_get(9)?,
        created_at: row.try_get(10)?,
        updated_at: row.try_get(11)?,
    })
}

fn credential_from_row(row: &MySqlRow) -> Result<ConnectionCredential, sqlx::Error> {
    Ok(ConnectionCredential {
        id: row.try_get(12)?,
        connection_id: row.try_get(13)?,
        credential_identity_hmac: row.try_get(14)?,
        encrypted_payload: row.try_get(15)?,
        payload_nonce: row.try_get(16)?,
        wrapped_data_key: row.try_get(17)?,
        wrap_nonce: row.try_get(18)?,
        credential_revision: row.try_get(19)?,
        status: row.try_get(20)?,
        created_at: row.try_get(21)?,
        updated_at: row.try_get(22)?,
    })
}

fn binding_from_row(row: &MySqlRow) -> Result<ConnectionModelBinding, sqlx::Error> {
    Ok(ConnectionModelBinding {
        connection_id: row.try_get(23)?,
        model_id: row.try_get(24)?,
        upstream_name: row.try_get(25)?,
        enabled: row.try_get(26)?,
        capabilities_json: row.try_get(27)?,
        discovered_at: row.try_get(28)?,
    })
}
