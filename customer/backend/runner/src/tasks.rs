//! Tasks belong to one authenticated Control session. Dropping the session aborts
//! its local HTTP futures; no detached worker survives into another connection.
use crate::{InflightGuard, Metrics, send_failure};
use aster_runner_protocol::RunnerToControl;
use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot},
    task::JoinSet,
    time::Instant,
};

pub(super) struct TaskSupervisor {
    capacity: Arc<Semaphore>,
    cancellations: Arc<Mutex<HashMap<String, Option<oneshot::Sender<()>>>>>,
    workers: JoinSet<()>,
}

pub(super) struct TaskExecution {
    pub id: String,
    pub deadline: Instant,
    pub outbound: mpsc::Sender<RunnerToControl>,
    pub metrics: Arc<Metrics>,
    pub permit: OwnedSemaphorePermit,
}

struct TaskGuard {
    cancellations: Arc<Mutex<HashMap<String, Option<oneshot::Sender<()>>>>>,
    id: String,
}
impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.cancellations
            .lock()
            .expect("task registry poisoned")
            .remove(&self.id);
    }
}

impl TaskSupervisor {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: Arc::new(Semaphore::new(capacity)),
            cancellations: Arc::default(),
            workers: JoinSet::new(),
        }
    }
    pub fn try_permit(&self) -> Option<OwnedSemaphorePermit> {
        self.capacity.clone().try_acquire_owned().ok()
    }
    pub fn is_empty(&self) -> bool {
        self.workers.is_empty()
    }
    pub async fn next_finished(&mut self) {
        let _ = self.workers.join_next().await;
    }
    pub fn cancel(&self, task_id: &str) {
        if let Some(sender) = self
            .cancellations
            .lock()
            .expect("task registry poisoned")
            .get_mut(task_id)
            .and_then(Option::take)
        {
            let _ = sender.send(());
        }
    }
    pub fn start(
        &mut self,
        task: TaskExecution,
        future: impl Future<Output = Result<(), ()>> + Send + 'static,
    ) -> bool {
        let (cancel, cancelled) = oneshot::channel();
        {
            let mut entries = self.cancellations.lock().expect("task registry poisoned");
            if entries.contains_key(&task.id) {
                return false;
            }
            entries.insert(task.id.clone(), Some(cancel));
        }
        task.metrics
            .inflight
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        task.metrics
            .recent_requests
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // Construct guards before spawning: abort before the first poll must also
        // release the metrics, registry entry and capacity permit.
        let guard = TaskGuard {
            cancellations: self.cancellations.clone(),
            id: task.id.clone(),
        };
        let inflight = InflightGuard(task.metrics.clone());
        self.workers.spawn(async move {
            let _guard = guard;
            let _inflight = inflight;
            let _permit = task.permit;
            let mut execution = Box::pin(future);
            let (failure, failed) = tokio::select! {
                biased;
                _ = cancelled => (Some("task_cancelled"), true),
                _ = tokio::time::sleep_until(task.deadline) => (Some("task_deadline_exceeded"), true),
                result = &mut execution => (None, result.is_err()),
            };
            // Close the local upstream future before acknowledging cancellation.
            // This still cannot prove whether the provider stopped billing.
            drop(execution);
            if let Some(category) = failure {
                let _ = tokio::time::timeout(Duration::from_secs(5), send_failure(&task.outbound, &task.id, category, false)).await;
            }
            if failed { task.metrics.recent_errors.fetch_add(1, std::sync::atomic::Ordering::Relaxed); }

        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[tokio::test]
    async fn cancellation_is_session_scoped_and_releases_capacity() {
        let mut session = TaskSupervisor::new(1);
        let other = TaskSupervisor::new(1);
        let metrics = Arc::new(Metrics::default());
        let (outbound, mut events) = mpsc::channel(4);
        assert!(session.start(
            TaskExecution {
                id: "task-a".into(),
                deadline: Instant::now() + Duration::from_secs(10),
                outbound,
                metrics: metrics.clone(),
                permit: session.try_permit().unwrap()
            },
            std::future::pending()
        ));
        assert!(session.try_permit().is_none());
        other.cancel("task-a");
        assert!(events.try_recv().is_err());
        session.cancel("task-a");
        session.next_finished().await;
        let RunnerToControl::TaskFailed(frame) = events.recv().await.unwrap() else {
            panic!("expected cancellation")
        };
        assert_eq!(frame.category, "task_cancelled");
        assert!(!frame.retryable_before_upstream);
        assert_eq!(metrics.inflight.load(Ordering::Relaxed), 0);
        assert!(session.try_permit().is_some());
        session.cancel("task-a");
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn deadline_covers_blocked_execution_and_emits_one_terminal_failure() {
        let mut session = TaskSupervisor::new(1);
        let metrics = Arc::new(Metrics::default());
        let (outbound, mut events) = mpsc::channel(4);
        session.start(
            TaskExecution {
                id: "task-a".into(),
                deadline: Instant::now(),
                outbound,
                metrics: metrics.clone(),
                permit: session.try_permit().unwrap(),
            },
            std::future::pending(),
        );
        session.next_finished().await;
        let RunnerToControl::TaskFailed(frame) = events.recv().await.unwrap() else {
            panic!("expected deadline")
        };
        assert_eq!(frame.category, "task_deadline_exceeded");
        assert!(!frame.retryable_before_upstream);
        assert!(events.try_recv().is_err());
        assert_eq!(metrics.inflight.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn session_drop_aborts_workers_even_before_first_poll() {
        let mut session = TaskSupervisor::new(1);
        let metrics = Arc::new(Metrics::default());
        let (outbound, mut events) = mpsc::channel(4);
        session.start(
            TaskExecution {
                id: "task-a".into(),
                deadline: Instant::now() + Duration::from_secs(10),
                outbound,
                metrics: metrics.clone(),
                permit: session.try_permit().unwrap(),
            },
            std::future::pending(),
        );
        drop(session);
        tokio::time::timeout(Duration::from_secs(2), async {
            while metrics.inflight.load(Ordering::Relaxed) != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(events.recv().await.is_none());
    }
    #[tokio::test]
    async fn local_execution_is_dropped_before_cancellation_acknowledgement() {
        struct Dropped(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let guard = Dropped(dropped.clone());
        let mut session = TaskSupervisor::new(1);
        let metrics = Arc::new(Metrics::default());
        let (outbound, mut events) = mpsc::channel(4);
        session.start(
            TaskExecution {
                id: "task-a".into(),
                deadline: Instant::now() + Duration::from_secs(10),
                outbound,
                metrics,
                permit: session.try_permit().unwrap(),
            },
            async move {
                let _guard = guard;
                std::future::pending().await
            },
        );
        session.cancel("task-a");
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(event, RunnerToControl::TaskFailed(_)));
        assert!(dropped.load(Ordering::Acquire));
        session.next_finished().await;
    }

    #[tokio::test]
    async fn cancelling_an_already_completed_task_does_not_emit_a_second_result() {
        let mut session = TaskSupervisor::new(1);
        let metrics = Arc::new(Metrics::default());
        let (outbound, mut events) = mpsc::channel(4);
        let response = outbound.clone();
        session.start(
            TaskExecution {
                id: "task-a".into(),
                deadline: Instant::now() + Duration::from_secs(10),
                outbound,
                metrics,
                permit: session.try_permit().unwrap(),
            },
            async move {
                response
                    .send(RunnerToControl::TaskFinished(
                        aster_runner_protocol::TaskResult {
                            task_id: "task-a".into(),
                            status: 200,
                            usage_json: None,
                        },
                    ))
                    .await
                    .unwrap();
                Ok(())
            },
        );
        session.next_finished().await;
        session.cancel("task-a");
        assert!(matches!(
            events.recv().await.unwrap(),
            RunnerToControl::TaskFinished(_)
        ));
        assert!(events.try_recv().is_err());
    }
}
