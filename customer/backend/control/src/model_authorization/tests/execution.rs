use super::*;
use crate::model_execution::{ModelExecution, RunnerTaskAuthorization};
use aster_runner_protocol::{TaskLifecycle, decode_control_frame};
use tower::ServiceExt as _;

const NOW: &str = "2026-09-07T00:00:00.000Z";
const MODEL: &str = "gpt-test";

#[tokio::test]
async fn monetary_reservation_revalidates_at_runner_dispatch_without_token_reservation() {
    use aster_policy_core::billing::{
        Currency, Money, TokenPrices,
        admission::AdmissionOutcome,
        plan::{RateCard, SelectedPrice},
    };

    let f = fixture().await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    let request_id = gateway_request_id(&HeaderMap::new()).unwrap();
    let AdmissionOutcome::Acquired(permit) = f
        .state
        .money_admission
        .try_acquire(
            consumer.identity_id(),
            Money {
                currency: Currency::Usd,
                nanos: 1_000_000_000,
            },
            Money {
                currency: Currency::Usd,
                nanos: 1_000_000_000,
            },
            None,
        )
        .unwrap()
    else {
        panic!("positive balance must admit first request");
    };
    let reservation = QuotaReservation {
        id: "billing_test".into(),
        identity_id: consumer.identity_id().into(),
        request_id: request_id.as_str().into(),
        client_request_id: None,
        reserved_tokens: 1,
        billing: Some(FrozenBillingContext {
            selected_price: SelectedPrice {
                public_model: MODEL.into(),
                version: "test".into(),
                tier: "standard".into(),
                currency: Currency::Usd,
                rate: RateCard::Tokens(TokenPrices {
                    input: Some(1_000_000_000),
                    output: Some(1_000_000_000),
                    ..TokenPrices::default()
                }),
            },
            settlement_currency: Currency::Usd,
            usd_to_cny: None,
            selected_at: NOW.into(),
            reference_nanos: 1_000_000_000,
        }),
        _money_permit: Some(Arc::new(permit)),
        execution: None,
    };
    let operation = operation();
    let execution = ModelExecution::new(&consumer, &reservation, &request_id, MODEL, &operation);
    let attempt = execution
        .prepare(&f.state, &HashSet::new(), None)
        .await
        .unwrap();
    let payload = serde_json::to_vec(attempt.request()).unwrap();
    RunnerTaskAuthorization::Model(&attempt)
        .authorize(&f.state, attempt.binding(), &payload)
        .await
        .unwrap();
}

async fn fixture() -> Fixture {
    let mut f = Fixture::with_features(vec![CapabilityId::Member, CapabilityId::Runner]).await;
    f.state = f
        .state
        .with_credential_vault(
            CredentialVault::new(&[109; 32], "installation_consumption_test").unwrap(),
        )
        .with_task_issuer(RunnerTaskIssuer::new(
            "task-test",
            SigningKey::from_bytes(&[110; 32]),
        ));
    {
        let mut store = f.store.lock().unwrap();
        store
            .insert_upstream_account_unchecked(
                "account_test",
                "openai",
                "subject_test",
                "upstream@example.test",
                NOW,
            )
            .unwrap();
        store
            .replace_account_models(
                "account_test",
                &[DiscoveredModel {
                    id: "model_test".to_owned(),
                    public_name: MODEL.to_owned(),
                    upstream_name: MODEL.to_owned(),
                    display_name: "Test model".to_owned(),
                }],
                NOW,
            )
            .unwrap();
        for id in ["runner_first", "runner_second"] {
            store
                .insert_runner_enrollment(&RunnerEnrollmentRecord {
                    id: format!("enrollment_{id}"),
                    token_hash: format!("hash_{id}"),
                    token_prefix: "test".to_owned(),
                    runner_name: id.to_owned(),
                    status: "pending".to_owned(),
                    expires_at: "2026-09-08T00:00:00.000Z".to_owned(),
                    created_by: f.identity.id.clone(),
                    created_at: NOW.to_owned(),
                })
                .unwrap();
            assert_eq!(
                store
                    .consume_runner_enrollment_unchecked(
                        &format!("hash_{id}"),
                        NOW,
                        &RunnerRegistrationRecord {
                            id: id.to_owned(),
                            credential_hash: format!("credential_{id}"),
                            version: "2.0.1".to_owned(),
                            protocol_version: RUNNER_PROTOCOL_VERSION,
                            platform: "linux".to_owned(),
                            architecture: "x86_64".to_owned(),
                            max_inflight: 4,
                            created_at: NOW.to_owned(),
                        }
                    )
                    .unwrap(),
                RunnerEnrollmentConsumeOutcome::Registered
            );
            store
                .record_runner_heartbeat(
                    id,
                    "2.0.1",
                    4,
                    aster_storage::RunnerHeartbeatUpdate {
                        inflight: 0,
                        recent_request_count: 0,
                        recent_error_count: 0,
                        latency_ms: 1,
                    },
                    NOW,
                )
                .unwrap();
        }
    }
    f.state.create_credential_instance("account_test", b"test-refresh", &serde_json::to_vec(&json!({
        "schema": OPENAI_CREDENTIAL_SCHEMA, "provider":"openai", "access_token":"test-access",
        "refresh_token":"test-refresh", "id_token":"", "expires_at":"2026-09-08T00:00:00.000Z",
        "account_id":"subject_test", "email":"upstream@example.test", "plan":"plus"
    })).unwrap(), "2026-09-08T00:00:00.000Z").await.unwrap();
    f.state
        .update_model_access(
            &f.identity.id,
            &f.identity,
            crate::model_access::UpdateModelAccessRequest {
                expected_revision: 0,
                mode: "selected".into(),
                model_ids: vec!["model_test".into()],
            },
        )
        .await
        .unwrap();
    f
}

async fn enable_monetary_gateway(f: &Fixture) {
    use aster_policy_core::billing::{
        Currency, TokenPrices,
        plan::{ModelPriceVersion, PriceTier, RateCard},
        schedule::PriceSchedule,
    };

    let storage = f.state.credential_storage().unwrap();
    let configuration = StoredBillingConfiguration {
        settlement_currency: BillingSettlementCurrency::Usd,
        usd_to_cny_nanos: None,
    };
    let book = StoredBillingPriceBook {
        public_model: MODEL.to_owned(),
        active_version: "test-price".to_owned(),
        versions: vec![StoredBillingPriceVersion {
            plan: ModelPriceVersion {
                public_model: MODEL.to_owned(),
                version: "test-price".to_owned(),
                currency: Currency::Usd,
                tiers: vec![PriceTier {
                    id: "standard".to_owned(),
                    schedule: PriceSchedule {
                        base: RateCard::Tokens(TokenPrices {
                            input: Some(1_000_000_000),
                            output: Some(1_000_000_000),
                            ..TokenPrices::default()
                        }),
                        windows: Vec::new(),
                    },
                }],
            },
            source: BillingPriceSource::Manual,
            source_url: None,
            verified_at: None,
            saved_at: NOW.to_owned(),
        }],
    };
    for (key, value) in [
        (
            BILLING_CONFIGURATION_KEY.to_owned(),
            serde_json::to_string(&configuration).unwrap(),
        ),
        (
            billing_price_key(MODEL),
            serde_json::to_string(&book).unwrap(),
        ),
    ] {
        let mut record = RuntimeSettingRecord {
            key,
            value,
            revision: 0,
            integrity_hmac: String::new(),
            updated_at: NOW.to_owned(),
        };
        record.integrity_hmac =
            runtime_setting_integrity_hmac(f.state.auth_core().unwrap(), &record).unwrap();
        assert!(matches!(
            storage.write_runtime_setting(None, &record).await.unwrap(),
            RuntimeSettingWriteOutcome::Applied
        ));
    }
    f.state
        .append_money_entry(
            &f.identity.id,
            "grant",
            10_000_000_000,
            "gateway-queue-fixture-grant",
            r#"{"reason":"gateway queue test"}"#,
            None,
        )
        .await
        .unwrap();
}

fn operation() -> providers::CanonicalOperation {
    providers::CanonicalOperation::Text(providers::types::CanonicalTextOperation {
        schema_version: providers::CANONICAL_SCHEMA_VERSION,
        public_model: MODEL.to_owned(),
        stream: false,
        request: json!({"model":MODEL, "input":"hello"}),
    })
}

async fn receive_ticket(
    outbound: &mut mpsc::Receiver<Vec<u8>>,
    runner: &str,
) -> aster_runner_protocol::VerifiedTaskTicket {
    let encoded = tokio::time::timeout(std::time::Duration::from_secs(5), outbound.recv())
        .await
        .unwrap()
        .unwrap();
    let ControlToRunner::Task(frame) = decode_control_frame(&encoded).unwrap() else {
        panic!("expected task")
    };
    let mut keys = aster_runner_protocol::TrustedTaskKeys::new();
    keys.insert(
        "task-test",
        SigningKey::from_bytes(&[110; 32]).verifying_key(),
    )
    .unwrap();
    aster_runner_protocol::verify_task_ticket(
        &decode_base64url(&frame.ticket_json_base64url).unwrap(),
        &keys,
        runner,
        &decode_base64url(&frame.payload_base64url).unwrap(),
        datetime!(2026-09-07 0:00 UTC),
    )
    .unwrap()
}

async fn respond(state: &ControlState, task_id: &str) {
    let lifecycle = || TaskLifecycle {
        task_id: task_id.to_owned(),
    };
    state
        .runner_hub
        .publish(RunnerToControl::TaskAccepted(lifecycle()))
        .await;
    state
        .runner_hub
        .publish(RunnerToControl::UpstreamStarted(lifecycle()))
        .await;
    state
        .runner_hub
        .publish(RunnerToControl::TaskResponseStarted(
            aster_runner_protocol::TaskResponseStarted {
                task_id: task_id.to_owned(),
                status: 200,
                headers: vec![],
            },
        ))
        .await;
    state
        .runner_hub
        .publish(RunnerToControl::TaskChunk(
            aster_runner_protocol::TaskChunk {
                task_id: task_id.to_owned(),
                sequence: 0,
                data_base64url: encode_base64url(b"response"),
            },
        ))
        .await;
    state
        .runner_hub
        .publish(RunnerToControl::TaskFinished(
            aster_runner_protocol::TaskResult {
                task_id: task_id.to_owned(),
                status: 200,
                usage_json: None,
            },
        ))
        .await;
}

#[tokio::test]
async fn real_model_execution_dispatches_signed_execute_and_collects_response() {
    let f = fixture().await;
    let (_, mut outbound) = f.state.runner_hub.register("runner_first").await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    let request_id = gateway_request_id(&HeaderMap::new()).unwrap();
    let reservation = f
        .state
        .reserve_model_quota(&consumer, &request_id, 1)
        .await
        .unwrap();
    let operation = operation();
    let scope = ModelExecution::new(&consumer, &reservation, &request_id, MODEL, &operation);
    let attempt = scope
        .prepare(&f.state, &HashSet::new(), None)
        .await
        .unwrap();
    let execution = f.state.execute_runner_http_request(&attempt);
    let responder = async {
        let ticket = receive_ticket(&mut outbound, "runner_first").await;
        assert_eq!(ticket.claims().command, TaskCommand::Execute);
        let aster_runner_protocol::TaskAuthorization::Model {
            subject,
            resource,
            license,
        } = &ticket.claims().authorization
        else {
            panic!("expected model authorization");
        };
        assert_eq!(subject.identity_id, f.identity.id);
        assert_eq!(
            subject.api_key_id,
            consumer.revalidate(&f.state).await.unwrap().api_key.id
        );
        assert_eq!(subject.request_id, request_id.as_str());
        assert_eq!(subject.reservation_id, reservation.id);
        assert_eq!(subject.reserved_tokens, reservation.reserved_tokens);
        assert!(ticket.claims().expires_at <= subject.reservation_expires_at);
        assert_eq!(resource.account_id, "account_test");
        assert_eq!(resource.public_model, MODEL);
        assert_eq!(resource.upstream_model, MODEL);
        let current_license = f.state.current_license().unwrap().unwrap();
        assert_eq!(license.license_id, current_license.as_ref().license_id());
        assert_eq!(license.license_sha256, sha256_hex(current_license.source()));
        respond(&f.state, &ticket.claims().task_id).await;
    };
    let (result, ()) = tokio::join!(execution, responder);
    assert_eq!(result.unwrap().body, b"response");
    assert_eq!(f.reserved(), 1);
}

#[tokio::test]
async fn model_scope_rejects_cross_request_released_reservation_and_payload_substitution() {
    let f = fixture().await;
    let (_, mut outbound) = f.state.runner_hub.register("runner_first").await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    let request_id = gateway_request_id(&HeaderMap::new()).unwrap();
    let other_id = gateway_request_id(&HeaderMap::new()).unwrap();
    let reservation = f
        .state
        .reserve_model_quota(&consumer, &request_id, 1)
        .await
        .unwrap();
    let operation = operation();
    assert!(matches!(
        ModelExecution::new(&consumer, &reservation, &other_id, MODEL, &operation)
            .prepare(&f.state, &HashSet::new(), None)
            .await,
        Err(ControlError::ExternalApiKeyInvalid)
    ));
    let scope = ModelExecution::new(&consumer, &reservation, &request_id, MODEL, &operation);
    let attempt = scope
        .prepare(&f.state, &HashSet::new(), None)
        .await
        .unwrap();
    assert!(matches!(
        f.state
            .dispatch_runner_task_excluding(
                None,
                attempt.binding(),
                RunnerTaskAuthorization::Model(&attempt),
                b"substituted",
                &HashSet::new()
            )
            .await,
        Err(RunnerDispatchFailure {
            error: ControlError::GatewayRequestInvalid,
            ..
        })
    ));
    let payload = serde_json::to_vec(attempt.request()).unwrap();
    let mut wrong_binding = attempt.binding().clone();
    wrong_binding.credential_revision = Some(100);
    assert!(matches!(
        f.state
            .dispatch_runner_task_excluding(
                None,
                &wrong_binding,
                RunnerTaskAuthorization::Model(&attempt),
                &payload,
                &HashSet::new()
            )
            .await,
        Err(RunnerDispatchFailure {
            error: ControlError::GatewayRequestInvalid,
            ..
        })
    ));
    f.state.release_model_quota(&reservation).await.unwrap();
    assert!(matches!(
        f.state.execute_runner_http_request(&attempt).await,
        Err(ControlError::QuotaSettlementConflict)
    ));
    assert!(outbound.try_recv().is_err());
}

#[tokio::test]
async fn runner_retry_rechecks_revoked_model_consumer_before_second_dispatch() {
    let f = fixture().await;
    let (_, mut first) = f.state.runner_hub.register("runner_first").await;
    let (_, mut second) = f.state.runner_hub.register("runner_second").await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    let request_id = gateway_request_id(&HeaderMap::new()).unwrap();
    let reservation = f
        .state
        .reserve_model_quota(&consumer, &request_id, 1)
        .await
        .unwrap();
    let operation = operation();
    let scope = ModelExecution::new(&consumer, &reservation, &request_id, MODEL, &operation);
    let attempt = scope
        .prepare(&f.state, &HashSet::new(), None)
        .await
        .unwrap();
    let execution = f.state.start_runner_http_request(
        Some("runner_first"),
        attempt.binding(),
        RunnerTaskAuthorization::Model(&attempt),
        attempt.request(),
    );
    let revoke = async {
        let ticket = receive_ticket(&mut first, "runner_first").await;
        f.revoke_key();
        f.state
            .runner_hub
            .publish(RunnerToControl::TaskFailed(
                aster_runner_protocol::TaskFailure {
                    task_id: ticket.claims().task_id.clone(),
                    category: "capacity".to_owned(),
                    retryable_before_upstream: true,
                },
            ))
            .await;
    };
    let (result, ()) = tokio::join!(execution, revoke);
    assert!(matches!(result, Err(ControlError::ExternalApiKeyInvalid)));
    assert!(second.try_recv().is_err());
}

#[tokio::test]
async fn prepared_model_attempt_cannot_survive_a_license_feature_change() {
    let f = fixture().await;
    let (_, mut outbound) = f.state.runner_hub.register("runner_first").await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    let request_id = gateway_request_id(&HeaderMap::new()).unwrap();
    let reservation = f
        .state
        .reserve_model_quota(&consumer, &request_id, 1)
        .await
        .unwrap();
    let operation = operation();
    let scope = ModelExecution::new(&consumer, &reservation, &request_id, MODEL, &operation);
    let attempt = scope
        .prepare(&f.state, &HashSet::new(), None)
        .await
        .unwrap();
    let replacement = license(vec![CapabilityId::Runner], true);
    f.state
        .license_state
        .as_ref()
        .unwrap()
        .accept_replacement(&replacement, (f.state.now)())
        .unwrap();
    f.state.replace_license(replacement).unwrap();
    assert!(matches!(
        f.state.execute_runner_http_request(&attempt).await,
        Err(ControlError::Policy(_))
    ));
    assert!(outbound.try_recv().is_err());
}

async fn fill_queue(state: &ControlState, runner: &str) -> mpsc::Sender<Vec<u8>> {
    let sender = state
        .runner_hub
        .inner
        .connections
        .lock()
        .await
        .get(runner)
        .unwrap()
        .sender
        .clone();
    while sender.capacity() > 0 {
        sender.try_send(Vec::new()).unwrap();
    }
    sender
}

async fn wait_for_dispatch_queue(sender: &mpsc::Sender<Vec<u8>>) {
    // The hub and this observer own two references. The dispatch path owns two
    // more while reserve_owned waits for capacity; no scheduler delay is assumed.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while sender.strong_count() < 4 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(sender.capacity(), 0);
}

#[tokio::test]
async fn obsolete_or_closed_runner_slot_never_receives_a_new_ticket() {
    let hub = RunnerHub::default();
    let (_, mut old) = hub.register("runner_test").await;
    let old_slot = hub.reserve_dispatch("runner_test").await.unwrap();
    let (_, mut current) = hub.register("runner_test").await;
    assert!(matches!(
        hub.dispatch_reserved(old_slot, "task_old", vec![1]).await,
        Err(RunnerDispatchFailure {
            error: ControlError::Routing(RoutingError::NoRunnerReady),
            excluded_runner_id: None
        })
    ));
    assert!(old.try_recv().is_err());
    assert!(current.try_recv().is_err());
    assert!(hub.inner.tasks.lock().await.is_empty());
    let slot = hub.reserve_dispatch("runner_test").await.unwrap();
    current.close();
    assert!(matches!(
        hub.dispatch_reserved(slot, "task_closed", vec![2]).await,
        Err(RunnerDispatchFailure {
            error: ControlError::Routing(RoutingError::NoRunnerReady),
            excluded_runner_id: Some(_)
        })
    ));
    assert!(hub.inner.tasks.lock().await.is_empty());
}

#[tokio::test]
async fn gateway_license_change_during_queue_wait_releases_quota_without_unknown_audit() {
    let f = fixture().await;
    enable_monetary_gateway(&f).await;
    let (_, mut outbound) = f.state.runner_hub.register("runner_first").await;
    let sender = fill_queue(&f.state, "runner_first").await;
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", format!("Bearer {}", f.token))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({"model":MODEL,"input":"hello"})).unwrap(),
        ))
        .unwrap();
    let response = router(f.state.clone()).oneshot(request);
    let change = async {
        wait_for_dispatch_queue(&sender).await;
        let replacement = license(vec![CapabilityId::Runner], true);
        // This succeeds while queue capacity is unavailable: no mutation lease
        // may be held across that wait.
        f.state
            .license_state
            .as_ref()
            .unwrap()
            .accept_replacement(&replacement, (f.state.now)())
            .unwrap();
        f.state.replace_license(replacement).unwrap();
        assert!(outbound.recv().await.unwrap().is_empty());
    };
    let (response, ()) = tokio::join!(response, change);
    assert_eq!(response.unwrap().status(), StatusCode::FORBIDDEN);
    assert_eq!(f.reserved(), 0);
    while let Ok(frame) = outbound.try_recv() {
        assert!(frame.is_empty());
    }
    assert!(
        !f.state
            .verified_audit_events()
            .await
            .unwrap()
            .iter()
            .any(|event| event.action == "gateway.request.unbilled_upstream_unknown")
    );
}

#[tokio::test]
async fn provider_rejects_stale_queued_attempt_without_switching_accounts() {
    let f = fixture().await;
    {
        let mut store = f.store.lock().unwrap();
        store
            .insert_upstream_account_unchecked(
                "account_z",
                "openai",
                "subject_z",
                "z@example.test",
                NOW,
            )
            .unwrap();
        store
            .replace_account_models(
                "account_z",
                &[DiscoveredModel {
                    id: "model_z".to_owned(),
                    public_name: MODEL.to_owned(),
                    upstream_name: MODEL.to_owned(),
                    display_name: "Test".to_owned(),
                }],
                NOW,
            )
            .unwrap();
    }
    f.state.create_credential_instance("account_z", b"refresh-z", &serde_json::to_vec(&json!({
        "schema":OPENAI_CREDENTIAL_SCHEMA, "provider":"openai", "access_token":"test-z", "refresh_token":"refresh-z",
        "id_token":"", "expires_at":"2026-09-08T00:00:00.000Z", "account_id":"subject_z", "email":"z@example.test", "plan":"plus"
    })).unwrap(), "2026-09-08T00:00:00.000Z").await.unwrap();
    let (_, mut outbound) = f.state.runner_hub.register("runner_first").await;
    assert!(
        f.store
            .lock()
            .unwrap()
            .update_account_last_success_runner("account_test", "runner_first", NOW)
            .unwrap()
    );
    let sender = fill_queue(&f.state, "runner_first").await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    let request_id = gateway_request_id(&HeaderMap::new()).unwrap();
    let reservation = f
        .state
        .reserve_model_quota(&consumer, &request_id, 1)
        .await
        .unwrap();
    let operation = operation();
    let scope = ModelExecution::new(&consumer, &reservation, &request_id, MODEL, &operation);
    let first_attempt = scope
        .prepare(&f.state, &HashSet::new(), None)
        .await
        .unwrap();
    assert_eq!(first_attempt.candidate().account_id, "account_test");
    let response = execute_provider_request(&f.state, &scope);
    let change = async {
        wait_for_dispatch_queue(&sender).await;
        f.store
            .lock()
            .unwrap()
            .update_upstream_account_status("account_test", "disabled", NOW)
            .unwrap();
        assert!(matches!(
            RunnerTaskAuthorization::Model(&first_attempt)
                .authorize(
                    &f.state,
                    first_attempt.binding(),
                    &serde_json::to_vec(first_attempt.request()).unwrap()
                )
                .await,
            Err(ControlError::ModelAttemptStale)
        ));
        for _ in 0..128 {
            assert!(outbound.recv().await.unwrap().is_empty());
        }
    };
    let (response, ()) = tokio::join!(response, change);
    assert!(matches!(response, Err(ControlError::ModelAttemptStale)));
    assert!(
        outbound.try_recv().is_err(),
        "stale attempt dispatched a task"
    );
    assert_eq!(
        scope
            .prepare(&f.state, &HashSet::new(), None)
            .await
            .unwrap()
            .candidate()
            .account_id,
        "account_z",
        "a usable alternative exists but the provider must not retry automatically"
    );
}

#[tokio::test]
async fn model_dispatch_reselects_same_runner_after_reconnect_before_enqueue() {
    let f = fixture().await;
    let (_, mut old) = f.state.runner_hub.register("runner_first").await;
    let sender = fill_queue(&f.state, "runner_first").await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    let request_id = gateway_request_id(&HeaderMap::new()).unwrap();
    let reservation = f
        .state
        .reserve_model_quota(&consumer, &request_id, 1)
        .await
        .unwrap();
    let operation = operation();
    let scope = ModelExecution::new(&consumer, &reservation, &request_id, MODEL, &operation);
    let attempt = scope
        .prepare(&f.state, &HashSet::new(), None)
        .await
        .unwrap();
    let execution = f.state.execute_runner_http_request(&attempt);
    let reconnect = async {
        wait_for_dispatch_queue(&sender).await;
        let (_, mut current) = f.state.runner_hub.register("runner_first").await;
        assert!(old.recv().await.unwrap().is_empty());
        let ticket = receive_ticket(&mut current, "runner_first").await;
        assert_eq!(ticket.claims().command, TaskCommand::Execute);
        respond(&f.state, &ticket.claims().task_id).await;
        while let Ok(frame) = old.try_recv() {
            assert!(frame.is_empty(), "obsolete connection received a task");
        }
        assert!(current.try_recv().is_err(), "duplicate task dispatched");
    };
    let (result, ()) = tokio::join!(execution, reconnect);
    assert_eq!(result.unwrap().body, b"response");
}

#[tokio::test]
async fn model_dispatch_excludes_closed_runner_before_retry() {
    let f = fixture().await;
    let (_, mut first) = f.state.runner_hub.register("runner_first").await;
    let (_, mut second) = f.state.runner_hub.register("runner_second").await;
    let sender = fill_queue(&f.state, "runner_first").await;
    let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
    let request_id = gateway_request_id(&HeaderMap::new()).unwrap();
    let reservation = f
        .state
        .reserve_model_quota(&consumer, &request_id, 1)
        .await
        .unwrap();
    let operation = operation();
    let scope = ModelExecution::new(&consumer, &reservation, &request_id, MODEL, &operation);
    let attempt = scope
        .prepare(&f.state, &HashSet::new(), None)
        .await
        .unwrap();
    let execution = f.state.start_runner_http_request(
        Some("runner_first"),
        attempt.binding(),
        RunnerTaskAuthorization::Model(&attempt),
        attempt.request(),
    );
    let close = async {
        wait_for_dispatch_queue(&sender).await;
        first.close();
        let ticket = receive_ticket(&mut second, "runner_second").await;
        respond(&f.state, &ticket.claims().task_id).await;
        while let Ok(frame) = first.try_recv() {
            assert!(frame.is_empty());
        }
    };
    let (result, ()) = tokio::join!(execution, close);
    assert_eq!(result.unwrap().runner_id, "runner_second");
}

#[tokio::test]
async fn ledger_chain_index_preserves_full_history_integrity_checks() {
    let f = Fixture::new().await;
    f.state
        .grant_model_quota(&f.identity.id, "index-test-one", 1, "test")
        .await
        .unwrap();
    f.state
        .grant_model_quota(&f.identity.id, "index-test-two", 1, "test")
        .await
        .unwrap();
    let snapshot = f
        .state
        .verified_quota_snapshot(&f.identity.id)
        .await
        .unwrap();
    let verify = |entries: &[QuotaLedgerEntry]| {
        f.state
            .verify_quota_ledger(&f.identity.id, &snapshot.balance, entries)
    };
    let mut entries = snapshot.ledger_entries;
    assert!(entries.len() >= 3);
    entries.reverse();
    verify(&entries).unwrap();
    let first = entries
        .iter()
        .position(|entry| entry.previous_entry_hmac.is_empty())
        .unwrap();
    let mut duplicate = entries.clone();
    duplicate.push(entries[first].clone());
    assert!(matches!(
        verify(&duplicate),
        Err(ControlError::DataIntegrityInvalid)
    ));
    let mut fork = entries.clone();
    let mut branch = entries[first].clone();
    branch.id = "valid-signed-fork".to_owned();
    branch.integrity_hmac = f.state.ledger_integrity_hmac(&branch).unwrap();
    fork.push(branch);
    assert!(matches!(
        verify(&fork),
        Err(ControlError::DataIntegrityInvalid)
    ));
    let mut missing_root = entries.clone();
    missing_root.remove(first);
    assert!(matches!(
        verify(&missing_root),
        Err(ControlError::DataIntegrityInvalid)
    ));
    let mut tampered = entries.clone();
    tampered[first].description = "changed-without-signature".to_owned();
    assert!(matches!(
        verify(&tampered),
        Err(ControlError::DataIntegrityInvalid)
    ));
    let mut wrong_tail = snapshot.balance.clone();
    wrong_tail.last_ledger_hmac = "wrong-tail".to_owned();
    assert!(matches!(
        f.state
            .verify_quota_ledger(&f.identity.id, &wrong_tail, &entries),
        Err(ControlError::DataIntegrityInvalid)
    ));
}

#[tokio::test]
async fn gateway_key_integrity_failure_before_dispatch_has_no_unknown_execution_audit() {
    let f = fixture().await;
    enable_monetary_gateway(&f).await;
    let (_, mut outbound) = f.state.runner_hub.register("runner_first").await;
    let sender = fill_queue(&f.state, "runner_first").await;
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", format!("Bearer {}", f.token))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({"model":MODEL,"input":"hello"})).unwrap(),
        ))
        .unwrap();
    let response = router(f.state.clone()).oneshot(request);
    let tamper = async {
        wait_for_dispatch_queue(&sender).await;
        assert!(
            f.store
                .lock()
                .unwrap()
                .update_api_key_status(
                    &f.key.id,
                    &f.identity.id,
                    f.key.revision,
                    "active",
                    "invalid-hmac"
                )
                .unwrap()
        );
        assert!(matches!(
            f.state.authorize_model_consumer(&f.token).await,
            Err(ControlError::DataIntegrityInvalid)
        ));
        assert!(outbound.recv().await.unwrap().is_empty());
    };
    let (response, ()) = tokio::join!(response, tamper);
    assert_eq!(response.unwrap().status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(f.reserved(), 0);
    while let Ok(frame) = outbound.try_recv() {
        assert!(frame.is_empty());
    }
    let audit = f.state.verified_audit_events().await.unwrap();
    assert!(
        !audit
            .iter()
            .any(|event| event.action == "gateway.request.unbilled_upstream_unknown")
    );
}

#[tokio::test]
async fn dispatched_model_timeout_or_lost_events_never_fail_over() {
    for timeout in [false, true] {
        let mut keep_events_open = None;
        let f = fixture().await;
        let (_, mut outbound) = f.state.runner_hub.register("runner_first").await;
        let (_, mut second) = f.state.runner_hub.register("runner_second").await;
        let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
        let request_id = gateway_request_id(&HeaderMap::new()).unwrap();
        let reservation = f
            .state
            .reserve_model_quota(&consumer, &request_id, 1)
            .await
            .unwrap();
        let operation = operation();
        let scope = ModelExecution::new(&consumer, &reservation, &request_id, MODEL, &operation);
        let execution = execute_provider_request(&f.state, &scope);
        let interrupt = async {
            let ticket = receive_ticket(&mut outbound, "runner_first").await;
            // No lifecycle event has arrived. Absence of UpstreamStarted is not
            // evidence that work was never started after a signed dispatch.
            if timeout {
                // Keep the channel alive beyond the hub's 600-second cleanup,
                // so this branch proves the timeout, not the closed-stream case.
                keep_events_open = Some(
                    f.state
                        .runner_hub
                        .inner
                        .tasks
                        .lock()
                        .await
                        .get(&ticket.claims().task_id)
                        .unwrap()
                        .clone(),
                );
                tokio::time::pause();
                tokio::time::advance(
                    RUNNER_EXECUTE_TASK_TIMEOUT + std::time::Duration::from_secs(1),
                )
                .await;
            } else {
                f.state
                    .runner_hub
                    .inner
                    .tasks
                    .lock()
                    .await
                    .remove(&ticket.claims().task_id);
            }
            ticket.claims().task_id.clone()
        };
        let (result, dispatched_id) = tokio::join!(execution, interrupt);
        if timeout {
            tokio::time::resume();
        }
        drop(keep_events_open);
        assert!(matches!(
            result,
            Err(ControlError::Routing(RoutingError::RetryForbidden))
        ));
        let cancellation = decode_control_frame(&outbound.try_recv().unwrap()).unwrap();
        assert!(
            matches!(cancellation, ControlToRunner::CancelTask(TaskLifecycle { task_id }) if task_id == dispatched_id)
        );
        assert!(
            outbound.try_recv().is_err(),
            "no second dispatch to the original Runner"
        );
        assert!(
            second.try_recv().is_err(),
            "uncertain upstream work must never fail over"
        );
    }
}

#[tokio::test]
async fn model_provider_retry_obeys_explicit_runner_failure_permission() {
    for retryable in [false, true] {
        let f = fixture().await;
        let (_, mut first) = f.state.runner_hub.register("runner_first").await;
        let (_, mut second) = f.state.runner_hub.register("runner_second").await;
        let consumer = f.state.authorize_model_consumer(&f.token).await.unwrap();
        let request_id = gateway_request_id(&HeaderMap::new()).unwrap();
        let reservation = f
            .state
            .reserve_model_quota(&consumer, &request_id, 1)
            .await
            .unwrap();
        let operation = operation();
        let scope = ModelExecution::new(&consumer, &reservation, &request_id, MODEL, &operation);
        let execution = execute_provider_request(&f.state, &scope);
        let fail = async {
            let ticket = receive_ticket(&mut first, "runner_first").await;
            f.state
                .runner_hub
                .publish(RunnerToControl::TaskFailed(
                    aster_runner_protocol::TaskFailure {
                        task_id: ticket.claims().task_id.clone(),
                        category: "test-failure".to_owned(),
                        retryable_before_upstream: retryable,
                    },
                ))
                .await;
            if retryable {
                let retried = receive_ticket(&mut second, "runner_second").await;
                assert_ne!(retried.claims().task_id, ticket.claims().task_id);
                respond(&f.state, &retried.claims().task_id).await;
            }
        };
        let (result, ()) = tokio::join!(execution, fail);
        if retryable {
            let (_, response) = result.unwrap();
            assert_eq!(response.runner_id, "runner_second");
            assert_eq!(response.body, b"response");
        } else {
            assert!(matches!(
                result,
                Err(ControlError::Routing(RoutingError::RetryForbidden))
            ));
        }
        assert!(first.try_recv().is_err());
        assert!(second.try_recv().is_err());
    }
}
