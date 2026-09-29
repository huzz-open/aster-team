//! Recover expired reservations without waiting for another client request.
//! Discovery is untrusted; the existing signed quota transaction is authoritative.
use crate::{ControlError, ControlState, finish_control_mutation};
use std::{sync::Arc, time::Duration};

const IDENTITY_PAGE: u32 = 16;
const RESERVATIONS_PER_IDENTITY: usize = 16;

impl ControlState {
    /// Run one bounded page at a time. Closed candidates and draining instances
    /// cannot admit a batch; a started transaction keeps its lifecycle lease.
    pub async fn run_quota_recovery(&self) {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut after = String::new();
        let mut outbox_after = String::new();
        loop {
            tokio::select! {
                biased;
                _ = self.request_lifecycle().wait_shutdown_started() => return,
                _ = interval.tick() => {},
            }
            match self.recover_settlement_outbox_page(&outbox_after).await {
                Ok(next) => outbox_after = next.unwrap_or_default(),
                Err(error) => tracing::warn!(
                    error_number = error.descriptor().number,
                    "settlement outbox discovery failed; retaining its cursor"
                ),
            }
            match self.recover_expired_model_quota_page(&after).await {
                Ok(next) => after = next.unwrap_or_default(),
                Err(error) => {
                    tracing::warn!(
                        error_number = error.descriptor().number,
                        "quota recovery discovery failed; retrying the same page"
                    );
                }
            }
        }
    }

    pub(crate) async fn recover_settlement_outbox_page(
        &self,
        after: &str,
    ) -> Result<Option<String>, ControlError> {
        let Some(store) = self.outbox()? else {
            return Ok(None);
        };
        let state = self.clone();
        let after = after.to_owned();
        self.request_lifecycle().run_background(async move {
            finish_control_mutation(Arc::clone(&state.mutation_tasks), async move {
                let scan = Arc::clone(&store);
                let keys = crate::lifecycle::spawn_blocking(move || scan.page(&after, IDENTITY_PAGE as usize)).await
                    .map_err(|_| ControlError::DataIntegrityInvalid)??;
                let next = (keys.len() == IDENTITY_PAGE as usize).then(|| keys.last().cloned()).flatten();
                for key in keys {
                    let store = Arc::clone(&store);
                    let recovered = async {
                        let Some(owner) = crate::lifecycle::spawn_blocking(move || store.acquire_key(&key)).await
                            .map_err(|_| ControlError::DataIntegrityInvalid)?? else { return Ok(()); };
                        crate::gateway::durable_settlement::recover_owned(&state, &owner).await?;
                        Ok::<_, ControlError>(())
                    }.await;
                    if let Err(error) = recovered {
                        tracing::warn!(error_number = error.descriptor().number,
                            "settlement outbox entry remains unresolved; retained for another scan");
                    }
                }
                Ok(next)
            }).await
        }).await.unwrap_or(Ok(None))
    }

    pub(crate) async fn recover_expired_model_quota_page(
        &self,
        after: &str,
    ) -> Result<Option<String>, ControlError> {
        let state = self.clone();
        let after = after.to_owned();
        self.request_lifecycle().run_background(async move {
            let identities = state.credential_storage()?.active_quota_identity_page(&after, IDENTITY_PAGE).await?;
            let next = (identities.len() == IDENTITY_PAGE as usize).then(|| identities.last().cloned()).flatten();
            for identity in identities {
                if let Err(error) = state.release_expired_quota_reservations_limited(&identity, RESERVATIONS_PER_IDENTITY).await {
                    tracing::warn!(error_number = error.descriptor().number,
                        "quota recovery remains unresolved for an identity; retained for next scan");
                }
            }
            Ok(next)
        }).await.unwrap_or(Ok(None))
    }
}
