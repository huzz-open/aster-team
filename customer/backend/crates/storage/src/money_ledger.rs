//! Customer-local monetary balance and immutable ledger. Monetary mutations
//! never use the legacy Token balance or quota reservation tables.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoneyBalanceRecord {
    pub identity_id: String,
    pub currency: String,
    pub balance_nanos: i64,
    pub credited_nanos: i64,
    pub debited_nanos: i64,
    pub revision: i64,
    pub last_ledger_hmac: String,
    pub integrity_hmac: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoneyLedgerEntry {
    pub id: String,
    pub identity_id: String,
    pub currency: String,
    pub kind: String,
    /// Signed delta to the member balance. A confirmed charge is negative.
    pub amount_nanos: i64,
    pub balance_revision: i64,
    /// Unique per member across grants, charges, anomalies, and adjustments.
    pub reference_id: String,
    pub billing_status: String,
    /// Signed by integrity_hmac; charge entries include the frozen price,
    /// exchange rate, public model, actual model, and verified usage here.
    pub details_json: String,
    pub previous_entry_hmac: String,
    pub integrity_hmac: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoneyStateSnapshot {
    pub balance: MoneyBalanceRecord,
    pub ledger_entries: Vec<MoneyLedgerEntry>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MoneyMutationOutcome {
    Applied,
    Conflict,
    DuplicateReference,
}

fn valid_currency(currency: &str) -> bool {
    matches!(currency, "CNY" | "USD")
}

pub fn valid_initial_money_balance(balance: &MoneyBalanceRecord) -> bool {
    !balance.identity_id.is_empty()
        && valid_currency(&balance.currency)
        && balance.balance_nanos == 0
        && balance.credited_nanos == 0
        && balance.debited_nanos == 0
        && balance.revision == 0
        && balance.last_ledger_hmac.is_empty()
        && !balance.integrity_hmac.is_empty()
        && !balance.updated_at.is_empty()
}

pub fn valid_money_transition(
    expected: &MoneyBalanceRecord,
    next: &MoneyBalanceRecord,
    entry: &MoneyLedgerEntry,
) -> bool {
    if !valid_currency(&expected.currency)
        || expected.identity_id.is_empty()
        || expected.currency != next.currency
        || expected.currency != entry.currency
        || expected.identity_id != next.identity_id
        || expected.identity_id != entry.identity_id
        || expected.revision < 0
        || expected.credited_nanos < 0
        || expected.debited_nanos < 0
        || expected.credited_nanos.checked_sub(expected.debited_nanos)
            != Some(expected.balance_nanos)
        || expected.integrity_hmac.is_empty()
        || next.integrity_hmac.is_empty()
        || entry.integrity_hmac.is_empty()
        || entry.id.is_empty()
        || entry.reference_id.is_empty()
        || entry.created_at.is_empty()
        || entry.previous_entry_hmac != expected.last_ledger_hmac
        || next.last_ledger_hmac != entry.integrity_hmac
        || next.updated_at.is_empty()
    {
        return false;
    }
    let kind_valid = match entry.kind.as_str() {
        "grant" | "refund" => entry.amount_nanos > 0 && entry.billing_status == "not_applicable",
        "charge" => entry.amount_nanos <= 0 && entry.billing_status == "charged",
        "anomaly" => entry.amount_nanos == 0 && entry.billing_status == "billing_error",
        "request_failed" => entry.amount_nanos == 0 && entry.billing_status == "not_applicable",
        "adjustment" => entry.amount_nanos != 0 && entry.billing_status == "not_applicable",
        _ => false,
    };
    if !kind_valid
        || entry.details_json.len() > 32_768
        || serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&entry.details_json)
            .is_err()
    {
        return false;
    }
    let credit = entry.amount_nanos.max(0);
    let Some(debit) = entry.amount_nanos.checked_neg().map(|value| value.max(0)) else {
        return false;
    };
    expected.revision.checked_add(1) == Some(next.revision)
        && entry.balance_revision == next.revision
        && expected.balance_nanos.checked_add(entry.amount_nanos) == Some(next.balance_nanos)
        && expected.credited_nanos.checked_add(credit) == Some(next.credited_nanos)
        && expected.debited_nanos.checked_add(debit) == Some(next.debited_nanos)
}

#[cfg(any(feature = "sqlite-dev", feature = "sqlcipher"))]
mod sqlite {
    use super::*;
    use crate::{
        AuditEventRecord, MutationWithAuditOutcome, SqlCipherStore, StorageError,
        append_audit_events_sqlite, is_sqlite_unique_violation,
    };
    use rusqlite::{OptionalExtension, TransactionBehavior, params};

    fn balance_from_row(row: &rusqlite::Row<'_>) -> Result<MoneyBalanceRecord, rusqlite::Error> {
        Ok(MoneyBalanceRecord {
            identity_id: row.get(0)?,
            currency: row.get(1)?,
            balance_nanos: row.get(2)?,
            credited_nanos: row.get(3)?,
            debited_nanos: row.get(4)?,
            revision: row.get(5)?,
            last_ledger_hmac: row.get(6)?,
            integrity_hmac: row.get(7)?,
            updated_at: row.get(8)?,
        })
    }

    fn ledger_from_row(row: &rusqlite::Row<'_>) -> Result<MoneyLedgerEntry, rusqlite::Error> {
        Ok(MoneyLedgerEntry {
            id: row.get(0)?,
            identity_id: row.get(1)?,
            currency: row.get(2)?,
            kind: row.get(3)?,
            amount_nanos: row.get(4)?,
            balance_revision: row.get(5)?,
            reference_id: row.get(6)?,
            billing_status: row.get(7)?,
            details_json: row.get(8)?,
            previous_entry_hmac: row.get(9)?,
            integrity_hmac: row.get(10)?,
            created_at: row.get(11)?,
        })
    }

    impl SqlCipherStore {
        pub fn initialize_money_balance(
            &self,
            balance: &MoneyBalanceRecord,
        ) -> Result<bool, StorageError> {
            if !valid_initial_money_balance(balance) {
                return Ok(false);
            }
            match self.connection.execute(
                "INSERT INTO money_balances(
                   identity_id,currency,balance_nanos,credited_nanos,debited_nanos,
                   revision,last_ledger_hmac,integrity_hmac,updated_at
                 ) VALUES(?,?,?,?,?,?,?,?,?)",
                params![
                    balance.identity_id,
                    balance.currency,
                    balance.balance_nanos,
                    balance.credited_nanos,
                    balance.debited_nanos,
                    balance.revision,
                    balance.last_ledger_hmac,
                    balance.integrity_hmac,
                    balance.updated_at,
                ],
            ) {
                Ok(1) => Ok(true),
                Ok(_) => Ok(false),
                Err(error) if is_sqlite_unique_violation(&error) => Ok(false),
                Err(error) => Err(StorageError::Database(error)),
            }
        }

        pub fn money_state_snapshot(
            &self,
            identity_id: &str,
        ) -> Result<Option<MoneyStateSnapshot>, StorageError> {
            let transaction = self.connection.unchecked_transaction()?;
            let balance = transaction
                .query_row(
                    "SELECT identity_id,currency,balance_nanos,credited_nanos,debited_nanos,
                            revision,last_ledger_hmac,integrity_hmac,updated_at
                     FROM money_balances WHERE identity_id=?",
                    [identity_id],
                    balance_from_row,
                )
                .optional()?;
            let Some(balance) = balance else {
                return Ok(None);
            };
            let mut statement = transaction.prepare(
                "SELECT id,identity_id,currency,kind,amount_nanos,balance_revision,reference_id,billing_status,
                        details_json,previous_entry_hmac,integrity_hmac,created_at
                 FROM money_ledger_entries WHERE identity_id=? ORDER BY balance_revision ASC",
            )?;
            let ledger_entries = statement
                .query_map([identity_id], ledger_from_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Some(MoneyStateSnapshot {
                balance,
                ledger_entries,
            }))
        }

        pub fn write_money_entry_with_audit(
            &mut self,
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
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let current = transaction
                .query_row(
                    "SELECT identity_id,currency,balance_nanos,credited_nanos,debited_nanos,
                            revision,last_ledger_hmac,integrity_hmac,updated_at
                     FROM money_balances WHERE identity_id=?",
                    [&expected.identity_id],
                    balance_from_row,
                )
                .optional()?;
            if current.as_ref() != Some(expected) {
                transaction.rollback()?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    MoneyMutationOutcome::Conflict,
                ));
            }
            let inserted = transaction.execute(
                "INSERT INTO money_ledger_entries(
                   id,identity_id,currency,kind,amount_nanos,balance_revision,reference_id,billing_status,
                   details_json,previous_entry_hmac,integrity_hmac,created_at
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    entry.id,
                    entry.identity_id,
                    entry.currency,
                    entry.kind,
                    entry.amount_nanos,
                    entry.balance_revision,
                    entry.reference_id,
                    entry.billing_status,
                    entry.details_json,
                    entry.previous_entry_hmac,
                    entry.integrity_hmac,
                    entry.created_at,
                ],
            );
            if let Err(error) = inserted {
                if is_sqlite_unique_violation(&error) {
                    transaction.rollback()?;
                    return Ok(MutationWithAuditOutcome::Mutation(
                        MoneyMutationOutcome::DuplicateReference,
                    ));
                }
                return Err(StorageError::Database(error));
            }
            let updated = transaction.execute(
                "UPDATE money_balances SET balance_nanos=?,credited_nanos=?,debited_nanos=?,
                   revision=?,last_ledger_hmac=?,integrity_hmac=?,updated_at=?
                 WHERE identity_id=? AND revision=? AND integrity_hmac=?",
                params![
                    next.balance_nanos,
                    next.credited_nanos,
                    next.debited_nanos,
                    next.revision,
                    next.last_ledger_hmac,
                    next.integrity_hmac,
                    next.updated_at,
                    expected.identity_id,
                    expected.revision,
                    expected.integrity_hmac,
                ],
            )?;
            if updated != 1 {
                transaction.rollback()?;
                return Ok(MutationWithAuditOutcome::Mutation(
                    MoneyMutationOutcome::Conflict,
                ));
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
                MoneyMutationOutcome::Applied,
            ))
        }
    }
}

#[cfg(all(test, any(feature = "sqlite-dev", feature = "sqlcipher")))]
mod tests {
    use super::*;
    use crate::{AuditEventRecord, MutationWithAuditOutcome, SqlCipherStore};
    use rusqlite::params;
    use tempfile::tempdir;

    fn audit(sequence: u64, previous_hmac: &str) -> AuditEventRecord {
        AuditEventRecord {
            id: format!("money-audit-{sequence}"),
            sequence,
            actor_identity_id: None,
            actor_role: "system".to_owned(),
            action: "money.ledger.write".to_owned(),
            target_type: "money_balance".to_owned(),
            target_id: Some("member".to_owned()),
            outcome: "succeeded".to_owned(),
            previous_event_hmac: previous_hmac.to_owned(),
            integrity_hmac: char::from(b'A' + u8::try_from(sequence).unwrap())
                .to_string()
                .repeat(43),
            created_at: "2026-09-23T00:00:00.000Z".to_owned(),
        }
    }

    fn entry(
        revision: i64,
        kind: &str,
        amount_nanos: i64,
        reference_id: &str,
        previous_hmac: &str,
    ) -> MoneyLedgerEntry {
        MoneyLedgerEntry {
            id: format!("money-ledger-{revision}"),
            identity_id: "member".to_owned(),
            currency: "USD".to_owned(),
            kind: kind.to_owned(),
            amount_nanos,
            balance_revision: revision,
            reference_id: reference_id.to_owned(),
            billing_status: if kind == "charge" {
                "charged"
            } else if kind == "anomaly" {
                "billing_error"
            } else {
                "not_applicable"
            }
            .to_owned(),
            details_json: "{}".to_owned(),
            previous_entry_hmac: previous_hmac.to_owned(),
            integrity_hmac: format!("money-ledger-hmac-{revision}"),
            created_at: "2026-09-23T00:00:00.000Z".to_owned(),
        }
    }

    fn advance(balance: &MoneyBalanceRecord, entry: &MoneyLedgerEntry) -> MoneyBalanceRecord {
        MoneyBalanceRecord {
            identity_id: balance.identity_id.clone(),
            currency: balance.currency.clone(),
            balance_nanos: balance.balance_nanos + entry.amount_nanos,
            credited_nanos: balance.credited_nanos + entry.amount_nanos.max(0),
            debited_nanos: balance.debited_nanos + (-entry.amount_nanos).max(0),
            revision: balance.revision + 1,
            last_ledger_hmac: entry.integrity_hmac.clone(),
            integrity_hmac: format!("money-balance-hmac-{}", balance.revision + 1),
            updated_at: entry.created_at.clone(),
        }
    }

    #[test]
    fn money_ledger_is_atomic_idempotent_and_allows_confirmed_overdraft() {
        let directory = tempdir().unwrap();
        let mut store =
            SqlCipherStore::initialize(&directory.path().join("money.db"), &[91_u8; 32]).unwrap();
        store.connection.execute(
            "INSERT INTO identities(id,email,display_name,password_hash,role,status,
              can_consume_model,password_change_required,revision,integrity_hmac,created_at,updated_at)
             VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
            params!["member", "member@example.com", "Member", "hash", "member", "active",
                    1, 0, 0, "identity-hmac", "2026-09-23T00:00:00.000Z", "2026-09-23T00:00:00.000Z"],
        ).unwrap();
        let initial = MoneyBalanceRecord {
            identity_id: "member".to_owned(),
            currency: "USD".to_owned(),
            balance_nanos: 0,
            credited_nanos: 0,
            debited_nanos: 0,
            revision: 0,
            last_ledger_hmac: String::new(),
            integrity_hmac: "money-balance-hmac-0".to_owned(),
            updated_at: "2026-09-23T00:00:00.000Z".to_owned(),
        };
        assert!(store.initialize_money_balance(&initial).unwrap());
        assert!(!store.initialize_money_balance(&initial).unwrap());

        let grant = entry(1, "grant", 2_000, "grant:1", "");
        let granted = advance(&initial, &grant);
        let first_audit = audit(1, "");
        assert_eq!(
            store
                .write_money_entry_with_audit(&initial, &granted, &grant, 0, "", &first_audit)
                .unwrap(),
            MutationWithAuditOutcome::Mutation(MoneyMutationOutcome::Applied)
        );

        let charge = entry(2, "charge", -3_000, "request:1", &grant.integrity_hmac);
        let overdrawn = advance(&granted, &charge);
        let second_audit = audit(2, &first_audit.integrity_hmac);
        assert_eq!(
            store
                .write_money_entry_with_audit(
                    &granted,
                    &overdrawn,
                    &charge,
                    1,
                    &first_audit.integrity_hmac,
                    &second_audit,
                )
                .unwrap(),
            MutationWithAuditOutcome::Mutation(MoneyMutationOutcome::Applied)
        );
        assert_eq!(overdrawn.balance_nanos, -1_000);
        assert_eq!(overdrawn.debited_nanos, 3_000);

        let duplicate = entry(3, "anomaly", 0, "request:1", &charge.integrity_hmac);
        let duplicate_next = advance(&overdrawn, &duplicate);
        let third_audit = audit(3, &second_audit.integrity_hmac);
        assert_eq!(
            store
                .write_money_entry_with_audit(
                    &overdrawn,
                    &duplicate_next,
                    &duplicate,
                    2,
                    &second_audit.integrity_hmac,
                    &third_audit,
                )
                .unwrap(),
            MutationWithAuditOutcome::Mutation(MoneyMutationOutcome::DuplicateReference)
        );
        let snapshot = store.money_state_snapshot("member").unwrap().unwrap();
        assert_eq!(snapshot.balance, overdrawn);
        assert_eq!(snapshot.ledger_entries, vec![grant, charge.clone()]);
        assert_eq!(
            store
                .connection
                .query_row("SELECT count(*) FROM audit_events", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );

        let anomaly = entry(3, "anomaly", 0, "request:2", &charge.integrity_hmac);
        let unchanged_amount = advance(&overdrawn, &anomaly);
        assert_eq!(unchanged_amount.balance_nanos, overdrawn.balance_nanos);
        assert_eq!(
            store
                .write_money_entry_with_audit(
                    &overdrawn,
                    &unchanged_amount,
                    &anomaly,
                    2,
                    &second_audit.integrity_hmac,
                    &third_audit,
                )
                .unwrap(),
            MutationWithAuditOutcome::Mutation(MoneyMutationOutcome::Applied)
        );
        let snapshot = store.money_state_snapshot("member").unwrap().unwrap();
        assert_eq!(snapshot.balance.balance_nanos, -1_000);
        assert_eq!(snapshot.ledger_entries[2].billing_status, "billing_error");
    }

    #[test]
    fn money_transition_rejects_wrong_currency_and_untrusted_usage() {
        let initial = MoneyBalanceRecord {
            identity_id: "member".to_owned(),
            currency: "USD".to_owned(),
            balance_nanos: 0,
            credited_nanos: 0,
            debited_nanos: 0,
            revision: 0,
            last_ledger_hmac: String::new(),
            integrity_hmac: "balance-hmac".to_owned(),
            updated_at: "now".to_owned(),
        };
        let mut charge = entry(1, "charge", -1, "request:1", "");
        let next = advance(&initial, &charge);
        assert!(valid_money_transition(&initial, &next, &charge));
        charge.currency = "CNY".to_owned();
        assert!(!valid_money_transition(&initial, &next, &charge));
        charge.currency = "USD".to_owned();
        charge.details_json = "not json".to_owned();
        assert!(!valid_money_transition(&initial, &next, &charge));
    }
}
