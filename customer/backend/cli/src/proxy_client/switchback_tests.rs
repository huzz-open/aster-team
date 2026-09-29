use super::{tests::*, *};
use aster_upgrade_core::{
    online::{
        OnlinePhase,
        switchback::{SwitchbackJournal, SwitchbackPhase, SwitchbackPlan},
    },
    runtime::{RuntimeRequestBudget, UpgradeClock},
};
use serde_json::json;

pub(crate) fn journal(
    layout: &aster_install_layout::InstallLayout,
    previous: ReleaseSlot,
    phase: SwitchbackPhase,
) -> SwitchbackJournal {
    let original = transition_journal(layout, previous, OnlinePhase::DrainingPrevious);
    let mut old = original.plan().previous_process.clone();
    old.lifecycle.accepting = false;
    old.lifecycle.revision += 1;
    let mut candidate = original.plan().candidate_process.clone();
    candidate.lifecycle.accepting = true;
    candidate.lifecycle.revision += 1;
    let plan = SwitchbackPlan {
        previous: old.clone(),
        candidate: candidate.clone(),
        candidate_budget: RuntimeRequestBudget {
            runtime: candidate.clone(),
            request_budget_ms: 600_000,
        },
        proxy: inspect(&config(candidate.slot)).unwrap(),
        clock: UpgradeClock {
            boot_id: original.plan().clock.boot_id.clone(),
            uptime_ms: 2_000,
        },
        original,
    };
    assert!(plan.valid());
    let revision = match phase {
        SwitchbackPhase::PreparingPrevious => 0,
        SwitchbackPhase::SwitchingBack => 1,
        SwitchbackPhase::ClosingCandidate => 2,
        SwitchbackPhase::DrainingCandidate => 3,
        SwitchbackPhase::RetiringCandidate | SwitchbackPhase::DrainBudgetExhausted => 4,
        SwitchbackPhase::CommittingPrevious => 5,
        SwitchbackPhase::Complete => 6,
    };
    old.lifecycle.accepting = true;
    old.lifecycle.revision += 1;
    candidate.lifecycle.accepting = false;
    candidate.lifecycle.revision += 1;
    candidate.lifecycle.in_flight = 0;
    let value = json!({
        "schema": "aster.online-switchback.v1", "revision": revision,
        "plan": plan, "phase": phase,
        "opened": (revision >= 1).then_some(old),
        "closed": (revision >= 3).then_some(&candidate),
        "drained": (revision >= 4 && phase != SwitchbackPhase::DrainBudgetExhausted).then_some(&candidate),
        "switch_started_ms": (revision >= 1).then_some(2_000),
        "drain_started_ms": (revision >= 2).then_some(2_100),
    });
    let journal: SwitchbackJournal = serde_json::from_value(value).unwrap();
    assert!(journal.valid());
    journal
}

#[test]
fn reverse_observation_checks_all_phases_and_directions_without_repair() {
    for previous in [ReleaseSlot::Blue, ReleaseSlot::Green] {
        for phase in [
            SwitchbackPhase::PreparingPrevious,
            SwitchbackPhase::SwitchingBack,
            SwitchbackPhase::ClosingCandidate,
            SwitchbackPhase::DrainingCandidate,
            SwitchbackPhase::RetiringCandidate,
            SwitchbackPhase::DrainBudgetExhausted,
            SwitchbackPhase::CommittingPrevious,
            SwitchbackPhase::Complete,
        ] {
            for disk_slot in [ReleaseSlot::Blue, ReleaseSlot::Green] {
                for live_slot in [ReleaseSlot::Blue, ReleaseSlot::Green] {
                    let (_temporary, layout) = crate::proxy_disk::tests::fixture();
                    let journal = journal(&layout, previous, phase);
                    let disk = crate::proxy_disk::InstalledUpstreams::open(&layout).unwrap();
                    disk.prepare(disk_slot, deadline()).unwrap();
                    let original = std::fs::read(layout.caddy_upstreams()).unwrap();
                    let (mut client, handle) = server(vec![served(config(live_slot))]);
                    client.disk = Some(disk);
                    let expected = match phase {
                        SwitchbackPhase::PreparingPrevious => {
                            disk_slot != previous && live_slot != previous
                        }
                        SwitchbackPhase::SwitchingBack => {
                            live_slot != previous || disk_slot == previous
                        }
                        _ => disk_slot == previous && live_slot == previous,
                    };
                    assert_eq!(
                        client.observe_switchback(&journal, deadline()).is_ok(),
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
fn reverse_observation_rejects_drift_and_concurrent_disk_change() {
    for previous in [ReleaseSlot::Blue, ReleaseSlot::Green] {
        for concurrent in [false, true] {
            let (_temporary, layout) = crate::proxy_disk::tests::fixture();
            let journal = journal(&layout, previous, SwitchbackPhase::SwitchingBack);
            let candidate = journal.plan().candidate.slot;
            let disk = crate::proxy_disk::InstalledUpstreams::open(&layout).unwrap();
            disk.prepare(candidate, deadline()).unwrap();
            let path = layout.caddy_upstreams();
            let mut live = config(candidate);
            if !concurrent {
                live["apps"]["http"]["servers"]["srv0"]["listen"] = json!([":8443"]);
            }
            let (mut client, handle) = server_with_inspection(vec![served(live)], move |_| {
                if concurrent {
                    std::fs::write(
                        &path,
                        crate::maintenance_executor::render_upstreams(previous, Default::default()),
                    )
                    .unwrap();
                }
            });
            client.disk = Some(disk);
            assert!(client.observe_switchback(&journal, deadline()).is_err());
            assert!(handle.join().unwrap()[0].starts_with("GET "));
            client
                .disk
                .as_ref()
                .unwrap()
                .assert_target(if concurrent { previous } else { candidate }, deadline())
                .unwrap();
        }
    }
}

#[test]
fn reverse_switch_persists_old_slot_before_cas_and_confirms_all_business_entries() {
    for previous in [ReleaseSlot::Blue, ReleaseSlot::Green] {
        let (_temporary, layout) = crate::proxy_disk::tests::fixture();
        let journal = journal(&layout, previous, SwitchbackPhase::SwitchingBack);
        let candidate = journal.plan().candidate.slot;
        let disk = crate::proxy_disk::InstalledUpstreams::open(&layout).unwrap();
        disk.prepare(candidate, deadline()).unwrap();
        let path = layout.caddy_upstreams();
        let expected = crate::maintenance_executor::render_upstreams(previous, Default::default());
        let (mut client, handle) = server_with_inspection(
            vec![
                served(config(candidate)),
                response("", "200 OK", "unused"),
                served(config(previous)),
            ],
            move |request| {
                if request.starts_with("POST ") {
                    assert_eq!(std::fs::read(&path).unwrap(), expected.as_bytes());
                    assert!(request.to_ascii_lowercase().contains("if-match:"));
                    let body = request.split_once("\r\n\r\n").unwrap().1;
                    assert_eq!(
                        inspect(&serde_json::from_str(body).unwrap()).unwrap().slot,
                        previous
                    );
                }
            },
        );
        client.disk = Some(disk);
        let observed = client.observe_switchback(&journal, deadline()).unwrap();
        let actual = client
            .switch_to(
                observed,
                previous,
                journal.plan().candidate_budget.request_budget_ms,
                deadline(),
            )
            .unwrap();
        assert_eq!(CaddyClient::snapshot(&actual).slot, previous);
        client
            .disk
            .as_ref()
            .unwrap()
            .assert_target(previous, deadline())
            .unwrap();
        let requests = handle.join().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[0].starts_with("GET "));
        assert!(requests[1].starts_with("POST "));
        assert!(requests[2].starts_with("GET "));
    }
}

#[test]
fn reverse_observation_requires_valid_intent_disk_and_remaining_budget() {
    let (_temporary, layout) = crate::proxy_disk::tests::fixture();
    let journal = journal(&layout, ReleaseSlot::Blue, SwitchbackPhase::SwitchingBack);
    let mut client = CaddyClient::build("http://127.0.0.1:1/config/".into()).unwrap();
    assert!(client.observe_switchback(&journal, deadline()).is_err());
    client.disk = Some(crate::proxy_disk::InstalledUpstreams::open(&layout).unwrap());
    assert!(client.observe_switchback(&journal, Instant::now()).is_err());
    let mut value = serde_json::to_value(&journal).unwrap();
    value["revision"] = 99.into();
    let invalid = serde_json::from_value(value).unwrap();
    assert!(client.observe_switchback(&invalid, deadline()).is_err());
    let mut live = journal.plan().proxy.clone();
    live.stream_close_delay_ms -= 1;
    assert!(!journal.accepts_proxy(&live, live.slot));
    live = journal.plan().proxy.clone();
    live.configuration_sha256 = "0".repeat(64);
    assert!(!journal.accepts_proxy(&live, live.slot));
}
