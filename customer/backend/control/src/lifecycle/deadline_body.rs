//! Deadline ownership is independent of whether the transport polls the body.
//! Expiration discards the source, cancels execution and releases only this
//! body's lease. Detached mutations/settlement keep their own owners.
use super::{PeerGuard, RequestLease};
use axum::body::{Body, Bytes};
use http_body::{Body as HttpBody, Frame, SizeHint};
use std::{
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};
use tokio::{sync::Notify, time::Instant};

struct Inner {
    source: Option<Body>,
    lease: Option<RequestLease>,
    peer: Option<PeerGuard>,
    deadline: Instant,
    expired: bool,
    error_emitted: bool,
    waker: Option<Waker>,
}

impl Inner {
    fn finish(&mut self) {
        self.source.take();
        self.peer.take();
        self.lease.take();
        self.waker.take();
    }
    fn expire(&mut self) -> Option<Waker> {
        let waker = self.waker.take();
        if self
            .source
            .as_ref()
            .is_some_and(|source| !source.is_end_stream())
        {
            self.expired = true;
            if let Some(lease) = &self.lease {
                lease.deadline_elapsed();
            }
        }
        self.finish();
        waker
    }
}

struct DeadlineBody {
    inner: Arc<Mutex<Inner>>,
    done: Arc<Notify>,
}

pub(super) fn wrap(
    body: Body,
    lease: RequestLease,
    deadline: Instant,
    peer: Option<PeerGuard>,
) -> Body {
    // An already empty source has no body work to own. In particular, do not
    // retain a request until the deadline when the transport skips polling it.
    if body.is_end_stream() {
        return body;
    }
    let inner = Arc::new(Mutex::new(Inner {
        source: Some(body),
        lease: Some(lease),
        peer,
        deadline,
        expired: false,
        error_emitted: false,
        waker: None,
    }));
    let done = Arc::new(Notify::new());
    let weak = Arc::downgrade(&inner);
    let finished = Arc::clone(&done);
    // This timer deliberately owns no RequestLease. EOF/drop wakes it; a body
    // never polled by a stalled client can still expire and release its source.
    tokio::spawn(async move {
        tokio::select! {
            biased;
            _ = finished.notified() => return,
            _ = tokio::time::sleep_until(deadline) => {},
        }
        if let Some(inner) = weak.upgrade() {
            let waker = inner.lock().expect("deadline body poisoned").expire();
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    });
    Body::new(DeadlineBody { inner, done })
}

impl Drop for DeadlineBody {
    fn drop(&mut self) {
        self.inner.lock().expect("deadline body poisoned").finish();
        self.done.notify_one();
    }
}

impl HttpBody for DeadlineBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let mut inner = self.inner.lock().expect("deadline body poisoned");
        if Instant::now() >= inner.deadline {
            // This poll is already awake; discard a previous poll's waker.
            let _ = inner.expire();
        }
        if inner.expired && !inner.error_emitted {
            inner.error_emitted = true;
            self.done.notify_one();
            return Poll::Ready(Some(Err(axum::Error::new(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "request deadline exceeded",
            )))));
        }
        let Some(source) = inner.source.as_mut() else {
            return Poll::Ready(None);
        };
        match Pin::new(source).poll_frame(cx) {
            Poll::Pending => {
                inner.waker = Some(cx.waker().clone());
                Poll::Pending
            }
            Poll::Ready(frame) => {
                if frame.is_none()
                    || frame.as_ref().is_some_and(Result::is_err)
                    || inner.source.as_ref().is_none_or(HttpBody::is_end_stream)
                {
                    // A final frame may be followed only by an is_end_stream
                    // check, with no extra EOF poll. Transport-buffered bytes
                    // remain the HTTP server's graceful-shutdown responsibility.
                    inner.finish();
                    self.done.notify_one();
                }
                Poll::Ready(frame)
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        let inner = self.inner.lock().expect("deadline body poisoned");
        if inner.expired && !inner.error_emitted {
            return false;
        }
        inner.source.as_ref().is_none_or(HttpBody::is_end_stream)
    }

    fn size_hint(&self) -> SizeHint {
        self.inner
            .lock()
            .expect("deadline body poisoned")
            .source
            .as_ref()
            .map_or_else(SizeHint::default, HttpBody::size_hint)
    }
}
