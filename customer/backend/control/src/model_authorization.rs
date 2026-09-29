use super::*;

#[cfg(all(test, any(feature = "sqlcipher", feature = "sqlite-dev")))]
mod tests;

/// An authenticated consumer, not a perpetual grant. Only a real API Key can
/// create this handle; every reservation resolves its current identity and
/// receives a fresh, operation-specific License permit.
#[derive(Clone)]
pub struct ModelConsumer {
    domain: Arc<()>,
    key_hash: String,
    identity_id: String,
    api_key_id: String,
}

impl ModelConsumer {
    pub(crate) fn identity_id(&self) -> &str {
        &self.identity_id
    }

    pub(crate) fn api_key_id(&self) -> &str {
        &self.api_key_id
    }

    pub(super) async fn revalidate(
        &self,
        state: &ControlState,
    ) -> Result<AuthorizedApiKey, ControlError> {
        if !Arc::ptr_eq(&self.domain, &state.authorization_domain) {
            return Err(ControlError::ExternalApiKeyInvalid);
        }
        let current = verified_consumer(state, &self.key_hash).await?;
        if current.identity.id != self.identity_id || current.api_key.id != self.api_key_id {
            return Err(ControlError::ExternalApiKeyInvalid);
        }
        Ok(current)
    }
}

async fn verified_consumer(
    state: &ControlState,
    key_hash: &str,
) -> Result<AuthorizedApiKey, ControlError> {
    let current = state
        .credential_storage()?
        .authorized_api_key_by_hash(key_hash)
        .await?
        .ok_or(ControlError::ExternalApiKeyInvalid)?;
    verify_api_key_integrity(state.auth_core()?, &current.api_key)?;
    verify_identity_integrity(state.auth_core()?, &current.identity)?;
    if current.identity.role != "member"
        || current.identity.status != "active"
        || !current.identity.can_consume_model
        || current.identity.password_change_required
        || current.api_key.status != "active"
        || current.api_key.identity_id != current.identity.id
    {
        return Err(ControlError::ExternalApiKeyInvalid);
    }
    Ok(current)
}

// Private fields and construction keep this permit specific to a verified
// subject, a current License and one reservation. It is never serialized or
// accepted as a caller-provided flag. D03's additional capability composition
// remains pending; this boundary uses the existing member consumption policy.
struct ReservationPermit {
    subject: AuthorizedApiKey,
    license: VerifiedProductLicense,
}

impl ReservationPermit {
    async fn authorize(
        state: &ControlState,
        consumer: &ModelConsumer,
    ) -> Result<Self, ControlError> {
        let subject = consumer.revalidate(state).await?;
        let (license, _) = authorize_model_consumption_license(state, "member").await?;
        Ok(Self { subject, license })
    }
}

struct PreparedReservation {
    expected: UserBalanceRecord,
    next: UserBalanceRecord,
    record: QuotaReservationRecord,
}

impl PreparedReservation {
    async fn new(
        state: &ControlState,
        subject: &AuthorizedApiKey,
        request_id: &str,
        reserved_tokens: i64,
    ) -> Result<Self, ControlError> {
        if !(1..=256).contains(&request_id.len()) || reserved_tokens <= 0 {
            return Err(ControlError::GatewayRequestInvalid);
        }
        let now = (state.now)();
        let created_at =
            format_database_time(now).map_err(|_| ControlError::DataIntegrityInvalid)?;
        let expires_at = format_database_time(now + QUOTA_RESERVATION_TTL)
            .map_err(|_| ControlError::DataIntegrityInvalid)?;
        let mut record = QuotaReservationRecord {
            id: random_identifier("quota")?,
            identity_id: subject.identity.id.clone(),
            api_key_id: subject.api_key.id.clone(),
            request_id: request_id.to_owned(),
            reserved_tokens,
            status: "active".to_owned(),
            revision: 0,
            integrity_hmac: String::new(),
            created_at: created_at.clone(),
            expires_at,
            settled_at: None,
        };
        record.integrity_hmac = state.reservation_integrity_hmac(&record)?;
        let expected = state
            .verified_quota_snapshot(&subject.identity.id)
            .await?
            .balance;
        let mut next = expected.clone();
        next.reserved_tokens = next
            .reserved_tokens
            .checked_add(reserved_tokens)
            .ok_or(ControlError::DataIntegrityInvalid)?;
        if next.reserved_tokens > next.balance_tokens {
            return Err(ControlError::InsufficientQuota);
        }
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or(ControlError::DataIntegrityInvalid)?;
        next.updated_at = created_at;
        next.integrity_hmac = state.balance_integrity_hmac(&next)?;
        Ok(Self {
            expected,
            next,
            record,
        })
    }

    async fn commit(
        self,
        storage: ControlStorage,
        subject: AuthorizedApiKey,
    ) -> Result<Option<QuotaReservation>, ControlError> {
        match storage
            .reserve_quota(&subject, &self.expected, &self.next, &self.record)
            .await?
        {
            QuotaMutationOutcome::Applied => Ok(Some(QuotaReservation {
                id: self.record.id,
                identity_id: self.record.identity_id,
                request_id: self.record.request_id,
                client_request_id: None,
                reserved_tokens: self.record.reserved_tokens,
                billing: None,
                _money_permit: None,
                execution: None,
            })),
            QuotaMutationOutcome::Conflict => Ok(None),
            QuotaMutationOutcome::Insufficient => Err(ControlError::InsufficientQuota),
            QuotaMutationOutcome::DuplicateRequest => Err(ControlError::DuplicateGatewayRequest),
        }
    }
}

impl ControlState {
    pub async fn authorize_model_consumer(
        &self,
        token: &str,
    ) -> Result<ModelConsumer, ControlError> {
        let key_hash = self
            .auth_core()?
            .api_key_digest(token)
            .map_err(|_| ControlError::ExternalApiKeyInvalid)?;
        let current = verified_consumer(self, &key_hash).await?;
        authorize_model_consumption(self, "member").await?;
        let last_used_at =
            format_database_time((self.now)()).map_err(|_| ControlError::DataIntegrityInvalid)?;
        if !self
            .credential_storage()?
            .touch_api_key_last_used(&current.api_key.id, &last_used_at)
            .await?
        {
            return Err(ControlError::ExternalApiKeyInvalid);
        }
        Ok(ModelConsumer {
            domain: Arc::clone(&self.authorization_domain),
            key_hash,
            identity_id: current.identity.id,
            api_key_id: current.api_key.id,
        })
    }

    pub async fn reserve_model_money(
        &self,
        consumer: &ModelConsumer,
        request_id: &str,
        public_model: &str,
        tier: &str,
        reference_request: &BillingReferenceRequest,
    ) -> Result<QuotaReservation, ControlError> {
        if !(1..=256).contains(&request_id.len()) {
            return Err(ControlError::GatewayRequestInvalid);
        }
        let execution = if let Some(store) = self.outbox()? {
            let request = request_id.to_owned();
            Some(
                lifecycle::spawn_blocking(move || store.acquire(&request))
                    .await
                    .map_err(|_| ControlError::DataIntegrityInvalid)??
                    .ok_or(ControlError::DuplicateGatewayRequest)?,
            )
        } else {
            None
        };
        let started = std::time::Instant::now();
        loop {
            let permit = ReservationPermit::authorize(self, consumer).await?;
            let frozen = self
                .freeze_billing_context(public_model, tier, reference_request)
                .await?;
            let balance = self
                .member_money_balance(&permit.subject.identity.id)
                .await?;
            let max_parallel = self
                .member_max_parallel(&permit.subject.identity.id)
                .await?;
            let currency = frozen.settlement_currency;
            let decision = self
                .money_admission
                .try_acquire(
                    &permit.subject.identity.id,
                    BillingMoney {
                        currency,
                        nanos: balance.balance_nanos,
                    },
                    BillingMoney {
                        currency,
                        nanos: frozen.reference_nanos,
                    },
                    max_parallel,
                )
                .map_err(|_| ControlError::BillingCalculationInvalid)?;
            match decision {
                AdmissionOutcome::Acquired(money_permit) => {
                    return Ok(QuotaReservation {
                        id: random_identifier("billing")?,
                        identity_id: permit.subject.identity.id,
                        request_id: request_id.to_owned(),
                        client_request_id: None,
                        reserved_tokens: 1,
                        billing: Some(frozen),
                        _money_permit: Some(Arc::new(money_permit)),
                        execution,
                    });
                }
                AdmissionOutcome::RejectInsufficientBalance => {
                    return Err(ControlError::InsufficientQuota);
                }
                AdmissionOutcome::Queue => {
                    if started.elapsed() >= RUNNER_EXECUTE_TASK_TIMEOUT {
                        return Err(ControlError::QuotaReservationConflict);
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            }
        }
    }

    pub async fn reserve_model_quota(
        &self,
        consumer: &ModelConsumer,
        request_id: &str,
        reserved_tokens: i64,
    ) -> Result<QuotaReservation, ControlError> {
        let execution = if let Some(store) = self.outbox()? {
            let request = request_id.to_owned();
            Some(
                lifecycle::spawn_blocking(move || store.acquire(&request))
                    .await
                    .map_err(|_| ControlError::DataIntegrityInvalid)??
                    .ok_or(ControlError::DuplicateGatewayRequest)?,
            )
        } else {
            None
        };
        for attempt in 0..QUOTA_MUTATION_RETRIES {
            let permit = ReservationPermit::authorize(self, consumer).await?;
            if attempt == 0 {
                self.release_expired_quota_reservations(&permit.subject.identity.id)
                    .await?;
            }
            let prepared =
                PreparedReservation::new(self, &permit.subject, request_id, reserved_tokens)
                    .await?;
            let guard = self.guard_licensed_mutation(Some(&permit.license), "member")?;
            let storage = self.credential_storage()?.clone();
            let retained = execution.clone();
            if let Some(reservation) =
                finish_licensed_mutation(Arc::clone(&self.mutation_tasks), guard, async move {
                    let mut result = prepared.commit(storage, permit.subject).await?;
                    if let Some(reservation) = &mut result {
                        reservation.execution = retained;
                    }
                    Ok(result)
                })
                .await?
            {
                return Ok(reservation);
            }
        }
        Err(ControlError::QuotaReservationConflict)
    }

    /// Developer fixture generation only. This symbol and its body are absent
    /// from Customer production builds, which never enable local-demo.
    #[cfg(any(test, feature = "local-demo"))]
    pub async fn reserve_mock_model_quota(
        &self,
        identity_id: &str,
        api_key_id: &str,
        request_id: &str,
        reserved_tokens: i64,
    ) -> Result<QuotaReservation, ControlError> {
        let key = self
            .credential_storage()?
            .api_keys_for_identity(identity_id)
            .await?
            .into_iter()
            .find(|key| key.id == api_key_id)
            .ok_or(ControlError::ExternalApiKeyInvalid)?;
        for attempt in 0..QUOTA_MUTATION_RETRIES {
            let subject = verified_consumer(self, &key.key_hash).await?;
            if attempt == 0 {
                self.release_expired_quota_reservations(identity_id).await?;
            }
            let prepared =
                PreparedReservation::new(self, &subject, request_id, reserved_tokens).await?;
            if let Some(reservation) = prepared
                .commit(self.credential_storage()?.clone(), subject)
                .await?
            {
                return Ok(reservation);
            }
        }
        Err(ControlError::QuotaReservationConflict)
    }
}
