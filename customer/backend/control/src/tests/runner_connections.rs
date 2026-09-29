use super::*;
use aster_runner_protocol::encode_runner_frame;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message as ClientMessage, client::IntoClientRequest},
};

const TOKEN: &str = "test-runner-credential-01234567890123456789";
const RUNNER: &str = "runner_connection_test";
const NOW: &str = "2026-08-28T00:00:00.000Z";
type ClientSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

struct Fixture {
    state: ControlState,
    store: Arc<StdMutex<aster_storage::SqlCipherStore>>,
    address: std::net::SocketAddr,
    server: tokio::task::JoinHandle<()>,
    _directory: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Fixture {
    async fn new() -> Self {
        Self::new_with_maintenance(false).await
    }

    async fn new_with_maintenance(managed: bool) -> Self {
        let directory = tempdir().unwrap();
        let store = Arc::new(StdMutex::new(
            aster_storage::SqlCipherStore::initialize(
                &directory.path().join("customer.db"),
                &[121; 32],
            )
            .unwrap(),
        ));
        let license = verified_license();
        let history = Arc::new(
            LicenseStateStore::new(directory.path().join("history.json"), &[122; 32]).unwrap(),
        );
        history
            .initialize(&license, datetime!(2026-08-28 0:00 UTC))
            .unwrap();
        let mut state = ControlState::new("0.1.0", Some(license))
            .with_storage(ControlStorage::SqlCipher(Arc::clone(&store)))
            .with_license_state(history)
            .with_auth_core(AuthCore::new(&[122; 32], "installation_connection_test").unwrap())
            .with_task_issuer(RunnerTaskIssuer::new(
                "connection-test-key",
                SigningKey::from_bytes(&[123; 32]),
            ))
            .with_now(datetime!(2026-08-28 0:00 UTC));
        if managed {
            let layout = InstallLayout::new(directory.path()).unwrap();
            std::fs::create_dir_all(layout.staging()).unwrap();
            std::fs::create_dir_all(layout.upgrade_running()).unwrap();
            std::fs::write(layout.marker_path(), layout.marker_json().unwrap()).unwrap();
            std::fs::write(layout.maintenance_lock(), b"").unwrap();
            state = state.with_maintenance_layout(layout);
        }
        let owner = state
            .initialize_owner_identity(
                "owner@example.test",
                "Owner",
                Zeroizing::new(b"test-owner-password".to_vec()),
            )
            .await
            .unwrap();
        {
            let mut store = store.lock().unwrap();
            store
                .insert_runner_enrollment(&RunnerEnrollmentRecord {
                    id: "enrollment_connection_test".into(),
                    token_hash: "test_enrollment_hash".into(),
                    token_prefix: "test".into(),
                    runner_name: RUNNER.into(),
                    status: "pending".into(),
                    expires_at: "2026-08-29T00:00:00.000Z".into(),
                    created_by: owner.id,
                    created_at: NOW.into(),
                })
                .unwrap();
            assert_eq!(
                store
                    .consume_runner_enrollment_unchecked(
                        "test_enrollment_hash",
                        NOW,
                        &RunnerRegistrationRecord {
                            id: RUNNER.into(),
                            credential_hash: sha256_hex(TOKEN.as_bytes()),
                            version: "2.0.0".into(),
                            protocol_version: 2,
                            platform: "linux".into(),
                            architecture: "x86_64".into(),
                            max_inflight: 1,
                            created_at: NOW.into(),
                        }
                    )
                    .unwrap(),
                RunnerEnrollmentConsumeOutcome::Registered
            );
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = router(state.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            state,
            store,
            address,
            server,
            _directory: directory,
        }
    }

    async fn connect(&self) -> ClientSocket {
        let mut request = format!("ws://{}/api/runner/channel", self.address)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert(AUTHORIZATION, format!("Bearer {TOKEN}").parse().unwrap());
        tokio::time::timeout(std::time::Duration::from_secs(5), connect_async(request))
            .await
            .unwrap()
            .unwrap()
            .0
    }

    fn record(&self) -> RunnerRecord {
        self.store
            .lock()
            .unwrap()
            .runner_by_credential_hash(&sha256_hex(TOKEN.as_bytes()))
            .unwrap()
            .unwrap()
    }

    async fn registered_after(&self, previous: u64) -> u64 {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(generation) = self
                    .state
                    .runner_hub
                    .inner
                    .connections
                    .lock()
                    .await
                    .get(RUNNER)
                    .map(|value| value.generation)
                    .filter(|value| *value > previous)
                {
                    return generation;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap()
    }
}

fn hello(protocol: u32, capacity: u32) -> RunnerHello {
    RunnerHello {
        runner_id: RUNNER.into(),
        protocol_version: protocol,
        runner_version: "2.1.0".into(),
        platform: "linux".into(),
        architecture: "x86_64".into(),
        max_inflight: capacity,
    }
}

async fn send(socket: &mut ClientSocket, frame: RunnerToControl) {
    socket
        .send(ClientMessage::Text(
            String::from_utf8(encode_runner_frame(&frame).unwrap())
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
}

async fn closed(socket: &mut ClientSocket) {
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
        .await
        .unwrap();
    assert!(matches!(
        result,
        None | Some(Err(_)) | Some(Ok(ClientMessage::Close(_)))
    ));
}

#[tokio::test]
async fn real_websocket_upgrades_existing_registration_and_rejects_old_hello() {
    let f = Fixture::new().await;
    let mut old = f.connect().await;
    send(&mut old, RunnerToControl::Hello(hello(2, 1))).await;
    closed(&mut old).await;
    assert_eq!(f.record().protocol_version, 2);
    assert!(f.state.runner_hub.inner.connections.lock().await.is_empty());

    let mut current = f.connect().await;
    send(&mut current, RunnerToControl::Hello(hello(4, 2))).await;
    let first_generation = f.registered_after(0).await;
    assert_eq!(f.record().protocol_version, 4);
    assert_eq!(f.record().max_inflight, 2);
    assert_eq!(f.store.lock().unwrap().list_runners().unwrap().len(), 1);

    let payload = serde_json::to_vec(&UpstreamHttpRequest {
        method: "GET".into(),
        url: OPENAI_CODEX_MODELS_ENDPOINT.into(),
        headers: vec![],
        body_base64url: String::new(),
    })
    .unwrap();
    let mut task = f
        .state
        .dispatch_runner_task(
            None,
            &RunnerTaskBinding::credential(
                "openai",
                &transport_credential_fixture().id,
                0,
                "chatgpt.com",
            ),
            RunnerTaskAuthorization::DiscoverModels {
                actor: None,
                credential: &transport_credential_fixture(),
            },
            &payload,
        )
        .await
        .unwrap();
    let ClientMessage::Text(encoded) =
        tokio::time::timeout(std::time::Duration::from_secs(5), current.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    else {
        panic!("expected task");
    };
    let ControlToRunner::Task(frame) = decode_control_frame(encoded.as_bytes()).unwrap() else {
        panic!("expected signed task");
    };
    let mut keys = TrustedTaskKeys::new();
    keys.insert(
        "connection-test-key",
        SigningKey::from_bytes(&[123; 32]).verifying_key(),
    )
    .unwrap();
    let ticket = verify_task_ticket(
        &decode_base64url(&frame.ticket_json_base64url).unwrap(),
        &keys,
        RUNNER,
        &payload,
        datetime!(2026-08-28 0:00 UTC),
    )
    .unwrap();
    assert_eq!(ticket.claims().schema, "aster.runner-task.v4");
    assert_eq!(ticket.claims().task_id, task.task_id);
    send(
        &mut current,
        RunnerToControl::TaskAccepted(TaskLifecycle {
            task_id: task.task_id.clone(),
        }),
    )
    .await;
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(5), task.recv())
            .await
            .unwrap(),
        Some(RunnerToControl::TaskAccepted(_))
    ));

    let mut next = f.connect().await;
    send(&mut next, RunnerToControl::Hello(hello(4, 5))).await;
    f.registered_after(first_generation).await;
    assert_eq!(f.record().max_inflight, 5);
    closed(&mut current).await;
    let mut downgraded = f.connect().await;
    send(&mut downgraded, RunnerToControl::Hello(hello(2, 1))).await;
    closed(&mut downgraded).await;
    assert_eq!(f.record().protocol_version, 4);
    assert_eq!(f.record().max_inflight, 5);
}

#[tokio::test]
async fn disable_between_websocket_upgrade_and_hello_cannot_register() {
    let f = Fixture::new().await;
    let mut socket = f.connect().await;
    f.store
        .lock()
        .unwrap()
        .update_runner_enabled(RUNNER, false, NOW)
        .unwrap();
    send(&mut socket, RunnerToControl::Hello(hello(4, 2))).await;
    closed(&mut socket).await;
    assert!(f.state.runner_hub.inner.connections.lock().await.is_empty());
    assert_eq!(
        f.store.lock().unwrap().list_runners().unwrap()[0].protocol_version,
        2
    );
}

#[tokio::test]
async fn old_generation_and_other_runner_cannot_publish_into_current_task() {
    let hub = RunnerHub::default();
    let (old, _old_outbound) = hub.register(RUNNER).await;
    let (current, _outbound) = hub.register(RUNNER).await;
    let (other, _other_outbound) = hub.register("runner_other").await;
    let slot = hub.reserve_dispatch(RUNNER).await.unwrap();
    let (mut events, _cancel) = hub
        .dispatch_reserved(slot, "task_connection_test", vec![])
        .await
        .unwrap();
    let frame = || {
        RunnerToControl::TaskAccepted(TaskLifecycle {
            task_id: "task_connection_test".into(),
        })
    };
    hub.publish_from(RUNNER, old, frame()).await;
    hub.publish_from("runner_other", other, frame()).await;
    assert!(events.try_recv().is_err());
    assert!(
        hub.inner
            .tasks
            .lock()
            .await
            .contains_key("task_connection_test")
    );
    hub.publish_from(RUNNER, current, frame()).await;
    assert!(matches!(
        events.try_recv(),
        Ok(RunnerToControl::TaskAccepted(_))
    ));
}

fn heartbeat() -> RunnerHeartbeat {
    RunnerHeartbeat {
        inflight: 1,
        recent_request_count: 7,
        recent_error_count: 0,
        latency_ms: 10,
        observed_at: datetime!(2026-08-28 0:00 UTC).unix_timestamp(),
    }
}

#[tokio::test]
async fn old_heartbeat_cannot_commit_after_new_connection_metadata() {
    let f = Fixture::new().await;
    let record = f.record();
    let old_hello = hello(4, 2);
    let credential_hash = sha256_hex(TOKEN.as_bytes());
    let (old_generation, _old_outbound) =
        register_runner_connection(&f.state, &record, &old_hello, &credential_hash)
            .await
            .unwrap();
    let current_record = f.record();
    let gate = f.state.runner_hub.session_gate(RUNNER).await;
    // Hold the real SQLite connection on a worker, so the old heartbeat has
    // acquired its session gate but cannot yet complete the database write.
    let store = Arc::clone(&f.store);
    let (locked, ready) = oneshot::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let blocker = tokio::task::spawn_blocking(move || {
        let _guard = store.lock().unwrap();
        locked.send(()).unwrap();
        let _ = resume.recv();
    });
    ready.await.unwrap();
    let mut old = Box::pin(record_heartbeat(
        &f.state,
        RUNNER,
        old_generation,
        &credential_hash,
        &old_hello,
        heartbeat(),
    ));
    assert!(futures_util::poll!(old.as_mut()).is_pending());
    assert!(
        gate.try_lock().is_err(),
        "old heartbeat must hold the per-Runner gate while storage waits"
    );
    let new_hello = hello(4, 5);
    let mut new = Box::pin(register_runner_connection(
        &f.state,
        &current_record,
        &new_hello,
        &credential_hash,
    ));
    assert!(futures_util::poll!(new.as_mut()).is_pending());
    assert!(
        f.state
            .runner_hub
            .is_current_connection(RUNNER, old_generation)
            .await
    );
    release.send(()).unwrap();
    blocker.await.unwrap();
    let (accepted, replacement) = tokio::join!(old, new);
    assert!(accepted);
    let (new_generation, _new_outbound) = replacement.unwrap();
    assert!(new_generation > old_generation);
    assert_eq!(f.record().max_inflight, 5);
    assert_eq!(f.record().inflight, 0);
    assert!(
        !record_heartbeat(
            &f.state,
            RUNNER,
            old_generation,
            &credential_hash,
            &old_hello,
            heartbeat()
        )
        .await
    );
    assert_eq!(f.record().max_inflight, 5);
}

#[tokio::test]
async fn queued_hellos_and_old_heartbeat_follow_one_takeover_order() {
    let f = Fixture::new().await;
    let record = f.record();
    let credential_hash = sha256_hex(TOKEN.as_bytes());
    let old_hello = hello(4, 1);
    let (old_generation, _old) =
        register_runner_connection(&f.state, &record, &old_hello, &credential_hash)
            .await
            .unwrap();
    let record = f.record();
    let gate = f.state.runner_hub.session_gate(RUNNER).await;
    let held = gate.lock().await;
    let first_hello = hello(4, 4);
    let second_hello = hello(4, 9);
    let mut first = Box::pin(register_runner_connection(
        &f.state,
        &record,
        &first_hello,
        &credential_hash,
    ));
    let mut second = Box::pin(register_runner_connection(
        &f.state,
        &record,
        &second_hello,
        &credential_hash,
    ));
    let mut stale = Box::pin(record_heartbeat(
        &f.state,
        RUNNER,
        old_generation,
        &credential_hash,
        &old_hello,
        heartbeat(),
    ));
    assert!(futures_util::poll!(first.as_mut()).is_pending());
    assert!(futures_util::poll!(second.as_mut()).is_pending());
    assert!(futures_util::poll!(stale.as_mut()).is_pending());
    drop(held);
    let (first, second, stale) = tokio::join!(first, second, stale);
    let (first_generation, _first_outbound) = first.unwrap();
    let (second_generation, _second_outbound) = second.unwrap();
    assert!(second_generation > first_generation);
    assert!(!stale);
    assert_eq!(f.record().max_inflight, 9);
    assert!(
        f.state
            .runner_hub
            .is_current_connection(RUNNER, second_generation)
            .await
    );
    assert!(
        !f.state
            .runner_hub
            .is_current_connection(RUNNER, first_generation)
            .await
    );
}

#[tokio::test]
async fn upgrade_and_unrecovered_job_cannot_promote_runner_protocol() {
    let f = Fixture::new_with_maintenance(true).await;
    let layout = &f.state.maintenance.as_ref().unwrap().layout;
    let executor_lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(layout.maintenance_lock())
        .unwrap();
    executor_lock.try_lock().unwrap();
    let record = f.record();
    let new_hello = hello(4, 5);
    let credential = sha256_hex(TOKEN.as_bytes());
    assert!(matches!(
        register_runner_connection(&f.state, &record, &new_hello, &credential).await,
        Err(ControlError::MaintenanceBusy)
    ));
    assert_eq!(f.record().protocol_version, 2);
    let interrupted = layout.upgrade_running().join("interrupted.json");
    std::fs::write(&interrupted, b"not-yet-recovered").unwrap();
    // Simulate executor death: the OS releases its lock, but recovery has not
    // reconciled the active slot or restored the old service yet.
    drop(executor_lock);
    assert!(matches!(
        register_runner_connection(&f.state, &record, &new_hello, &credential).await,
        Err(ControlError::MaintenanceBusy)
    ));
    assert_eq!(f.record().protocol_version, 2);
    std::fs::remove_file(interrupted).unwrap();
    let (_generation, _outbound) =
        register_runner_connection(&f.state, &record, &new_hello, &credential)
            .await
            .unwrap();
    assert_eq!(f.record().protocol_version, 4);
}

async fn cancellation_preserves_session_order(heartbeat_mode: bool) {
    let f = Fixture::new_with_maintenance(true).await;
    let credential = sha256_hex(TOKEN.as_bytes());
    let old_hello = hello(4, 2);
    let (generation, _old) =
        register_runner_connection(&f.state, &f.record(), &old_hello, &credential)
            .await
            .unwrap();
    let record = f.record();
    let gate = f.state.runner_hub.session_gate(RUNNER).await;
    let store = Arc::clone(&f.store);
    let (locked, ready) = oneshot::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let blocker = tokio::task::spawn_blocking(move || {
        let _guard = store.lock().unwrap();
        locked.send(()).unwrap();
        let _ = resume.recv();
    });
    ready.await.unwrap();
    let mut pending = Box::pin(async {
        if heartbeat_mode {
            assert!(
                record_heartbeat(
                    &f.state,
                    RUNNER,
                    generation,
                    &credential,
                    &old_hello,
                    heartbeat(),
                )
                .await
            );
        } else {
            let _result = register_runner_connection(&f.state, &record, &old_hello, &credential)
                .await
                .unwrap();
        }
    });
    assert!(futures_util::poll!(pending.as_mut()).is_pending());
    drop(pending);
    assert!(
        gate.try_lock().is_err(),
        "caller cancellation released session order"
    );
    let executor_lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(
            f.state
                .maintenance
                .as_ref()
                .unwrap()
                .layout
                .maintenance_lock(),
        )
        .unwrap();
    if !heartbeat_mode {
        assert!(matches!(
            executor_lock.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
    }
    let next_hello = hello(4, 9);
    let mut replacement = Box::pin(register_runner_connection(
        &f.state,
        &record,
        &next_hello,
        &credential,
    ));
    assert!(futures_util::poll!(replacement.as_mut()).is_pending());
    release.send(()).unwrap();
    blocker.await.unwrap();
    let (current, _outbound) = replacement.await.unwrap();
    assert!(current > generation);
    assert_eq!(f.record().max_inflight, 9);
    assert_eq!(f.record().inflight, 0);
    executor_lock.try_lock().unwrap();
}

#[tokio::test]
async fn cancelled_hello_keeps_both_leases_until_commit_and_takeover() {
    cancellation_preserves_session_order(false).await;
}

#[tokio::test]
async fn cancelled_heartbeat_keeps_session_gate_until_commit() {
    cancellation_preserves_session_order(true).await;
}
