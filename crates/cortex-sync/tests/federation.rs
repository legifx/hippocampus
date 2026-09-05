//! M7's Definition of Done, against the reference peer over the real protocol.
//!
//! 1. Seventy-two hours offline, then a clean reconcile: every event arrives once, in order.
//! 2. Replaying the entire outbox twice changes nothing the peer knows.
//! 3. The backlog drains without exceeding the configured rate, and the worker's memory does
//!    not grow with the size of the backlog.
//!
//! The clock is synthetic — `Worker::tick(now_ms)` — so seventy-two hours take a second and
//! the rate limit is asserted exactly rather than approximately. The peer is the real binary,
//! spoken to over stdio by the real plugin transport, or through a real directory by the real
//! file-drop transport.

use cortex_core::config::{FederationConfig, FederationTransport};
use cortex_core::model::{Action, Actor};
use cortex_core::plugins::PluginDecl;
use cortex_store::Store;
use cortex_sync::filedrop::FileDrop;
use cortex_sync::plugin::Plugin;
use cortex_sync::{TickOutcome, Worker};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

const PEER: &str = env!("CARGO_BIN_EXE_hc-test-peer");

/// One event every thirty seconds for three days, nights included: what three days of opens,
/// saves and notes look like when nobody has been reachable to hear about it. The number is
/// a choice — the criterion names the hours, not the rate — and it is kept where the test
/// still runs in seconds: the reference peer rewrites its whole state per batch, which is
/// honest for a dumb peer and quadratic for a large one.
const HOURS_OFFLINE: i64 = 72;
const EVENTS_PER_HOUR: i64 = 120;

fn config(transport: FederationTransport) -> FederationConfig {
    FederationConfig {
        enabled: true,
        transport,
        batch_size: 500,
        batch_interval_ms: 1_000,
        // A quarter of an hour, three times the default: every unreachable attempt here starts
        // a process, and the three days are spent mostly waiting either way.
        max_backoff_ms: 900_000,
        ..FederationConfig::default()
    }
}

fn seeded_store(events: i64) -> Arc<Store> {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mut ts = 1_700_000_000;
    for i in 0..events {
        ts += 10;
        // Mostly memories, with a note now and then: the things a peer is for.
        let (action, meta) = if i % 7 == 0 {
            (
                Action::Annotate,
                serde_json::json!({ "text": format!("note {i}") }),
            )
        } else {
            (
                Action::Remember,
                serde_json::json!({ "scope": "global", "kind": "fact", "text": format!("fact {i}") }),
            )
        };
        store
            .append_episode(ts, &Actor::Agent("t".into()), action, None, None, &meta)
            .unwrap();
    }
    store
}

fn peer_state(path: &Path) -> serde_json::Value {
    let out = Command::new(PEER)
        .args(["show", "--state"])
        .arg(path)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

/// A plugin declaration that runs the peer only while `flag` exists — the peer being "up".
fn gated_peer(dir: &Path, state: &Path, flag: &Path) -> PluginDecl {
    let script = dir.join("peer.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n[ -e {flag:?} ] || exit 3\nexec {PEER:?} push --state {state:?}\n",
            flag = flag.display(),
            state = state.display(),
        ),
    )
    .unwrap();
    PluginDecl::from_toml(
        &format!("kind = \"sync\"\ncommand = \"sh\"\nargs = [{script:?}]\ntimeout_ms = 10000\n",
            script = script.display()),
        &PathBuf::from("<test>"),
    )
    .unwrap()
}

/// Drive the worker on a synthetic clock until it is idle. Returns the (time, size) of every
/// batch that went out, in order.
fn drain(worker: &mut Worker, now: &mut i64, max_ticks: usize) -> Vec<(i64, usize)> {
    let mut sends = Vec::new();
    for _ in 0..max_ticks {
        let report = worker.tick(*now);
        match report.outcome {
            TickOutcome::Sent => sends.push((*now, report.sent)),
            TickOutcome::Waiting { until_ms } => *now = until_ms,
            TickOutcome::Idle => return sends,
            TickOutcome::Unreachable(_) => {
                // The worker has set its backoff; the next tick says how long.
                if let TickOutcome::Waiting { until_ms } = worker.tick(*now).outcome {
                    *now = until_ms;
                }
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
    panic!("did not drain within {max_ticks} ticks");
}

fn rss_kib() -> u64 {
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    let pages: u64 = statm.split_whitespace().nth(1).and_then(|p| p.parse().ok()).unwrap_or(0);
    pages * 4
}

#[test]
fn three_days_offline_then_everything_arrives_once_in_order_under_the_rate() {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("peer.json");
    let flag = tmp.path().join("up");
    let events = HOURS_OFFLINE * EVENTS_PER_HOUR;
    let store = seeded_store(events);
    let cfg = config(FederationTransport::Plugin);
    let mut worker = Worker::new(
        store.clone(),
        Box::new(Plugin::new("gate", gated_peer(tmp.path(), &state, &flag))),
        cfg.clone(),
    )
    .unwrap();

    // Offline: the peer is down for the whole three days. The outbox fills, nothing is
    // dead-lettered, and the worker backs off to its ceiling rather than hammering.
    let mut now = 0i64;
    let mut unreachable = 0;
    while now < HOURS_OFFLINE * 3_600_000 {
        match worker.tick(now).outcome {
            TickOutcome::Unreachable(_) => unreachable += 1,
            TickOutcome::Waiting { until_ms } => now = until_ms,
            other => panic!("{other:?}"),
        }
    }
    let counts = store.outbox_counts().unwrap();
    assert_eq!(counts.pending, events, "every episode became exactly one queued event");
    assert_eq!(counts.dead, 0, "silence is not a refusal");
    assert_eq!(counts.episodes_behind, 0);
    assert!(
        unreachable <= 20 + HOURS_OFFLINE * 60,
        "{unreachable} attempts in three days is more than one a minute"
    );
    assert!(!state.exists(), "nothing reached the peer while it was down");

    // Reconnect.
    std::fs::write(&flag, "").unwrap();
    let before = rss_kib();
    let sends = drain(&mut worker, &mut now, 10_000);
    let after = rss_kib();

    // The rate: never more than a batch, never closer than the interval.
    assert!(sends.iter().all(|(_, n)| *n <= cfg.batch_size), "{sends:?}");
    assert!(
        sends.windows(2).all(|w| w[1].0 - w[0].0 >= cfg.batch_interval_ms as i64),
        "two batches closer than {} ms: {:?}",
        cfg.batch_interval_ms,
        sends.windows(2).find(|w| w[1].0 - w[0].0 < cfg.batch_interval_ms as i64)
    );
    let expected_batches = (events as usize).div_ceil(cfg.batch_size);
    assert_eq!(sends.len(), expected_batches);

    // The peer: everything, once, in order.
    let peer = peer_state(&state);
    let seen = peer["events"].as_object().unwrap();
    assert_eq!(seen.len(), events as usize);
    assert!(seen.values().all(|e| e["seen"] == 1), "a duplicate reached a peer that keeps none");
    let arrivals = peer["arrivals"].as_array().unwrap();
    assert_eq!(arrivals.len(), events as usize, "no duplicate arrivals");
    let clocks: Vec<i64> = arrivals
        .iter()
        .map(|id| seen[id.as_str().unwrap()]["lamport"].as_i64().unwrap())
        .collect();
    assert!(clocks.windows(2).all(|w| w[0] < w[1]), "arrivals are not in clock order");
    let stamps: Vec<i64> = arrivals
        .iter()
        .map(|id| seen[id.as_str().unwrap()]["ts"].as_i64().unwrap())
        .collect();
    assert!(stamps.windows(2).all(|w| w[0] <= w[1]), "clock order is not history order");

    // Memory: draining a backlog of this size must not cost memory proportional to it. A
    // batch is at most 500 events; the number below is a generous ceiling on that, not a
    // measurement of it, and it fails only when something holds the whole backlog at once.
    let grown_mib = after.saturating_sub(before) / 1024;
    assert!(grown_mib < 64, "the worker grew by {grown_mib} MiB draining {events} events");

    assert_eq!(store.outbox_counts().unwrap().pending, 0);
    assert_eq!(store.sync_state(&cfg.peer).unwrap().last_acked_lamport, events);
}

#[test]
fn replaying_the_entire_outbox_twice_changes_nothing_on_the_peer() {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("peer.json");
    let flag = tmp.path().join("up");
    std::fs::write(&flag, "").unwrap();
    let store = seeded_store(1_200);
    let cfg = config(FederationTransport::Plugin);
    let mut worker = Worker::new(
        store.clone(),
        Box::new(Plugin::new("gate", gated_peer(tmp.path(), &state, &flag))),
        cfg,
    )
    .unwrap();
    let mut now = 0;
    drain(&mut worker, &mut now, 1_000);
    let first = peer_state(&state);

    for round in 1..=2 {
        store.outbox_reset().unwrap();
        assert_eq!(store.outbox_counts().unwrap().pending, 0);
        drain(&mut worker, &mut now, 1_000);
        let again = peer_state(&state);
        // What the peer knows is identical: the same ids, the same payloads, the same clocks
        // on first sight. Only the arrival log grew, which is the peer *observing* the
        // duplicate that its knowledge did not absorb.
        let strip = |v: &serde_json::Value| -> serde_json::Value {
            let mut events = v["events"].clone();
            for e in events.as_object_mut().unwrap().values_mut() {
                e.as_object_mut().unwrap().remove("seen");
            }
            events
        };
        assert_eq!(strip(&first), strip(&again), "replay {round} changed what the peer knows");
        assert!(
            again["events"].as_object().unwrap().values().all(|e| e["seen"] == round + 1),
            "replay {round}: every event arrived exactly once more"
        );
    }
}

#[test]
fn the_file_drop_transport_reaches_the_same_peer_state_as_the_plugin() {
    let tmp = tempfile::tempdir().unwrap();
    let inbox = tmp.path().join("share").join("inbox");
    let state = tmp.path().join("peer.json");
    let store = seeded_store(700);
    let cfg = config(FederationTransport::FileDrop);
    let mut worker = Worker::new(store.clone(), Box::new(FileDrop::new(&inbox)), cfg).unwrap();

    // The share is not mounted. Fail closed: nothing is written anywhere.
    let mut now = 0;
    assert!(matches!(worker.tick(now).outcome, TickOutcome::Unreachable(_)));
    assert!(!tmp.path().join("share").exists());

    std::fs::create_dir_all(&inbox).unwrap();
    now += 60_000;
    let sends = drain(&mut worker, &mut now, 100);
    assert_eq!(sends.iter().map(|s| s.1).collect::<Vec<_>>(), [500, 200]);

    let ingested = Command::new(PEER)
        .args(["ingest", "--state"])
        .arg(&state)
        .arg("--dir")
        .arg(&inbox)
        .output()
        .unwrap();
    assert!(ingested.status.success(), "{}", String::from_utf8_lossy(&ingested.stderr));
    assert_eq!(String::from_utf8_lossy(&ingested.stdout).trim(), "700");
    let peer = peer_state(&state);
    assert_eq!(peer["events"].as_object().unwrap().len(), 700);
    assert!(std::fs::read_dir(&inbox).unwrap().next().is_none(), "the peer consumed its inbox");

    // A resend overwrites by name and the peer, ingesting again, sees each once more — never
    // a second file per event.
    store.outbox_reset().unwrap();
    drain(&mut worker, &mut now, 100);
    assert_eq!(std::fs::read_dir(&inbox).unwrap().count(), 700);
}

#[test]
fn a_peer_asking_for_quiet_is_left_alone_for_that_long() {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("peer.json");
    let decl = PluginDecl::from_toml(
        &format!(
            "kind = \"sync\"\ncommand = {PEER:?}\nargs = [\"push\", \"--state\", {state:?}, \"--quiet-ms\", \"3600000\", \"--lamport\", \"9000\"]\n",
            state = state.display()
        ),
        &PathBuf::from("<test>"),
    )
    .unwrap();
    let store = seeded_store(1_000);
    let mut worker =
        Worker::new(store.clone(), Box::new(Plugin::new("quiet", decl)), config(FederationTransport::Plugin))
            .unwrap();
    assert_eq!(worker.tick(0).outcome, TickOutcome::Sent);
    assert_eq!(worker.tick(1_000).outcome, TickOutcome::Waiting { until_ms: 3_600_000 });
    assert_eq!(worker.tick(3_600_000).outcome, TickOutcome::Sent);
    assert!(store.lamport().unwrap() >= 9_000, "the peer's clock was observed");
}
