//! Asking the peer (D126): a question from `brain.ask` handed to a declared `recall` plugin.
//!
//! Federation (D9, D17) sends what happened here *to* a peer. This is the way back: what the
//! peer knows, read at the moment an agent asks. The plugin gets one request and gives one
//! answer, exactly like a `sync` plugin (`plugin::exchange`), and the Core learns nothing about
//! where the peer is or how it is reached:
//!
//! ```json
//! {"jsonrpc": "2.0", "id": 1, "method": "recall.query", "params": {"question": "…", "limit": 3}}
//! {"jsonrpc": "2.0", "id": 1, "result": {"items": [{"title": "…", "location": "…", "snippet": "…"}]}}
//! ```
//!
//! What comes back is shown, never stored: the index stays a cache of this machine's files
//! (R2), and a peer's answer is not one of them. Nothing here is called unless
//! `federation.recall_plugin` names a declaration.

use crate::plugin::exchange;
use crate::transport::TransportError;
use cortex_core::plugins::{PluginDecl, PluginKind};

pub const METHOD_RECALL: &str = "recall.query";

const MAX_TITLE: usize = 160;
const MAX_LOCATION: usize = 300;
const MAX_SNIPPET: usize = 600;

/// One thing the peer knows. `location` is the peer's own name for where it is written down —
/// a path in its store, an id — and means nothing on this machine.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RecallItem {
    pub title: String,
    pub location: String,
    #[serde(default)]
    pub snippet: Option<String>,
}

#[derive(serde::Deserialize)]
struct RecallAnswer {
    #[serde(default)]
    items: Vec<RecallItem>,
}

pub struct Recall {
    name: String,
    decl: PluginDecl,
}

impl Recall {
    /// Refuses a declaration of another kind: a `sync` plugin handed a question would read it
    /// as a batch.
    pub fn new(name: impl Into<String>, decl: PluginDecl) -> Result<Self, String> {
        let name = name.into();
        if decl.kind != PluginKind::Recall {
            return Err(format!("plugin {name:?} is declared as {:?}, not recall", decl.kind));
        }
        Ok(Self { name, decl })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Blocking, bounded by the declaration's `timeout_ms`. At most `limit` items, each cut to
    /// a size an agent's context can afford, empty ones dropped.
    pub fn query(&self, question: &str, limit: usize) -> Result<Vec<RecallItem>, TransportError> {
        let result = exchange(
            &self.decl,
            METHOD_RECALL,
            serde_json::json!({ "question": question, "limit": limit }),
        )?;
        let answer: RecallAnswer = serde_json::from_value(result)
            .map_err(|e| TransportError::Malformed(format!("result is not a recall answer: {e}")))?;
        Ok(answer
            .items
            .into_iter()
            .filter(|i| !i.title.trim().is_empty() || !i.location.trim().is_empty())
            .take(limit)
            .map(|i| RecallItem {
                title: cut(i.title.trim(), MAX_TITLE),
                location: cut(i.location.trim(), MAX_LOCATION),
                snippet: i
                    .snippet
                    .map(|s| cut(&s.split_whitespace().collect::<Vec<_>>().join(" "), MAX_SNIPPET))
                    .filter(|s| !s.is_empty()),
            })
            .collect())
    }
}

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max - 1).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn decl(kind: &str, script: &str, timeout_ms: u64) -> PluginDecl {
        PluginDecl::from_toml(
            &format!(
                "kind = {kind:?}\ncommand = \"sh\"\nargs = [\"-c\", {script:?}]\ntimeout_ms = {timeout_ms}\n"
            ),
            &PathBuf::from("<test>"),
        )
        .unwrap()
    }

    #[test]
    fn a_sync_declaration_is_never_asked_a_question() {
        assert!(Recall::new("bridge", decl("sync", "true", 1000)).is_err());
    }

    #[test]
    fn the_answer_is_cut_to_the_limit_and_to_size() {
        let long = "x".repeat(2000);
        let script = format!(
            "read line; echo '{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"items\":[\
             {{\"title\":\"a\",\"location\":\"notes/a.md\",\"snippet\":\"{long}\"}},\
             {{\"title\":\"\",\"location\":\"\"}},\
             {{\"title\":\"b\",\"location\":\"notes/b.md\"}},\
             {{\"title\":\"c\",\"location\":\"notes/c.md\"}}]}}}}'"
        );
        let recall = Recall::new("peer", decl("recall", &script, 5000)).unwrap();
        let items = recall.query("where is it", 2).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].location, "notes/a.md");
        assert_eq!(items[0].snippet.as_ref().unwrap().chars().count(), MAX_SNIPPET);
        assert_eq!(items[1].title, "b", "the empty item is dropped, not counted");
    }

    #[test]
    fn the_question_reaches_the_plugin() {
        // The plugin echoes the question it read back as the title of its one item.
        let script = "read line; q=$(printf '%s' \"$line\" | sed 's/.*\"question\":\"\\([^\"]*\\)\".*/\\1/'); \
                      printf '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"items\":[{\"title\":\"%s\",\"location\":\"x\"}]}}\\n' \"$q\"";
        let recall = Recall::new("peer", decl("recall", script, 5000)).unwrap();
        let items = recall.query("takt microphone", 3).unwrap();
        assert_eq!(items[0].title, "takt microphone");
    }

    #[test]
    fn a_slow_peer_is_given_up_on_at_the_deadline() {
        let recall = Recall::new("peer", decl("recall", "sleep 5", 200)).unwrap();
        let started = std::time::Instant::now();
        assert!(matches!(recall.query("q", 3), Err(TransportError::Unreachable(_))));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }
}
