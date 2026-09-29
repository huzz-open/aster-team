//! Complete per-member model grant sets. Callers verify the signed header
//! against the sorted grants before using a record for admission.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelAccessPolicyRecord {
    pub identity_id: String,
    pub mode: String,
    pub revision: i64,
    pub grant_set_digest: String,
    pub integrity_hmac: String,
    pub updated_by: String,
    pub updated_at: String,
    pub grants: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelAccessWriteOutcome {
    Applied,
    RevisionConflict,
    IdentityNotFound,
    ModelNotFound,
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
mod sqlite {
    use super::*;
    use crate::{
        AuditEventRecord, MutationWithAuditOutcome, SqlCipherStore, StorageError,
        append_audit_events_sqlite,
    };
    use rusqlite::{OptionalExtension, TransactionBehavior, params};

    impl SqlCipherStore {
        pub fn admit_model_attempt(
            &mut self,
            policy: &ModelAccessPolicyRecord,
            model_id: &str,
            attempt_id: &str,
            request_id: &str,
            admitted_at: &str,
        ) -> Result<bool, StorageError> {
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let current: Option<(String,i64,String,String)> = transaction.query_row(
                "SELECT mode,revision,grant_set_digest,integrity_hmac FROM identity_model_policies WHERE identity_id=?",
                [&policy.identity_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
            ).optional()?;
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
            let allowed: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM models m WHERE m.id=? AND m.enabled=1 AND
                 (?='all_enabled' OR EXISTS(SELECT 1 FROM identity_model_grants g
                   WHERE g.identity_id=? AND g.public_model_id=m.id)))",
                params![model_id, policy.mode, policy.identity_id],
                |row| row.get(0),
            )?;
            if !allowed {
                return Ok(false);
            }
            transaction.execute(
                "INSERT INTO model_attempt_admissions(attempt_id,identity_id,public_model_id,policy_revision,request_id,admitted_at)
                 VALUES(?,?,?,?,?,?)",
                params![attempt_id,policy.identity_id,model_id,policy.revision,request_id,admitted_at],
            )?;
            transaction.commit()?;
            Ok(true)
        }

        pub fn model_access_policy(
            &self,
            identity_id: &str,
        ) -> Result<Option<ModelAccessPolicyRecord>, StorageError> {
            let mut policy = self.connection.query_row(
                "SELECT identity_id,mode,revision,grant_set_digest,integrity_hmac,updated_by,updated_at
                 FROM identity_model_policies WHERE identity_id=?",
                [identity_id],
                |row| Ok(ModelAccessPolicyRecord {
                    identity_id: row.get(0)?, mode: row.get(1)?, revision: row.get(2)?,
                    grant_set_digest: row.get(3)?, integrity_hmac: row.get(4)?,
                    updated_by: row.get(5)?, updated_at: row.get(6)?, grants: Vec::new(),
                }),
            ).optional()?;
            if let Some(policy) = &mut policy {
                let mut statement = self.connection.prepare(
                    "SELECT public_model_id FROM identity_model_grants WHERE identity_id=? ORDER BY public_model_id",
                )?;
                policy.grants = statement
                    .query_map([identity_id], |row| row.get(0))?
                    .collect::<Result<Vec<_>, _>>()?;
            }
            Ok(policy)
        }

        pub fn pending_model_access_policies(
            &self,
        ) -> Result<Vec<ModelAccessPolicyRecord>, StorageError> {
            let mut statement = self.connection.prepare(
                "SELECT identity_id FROM identity_model_policies WHERE updated_by='migration'
                 AND integrity_hmac='' ORDER BY identity_id LIMIT 10000",
            )?;
            let ids = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            ids.iter()
                .map(|id| {
                    self.model_access_policy(id)?
                        .ok_or(StorageError::MigrationIntegrity)
                })
                .collect()
        }

        pub fn finalize_model_access_migration(
            &mut self,
            expected: &ModelAccessPolicyRecord,
            digest: &str,
            hmac: &str,
        ) -> Result<bool, StorageError> {
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let current: Option<(String,i64,String)> = transaction.query_row(
                "SELECT mode,revision,integrity_hmac FROM identity_model_policies WHERE identity_id=?",
                [&expected.identity_id],
                |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
            ).optional()?;
            if current != Some(("selected".to_owned(), 0, String::new())) {
                transaction.rollback()?;
                return Ok(false);
            }
            let grants = {
                let mut statement = transaction.prepare(
                    "SELECT public_model_id FROM identity_model_grants WHERE identity_id=? ORDER BY public_model_id",
                )?;
                statement
                    .query_map([&expected.identity_id], |row| row.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?
            };
            if grants != expected.grants {
                transaction.rollback()?;
                return Ok(false);
            }
            let updated = transaction.execute(
                "UPDATE identity_model_policies SET grant_set_digest=?,integrity_hmac=?
                 WHERE identity_id=? AND integrity_hmac='' AND updated_by='migration'",
                params![digest, hmac, expected.identity_id],
            )? == 1;
            transaction.commit()?;
            Ok(updated)
        }

        pub fn model_id_by_public_name(&self, name: &str) -> Result<Option<String>, StorageError> {
            self.connection
                .query_row(
                    "SELECT id FROM models WHERE public_name=? AND enabled=1",
                    [name],
                    |row| row.get(0),
                )
                .optional()
                .map_err(StorageError::from)
        }

        pub fn replace_model_access_policy(
            &mut self,
            policy: &ModelAccessPolicyRecord,
            expected_revision: i64,
            expected_audit_sequence: u64,
            expected_audit_hmac: &str,
            audit_event: &AuditEventRecord,
        ) -> Result<MutationWithAuditOutcome<ModelAccessWriteOutcome>, StorageError> {
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let exists: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM identities WHERE id=? AND role='member' AND status<>'deleted')",
                [&policy.identity_id], |row| row.get(0),
            )?;
            if !exists {
                return Ok(MutationWithAuditOutcome::Mutation(
                    ModelAccessWriteOutcome::IdentityNotFound,
                ));
            }
            let current: Option<i64> = transaction
                .query_row(
                    "SELECT revision FROM identity_model_policies WHERE identity_id=?",
                    [&policy.identity_id],
                    |row| row.get(0),
                )
                .optional()?;
            if current.unwrap_or(0) != expected_revision {
                return Ok(MutationWithAuditOutcome::Mutation(
                    ModelAccessWriteOutcome::RevisionConflict,
                ));
            }
            for id in &policy.grants {
                let exists: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM models WHERE id=?)",
                    [id],
                    |row| row.get(0),
                )?;
                if !exists {
                    return Ok(MutationWithAuditOutcome::Mutation(
                        ModelAccessWriteOutcome::ModelNotFound,
                    ));
                }
            }
            if current.is_some() {
                transaction.execute(
                    "DELETE FROM identity_model_grants WHERE identity_id=?",
                    [&policy.identity_id],
                )?;
                transaction.execute(
                    "UPDATE identity_model_policies SET mode=?,revision=?,grant_set_digest=?,integrity_hmac=?,updated_by=?,updated_at=?
                     WHERE identity_id=? AND revision=?",
                    params![policy.mode,policy.revision,policy.grant_set_digest,policy.integrity_hmac,
                        policy.updated_by,policy.updated_at,policy.identity_id,expected_revision],
                )?;
            } else {
                transaction.execute(
                    "INSERT INTO identity_model_policies(identity_id,mode,revision,grant_set_digest,integrity_hmac,updated_by,updated_at)
                     VALUES(?,?,?,?,?,?,?)",
                    params![policy.identity_id,policy.mode,policy.revision,policy.grant_set_digest,
                        policy.integrity_hmac,policy.updated_by,policy.updated_at],
                )?;
            }
            for id in &policy.grants {
                transaction.execute(
                    "INSERT INTO identity_model_grants(identity_id,public_model_id) VALUES(?,?)",
                    params![policy.identity_id, id],
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
                ModelAccessWriteOutcome::Applied,
            ))
        }
    }
}

#[cfg(feature = "mariadb")]
mod mariadb;
