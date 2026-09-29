//! Admission and ownership of work across HTTP, response bodies and detached tasks.
//! A request leaves the registry only after its last owner has finished.
use std::{
    collections::BTreeMap,
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use axum::{
    body::Body,
    extract::{Request, State},
    http::HeaderValue,
    middleware::Next,
    response::{IntoResponse, Response},
};
use tokio::{sync::Notify, task::JoinHandle, time::Instant};

mod deadline_body;
#[cfg(test)]
mod deadline_tests;

const REQUEST_BUDGET: Duration = Duration::from_secs(720);
const SETTLEMENT_RESERVE: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct RequestLifecycle(Arc<Inner>);

struct Inner {
    state: Mutex<Registry>,
    changed: Notify,
}

struct Registry {
    accepting: bool,
    stopping: bool,
    revision: u64,
    next_id: u64,
    requests: BTreeMap<u64, Instant>,
}

pub use aster_upgrade_core::runtime::LifecycleSnapshot as DrainSnapshot;

struct RequestLease(Arc<LeaseInner>);

impl Clone for RequestLease {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

struct LeaseInner {
    execution_deadline: Instant,
    response_deadline: Instant,
    deadline_elapsed: AtomicBool,
    disconnected: AtomicBool,
    cancelled: Notify,
    lifecycle: RequestLifecycle,
    id: u64,
}

impl Drop for LeaseInner {
    fn drop(&mut self) {
        self.lifecycle
            .0
            .state
            .lock()
            .expect("request registry poisoned")
            .requests
            .remove(&self.id);
        self.lifecycle.0.changed.notify_waiters();
    }
}

tokio::task_local! {
    static CURRENT: Option<RequestLease>;
}

impl Default for RequestLifecycle {
    fn default() -> Self {
        Self(Arc::new(Inner {
            state: Mutex::new(Registry {
                accepting: true,
                stopping: false,
                revision: 0,
                next_id: 0,
                requests: BTreeMap::new(),
            }),
            changed: Notify::new(),
        }))
    }
}

impl RequestLifecycle {
    pub(crate) fn request_budget_ms(&self) -> u64 {
        u64::try_from(REQUEST_BUDGET.as_millis()).expect("request budget fits milliseconds")
    }

    fn admit(&self) -> Option<RequestLease> {
        let mut state = self.0.state.lock().expect("request registry poisoned");
        if !state.accepting {
            return None;
        }
        let id = state.next_id;
        state.next_id = state.next_id.checked_add(1)?;
        let admitted_at = Instant::now();
        state.requests.insert(id, admitted_at);
        Some(RequestLease(Arc::new(LeaseInner {
            lifecycle: self.clone(),
            execution_deadline: admitted_at + REQUEST_BUDGET - SETTLEMENT_RESERVE,
            response_deadline: admitted_at + REQUEST_BUDGET,
            deadline_elapsed: AtomicBool::new(false),
            disconnected: AtomicBool::new(false),
            cancelled: Notify::new(),
            id,
        })))
    }

    /// Background recovery uses the same atomic admission and drain ownership
    /// as HTTP work. Detached mutations inherit this lease through `spawn`.
    pub(crate) async fn run_background<F: Future>(&self, work: F) -> Option<F::Output> {
        let lease = self.admit()?;
        Some(CURRENT.scope(Some(lease), work).await)
    }

    /// Admission and the drain transition share one lock, so no accepted request
    /// can appear after a snapshot has declared this draining instance empty.
    pub fn begin_drain(&self) {
        let mut state = self.0.state.lock().expect("request registry poisoned");
        if state.accepting {
            state.accepting = false;
            state.revision = state.revision.saturating_add(1);
        }
        self.0.changed.notify_waiters();
    }

    /// Once process shutdown starts, no executor command can reopen admission.
    pub fn begin_shutdown(&self) {
        let mut state = self.0.state.lock().expect("request registry poisoned");
        if !state.stopping {
            state.stopping = true;
            state.accepting = false;
            state.revision = state.revision.saturating_add(1);
        }
        self.0.changed.notify_waiters();
    }

    /// Retire only the closed, empty revision the executor actually drained.
    /// Checking and entering the irreversible stopping state share admission's
    /// lock; a concurrent resume cannot slip between the two operations.
    pub fn retire_drained(&self, expected_revision: u64) -> bool {
        let mut state = self.0.state.lock().expect("request registry poisoned");
        if state.accepting
            || state.stopping
            || !state.requests.is_empty()
            || state.revision != expected_revision
        {
            return false;
        }
        let Some(revision) = state.revision.checked_add(1) else {
            return false;
        };
        state.stopping = true;
        state.revision = revision;
        self.0.changed.notify_waiters();
        true
    }

    pub async fn wait_shutdown_started(&self) {
        loop {
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.snapshot().stopping {
                return;
            }
            changed.await;
        }
    }

    /// Executor commands use a compare-and-set revision. A delayed command must
    /// not undo a newer drain/resume decision. Resume requires the caller to have
    /// separately validated the destination; it is not a readiness assertion.
    pub fn set_admission(&self, expected_revision: u64, accepting: bool) -> bool {
        let mut state = self.0.state.lock().expect("request registry poisoned");
        if state.stopping || state.revision != expected_revision {
            return false;
        }
        let Some(revision) = state.revision.checked_add(1) else {
            return false;
        };
        state.accepting = accepting;
        state.revision = revision;
        self.0.changed.notify_waiters();
        true
    }

    pub fn snapshot(&self) -> DrainSnapshot {
        let state = self.0.state.lock().expect("request registry poisoned");
        DrainSnapshot {
            accepting: state.accepting,
            stopping: state.stopping,
            revision: state.revision,
            in_flight: state.requests.len(),
            oldest_request_age_ms: state
                .requests
                .values()
                .map(|start| u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX))
                .max()
                .unwrap_or(0),
        }
    }

    pub async fn wait_drained(&self) {
        loop {
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let snapshot = self.snapshot();
            if !snapshot.accepting && snapshot.in_flight == 0 {
                return;
            }
            changed.await;
        }
    }
}

/// The first execution budget uses the agreed 720-second baseline with 30
/// seconds reserved for settlement. Every attempt consumes this same deadline.
/// HTTP input, handler waiting and response ownership share the admission clock.
/// Outstanding durable writes still retain their own lease after a response
/// deadline; they must never be mistaken for completed settlement.
pub(crate) fn execution_budget(maximum: Duration) -> Result<Duration, ()> {
    let lease = CURRENT.try_with(Clone::clone).ok().flatten();
    let Some(lease) = lease else {
        return Ok(maximum);
    };
    if lease.0.disconnected.load(Ordering::Acquire) {
        return Err(());
    }
    let remaining = lease
        .0
        .execution_deadline
        .saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(());
    }
    Ok(remaining.min(maximum))
}

async fn peer_disconnected(lease: Option<RequestLease>) {
    let Some(lease) = lease else {
        std::future::pending::<()>().await;
        return;
    };
    loop {
        let notified = lease.0.cancelled.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if lease.0.disconnected.load(Ordering::Acquire) {
            return;
        }
        notified.await;
    }
}

pub(crate) async fn run_execution<F: Future>(
    maximum: Duration,
    future: F,
) -> Result<F::Output, ()> {
    let budget = execution_budget(maximum)?;
    let lease = CURRENT.try_with(Clone::clone).ok().flatten();
    tokio::select! {
        biased;
        _ = peer_disconnected(lease) => Err(()),
        result = tokio::time::timeout(budget, future) => result.map_err(|_| ()),
    }
}

impl RequestLease {
    fn deadline_elapsed(&self) {
        self.0.deadline_elapsed.store(true, Ordering::Release);
        self.0.disconnected.store(true, Ordering::Release);
        self.0.cancelled.notify_waiters();
    }
}

struct PeerGuard(RequestLease);
impl Drop for PeerGuard {
    fn drop(&mut self) {
        self.0.0.disconnected.store(true, Ordering::Release);
        self.0.0.cancelled.notify_waiters();
    }
}

/// Every detached business task inherits the admitted request. Stream settlement
/// must remain owned even after the client drops its response body.
pub(crate) fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let lease = CURRENT.try_with(Clone::clone).ok().flatten();
    tokio::spawn(CURRENT.scope(lease, future))
}

/// Dropping a spawn_blocking JoinHandle does not stop its transaction. Retain the
/// request until the closure actually returns, including cancellation and panic.
pub(crate) fn spawn_blocking<F, R>(function: F) -> JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let lease = CURRENT.try_with(Clone::clone).ok().flatten();
    tokio::task::spawn_blocking(move || CURRENT.sync_scope(lease, function))
}

pub(crate) async fn track(
    State(lifecycle): State<RequestLifecycle>,
    mut request: Request,
    next: Next,
) -> Response {
    // Transport control channels stay alive while their already-admitted tasks
    // drain. Health is liveness only; it must never be used as business readiness.
    if matches!(request.uri().path(), "/healthz" | "/api/runner/channel")
        || request.extensions().get::<RequestLease>().is_some()
    {
        return next.run(request).await;
    }
    let Some(lease) = lifecycle.admit() else {
        let mut response = if request.uri().path().starts_with("/v1/") {
            crate::GatewayError::new(
                crate::ControlError::InstanceDraining,
                request.uri().path() == "/v1/messages"
                    || crate::anthropic_protocol(request.headers()),
            )
            .into_response()
        } else {
            crate::ControlError::InstanceDraining.into_response()
        };
        response.headers_mut().insert(
            axum::http::header::RETRY_AFTER,
            HeaderValue::from_static("1"),
        );
        return response;
    };
    let gateway = request.uri().path().starts_with("/v1/");
    let anthropic =
        request.uri().path() == "/v1/messages" || crate::anthropic_protocol(request.headers());
    let peer = PeerGuard(lease.clone());
    request.extensions_mut().insert(lease.clone());
    let input = std::mem::replace(request.body_mut(), Body::empty());
    *request.body_mut() =
        deadline_body::wrap(input, lease.clone(), lease.0.execution_deadline, None);
    // Dropping the JoinHandle at the HTTP deadline leaves the owned handler and
    // already-started mutations alive to reconcile reservations and settlement.
    let execution = CURRENT.sync_scope(Some(lease.clone()), || spawn(next.run(request)));
    let response = tokio::select! {
        biased;
        _ = tokio::time::sleep_until(lease.0.response_deadline) => {
            lease.deadline_elapsed();
            return deadline_response(gateway, anthropic);
        }
        response = execution => response.expect("request handler task failed"),
    };
    if lease.0.deadline_elapsed.load(Ordering::Acquire)
        || Instant::now() >= lease.0.response_deadline
    {
        lease.deadline_elapsed();
        return deadline_response(gateway, anthropic);
    }
    let (parts, body) = response.into_parts();
    let deadline = lease.0.response_deadline;
    Response::from_parts(
        parts,
        deadline_body::wrap(body, lease, deadline, Some(peer)),
    )
}

fn deadline_response(gateway: bool, anthropic: bool) -> Response {
    let error = crate::ControlError::RequestDeadlineExceeded;
    if gateway {
        crate::GatewayError::new(error, anthropic).into_response()
    } else {
        error.into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, http::StatusCode, routing::get};
    use tokio::sync::oneshot;
    use tower::ServiceExt as _;

    fn app(lifecycle: RequestLifecycle) -> Router {
        Router::new()
            .route("/work", get(|| async { "done" }))
            .layer(axum::middleware::from_fn_with_state(lifecycle, track))
    }

    #[tokio::test]
    async fn response_body_is_owned_until_consumed_and_drain_rejects_new_work() {
        let lifecycle = RequestLifecycle::default();
        let app = app(lifecycle.clone());
        let response = app
            .clone()
            .oneshot(Request::builder().uri("/work").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(lifecycle.snapshot().in_flight, 1);
        lifecycle.begin_drain();
        let rejected = app
            .oneshot(Request::builder().uri("/work").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(rejected.headers()[axum::http::header::RETRY_AFTER], "1");
        assert_eq!(lifecycle.snapshot().in_flight, 1);
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 100)
                .await
                .unwrap(),
            "done"
        );
        lifecycle.wait_drained().await;
        assert_eq!(lifecycle.snapshot().in_flight, 0);
        assert!(lifecycle.set_admission(lifecycle.snapshot().revision, true));
        assert!(lifecycle.admit().is_some());
    }

    #[tokio::test]
    async fn disconnected_stream_stays_owned_until_detached_settlement_finishes() {
        let lifecycle = RequestLifecycle::default();
        let (release, finish) = oneshot::channel();
        let finish = Arc::new(Mutex::new(Some(finish)));
        let app = Router::new()
            .route(
                "/stream",
                get(move || {
                    let finish = finish.lock().unwrap().take().unwrap();
                    async move {
                        spawn(async move {
                            let _ = finish.await;
                        });
                        Body::from_stream(futures_util::stream::pending::<
                            Result<axum::body::Bytes, std::convert::Infallible>,
                        >())
                    }
                }),
            )
            .layer(axum::middleware::from_fn_with_state(
                lifecycle.clone(),
                track,
            ));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        lifecycle.begin_drain();
        drop(response); // The peer disconnects while the worker still settles.
        assert_eq!(lifecycle.snapshot().in_flight, 1);
        release.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), lifecycle.wait_drained())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn disconnected_non_streaming_request_keeps_handler_until_settlement() {
        let lifecycle = RequestLifecycle::default();
        let (started, ready) = oneshot::channel();
        let (release, finish) = oneshot::channel();
        let signals = Arc::new(Mutex::new(Some((started, finish))));
        let app = Router::new()
            .route(
                "/buffered",
                get(move || {
                    let (started, finish) = signals.lock().unwrap().take().unwrap();
                    async move {
                        started.send(()).unwrap();
                        finish.await.unwrap();
                        "settled"
                    }
                }),
            )
            .layer(axum::middleware::from_fn_with_state(
                lifecycle.clone(),
                track,
            ));
        let request = tokio::spawn(
            app.oneshot(
                Request::builder()
                    .uri("/buffered")
                    .body(Body::empty())
                    .unwrap(),
            ),
        );
        ready.await.unwrap();
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());
        lifecycle.begin_drain();
        assert_eq!(lifecycle.snapshot().in_flight, 1);
        release.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), lifecycle.wait_drained())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cancelled_handler_does_not_hide_a_running_blocking_transaction() {
        let lifecycle = RequestLifecycle::default();
        let lease = lifecycle.admit().unwrap();
        let (started, ready) = oneshot::channel();
        let (release, finish) = std::sync::mpsc::channel();
        let worker = CURRENT.sync_scope(Some(lease.clone()), || {
            spawn_blocking(move || {
                let _ = started.send(());
                finish.recv().unwrap();
            })
        });
        ready.await.unwrap();
        worker.abort(); // An already-running blocking task cannot be aborted.
        drop(lease);
        lifecycle.begin_drain();
        assert_eq!(lifecycle.snapshot().in_flight, 1);
        release.send(()).unwrap();
        worker.await.unwrap();
        lifecycle.wait_drained().await;
    }

    #[test]
    fn drain_and_admission_are_serialized() {
        let lifecycle = RequestLifecycle::default();
        let workers: Vec<_> = (0..16)
            .map(|_| {
                let lifecycle = lifecycle.clone();
                std::thread::spawn(move || lifecycle.admit())
            })
            .collect();
        lifecycle.begin_drain();
        let leases: Vec<_> = workers
            .into_iter()
            .filter_map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(lifecycle.snapshot().in_flight, leases.len());
        assert!(lifecycle.admit().is_none());
        drop(leases);
        assert_eq!(lifecycle.snapshot().in_flight, 0);
    }

    #[tokio::test]
    async fn retirement_requires_drained_revision_and_wakes_late_shutdown_observer() {
        let lifecycle = RequestLifecycle::default();
        assert!(!lifecycle.retire_drained(0));
        let lease = lifecycle.admit().unwrap();
        lifecycle.begin_drain();
        assert!(!lifecycle.retire_drained(1));
        drop(lease);
        assert!(!lifecycle.retire_drained(0));
        assert!(lifecycle.retire_drained(1));
        assert!(!lifecycle.set_admission(2, true));
        assert!(!lifecycle.retire_drained(1));
        lifecycle.begin_shutdown();
        assert_eq!(lifecycle.snapshot().revision, 2);
        tokio::time::timeout(Duration::from_secs(1), lifecycle.wait_shutdown_started())
            .await
            .unwrap();
    }

    #[test]
    fn retirement_and_resume_cannot_both_succeed() {
        let lifecycle = RequestLifecycle::default();
        lifecycle.begin_drain();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let worker = {
            let lifecycle = lifecycle.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                lifecycle.set_admission(1, true)
            })
        };
        barrier.wait();
        let retired = lifecycle.retire_drained(1);
        let resumed = worker.join().unwrap();
        assert_ne!(retired, resumed);
        let state = lifecycle.snapshot();
        assert_eq!(state.stopping, retired);
        assert_eq!(state.accepting, resumed);
        assert_eq!(state.revision, 2);
    }
    #[tokio::test]
    async fn execution_attempts_share_remaining_budget_and_peer_cancel_wakes_waiters() {
        let lifecycle = RequestLifecycle::default();
        let mut lease = lifecycle.admit().unwrap();
        Arc::get_mut(&mut lease.0).unwrap().execution_deadline =
            Instant::now() + Duration::from_secs(1);
        CURRENT
            .scope(Some(lease.clone()), async {
                let first = execution_budget(Duration::from_secs(600)).unwrap();
                assert!(first <= Duration::from_secs(1));
                assert!(execution_budget(Duration::from_secs(600)).unwrap() <= first);
                let peer = PeerGuard(lease.clone());
                let waiter = spawn(run_execution(
                    Duration::from_secs(600),
                    std::future::pending::<()>(),
                ));
                drop(peer);
                assert!(
                    tokio::time::timeout(Duration::from_secs(1), waiter)
                        .await
                        .unwrap()
                        .unwrap()
                        .is_err()
                );
                assert!(execution_budget(Duration::from_secs(600)).is_err());
            })
            .await;
    }
}
