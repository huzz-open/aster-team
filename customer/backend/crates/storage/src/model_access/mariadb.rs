use super::{ModelAccessPolicyRecord, ModelAccessWriteOutcome};
use crate::mariadb::{append_audit_events_mariadb, checked_i64};
use crate::{AuditEventRecord, MariaDbStore, MutationWithAuditOutcome, StorageError};
use sqlx::Row;

impl MariaDbStore {
    pub async fn admit_model_attempt(
        &self,
        policy: &ModelAccessPolicyRecord,
        model_id: &str,
        attempt_id: &str,
        request_id: &str,
        admitted_at: &str,
    ) -> Result<bool, StorageError> {
        let mut transaction = self.pool().begin().await?;
        let current: Option<(String,u64,String,String)> = sqlx::query_as(
            "SELECT mode,revision,grant_set_digest,integrity_hmac FROM identity_model_policies WHERE identity_id=? FOR UPDATE",
        ).bind(&policy.identity_id).fetch_optional(&mut *transaction).await?;
        let current = current
            .map(|(mode, revision, digest, hmac)| {
                Ok::<_, sqlx::Error>((mode, checked_i64(revision)?, digest, hmac))
            })
            .transpose()?;
        if current
            != Some((
                policy.mode.clone(),
                policy.revision,
                policy.grant_set_digest.clone(),
                policy.integrity_hmac.clone(),
            ))
        {
            return Ok(false);
        }
        let allowed: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM models m WHERE m.id=? AND m.enabled=1 AND
             (?='all_enabled' OR EXISTS(SELECT 1 FROM identity_model_grants g
                WHERE g.identity_id=? AND g.public_model_id=m.id))",
        )
        .bind(model_id)
        .bind(&policy.mode)
        .bind(&policy.identity_id)
        .fetch_one(&mut *transaction)
        .await?;
        if allowed != 1 {
            return Ok(false);
        }
        sqlx::query(
            "INSERT INTO model_attempt_admissions(attempt_id,identity_id,public_model_id,policy_revision,request_id,admitted_at)
             VALUES(?,?,?,?,?,?)",
        ).bind(attempt_id).bind(&policy.identity_id).bind(model_id).bind(policy.revision)
            .bind(request_id).bind(admitted_at).execute(&mut *transaction).await?;
        transaction.commit().await?;
        Ok(true)
    }

    pub async fn model_access_policy(
        &self,
        identity_id: &str,
    ) -> Result<Option<ModelAccessPolicyRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT identity_id,mode,revision,grant_set_digest,integrity_hmac,updated_by,updated_at
             FROM identity_model_policies WHERE identity_id=?",
        )
        .bind(identity_id)
        .fetch_optional(self.pool())
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let grants = sqlx::query_scalar::<_,String>(
            "SELECT public_model_id FROM identity_model_grants WHERE identity_id=? ORDER BY public_model_id",
        )
        .bind(identity_id)
        .fetch_all(self.pool())
        .await?;
        Ok(Some(ModelAccessPolicyRecord {
            identity_id: row.try_get("identity_id")?,
            mode: row.try_get("mode")?,
            revision: checked_i64(row.try_get::<u64, _>("revision")?)?,
            grant_set_digest: row.try_get("grant_set_digest")?,
            integrity_hmac: row.try_get("integrity_hmac")?,
            updated_by: row.try_get("updated_by")?,
            updated_at: row.try_get("updated_at")?,
            grants,
        }))
    }

    pub async fn pending_model_access_policies(
        &self,
    ) -> Result<Vec<ModelAccessPolicyRecord>, StorageError> {
        let ids = sqlx::query_scalar::<_, String>(
            "SELECT identity_id FROM identity_model_policies WHERE updated_by='migration'
             AND integrity_hmac='' ORDER BY identity_id LIMIT 10000",
        )
        .fetch_all(self.pool())
        .await?;
        let mut policies = Vec::with_capacity(ids.len());
        for id in ids {
            policies.push(
                self.model_access_policy(&id)
                    .await?
                    .ok_or(StorageError::MigrationIntegrity)?,
            );
        }
        Ok(policies)
    }

    pub async fn finalize_model_access_migration(
        &self,
        expected: &ModelAccessPolicyRecord,
        digest: &str,
        hmac: &str,
    ) -> Result<bool, StorageError> {
        let mut transaction = self.pool().begin().await?;
        let current: Option<(String,u64,String)> = sqlx::query_as(
            "SELECT mode,revision,integrity_hmac FROM identity_model_policies WHERE identity_id=? FOR UPDATE",
        )
        .bind(&expected.identity_id)
        .fetch_optional(&mut *transaction)
        .await?;
        if current != Some(("selected".to_owned(), 0, String::new())) {
            transaction.rollback().await?;
            return Ok(false);
        }
        let grants = sqlx::query_scalar::<_,String>(
            "SELECT public_model_id FROM identity_model_grants WHERE identity_id=? ORDER BY public_model_id",
        )
        .bind(&expected.identity_id)
        .fetch_all(&mut *transaction)
        .await?;
        if grants != expected.grants {
            transaction.rollback().await?;
            return Ok(false);
        }
        let updated = sqlx::query(
            "UPDATE identity_model_policies SET grant_set_digest=?,integrity_hmac=?
             WHERE identity_id=? AND integrity_hmac='' AND updated_by='migration'",
        )
        .bind(digest)
        .bind(hmac)
        .bind(&expected.identity_id)
        .execute(&mut *transaction)
        .await?
        .rows_affected()
            == 1;
        transaction.commit().await?;
        Ok(updated)
    }

    pub async fn model_id_by_public_name(
        &self,
        name: &str,
    ) -> Result<Option<String>, StorageError> {
        sqlx::query_scalar("SELECT id FROM models WHERE public_name=? AND enabled=1")
            .bind(name)
            .fetch_optional(self.pool())
            .await
            .map_err(StorageError::from)
    }

    pub async fn replace_model_access_policy(
        &self,
        policy: &ModelAccessPolicyRecord,
        expected_revision: i64,
        expected_audit_sequence: u64,
        expected_audit_hmac: &str,
        audit_event: &AuditEventRecord,
    ) -> Result<MutationWithAuditOutcome<ModelAccessWriteOutcome>, StorageError> {
        let mut transaction = self.pool().begin().await?;
        let identity: Option<String> = sqlx::query_scalar(
            "SELECT id FROM identities WHERE id=? AND role='member' AND status<>'deleted' FOR UPDATE",
        )
        .bind(&policy.identity_id)
        .fetch_optional(&mut *transaction)
        .await?;
        if identity.is_none() {
            return Ok(MutationWithAuditOutcome::Mutation(
                ModelAccessWriteOutcome::IdentityNotFound,
            ));
        }
        let current: Option<u64> = sqlx::query_scalar(
            "SELECT revision FROM identity_model_policies WHERE identity_id=? FOR UPDATE",
        )
        .bind(&policy.identity_id)
        .fetch_optional(&mut *transaction)
        .await?;
        let current = current.map(checked_i64).transpose()?;
        if current.unwrap_or(0) != expected_revision {
            return Ok(MutationWithAuditOutcome::Mutation(
                ModelAccessWriteOutcome::RevisionConflict,
            ));
        }
        for id in &policy.grants {
            let exists: Option<String> = sqlx::query_scalar("SELECT id FROM models WHERE id=?")
                .bind(id)
                .fetch_optional(&mut *transaction)
                .await?;
            if exists.is_none() {
                return Ok(MutationWithAuditOutcome::Mutation(
                    ModelAccessWriteOutcome::ModelNotFound,
                ));
            }
        }
        if current.is_some() {
            sqlx::query("DELETE FROM identity_model_grants WHERE identity_id=?")
                .bind(&policy.identity_id)
                .execute(&mut *transaction)
                .await?;
            sqlx::query(
                "UPDATE identity_model_policies SET mode=?,revision=?,grant_set_digest=?,integrity_hmac=?,updated_by=?,updated_at=?
                 WHERE identity_id=? AND revision=?",
            )
            .bind(&policy.mode).bind(policy.revision).bind(&policy.grant_set_digest)
            .bind(&policy.integrity_hmac).bind(&policy.updated_by).bind(&policy.updated_at)
            .bind(&policy.identity_id).bind(expected_revision)
            .execute(&mut *transaction).await?;
        } else {
            sqlx::query(
                "INSERT INTO identity_model_policies(identity_id,mode,revision,grant_set_digest,integrity_hmac,updated_by,updated_at)
                 VALUES(?,?,?,?,?,?,?)",
            )
            .bind(&policy.identity_id).bind(&policy.mode).bind(policy.revision)
            .bind(&policy.grant_set_digest).bind(&policy.integrity_hmac)
            .bind(&policy.updated_by).bind(&policy.updated_at)
            .execute(&mut *transaction).await?;
        }
        for id in &policy.grants {
            sqlx::query(
                "INSERT INTO identity_model_grants(identity_id,public_model_id) VALUES(?,?)",
            )
            .bind(&policy.identity_id)
            .bind(id)
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
            ModelAccessWriteOutcome::Applied,
        ))
    }
}
