#[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
use std::sync::Arc;

#[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
use crate::lifecycle;
use crate::{
    ControlError, ControlState, ControlStorage, GrantMemberQuotaRequest, QUOTA_MUTATION_RETRIES,
    authorize_non_consuming_feature, checked_add_nonnegative, format_database_time,
    random_identifier, require_admin_ready, require_member_ready, valid_client_request_id,
    valid_resource_identifier, validate_admin_actor, validate_quota_grant_target,
    verify_identity_integrity,
};
use aster_auth_core::SecurityStateIntegrityInput;
use aster_storage::{
    AuditEventRecord, ImageBalanceRecord, ImageLedgerRecord, ImageQuotaMutationOutcome,
    ImageReservationRecord, MutationWithAuditOutcome, QuotaBatchImageWrite, QuotaBatchTokenWrite,
    QuotaLedgerEntry, QuotaMutationOutcome,
};
use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::HeaderMap,
};
use serde::Deserialize;
use serde_json::{Value, json};

const IMAGE_BALANCE_KEY: &str = "image-quota-balance-v1";
const IMAGE_RESERVATION_KEY: &str = "image-quota-reservation-v1";
const IMAGE_LEDGER_KEY: &str = "image-quota-ledger-v1";

#[derive(Clone)]
struct ImageQuotaWrite {
    expected_balance: Option<ImageBalanceRecord>,
    next_balance: ImageBalanceRecord,
    expected_reservation: Option<ImageReservationRecord>,
    next_reservation: Option<ImageReservationRecord>,
    ledger: ImageLedgerRecord,
    audit: Option<(u64, String, AuditEventRecord)>,
}

impl ControlStorage {
    async fn expired_image_reservations(
        &self,
        expired_before: &str,
    ) -> Result<Vec<ImageReservationRecord>, ControlError> {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(store) => store
                .expired_image_reservations(expired_before)
                .await
                .map_err(ControlError::storage),
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            Self::SqlCipher(store) => {
                let store = Arc::clone(store);
                let expired_before = expired_before.to_owned();
                lifecycle::spawn_blocking(move || {
                    store
                        .lock()
                        .map_err(|_| ControlError::DataIntegrityInvalid)?
                        .expired_image_reservations(&expired_before)
                        .map_err(ControlError::storage)
                })
                .await
                .map_err(ControlError::storage)?
            }
            #[cfg(test)]
            Self::Fixed(_) | Self::Test { .. } => Ok(Vec::new()),
        }
    }

    pub(crate) async fn image_quota_unit(
        &self,
        model_id: &str,
    ) -> Result<Option<String>, ControlError> {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(store) => store
                .image_quota_unit(model_id)
                .await
                .map_err(ControlError::storage),
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            Self::SqlCipher(store) => {
                let store = Arc::clone(store);
                let model_id = model_id.to_owned();
                lifecycle::spawn_blocking(move || {
                    store
                        .lock()
                        .map_err(|_| ControlError::DataIntegrityInvalid)?
                        .image_quota_unit(&model_id)
                        .map_err(ControlError::storage)
                })
                .await
                .map_err(ControlError::storage)?
            }
            #[cfg(test)]
            Self::Fixed(_) | Self::Test { .. } => Ok(None),
        }
    }

    async fn image_balance(
        &self,
        identity_id: &str,
        model_id: &str,
    ) -> Result<Option<ImageBalanceRecord>, ControlError> {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(store) => store
                .image_balance(identity_id, model_id)
                .await
                .map_err(ControlError::storage),
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            Self::SqlCipher(store) => {
                let store = Arc::clone(store);
                let identity_id = identity_id.to_owned();
                let model_id = model_id.to_owned();
                lifecycle::spawn_blocking(move || {
                    store
                        .lock()
                        .map_err(|_| ControlError::DataIntegrityInvalid)?
                        .image_balance(&identity_id, &model_id)
                        .map_err(ControlError::storage)
                })
                .await
                .map_err(ControlError::storage)?
            }
            #[cfg(test)]
            Self::Fixed(_) | Self::Test { .. } => Ok(None),
        }
    }

    async fn image_reservation(
        &self,
        id: &str,
    ) -> Result<Option<ImageReservationRecord>, ControlError> {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(store) => store
                .image_reservation(id)
                .await
                .map_err(ControlError::storage),
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            Self::SqlCipher(store) => {
                let store = Arc::clone(store);
                let id = id.to_owned();
                lifecycle::spawn_blocking(move || {
                    store
                        .lock()
                        .map_err(|_| ControlError::DataIntegrityInvalid)?
                        .image_reservation(&id)
                        .map_err(ControlError::storage)
                })
                .await
                .map_err(ControlError::storage)?
            }
            #[cfg(test)]
            Self::Fixed(_) | Self::Test { .. } => Ok(None),
        }
    }

    async fn image_ledger(&self, id: &str) -> Result<Option<ImageLedgerRecord>, ControlError> {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(store) => store.image_ledger(id).await.map_err(ControlError::storage),
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            Self::SqlCipher(store) => {
                let store = Arc::clone(store);
                let id = id.to_owned();
                lifecycle::spawn_blocking(move || {
                    store
                        .lock()
                        .map_err(|_| ControlError::DataIntegrityInvalid)?
                        .image_ledger(&id)
                        .map_err(ControlError::storage)
                })
                .await
                .map_err(ControlError::storage)?
            }
            #[cfg(test)]
            Self::Fixed(_) | Self::Test { .. } => Ok(None),
        }
    }

    async fn image_ledger_for_member(
        &self,
        identity_id: &str,
    ) -> Result<Vec<ImageLedgerRecord>, ControlError> {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(store) => store
                .image_ledger_for_member(identity_id)
                .await
                .map_err(ControlError::storage),
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            Self::SqlCipher(store) => {
                let store = Arc::clone(store);
                let identity_id = identity_id.to_owned();
                lifecycle::spawn_blocking(move || {
                    store
                        .lock()
                        .map_err(|_| ControlError::DataIntegrityInvalid)?
                        .image_ledger_for_member(&identity_id)
                        .map_err(ControlError::storage)
                })
                .await
                .map_err(ControlError::storage)?
            }
            #[cfg(test)]
            Self::Fixed(_) | Self::Test { .. } => Ok(Vec::new()),
        }
    }

    async fn mutate_image_quota(
        &self,
        write: ImageQuotaWrite,
    ) -> Result<ImageQuotaMutationOutcome, ControlError> {
        match self {
            #[cfg(feature = "mariadb")]
            Self::MariaDb(store) => store
                .mutate_image_quota(
                    write.expected_balance.as_ref(),
                    &write.next_balance,
                    write.expected_reservation.as_ref(),
                    write.next_reservation.as_ref(),
                    &write.ledger,
                    write
                        .audit
                        .as_ref()
                        .map(|(sequence, hmac, event)| (*sequence, hmac.as_str(), event)),
                )
                .await
                .map_err(ControlError::storage),
            #[cfg(any(feature = "sqlcipher", feature = "sqlite-dev"))]
            Self::SqlCipher(store) => {
                let store = Arc::clone(store);
                lifecycle::spawn_blocking(move || {
                    store
                        .lock()
                        .map_err(|_| ControlError::DataIntegrityInvalid)?
                        .mutate_image_quota(
                            write.expected_balance.as_ref(),
                            &write.next_balance,
                            write.expected_reservation.as_ref(),
                            write.next_reservation.as_ref(),
                            &write.ledger,
                            write
                                .audit
                                .as_ref()
                                .map(|(sequence, hmac, event)| (*sequence, hmac.as_str(), event)),
                        )
                        .map_err(ControlError::storage)
                })
                .await
                .map_err(ControlError::storage)?
            }
            #[cfg(test)]
            Self::Fixed(_) | Self::Test { .. } => Err(ControlError::DataIntegrityInvalid),
        }
    }
}

impl ControlState {
    pub(crate) async fn admin_image_adjustment_history(
        &self,
        identity_id: &str,
    ) -> Result<Vec<Value>, ControlError> {
        let models = self.credential_storage()?.list_models().await?;
        let model_names = models
            .into_iter()
            .map(|model| (model.id, model.public_name))
            .collect::<std::collections::HashMap<_, _>>();
        let rows = self
            .credential_storage()?
            .image_ledger_for_member(identity_id)
            .await?;
        let mut items = Vec::new();
        for row in rows {
            if self.image_ledger_hmac(&row)? != row.integrity_hmac || row.identity_id != identity_id
            {
                return Err(ControlError::DataIntegrityInvalid);
            }
            if row.kind != "adjust" {
                continue;
            }
            let admin_email = self
                .credential_storage()?
                .identity_by_id(&row.actor)
                .await?
                .map(|identity| identity.email)
                .unwrap_or_else(|| "deleted-admin".to_owned());
            items.push(json!({
                "id": row.id,
                "amount_images": row.amount_images,
                "model": model_names.get(&row.public_model_id).map_or(row.public_model_id.as_str(), String::as_str),
                "reason": row.reason,
                "created_at": row.created_at,
                "admin_email": admin_email,
            }));
        }
        Ok(items)
    }

    pub(crate) async fn grant_member_quotas_with_audit(
        &self,
        admin: &aster_storage::IdentityRecord,
        identity_id: &str,
        request: &GrantMemberQuotaRequest,
    ) -> Result<(i64, i64), ControlError> {
        validate_admin_actor(self, admin).await?;
        authorize_non_consuming_feature(self, "member")?;
        validate_quota_grant_target(self, identity_id).await?;
        let reason = request.reason.trim();
        let mut adjustments = request.image_adjustments.iter().collect::<Vec<_>>();
        adjustments.sort_by(|left, right| left.model_id.cmp(&right.model_id));
        if adjustments
            .windows(2)
            .any(|pair| pair[0].model_id == pair[1].model_id)
        {
            return Err(ControlError::QuotaAdjustmentInvalid);
        }
        for _ in 0..QUOTA_MUTATION_RETRIES {
            let snapshot = self.verified_quota_snapshot(identity_id).await?;
            let token = if request.amount_tokens > 0 {
                let created_at = format_database_time((self.now)())
                    .map_err(|_| ControlError::DataIntegrityInvalid)?;
                let mut ledger = QuotaLedgerEntry {
                    id: random_identifier("ledger")?,
                    identity_id: identity_id.to_owned(),
                    kind: "grant".to_owned(),
                    amount_tokens: request.amount_tokens,
                    uncached_input: 0,
                    cached_input: 0,
                    cache_write: 0,
                    output_tokens: 0,
                    uncovered_tokens: 0,
                    raw_tokens: 0,
                    billed_tokens: 0,
                    multiplier_micros: 1_000_000,
                    reference_id: format!("admin_grant:{}:{}", admin.id, request.request_id),
                    client_request_id: None,
                    description: reason.to_owned(),
                    protocol: String::new(),
                    model: String::new(),
                    requested_model: None,
                    processing_tier: None,
                    reasoning_effort: None,
                    api_key_id: None,
                    runner_id: None,
                    previous_entry_hmac: snapshot.balance.last_ledger_hmac.clone(),
                    integrity_hmac: String::new(),
                    created_at: created_at.clone(),
                };
                ledger.integrity_hmac = self.ledger_integrity_hmac(&ledger)?;
                let mut next_balance = snapshot.balance.clone();
                next_balance.balance_tokens =
                    checked_add_nonnegative(next_balance.balance_tokens, request.amount_tokens)?;
                next_balance.granted_tokens =
                    checked_add_nonnegative(next_balance.granted_tokens, request.amount_tokens)?;
                next_balance.revision = next_balance
                    .revision
                    .checked_add(1)
                    .ok_or(ControlError::DataIntegrityInvalid)?;
                next_balance.last_ledger_hmac = ledger.integrity_hmac.clone();
                next_balance.updated_at = created_at;
                next_balance.integrity_hmac = self.balance_integrity_hmac(&next_balance)?;
                Some(QuotaBatchTokenWrite {
                    expected_balance: snapshot.balance.clone(),
                    next_balance,
                    ledger,
                })
            } else {
                None
            };
            let response_balance = token
                .as_ref()
                .map_or(&snapshot.balance, |write| &write.next_balance);
            let result = (
                response_balance.balance_tokens,
                response_balance.granted_tokens,
            );
            let mut images = Vec::with_capacity(adjustments.len());
            for adjustment in &adjustments {
                if self
                    .credential_storage()?
                    .image_quota_unit(&adjustment.model_id)
                    .await?
                    .as_deref()
                    != Some("image")
                {
                    return Err(ControlError::ModelNotFound);
                }
                let expected = self
                    .verified_image_balance(identity_id, &adjustment.model_id)
                    .await?;
                if expected.as_ref().map_or(0, |value| value.revision)
                    != adjustment.expected_revision
                {
                    return Err(ControlError::QuotaUpdateConflict);
                }
                let available = expected
                    .as_ref()
                    .map_or(0, |value| value.available_images)
                    .checked_add(adjustment.delta_images)
                    .filter(|value| *value >= 0)
                    .ok_or(ControlError::QuotaUpdateConflict)?;
                let now = format_database_time((self.now)())
                    .map_err(|_| ControlError::DataIntegrityInvalid)?;
                let mut ledger = ImageLedgerRecord {
                    id: format!(
                        "image_adjust_{}",
                        crate::sha256_hex(
                            format!("{}:{}", request.request_id, adjustment.model_id).as_bytes(),
                        )
                    ),
                    identity_id: identity_id.to_owned(),
                    public_model_id: adjustment.model_id.clone(),
                    reservation_id: None,
                    child_id: None,
                    kind: "adjust".to_owned(),
                    amount_images: adjustment.delta_images,
                    produced_images: 0,
                    delivery_state: "none".to_owned(),
                    actor: admin.id.clone(),
                    reason: reason.to_owned(),
                    previous_hmac: expected
                        .as_ref()
                        .map_or(String::new(), |value| value.last_ledger_hmac.clone()),
                    integrity_hmac: String::new(),
                    created_at: now,
                };
                ledger.integrity_hmac = self.image_ledger_hmac(&ledger)?;
                let mut next_balance = ImageBalanceRecord {
                    identity_id: identity_id.to_owned(),
                    public_model_id: adjustment.model_id.clone(),
                    available_images: available,
                    reserved_images: expected.as_ref().map_or(0, |value| value.reserved_images),
                    consumed_images: expected.as_ref().map_or(0, |value| value.consumed_images),
                    revision: expected.as_ref().map_or(Ok(0), |value| {
                        value
                            .revision
                            .checked_add(1)
                            .ok_or(ControlError::DataIntegrityInvalid)
                    })?,
                    last_ledger_hmac: ledger.integrity_hmac.clone(),
                    integrity_hmac: String::new(),
                };
                next_balance.integrity_hmac = self.image_balance_hmac(&next_balance)?;
                images.push(QuotaBatchImageWrite {
                    expected_balance: expected,
                    next_balance,
                    ledger,
                });
            }
            let (sequence, hmac, event) = self
                .prepare_audit_event(
                    Some(admin),
                    "member.quota.grant",
                    "identity",
                    Some(identity_id),
                    "succeeded",
                )
                .await?;
            validate_admin_actor(self, admin).await?;
            authorize_non_consuming_feature(self, "member")?;
            validate_quota_grant_target(self, identity_id).await?;
            match self
                .credential_storage()?
                .grant_quota_batch_and_audit(token, images, sequence, &hmac, event)
                .await?
            {
                MutationWithAuditOutcome::Mutation(QuotaMutationOutcome::Applied) => {
                    return Ok(result);
                }
                MutationWithAuditOutcome::Mutation(QuotaMutationOutcome::Conflict)
                | MutationWithAuditOutcome::AuditConflict => continue,
                MutationWithAuditOutcome::Mutation(QuotaMutationOutcome::DuplicateRequest) => {
                    return Err(ControlError::QuotaGrantDuplicate);
                }
                MutationWithAuditOutcome::Mutation(QuotaMutationOutcome::Insufficient) => {
                    return Err(ControlError::DataIntegrityInvalid);
                }
            }
        }
        Err(ControlError::QuotaUpdateConflict)
    }

    async fn member_image_ledger(&self, identity_id: &str) -> Result<Value, ControlError> {
        let rows = self
            .credential_storage()?
            .image_ledger_for_member(identity_id)
            .await?;
        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            if self.image_ledger_hmac(&row)? != row.integrity_hmac || row.identity_id != identity_id
            {
                return Err(ControlError::DataIntegrityInvalid);
            }
            items.push(json!({
                "id":row.id,"model_id":row.public_model_id,"reservation_id":row.reservation_id,
                "child_id":row.child_id,"kind":row.kind,"amount_images":row.amount_images,
                "produced_images":row.produced_images,"delivery_state":row.delivery_state,
                "created_at":row.created_at,
            }));
        }
        Ok(json!({"items":items}))
    }

    async fn reclaim_expired_image_reservations(&self) -> Result<(), ControlError> {
        let now =
            format_database_time((self.now)()).map_err(|_| ControlError::DataIntegrityInvalid)?;
        let reservations = self
            .credential_storage()?
            .expired_image_reservations(&now)
            .await?;
        for reservation in reservations {
            if self.image_reservation_hmac(&reservation)? != reservation.integrity_hmac {
                return Err(ControlError::DataIntegrityInvalid);
            }
            if reservation.status != "reserved" || reservation.expires_at > now {
                return Err(ControlError::DataIntegrityInvalid);
            }
            for index in 0..reservation.reserved_images {
                let child = format!("child_{index}");
                let final_id = format!("image_child_{}_{}_final", reservation.id, child);
                if self.verified_image_ledger(&final_id).await?.is_none() {
                    self.settle_image_child(&reservation, &child, "release", "not_delivered")
                        .await?;
                }
            }
        }
        Ok(())
    }

    fn image_integrity_hmac(
        &self,
        key: &str,
        revision: i64,
        value: &impl serde::Serialize,
    ) -> Result<String, ControlError> {
        let value = serde_json::to_vec(value).map_err(|_| ControlError::DataIntegrityInvalid)?;
        self.auth_core()?
            .security_state_integrity_hmac(SecurityStateIntegrityInput {
                key,
                value: &value,
                revision: u64::try_from(revision)
                    .map_err(|_| ControlError::DataIntegrityInvalid)?,
            })
            .map_err(ControlError::Auth)
    }

    fn image_balance_hmac(&self, record: &ImageBalanceRecord) -> Result<String, ControlError> {
        self.image_integrity_hmac(
            IMAGE_BALANCE_KEY,
            record.revision,
            &(
                &record.identity_id,
                &record.public_model_id,
                record.available_images,
                record.reserved_images,
                record.consumed_images,
                &record.last_ledger_hmac,
            ),
        )
    }

    fn image_reservation_hmac(
        &self,
        record: &ImageReservationRecord,
    ) -> Result<String, ControlError> {
        self.image_integrity_hmac(
            IMAGE_RESERVATION_KEY,
            record.revision,
            &(
                &record.id,
                &record.identity_id,
                &record.api_key_id,
                &record.public_model_id,
                &record.request_id,
                record.reserved_images,
                record.confirmed_images,
                record.released_images,
                &record.status,
                &record.created_at,
                &record.expires_at,
                &record.settled_at,
            ),
        )
    }

    fn image_ledger_hmac(&self, record: &ImageLedgerRecord) -> Result<String, ControlError> {
        self.image_integrity_hmac(
            IMAGE_LEDGER_KEY,
            0,
            &(
                &record.id,
                &record.identity_id,
                &record.public_model_id,
                &record.reservation_id,
                &record.child_id,
                &record.kind,
                record.amount_images,
                record.produced_images,
                &record.delivery_state,
                &record.actor,
                &record.reason,
                &record.previous_hmac,
                &record.created_at,
            ),
        )
    }

    pub(crate) async fn verified_image_balance(
        &self,
        identity_id: &str,
        model_id: &str,
    ) -> Result<Option<ImageBalanceRecord>, ControlError> {
        let record = self
            .credential_storage()?
            .image_balance(identity_id, model_id)
            .await?;
        if let Some(record) = record.as_ref()
            && (record.integrity_hmac != self.image_balance_hmac(record)?
                || record.available_images < 0
                || record.reserved_images < 0
                || record.consumed_images < 0)
        {
            return Err(ControlError::DataIntegrityInvalid);
        }
        Ok(record)
    }

    pub(crate) async fn verified_image_reservation(
        &self,
        id: &str,
    ) -> Result<Option<ImageReservationRecord>, ControlError> {
        let record = self.credential_storage()?.image_reservation(id).await?;
        if let Some(record) = record.as_ref()
            && (record.integrity_hmac != self.image_reservation_hmac(record)?
                || record.reserved_images <= 0
                || record.confirmed_images < 0
                || record.released_images < 0
                || record
                    .confirmed_images
                    .checked_add(record.released_images)
                    .is_none_or(|value| value > record.reserved_images))
        {
            return Err(ControlError::DataIntegrityInvalid);
        }
        Ok(record)
    }

    async fn verified_image_ledger(
        &self,
        id: &str,
    ) -> Result<Option<ImageLedgerRecord>, ControlError> {
        let record = self.credential_storage()?.image_ledger(id).await?;
        if let Some(record) = record.as_ref()
            && record.integrity_hmac != self.image_ledger_hmac(record)?
        {
            return Err(ControlError::DataIntegrityInvalid);
        }
        Ok(record)
    }

    async fn image_quota_listing(
        &self,
        identity_id: &str,
        permitted: bool,
    ) -> Result<Value, ControlError> {
        self.reclaim_expired_image_reservations().await?;
        let mut items = Vec::new();
        let permitted_models = if permitted {
            self.permitted_model_ids(identity_id).await?
        } else {
            None
        };
        for model in self.credential_storage()?.list_models().await? {
            if self
                .credential_storage()?
                .image_quota_unit(&model.id)
                .await?
                .as_deref()
                != Some("image")
            {
                continue;
            }
            if permitted_models
                .as_ref()
                .is_some_and(|ids| !ids.contains(&model.id))
            {
                continue;
            }
            let balance = self.verified_image_balance(identity_id, &model.id).await?;
            items.push(json!({
                "model_id":model.id, "model":model.public_name, "enabled":model.enabled,
                "available_images":balance.as_ref().map_or(0, |value| value.available_images),
                "reserved_images":balance.as_ref().map_or(0, |value| value.reserved_images),
                "consumed_images":balance.as_ref().map_or(0, |value| value.consumed_images),
                "revision":balance.as_ref().map_or(0, |value| value.revision),
            }));
        }
        Ok(json!({"items":items}))
    }

    async fn adjust_image_quota(
        &self,
        admin: &aster_storage::IdentityRecord,
        identity_id: &str,
        model_id: &str,
        request: &AdjustImageQuotaRequest,
    ) -> Result<Value, ControlError> {
        let delta = request.delta_images;
        let expected_revision = request.expected_revision;
        let adjustment_id = request.adjustment_id.as_str();
        let reason = request.reason.trim();
        if delta == 0
            || !(-1_000_000..=1_000_000).contains(&delta)
            || expected_revision < 0
            || !valid_client_request_id(adjustment_id)
            || !(2..=200).contains(&reason.chars().count())
            || reason.chars().any(char::is_control)
        {
            return Err(ControlError::QuotaAdjustmentInvalid);
        }
        let identity = self
            .credential_storage()?
            .identity_by_id(identity_id)
            .await?
            .filter(|value| value.role == "member" && value.status != "deleted")
            .ok_or(ControlError::IdentityNotFound)?;
        verify_identity_integrity(self.auth_core()?, &identity)?;
        if self
            .credential_storage()?
            .image_quota_unit(model_id)
            .await?
            .as_deref()
            != Some("image")
        {
            return Err(ControlError::ModelNotFound);
        }
        let ledger_id = format!(
            "image_adjust_{}",
            crate::sha256_hex(adjustment_id.as_bytes())
        );
        if let Some(previous) = self.verified_image_ledger(&ledger_id).await? {
            if previous.identity_id == identity_id
                && previous.public_model_id == model_id
                && previous.kind == "adjust"
                && previous.amount_images == delta
                && previous.actor == admin.id
                && previous.reason == reason
            {
                let balance = self
                    .verified_image_balance(identity_id, model_id)
                    .await?
                    .ok_or(ControlError::DataIntegrityInvalid)?;
                return Ok(json!({
                    "model_id":model_id,"identity_id":identity_id,
                    "available_images":balance.available_images,
                    "reserved_images":balance.reserved_images,
                    "consumed_images":balance.consumed_images,
                    "revision":balance.revision,
                }));
            }
            return Err(ControlError::QuotaReservationConflict);
        }
        let expected = self.verified_image_balance(identity_id, model_id).await?;
        if expected.as_ref().map_or(0, |value| value.revision) != expected_revision {
            return Err(ControlError::QuotaReservationConflict);
        }
        let available = expected
            .as_ref()
            .map_or(0, |value| value.available_images)
            .checked_add(delta)
            .filter(|value| *value >= 0)
            .ok_or(ControlError::QuotaReservationConflict)?;
        let now =
            format_database_time((self.now)()).map_err(|_| ControlError::DataIntegrityInvalid)?;
        let mut ledger = ImageLedgerRecord {
            id: ledger_id,
            identity_id: identity_id.into(),
            public_model_id: model_id.into(),
            reservation_id: None,
            child_id: None,
            kind: "adjust".into(),
            amount_images: delta,
            produced_images: 0,
            delivery_state: "none".into(),
            actor: admin.id.clone(),
            reason: reason.into(),
            previous_hmac: expected
                .as_ref()
                .map_or(String::new(), |value| value.last_ledger_hmac.clone()),
            integrity_hmac: String::new(),
            created_at: now,
        };
        ledger.integrity_hmac = self.image_ledger_hmac(&ledger)?;
        let mut next = ImageBalanceRecord {
            identity_id: identity_id.into(),
            public_model_id: model_id.into(),
            available_images: available,
            reserved_images: expected.as_ref().map_or(0, |value| value.reserved_images),
            consumed_images: expected.as_ref().map_or(0, |value| value.consumed_images),
            revision: expected.as_ref().map_or(Ok(0), |value| {
                value
                    .revision
                    .checked_add(1)
                    .ok_or(ControlError::DataIntegrityInvalid)
            })?,
            last_ledger_hmac: ledger.integrity_hmac.clone(),
            integrity_hmac: String::new(),
        };
        next.integrity_hmac = self.image_balance_hmac(&next)?;
        for _ in 0..4 {
            let (sequence, hmac, event) = self
                .prepare_audit_event(
                    Some(admin),
                    "image_quota.adjust",
                    "model",
                    Some(model_id),
                    "succeeded",
                )
                .await?;
            let write = ImageQuotaWrite {
                expected_balance: expected.clone(),
                next_balance: next.clone(),
                expected_reservation: None,
                next_reservation: None,
                ledger: ledger.clone(),
                audit: Some((sequence, hmac, event)),
            };
            match self.credential_storage()?.mutate_image_quota(write).await? {
                ImageQuotaMutationOutcome::Applied => {
                    return Ok(json!({
                        "model_id":model_id,"identity_id":identity_id,"available_images":available,
                        "reserved_images":next.reserved_images,"consumed_images":next.consumed_images,
                        "revision":next.revision,
                    }));
                }
                ImageQuotaMutationOutcome::Conflict => continue,
                ImageQuotaMutationOutcome::Duplicate(previous) => {
                    if previous.identity_id == identity_id
                        && previous.public_model_id == model_id
                        && previous.kind == "adjust"
                        && previous.amount_images == delta
                        && previous.actor == admin.id
                        && previous.reason == reason
                    {
                        let balance = self
                            .verified_image_balance(identity_id, model_id)
                            .await?
                            .ok_or(ControlError::DataIntegrityInvalid)?;
                        return Ok(json!({
                            "model_id":model_id,"identity_id":identity_id,
                            "available_images":balance.available_images,
                            "reserved_images":balance.reserved_images,
                            "consumed_images":balance.consumed_images,
                            "revision":balance.revision,
                        }));
                    }
                    return Err(ControlError::QuotaReservationConflict);
                }
            }
        }
        Err(ControlError::QuotaReservationConflict)
    }

    pub(crate) async fn settle_image_child(
        &self,
        reservation: &ImageReservationRecord,
        child_id: &str,
        outcome: &str,
        delivery_state: &str,
    ) -> Result<(), ControlError> {
        if !matches!(outcome, "confirm" | "release") {
            return Err(ControlError::UsageSettlementInvalid);
        }
        self.update_image_child(reservation, child_id, outcome, delivery_state)
            .await
    }

    async fn update_image_child(
        &self,
        reservation: &ImageReservationRecord,
        child_id: &str,
        outcome: &str,
        delivery_state: &str,
    ) -> Result<(), ControlError> {
        if child_id.is_empty() || child_id.len() > 128 || child_id.chars().any(char::is_control) {
            return Err(ControlError::UsageSettlementInvalid);
        }
        let marker = match outcome {
            "dispatch" => "dispatch",
            "delivery" => "delivery",
            _ => "final",
        };
        let ledger_id = format!("image_child_{}_{}_{}", reservation.id, child_id, marker);
        if let Some(previous) = self.verified_image_ledger(&ledger_id).await? {
            return if previous.kind == outcome
                && previous.delivery_state == delivery_state
                && previous.reservation_id.as_deref() == Some(&reservation.id)
            {
                Ok(())
            } else {
                Err(ControlError::QuotaSettlementConflict)
            };
        }
        if !matches!(outcome, "dispatch" | "release") {
            let dispatch_id = format!("image_child_{}_{}_dispatch", reservation.id, child_id);
            let dispatch = self
                .verified_image_ledger(&dispatch_id)
                .await?
                .ok_or(ControlError::QuotaSettlementConflict)?;
            if dispatch.kind != "dispatch"
                || dispatch.reservation_id.as_deref() != Some(&reservation.id)
                || dispatch.child_id.as_deref() != Some(child_id)
            {
                return Err(ControlError::QuotaSettlementConflict);
            }
        }
        for _ in 0..4 {
            let previous_reservation = self
                .verified_image_reservation(&reservation.id)
                .await?
                .ok_or(ControlError::QuotaSettlementConflict)?;
            if previous_reservation.identity_id != reservation.identity_id
                || previous_reservation.api_key_id != reservation.api_key_id
                || previous_reservation.public_model_id != reservation.public_model_id
                || previous_reservation.request_id != reservation.request_id
            {
                return Err(ControlError::DataIntegrityInvalid);
            }
            let previous = self
                .verified_image_balance(&reservation.identity_id, &reservation.public_model_id)
                .await?
                .ok_or(ControlError::DataIntegrityInvalid)?;
            let completed = previous_reservation
                .confirmed_images
                .checked_add(previous_reservation.released_images)
                .ok_or(ControlError::DataIntegrityInvalid)?;
            if !matches!(outcome, "dispatch" | "delivery")
                && completed >= reservation.reserved_images
            {
                return Err(ControlError::QuotaSettlementConflict);
            }
            let mut next_reservation = previous_reservation.clone();
            let mut next = previous.clone();
            let amount = match outcome {
                "confirm" => {
                    next.reserved_images = next
                        .reserved_images
                        .checked_sub(1)
                        .ok_or(ControlError::DataIntegrityInvalid)?;
                    next.consumed_images = next
                        .consumed_images
                        .checked_add(1)
                        .ok_or(ControlError::DataIntegrityInvalid)?;
                    next_reservation.confirmed_images = next_reservation
                        .confirmed_images
                        .checked_add(1)
                        .ok_or(ControlError::DataIntegrityInvalid)?;
                    1
                }
                "release" => {
                    next.reserved_images = next
                        .reserved_images
                        .checked_sub(1)
                        .ok_or(ControlError::DataIntegrityInvalid)?;
                    next.available_images = next
                        .available_images
                        .checked_add(1)
                        .ok_or(ControlError::DataIntegrityInvalid)?;
                    next_reservation.released_images = next_reservation
                        .released_images
                        .checked_add(1)
                        .ok_or(ControlError::DataIntegrityInvalid)?;
                    1
                }
                "dispatch" | "delivery" => 0,
                _ => return Err(ControlError::UsageSettlementInvalid),
            };
            if next.reserved_images < 0 {
                return Err(ControlError::DataIntegrityInvalid);
            }
            if !matches!(outcome, "dispatch" | "delivery") {
                let total = next_reservation
                    .confirmed_images
                    .checked_add(next_reservation.released_images)
                    .ok_or(ControlError::DataIntegrityInvalid)?;
                if total == next_reservation.reserved_images {
                    next_reservation.status = "settled".into();
                    next_reservation.settled_at = Some(
                        format_database_time((self.now)())
                            .map_err(|_| ControlError::DataIntegrityInvalid)?,
                    );
                }
            }
            next_reservation.revision = next_reservation
                .revision
                .checked_add(1)
                .ok_or(ControlError::DataIntegrityInvalid)?;
            next_reservation.integrity_hmac = self.image_reservation_hmac(&next_reservation)?;
            let mut ledger = ImageLedgerRecord {
                id: ledger_id.clone(),
                identity_id: reservation.identity_id.clone(),
                public_model_id: reservation.public_model_id.clone(),
                reservation_id: Some(reservation.id.clone()),
                child_id: Some(child_id.into()),
                kind: outcome.into(),
                amount_images: amount,
                produced_images: i64::from(outcome == "confirm"),
                delivery_state: delivery_state.into(),
                actor: reservation.identity_id.clone(),
                reason: "image child outcome".into(),
                previous_hmac: previous.last_ledger_hmac.clone(),
                integrity_hmac: String::new(),
                created_at: format_database_time((self.now)())
                    .map_err(|_| ControlError::DataIntegrityInvalid)?,
            };
            ledger.integrity_hmac = self.image_ledger_hmac(&ledger)?;
            next.revision = next
                .revision
                .checked_add(1)
                .ok_or(ControlError::DataIntegrityInvalid)?;
            next.last_ledger_hmac = ledger.integrity_hmac.clone();
            next.integrity_hmac = self.image_balance_hmac(&next)?;
            let write = ImageQuotaWrite {
                expected_balance: Some(previous),
                next_balance: next,
                expected_reservation: Some(previous_reservation),
                next_reservation: Some(next_reservation),
                ledger,
                audit: None,
            };
            match self.credential_storage()?.mutate_image_quota(write).await? {
                ImageQuotaMutationOutcome::Applied => return Ok(()),
                ImageQuotaMutationOutcome::Conflict => continue,
                ImageQuotaMutationOutcome::Duplicate(previous) => {
                    return if previous.kind == outcome && previous.delivery_state == delivery_state
                    {
                        Ok(())
                    } else {
                        Err(ControlError::QuotaSettlementConflict)
                    };
                }
            }
        }
        Err(ControlError::QuotaSettlementConflict)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdjustImageQuotaRequest {
    delta_images: i64,
    expected_revision: i64,
    adjustment_id: String,
    reason: String,
}

pub(crate) async fn admin_image_quotas(
    State(state): State<ControlState>,
    headers: HeaderMap,
    AxumPath(identity_id): AxumPath<String>,
) -> Result<Json<Value>, ControlError> {
    require_admin_ready(&state, &headers).await?;
    if !valid_resource_identifier(&identity_id, "identity_") {
        return Err(ControlError::IdentityInvalid);
    }
    Ok(Json(state.image_quota_listing(&identity_id, false).await?))
}

pub(crate) async fn member_image_quotas(
    State(state): State<ControlState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ControlError> {
    let member = require_member_ready(&state, &headers).await?;
    Ok(Json(state.image_quota_listing(&member.id, true).await?))
}

pub(crate) async fn member_image_ledger(
    State(state): State<ControlState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ControlError> {
    let member = require_member_ready(&state, &headers).await?;
    Ok(Json(state.member_image_ledger(&member.id).await?))
}

pub(crate) async fn admin_adjust_image_quota(
    State(state): State<ControlState>,
    headers: HeaderMap,
    AxumPath((identity_id, model_id)): AxumPath<(String, String)>,
    request: Result<Json<AdjustImageQuotaRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>, ControlError> {
    let admin = require_admin_ready(&state, &headers).await?;
    authorize_non_consuming_feature(&state, "member")?;
    let Json(request) = request.map_err(|_| ControlError::QuotaAdjustmentInvalid)?;
    let result = state
        .adjust_image_quota(&admin, &identity_id, &model_id, &request)
        .await?;
    Ok(Json(result))
}
