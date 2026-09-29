use super::*;
use axum::{
    Router,
    body::Bytes,
    http::StatusCode,
    routing::{get, post},
};
use http_body::Frame;
use http_body_util::{BodyExt as _, StreamBody};
use std::{
    convert::Infallible,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::sync::oneshot;
use tower::ServiceExt as _;

struct PendingBody(Arc<AtomicBool>);

#[tokio::test(start_paused = true)]
async fn failed_settlement_retries_after_http_timeout_and_blocks_retirement_until_confirmed() {
    let lifecycle = RequestLifecycle::default();
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let committed = Arc::new(AtomicBool::new(false));
    let release = Arc::new(Notify::new());
    let (ready, started) = oneshot::channel();
    let ready = Arc::new(Mutex::new(Some(ready)));
    let app = Router::new()
        .route(
            "/settle",
            get({
                let attempts = Arc::clone(&attempts);
                let committed = Arc::clone(&committed);
                let release = Arc::clone(&release);
                move || {
                    let attempts = Arc::clone(&attempts);
                    let committed = Arc::clone(&committed);
                    let release = Arc::clone(&release);
                    let ready = ready.lock().unwrap().take().unwrap();
                    async move {
                        ready.send(()).unwrap();
                        crate::finish_control_mutation(
                            Arc::new(tokio::sync::RwLock::new(false)),
                            async move {
                                crate::gateway::durable_settlement::retry(|| async {
                                    if attempts.fetch_add(1, Ordering::AcqRel) < 2 {
                                        return Err(crate::ControlError::Storage(
                                            "unavailable".into(),
                                        ));
                                    }
                                    release.notified().await;
                                    committed.store(true, Ordering::Release);
                                    Ok(())
                                })
                                .await
                            },
                        )
                        .await
                        .unwrap();
                        "settled"
                    }
                }
            }),
        )
        .layer(axum::middleware::from_fn_with_state(
            lifecycle.clone(),
            track,
        ));
    let call = tokio::spawn(
        app.oneshot(
            Request::builder()
                .uri("/settle")
                .body(Body::empty())
                .unwrap(),
        ),
    );
    started.await.unwrap();
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(2)).await;
    tokio::task::yield_now().await;
    assert_eq!(attempts.load(Ordering::Acquire), 3);
    tokio::time::advance(REQUEST_BUDGET).await;
    assert_eq!(
        call.await.unwrap().unwrap().status(),
        StatusCode::GATEWAY_TIMEOUT
    );
    lifecycle.begin_drain();
    assert_eq!(lifecycle.snapshot().in_flight, 1);
    assert!(!lifecycle.retire_drained(lifecycle.snapshot().revision));
    assert!(!committed.load(Ordering::Acquire));
    release.notify_one();
    lifecycle.wait_drained().await;
    assert!(committed.load(Ordering::Acquire));
    assert!(lifecycle.retire_drained(lifecycle.snapshot().revision));
}

impl futures_util::Stream for PendingBody {
    type Item = Result<Bytes, Infallible>;
    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Pending
    }
}
impl Drop for PendingBody {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[tokio::test(start_paused = true)]
async fn known_terminal_bodies_release_without_an_extra_eof_poll_and_stay_terminal() {
    use http_body::Body as _;

    for source in [Body::empty(), Body::from("complete")] {
        let lifecycle = RequestLifecycle::default();
        let lease = lifecycle.admit().unwrap();
        let deadline = lease.0.response_deadline;
        let mut body = deadline_body::wrap(source, lease, deadline, None);
        if !body.is_end_stream() {
            assert_eq!(lifecycle.snapshot().in_flight, 1);
            assert_eq!(
                body.frame().await.unwrap().unwrap().into_data().unwrap(),
                "complete"
            );
        }
        assert!(body.is_end_stream());
        assert_eq!(lifecycle.snapshot().in_flight, 0);
        tokio::time::advance(REQUEST_BUDGET).await;
        tokio::task::yield_now().await;
        assert!(body.is_end_stream());
        assert!(body.frame().await.is_none());
    }
}

#[tokio::test(start_paused = true)]
async fn unpolled_response_expires_and_releases_source_without_fabricating_eof() {
    let lifecycle = RequestLifecycle::default();
    let lease = lifecycle.admit().unwrap();
    let dropped = Arc::new(AtomicBool::new(false));
    let deadline = lease.0.response_deadline;
    let mut body = deadline_body::wrap(
        Body::from_stream(PendingBody(Arc::clone(&dropped))),
        lease,
        deadline,
        None,
    );
    assert_eq!(lifecycle.snapshot().in_flight, 1);
    tokio::time::advance(REQUEST_BUDGET).await;
    tokio::task::yield_now().await;
    assert!(dropped.load(Ordering::Acquire));
    assert_eq!(lifecycle.snapshot().in_flight, 0);
    assert!(body.frame().await.unwrap().is_err());
    assert!(body.frame().await.is_none());
}

#[tokio::test(start_paused = true)]
async fn input_deadline_returns_protocol_timeout_without_entering_handler() {
    let lifecycle = RequestLifecycle::default();
    let entered = Arc::new(AtomicBool::new(false));
    let handler_entered = Arc::clone(&entered);
    let app = Router::new()
        .route(
            "/v1/messages",
            post(move |_: Bytes| {
                handler_entered.store(true, Ordering::Release);
                async { "unexpected" }
            }),
        )
        .layer(axum::middleware::from_fn_with_state(
            lifecycle.clone(),
            track,
        ));
    let dropped = Arc::new(AtomicBool::new(false));
    let call = tokio::spawn(
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .body(Body::from_stream(PendingBody(Arc::clone(&dropped))))
                .unwrap(),
        ),
    );
    tokio::task::yield_now().await;
    assert_eq!(lifecycle.snapshot().in_flight, 1);
    tokio::time::advance(REQUEST_BUDGET - SETTLEMENT_RESERVE).await;
    let response = call.await.unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(response.headers()["X-Aster-Error-Number"], "91004");
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["type"], "error");
    assert_eq!(body["error"]["type"], "api_error");
    assert_eq!(body["error"]["number"], 91004);
    assert!(dropped.load(Ordering::Acquire));
    assert!(!entered.load(Ordering::Acquire));
    assert_eq!(lifecycle.snapshot().in_flight, 0);
}

#[tokio::test(start_paused = true)]
async fn header_wait_times_out_but_owned_mutation_is_not_aborted_or_removed() {
    let lifecycle = RequestLifecycle::default();
    let (started, ready) = oneshot::channel();
    let (release, finish) = oneshot::channel();
    let signals = Arc::new(Mutex::new(Some((started, finish))));
    let app = Router::new()
        .route(
            "/mutation",
            post(move || {
                let (started, finish) = signals.lock().unwrap().take().unwrap();
                async move {
                    started.send(()).unwrap();
                    finish.await.unwrap();
                    "committed"
                }
            }),
        )
        .layer(axum::middleware::from_fn_with_state(
            lifecycle.clone(),
            track,
        ));
    let call = tokio::spawn(
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri("/mutation")
                .body(Body::empty())
                .unwrap(),
        ),
    );
    ready.await.unwrap();
    tokio::time::advance(REQUEST_BUDGET).await;
    let response = call.await.unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    lifecycle.begin_drain();
    assert_eq!(lifecycle.snapshot().in_flight, 1);
    assert!(!lifecycle.retire_drained(lifecycle.snapshot().revision));
    release.send(()).unwrap();
    lifecycle.wait_drained().await;
    assert_eq!(lifecycle.snapshot().in_flight, 0);
}

#[tokio::test(start_paused = true)]
async fn response_deadline_cancels_execution_but_keeps_detached_settlement_owned() {
    let lifecycle = RequestLifecycle::default();
    let (release, finish) = oneshot::channel();
    let (cancelled, observed_cancel) = oneshot::channel();
    let signals = Arc::new(Mutex::new(Some((finish, cancelled))));
    let app = Router::new()
        .route(
            "/stream",
            get(move || {
                let (finish, cancelled) = signals.lock().unwrap().take().unwrap();
                async move {
                    spawn(async move {
                        peer_disconnected(CURRENT.try_with(Clone::clone).ok().flatten()).await;
                        cancelled.send(()).unwrap();
                        finish.await.unwrap();
                    });
                    Body::from_stream(futures_util::stream::pending::<Result<Bytes, Infallible>>())
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
    tokio::time::advance(REQUEST_BUDGET).await;
    observed_cancel.await.unwrap();
    lifecycle.begin_drain();
    assert_eq!(lifecycle.snapshot().in_flight, 1);
    assert!(!lifecycle.retire_drained(lifecycle.snapshot().revision));
    release.send(()).unwrap();
    lifecycle.wait_drained().await;
    assert!(
        axum::body::to_bytes(response.into_body(), 8192)
            .await
            .is_err()
    );
}

#[tokio::test(start_paused = true)]
async fn response_progress_does_not_refresh_deadline_and_normal_frames_keep_trailers() {
    let lifecycle = RequestLifecycle::default();
    let lease = lifecycle.admit().unwrap();
    let deadline = lease.0.response_deadline;
    let mut trailers = axum::http::HeaderMap::new();
    trailers.insert("x-finished", HeaderValue::from_static("yes"));
    let frames: Vec<Result<Frame<Bytes>, Infallible>> = vec![
        Ok(Frame::data(Bytes::from_static(b"chunk"))),
        Ok(Frame::trailers(trailers.clone())),
    ];
    let mut body = deadline_body::wrap(
        Body::new(StreamBody::new(futures_util::stream::iter(frames))),
        lease,
        deadline,
        None,
    );
    assert_eq!(
        body.frame().await.unwrap().unwrap().into_data().unwrap(),
        "chunk"
    );
    assert_eq!(
        body.frame()
            .await
            .unwrap()
            .unwrap()
            .into_trailers()
            .unwrap(),
        trailers
    );
    assert!(body.frame().await.is_none());
    assert_eq!(lifecycle.snapshot().in_flight, 0);

    let lease = lifecycle.admit().unwrap();
    let deadline = lease.0.response_deadline;
    let frames =
        futures_util::stream::once(async { Ok::<_, Infallible>(Bytes::from_static(b"one")) })
            .chain(futures_util::stream::pending());
    use futures_util::StreamExt as _;
    let mut body = deadline_body::wrap(Body::from_stream(frames), lease, deadline, None);
    tokio::time::advance(REQUEST_BUDGET - Duration::from_secs(1)).await;
    assert_eq!(
        body.frame().await.unwrap().unwrap().into_data().unwrap(),
        "one"
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(body.frame().await.unwrap().is_err());
    assert_eq!(lifecycle.snapshot().in_flight, 0);
}

#[tokio::test]
async fn background_recovery_owns_drain_until_detached_mutation_finishes() {
    let lifecycle = RequestLifecycle::default();
    let (started, ready) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let worker = lifecycle.clone();
    let task = tokio::spawn(async move {
        worker
            .run_background(async move {
                crate::finish_control_mutation(
                    Arc::new(tokio::sync::RwLock::new(false)),
                    async move {
                        started.send(()).unwrap();
                        released.await.unwrap();
                        Ok(())
                    },
                )
                .await
            })
            .await
    });
    ready.await.unwrap();
    task.abort();
    let _ = task.await;
    lifecycle.begin_drain();
    assert_eq!(lifecycle.snapshot().in_flight, 1);
    assert!(!lifecycle.retire_drained(lifecycle.snapshot().revision));
    assert_eq!(
        lifecycle
            .run_background(async { panic!("closed instance cannot admit recovery") })
            .await,
        None::<()>
    );
    release.send(()).unwrap();
    lifecycle.wait_drained().await;
    assert!(lifecycle.retire_drained(lifecycle.snapshot().revision));
}
