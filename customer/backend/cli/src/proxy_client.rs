//! Caddy 2.11.3 live-config adapter. Whole-config ETag CAS changes all three
//! business entries in one request; no retry and no inference from disk state.
use std::{
    io::Read as _,
    time::{Duration, Instant},
};

use aster_error_catalog::delivery;
use aster_upgrade_core::{
    ReleaseSlot,
    runtime::{ProxySnapshot, SlotProxy},
};
use reqwest::{
    blocking::{Client, RequestBuilder, Response},
    header::{ETAG, HeaderValue, IF_MATCH},
};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

use super::CliFailure;

const MAX_CONFIG_BYTES: u64 = 512 * 1024;
const REQUEST_BUDGET: Duration = Duration::from_secs(6);

pub(super) struct CaddyClient {
    client: Client,
    endpoint: String,
    disk: Option<super::proxy_disk::InstalledUpstreams>,
}

// No Debug/Clone: a live configuration may contain sensitive material and its
// ETag authorizes one attempted mutation, not repeated commands after a timeout.
pub(super) struct Observation {
    config: Value,
    etag: HeaderValue,
    snapshot: ProxySnapshot,
}

impl CaddyClient {
    fn build(endpoint: String) -> Result<Self, CliFailure> {
        let client = Client::builder()
            .no_proxy()
            // Reload replaces Caddy's admin server and closes its old idle
            // sockets asynchronously. A fresh connection avoids reusing one.
            .pool_max_idle_per_host(0)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(1))
            .timeout(REQUEST_BUDGET)
            .build()
            .map_err(|_| failed("cannot initialize Caddy loopback client"))?;
        Ok(Self {
            client,
            endpoint,
            disk: None,
        })
    }

    fn send(&self, request: RequestBuilder, deadline: Instant) -> Result<Response, CliFailure> {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|value| !value.is_zero())
            .ok_or_else(|| failed("Caddy operation deadline elapsed"))?;
        let response = request.header(reqwest::header::CONNECTION, "close")
            .timeout(remaining.min(REQUEST_BUDGET)).send()
            .map_err(|_| failed("Caddy operation outcome is unknown; observe live configuration before retrying"))?;
        if !response.status().is_success() {
            return Err(failed(
                if response.status() == reqwest::StatusCode::PRECONDITION_FAILED {
                    "Caddy configuration changed; a fresh observation is required"
                } else {
                    "Caddy operation failed; observe live configuration before continuing"
                },
            ));
        }
        Ok(response)
    }
}

impl SlotProxy for CaddyClient {
    type Error = CliFailure;
    type Observation = Observation;

    fn connect() -> Result<Self, Self::Error> {
        let mut client = Self::build("http://127.0.0.1:2019/config/".into())?;
        client.disk = Some(super::proxy_disk::InstalledUpstreams::open(
            &super::install_layout(),
        )?);
        Ok(client)
    }

    fn observe(&self, deadline: Instant) -> Result<Observation, Self::Error> {
        let response = self.send(self.client.get(&self.endpoint), deadline)?;
        let etag = response
            .headers()
            .get(ETAG)
            .cloned()
            .ok_or_else(|| failed("Caddy did not return whole-config concurrency evidence"))?;
        let raw = etag.to_str().map_err(|_| failed("Caddy ETag is invalid"))?;
        let hash = raw
            .strip_prefix("\"/config/ ")
            .and_then(|value| value.strip_suffix('"'))
            .filter(|value| {
                !value.is_empty()
                    && value.len() <= 128
                    && value.bytes().all(|byte| byte.is_ascii_hexdigit())
            });
        if hash.is_none() {
            return Err(failed("Caddy ETag does not bind the whole configuration"));
        }
        let mut bytes = Zeroizing::new(Vec::new());
        response
            .take(MAX_CONFIG_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| failed("Caddy configuration response is incomplete"))?;
        if bytes.len() as u64 > MAX_CONFIG_BYTES {
            return Err(failed("Caddy configuration exceeds the supported size"));
        }
        let config: Value = serde_json::from_slice(&bytes)
            .map_err(|_| failed("Caddy configuration response is invalid"))?;
        let snapshot = inspect(&config)?;
        Ok(Observation {
            config,
            etag,
            snapshot,
        })
    }

    fn snapshot(observed: &Observation) -> &ProxySnapshot {
        &observed.snapshot
    }

    fn observe_transition(
        &self,
        journal: &aster_upgrade_core::online::OnlineJournal,
        deadline: Instant,
    ) -> Result<Observation, Self::Error> {
        if !journal.valid() {
            return Err(failed("online transition journal is invalid"));
        }
        let disk = self.disk.as_ref().ok_or_else(|| {
            failed("online recovery requires installer-managed disk configuration")
        })?;
        let before = disk.observe(deadline)?;
        let live = self.observe(deadline)?;
        // Detect a concurrent disk change during the network observation. Never
        // rewrite intent or reload Caddy to hide a mismatch during recovery.
        disk.assert_target(before, deadline)?;
        if !journal.accepts_proxy(&live.snapshot, before) {
            return Err(failed(
                "disk and live proxy state do not match the durable online phase; preserve both slots",
            ));
        }
        Ok(live)
    }

    fn observe_switchback(
        &self,
        journal: &aster_upgrade_core::online::switchback::SwitchbackJournal,
        deadline: Instant,
    ) -> Result<Observation, Self::Error> {
        if !journal.valid() {
            return Err(failed("online switchback journal is invalid"));
        }
        let disk = self.disk.as_ref().ok_or_else(|| {
            failed("online switchback requires installer-managed disk configuration")
        })?;
        let before = disk.observe(deadline)?;
        let live = self.observe(deadline)?;
        disk.assert_target(before, deadline)?;
        if !journal.accepts_proxy(&live.snapshot, before) {
            return Err(failed(
                "disk and live proxy state do not match the durable switchback phase; preserve both slots",
            ));
        }
        Ok(live)
    }

    fn switch_to(
        &self,
        observed: Observation,
        target: ReleaseSlot,
        required_stream_delay_ms: u64,
        deadline: Instant,
    ) -> Result<Observation, Self::Error> {
        if required_stream_delay_ms == 0
            || observed.snapshot.stream_close_delay_ms < required_stream_delay_ms
        {
            return Err(failed(
                "live Caddy stream retention is insufficient; establish it before an online upgrade",
            ));
        }
        if let Some(disk) = &self.disk {
            if observed.snapshot.stream_close_delay_ms != 900_000 {
                return Err(failed(
                    "online disk persistence requires the installed 15-minute retention policy",
                ));
            }
            // The caller has already persisted its switch intent and holds the
            // installation lock. Save disk first: a lost live-switch reply must
            // never leave a confirmed new live slot with old boot configuration.
            disk.prepare(target, deadline)?;
        }
        let expected_digest = observed.snapshot.configuration_sha256;
        if observed.snapshot.slot != target {
            let mut next = observed.config;
            let mut topology = Topology::default();
            visit(http_servers(&mut next)?, &mut topology, Some(target), 0)?;
            let response = self.send(
                self.client
                    .post(&self.endpoint)
                    .header(IF_MATCH, observed.etag)
                    .json(&next),
                deadline,
            )?;
            // Finishing the bounded response is part of the acknowledgement.
            let mut body = Zeroizing::new(Vec::new());
            response
                .take(4097)
                .read_to_end(&mut body)
                .map_err(|_| failed("Caddy switch acknowledgement is incomplete"))?;
            if body.len() > 4096 {
                return Err(failed("Caddy switch acknowledgement is too large"));
            }
        }
        let actual = self.observe(deadline)?;
        if actual.snapshot.slot != target || actual.snapshot.configuration_sha256 != expected_digest
        {
            return Err(failed(
                "Caddy live configuration does not match the intended cutover",
            ));
        }
        if let Some(disk) = &self.disk {
            disk.assert_target(target, deadline)?;
        }
        Ok(actual)
    }
}

#[derive(Default)]
struct Topology {
    slot: Option<ReleaseSlot>,
    roles: [bool; 3],
    close_delay_ms: Option<u64>,
}

fn inspect(config: &Value) -> Result<ProxySnapshot, CliFailure> {
    let mut normalized = config.clone();
    // This adapter is for the installer-managed loopback endpoint. Changes to
    // its address or persistence policy need a separate deployment strategy.
    if normalized.pointer("/admin/listen").and_then(Value::as_str) != Some("127.0.0.1:2019")
        || normalized
            .pointer("/admin/config/persist")
            .and_then(Value::as_bool)
            != Some(false)
    {
        return Err(failed(
            "Caddy administrative configuration is unsupported for online cutover",
        ));
    }
    let mut topology = Topology::default();
    visit(http_servers(&mut normalized)?, &mut topology, None, 0)?;
    if topology.roles != [true; 3] {
        return Err(failed(
            "Caddy must have exactly one API, Member and Admin upstream",
        ));
    }
    let encoded = serde_json::to_vec(&normalized)
        .map_err(|_| failed("cannot fingerprint Caddy configuration"))?;
    Ok(ProxySnapshot {
        slot: topology
            .slot
            .ok_or_else(|| failed("Caddy has no active slot"))?,
        configuration_sha256: super::lowercase_hex(&Sha256::digest(encoded)),
        stream_close_delay_ms: topology.close_delay_ms.unwrap_or(0),
    })
}

fn http_servers(config: &mut Value) -> Result<&mut Value, CliFailure> {
    let servers = config
        .pointer_mut("/apps/http/servers")
        .ok_or_else(|| failed("Caddy has no HTTP servers"))?;
    if servers.as_object().is_none_or(|servers| servers.is_empty()) {
        return Err(failed("Caddy HTTP servers are invalid"));
    }
    Ok(servers)
}

/// Accept the installer-generated single-upstream topology. Normalize only the
/// slot-specific dials; hosts, listeners, TLS, policies and all other values stay
/// in the fingerprint. Mixed slots, dynamic routing and extra handlers fail.
fn visit(
    value: &mut Value,
    topology: &mut Topology,
    target: Option<ReleaseSlot>,
    depth: usize,
) -> Result<(), CliFailure> {
    if depth > 64 {
        return Err(failed(
            "Caddy configuration nesting exceeds the supported limit",
        ));
    }
    match value {
        Value::Object(object) => {
            if let Some(handler) = object.get("handler") {
                match handler.as_str() {
                    Some("reverse_proxy") => {
                        if object.contains_key("dynamic_upstreams") {
                            return Err(failed("dynamic Caddy upstreams cannot be cut over"));
                        }
                        let delay_ms = match object.get("stream_close_delay") {
                            None => 0,
                            Some(value) => value.as_u64().ok_or_else(|| {
                                failed(
                                    "Caddy stream retention must be a nonnegative numeric duration",
                                )
                            })? / 1_000_000,
                        };
                        topology.close_delay_ms = Some(
                            topology
                                .close_delay_ms
                                .map_or(delay_ms, |old| old.min(delay_ms)),
                        );
                        let upstreams = object
                            .get_mut("upstreams")
                            .and_then(Value::as_array_mut)
                            .filter(|values| values.len() == 1)
                            .ok_or_else(|| failed("Caddy proxy must have one fixed upstream"))?;
                        let dial = upstreams[0]
                            .get_mut("dial")
                            .ok_or_else(|| failed("Caddy upstream has no dial address"))?;
                        let (slot, role) = parse_dial(
                            dial.as_str()
                                .ok_or_else(|| failed("Caddy dial address is invalid"))?,
                        )?;
                        if topology.roles[role] || topology.slot.is_some_and(|old| old != slot) {
                            return Err(failed(
                                "Caddy business entries have duplicate roles or mixed slots",
                            ));
                        }
                        topology.roles[role] = true;
                        topology.slot = Some(slot);
                        *dial = if let Some(target) = target {
                            let ports = target.ports();
                            format!("127.0.0.1:{}", [ports.api, ports.member, ports.admin][role])
                                .into()
                        } else {
                            format!("aster-slot-role-{role}").into()
                        };
                    }
                    Some("subroute") => {}
                    _ => {
                        return Err(failed(
                            "Caddy HTTP handler topology is unsupported for online cutover",
                        ));
                    }
                }
            }
            for child in object.values_mut() {
                visit(child, topology, target, depth + 1)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                visit(child, topology, target, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn parse_dial(dial: &str) -> Result<(ReleaseSlot, usize), CliFailure> {
    for slot in [ReleaseSlot::Blue, ReleaseSlot::Green] {
        let ports = slot.ports();
        for (role, port) in [ports.api, ports.member, ports.admin]
            .into_iter()
            .enumerate()
        {
            if dial == format!("127.0.0.1:{port}") {
                return Ok((slot, role));
            }
        }
    }
    Err(failed(
        "Caddy upstream does not belong to a supported local slot",
    ))
}

fn failed(message: &str) -> CliFailure {
    CliFailure::new(delivery::UPGRADE_FAILED, message)
}

#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
#[path = "proxy_client/switchback_tests.rs"]
pub(crate) mod switchback_tests;
