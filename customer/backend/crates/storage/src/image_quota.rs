//! Independent, integer image balances. Mutations are one transaction across
//! balance, reservation, ledger and optional administrator audit records.
#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
use crate::StorageError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageBalanceRecord {
    pub identity_id: String,
    pub public_model_id: String,
    pub available_images: i64,
    pub reserved_images: i64,
    pub consumed_images: i64,
    pub revision: i64,
    pub last_ledger_hmac: String,
    pub integrity_hmac: String,
}

#[cfg(test)]
mod transition_tests {
    use super::*;

    #[test]
    fn delivery_event_preserves_charged_image_and_settled_reservation() {
        let before = ImageBalanceRecord {
            identity_id: "member".into(),
            public_model_id: "image_model".into(),
            available_images: 2,
            reserved_images: 0,
            consumed_images: 1,
            revision: 3,
            last_ledger_hmac: "prior".into(),
            integrity_hmac: String::new(),
        };
        let mut after = before.clone();
        after.revision += 1;
        after.last_ledger_hmac = "delivery_hmac".into();
        let previous = ImageReservationRecord {
            id: "reservation".into(),
            identity_id: "member".into(),
            api_key_id: "api_key".into(),
            public_model_id: "image_model".into(),
            request_id: "request".into(),
            reserved_images: 1,
            confirmed_images: 1,
            released_images: 0,
            status: "settled".into(),
            revision: 2,
            integrity_hmac: String::new(),
            created_at: "created".into(),
            expires_at: "expires".into(),
            settled_at: Some("settled".into()),
        };
        let mut next = previous.clone();
        next.revision += 1;
        let event = ImageLedgerRecord {
            id: "delivery".into(),
            identity_id: "member".into(),
            public_model_id: "image_model".into(),
            reservation_id: Some("reservation".into()),
            child_id: Some("child_0".into()),
            kind: "delivery".into(),
            amount_images: 0,
            produced_images: 0,
            delivery_state: "failed".into(),
            actor: "member".into(),
            reason: "delivery".into(),
            previous_hmac: "prior".into(),
            integrity_hmac: "delivery_hmac".into(),
            created_at: "created".into(),
        };
        assert!(valid_image_balance_transition(
            Some(&before),
            &after,
            &event
        ));
        assert!(valid_image_reservation_transition(
            Some(&previous),
            Some(&next),
            &event
        ));
        next.status = "reserved".into();
        assert!(!valid_image_reservation_transition(
            Some(&previous),
            Some(&next),
            &event
        ));
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageReservationRecord {
    pub id: String,
    pub identity_id: String,
    pub api_key_id: String,
    pub public_model_id: String,
    pub request_id: String,
    pub reserved_images: i64,
    pub confirmed_images: i64,
    pub released_images: i64,
    pub status: String,
    pub revision: i64,
    pub integrity_hmac: String,
    pub created_at: String,
    pub expires_at: String,
    pub settled_at: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageLedgerRecord {
    pub id: String,
    pub identity_id: String,
    pub public_model_id: String,
    pub reservation_id: Option<String>,
    pub child_id: Option<String>,
    pub kind: String,
    pub amount_images: i64,
    pub produced_images: i64,
    pub delivery_state: String,
    pub actor: String,
    pub reason: String,
    pub previous_hmac: String,
    pub integrity_hmac: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImageQuotaMutationOutcome {
    Applied,
    Conflict,
    Duplicate(Box<ImageLedgerRecord>),
}

#[cfg(feature = "mariadb")]
mod mariadb;

pub fn valid_image_balance_transition(
    expected: Option<&ImageBalanceRecord>,
    next: &ImageBalanceRecord,
    ledger: &ImageLedgerRecord,
) -> bool {
    if next.available_images < 0
        || next.reserved_images < 0
        || next.consumed_images < 0
        || ledger.identity_id != next.identity_id
        || ledger.public_model_id != next.public_model_id
        || ledger.previous_hmac != expected.map_or("", |value| &value.last_ledger_hmac)
        || next.last_ledger_hmac != ledger.integrity_hmac
        || !expected.map_or(next.revision == 0, |value| {
            next.revision.checked_sub(1) == Some(value.revision)
        })
    {
        return false;
    }
    let previous = expected.cloned().unwrap_or(ImageBalanceRecord {
        identity_id: next.identity_id.clone(),
        public_model_id: next.public_model_id.clone(),
        available_images: 0,
        reserved_images: 0,
        consumed_images: 0,
        revision: 0,
        last_ledger_hmac: String::new(),
        integrity_hmac: String::new(),
    });
    match ledger.kind.as_str() {
        "adjust" => {
            next.available_images.checked_sub(previous.available_images)
                == Some(ledger.amount_images)
                && next.reserved_images == previous.reserved_images
                && next.consumed_images == previous.consumed_images
                && ledger.reservation_id.is_none()
                && ledger.child_id.is_none()
        }
        "reserve" => {
            ledger.amount_images > 0
                && previous.available_images.checked_sub(next.available_images)
                    == Some(ledger.amount_images)
                && next.reserved_images.checked_sub(previous.reserved_images)
                    == Some(ledger.amount_images)
                && next.consumed_images == previous.consumed_images
        }
        "dispatch" | "delivery" => {
            ledger.amount_images == 0
                && next.available_images == previous.available_images
                && next.reserved_images == previous.reserved_images
                && next.consumed_images == previous.consumed_images
        }
        "confirm" => {
            ledger.amount_images > 0
                && previous.reserved_images.checked_sub(next.reserved_images)
                    == Some(ledger.amount_images)
                && next.consumed_images.checked_sub(previous.consumed_images)
                    == Some(ledger.amount_images)
                && next.available_images == previous.available_images
        }
        "release" => {
            ledger.amount_images > 0
                && previous.reserved_images.checked_sub(next.reserved_images)
                    == Some(ledger.amount_images)
                && next.available_images.checked_sub(previous.available_images)
                    == Some(ledger.amount_images)
                && next.consumed_images == previous.consumed_images
        }
        _ => false,
    }
}

pub fn valid_image_reservation_transition(
    expected: Option<&ImageReservationRecord>,
    next: Option<&ImageReservationRecord>,
    ledger: &ImageLedgerRecord,
) -> bool {
    let Some(next) = next else {
        return expected.is_none() && ledger.kind == "adjust";
    };
    if next.identity_id != ledger.identity_id
        || next.public_model_id != ledger.public_model_id
        || ledger.reservation_id.as_deref() != Some(next.id.as_str())
        || next.reserved_images <= 0
        || next.confirmed_images < 0
        || next.released_images < 0
        || next
            .confirmed_images
            .checked_add(next.released_images)
            .is_none_or(|value| value > next.reserved_images)
    {
        return false;
    }
    let Some(completed) = next.confirmed_images.checked_add(next.released_images) else {
        return false;
    };
    let expected_status = if completed < next.reserved_images {
        "reserved"
    } else {
        "settled"
    };
    if next.status != expected_status
        || next.settled_at.is_some() != (completed == next.reserved_images)
    {
        return false;
    }
    let Some(expected) = expected else {
        return ledger.kind == "reserve"
            && ledger.child_id.is_none()
            && next.status == "reserved"
            && next.revision == 0
            && next.confirmed_images == 0
            && next.released_images == 0
            && next.reserved_images == ledger.amount_images;
    };
    if expected.id != next.id
        || expected.identity_id != next.identity_id
        || expected.api_key_id != next.api_key_id
        || expected.public_model_id != next.public_model_id
        || expected.request_id != next.request_id
        || expected.reserved_images != next.reserved_images
        || expected.created_at != next.created_at
        || expected.expires_at != next.expires_at
        || next.revision.checked_sub(1) != Some(expected.revision)
    {
        return false;
    }
    let Some(delta_confirmed) = next.confirmed_images.checked_sub(expected.confirmed_images) else {
        return false;
    };
    let Some(delta_released) = next.released_images.checked_sub(expected.released_images) else {
        return false;
    };
    match ledger.kind.as_str() {
        "dispatch" | "delivery" => {
            delta_confirmed == 0 && delta_released == 0 && next.status == expected.status
        }
        "confirm" => delta_confirmed == 1 && delta_released == 0 && ledger.produced_images == 1,
        "release" => delta_confirmed == 0 && delta_released == 1 && ledger.produced_images == 0,
        _ => false,
    }
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
mod sqlite {
    use super::*;
    use crate::{AuditEventRecord, SqlCipherStore, append_audit_events_sqlite};
    use rusqlite::{OptionalExtension, TransactionBehavior, params};

    impl SqlCipherStore {
        pub fn expired_image_reservations(
            &self,
            expired_before: &str,
        ) -> Result<Vec<ImageReservationRecord>, StorageError> {
            let mut statement = self.connection.prepare(
                "SELECT id,identity_id,api_key_id,public_model_id,request_id,reserved_images,
                 confirmed_images,released_images,status,revision,integrity_hmac,
                 created_at,expires_at,settled_at FROM image_quota_reservations
                 WHERE status='reserved' AND expires_at<=?
                 ORDER BY created_at LIMIT 200",
            )?;
            statement
                .query_map([expired_before], |row| {
                    Ok(ImageReservationRecord {
                        id: row.get(0)?,
                        identity_id: row.get(1)?,
                        api_key_id: row.get(2)?,
                        public_model_id: row.get(3)?,
                        request_id: row.get(4)?,
                        reserved_images: row.get(5)?,
                        confirmed_images: row.get(6)?,
                        released_images: row.get(7)?,
                        status: row.get(8)?,
                        revision: row.get(9)?,
                        integrity_hmac: row.get(10)?,
                        created_at: row.get(11)?,
                        expires_at: row.get(12)?,
                        settled_at: row.get(13)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::from)
        }

        pub fn image_quota_unit(&self, model_id: &str) -> Result<Option<String>, StorageError> {
            self.connection
                .query_row(
                    "SELECT CASE WHEN u.quota_unit='image' THEN 'image' ELSE 'token' END
                 FROM models m LEFT JOIN model_quota_units u ON u.model_id=m.id WHERE m.id=?",
                    [model_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(StorageError::from)
        }

        pub fn image_balance(
            &self,
            identity_id: &str,
            model_id: &str,
        ) -> Result<Option<ImageBalanceRecord>, StorageError> {
            self.connection.query_row(
                "SELECT identity_id,public_model_id,available_images,reserved_images,consumed_images,
                 revision,last_ledger_hmac,integrity_hmac FROM member_image_balances
                 WHERE identity_id=? AND public_model_id=?",
                params![identity_id,model_id], |row| Ok(ImageBalanceRecord {
                    identity_id:row.get(0)?, public_model_id:row.get(1)?, available_images:row.get(2)?,
                    reserved_images:row.get(3)?, consumed_images:row.get(4)?, revision:row.get(5)?,
                    last_ledger_hmac:row.get(6)?, integrity_hmac:row.get(7)?,
                }),
            ).optional().map_err(StorageError::from)
        }

        pub fn image_reservation(
            &self,
            id: &str,
        ) -> Result<Option<ImageReservationRecord>, StorageError> {
            self.connection.query_row(
                "SELECT id,identity_id,api_key_id,public_model_id,request_id,reserved_images,
                 confirmed_images,released_images,status,revision,integrity_hmac,created_at,expires_at,settled_at
                 FROM image_quota_reservations WHERE id=?",
                [id], |row| Ok(ImageReservationRecord {
                    id:row.get(0)?, identity_id:row.get(1)?, api_key_id:row.get(2)?,
                    public_model_id:row.get(3)?, request_id:row.get(4)?, reserved_images:row.get(5)?,
                    confirmed_images:row.get(6)?, released_images:row.get(7)?, status:row.get(8)?,
                    revision:row.get(9)?, integrity_hmac:row.get(10)?, created_at:row.get(11)?,
                    expires_at:row.get(12)?, settled_at:row.get(13)?,
                }),
            ).optional().map_err(StorageError::from)
        }

        pub fn image_ledger(&self, id: &str) -> Result<Option<ImageLedgerRecord>, StorageError> {
            self.connection.query_row(
                "SELECT id,identity_id,public_model_id,reservation_id,child_id,kind,amount_images,
                 produced_images,delivery_state,actor,reason,previous_hmac,integrity_hmac,created_at
                 FROM image_quota_ledger WHERE id=?",
                [id], |row| Ok(ImageLedgerRecord {
                    id:row.get(0)?, identity_id:row.get(1)?, public_model_id:row.get(2)?,
                    reservation_id:row.get(3)?, child_id:row.get(4)?, kind:row.get(5)?,
                    amount_images:row.get(6)?, produced_images:row.get(7)?,
                    delivery_state:row.get(8)?, actor:row.get(9)?, reason:row.get(10)?,
                    previous_hmac:row.get(11)?, integrity_hmac:row.get(12)?, created_at:row.get(13)?,
                }),
            ).optional().map_err(StorageError::from)
        }

        pub fn image_ledger_for_member(
            &self,
            identity_id: &str,
        ) -> Result<Vec<ImageLedgerRecord>, StorageError> {
            let mut statement = self.connection.prepare(
                "SELECT id,identity_id,public_model_id,reservation_id,child_id,kind,amount_images,
                 produced_images,delivery_state,actor,reason,previous_hmac,integrity_hmac,created_at
                 FROM image_quota_ledger WHERE identity_id=? ORDER BY created_at DESC,id DESC LIMIT 100",
            )?;
            statement
                .query_map([identity_id], |row| {
                    Ok(ImageLedgerRecord {
                        id: row.get(0)?,
                        identity_id: row.get(1)?,
                        public_model_id: row.get(2)?,
                        reservation_id: row.get(3)?,
                        child_id: row.get(4)?,
                        kind: row.get(5)?,
                        amount_images: row.get(6)?,
                        produced_images: row.get(7)?,
                        delivery_state: row.get(8)?,
                        actor: row.get(9)?,
                        reason: row.get(10)?,
                        previous_hmac: row.get(11)?,
                        integrity_hmac: row.get(12)?,
                        created_at: row.get(13)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::from)
        }

        pub fn mutate_image_quota(
            &mut self,
            expected_balance: Option<&ImageBalanceRecord>,
            next_balance: &ImageBalanceRecord,
            expected_reservation: Option<&ImageReservationRecord>,
            next_reservation: Option<&ImageReservationRecord>,
            ledger: &ImageLedgerRecord,
            audit: Option<(u64, &str, &AuditEventRecord)>,
        ) -> Result<ImageQuotaMutationOutcome, StorageError> {
            if !valid_image_balance_transition(expected_balance, next_balance, ledger)
                || !valid_image_reservation_transition(
                    expected_reservation,
                    next_reservation,
                    ledger,
                )
            {
                return Err(StorageError::MigrationIntegrity);
            }
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let duplicate = transaction.query_row(
                "SELECT id,identity_id,public_model_id,reservation_id,child_id,kind,amount_images,
                 produced_images,delivery_state,actor,reason,previous_hmac,integrity_hmac,created_at
                 FROM image_quota_ledger WHERE id=?",
                [&ledger.id], |row| Ok(ImageLedgerRecord {
                    id:row.get(0)?, identity_id:row.get(1)?, public_model_id:row.get(2)?,
                    reservation_id:row.get(3)?, child_id:row.get(4)?, kind:row.get(5)?,
                    amount_images:row.get(6)?, produced_images:row.get(7)?,
                    delivery_state:row.get(8)?, actor:row.get(9)?, reason:row.get(10)?,
                    previous_hmac:row.get(11)?, integrity_hmac:row.get(12)?, created_at:row.get(13)?,
                }),
            ).optional()?;
            if let Some(existing) = duplicate {
                return Ok(ImageQuotaMutationOutcome::Duplicate(Box::new(existing)));
            }
            let unit: Option<String> = transaction
                .query_row(
                    "SELECT u.quota_unit FROM model_quota_units u JOIN models m ON m.id=u.model_id
                 WHERE u.model_id=? AND m.enabled=1",
                    [&next_balance.public_model_id],
                    |row| row.get(0),
                )
                .optional()?;
            if unit.as_deref() != Some("image") {
                return Ok(ImageQuotaMutationOutcome::Conflict);
            }
            let current: Option<(i64, String)> = transaction
                .query_row(
                    "SELECT revision,integrity_hmac FROM member_image_balances
                 WHERE identity_id=? AND public_model_id=?",
                    params![next_balance.identity_id, next_balance.public_model_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if current
                != expected_balance.map(|value| (value.revision, value.integrity_hmac.clone()))
            {
                return Ok(ImageQuotaMutationOutcome::Conflict);
            }
            if let Some(expected) = expected_balance {
                transaction.execute(
                    "UPDATE member_image_balances SET available_images=?,reserved_images=?,consumed_images=?,
                     revision=?,last_ledger_hmac=?,integrity_hmac=? WHERE identity_id=? AND public_model_id=?
                     AND revision=? AND integrity_hmac=?",
                    params![next_balance.available_images,next_balance.reserved_images,next_balance.consumed_images,
                        next_balance.revision,next_balance.last_ledger_hmac,next_balance.integrity_hmac,
                        next_balance.identity_id,next_balance.public_model_id,expected.revision,expected.integrity_hmac],
                )?;
            } else {
                transaction.execute(
                    "INSERT INTO member_image_balances(identity_id,public_model_id,available_images,
                     reserved_images,consumed_images,revision,last_ledger_hmac,integrity_hmac)
                     VALUES(?,?,?,?,?,?,?,?)",
                    params![next_balance.identity_id,next_balance.public_model_id,next_balance.available_images,
                        next_balance.reserved_images,next_balance.consumed_images,next_balance.revision,
                        next_balance.last_ledger_hmac,next_balance.integrity_hmac],
                )?;
            }
            if let Some(next) = next_reservation {
                if next.identity_id != next_balance.identity_id
                    || next.public_model_id != next_balance.public_model_id
                {
                    return Err(StorageError::MigrationIntegrity);
                }
                if let Some(expected) = expected_reservation {
                    let changed = transaction.execute(
                        "UPDATE image_quota_reservations SET confirmed_images=?,released_images=?,status=?,
                         revision=?,integrity_hmac=?,settled_at=? WHERE id=? AND revision=? AND integrity_hmac=?",
                        params![next.confirmed_images,next.released_images,next.status,next.revision,
                            next.integrity_hmac,next.settled_at,next.id,expected.revision,expected.integrity_hmac],
                    )?;
                    if changed != 1 {
                        return Ok(ImageQuotaMutationOutcome::Conflict);
                    }
                } else {
                    transaction.execute(
                        "INSERT INTO image_quota_reservations(id,identity_id,api_key_id,public_model_id,
                         request_id,reserved_images,confirmed_images,released_images,status,revision,
                         integrity_hmac,created_at,expires_at,settled_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                        params![next.id,next.identity_id,next.api_key_id,next.public_model_id,next.request_id,
                            next.reserved_images,next.confirmed_images,next.released_images,next.status,
                            next.revision,next.integrity_hmac,next.created_at,next.expires_at,next.settled_at],
                    )?;
                }
            } else if expected_reservation.is_some() {
                return Err(StorageError::MigrationIntegrity);
            }
            transaction.execute(
                "INSERT INTO image_quota_ledger(id,identity_id,public_model_id,reservation_id,child_id,
                 kind,amount_images,produced_images,delivery_state,actor,reason,previous_hmac,
                 integrity_hmac,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                params![ledger.id,ledger.identity_id,ledger.public_model_id,ledger.reservation_id,
                    ledger.child_id,ledger.kind,ledger.amount_images,ledger.produced_images,
                    ledger.delivery_state,ledger.actor,ledger.reason,ledger.previous_hmac,
                    ledger.integrity_hmac,ledger.created_at],
            )?;
            if let Some((sequence, hmac, event)) = audit
                && !append_audit_events_sqlite(
                    &transaction,
                    sequence,
                    hmac,
                    std::slice::from_ref(event),
                )?
            {
                return Ok(ImageQuotaMutationOutcome::Conflict);
            }
            transaction.commit()?;
            Ok(ImageQuotaMutationOutcome::Applied)
        }
    }
}
