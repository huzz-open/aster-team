//! Settlement survives HTTP cancellation. A lost database acknowledgement is
//! reconciled against the authenticated ledger before another write is tried.
use crate::{
    ControlError, ControlState, FailedModelRequest, ModelUsage, QuotaReservation,
    QuotaStateSnapshot, billed_tokens, checked_raw_tokens, finish_control_mutation,
};
use aster_policy_core::billing::{
    TokenUsage,
    plan::{ImageSpec, RateCard, VerifiedUsage},
};
use serde::{Deserialize, Serialize};
use std::{future::Future, sync::Arc, time::Duration};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(crate) enum Outcome {
    Usage(ModelUsage),
    ImageUsage {
        usage: ModelUsage,
        confirmed_count: u32,
        spec: ImageSpec,
    },
    Failure(FailedModelRequest),
    FailureWithUnknown(FailedModelRequest),
}

#[derive(Clone, Copy)]
enum Completion {
    Recorded,
    Expired,
}

pub(crate) fn expired_failure() -> FailedModelRequest {
    FailedModelRequest {
        protocol: "gateway_reservation_expired".into(),
        model: "unknown".into(),
        requested_model: None,
        processing_tier: None,
        reasoning_effort: None,
        runner_id: None,
        description: "预占到期回收，未计费且上游执行结果不确定".into(),
    }
}

fn invalid_usage_failure() -> FailedModelRequest {
    FailedModelRequest {
        protocol: "gateway_usage_invalid".into(),
        description: "上游用量无法结算，未计费且执行结果不确定".into(),
        ..expired_failure()
    }
}

pub(crate) async fn complete(
    state: &ControlState,
    reservation: &QuotaReservation,
    outcome: Outcome,
) -> Result<(), ControlError> {
    let intent = SettlementIntent::new(reservation, outcome);
    if !intent.valid() {
        return Err(ControlError::UsageSettlementInvalid);
    }
    let state = state.clone();
    let store = state.outbox()?;
    finish_control_mutation(Arc::clone(&state.mutation_tasks), async move {
        let owner = if let Some(store) = store {
            let owner = if let Some(owner) = &intent.reservation.execution {
                if !owner.matches(&store, &intent.reservation.request_id) {
                    return Err(ControlError::DataIntegrityInvalid);
                }
                Arc::clone(owner)
            } else {
                retry(|| {
                    let store = Arc::clone(&store);
                    let request = intent.reservation.request_id.clone();
                    async move {
                        crate::lifecycle::spawn_blocking(move || store.acquire(&request))
                            .await
                            .map_err(|_| ControlError::DataIntegrityInvalid)??
                            .ok_or(ControlError::QuotaSettlementConflict)
                    }
                })
                .await?
            };
            retry(|| persist(&state, &owner, &intent, false)).await?;
            Some(owner)
        } else {
            None
        };
        let completion = retry(|| {
            attempt(
                &state,
                &intent.reservation,
                &intent.outcome,
                intent.upstream_unknown,
            )
        })
        .await?;
        if intent.upstream_unknown {
            retry(|| record_unknown_audit(&state, &intent.reservation)).await?;
        }
        if let Some(owner) = owner {
            retry(|| persist(&state, &owner, &intent, true)).await?;
        }
        if intent.invalid_usage || matches!(completion, Completion::Expired) {
            Err(ControlError::UsageSettlementInvalid)
        } else {
            Ok(())
        }
    })
    .await
}

async fn persist(
    state: &ControlState,
    owner: &Arc<super::settlement_outbox::RequestLock>,
    intent: &SettlementIntent,
    finish: bool,
) -> Result<(), ControlError> {
    let owner = Arc::clone(owner);
    let intent = intent.clone();
    let auth = state.auth_core()?.clone();
    crate::lifecycle::spawn_blocking(move || {
        if finish {
            owner.finish(&auth, &intent)
        } else {
            owner.publish(&auth, &intent)
        }
    })
    .await
    .map_err(|_| ControlError::DataIntegrityInvalid)?
}

/// A background owner retries on a later scan instead of monopolizing a page.
/// Its caller retains the OS lock and mutation lease through every SQL await.
pub(crate) async fn recover_owned(
    state: &ControlState,
    owner: &Arc<super::settlement_outbox::RequestLock>,
) -> Result<bool, ControlError> {
    let retained = Arc::clone(owner);
    let auth = state.auth_core()?.clone();
    let Some(intent) = crate::lifecycle::spawn_blocking(move || retained.load(&auth))
        .await
        .map_err(|_| ControlError::DataIntegrityInvalid)??
    else {
        return Ok(false);
    };
    attempt(
        state,
        &intent.reservation,
        &intent.outcome,
        intent.upstream_unknown,
    )
    .await?;
    if intent.upstream_unknown {
        record_unknown_audit(state, &intent.reservation).await?;
    }
    persist(state, owner, &intent, true).await?;
    Ok(true)
}

/// Both foreground and scheduled TTL recovery enter this owned mutation before
/// opening the request lock. Cancellation cannot expose a still-running commit.
pub(crate) async fn reclaim_expired(
    state: &ControlState,
    reservation: &QuotaReservation,
) -> Result<(), ControlError> {
    let Some(store) = state.outbox()? else {
        return state
            .fail_model_request(reservation, &expired_failure())
            .await;
    };
    let state = state.clone();
    let reservation = reservation.clone();
    finish_control_mutation(Arc::clone(&state.mutation_tasks), async move {
        let request = reservation.request_id.clone();
        let Some(owner) = crate::lifecycle::spawn_blocking(move || store.acquire(&request))
            .await
            .map_err(|_| ControlError::DataIntegrityInvalid)??
        else {
            return Ok(());
        };
        if recover_owned(&state, &owner).await? {
            return Ok(());
        }
        let snapshot = state
            .verified_quota_snapshot(&reservation.identity_id)
            .await?;
        let Some(current) = snapshot
            .active_reservations
            .iter()
            .find(|r| r.id == reservation.id)
        else {
            return Ok(());
        };
        if current.request_id != reservation.request_id
            || current.reserved_tokens != reservation.reserved_tokens
        {
            return Err(ControlError::DataIntegrityInvalid);
        }
        let expires = time::OffsetDateTime::parse(
            &current.expires_at,
            &time::format_description::well_known::Rfc3339,
        )
        .map_err(|_| ControlError::DataIntegrityInvalid)?;
        if expires > (state.now)() {
            return Ok(());
        }
        state
            .fail_model_request(&reservation, &expired_failure())
            .await
    })
    .await
}

/// One recovery intent includes every required terminal action.
/// Persistence will serialize this normalized value, never reconstruct billing
/// from mutable configuration or infer audit intent after settling the quota.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SettlementIntent {
    pub(crate) reservation: QuotaReservation,
    outcome: Outcome,
    upstream_unknown: bool,
    invalid_usage: bool,
}

impl SettlementIntent {
    pub(crate) fn valid(&self) -> bool {
        let r = &self.reservation;
        if r.id.is_empty()
            || r.id.len() > 128
            || r.identity_id.is_empty()
            || r.identity_id.len() > 128
            || r.request_id.is_empty()
            || r.request_id.len() > 256
            || r.reserved_tokens <= 0
            || r.client_request_id
                .as_ref()
                .is_some_and(|id| id.is_empty() || id.len() > 256)
        {
            return false;
        }
        match &self.outcome {
            Outcome::Usage(usage) => {
                !self.upstream_unknown
                    && !self.invalid_usage
                    && (if r.billing.is_some() {
                        checked_raw_tokens(usage).is_ok()
                    } else {
                        checked_raw_tokens(usage)
                            .and_then(|raw| billed_tokens(raw, usage.multiplier_micros))
                            .is_ok()
                    })
            }
            Outcome::ImageUsage { usage, .. } => {
                r.billing.is_some()
                    && !self.upstream_unknown
                    && !self.invalid_usage
                    && checked_raw_tokens(usage).is_ok()
            }
            Outcome::Failure(failure) => {
                !failure.protocol.is_empty()
                    && !failure.model.is_empty()
                    && !failure.description.is_empty()
                    && failure.description.chars().count() <= 512
                    && (!self.invalid_usage
                        || (self.upstream_unknown && *failure == invalid_usage_failure()))
            }
            Outcome::FailureWithUnknown(_) => false,
        }
    }

    pub(crate) fn new(reservation: &QuotaReservation, outcome: Outcome) -> Self {
        let invalid_usage = match &outcome {
            Outcome::Usage(usage) if reservation.billing.is_some() => {
                checked_raw_tokens(usage).is_err()
            }
            Outcome::Usage(usage) => checked_raw_tokens(usage)
                .and_then(|raw| billed_tokens(raw, usage.multiplier_micros))
                .is_err(),
            Outcome::ImageUsage { usage, .. } => checked_raw_tokens(usage).is_err(),
            _ => false,
        };
        let (outcome, upstream_unknown) = if invalid_usage {
            (Outcome::Failure(invalid_usage_failure()), true)
        } else if let Outcome::FailureWithUnknown(failure) = outcome {
            (Outcome::Failure(failure), true)
        } else {
            (outcome, false)
        };
        Self {
            reservation: reservation.clone(),
            outcome,
            upstream_unknown,
            invalid_usage,
        }
    }
}

/// No response deadline cancels a durable write or turns an unresolved outcome
/// into successful drain. Retry failures retain the original task lease.
pub(crate) async fn retry<F, Fut, T>(mut operation: F) -> Result<T, ControlError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, ControlError>>,
{
    let mut failures = 0_u64;
    loop {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) => {
                failures = failures.saturating_add(1);
                if failures.is_power_of_two() {
                    tracing::warn!(
                        error_number = error.descriptor().number,
                        failures,
                        "durable gateway settlement remains unresolved; retaining request ownership"
                    );
                }
                tokio::time::sleep(Duration::from_secs(failures.min(5))).await;
            }
        }
    }
}

async fn attempt(
    state: &ControlState,
    reservation: &QuotaReservation,
    outcome: &Outcome,
    upstream_unknown: bool,
) -> Result<Completion, ControlError> {
    if let Some(frozen) = &reservation.billing {
        match outcome {
            Outcome::Usage(usage) => {
                let verified = VerifiedUsage::Tokens(money_token_usage(usage)?);
                state
                    .settle_member_money(
                        &reservation.identity_id,
                        &reservation.request_id,
                        frozen,
                        verified,
                        &usage.model,
                    )
                    .await?;
            }
            Outcome::ImageUsage {
                usage,
                confirmed_count,
                spec,
            } => {
                if matches!(
                    frozen.selected_price.rate,
                    RateCard::Tokens(_) | RateCard::ContextTokens { .. }
                ) && checked_raw_tokens(usage)? == 0
                {
                    let failure = FailedModelRequest {
                        protocol: usage.protocol.clone(),
                        model: usage.model.clone(),
                        requested_model: usage.requested_model.clone(),
                        processing_tier: usage.processing_tier.clone(),
                        reasoning_effort: usage.reasoning_effort.clone(),
                        runner_id: (!usage.runner_id.is_empty()).then(|| usage.runner_id.clone()),
                        description: "上游未提供可核实的图片 Token 用量，费用按 0 记录".into(),
                    };
                    state
                        .record_member_money_anomaly(
                            &reservation.identity_id,
                            &reservation.request_id,
                            &failure,
                        )
                        .await?;
                    return Ok(Completion::Recorded);
                }
                let verified = if matches!(frozen.selected_price.rate, RateCard::Images { .. }) {
                    VerifiedUsage::Images {
                        confirmed_count: *confirmed_count,
                        spec: spec.clone(),
                    }
                } else {
                    VerifiedUsage::Tokens(money_token_usage(usage)?)
                };
                state
                    .settle_member_money(
                        &reservation.identity_id,
                        &reservation.request_id,
                        frozen,
                        verified,
                        &usage.model,
                    )
                    .await?;
            }
            Outcome::Failure(failure) | Outcome::FailureWithUnknown(failure) => {
                if *failure == invalid_usage_failure() {
                    state
                        .record_member_money_anomaly(
                            &reservation.identity_id,
                            &reservation.request_id,
                            failure,
                        )
                        .await?;
                } else {
                    state
                        .record_member_money_failure(
                            &reservation.identity_id,
                            &reservation.request_id,
                            failure,
                            upstream_unknown,
                        )
                        .await?;
                }
            }
        }
        return Ok(Completion::Recorded);
    }
    let snapshot = state
        .verified_quota_snapshot(&reservation.identity_id)
        .await?;
    if let Some(completion) = confirmed(&snapshot, reservation, outcome)? {
        return Ok(completion);
    }
    match outcome {
        Outcome::Usage(usage) => state
            .settle_model_quota(reservation, usage)
            .await
            .map(|_| Completion::Recorded),
        Outcome::ImageUsage { .. } => Err(ControlError::DataIntegrityInvalid),
        Outcome::Failure(failure) | Outcome::FailureWithUnknown(failure) => state
            .fail_model_request(reservation, failure)
            .await
            .map(|_| Completion::Recorded),
    }
}

fn money_token_usage(usage: &ModelUsage) -> Result<TokenUsage, ControlError> {
    Ok(TokenUsage {
        input: u64::try_from(usage.uncached_input)
            .map_err(|_| ControlError::UpstreamUsageInvalid)?,
        cached_read: u64::try_from(usage.cached_input)
            .map_err(|_| ControlError::UpstreamUsageInvalid)?,
        cached_write: u64::try_from(usage.cache_write)
            .map_err(|_| ControlError::UpstreamUsageInvalid)?,
        output: u64::try_from(usage.output_tokens)
            .map_err(|_| ControlError::UpstreamUsageInvalid)?,
        image_input: 0,
        image_output: 0,
    })
}

fn confirmed(
    snapshot: &QuotaStateSnapshot,
    reservation: &QuotaReservation,
    outcome: &Outcome,
) -> Result<Option<Completion>, ControlError> {
    if snapshot
        .active_reservations
        .iter()
        .any(|entry| entry.id == reservation.id || entry.request_id == reservation.request_id)
    {
        return Ok(None);
    }
    let mut matching = snapshot.ledger_entries.iter().filter(|entry| {
        entry.reference_id == reservation.request_id
            && matches!(entry.kind.as_str(), "usage" | "usage_failed")
    });
    let Some(entry) = matching.next() else {
        return Ok(None);
    };
    if matching.next().is_some() || entry.identity_id != reservation.identity_id {
        return Err(ControlError::DataIntegrityInvalid);
    }
    let expired = expired_failure();
    if entry.kind == "usage_failed"
        && entry.protocol == expired.protocol
        && entry.model == expired.model
        && entry.description == expired.description
        && entry.client_request_id.is_none()
        && entry.requested_model.is_none()
        && entry.processing_tier.is_none()
        && entry.reasoning_effort.is_none()
        && entry.runner_id.is_none()
    {
        // This signed, atomic failure ledger is the TTL reaper's explicit
        // unbilled/unknown terminal outcome, not evidence of successful usage.
        return Ok(Some(Completion::Expired));
    }
    if entry.client_request_id != reservation.client_request_id {
        return Err(ControlError::DataIntegrityInvalid);
    }
    let same = match outcome {
        Outcome::Usage(usage) => {
            let raw = checked_raw_tokens(usage)?;
            entry.kind == "usage"
                && entry.protocol == usage.protocol
                && entry.model == usage.model
                && entry.requested_model == usage.requested_model
                && entry.processing_tier == usage.processing_tier
                && entry.reasoning_effort == usage.reasoning_effort
                && entry.runner_id.as_deref() == Some(usage.runner_id.as_str())
                && entry.uncached_input == usage.uncached_input
                && entry.cached_input == usage.cached_input
                && entry.cache_write == usage.cache_write
                && entry.output_tokens == usage.output_tokens
                && entry.multiplier_micros == usage.multiplier_micros
                && entry.raw_tokens == raw
                && entry.billed_tokens == billed_tokens(raw, usage.multiplier_micros)?
        }
        Outcome::ImageUsage { .. } => false,
        Outcome::Failure(failure) | Outcome::FailureWithUnknown(failure) => {
            entry.kind == "usage_failed"
                && entry.protocol == failure.protocol
                && entry.model == failure.model
                && entry.requested_model == failure.requested_model
                && entry.processing_tier == failure.processing_tier
                && entry.reasoning_effort == failure.reasoning_effort
                && entry.runner_id == failure.runner_id
                && entry.description == failure.description
        }
    };
    if same {
        Ok(Some(Completion::Recorded))
    } else {
        Err(ControlError::DataIntegrityInvalid)
    }
}

/// Idempotency is based on a purpose-specific immutable request identity. A
/// lost acknowledgement or another Control cannot create a second marker.
async fn record_unknown_audit(
    state: &ControlState,
    reservation: &QuotaReservation,
) -> Result<(), ControlError> {
    use crate::{AUDIT_APPEND_RETRIES, AuditAppendOutcome, audit_event_integrity_input};
    use sha2::{Digest as _, Sha256};
    const ACTION: &str = "gateway.request.unbilled_upstream_unknown";
    let identity = serde_json::to_vec(&(
        "aster.gateway-unknown-audit.v1",
        &reservation.identity_id,
        &reservation.id,
        &reservation.request_id,
    ))
    .map_err(|_| ControlError::DataIntegrityInvalid)?;
    let event_id = format!(
        "audit_gw_unknown_{}",
        crate::hex_encode(&Sha256::digest(identity))
    );
    for _ in 0..AUDIT_APPEND_RETRIES {
        let events = state.verified_audit_events().await?;
        if let Some(event) = events.iter().find(|event| event.id == event_id) {
            if event.actor_identity_id.is_some()
                || event.actor_role != "system"
                || event.action != ACTION
                || event.target_type != "gateway_request"
                || event.target_id.as_deref() != Some(reservation.request_id.as_str())
                || event.outcome != "failed"
            {
                return Err(ControlError::DataIntegrityInvalid);
            }
            return Ok(());
        }
        let (sequence, previous_hmac, mut event) = state
            .prepare_audit_event(
                None,
                ACTION,
                "gateway_request",
                Some(&reservation.request_id),
                "failed",
            )
            .await?;
        event.id = event_id.clone();
        event.integrity_hmac = state
            .auth_core()?
            .audit_event_integrity_hmac(audit_event_integrity_input(&event))
            .map_err(ControlError::Auth)?;
        if state
            .credential_storage()?
            .append_audit_event(sequence, &previous_hmac, &event)
            .await?
            == AuditAppendOutcome::Applied
        {
            return Ok(());
        }
    }
    Err(ControlError::DataIntegrityInvalid)
}
