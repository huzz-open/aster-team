//! A prerequisite of candidate readiness, not a complete readiness verdict.
use super::*;
use aster_runner_protocol::{PROBE_HOST, PROBE_PROVIDER, PROBE_TIMEOUT_MS};

impl ControlState {
    pub(crate) async fn probe_runner(&self, runner_id: &str) -> Result<(), ControlError> {
        let task_id = random_identifier("probe")?;
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(u64::from(PROBE_TIMEOUT_MS)),
            async {
                authorize_non_consuming_feature(self, "runner")?;
                let runners = self
                    .credential_storage()?
                    .list_enabled_runners()
                    .await?
                    .into_iter()
                    .filter(|runner| runner.id == runner_id)
                    .filter_map(runner_snapshot)
                    .collect::<Vec<_>>();
                // Require this exact runner, including its protocol, heartbeat and
                // capacity. Another slot's healthy runner cannot satisfy the probe.
                select_runner(
                    &runners,
                    None,
                    RUNNER_PROTOCOL_VERSION,
                    (self.now)(),
                    RUNNER_HEARTBEAT_TIMEOUT,
                )
                .map_err(ControlError::Routing)?;
                let issuer = self
                    .task_issuer
                    .as_ref()
                    .ok_or(ControlError::InvalidUpstreamResponse)?;
                self.runner_hub
                    .probe(issuer, runner_id, &task_id, (self.now)())
                    .await
            },
        )
        .await
        .unwrap_or(Err(ControlError::Routing(RoutingError::NoRunnerReady)));
        // A timed out probe must not leave a task entry until the generic
        // 600-second cleanup. Cancellation remains bound to the original socket.
        self.runner_hub.inner.tasks.lock().await.remove(&task_id);
        result
    }
}

impl RunnerHub {
    async fn probe(
        &self,
        issuer: &RunnerTaskIssuer,
        runner_id: &str,
        task_id: &str,
        now: OffsetDateTime,
    ) -> Result<(), ControlError> {
        let generation = self
            .inner
            .connections
            .lock()
            .await
            .get(runner_id)
            .map(|connection| connection.generation)
            .ok_or(ControlError::Routing(RoutingError::NoRunnerReady))?;
        let mut challenge = [0; 32];
        getrandom::fill(&mut challenge).map_err(|_| ControlError::InvalidUpstreamResponse)?;
        let nonce = random_identifier("nonce")?;
        let ticket = issue_task_ticket(TaskTicketIssue {
            key_id: &issuer.key_id,
            signing_key: &issuer.signing_key,
            task_id,
            runner_id,
            provider_id: PROBE_PROVIDER,
            credential_instance_id: None,
            credential_revision: None,
            upstream_host: PROBE_HOST,
            command: TaskCommand::Probe,
            authorization: aster_runner_protocol::TaskAuthorization::Probe {},
            payload: &challenge,
            nonce: &nonce,
            now,
            execution_timeout_ms: PROBE_TIMEOUT_MS,
        })
        .map_err(ControlError::RunnerTicket)?;
        let ticket_json =
            serde_json::to_vec(&ticket).map_err(|_| ControlError::InvalidUpstreamResponse)?;
        let wire = encode_control_frame(&ControlToRunner::Task(TaskFrame {
            ticket_json_base64url: encode_base64url(&ticket_json),
            payload_base64url: encode_base64url(&challenge),
        }))
        .map_err(ControlError::RunnerTicket)?;
        let (mut events, _cancel) = self.dispatch(runner_id, task_id, wire).await?;
        if !matches!(events.recv().await, Some(RunnerToControl::TaskAccepted(frame)) if frame.task_id == task_id)
        {
            return Err(ControlError::InvalidUpstreamResponse);
        }
        match events.recv().await {
            Some(RunnerToControl::TaskChunk(frame))
                if frame.task_id == task_id
                    && frame.sequence == 0
                    && decode_base64url(&frame.data_base64url)
                        .is_ok_and(|data| data == challenge) => {}
            _ => return Err(ControlError::InvalidUpstreamResponse),
        }
        if !matches!(events.recv().await, Some(RunnerToControl::TaskFinished(frame)) if frame.task_id == task_id && frame.status == 204 && frame.usage_json.is_none())
        {
            return Err(ControlError::InvalidUpstreamResponse);
        }
        if self
            .inner
            .connections
            .lock()
            .await
            .get(runner_id)
            .is_none_or(|connection| connection.generation != generation)
        {
            return Err(ControlError::Routing(RoutingError::NoRunnerReady));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aster_runner_protocol::{
        TaskChunk, TaskLifecycle, TaskResult, TrustedTaskKeys, verify_task_ticket,
    };

    #[tokio::test]
    async fn probe_requires_signed_challenge_and_ordered_terminal_from_selected_connection() {
        for (valid, reconnect) in [(true, false), (false, false), (true, true)] {
            let hub = RunnerHub::default();
            let (generation, mut wire) = hub.register("runner-probe-test").await;
            let (_, mut other) = hub.register("runner-other-test").await;
            let key = SigningKey::from_bytes(&[39; 32]);
            let issuer = RunnerTaskIssuer::new("probe-key", key.clone());
            let now = OffsetDateTime::now_utc();
            let peer_hub = hub.clone();
            let peer = tokio::spawn(async move {
                let ControlToRunner::Task(frame) =
                    aster_runner_protocol::decode_control_frame(&wire.recv().await.unwrap())
                        .unwrap()
                else {
                    panic!("expected task");
                };
                let payload = decode_base64url(&frame.payload_base64url).unwrap();
                let ticket = decode_base64url(&frame.ticket_json_base64url).unwrap();
                let mut keys = TrustedTaskKeys::new();
                keys.insert("probe-key", key.verifying_key()).unwrap();
                let verified =
                    verify_task_ticket(&ticket, &keys, "runner-probe-test", &payload, now).unwrap();
                assert_eq!(verified.claims().command, TaskCommand::Probe);
                assert!(verified.claims().credential_instance_id.is_none());
                assert_eq!(payload.len(), 32);
                let id = verified.claims().task_id.clone();
                peer_hub
                    .publish(RunnerToControl::TaskAccepted(TaskLifecycle {
                        task_id: id.clone(),
                    }))
                    .await;
                let echoed = if valid { payload } else { vec![0; 32] };
                peer_hub
                    .publish(RunnerToControl::TaskChunk(TaskChunk {
                        task_id: id.clone(),
                        sequence: 0,
                        data_base64url: encode_base64url(&echoed),
                    }))
                    .await;
                let replacement = if reconnect {
                    let replacement = peer_hub.register("runner-probe-test").await.1;
                    // The old WebSocket exits and unregisters its own generation.
                    // Its terminal message cannot be delivered through the replacement.
                    peer_hub.unregister("runner-probe-test", generation).await;
                    Some(replacement)
                } else {
                    None
                };
                peer_hub
                    .publish(RunnerToControl::TaskFinished(TaskResult {
                        task_id: id,
                        status: 204,
                        usage_json: None,
                    }))
                    .await;

                // Keep receivers alive until the result is consumed.
                (wire, replacement)
            });
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                hub.probe(&issuer, "runner-probe-test", "probe-test", now),
            )
            .await
            .unwrap();
            assert_eq!(result.is_ok(), valid && !reconnect);
            let _wire = peer.await.unwrap();
            assert!(other.try_recv().is_err());
            assert!(hub.inner.tasks.lock().await.is_empty());
        }
    }

    #[tokio::test]
    async fn peer_exit_or_wrong_first_frame_cannot_pass_probe() {
        for send_wrong_frame in [true, false] {
            let hub = RunnerHub::default();
            let (generation, mut wire) = hub.register("runner-probe-test").await;
            let issuer = RunnerTaskIssuer::new("probe-key", SigningKey::from_bytes(&[39; 32]));
            let peer_hub = hub.clone();
            let peer = tokio::spawn(async move {
                wire.recv().await.unwrap();
                if send_wrong_frame {
                    peer_hub
                        .publish(RunnerToControl::UpstreamStarted(TaskLifecycle {
                            task_id: "probe-test".into(),
                        }))
                        .await;
                } else {
                    peer_hub.unregister("runner-probe-test", generation).await;
                }
                wire
            });
            assert!(
                tokio::time::timeout(
                    std::time::Duration::from_secs(1),
                    hub.probe(
                        &issuer,
                        "runner-probe-test",
                        "probe-test",
                        OffsetDateTime::now_utc()
                    )
                )
                .await
                .unwrap()
                .is_err()
            );
            let mut wire = peer.await.unwrap();
            if send_wrong_frame {
                assert!(matches!(
                    aster_runner_protocol::decode_control_frame(&wire.recv().await.unwrap())
                        .unwrap(),
                    ControlToRunner::CancelTask(_)
                ));
            }
        }
    }
    #[tokio::test]
    async fn probe_checks_exact_runner_eligibility_and_cancels_after_timeout() {
        let directory = tempfile::tempdir().unwrap();
        let now = time::macros::datetime!(2026-08-28 0:00:01 UTC);
        let license = crate::tests::verified_license();
        let license_state = Arc::new(
            LicenseStateStore::new(directory.path().join("license-state"), &[51; 32]).unwrap(),
        );
        license_state.initialize(&license, now).unwrap();
        let runner = RunnerRecord {
            id: "runner-probe-test".into(),
            enabled: true,
            version: "0.1.0".into(),
            protocol_version: RUNNER_PROTOCOL_VERSION,
            max_inflight: 4,
            inflight: 0,
            recent_request_count: 0,
            recent_error_count: 0,
            latency_ms: 1,
            last_seen_at: Some("2026-08-28T00:00:00.000Z".into()),
        };
        let state = ControlState::new("0.1.0", Some(license))
            .with_license_state(license_state)
            .with_task_issuer(RunnerTaskIssuer::new(
                "probe-key",
                SigningKey::from_bytes(&[39; 32]),
            ))
            .with_now(now)
            .with_test_runners(3, vec![runner.clone()]);
        let (_, mut wire) = state.runner_hub.register(&runner.id).await;
        let (_, mut other_wire) = state.runner_hub.register("runner-other").await;
        for case in 0..3 {
            let mut unavailable = runner.clone();
            match case {
                0 => unavailable.protocol_version = RUNNER_PROTOCOL_VERSION - 1,
                1 => unavailable.last_seen_at = None,
                _ => unavailable.inflight = unavailable.max_inflight,
            }
            let unavailable_state = state.clone().with_test_runners(3, vec![unavailable]);
            assert!(unavailable_state.probe_runner(&runner.id).await.is_err());
            assert!(wire.try_recv().is_err());
            assert!(other_wire.try_recv().is_err());
        }
        assert!(state.probe_runner("missing-runner").await.is_err());
        assert!(wire.try_recv().is_err());
        let started = std::time::Instant::now();
        assert!(state.probe_runner(&runner.id).await.is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        assert!(matches!(
            aster_runner_protocol::decode_control_frame(&wire.recv().await.unwrap()).unwrap(),
            ControlToRunner::Task(_)
        ));
        assert!(matches!(
            aster_runner_protocol::decode_control_frame(&wire.recv().await.unwrap()).unwrap(),
            ControlToRunner::CancelTask(_)
        ));
        assert!(state.runner_hub.inner.tasks.lock().await.is_empty());
        assert!(other_wire.try_recv().is_err());
    }
}
