//! The `brain.*` tools: what an agent may write that is not a file.
//!
//! Two things are being checked. First that they work — annotate, link, pin, traverse, and read
//! a timeline. Second, and more importantly, that they are **gated by the kill switch but not
//! by the write allow-list**, which is the line rule R6 draws and the easiest one to get wrong
//! in either direction.

use cortex_agent::fs_tools::{Surface, ToolError};
use cortex_agent::plans::PlanBook;
use cortex_agent::policy::{Policy, Refusal};
use cortex_agent::Auditor;
use cortex_core::config::AgentConfig;
use cortex_core::ids::NodeId;
use cortex_core::model::{Actor, NodeKind};
use cortex_core::tools::*;
use cortex_store::queries::NodeUpsert;
use cortex_store::Store;
use std::path::PathBuf;
use std::sync::Arc;

struct Env {
    _dir: tempfile::TempDir,
    root: PathBuf,
    store: Arc<Store>,
    auditor: Auditor,
    plans: PlanBook,
    policy: Policy,
    trash: cortex_agent::trash::Trash,
}

impl Env {
    /// An index with two indexed files and an empty write allow-list, because the point of most
    /// of these tests is that the brain tools do not consult it.
    fn new() -> (Self, NodeId, NodeId) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("tree");
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();

        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut ids = Vec::new();
        for name in ["report.md", "invoice.pdf"] {
            let path = root.join(name);
            std::fs::write(&path, "content").unwrap();
            ids.push(
                store
                    .upsert_node(
                        &NodeUpsert {
                            kind: NodeKind::File,
                            path,
                            parent_id: None,
                            mime: None,
                            size: 7,
                            mtime: 1,
                            ctime: 1,
                            inode: 0,
                            dev: 0,
                            zone_id: None,
                        },
                        1,
                    )
                    .unwrap(),
            );
        }

        let config = AgentConfig {
            enabled: true,
            write_allowlist: Vec::new(),
            require_dry_run: true,
            undo_journal_depth: 100,
        };
        let auditor = Auditor::new(store.clone(), &Actor::Agent("brain".into()));
        let env = Self {
            policy: Policy::new(&config, vec![]),
            trash: cortex_agent::trash::Trash::at(dir.path().join("Trash")),
            _dir: dir,
            root,
            store,
            auditor,
            plans: PlanBook::new(120),
        };
        (env, ids[0], ids[1])
    }

    fn surface(&self, enabled: bool) -> Surface<'_> {
        self.surface_at(enabled, 5000)
    }

    fn surface_at(&self, enabled: bool, now: i64) -> Surface<'_> {
        Surface {
            policy: &self.policy,
            enabled,
            store: &self.store,
            auditor: &self.auditor,
            plans: &self.plans,
            trash: &self.trash,
            actor: Actor::Agent("brain".into()),
            now,
        }
    }
}

fn refusal(e: ToolError) -> Refusal {
    match e {
        ToolError::Refused(r) => r,
        other => panic!("expected a refusal, got {other}"),
    }
}

#[test]
fn annotating_works_without_any_write_allowlist() {
    let (env, report, _) = Env::new();

    let result = env
        .surface(true)
        .annotate(&AnnotateParams {
            target: Target::Node(report.0),
            tags: vec!["quarterly".into()],
            note: "the numbers in here were revised twice".into(),
        })
        .unwrap();

    assert_eq!(result.target_id, report.0);
    // The allow-list is empty and this still worked: an agent's notes about a file cannot
    // damage the file, so they are not gated on permission to write it (R6).
    let annotations = env.store.annotations_for(report).unwrap();
    assert_eq!(annotations.len(), 1);
    assert_eq!(annotations[0].tags, vec!["quarterly"]);
    assert_eq!(annotations[0].actor, "agent:brain");
}

#[test]
fn a_file_can_be_annotated_by_its_path() {
    let (env, report, _) = Env::new();
    let result = env
        .surface(true)
        .annotate(&AnnotateParams {
            target: Target::Path(env.root.join("report.md")),
            tags: vec![],
            note: "found by path".into(),
        })
        .unwrap();
    assert_eq!(result.target_id, report.0);
}

#[test]
fn every_brain_tool_refuses_while_the_surface_is_off() {
    let (env, a, b) = Env::new();
    let s = env.surface(false);

    let calls: Vec<Box<dyn Fn() -> Result<(), ToolError>>> = vec![
        Box::new(|| {
            s.annotate(&AnnotateParams {
                target: Target::Node(a.0),
                tags: vec![],
                note: "x".into(),
            })
            .map(|_| ())
        }),
        Box::new(|| {
            s.pin(&PinParams { target: Target::Node(a.0) }, true).map(|_| ())
        }),
        Box::new(|| {
            s.link(&LinkParams {
                a: Target::Node(a.0),
                b: Target::Node(b.0),
                edge: None,
                remove: false,
            })
            .map(|_| ())
        }),
        Box::new(|| {
            s.neighbors(&NeighborsParams {
                target: Target::Node(a.0),
                edge_types: vec![],
                depth: None,
                limit: None,
            })
            .map(|_| ())
        }),
        Box::new(|| {
            s.timeline(&TimelineParams {
                from: None,
                to: None,
                actor: None,
                action: None,
                limit: None,
            })
            .map(|_| ())
        }),
        Box::new(|| s.working_set(None).map(|_| ())),
        Box::new(|| {
            s.diff(&DiffParams { since: None, limit: None }).map(|_| ())
        }),
    ];

    for (i, call) in calls.iter().enumerate() {
        let err = call().unwrap_err();
        assert_eq!(
            refusal(err),
            Refusal::SurfaceDisabled,
            "brain tool {i} served a disabled surface"
        );
    }

    let refused = env.store.recent_audit(100, None, true).unwrap();
    assert_eq!(refused.len(), calls.len(), "a refusal went unaudited");
}

#[test]
fn a_link_is_explicit_and_traversable_from_both_ends() {
    let (env, a, b) = Env::new();
    env.surface(true)
        .link(&LinkParams {
            a: Target::Node(a.0),
            b: Target::Node(b.0),
            edge: Some("derived_from".into()),
            remove: false,
        })
        .unwrap();

    for (from, to) in [(a, b), (b, a)] {
        let result = env
            .surface(true)
            .neighbors(&NeighborsParams {
                target: Target::Node(from.0),
                edge_types: vec![],
                depth: None,
                limit: None,
            })
            .unwrap();
        assert_eq!(result.neighbors.len(), 1);
        assert_eq!(result.neighbors[0].node_id, to.0);
        assert_eq!(
            result.neighbors[0].source,
            cortex_core::model::EdgeSource::Explicit,
            "an assertion must be marked explicit so nothing prunes it"
        );
    }
}

#[test]
fn an_unknown_edge_type_is_named_rather_than_guessed() {
    let (env, a, b) = Env::new();
    let err = env
        .surface(true)
        .link(&LinkParams {
            a: Target::Node(a.0),
            b: Target::Node(b.0),
            edge: Some("reminds_me_of".into()),
            remove: false,
        })
        .unwrap_err();
    assert!(err.to_string().contains("similar_to"), "the error must list what is allowed");
}

#[test]
fn a_link_can_be_removed_again() {
    let (env, a, b) = Env::new();
    let params = |remove| LinkParams {
        a: Target::Node(a.0),
        b: Target::Node(b.0),
        edge: Some("imports".into()),
        remove,
    };
    env.surface(true).link(&params(false)).unwrap();
    assert_eq!(env.store.edge_count().unwrap(), 1);
    env.surface(true).link(&params(true)).unwrap();
    assert_eq!(env.store.edge_count().unwrap(), 0);
}

#[test]
fn a_node_cannot_be_linked_to_itself() {
    let (env, a, _) = Env::new();
    let err = env
        .surface(true)
        .link(&LinkParams {
            a: Target::Node(a.0),
            b: Target::Node(a.0),
            edge: None,
            remove: false,
        })
        .unwrap_err();
    assert!(err.to_string().contains("itself"));
}

#[test]
fn pinning_and_forgetting_are_recorded_as_assertions() {
    let (env, a, _) = Env::new();

    env.surface(true)
        .pin(&PinParams { target: Target::Node(a.0) }, true)
        .unwrap();
    assert!(env.store.is_pinned(a).unwrap());

    env.surface(true)
        .pin(&PinParams { target: Target::Node(a.0) }, false)
        .unwrap();
    assert!(!env.store.is_pinned(a).unwrap());

    // Both must be in the episode log, or a rebuild would forget that they happened.
    let episodes = env.store.recent_episodes(50).unwrap();
    let assertions: Vec<_> = episodes
        .iter()
        .filter_map(|e| e.metadata.get("assertion").and_then(|a| a.as_str()))
        .collect();
    assert!(assertions.contains(&"pin"));
    assert!(assertions.contains(&"forget"));
}

#[test]
fn an_unindexed_path_is_refused_rather_than_indexed_on_the_spot() {
    let (env, _, _) = Env::new();
    let err = env
        .surface(true)
        .annotate(&AnnotateParams {
            target: Target::Path(env.root.join("never-seen.txt")),
            tags: vec![],
            note: "x".into(),
        })
        .unwrap_err();
    assert!(matches!(refusal(err), Refusal::Unresolvable { .. }));
}

#[test]
fn a_denied_path_cannot_be_probed_through_annotate() {
    let (env, _, _) = Env::new();
    let err = env
        .surface(true)
        .annotate(&AnnotateParams {
            target: Target::Path(env.root.join(".ssh").join("id_ed25519")),
            tags: vec![],
            note: "x".into(),
        })
        .unwrap_err();
    assert!(
        matches!(refusal(err), Refusal::Denied { .. }),
        "the deny-list must be consulted before the index is"
    );
}

#[test]
fn an_empty_note_is_refused() {
    let (env, a, _) = Env::new();
    let err = env
        .surface(true)
        .annotate(&AnnotateParams {
            target: Target::Node(a.0),
            tags: vec!["tag".into()],
            note: "   ".into(),
        })
        .unwrap_err();
    assert!(err.to_string().contains("says nothing"));
}

#[test]
fn the_timeline_narrows_by_actor_and_time() {
    let (env, a, _) = Env::new();
    env.store
        .append_episode(
            100,
            &Actor::Human,
            cortex_core::model::Action::Open,
            Some(a),
            None,
            &serde_json::json!({}),
        )
        .unwrap();
    env.store
        .append_episode(
            200,
            &Actor::Agent("other".into()),
            cortex_core::model::Action::Open,
            Some(a),
            None,
            &serde_json::json!({}),
        )
        .unwrap();

    let all = env
        .surface(true)
        .timeline(&TimelineParams {
            from: None,
            to: None,
            actor: None,
            action: Some("open".into()),
            limit: None,
        })
        .unwrap();
    assert_eq!(all.episodes.len(), 2);

    let mine = env
        .surface(true)
        .timeline(&TimelineParams {
            from: None,
            to: None,
            actor: Some("human".into()),
            action: None,
            limit: None,
        })
        .unwrap();
    assert_eq!(mine.episodes.len(), 1);

    let recent = env
        .surface(true)
        .timeline(&TimelineParams {
            from: Some(150),
            to: None,
            actor: None,
            action: Some("open".into()),
            limit: None,
        })
        .unwrap();
    assert_eq!(recent.episodes.len(), 1);
}

#[test]
fn diff_resumes_from_its_own_cursor_without_repeating_or_skipping() {
    let (env, a, _) = Env::new();
    for ts in 0..5 {
        env.store
            .append_episode(
                // Deliberately the same timestamp for two of them: a cursor that resumed by
                // time rather than by id would repeat or lose one here.
                if ts < 2 { 100 } else { 100 + ts },
                &Actor::Human,
                cortex_core::model::Action::Open,
                Some(a),
                None,
                &serde_json::json!({ "n": ts }),
            )
            .unwrap();
    }

    let first = env
        .surface(true)
        .diff(&DiffParams { since: None, limit: Some(3) })
        .unwrap();
    assert_eq!(first.changes.len(), 3);
    assert!(first.more);

    let second = env
        .surface(true)
        .diff(&DiffParams { since: Some(first.cursor), limit: Some(10) })
        .unwrap();
    assert!(!second.more);

    let ids: Vec<i64> = first
        .changes
        .iter()
        .chain(second.changes.iter())
        .map(|e| e.id.0)
        .collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(ids.len(), sorted.len(), "the cursor repeated an episode");
    assert_eq!(sorted.len(), 5, "the cursor skipped an episode");
}

#[test]
fn the_working_set_says_what_it_is_made_of() {
    let (env, a, _) = Env::new();
    env.store
        .append_episode(
            4999,
            &Actor::Human,
            cortex_core::model::Action::Open,
            Some(a),
            None,
            &serde_json::json!({}),
        )
        .unwrap();

    let result = env.surface(true).working_set(Some(12)).unwrap();
    assert_eq!(result.nodes.len(), 1);
    assert_eq!(result.nodes[0].node_id, a.0);
    assert_eq!(result.idle_secs, Some(12));
    assert!(
        result.basis.contains("M5"),
        "a tool that is not yet what its name promises must say so"
    );
}

#[test]
fn the_working_set_ignores_what_is_outside_its_window() {
    let (env, a, _) = Env::new();
    env.store
        .append_episode(
            1,
            &Actor::Human,
            cortex_core::model::Action::Open,
            Some(a),
            None,
            &serde_json::json!({}),
        )
        .unwrap();
    // Far enough in the future that the four-hour window has long closed behind that episode.
    let much_later = 1 + cortex_agent::brain_tools::WORKING_SET_WINDOW_SECS * 10;
    assert!(env
        .surface_at(true, much_later)
        .working_set(None)
        .unwrap()
        .nodes
        .is_empty());
}
