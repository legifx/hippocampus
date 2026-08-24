//! The audit log (hard rule R8): timestamp, agent identity, tool, arguments, result size.
//!
//! Every call through the Agent Surface produces exactly one entry, and the refusals are the
//! ones worth having. A log that only records what succeeded answers "what did it do" but not
//! "what did it try", and the second question is the one asked after something goes wrong.
//!
//! Arguments are stored after [`redact`]. The log must stay readable and bounded: a 4 MB
//! `fs.write` payload in an audit row helps nobody and quietly turns the index into a copy of
//! the filesystem, which rule R10 forbids on principle and disk space forbids in practice.

use crate::policy::Refusal;
use cortex_store::Store;
use std::sync::Arc;

/// How much of a string argument is kept verbatim.
const MAX_STRING: usize = 256;
/// How many elements of an array argument are kept.
const MAX_ARRAY: usize = 16;

/// Records one actor's calls. Cheap to construct per connection.
pub struct Auditor {
    store: Arc<Store>,
    actor: String,
}

impl Auditor {
    pub fn new(store: Arc<Store>, actor: &cortex_core::model::Actor) -> Self {
        Self {
            store,
            actor: actor.as_str(),
        }
    }

    pub fn actor(&self) -> &str {
        &self.actor
    }

    /// Record an allowed call. `summary` describes the result's size or shape — never its
    /// content.
    pub fn allowed(&self, ts: i64, tool: &str, args: &serde_json::Value, summary: &str) {
        self.write(ts, tool, args, Some(summary), true);
    }

    /// Record a refusal. Returns the refusal so a call site can log and propagate in one
    /// expression, which is what keeps the two from drifting apart.
    pub fn refuse(&self, ts: i64, tool: &str, args: &serde_json::Value, refusal: Refusal) -> Refusal {
        self.write(ts, tool, args, Some(&refusal.summary()), false);
        refusal
    }

    fn write(
        &self,
        ts: i64,
        tool: &str,
        args: &serde_json::Value,
        summary: Option<&str>,
        allowed: bool,
    ) {
        let redacted = redact(args);
        if let Err(e) = self
            .store
            .append_audit(ts, &self.actor, tool, &redacted, summary, allowed)
        {
            // Deliberately not fatal to the call that is already refused, but loud: an audit
            // log that stopped being written is a silent failure of R8, and the daemon's log
            // is the only place left to say so.
            tracing::error!(actor = %self.actor, tool, error = %e, "could not write audit entry");
        }
    }
}

/// Shrink arguments to something a log can hold: long strings truncated with their true length
/// recorded, long arrays cut, structure otherwise preserved so the entry still reads like the
/// call that produced it.
pub fn redact(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) if s.len() > MAX_STRING => {
            let head: String = s.chars().take(MAX_STRING).collect();
            serde_json::Value::String(format!("{head}… <{} bytes total>", s.len()))
        }
        serde_json::Value::Array(items) => {
            let mut out: Vec<_> = items.iter().take(MAX_ARRAY).map(redact).collect();
            if items.len() > MAX_ARRAY {
                out.push(serde_json::Value::String(format!(
                    "… <{} more>",
                    items.len() - MAX_ARRAY
                )));
            }
            serde_json::Value::Array(out)
        }
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), redact(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// A one-line description of a successful result, for the `result_summary` column.
pub fn summarize(kind: &str, count: usize, bytes: Option<usize>) -> String {
    match bytes {
        Some(b) => format!("{count} {kind}, {b} bytes"),
        None => format!("{count} {kind}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::model::Actor;

    #[test]
    fn long_strings_are_truncated_but_their_length_is_kept() {
        let long = "x".repeat(5000);
        let out = redact(&serde_json::json!({ "content": long }));
        let s = out["content"].as_str().unwrap();
        assert!(s.len() < 400);
        assert!(s.contains("5000 bytes total"));
    }

    #[test]
    fn short_arguments_survive_untouched() {
        let args = serde_json::json!({"path": "/tmp/a.txt", "depth": 2, "glob": null});
        assert_eq!(redact(&args), args);
    }

    #[test]
    fn long_arrays_are_cut_and_say_so() {
        let args = serde_json::json!({"paths": (0..100).map(|i| i.to_string()).collect::<Vec<_>>()});
        let out = redact(&args);
        let arr = out["paths"].as_array().unwrap();
        assert_eq!(arr.len(), MAX_ARRAY + 1);
        assert!(arr.last().unwrap().as_str().unwrap().contains("84 more"));
    }

    #[test]
    fn a_refusal_is_logged_and_returned() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let auditor = Auditor::new(store.clone(), &Actor::Agent("test".into()));
        let args = serde_json::json!({"path": "/etc/shadow"});

        let refusal = auditor.refuse(
            7,
            "fs.read",
            &args,
            Refusal::Denied {
                path: "/etc/shadow".into(),
            },
        );
        assert_eq!(refusal.tag(), "denied");

        let entries = store.recent_audit(10, None, true, None).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].actor, "agent:test");
        assert_eq!(entries[0].tool, "fs.read");
        assert!(!entries[0].allowed);
        assert!(entries[0].result_summary.as_ref().unwrap().contains("denied"));
    }

    #[test]
    fn an_allowed_call_records_a_size_and_not_a_payload() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let auditor = Auditor::new(store.clone(), &Actor::Agent("test".into()));
        auditor.allowed(
            1,
            "fs.read",
            &serde_json::json!({"path": "/tmp/a"}),
            &summarize("bytes read", 0, Some(1200)),
        );

        let entries = store.recent_audit(10, None, false, None).unwrap();
        assert!(entries[0].allowed);
        assert!(entries[0].result_summary.as_ref().unwrap().contains("1200"));
    }
}
