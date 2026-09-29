use super::*;
use crate::mariadb::{append_audit_events_mariadb, checked_i64};
use crate::{AuditEventRecord, MariaDbStore};
use sqlx::Row;

impl MariaDbStore {
    pub async fn expired_image_reservations(
        &self,
        expired_before: &str,
    ) -> Result<Vec<ImageReservationRecord>, crate::StorageError> {
        let rows = sqlx::query(
            "SELECT id,identity_id,api_key_id,public_model_id,request_id,reserved_images,
             confirmed_images,released_images,status,revision,integrity_hmac,
             created_at,expires_at,settled_at FROM image_quota_reservations
             WHERE status='reserved' AND expires_at<=?
             ORDER BY created_at LIMIT 200",
        )
        .bind(expired_before)
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ImageReservationRecord {
                    id: row.try_get("id")?,
                    identity_id: row.try_get("identity_id")?,
                    api_key_id: row.try_get("api_key_id")?,
                    public_model_id: row.try_get("public_model_id")?,
                    request_id: row.try_get("request_id")?,
                    reserved_images: row.try_get("reserved_images")?,
                    confirmed_images: row.try_get("confirmed_images")?,
                    released_images: row.try_get("released_images")?,
                    status: row.try_get("status")?,
                    revision: checked_i64(row.try_get::<u64, _>("revision")?)?,
                    integrity_hmac: row.try_get("integrity_hmac")?,
                    created_at: row.try_get("created_at")?,
                    expires_at: row.try_get("expires_at")?,
                    settled_at: row.try_get("settled_at")?,
                })
            })
            .collect()
    }

    pub async fn image_quota_unit(
        &self,
        model_id: &str,
    ) -> Result<Option<String>, crate::StorageError> {
        let row = sqlx::query(
            "SELECT CASE WHEN u.quota_unit='image' THEN 'image' ELSE 'token' END AS quota_unit
             FROM models m LEFT JOIN model_quota_units u ON u.model_id=m.id WHERE m.id=?",
        )
        .bind(model_id)
        .fetch_optional(self.pool())
        .await?;
        row.map(|row| row.try_get("quota_unit").map_err(crate::StorageError::from))
            .transpose()
    }

    pub async fn image_balance(
        &self,
        identity_id: &str,
        model_id: &str,
    ) -> Result<Option<ImageBalanceRecord>, crate::StorageError> {
        let row = sqlx::query(
            "SELECT identity_id,public_model_id,available_images,reserved_images,consumed_images,
             revision,last_ledger_hmac,integrity_hmac FROM member_image_balances
             WHERE identity_id=? AND public_model_id=?",
        )
        .bind(identity_id)
        .bind(model_id)
        .fetch_optional(self.pool())
        .await?;
        row.map(|row| {
            Ok(ImageBalanceRecord {
                identity_id: row.try_get("identity_id")?,
                public_model_id: row.try_get("public_model_id")?,
                available_images: row.try_get("available_images")?,
                reserved_images: row.try_get("reserved_images")?,
                consumed_images: row.try_get("consumed_images")?,
                revision: checked_i64(row.try_get::<u64, _>("revision")?)?,
                last_ledger_hmac: row.try_get("last_ledger_hmac")?,
                integrity_hmac: row.try_get("integrity_hmac")?,
            })
        })
        .transpose()
    }

    pub async fn image_reservation(
        &self,
        id: &str,
    ) -> Result<Option<ImageReservationRecord>, crate::StorageError> {
        let row = sqlx::query(
            "SELECT id,identity_id,api_key_id,public_model_id,request_id,reserved_images,
             confirmed_images,released_images,status,revision,integrity_hmac,created_at,expires_at,settled_at
             FROM image_quota_reservations WHERE id=?",
        ).bind(id).fetch_optional(self.pool()).await?;
        row.map(|row| {
            Ok(ImageReservationRecord {
                id: row.try_get("id")?,
                identity_id: row.try_get("identity_id")?,
                api_key_id: row.try_get("api_key_id")?,
                public_model_id: row.try_get("public_model_id")?,
                request_id: row.try_get("request_id")?,
                reserved_images: row.try_get("reserved_images")?,
                confirmed_images: row.try_get("confirmed_images")?,
                released_images: row.try_get("released_images")?,
                status: row.try_get("status")?,
                revision: checked_i64(row.try_get::<u64, _>("revision")?)?,
                integrity_hmac: row.try_get("integrity_hmac")?,
                created_at: row.try_get("created_at")?,
                expires_at: row.try_get("expires_at")?,
                settled_at: row.try_get("settled_at")?,
            })
        })
        .transpose()
    }

    pub async fn image_ledger(
        &self,
        id: &str,
    ) -> Result<Option<ImageLedgerRecord>, crate::StorageError> {
        let row = sqlx::query(
            "SELECT id,identity_id,public_model_id,reservation_id,child_id,kind,amount_images,
             produced_images,delivery_state,actor,reason,previous_hmac,integrity_hmac,created_at
             FROM image_quota_ledger WHERE id=?",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        row.map(|row| {
            Ok(ImageLedgerRecord {
                id: row.try_get("id")?,
                identity_id: row.try_get("identity_id")?,
                public_model_id: row.try_get("public_model_id")?,
                reservation_id: row.try_get("reservation_id")?,
                child_id: row.try_get("child_id")?,
                kind: row.try_get("kind")?,
                amount_images: row.try_get("amount_images")?,
                produced_images: row.try_get("produced_images")?,
                delivery_state: row.try_get("delivery_state")?,
                actor: row.try_get("actor")?,
                reason: row.try_get("reason")?,
                previous_hmac: row.try_get("previous_hmac")?,
                integrity_hmac: row.try_get("integrity_hmac")?,
                created_at: row.try_get("created_at")?,
            })
        })
        .transpose()
    }

    pub async fn image_ledger_for_member(
        &self,
        identity_id: &str,
    ) -> Result<Vec<ImageLedgerRecord>, crate::StorageError> {
        let rows = sqlx::query(
            "SELECT id,identity_id,public_model_id,reservation_id,child_id,kind,amount_images,
             produced_images,delivery_state,actor,reason,previous_hmac,integrity_hmac,created_at
             FROM image_quota_ledger WHERE identity_id=? ORDER BY created_at DESC,id DESC LIMIT 100",
        ).bind(identity_id).fetch_all(self.pool()).await?;
        rows.into_iter()
            .map(|row| {
                Ok(ImageLedgerRecord {
                    id: row.try_get("id")?,
                    identity_id: row.try_get("identity_id")?,
                    public_model_id: row.try_get("public_model_id")?,
                    reservation_id: row.try_get("reservation_id")?,
                    child_id: row.try_get("child_id")?,
                    kind: row.try_get("kind")?,
                    amount_images: row.try_get("amount_images")?,
                    produced_images: row.try_get("produced_images")?,
                    delivery_state: row.try_get("delivery_state")?,
                    actor: row.try_get("actor")?,
                    reason: row.try_get("reason")?,
                    previous_hmac: row.try_get("previous_hmac")?,
                    integrity_hmac: row.try_get("integrity_hmac")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn mutate_image_quota(
        &self,
        expected_balance: Option<&ImageBalanceRecord>,
        next_balance: &ImageBalanceRecord,
        expected_reservation: Option<&ImageReservationRecord>,
        next_reservation: Option<&ImageReservationRecord>,
        ledger: &ImageLedgerRecord,
        audit: Option<(u64, &str, &AuditEventRecord)>,
    ) -> Result<ImageQuotaMutationOutcome, crate::StorageError> {
        if !valid_image_balance_transition(expected_balance, next_balance, ledger)
            || !valid_image_reservation_transition(expected_reservation, next_reservation, ledger)
        {
            return Err(crate::StorageError::MigrationIntegrity);
        }
        let mut transaction = self.pool().begin().await?;
        let duplicate = sqlx::query(
            "SELECT id,identity_id,public_model_id,reservation_id,child_id,kind,amount_images,
             produced_images,delivery_state,actor,reason,previous_hmac,integrity_hmac,created_at
             FROM image_quota_ledger WHERE id=? FOR UPDATE",
        )
        .bind(&ledger.id)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(row) = duplicate {
            return Ok(ImageQuotaMutationOutcome::Duplicate(Box::new(
                ImageLedgerRecord {
                    id: row.try_get("id")?,
                    identity_id: row.try_get("identity_id")?,
                    public_model_id: row.try_get("public_model_id")?,
                    reservation_id: row.try_get("reservation_id")?,
                    child_id: row.try_get("child_id")?,
                    kind: row.try_get("kind")?,
                    amount_images: row.try_get("amount_images")?,
                    produced_images: row.try_get("produced_images")?,
                    delivery_state: row.try_get("delivery_state")?,
                    actor: row.try_get("actor")?,
                    reason: row.try_get("reason")?,
                    previous_hmac: row.try_get("previous_hmac")?,
                    integrity_hmac: row.try_get("integrity_hmac")?,
                    created_at: row.try_get("created_at")?,
                },
            )));
        }
        let unit: Option<String> = sqlx::query_scalar(
            "SELECT u.quota_unit FROM model_quota_units u JOIN models m ON m.id=u.model_id
             WHERE u.model_id=? AND m.enabled=1",
        )
        .bind(&next_balance.public_model_id)
        .fetch_optional(&mut *transaction)
        .await?;
        if unit.as_deref() != Some("image") {
            return Ok(ImageQuotaMutationOutcome::Conflict);
        }
        let current = sqlx::query(
            "SELECT revision,integrity_hmac FROM member_image_balances
             WHERE identity_id=? AND public_model_id=? FOR UPDATE",
        )
        .bind(&next_balance.identity_id)
        .bind(&next_balance.public_model_id)
        .fetch_optional(&mut *transaction)
        .await?;
        let current = current
            .map(|row| {
                Ok::<_, sqlx::Error>((
                    checked_i64(row.try_get::<u64, _>("revision")?)?,
                    row.try_get::<String, _>("integrity_hmac")?,
                ))
            })
            .transpose()?;
        if current != expected_balance.map(|value| (value.revision, value.integrity_hmac.clone())) {
            return Ok(ImageQuotaMutationOutcome::Conflict);
        }
        if let Some(expected) = expected_balance {
            let changed = sqlx::query(
                "UPDATE member_image_balances SET available_images=?,reserved_images=?,consumed_images=?,
                 revision=?,last_ledger_hmac=?,integrity_hmac=? WHERE identity_id=? AND public_model_id=?
                 AND revision=? AND integrity_hmac=?",
            ).bind(next_balance.available_images).bind(next_balance.reserved_images)
                .bind(next_balance.consumed_images).bind(next_balance.revision)
                .bind(&next_balance.last_ledger_hmac).bind(&next_balance.integrity_hmac)
                .bind(&next_balance.identity_id).bind(&next_balance.public_model_id)
                .bind(expected.revision).bind(&expected.integrity_hmac)
                .execute(&mut *transaction).await?.rows_affected();
            if changed != 1 {
                return Ok(ImageQuotaMutationOutcome::Conflict);
            }
        } else {
            sqlx::query(
                "INSERT INTO member_image_balances(identity_id,public_model_id,available_images,
                 reserved_images,consumed_images,revision,last_ledger_hmac,integrity_hmac)
                 VALUES(?,?,?,?,?,?,?,?)",
            )
            .bind(&next_balance.identity_id)
            .bind(&next_balance.public_model_id)
            .bind(next_balance.available_images)
            .bind(next_balance.reserved_images)
            .bind(next_balance.consumed_images)
            .bind(next_balance.revision)
            .bind(&next_balance.last_ledger_hmac)
            .bind(&next_balance.integrity_hmac)
            .execute(&mut *transaction)
            .await?;
        }
        if let Some(next) = next_reservation {
            if next.identity_id != next_balance.identity_id
                || next.public_model_id != next_balance.public_model_id
            {
                return Err(crate::StorageError::MigrationIntegrity);
            }
            if let Some(expected) = expected_reservation {
                let changed = sqlx::query(
                    "UPDATE image_quota_reservations SET confirmed_images=?,released_images=?,status=?,
                     revision=?,integrity_hmac=?,settled_at=? WHERE id=? AND revision=? AND integrity_hmac=?",
                ).bind(next.confirmed_images).bind(next.released_images).bind(&next.status)
                    .bind(next.revision).bind(&next.integrity_hmac).bind(&next.settled_at)
                    .bind(&next.id).bind(expected.revision).bind(&expected.integrity_hmac)
                    .execute(&mut *transaction).await?.rows_affected();
                if changed != 1 {
                    return Ok(ImageQuotaMutationOutcome::Conflict);
                }
            } else {
                sqlx::query(
                    "INSERT INTO image_quota_reservations(id,identity_id,api_key_id,public_model_id,
                     request_id,reserved_images,confirmed_images,released_images,status,revision,
                     integrity_hmac,created_at,expires_at,settled_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                ).bind(&next.id).bind(&next.identity_id).bind(&next.api_key_id)
                    .bind(&next.public_model_id).bind(&next.request_id).bind(next.reserved_images)
                    .bind(next.confirmed_images).bind(next.released_images).bind(&next.status)
                    .bind(next.revision).bind(&next.integrity_hmac).bind(&next.created_at)
                    .bind(&next.expires_at).bind(&next.settled_at).execute(&mut *transaction).await?;
            }
        } else if expected_reservation.is_some() {
            return Err(crate::StorageError::MigrationIntegrity);
        }
        sqlx::query(
            "INSERT INTO image_quota_ledger(id,identity_id,public_model_id,reservation_id,child_id,
             kind,amount_images,produced_images,delivery_state,actor,reason,previous_hmac,
             integrity_hmac,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&ledger.id)
        .bind(&ledger.identity_id)
        .bind(&ledger.public_model_id)
        .bind(&ledger.reservation_id)
        .bind(&ledger.child_id)
        .bind(&ledger.kind)
        .bind(ledger.amount_images)
        .bind(ledger.produced_images)
        .bind(&ledger.delivery_state)
        .bind(&ledger.actor)
        .bind(&ledger.reason)
        .bind(&ledger.previous_hmac)
        .bind(&ledger.integrity_hmac)
        .bind(&ledger.created_at)
        .execute(&mut *transaction)
        .await?;
        if let Some((sequence, hmac, event)) = audit
            && !append_audit_events_mariadb(
                &mut transaction,
                sequence,
                hmac,
                std::slice::from_ref(event),
            )
            .await?
        {
            return Ok(ImageQuotaMutationOutcome::Conflict);
        }
        transaction.commit().await?;
        Ok(ImageQuotaMutationOutcome::Applied)
    }
}
