use super::*;
use aster_runner_protocol::VerifiedTaskTicket;
use tokio::time::Instant;

#[cfg(test)]
mod tests;

/// A verified ticket bounded by both its signed wall-clock expiry and elapsed
/// monotonic time. Moving the system clock backwards cannot extend its lifetime.
pub(super) struct TaskExecutionPermit {
    ticket: VerifiedTaskTicket,
    deadline: Instant,
}

impl TaskExecutionPermit {
    pub(super) fn new(
        ticket: VerifiedTaskTicket,
        received_at: time::OffsetDateTime,
        received_instant: Instant,
    ) -> Result<Self, RunnerFailure> {
        ticket
            .validate_execution_time(received_at)
            .map_err(|_| RunnerFailure::new("task_ticket_expired"))?;
        let expiry = time::OffsetDateTime::from_unix_timestamp(ticket.claims().expires_at)
            .map_err(|_| RunnerFailure::new("task_ticket_expired"))?;
        let remaining = StdDuration::try_from(expiry - received_at)
            .map_err(|_| RunnerFailure::new("task_ticket_expired"))?;
        let deadline = received_instant
            .checked_add(remaining)
            .ok_or_else(|| RunnerFailure::new("task_ticket_expired"))?;
        Ok(Self { ticket, deadline })
    }

    pub(super) fn task_id(&self) -> &str {
        &self.ticket.claims().task_id
    }

    fn valid(&self) -> bool {
        Instant::now() < self.deadline
            && self
                .ticket
                .validate_execution_time(time::OffsetDateTime::now_utc())
                .is_ok()
    }

    fn expired_frame(&self) -> RunnerToControl {
        RunnerToControl::TaskFailed(TaskFailure {
            task_id: self.task_id().to_owned(),
            category: "task_ticket_expired".to_owned(),
            retryable_before_upstream: true,
        })
    }

    async fn send_before_upstream(
        &self,
        outbound: &mpsc::Sender<RunnerToControl>,
        event: RunnerToControl,
    ) -> Result<(), ()> {
        // Reserve capacity first, then recheck time and send synchronously. No
        // unbounded event-queue wait remains after the last permission check.
        let slot = match tokio::time::timeout_at(self.deadline, outbound.reserve()).await {
            Ok(Ok(slot)) => slot,
            Ok(Err(_)) => return Err(()),
            Err(_) => {
                // Never keep the Runner's execution slot occupied indefinitely
                // just to report expiration through a blocked Control channel.
                let _ = outbound.try_send(self.expired_frame());
                return Err(());
            }
        };
        if !self.valid() {
            slot.send(self.expired_frame());
            return Err(());
        }
        slot.send(event);
        Ok(())
    }

    pub(super) async fn accepted(
        &self,
        outbound: &mpsc::Sender<RunnerToControl>,
    ) -> Result<(), ()> {
        self.send_before_upstream(
            outbound,
            RunnerToControl::TaskAccepted(TaskLifecycle {
                task_id: self.task_id().to_owned(),
            }),
        )
        .await
    }

    pub(super) async fn start_upstream(
        &self,
        outbound: &mpsc::Sender<RunnerToControl>,
    ) -> Result<(), ()> {
        self.send_before_upstream(
            outbound,
            RunnerToControl::UpstreamStarted(TaskLifecycle {
                task_id: self.task_id().to_owned(),
            }),
        )
        .await
    }
}
