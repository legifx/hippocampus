//! What becomes an event, and what it looks like.
//!
//! Events are derived, never authored: every one comes from a row in `episodes` or from a
//! concept node, and can be derived again from the same row with the same identifier. That is
//! why "replay the whole outbox" is a cheap operation rather than a dangerous one — the outbox
//! is a cache of this function's output (R10).

use crate::protocol::{event_id, SyncEvent};
use cortex_core::config::FederationConfig;
use cortex_core::model::{Action, Episode};
use cortex_store::outbox::ConceptForSync;

/// Turn an episode into an event, or decide that it stays home.
///
/// `path` is the path of the node the episode is about, looked up by the caller — an episode
/// keeps only the id, and the id means nothing on another machine.
pub fn from_episode(
    episode: &Episode,
    path: Option<&str>,
    origin: &str,
    config: &FederationConfig,
) -> Option<SyncEvent> {
    let about_a_file = matches!(
        episode.action,
        Action::Create | Action::Modify | Action::Open | Action::Move | Action::Delete
    );
    match episode.action {
        // What somebody typed into a search box is theirs. It never leaves (R5's spirit, and
        // the reason the deny-list exists).
        Action::Search => return None,
        _ if about_a_file && !config.send_file_activity => return None,
        _ => {}
    }

    let mut payload = serde_json::json!({
        "action": episode.action.as_str(),
        "actor": episode.actor.as_str(),
    });
    if let Some(session) = &episode.session_id {
        payload["session_id"] = serde_json::Value::String(session.0.clone());
    }
    if let Some(path) = path {
        payload["path"] = serde_json::Value::String(path.to_string());
    }
    // The metadata is the episode's own record of itself — for a memory it is the memory
    // (scope, kind, text, confidence); for a move it is where from and where to. It is passed
    // through whole, minus anything that would be content rather than metadata.
    if let serde_json::Value::Object(meta) = &episode.metadata {
        let mut kept = serde_json::Map::new();
        for (k, v) in meta {
            if !config.send_chunk_text && k == "chunk_text" {
                continue;
            }
            kept.insert(k.clone(), v.clone());
        }
        if !kept.is_empty() {
            payload["metadata"] = serde_json::Value::Object(kept);
        }
    }

    Some(SyncEvent {
        event_id: event_id(origin, &format!("episode:{}", episode.id.0)),
        lamport: 0,
        origin: origin.to_string(),
        ts: episode.ts,
        kind: "episode".to_string(),
        payload,
    })
}

/// A concept, as a name and the places it names. Its identity is the label plus the member
/// set, so it is sent again exactly when it changed.
pub fn from_concept(concept: &ConceptForSync, origin: &str) -> SyncEvent {
    let mut identity = String::from("concept:");
    identity.push_str(&concept.label);
    for member in &concept.members {
        identity.push('\0');
        identity.push_str(member);
    }
    SyncEvent {
        event_id: event_id(origin, &identity),
        lamport: 0,
        origin: origin.to_string(),
        ts: concept.created_at,
        kind: "concept".to_string(),
        payload: serde_json::json!({
            "label": concept.label,
            "members": concept.members,
            "member_count": concept.members.len(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::ids::EpisodeId;
    use cortex_core::model::Actor;

    fn episode(action: Action, metadata: serde_json::Value) -> Episode {
        Episode {
            id: EpisodeId(7),
            ts: 100,
            actor: Actor::Agent("claude".into()),
            action,
            node_id: None,
            session_id: None,
            metadata,
        }
    }

    #[test]
    fn a_search_never_leaves() {
        let e = episode(Action::Search, serde_json::json!({ "query": "tax return" }));
        assert!(from_episode(&e, None, "o", &FederationConfig::default()).is_none());
    }

    #[test]
    fn file_activity_is_metadata_and_can_be_switched_off() {
        let e = episode(Action::Open, serde_json::json!({}));
        let on = FederationConfig::default();
        let ev = from_episode(&e, Some("/tmp/x"), "o", &on).unwrap();
        assert_eq!(ev.payload["path"], "/tmp/x");
        assert_eq!(ev.payload["action"], "open");
        let off = FederationConfig {
            send_file_activity: false,
            ..FederationConfig::default()
        };
        assert!(from_episode(&e, Some("/tmp/x"), "o", &off).is_none());
    }

    #[test]
    fn a_memory_travels_whole_and_chunk_text_only_when_allowed() {
        let e = episode(
            Action::Remember,
            serde_json::json!({ "scope": "global", "kind": "fact", "text": "x is y",
                                "chunk_text": "the whole file" }),
        );
        let ev = from_episode(&e, None, "o", &FederationConfig::default()).unwrap();
        assert_eq!(ev.payload["metadata"]["text"], "x is y");
        assert!(ev.payload["metadata"].get("chunk_text").is_none());
        let permissive = FederationConfig {
            send_chunk_text: true,
            ..FederationConfig::default()
        };
        let ev = from_episode(&e, None, "o", &permissive).unwrap();
        assert_eq!(ev.payload["metadata"]["chunk_text"], "the whole file");
    }

    #[test]
    fn the_same_episode_always_has_the_same_identifier() {
        let e = episode(Action::Annotate, serde_json::json!({ "text": "a" }));
        let a = from_episode(&e, None, "o", &FederationConfig::default()).unwrap();
        let mut later = e.clone();
        later.metadata = serde_json::json!({ "text": "a", "extra": true });
        let b = from_episode(&later, None, "o", &FederationConfig::default()).unwrap();
        assert_eq!(a.event_id, b.event_id, "an identifier must not depend on the payload");
    }

    #[test]
    fn a_concept_changes_identity_exactly_when_its_membership_does() {
        let c = ConceptForSync {
            id: 1,
            label: "invoices".into(),
            created_at: 0,
            members: vec!["/a".into(), "/b".into()],
        };
        let same = from_concept(&c, "o");
        let mut grown = c.clone();
        grown.members.push("/c".into());
        assert_eq!(same.event_id, from_concept(&c, "o").event_id);
        assert_ne!(same.event_id, from_concept(&grown, "o").event_id);
        assert_eq!(same.payload["member_count"], 2);
    }
}
