//! Authenticated loopback client. Never put the bearer in process arguments or errors.
use std::{
    fs,
    io::Read as _,
    time::{Duration, Instant},
};

use aster_error_catalog::delivery;
use aster_install_layout::InstallLayout;
use aster_upgrade_core::{
    ActiveReleaseSlot,
    runtime::{
        DrainProgress, ReadinessExpectation, ReadinessModelInventory, RuntimeSnapshot,
        SlotRetirement, SlotRuntime,
    },
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{blocking::Client, header::HeaderValue};
use serde::Deserialize;
use zeroize::Zeroizing;

use super::CliFailure;

#[derive(Deserialize)]
struct ReadinessReport {
    schema: String,
    instance_id: String,
    slot: aster_upgrade_core::ReleaseSlot,
    revision: u64,
    manifest_sha256: String,
    ready: bool,
    checks: ReadinessChecks,
    readiness_token: Option<String>,
    valid_for_seconds: u64,
}

#[derive(Deserialize)]
struct ReadinessChecks {
    platform: bool,
    dependencies: DependencyChecks,
    runners: bool,
    assets: bool,
}

#[derive(Deserialize)]
struct DependencyChecks {
    database_read_write: bool,
    business_state: bool,
    model_routes: bool,
}

// Deliberately neither Clone nor Debug: one checked proof authorizes one attempt.
pub(super) struct ReadyPermit {
    observed: RuntimeSnapshot,
    token: Zeroizing<String>,
    expires: Instant,
}

#[derive(Deserialize)]
struct Probe {
    schema: String,
    instance_id: String,
    runner_id: String,
    passed: bool,
}

fn failed(message: &str) -> CliFailure {
    CliFailure::new(delivery::UPGRADE_FAILED, message)
}

pub(super) struct RuntimeClient {
    client: Client,
    base: String,
    authorization: HeaderValue,
    installation_id: String,
    binding: ActiveReleaseSlot,
}

impl RuntimeClient {
    pub(super) fn new(
        layout: &InstallLayout,
        active: &ActiveReleaseSlot,
    ) -> Result<Self, CliFailure> {
        let token_path = layout.control_slot_runtime_token(active.slot.id());
        let metadata = fs::symlink_metadata(&token_path)
            .map_err(|_| failed("private runtime token is unavailable"))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != 32 {
            return Err(failed("private runtime token is invalid"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            if metadata.permissions().mode() & 0o027 != 0 {
                return Err(failed("private runtime token has unsafe permissions"));
            }
        }
        let mut token = Zeroizing::new(Vec::new());
        fs::File::open(token_path)
            .and_then(|file| file.take(33).read_to_end(&mut token))
            .map_err(|_| failed("private runtime token could not be read"))?;
        if token.len() != 32 {
            return Err(failed("private runtime token is invalid"));
        }
        let encoded = Zeroizing::new(format!(
            "Bearer {}",
            URL_SAFE_NO_PAD.encode(token.as_slice())
        ));
        let mut authorization = HeaderValue::from_str(&encoded)
            .map_err(|_| failed("private runtime authorization is invalid"))?;
        authorization.set_sensitive(true);
        #[derive(Deserialize)]
        struct Profile {
            installation_id: String,
        }
        let source = fs::read(layout.installation_profile())
            .map_err(|_| failed("installation profile is unavailable"))?;
        let profile: Profile = serde_json::from_slice(&source)
            .map_err(|_| failed("installation profile is invalid"))?;
        if profile.installation_id.is_empty() {
            return Err(failed("installation identity is missing"));
        }
        Self::build(
            format!("http://127.0.0.1:{}", active.slot.runtime_port()),
            authorization,
            profile.installation_id,
            active,
        )
    }

    fn build(
        base: String,
        authorization: HeaderValue,
        installation_id: String,
        binding: &ActiveReleaseSlot,
    ) -> Result<Self, CliFailure> {
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(1))
            .timeout(Duration::from_secs(6))
            .build()
            .map_err(|_| failed("could not initialize private runtime client"))?;
        Ok(Self {
            client,
            base,
            authorization,
            installation_id,
            binding: binding.clone(),
        })
    }

    fn read<T: serde::de::DeserializeOwned>(
        &self,
        request: reqwest::blocking::RequestBuilder,
        deadline: Instant,
        request_budget: Duration,
    ) -> Result<T, CliFailure> {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| failed("private runtime deadline has expired"))?;
        let response = request
            .timeout(remaining.min(request_budget))
            .header(reqwest::header::AUTHORIZATION, self.authorization.clone())
            .send()
            .map_err(|_| failed("private runtime request failed"))?;
        if !response.status().is_success() {
            return Err(failed("private runtime rejected the request"));
        }
        let mut bytes = Zeroizing::new(Vec::new());
        response
            .take(16 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| failed("private runtime response could not be read"))?;
        if bytes.len() > 16 * 1024 {
            return Err(failed("private runtime response exceeded its size limit"));
        }
        serde_json::from_slice(&bytes).map_err(|_| failed("private runtime response is invalid"))
    }

    fn validate_observed(&self, observed: &RuntimeSnapshot) -> Result<(), CliFailure> {
        if observed.schema != "aster.control-runtime.v1"
            || observed.installation_id != self.installation_id
            || observed.slot != self.binding.slot
            || observed.product_version != self.binding.version
            || observed.instance_id.is_empty()
        {
            return Err(failed(
                "private runtime does not match the bound installation, slot and release",
            ));
        }
        Ok(())
    }

    fn admission(
        &self,
        observed: &RuntimeSnapshot,
        accepting: bool,
        token: Option<&str>,
        deadline: Instant,
    ) -> Result<RuntimeSnapshot, CliFailure> {
        self.validate_observed(observed)?;
        if observed.lifecycle.stopping {
            return Err(failed("a stopping instance cannot change admission"));
        }
        let next_revision = observed
            .lifecycle
            .revision
            .checked_add(1)
            .ok_or_else(|| failed("private runtime revision exhausted"))?;
        // Never automatically retry this mutation: the server may have applied
        // it even when the response is lost. Reconcile the same process first.
        let response: RuntimeSnapshot = self.read(self.client.post(format!("{}/v1/admission", self.base))
            .json(&serde_json::json!({"instance_id": observed.instance_id, "expected_revision": observed.lifecycle.revision, "accepting": accepting, "readiness_token": token})), deadline, Duration::from_secs(6))?;
        self.validate_observed(&response)?;
        if !response.same_process(observed)
            || response.lifecycle.revision != next_revision
            || response.lifecycle.accepting != accepting
            || response.lifecycle.stopping
        {
            return Err(failed(
                "private runtime admission outcome requires reconciliation",
            ));
        }
        Ok(response)
    }

    pub(super) fn probe_runner(&self, active: &ActiveReleaseSlot) -> Result<(), CliFailure> {
        if active != &self.binding {
            return Err(failed("Runner probe does not match the bound slot"));
        }
        let runner = active
            .local_runner
            .as_ref()
            .ok_or_else(|| failed("slot Runner identity is missing"))?;
        let deadline = Instant::now() + Duration::from_secs(12);
        let snapshot = self.status(deadline)?;
        if !snapshot.lifecycle.accepting || snapshot.lifecycle.stopping {
            return Err(failed(
                "private runtime is not accepting maintenance traffic",
            ));
        }
        let probe: Probe = self.read(self.client.post(format!("{}/v1/runner-probe", self.base))
            .json(&serde_json::json!({ "instance_id": snapshot.instance_id, "runner_id": runner.runner_id })), deadline, Duration::from_secs(6))?;
        if probe.schema != "aster.runner-probe.v1"
            || probe.instance_id != snapshot.instance_id
            || probe.runner_id != runner.runner_id
            || !probe.passed
        {
            return Err(failed("slot Runner signed probe did not pass"));
        }
        Ok(())
    }

    pub(super) fn configured_models(
        &self,
        observed: &RuntimeSnapshot,
        deadline: Instant,
    ) -> Result<Vec<String>, CliFailure> {
        self.validate_observed(observed)?;
        if observed.lifecycle.stopping {
            return Err(failed(
                "stopping runtime cannot supply candidate model coverage",
            ));
        }
        let inventory: ReadinessModelInventory = self.read(
            self.client.post(format!("{}/v1/readiness-models", self.base))
                .json(&serde_json::json!({"instance_id": observed.instance_id, "expected_revision": observed.lifecycle.revision})),
            deadline, Duration::from_secs(6),
        )?;
        if !inventory.valid_for(observed) {
            return Err(failed(
                "runtime model inventory is incomplete or belongs to another process",
            ));
        }
        Ok(inventory.models)
    }

    #[cfg(any(target_os = "linux", test))]
    pub(super) fn request_budget_ms(
        &self,
        observed: &RuntimeSnapshot,
        deadline: Instant,
    ) -> Result<u64, CliFailure> {
        self.validate_observed(observed)?;
        let budget: aster_upgrade_core::runtime::RuntimeRequestBudget = self.read(
            self.client.get(format!("{}/v1/status", self.base)),
            deadline,
            Duration::from_secs(6),
        )?;
        if !budget.valid_for(observed) {
            return Err(failed(
                "request budget does not belong to the pinned runtime",
            ));
        }
        Ok(budget.request_budget_ms)
    }
}

impl SlotRuntime for RuntimeClient {
    type Error = CliFailure;
    type ReadyPermit = ReadyPermit;

    fn status(&self, deadline: Instant) -> Result<RuntimeSnapshot, CliFailure> {
        let observed = self.read(
            self.client.get(format!("{}/v1/status", self.base)),
            deadline,
            Duration::from_secs(6),
        )?;
        self.validate_observed(&observed)?;
        Ok(observed)
    }

    fn readiness(
        &self,
        observed: &RuntimeSnapshot,
        expected: &ReadinessExpectation,
        deadline: Instant,
    ) -> Result<ReadyPermit, CliFailure> {
        self.validate_observed(observed)?;
        if observed.lifecycle.stopping || !expected.valid() {
            return Err(failed("invalid private runtime readiness request"));
        }
        if self.binding.local_runner.as_ref().is_none_or(|runner| {
            runner.manifest_sha256 != expected.manifest_sha256
                || !expected.runner_ids.contains(&runner.runner_id)
        }) {
            return Err(failed(
                "readiness coverage does not include the bound release and local Runner",
            ));
        }
        let models = self.configured_models(observed, deadline)?;
        let mut expected_models = expected.models.clone();
        expected_models.sort();
        if models != expected_models {
            return Err(failed(
                "readiness must cover every currently configured model",
            ));
        }
        let mut report: ReadinessReport = self.read(self.client.post(format!("{}/v1/readiness", self.base))
            .json(&serde_json::json!({"instance_id": observed.instance_id, "expected_revision": observed.lifecycle.revision, "manifest_sha256": expected.manifest_sha256, "models": expected.models, "runner_ids": expected.runner_ids})), deadline, Duration::from_secs(21))?;
        let token = Zeroizing::new(report.readiness_token.take().unwrap_or_default());
        if report.schema != "aster.control-readiness.v1"
            || report.instance_id != observed.instance_id
            || report.slot != observed.slot
            || report.revision != observed.lifecycle.revision
            || report.manifest_sha256 != expected.manifest_sha256
            || !report.ready
            || !report.checks.platform
            || !report.checks.runners
            || !report.checks.assets
            || !report.checks.dependencies.database_read_write
            || !report.checks.dependencies.business_state
            || !report.checks.dependencies.model_routes
            || token.is_empty()
            || token.len() > 160
            || !(1..=10).contains(&report.valid_for_seconds)
        {
            return Err(failed(
                "private runtime returned incomplete or mismatched readiness evidence",
            ));
        }
        Ok(ReadyPermit {
            observed: observed.clone(),
            token,
            expires: Instant::now() + Duration::from_secs(report.valid_for_seconds),
        })
    }

    fn open_admission(
        &self,
        permit: ReadyPermit,
        deadline: Instant,
    ) -> Result<RuntimeSnapshot, CliFailure> {
        if Instant::now() >= permit.expires {
            return Err(failed("private runtime readiness permit expired"));
        }
        self.admission(
            &permit.observed,
            true,
            Some(&permit.token),
            deadline.min(permit.expires),
        )
    }

    fn close_admission(
        &self,
        observed: &RuntimeSnapshot,
        deadline: Instant,
    ) -> Result<RuntimeSnapshot, CliFailure> {
        self.admission(observed, false, None, deadline)
    }

    fn observe_drain(
        &self,
        closed: &RuntimeSnapshot,
        deadline: Instant,
    ) -> Result<DrainProgress, CliFailure> {
        self.validate_observed(closed)?;
        if closed.lifecycle.accepting || closed.lifecycle.stopping {
            return Err(failed(
                "drain requires a successful close-admission observation",
            ));
        }
        let observed = self.status(deadline)?;
        if !observed.observes_drain(closed) {
            return Err(failed("draining process or admission revision changed"));
        }
        Ok(if observed.lifecycle.in_flight == 0 {
            DrainProgress::Drained
        } else {
            DrainProgress::Outstanding {
                count: observed.lifecycle.in_flight,
                oldest_request_age_ms: observed.lifecycle.oldest_request_age_ms,
            }
        })
    }
}

impl SlotRetirement for RuntimeClient {
    fn retire_drained(
        &self,
        drained: &RuntimeSnapshot,
        deadline: Instant,
    ) -> Result<RuntimeSnapshot, CliFailure> {
        self.validate_observed(drained)?;
        if drained.lifecycle.accepting
            || drained.lifecycle.stopping
            || drained.lifecycle.in_flight != 0
        {
            return Err(failed("retirement requires a closed and drained instance"));
        }
        drained
            .lifecycle
            .revision
            .checked_add(1)
            .ok_or_else(|| failed("private runtime revision exhausted"))?;
        // A mutation is sent once. Neither a lost reply nor a closed listener
        // proves exit; the deployment adapter must reconcile process ownership.
        let response: RuntimeSnapshot = self.read(
            self.client.post(format!("{}/v1/retire", self.base))
                .json(&serde_json::json!({"instance_id": drained.instance_id, "expected_revision": drained.lifecycle.revision})),
            deadline,
            Duration::from_secs(6),
        )?;
        self.validate_observed(&response)?;
        if !response.observes_retirement(drained) {
            return Err(failed(
                "private runtime retirement outcome requires reconciliation",
            ));
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write as _, net::TcpListener, thread};

    fn active() -> ActiveReleaseSlot {
        ActiveReleaseSlot {
            schema: aster_upgrade_core::ACTIVE_SLOT_RUNTIME_SCHEMA.into(),
            slot: aster_upgrade_core::ReleaseSlot::Green,
            version: "2.1.0".into(),
            local_runner: Some(aster_upgrade_core::ActiveLocalRunner {
                runner_id: format!("runner_{}", "a".repeat(32)),
                manifest_sha256: "b".repeat(64),
            }),
        }
    }

    fn server(responses: Vec<String>) -> (RuntimeClient, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = RuntimeClient::build(
            format!("http://{}", listener.local_addr().unwrap()),
            HeaderValue::from_static("Bearer test-only"),
            "installation-test".into(),
            &active(),
        )
        .unwrap();
        let handle = thread::spawn(move || {
            responses
                .into_iter()
                .map(|response| {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut request = Vec::new();
                    loop {
                        let mut buffer = [0; 4096];
                        let count = stream.read(&mut buffer).unwrap();
                        assert_ne!(count, 0);
                        request.extend_from_slice(&buffer[..count]);
                        if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n")
                        {
                            let headers = String::from_utf8_lossy(&request[..end]);
                            let length = headers
                                .lines()
                                .find_map(|line| {
                                    line.to_ascii_lowercase()
                                        .strip_prefix("content-length: ")
                                        .map(|value| value.parse::<usize>().unwrap())
                                })
                                .unwrap_or(0);
                            if request.len() >= end + 4 + length {
                                break;
                            }
                        }
                    }
                    stream.write_all(response.as_bytes()).unwrap();
                    String::from_utf8(request).unwrap()
                })
                .collect()
        });
        (client, handle)
    }

    fn json(value: serde_json::Value) -> String {
        let body = value.to_string();
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
    }

    fn status() -> serde_json::Value {
        serde_json::json!({ "schema": "aster.control-runtime.v1", "installation_id": "installation-test", "slot": "green", "instance_id": "instance-test", "product_version": "2.1.0", "lifecycle": { "accepting": true, "stopping": false, "revision": 1, "in_flight": 0, "oldest_request_age_ms": 0 } })
    }

    #[test]
    fn retirement_is_instance_bound_and_does_not_retry_lost_acknowledgment() {
        let mut value = status();
        value["lifecycle"]["accepting"] = false.into();
        let drained: RuntimeSnapshot = serde_json::from_value(value.clone()).unwrap();
        value["lifecycle"]["stopping"] = true.into();
        value["lifecycle"]["revision"] = 2.into();
        let (client, server) = server(vec![json(value)]);
        let reply = client
            .retire_drained(&drained, Instant::now() + Duration::from_secs(3))
            .unwrap();
        assert!(reply.lifecycle.stopping);
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with("POST /v1/retire "));
        assert!(requests[0].contains("\"expected_revision\":1"));
        assert!(requests[0].contains("\"instance_id\":\"instance-test\""));

        let (client, server) = self::server(vec![String::new()]);
        assert!(
            client
                .retire_drained(&drained, Instant::now() + Duration::from_secs(3))
                .is_err()
        );
        assert_eq!(server.join().unwrap().len(), 1);
    }

    #[test]
    fn retirement_rejects_unsafe_input_and_mismatched_outcomes() {
        let mut value = status();
        value["lifecycle"]["accepting"] = false.into();
        let drained: RuntimeSnapshot = serde_json::from_value(value.clone()).unwrap();
        for (field, replacement) in [
            ("accepting", serde_json::json!(true)),
            ("stopping", serde_json::json!(true)),
            ("in_flight", serde_json::json!(1)),
            ("revision", serde_json::json!(u64::MAX)),
        ] {
            let mut invalid = value.clone();
            invalid["lifecycle"][field] = replacement;
            let (client, server) = server(vec![]);
            assert!(
                client
                    .retire_drained(
                        &serde_json::from_value(invalid).unwrap(),
                        Instant::now() + Duration::from_secs(3)
                    )
                    .is_err()
            );
            assert!(server.join().unwrap().is_empty());
        }
        value["lifecycle"]["stopping"] = true.into();
        value["lifecycle"]["revision"] = 2.into();
        let mut outcomes = vec![];
        for (field, replacement) in [
            ("accepting", serde_json::json!(true)),
            ("stopping", serde_json::json!(false)),
            ("in_flight", serde_json::json!(1)),
            ("revision", serde_json::json!(3)),
        ] {
            let mut invalid = value.clone();
            invalid["lifecycle"][field] = replacement;
            outcomes.push(invalid);
        }
        value["instance_id"] = "replacement-instance".into();
        outcomes.push(value);
        for invalid in outcomes {
            let (client, server) = server(vec![json(invalid)]);
            assert!(
                client
                    .retire_drained(&drained, Instant::now() + Duration::from_secs(3))
                    .is_err()
            );
            assert_eq!(server.join().unwrap().len(), 1);
        }
    }

    #[test]
    fn loads_only_the_fixed_slot_token_and_rejects_invalid_material() {
        let temp = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(temp.path()).unwrap();
        fs::create_dir_all(layout.control_config()).unwrap();
        fs::create_dir_all(layout.installation_profile().parent().unwrap()).unwrap();
        fs::write(
            layout.installation_profile(),
            br#"{"installation_id":"installation-test"}"#,
        )
        .unwrap();
        assert!(RuntimeClient::new(&layout, &active()).is_err());
        let path = layout.control_slot_runtime_token("green");
        fs::write(&path, [7u8; 32]).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        }
        let client = RuntimeClient::new(&layout, &active()).unwrap();
        assert_eq!(client.base, "http://127.0.0.1:11483");
        assert!(client.authorization.is_sensitive());
        fs::write(path, [7u8; 33]).unwrap();
        assert!(RuntimeClient::new(&layout, &active()).is_err());
    }

    #[test]
    fn binds_signed_probe_to_the_expected_process_and_runner() {
        let active = active();
        let (client, server) = server(vec![
            json(status()),
            json(
                serde_json::json!({ "schema":"aster.runner-probe.v1", "instance_id":"instance-test", "runner_id": active.local_runner.as_ref().unwrap().runner_id, "passed": true }),
            ),
        ]);
        client.probe_runner(&active).unwrap();
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("GET /v1/status "));
        assert!(requests[1].starts_with("POST /v1/runner-probe "));
        assert!(
            requests
                .iter()
                .all(|request| request.contains("authorization: Bearer test-only"))
        );
        assert!(requests[1].contains("instance-test"));
    }

    #[test]
    fn wrong_slot_version_identity_or_lifecycle_never_reaches_probe() {
        for (key, value) in [
            ("slot", serde_json::json!("blue")),
            ("installation_id", serde_json::json!("another")),
            ("product_version", serde_json::json!("2.0.0")),
            (
                "lifecycle",
                serde_json::json!({"accepting": true, "stopping": true, "revision": 1, "in_flight": 0, "oldest_request_age_ms": 0}),
            ),
        ] {
            let mut snapshot = status();
            snapshot[key] = value;
            let (client, server) = server(vec![json(snapshot)]);
            assert!(client.probe_runner(&active()).is_err());
            assert_eq!(server.join().unwrap().len(), 1);
        }
    }

    #[test]
    fn stale_process_wrong_runner_and_failed_challenges_do_not_pass() {
        for (key, value) in [
            ("instance_id", serde_json::json!("restarted")),
            ("runner_id", serde_json::json!("other")),
            ("passed", serde_json::json!(false)),
        ] {
            let active = active();
            let mut probe = serde_json::json!({"schema":"aster.runner-probe.v1", "instance_id":"instance-test", "runner_id":active.local_runner.as_ref().unwrap().runner_id, "passed":true});
            probe[key] = value;
            let (client, server) = server(vec![json(status()), json(probe)]);
            assert!(client.probe_runner(&active).is_err());
            assert_eq!(server.join().unwrap().len(), 2);
        }
    }

    fn expected() -> ReadinessExpectation {
        let runner = active().local_runner.unwrap();
        ReadinessExpectation {
            manifest_sha256: runner.manifest_sha256,
            models: vec!["model".into()],
            runner_ids: vec![runner.runner_id],
        }
    }

    fn model_report() -> serde_json::Value {
        serde_json::json!({"schema":"aster.readiness-models.v1", "installation_id":"installation-test",
            "instance_id":"instance-test", "slot":"green", "revision":1, "models":["model"]})
    }

    fn ready_report() -> serde_json::Value {
        serde_json::json!({"schema":"aster.control-readiness.v1", "instance_id":"instance-test", "slot":"green", "revision":1,
            "manifest_sha256":expected().manifest_sha256, "ready":true, "readiness_token":"ready-test-only", "valid_for_seconds":10,
            "checks":{"platform":true,"dependencies":{"database_read_write":true,"business_state":true,"model_routes":true},"runners":true,"assets":true}})
    }

    fn observed() -> RuntimeSnapshot {
        serde_json::from_value(status()).unwrap()
    }

    #[test]
    fn readiness_admission_and_drain_use_one_pinned_process_and_revision_chain() {
        let mut opened = status();
        opened["lifecycle"]["revision"] = 2.into();
        let mut closed = opened.clone();
        closed["lifecycle"]["revision"] = 3.into();
        closed["lifecycle"]["accepting"] = false.into();
        let mut outstanding = closed.clone();
        outstanding["lifecycle"]["in_flight"] = 2.into();
        outstanding["lifecycle"]["oldest_request_age_ms"] = 42.into();
        let (client, server) = server(vec![
            json(model_report()),
            json(ready_report()),
            json(opened),
            json(closed.clone()),
            json(outstanding),
            json(closed),
        ]);
        let deadline = Instant::now() + Duration::from_secs(10);
        let permit = client
            .readiness(&observed(), &expected(), deadline)
            .unwrap();
        let opened = client.open_admission(permit, deadline).unwrap();
        let closed = client.close_admission(&opened, deadline).unwrap();
        assert_eq!(
            client.observe_drain(&closed, deadline).unwrap(),
            DrainProgress::Outstanding {
                count: 2,
                oldest_request_age_ms: 42
            }
        );
        assert_eq!(
            client.observe_drain(&closed, deadline).unwrap(),
            DrainProgress::Drained
        );
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("POST /v1/readiness-models "));
        assert!(requests[1].starts_with("POST /v1/readiness "));
        assert!(requests[2].contains("\"readiness_token\":\"ready-test-only\""));
        assert!(requests[2].contains("\"expected_revision\":1"));
        assert!(requests[3].contains("\"expected_revision\":2"));
        assert!(requests[3].contains("\"accepting\":false"));
        assert!(!requests[3].contains("ready-test-only"));
    }

    #[test]
    fn incomplete_or_mismatched_readiness_never_creates_an_open_permit() {
        for pointer in [
            "/ready",
            "/checks/assets",
            "/checks/platform",
            "/checks/runners",
            "/checks/dependencies/database_read_write",
            "/checks/dependencies/business_state",
            "/checks/dependencies/model_routes",
        ] {
            let mut report = ready_report();
            *report.pointer_mut(pointer).unwrap() = false.into();
            let (client, server) = server(vec![json(model_report()), json(report)]);
            assert!(
                client
                    .readiness(
                        &observed(),
                        &expected(),
                        Instant::now() + Duration::from_secs(5)
                    )
                    .is_err()
            );
            assert_eq!(server.join().unwrap().len(), 2);
        }
        for (key, value) in [
            ("instance_id", serde_json::json!("restarted")),
            ("revision", serde_json::json!(2)),
            ("manifest_sha256", serde_json::json!("a".repeat(64))),
            ("readiness_token", serde_json::Value::Null),
            ("valid_for_seconds", serde_json::json!(0)),
            ("valid_for_seconds", serde_json::json!(11)),
        ] {
            let mut report = ready_report();
            report[key] = value;
            let (client, server) = server(vec![json(model_report()), json(report)]);
            assert!(
                client
                    .readiness(
                        &observed(),
                        &expected(),
                        Instant::now() + Duration::from_secs(5)
                    )
                    .is_err()
            );
            server.join().unwrap();
        }
    }

    #[test]
    fn incomplete_or_changed_model_inventory_never_reaches_the_readiness_command() {
        for (key, value) in [
            ("schema", serde_json::json!("unsupported")),
            ("installation_id", serde_json::json!("other")),
            ("instance_id", serde_json::json!("replacement")),
            ("revision", serde_json::json!(2)),
            ("slot", serde_json::json!("blue")),
            ("models", serde_json::json!([])),
            ("models", serde_json::json!(["model", "model"])),
            ("models", serde_json::json!(["model", "new-model"])),
        ] {
            let mut report = model_report();
            report[key] = value;
            let (client, server) = server(vec![json(report)]);
            assert!(
                client
                    .readiness(
                        &observed(),
                        &expected(),
                        Instant::now() + Duration::from_secs(5)
                    )
                    .is_err()
            );
            let requests = server.join().unwrap();
            assert_eq!(requests.len(), 1);
            assert!(requests[0].starts_with("POST /v1/readiness-models "));
        }
    }

    #[test]
    fn request_budget_is_read_from_the_original_runtime_without_a_fallback() {
        let mut value = status();
        value["request_budget_ms"] = 812_345.into();
        let (client, server_handle) = server(vec![json(value.clone())]);
        assert_eq!(
            client
                .request_budget_ms(&observed(), Instant::now() + Duration::from_secs(5))
                .unwrap(),
            812_345
        );
        assert_eq!(server_handle.join().unwrap().len(), 1);
        for (pointer, replacement) in [
            ("/request_budget_ms", serde_json::Value::Null),
            ("/request_budget_ms", serde_json::json!(0)),
            ("/instance_id", serde_json::json!("replacement")),
            ("/lifecycle/revision", serde_json::json!(2)),
            ("/lifecycle/accepting", serde_json::json!(false)),
            ("/lifecycle/stopping", serde_json::json!(true)),
        ] {
            let mut invalid = value.clone();
            *invalid.pointer_mut(pointer).unwrap() = replacement;
            let (client, server) = server(vec![json(invalid)]);
            assert!(
                client
                    .request_budget_ms(&observed(), Instant::now() + Duration::from_secs(5))
                    .is_err()
            );
            server.join().unwrap();
        }
    }

    #[test]
    fn a_restarted_reopened_or_unreachable_instance_is_not_drained() {
        let mut closed = observed();
        closed.lifecycle.accepting = false;
        for (pointer, value) in [
            ("/instance_id", serde_json::json!("restarted")),
            ("/lifecycle/revision", serde_json::json!(2)),
            ("/lifecycle/accepting", serde_json::json!(true)),
            ("/lifecycle/stopping", serde_json::json!(true)),
        ] {
            let mut result = serde_json::to_value(&closed).unwrap();
            *result.pointer_mut(pointer).unwrap() = value;
            let (client, server) = server(vec![json(result)]);
            assert!(
                client
                    .observe_drain(&closed, Instant::now() + Duration::from_secs(5))
                    .is_err()
            );
            server.join().unwrap();
        }
        let (client, server) = server(vec![
            "HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\n\r\n".into(),
        ]);
        assert!(
            client
                .observe_drain(&closed, Instant::now() + Duration::from_secs(5))
                .is_err()
        );
        server.join().unwrap();
    }

    #[test]
    fn expired_deadlines_permits_and_invalid_coverage_do_not_send_requests() {
        let client = RuntimeClient::build(
            "http://127.0.0.1:1".into(),
            HeaderValue::from_static("Bearer test-only"),
            "installation-test".into(),
            &active(),
        )
        .unwrap();
        let expired = Instant::now() - Duration::from_secs(1);
        assert!(client.status(expired).is_err());
        let permit = ReadyPermit {
            observed: observed(),
            token: Zeroizing::new("ready-test-only".into()),
            expires: expired,
        };
        assert!(
            client
                .open_admission(permit, Instant::now() + Duration::from_secs(5))
                .is_err()
        );
        let mut coverage = expected();
        coverage.runner_ids = vec!["another-runner".into()];
        assert!(
            client
                .readiness(
                    &observed(),
                    &coverage,
                    Instant::now() + Duration::from_secs(5)
                )
                .is_err()
        );
    }

    #[test]
    fn admission_with_an_uncertain_response_is_not_automatically_retried() {
        let (client, server) = server(vec![
            "HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\n{".into(),
        ]);
        assert!(
            client
                .close_admission(&observed(), Instant::now() + Duration::from_secs(5))
                .is_err()
        );
        assert_eq!(server.join().unwrap().len(), 1);
    }

    #[test]
    fn redirect_errors_and_oversized_responses_are_rejected() {
        for response in ["HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/forbidden\r\nContent-Length: 0\r\n\r\n".into(), "HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\n\r\n".into(), json(serde_json::json!({"oversized": "x".repeat(17000)}))] {
            let (client, server) = server(vec![response]);
            let error = client.probe_runner(&active()).err().unwrap().to_string();
            assert!(!error.contains("test-only"));
            server.join().unwrap();
        }
    }
}
