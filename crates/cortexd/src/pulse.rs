//! The live pulse: what is being asked of the brain and written to it right now, by whom, and
//! in which zone.
//!
//! The Agent View's live half needs three things the index does not keep: the last few hundred
//! calls in the order they happened, how hot each zone is *now*, and which agents are busy. The
//! audit log is the record and stays the record; this is a view over the same traffic, held in
//! memory, bounded, and gone when the daemon restarts. Nothing here is written to the store,
//! so none of it can be needed by a rebuild (R2).
//!
//! Cost is the whole design constraint. `record` runs on every dispatched call, so it takes one
//! short lock, walks at most a bounded part of the answer for paths, and never touches the
//! database — the zone table is copied in at most every two minutes and matched by prefix.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use cortex_rpc::protocol::{method, AgentPulse, BrainActivityResult, PulseEvent, ZoneHeat};
use cortex_store::Store;

/// How many events the ring keeps. A busy agent makes a few calls a second; this is minutes.
const RING: usize = 400;
/// The heat halves every five minutes: long enough that a zone stays lit while an agent works
/// in it, short enough that yesterday's work is not what the heatmap shows.
pub const HALF_LIFE_MS: i64 = 5 * 60 * 1000;
/// Paths per event. The scene lights these; more would be noise at the size it draws them.
const MAX_PATHS: usize = 6;
/// JSON values visited per call while looking for paths. Bounds the walk over a large answer.
const MAX_VISIT: usize = 400;
const ZONE_REFRESH: std::time::Duration = std::time::Duration::from_secs(120);

#[derive(Clone)]
struct ZoneRef {
    id: i64,
    root: String,
    name: String,
    hue: u16,
}

#[derive(Default)]
struct Heat {
    reads: f64,
    writes: f64,
    /// When the counters were last brought up to date.
    decayed_at: i64,
    /// When the zone was last touched.
    last_ms: i64,
    hue: Option<u16>,
}

impl Heat {
    fn decay_to(&mut self, now_ms: i64) {
        if self.decayed_at == 0 || now_ms <= self.decayed_at {
            self.decayed_at = self.decayed_at.max(now_ms);
            return;
        }
        let factor = 0.5f64.powf((now_ms - self.decayed_at) as f64 / HALF_LIFE_MS as f64);
        self.reads *= factor;
        self.writes *= factor;
        self.decayed_at = now_ms;
    }
}

struct AgentState {
    stamps: VecDeque<i64>,
    last_ms: i64,
    last_method: String,
}

#[derive(Default)]
struct Inner {
    seq: u64,
    ring: VecDeque<PulseEvent>,
    heat: HashMap<String, Heat>,
    agents: HashMap<String, AgentState>,
    zones: Vec<ZoneRef>,
    zones_at: Option<Instant>,
}

#[derive(Default)]
pub struct Pulse {
    inner: Mutex<Inner>,
}

/// Methods that are the window asking about itself or the daemon, not about the brain. Drawing
/// them would make the live view a picture of its own polling.
fn is_housekeeping(m: &str) -> bool {
    matches!(
        m,
        method::PING
            | method::STATUS
            | method::STATS_OVERVIEW
            | method::ZONES_LIST
            | method::AUDIT_RECENT
            | method::AGENT_SESSIONS
            | method::AGENT_STATUS
            | method::BRAIN_ACTIVITY
            | method::MODEL_STATUS
            | method::WATCH_STATUS
            | method::EXTRACT_STATUS
            | method::SYNC_STATUS
            | method::ACTIVITY_PING
            | method::IDENTIFY
            | method::FS_PLACES
            | method::APPS_FOR_PATH
            | method::FS_STAT
            | method::BRAIN_GRAPH
            | method::SEARCH_UPDATE
            | method::VIEW_UPDATE
            | method::OPERATION_CANCEL
            | method::AGENT_SESSION_BEGIN
            | method::AGENT_SESSION_END
    )
}

pub fn kind_of(m: &str, ok: bool) -> &'static str {
    if !ok {
        return "refused";
    }
    match m {
        method::BRAIN_REMEMBER
        | method::BRAIN_ANNOTATE
        | method::BRAIN_PIN
        | method::BRAIN_LINK
        | method::BRAIN_FORGET
        | method::BRAIN_FORGET_MEMORY
        | method::BRAIN_SUPERSEDE
        | method::BRAIN_OUTCOME
        | method::BRAIN_CONSOLIDATE
        | method::FS_WRITE
        | method::FS_MOVE
        | method::FS_TRASH
        | method::FS_MKDIR
        | method::FS_RENAME
        | method::FS_TRANSFER
        | method::FS_EXTRACT
        | method::UNDO
        | method::EPISODE_APPEND => "write",
        method::SEARCH
        | method::BRAIN_SEARCH
        | method::BRAIN_RECALL
        | method::BRAIN_CONTEXT
        | method::BRAIN_ASK => "search",
        _ => "read",
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// The first line a person would recognise the call by.
fn label_of(params: Option<&serde_json::Value>) -> String {
    let Some(p) = params else {
        return String::new();
    };
    for key in ["query", "task", "question", "cue", "text", "note", "target", "path", "since"] {
        if let Some(v) = p.get(key) {
            let text = match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                _ => continue,
            };
            let text = if key == "path" || key == "target" {
                text.rsplit('/').next().unwrap_or(&text).to_string()
            } else {
                text
            };
            let line = text.lines().next().unwrap_or("").trim();
            let mut out: String = line.chars().take(90).collect();
            if line.chars().count() > 90 {
                out.push('…');
            }
            return out;
        }
    }
    String::new()
}

/// Absolute paths mentioned in a JSON value, in the order they appear, at most `MAX_PATHS`.
fn collect_paths(value: &serde_json::Value, out: &mut Vec<String>, seen: &mut HashSet<String>) {
    let mut stack = vec![value];
    let mut visited = 0usize;
    while let Some(v) = stack.pop() {
        visited += 1;
        if visited > MAX_VISIT || out.len() >= MAX_PATHS {
            return;
        }
        match v {
            serde_json::Value::String(s) => {
                if s.starts_with('/') && s.len() < 1024 && !s.contains('\n') && seen.insert(s.clone())
                {
                    out.push(s.clone());
                }
            }
            serde_json::Value::Array(items) => {
                for item in items.iter().rev() {
                    stack.push(item);
                }
            }
            serde_json::Value::Object(map) => {
                // Text fields are content, not addresses; skipping them keeps the walk cheap and
                // keeps a quoted path inside a note from lighting a zone it was only mentioned in.
                let mut children: Vec<&serde_json::Value> = map
                    .iter()
                    .filter(|(k, _)| {
                        !matches!(
                            k.as_str(),
                            "text" | "snippet" | "content" | "rendered" | "chunk_text" | "body"
                        )
                    })
                    .map(|(_, v)| v)
                    .collect();
                children.reverse();
                stack.extend(children);
            }
            _ => {}
        }
    }
}

/// A memory's scope names its project — by zone id or by name, depending on who wrote it down;
/// that is the zone it belongs to when no path does.
fn scope_zone(values: &[Option<&serde_json::Value>]) -> Option<String> {
    values.iter().flatten().find_map(|v| {
        let scope = v.get("scope")?;
        let text = match scope {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Object(m) => m.get("project").and_then(|p| match p {
                serde_json::Value::String(s) => Some(s.clone()),
                serde_json::Value::Number(n) => Some(n.to_string()),
                _ => None,
            }).map(|p| format!("project:{p}"))?,
            _ => return None,
        };
        text.strip_prefix("project:").map(|s| s.to_string())
    })
}

impl Pulse {
    pub fn new() -> Self {
        Self::default()
    }

    fn refresh_zones(&self, store: &Store) {
        let stale = {
            let inner = self.inner.lock().unwrap();
            inner.zones_at.is_none_or(|at| at.elapsed() > ZONE_REFRESH)
        };
        if !stale {
            return;
        }
        // Read outside the lock: a slow database must not hold up every other call's record.
        let zones = match store.zones() {
            Ok(z) => z,
            Err(e) => {
                tracing::debug!(error = %e, "pulse: zones unavailable");
                Vec::new()
            }
        };
        let mut refs: Vec<ZoneRef> = zones
            .into_iter()
            .map(|z| ZoneRef {
                id: z.id.map(|i| i.0).unwrap_or_default(),
                root: z.root_path.display().to_string(),
                name: z.name,
                hue: z.color_hue,
            })
            .collect();
        // Longest root first, so the first prefix that matches is the most specific zone.
        refs.sort_by(|a, b| b.root.len().cmp(&a.root.len()));
        let mut inner = self.inner.lock().unwrap();
        inner.zones = refs;
        inner.zones_at = Some(Instant::now());
    }

    fn zone_for_path<'a>(zones: &'a [ZoneRef], path: &str) -> Option<&'a ZoneRef> {
        zones.iter().find(|z| {
            path.starts_with(&z.root)
                && (path.len() == z.root.len() || path.as_bytes().get(z.root.len()) == Some(&b'/'))
        })
    }

    /// Note one dispatched call. Cheap by construction; see the module comment.
    pub fn record(
        &self,
        store: &Store,
        actor: &str,
        method_name: &str,
        ok: bool,
        params: Option<&serde_json::Value>,
        answer: Option<&serde_json::Value>,
    ) {
        if is_housekeeping(method_name) {
            return;
        }
        // The window browsing directories is a person reading, and it is drawn — but not the
        // window's thumbnail and preview traffic, which is one call per visible row.
        if actor == "human" && matches!(method_name, method::FS_READ | method::NODE_GET | method::NODE_WHY | method::BRAIN_NEIGHBORS) {
            return;
        }
        self.refresh_zones(store);

        let mut paths = Vec::new();
        let mut seen = HashSet::new();
        if let Some(p) = params {
            collect_paths(p, &mut paths, &mut seen);
        }
        if let Some(a) = answer {
            collect_paths(a, &mut paths, &mut seen);
        }
        let kind = kind_of(method_name, ok);
        let label = label_of(params);
        let ts = now_ms();

        let mut inner = self.inner.lock().unwrap();
        // The zone the call touched most often; ties go to the first mentioned.
        let mut tally: Vec<(String, u16, usize)> = Vec::new();
        for path in &paths {
            if let Some(z) = Self::zone_for_path(&inner.zones, path) {
                match tally.iter_mut().find(|(n, _, _)| n == &z.name) {
                    Some(t) => t.2 += 1,
                    None => tally.push((z.name.clone(), z.hue, 1)),
                }
            }
        }
        let mut best = tally.iter().max_by_key(|(_, _, c)| *c).map(|(n, h, _)| (n.clone(), Some(*h)));
        if best.is_none() {
            if let Some(scope) = scope_zone(&[params, answer]) {
                let by_id = scope.parse::<i64>().ok();
                best = match inner
                    .zones
                    .iter()
                    .find(|z| by_id == Some(z.id) || z.name.eq_ignore_ascii_case(&scope))
                {
                    Some(z) => Some((z.name.clone(), Some(z.hue))),
                    None if by_id.is_none() => Some((scope, None)),
                    None => None,
                };
            }
        }

        inner.seq += 1;
        let seq = inner.seq;
        let (zone, hue) = match best {
            Some((z, h)) => (Some(z), h),
            None => (None, None),
        };

        if let Some(z) = &zone {
            let heat = inner.heat.entry(z.clone()).or_default();
            heat.decay_to(ts);
            heat.last_ms = ts;
            match kind {
                "write" => heat.writes += 1.0,
                "refused" => {}
                _ => heat.reads += 1.0,
            }
            if hue.is_some() {
                heat.hue = hue;
            }
        }

        let agent = inner.agents.entry(actor.to_string()).or_insert_with(|| AgentState {
            stamps: VecDeque::new(),
            last_ms: 0,
            last_method: String::new(),
        });
        agent.stamps.push_back(ts);
        while agent.stamps.front().is_some_and(|t| ts - t > 2 * HALF_LIFE_MS) {
            agent.stamps.pop_front();
        }
        agent.last_ms = ts;
        agent.last_method = method_name.to_string();

        inner.ring.push_back(PulseEvent {
            seq,
            ts_ms: ts,
            actor: actor.to_string(),
            method: method_name.to_string(),
            kind: kind.to_string(),
            zone,
            hue,
            label,
            paths,
        });
        while inner.ring.len() > RING {
            inner.ring.pop_front();
        }
    }

    pub fn snapshot(&self, after: u64, limit: usize) -> BrainActivityResult {
        let now = now_ms();
        let mut inner = self.inner.lock().unwrap();
        let events: Vec<PulseEvent> = {
            let newer: Vec<&PulseEvent> = inner.ring.iter().filter(|e| e.seq > after).collect();
            let skip = newer.len().saturating_sub(limit);
            newer.into_iter().skip(skip).cloned().collect()
        };
        let mut zones: Vec<ZoneHeat> = inner
            .heat
            .iter_mut()
            .map(|(name, h)| {
                h.decay_to(now);
                ZoneHeat {
                    zone: name.clone(),
                    hue: h.hue,
                    reads: h.reads,
                    writes: h.writes,
                    heat: h.reads + 2.0 * h.writes,
                    last_ms: h.last_ms,
                }
            })
            .filter(|z| z.heat >= 0.01)
            .collect();
        zones.sort_by(|a, b| b.heat.partial_cmp(&a.heat).unwrap_or(std::cmp::Ordering::Equal));
        // A zone that has cooled to nothing is dropped, so the map does not grow for ever.
        inner.heat.retain(|_, h| h.reads + 2.0 * h.writes >= 0.01);

        let mut agents: Vec<AgentPulse> = inner
            .agents
            .iter()
            .filter(|(_, a)| now - a.last_ms < 2 * HALF_LIFE_MS)
            .map(|(actor, a)| AgentPulse {
                actor: actor.clone(),
                calls: a.stamps.iter().filter(|t| now - **t <= 2 * HALF_LIFE_MS).count() as u64,
                last_ms: a.last_ms,
                last_method: a.last_method.clone(),
            })
            .collect();
        agents.sort_by(|a, b| b.last_ms.cmp(&a.last_ms));
        let active_agents = agents.iter().filter(|a| a.actor.starts_with("agent:")).count();
        let calls = agents.iter().map(|a| a.calls).sum();

        BrainActivityResult {
            active_agents,
            calls,
            seq: inner.seq,
            now_ms: now,
            half_life_ms: HALF_LIFE_MS,
            events,
            zones,
            agents,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn zones() -> Vec<ZoneRef> {
        let mut z = vec![
            ZoneRef { id: 1, root: "/w/Projects".into(), name: "Projects".into(), hue: 10 },
            ZoneRef { id: 2, root: "/w/Projects/alpha".into(), name: "alpha".into(), hue: 200 },
            ZoneRef { id: 3, root: "/w/Projects/alphabet".into(), name: "alphabet".into(), hue: 300 },
        ];
        z.sort_by(|a, b| b.root.len().cmp(&a.root.len()));
        z
    }

    #[test]
    fn the_most_specific_zone_wins_and_a_shared_prefix_is_not_a_parent() {
        let z = zones();
        assert_eq!(Pulse::zone_for_path(&z, "/w/Projects/alpha/src/x.rs").unwrap().name, "alpha");
        assert_eq!(Pulse::zone_for_path(&z, "/w/Projects/alphabet/a").unwrap().name, "alphabet");
        assert_eq!(Pulse::zone_for_path(&z, "/w/Projects/beta").unwrap().name, "Projects");
        assert!(Pulse::zone_for_path(&z, "/elsewhere").is_none());
    }

    #[test]
    fn paths_are_found_in_answers_but_not_inside_text() {
        let answer = json!({
            "hits": [{"path": "/a/one.md", "snippet": "/not/this"}, {"path": "/a/two.md"}],
            "text": "see /not/that either"
        });
        let mut out = Vec::new();
        collect_paths(&answer, &mut out, &mut HashSet::new());
        assert_eq!(out, vec!["/a/one.md".to_string(), "/a/two.md".to_string()]);
    }

    #[test]
    fn the_walk_stops_at_its_bounds() {
        let many: Vec<_> = (0..1000).map(|i| json!({ "path": format!("/p/{i}") })).collect();
        let mut out = Vec::new();
        collect_paths(&json!(many), &mut out, &mut HashSet::new());
        assert_eq!(out.len(), MAX_PATHS);
    }

    #[test]
    fn a_scope_names_its_project_by_id_or_by_name() {
        assert_eq!(scope_zone(&[Some(&json!({"scope": "project:7"}))]).as_deref(), Some("7"));
        assert_eq!(
            scope_zone(&[None, Some(&json!({"scope": {"project": "takt"}}))]).as_deref(),
            Some("takt")
        );
        assert_eq!(scope_zone(&[Some(&json!({"scope": "global"}))]), None);
    }

    #[test]
    fn heat_halves_over_one_half_life() {
        let mut h = Heat { reads: 8.0, writes: 2.0, decayed_at: 1_000, last_ms: 1_000, hue: None };
        h.decay_to(1_000 + HALF_LIFE_MS);
        assert!((h.reads - 4.0).abs() < 1e-9);
        assert!((h.writes - 1.0).abs() < 1e-9);
    }

    #[test]
    fn kinds_follow_the_method_and_a_refusal_is_a_refusal() {
        assert_eq!(kind_of(method::BRAIN_REMEMBER, true), "write");
        assert_eq!(kind_of(method::BRAIN_CONTEXT, true), "search");
        assert_eq!(kind_of(method::BRAIN_NEIGHBORS, true), "read");
        assert_eq!(kind_of(method::BRAIN_REMEMBER, false), "refused");
    }

    #[test]
    fn labels_are_short_and_a_path_is_its_file_name() {
        assert_eq!(label_of(Some(&json!({"path": "/a/b/c.txt"}))), "c.txt");
        let long = "x".repeat(200);
        assert_eq!(label_of(Some(&json!({ "query": long }))).chars().count(), 91);
    }
}
