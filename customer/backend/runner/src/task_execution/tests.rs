use super::*;
use aster_runner_protocol::{
    AdminSubject, ControlService, ImageModelSubject, MaintenanceActor, ModelResource, ModelSubject,
    SignedExpiry, TaskAuthorization, TaskLicense, TaskTicketIssue, issue_task_ticket,
};
use ed25519_dalek::SigningKey;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

const HOST: &str = "upstream.example.test";
const RUNNER: &str = "runner_execution_test";
const CREDENTIAL: &str = "credential_00000000000000000000000000000001";

fn request(port: u16) -> UpstreamHttpRequest {
    UpstreamHttpRequest {
        method: "POST".to_owned(),
        url: format!("https://{HOST}:{port}/execute"),
        headers: vec![],
        body_base64url: encode_base64url(b"{}"),
    }
}

fn signed_frame(command: TaskCommand, payload: &[u8]) -> (TaskFrame, TrustedTaskKeys) {
    let key = SigningKey::from_bytes(&[111; 32]);
    let now = time::OffsetDateTime::now_utc();
    let expires_at = now.unix_timestamp() + 120;
    let license = TaskLicense {
        license_id: "license-execution-test".into(),
        license_sha256: "ab".repeat(32),
        expiry: SignedExpiry::Fixed { expires_at },
    };
    let authorization = match command {
        TaskCommand::Probe => panic!("probe uses its own internal worker fixture"),
        TaskCommand::Execute => TaskAuthorization::Model {
            subject: ModelSubject {
                identity_id: "identity-execution-test".into(),
                api_key_id: "api-key-execution-test".into(),
                request_id: "request-execution-test".into(),
                reservation_id: "reservation-execution-test".into(),
                reserved_tokens: 100,
                reservation_expires_at: expires_at,
            },
            resource: ModelResource {
                account_id: "account-execution-test".into(),
                public_model: "test-model".into(),
                upstream_model: "test-model".into(),
            },
            license,
        },
        TaskCommand::FetchAsset => TaskAuthorization::FetchAsset {
            subject: ImageModelSubject {
                identity_id: "identity-execution-test".into(),
                api_key_id: "api-key-execution-test".into(),
                request_id: "request-execution-test".into(),
                reservation_id: "reservation-execution-test".into(),
                reserved_images: 1,
                reservation_expires_at: expires_at,
            },
            resource: ModelResource {
                account_id: "account-execution-test".into(),
                public_model: "test-model".into(),
                upstream_model: "test-model".into(),
            },
            license,
        },
        TaskCommand::DiscoverModels => TaskAuthorization::DiscoverModels {
            actor: MaintenanceActor::Service {
                service: ControlService::ModelCatalog,
            },
            account_id: "account-execution-test".into(),
            license,
        },
        TaskCommand::RefreshCredential => TaskAuthorization::RefreshCredential {
            actor: MaintenanceActor::Service {
                service: ControlService::CredentialBroker,
            },
            account_id: "account-execution-test".into(),
            lease_sha256: "cd".repeat(32),
            lease_expires_at: expires_at,
            license,
        },
        TaskCommand::AuthorizeCredential => TaskAuthorization::AuthorizeCredential {
            actor: AdminSubject {
                identity_id: "admin-execution-test".into(),
            },
            enrollment_id: "enrollment-execution-test".into(),
            session_expires_at: expires_at,
            license,
        },
    };
    let document = issue_task_ticket(TaskTicketIssue {
        key_id: "task-execution-test",
        signing_key: &key,
        task_id: "task_execution_test",
        runner_id: RUNNER,
        provider_id: "openai",
        credential_instance_id: (command != TaskCommand::AuthorizeCredential).then_some(CREDENTIAL),
        credential_revision: (command != TaskCommand::AuthorizeCredential).then_some(1),
        upstream_host: HOST,
        command,
        authorization,
        payload,
        nonce: "execution_nonce",
        now,
        execution_timeout_ms: aster_runner_protocol::MAX_TASK_EXECUTION_MILLISECONDS,
    })
    .unwrap();
    let mut keys = TrustedTaskKeys::new();
    keys.insert("task-execution-test", key.verifying_key())
        .unwrap();
    (
        TaskFrame {
            ticket_json_base64url: encode_base64url(&serde_json::to_vec(&document).unwrap()),
            payload_base64url: encode_base64url(payload),
        },
        keys,
    )
}

fn permit(command: TaskCommand, payload: &[u8]) -> TaskExecutionPermit {
    let (frame, keys) = signed_frame(command, payload);
    let now = time::OffsetDateTime::now_utc();
    let instant = Instant::now();
    let ticket = verify_task_ticket(
        &decode_base64url(&frame.ticket_json_base64url).unwrap(),
        &keys,
        RUNNER,
        payload,
        now,
    )
    .unwrap();
    TaskExecutionPermit::new(ticket, now, instant).unwrap()
}

fn listener_and_client() -> (std::net::TcpListener, Client) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let client = Client::builder()
        .no_proxy()
        .resolve(HOST, listener.local_addr().unwrap())
        .build()
        .unwrap();
    (listener, client)
}

#[tokio::test(start_paused = true)]
async fn delayed_accepted_event_expires_and_releases_runner_capacity_without_connecting() {
    let (listener, client) = listener_and_client();
    let payload = serde_json::to_vec(&request(listener.local_addr().unwrap().port())).unwrap();
    let (frame, keys) = signed_frame(TaskCommand::Execute, &payload);
    let config = Arc::new(RuntimeConfig {
        control_wss: "wss://control.example.test".to_owned(),
        runner_id: RUNNER.to_owned(),
        credential: Zeroizing::new("test-only".to_owned()),
        control_tls_connector: None,
        upstream_ca_certificate: None,
        task_keys: Arc::new(keys),
        max_inflight: 1,
        heartbeat: StdDuration::from_secs(10),
        allowed_upstream_hosts: Arc::new(BTreeSet::from([HOST.to_owned()])),
    });
    let metrics = Arc::new(Metrics::default());
    let replay = Arc::new(Mutex::new(ReplayCache::new()));
    let mut tasks = TaskSupervisor::new(1);
    let (outbound, mut events) = mpsc::channel(1);
    outbound
        .try_send(RunnerToControl::Pong(PingFrame {
            nonce: "occupied".to_owned(),
        }))
        .unwrap();
    accept_task(
        frame, &config, &metrics, &replay, &mut tasks, &client, &outbound,
    )
    .await
    .unwrap();
    assert!(tasks.try_permit().is_none());
    tokio::task::yield_now().await;
    tokio::time::advance(StdDuration::from_secs(121)).await;
    tokio::time::timeout(StdDuration::from_secs(1), tasks.next_finished())
        .await
        .unwrap();
    assert!(tasks.try_permit().is_some());
    assert_eq!(metrics.inflight.load(Ordering::Relaxed), 0);
    assert!(matches!(
        events.try_recv().unwrap(),
        RunnerToControl::Pong(_)
    ));
    assert!(events.try_recv().is_err());
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[tokio::test(start_paused = true)]
async fn delayed_upstream_event_never_connects_for_execute_or_refresh() {
    for command in [
        TaskCommand::Execute,
        TaskCommand::DiscoverModels,
        TaskCommand::AuthorizeCredential,
        TaskCommand::RefreshCredential,
    ] {
        let (listener, client) = listener_and_client();
        let request = request(listener.local_addr().unwrap().port());
        let payload = if command == TaskCommand::RefreshCredential {
            serde_json::to_vec(&CredentialRefreshTask {
                credential_id: CREDENTIAL.to_owned(),
                expected_revision: 1,
                request,
            })
            .unwrap()
        } else {
            serde_json::to_vec(&request).unwrap()
        };
        let permit = permit(command, &payload);
        let (outbound, mut events) = mpsc::channel(1);
        permit.accepted(&outbound).await.unwrap();
        // TaskAccepted occupies the only slot. The production execution path
        // must time out waiting to publish UpstreamStarted before touching TLS.
        let hosts = BTreeSet::from([HOST.to_owned()]);
        let execute = async {
            if command == TaskCommand::RefreshCredential {
                execute_credential_refresh(
                    &client,
                    &hosts,
                    CredentialRefreshBinding {
                        upstream_host: HOST,
                        credential_instance_id: Some(CREDENTIAL),
                        credential_revision: Some(1),
                    },
                    &permit,
                    Zeroizing::new(payload),
                    &outbound,
                )
                .await
            } else {
                execute_task(
                    &client,
                    &hosts,
                    HOST,
                    false,
                    &permit,
                    Zeroizing::new(payload),
                    &outbound,
                )
                .await
            }
        };
        let expiry = async {
            tokio::task::yield_now().await;
            tokio::time::advance(StdDuration::from_secs(121)).await;
        };
        let (result, ()) = tokio::join!(execute, expiry);
        assert!(result.is_err());
        assert!(matches!(
            events.try_recv().unwrap(),
            RunnerToControl::TaskAccepted(_)
        ));
        assert!(events.try_recv().is_err());
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}

#[tokio::test(start_paused = true)]
async fn monotonic_expiry_cannot_be_extended_by_an_earlier_wall_clock() {
    let permit = permit(TaskCommand::Execute, b"{}");
    assert!(permit.valid());
    tokio::time::advance(StdDuration::from_secs(121)).await;
    // Tokio's time advanced; the operating system's wall clock did not. A
    // wall-clock-only check would still accept this verified ticket.
    permit
        .ticket
        .validate_execution_time(time::OffsetDateTime::now_utc())
        .unwrap();
    assert!(!permit.valid());
    let (outbound, mut events) = mpsc::channel(1);
    assert!(permit.start_upstream(&outbound).await.is_err());
    let RunnerToControl::TaskFailed(failure) = events.recv().await.unwrap() else {
        panic!("expired task must fail");
    };
    assert_eq!(failure.category, "task_ticket_expired");
    assert!(failure.retryable_before_upstream);
}

#[tokio::test]
async fn valid_execution_completes_a_real_tls_request() {
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec![HOST.to_owned()]).unwrap();
    let crypto = rustls::crypto::ring::default_provider();
    let server = rustls::ServerConfig::builder_with_provider(Arc::new(crypto))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into(),
        )
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let client = Client::builder()
        .no_proxy()
        .resolve(HOST, address)
        .add_root_certificate(reqwest::Certificate::from_der(cert.der()).unwrap())
        .build()
        .unwrap();
    let payload = serde_json::to_vec(&request(address.port())).unwrap();
    let permit = permit(TaskCommand::Execute, &payload);
    let (outbound, mut events) = mpsc::channel(16);
    permit.accepted(&outbound).await.unwrap();
    let hosts = BTreeSet::from([HOST.to_owned()]);
    let execute = execute_task(
        &client,
        &hosts,
        HOST,
        false,
        &permit,
        Zeroizing::new(payload),
        &outbound,
    );
    let serve = async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut tls = acceptor.accept(stream).await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 1024];
        while !request.windows(4).any(|part| part == b"\r\n\r\n") {
            let count = tls.read(&mut buffer).await.unwrap();
            assert!(count > 0);
            request.extend_from_slice(&buffer[..count]);
        }
        assert!(request.starts_with(b"POST /execute HTTP/1.1\r\n"));
        tls.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
            .await
            .unwrap();
        tls.shutdown().await.unwrap();
    };
    let (result, ()) = tokio::time::timeout(StdDuration::from_secs(10), async {
        tokio::join!(execute, serve)
    })
    .await
    .unwrap();
    result.unwrap();
    assert!(matches!(
        events.recv().await.unwrap(),
        RunnerToControl::TaskAccepted(_)
    ));
    assert!(matches!(
        events.recv().await.unwrap(),
        RunnerToControl::UpstreamStarted(_)
    ));
    assert!(matches!(
        events.recv().await.unwrap(),
        RunnerToControl::TaskResponseStarted(_)
    ));
    let mut finished = false;
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event, RunnerToControl::TaskFailed(_)));
        if let RunnerToControl::TaskFinished(result) = event {
            assert_eq!(result.status, 200);
            finished = true;
        }
    }
    assert!(finished);
}
