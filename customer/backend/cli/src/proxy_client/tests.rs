use super::*;
use serde_json::json;
use std::{io::Write as _, net::TcpListener, thread};

pub(super) fn config(slot: ReleaseSlot) -> Value {
    let ports = slot.ports();
    let routes = [ports.api, ports.member, ports.admin].into_iter().enumerate().map(|(role, port)| json!({
        "match": [{"host": [format!("role-{role}.example")]}],
        "handle": [{"handler": "subroute", "routes": [{"handle": [{
            "handler": "reverse_proxy", "upstreams": [{"dial": format!("127.0.0.1:{port}")}],
            "stream_close_delay": 900_000_000_000_u64
        }]}]}]
    })).collect::<Vec<_>>();
    json!({
        "admin": { "listen": "127.0.0.1:2019", "config": { "persist": false } },
        "apps": { "http": { "servers": { "srv0": {
            "listen": [":443"], "tls_connection_policies": [{}], "routes": routes
        }}}}
    })
}

fn proxy_mut(config: &mut Value, role: usize) -> &mut Value {
    &mut config["apps"]["http"]["servers"]["srv0"]["routes"][role]["handle"][0]["routes"][0]["handle"]
        [0]
}

pub(super) fn response(body: &str, status: &str, etag: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nETag: {etag}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

pub(super) fn served(config: Value) -> String {
    response(&config.to_string(), "200 OK", "\"/config/ 1234abcd\"")
}

pub(super) fn server(responses: Vec<String>) -> (CaddyClient, thread::JoinHandle<Vec<String>>) {
    server_with_inspection(responses, |_| {})
}

pub(super) fn server_with_inspection(
    responses: Vec<String>,
    inspect_request: impl Fn(&str) + Send + 'static,
) -> (CaddyClient, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client =
        CaddyClient::build(format!("http://{}/config/", listener.local_addr().unwrap())).unwrap();
    let handle = thread::spawn(move || {
        let mut retained = Vec::new();
        responses
            .into_iter()
            .map(|response| {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                loop {
                    let mut chunk = [0_u8; 4096];
                    let count = stream.read(&mut chunk).unwrap();
                    assert_ne!(count, 0);
                    request.extend_from_slice(&chunk[..count]);
                    if let Some(end) = request.windows(4).position(|chunk| chunk == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length: ")
                                    .map(|length| length.parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                // Rejection of an oversized body or bad ETag may close the socket early.
                inspect_request(std::str::from_utf8(&request).unwrap());
                let _ = stream.write_all(response.as_bytes());
                if response.contains("Connection: keep-alive") {
                    // Simulate the old admin server still draining after POST.
                    // Its idle socket must not receive the next observation.
                    retained.push(stream);
                }
                String::from_utf8(request).unwrap()
            })
            .collect()
    });
    (client, handle)
}

pub(super) fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}

#[test]
fn installed_upstreams_are_persisted_before_live_switch_and_confirmed_afterward() {
    let (_temporary, layout) = crate::proxy_disk::tests::fixture();
    let path = layout.caddy_upstreams();
    let expected =
        crate::maintenance_executor::render_upstreams(ReleaseSlot::Green, Default::default());
    let (mut client, handle) = server_with_inspection(
        vec![
            served(config(ReleaseSlot::Blue)),
            response("", "200 OK", "unused"),
            served(config(ReleaseSlot::Green)),
        ],
        move |request| {
            if request.starts_with("POST ") {
                assert_eq!(std::fs::read(&path).unwrap(), expected.as_bytes());
            }
        },
    );
    client.disk = Some(crate::proxy_disk::InstalledUpstreams::open(&layout).unwrap());
    let before = client.observe(deadline()).unwrap();
    client
        .switch_to(before, ReleaseSlot::Green, 720_000, deadline())
        .unwrap();
    client
        .disk
        .as_ref()
        .unwrap()
        .assert_target(ReleaseSlot::Green, deadline())
        .unwrap();
    assert_eq!(handle.join().unwrap().len(), 3);
}

#[test]
fn lost_live_reply_retains_disk_intent_and_reconciles_without_reloading_again() {
    let (_temporary, layout) = crate::proxy_disk::tests::fixture();
    let (mut client, handle) = server(vec![served(config(ReleaseSlot::Blue)), String::new()]);
    client.disk = Some(crate::proxy_disk::InstalledUpstreams::open(&layout).unwrap());
    let before = client.observe(deadline()).unwrap();
    assert!(
        client
            .switch_to(before, ReleaseSlot::Green, 720_000, deadline())
            .is_err()
    );
    assert_eq!(handle.join().unwrap().len(), 2);
    client
        .disk
        .as_ref()
        .unwrap()
        .assert_target(ReleaseSlot::Green, deadline())
        .unwrap();
    let (mut recovered, handle) = server(vec![
        served(config(ReleaseSlot::Green)),
        served(config(ReleaseSlot::Green)),
    ]);
    recovered.disk = Some(crate::proxy_disk::InstalledUpstreams::open(&layout).unwrap());
    let before = recovered.observe(deadline()).unwrap();
    recovered
        .switch_to(before, ReleaseSlot::Green, 720_000, deadline())
        .unwrap();
    assert!(
        handle
            .join()
            .unwrap()
            .iter()
            .all(|request| request.starts_with("GET "))
    );
}

#[test]
fn live_conflict_does_not_restore_disk_or_mask_the_failed_cutover() {
    let (_temporary, layout) = crate::proxy_disk::tests::fixture();
    let (mut client, handle) = server(vec![
        served(config(ReleaseSlot::Blue)),
        response("", "412 Precondition Failed", "unused"),
    ]);
    client.disk = Some(crate::proxy_disk::InstalledUpstreams::open(&layout).unwrap());
    let before = client.observe(deadline()).unwrap();
    assert!(
        client
            .switch_to(before, ReleaseSlot::Green, 720_000, deadline())
            .is_err()
    );
    client
        .disk
        .as_ref()
        .unwrap()
        .assert_target(ReleaseSlot::Green, deadline())
        .unwrap();
    assert_eq!(handle.join().unwrap().len(), 2);
}

#[test]
fn snapshot_pins_everything_except_the_three_slot_dials() {
    let blue = config(ReleaseSlot::Blue);
    let mut green = config(ReleaseSlot::Green);
    assert_eq!(
        inspect(&blue).unwrap().configuration_sha256,
        inspect(&green).unwrap().configuration_sha256
    );
    assert_eq!(inspect(&green).unwrap().stream_close_delay_ms, 900_000);
    green["apps"]["http"]["servers"]["srv0"]["listen"] = json!([":8443"]);
    assert_ne!(
        inspect(&blue).unwrap().configuration_sha256,
        inspect(&green).unwrap().configuration_sha256
    );
}

#[test]
fn mixed_missing_duplicate_dynamic_and_unmanaged_routing_are_rejected() {
    for fault in 0..7 {
        let mut value = config(ReleaseSlot::Blue);
        match fault {
            0 => proxy_mut(&mut value, 1)["upstreams"][0]["dial"] = "127.0.0.1:11481".into(),
            1 => proxy_mut(&mut value, 1)["upstreams"][0]["dial"] = "127.0.0.1:11380".into(),
            2 => proxy_mut(&mut value, 1)["upstreams"] = json!([]),
            3 => proxy_mut(&mut value, 1)["dynamic_upstreams"] = json!({"source": "srv"}),
            4 => proxy_mut(&mut value, 1)["upstreams"][0]["dial"] = "outside.example:443".into(),
            5 => proxy_mut(&mut value, 1)["handler"] = "file_server".into(),
            _ => value["admin"]["listen"] = "0.0.0.0:2019".into(),
        }
        assert!(inspect(&value).is_err(), "fault {fault}");
    }
}

#[test]
fn one_etag_mutation_switches_all_entries_and_requires_live_readback() {
    let (client, handle) = server(vec![
        served(config(ReleaseSlot::Blue)),
        response("", "200 OK", "unused"),
        served(config(ReleaseSlot::Green)),
    ]);
    let observed = client.observe(deadline()).unwrap();
    let actual = client
        .switch_to(observed, ReleaseSlot::Green, 720_000, deadline())
        .unwrap();
    assert_eq!(CaddyClient::snapshot(&actual).slot, ReleaseSlot::Green);
    let requests = handle.join().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[1].starts_with("POST /config/ HTTP/1.1"));
    assert!(
        requests[1]
            .to_ascii_lowercase()
            .contains("if-match: \"/config/ 1234abcd\"")
    );
    let posted: Value =
        serde_json::from_str(requests[1].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(posted, config(ReleaseSlot::Green));
}

#[test]
fn lost_acknowledgement_and_conflict_never_replay_the_mutation() {
    for acknowledgement in [
        String::new(),
        response("", "412 Precondition Failed", "unused"),
    ] {
        let (client, handle) = server(vec![served(config(ReleaseSlot::Blue)), acknowledgement]);
        let observed = client.observe(deadline()).unwrap();
        assert!(
            client
                .switch_to(observed, ReleaseSlot::Green, 720_000, deadline())
                .is_err()
        );
        assert_eq!(handle.join().unwrap().len(), 2);
    }
}

#[test]
fn success_ack_does_not_hide_wrong_slot_or_other_configuration_changes() {
    for fault in 0..3 {
        let mut after = config(if fault == 0 {
            ReleaseSlot::Blue
        } else {
            ReleaseSlot::Green
        });
        if fault == 1 {
            after["apps"]["http"]["servers"]["srv0"]["listen"] = json!([":8443"]);
        }
        if fault == 2 {
            proxy_mut(&mut after, 1)["upstreams"][0]["dial"] = "127.0.0.1:11381".into();
        }
        let (client, handle) = server(vec![
            served(config(ReleaseSlot::Blue)),
            response("", "200 OK", "unused"),
            served(after),
        ]);
        let observed = client.observe(deadline()).unwrap();
        assert!(
            client
                .switch_to(observed, ReleaseSlot::Green, 720_000, deadline())
                .is_err()
        );
        assert_eq!(handle.join().unwrap().len(), 3);
    }
}

#[test]
fn insufficient_existing_retention_and_expired_deadline_cannot_send_a_switch() {
    let mut source = config(ReleaseSlot::Blue);
    proxy_mut(&mut source, 0)
        .as_object_mut()
        .unwrap()
        .remove("stream_close_delay");
    let (client, handle) = server(vec![served(source)]);
    let observed = client.observe(deadline()).unwrap();
    assert!(
        client
            .switch_to(observed, ReleaseSlot::Green, 720_000, deadline())
            .is_err()
    );
    assert_eq!(handle.join().unwrap().len(), 1);
    assert!(client.observe(Instant::now()).is_err());
}

#[test]
fn invalid_concurrency_evidence_and_oversized_response_are_rejected() {
    for etag in [
        "",
        "W/\"/config/ 1234\"",
        "\"/config/apps/http 1234\"",
        "\"/config/ invalid\"",
    ] {
        let (client, handle) = server(vec![response(
            &config(ReleaseSlot::Blue).to_string(),
            "200 OK",
            etag,
        )]);
        assert!(client.observe(deadline()).is_err());
        handle.join().unwrap();
    }
    let mut huge = config(ReleaseSlot::Blue);
    huge["padding"] = "x".repeat(MAX_CONFIG_BYTES as usize).into();
    let (client, handle) = server(vec![served(huge)]);
    assert!(client.observe(deadline()).is_err());
    handle.join().unwrap();
}

#[test]
fn already_targeted_slot_still_requires_a_fresh_observation() {
    let (client, handle) = server(vec![
        served(config(ReleaseSlot::Green)),
        served(config(ReleaseSlot::Blue)),
    ]);
    let observed = client.observe(deadline()).unwrap();
    assert!(
        client
            .switch_to(observed, ReleaseSlot::Green, 720_000, deadline())
            .is_err()
    );
    assert!(
        handle
            .join()
            .unwrap()
            .iter()
            .all(|request| request.starts_with("GET "))
    );
}

pub(crate) fn transition_journal(
    layout: &aster_install_layout::InstallLayout,
    previous: ReleaseSlot,
    phase: aster_upgrade_core::online::OnlinePhase,
) -> aster_upgrade_core::online::OnlineJournal {
    use aster_upgrade_core::online::{OnlineJournal, OnlinePhase};
    let mut plan = crate::online_journal::tests::plan();
    let candidate = if previous == ReleaseSlot::Blue {
        ReleaseSlot::Green
    } else {
        ReleaseSlot::Blue
    };
    plan.previous.slot = previous;
    plan.previous_process.slot = previous;
    plan.candidate.slot = candidate;
    plan.candidate_process.slot = candidate;
    plan.proxy = inspect(&config(previous)).unwrap();
    let mut store = crate::online_journal::JournalFile::open(layout).unwrap();
    let journal = OnlineJournal::create(plan.clone(), &mut store).unwrap();
    let mut value = serde_json::to_value(journal).unwrap();
    value["phase"] = serde_json::to_value(phase).unwrap();
    let revision = match phase {
        OnlinePhase::OpeningCandidate => 0,
        OnlinePhase::SwitchingTraffic => 1,
        OnlinePhase::ClosingPrevious => 2,
        OnlinePhase::DrainingPrevious => 3,
        OnlinePhase::DrainBudgetExhausted | OnlinePhase::RetiringPrevious => 4,
        OnlinePhase::Committing => 5,
        OnlinePhase::Complete => 6,
    };
    value["revision"] = revision.into();
    if revision >= 1 {
        let mut opened = plan.candidate_process.clone();
        opened.lifecycle.accepting = true;
        opened.lifecycle.revision += 1;
        value["opened"] = serde_json::to_value(opened).unwrap();
        value["cutover_started_at_ms"] = 1000.into();
    }
    if revision >= 2 {
        value["drain_started_at_ms"] = 1000.into();
    }
    let mut closed = plan.previous_process;
    closed.lifecycle.accepting = false;
    closed.lifecycle.revision += 1;
    if revision >= 3 {
        value["closed"] = serde_json::to_value(&closed).unwrap();
    }
    if revision >= 4 {
        value[if phase == OnlinePhase::DrainBudgetExhausted {
            "exhausted"
        } else {
            "drained"
        }] = serde_json::to_value(closed).unwrap();
    }
    let journal: OnlineJournal = serde_json::from_value(value).unwrap();
    assert!(journal.valid());
    journal
}

#[test]
fn recovery_checks_every_phase_and_both_cutover_directions_without_mutation() {
    use aster_upgrade_core::online::OnlinePhase;
    for previous in [ReleaseSlot::Blue, ReleaseSlot::Green] {
        for phase in [
            OnlinePhase::OpeningCandidate,
            OnlinePhase::SwitchingTraffic,
            OnlinePhase::ClosingPrevious,
            OnlinePhase::DrainingPrevious,
            OnlinePhase::DrainBudgetExhausted,
            OnlinePhase::RetiringPrevious,
            OnlinePhase::Committing,
            OnlinePhase::Complete,
        ] {
            for disk_slot in [ReleaseSlot::Blue, ReleaseSlot::Green] {
                for live_slot in [ReleaseSlot::Blue, ReleaseSlot::Green] {
                    let (_temporary, layout) = crate::proxy_disk::tests::fixture();
                    let journal = transition_journal(&layout, previous, phase);
                    let disk = crate::proxy_disk::InstalledUpstreams::open(&layout).unwrap();
                    disk.prepare(disk_slot, deadline()).unwrap();
                    let original = std::fs::read(layout.caddy_upstreams()).unwrap();
                    let (mut client, handle) = server(vec![served(config(live_slot))]);
                    client.disk = Some(disk);
                    let expected = match phase {
                        OnlinePhase::OpeningCandidate => {
                            disk_slot == previous && live_slot == previous
                        }
                        OnlinePhase::SwitchingTraffic => {
                            live_slot == previous || disk_slot != previous
                        }
                        _ => disk_slot != previous && live_slot != previous,
                    };
                    let result = client.observe_transition(&journal, deadline());
                    assert_eq!(
                        result.is_ok(),
                        expected,
                        "{previous:?} {phase:?} disk={disk_slot:?} live={live_slot:?}"
                    );
                    assert_eq!(std::fs::read(layout.caddy_upstreams()).unwrap(), original);
                    let requests = handle.join().unwrap();
                    assert_eq!(requests.len(), 1);
                    assert!(requests[0].starts_with("GET "));
                }
            }
        }
    }
}

#[test]
fn recovery_rejects_configuration_drift_and_a_disk_change_during_observation() {
    use aster_upgrade_core::online::OnlinePhase;
    for concurrent_disk_change in [false, true] {
        let (_temporary, layout) = crate::proxy_disk::tests::fixture();
        let journal = transition_journal(&layout, ReleaseSlot::Blue, OnlinePhase::SwitchingTraffic);
        let disk = crate::proxy_disk::InstalledUpstreams::open(&layout).unwrap();
        let path = layout.caddy_upstreams();
        let mut live = config(ReleaseSlot::Blue);
        if !concurrent_disk_change {
            live["apps"]["http"]["servers"]["srv0"]["listen"] = json!([":8443"]);
        }
        let (mut client, handle) = server_with_inspection(vec![served(live)], move |_| {
            if concurrent_disk_change {
                std::fs::write(
                    &path,
                    crate::maintenance_executor::render_upstreams(
                        ReleaseSlot::Green,
                        Default::default(),
                    ),
                )
                .unwrap();
            }
        });
        client.disk = Some(disk);
        assert!(client.observe_transition(&journal, deadline()).is_err());
        assert!(handle.join().unwrap()[0].starts_with("GET "));
        client
            .disk
            .as_ref()
            .unwrap()
            .assert_target(
                if concurrent_disk_change {
                    ReleaseSlot::Green
                } else {
                    ReleaseSlot::Blue
                },
                deadline(),
            )
            .unwrap();
    }
}

#[test]
fn recovery_requires_valid_journal_disk_and_remaining_deadline_before_network_io() {
    use aster_upgrade_core::online::OnlinePhase;
    let (_temporary, layout) = crate::proxy_disk::tests::fixture();
    let journal = transition_journal(&layout, ReleaseSlot::Blue, OnlinePhase::OpeningCandidate);
    let mut client = CaddyClient::build("http://127.0.0.1:1/config/".into()).unwrap();
    assert!(client.observe_transition(&journal, deadline()).is_err());
    client.disk = Some(crate::proxy_disk::InstalledUpstreams::open(&layout).unwrap());
    assert!(client.observe_transition(&journal, Instant::now()).is_err());
    let mut value = serde_json::to_value(&journal).unwrap();
    value["revision"] = 99.into();
    let invalid = serde_json::from_value(value).unwrap();
    assert!(client.observe_transition(&invalid, deadline()).is_err());
}

#[test]
fn admin_reload_readback_uses_new_connections_even_when_old_sockets_remain_open() {
    let replies = vec![
        served(config(ReleaseSlot::Blue)),
        response("", "200 OK", "unused"),
        served(config(ReleaseSlot::Green)),
    ]
    .into_iter()
    .map(|reply| reply.replace("Connection: close", "Connection: keep-alive"))
    .collect();
    let (client, handle) = server_with_inspection(replies, |request| {
        assert!(
            request
                .to_ascii_lowercase()
                .contains("connection: close\r\n")
        );
    });
    let original = client.observe(deadline()).unwrap();
    let current = client
        .switch_to(original, ReleaseSlot::Green, 720_000, deadline())
        .unwrap();
    assert_eq!(CaddyClient::snapshot(&current).slot, ReleaseSlot::Green);
    let requests = handle.join().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[0].starts_with("GET "));
    assert!(requests[1].starts_with("POST "));
    assert!(requests[2].starts_with("GET "));
}
