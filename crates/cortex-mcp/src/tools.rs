//! The tool catalogue: one table, mapping an MCP tool to the RPC method behind it.
//!
//! Every entry is a name, a description an agent reads to decide whether it wants the tool, a
//! JSON schema for its arguments, and the daemon method it forwards to. There is no per-tool
//! code — a tool that needed some is a tool whose logic has escaped the daemon, and then the
//! three surfaces would no longer share one implementation (M3 §5).
//!
//! MCP names use underscores where the wire protocol uses dots. Clients differ on whether a
//! dot in a tool name is acceptable, and the mapping is one line here against an argument with
//! every client forever.

use cortex_rpc::protocol::method;
use serde_json::{json, Value};

pub struct Tool {
    pub name: &'static str,
    pub method: &'static str,
    pub description: &'static str,
    /// True for anything that can change a file. Used only to describe the tool honestly — the
    /// enforcement is the daemon's, and a client that ignored this would still be refused.
    pub gated: bool,
    pub schema: fn() -> Value,
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

/// The `target` argument every brain tool takes: a path or a node id.
fn target_property() -> Value {
    json!({
        "description": "An absolute path, or a numeric node id from a previous answer.",
        "anyOf": [{ "type": "string" }, { "type": "integer" }]
    })
}

pub const TOOLS: &[Tool] = &[
    Tool {
        name: "brain_search",
        method: method::SEARCH,
        gated: false,
        description: "Search the index by meaning, by words, or by path. This is the tool to \
                      reach for when you know roughly what a file is about but not what it is \
                      called. Returns paths with snippets.",
        schema: || {
            object(
                json!({
                    "query": { "type": "string", "description": "What you are looking for." },
                    "limit": { "type": "integer", "description": "How many hits. Default 20." },
                    "mode": {
                        "type": "string",
                        "enum": ["auto", "locate", "recall", "reference"],
                        "description": "Leave unset unless you have a reason. `auto` picks."
                    }
                }),
                &["query"],
            )
        },
    },
    Tool {
        name: "brain_recall",
        method: method::BRAIN_RECALL,
        gated: false,
        description: "Pattern completion: give a fragment, get back what belongs with it. \
                      Search finds the way in; the associative graph supplies the notes, links \
                      and related files around each hit, including ones that share no words \
                      with the cue.",
        schema: || {
            object(
                json!({
                    "cue": { "type": "string", "description": "A phrase, a filename, a half-memory." },
                    "depth": { "type": "integer", "description": "Hops out from each hit. Default 1, max 3." },
                    "limit": { "type": "integer", "description": "How many hits to expand around." }
                }),
                &["cue"],
            )
        },
    },
    Tool {
        name: "brain_explain",
        method: method::SEARCH_EXPLAIN,
        gated: false,
        description: "Why did this file rank where it did for this query — or why did it not \
                      appear at all? Answers with the score breakdown and, when the answer is \
                      'it was never indexed' or 'extraction skipped it', says so.",
        schema: || {
            object(
                json!({
                    "query": { "type": "string" },
                    "path": { "type": "string", "description": "The file to explain." },
                    "id": { "type": "integer", "description": "Or its node id." }
                }),
                &["query"],
            )
        },
    },
    Tool {
        name: "brain_neighbors",
        method: method::BRAIN_NEIGHBORS,
        gated: false,
        description: "What is this node connected to? Returns annotations on it and edges out \
                      of it, in both directions, with the note text spelled out rather than \
                      left as ids to chase.",
        schema: || {
            object(
                json!({
                    "target": target_property(),
                    "depth": { "type": "integer", "description": "Default 1." },
                    "edge_types": {
                        "type": "array",
                        "items": { "type": "string",
                                   "enum": ["contains", "co_access", "imports", "similar_to",
                                            "derived_from", "annotates"] },
                        "description": "Empty means all of them."
                    },
                    "limit": { "type": "integer" }
                }),
                &["target"],
            )
        },
    },
    Tool {
        name: "brain_timeline",
        method: method::BRAIN_TIMELINE,
        gated: false,
        description: "Episodic memory: what happened, when, and who caused it. Filter by time \
                      range, by actor (`human`, `system`, `agent:<name>`) or by action.",
        schema: || {
            object(
                json!({
                    "from": { "type": "integer", "description": "Unix seconds, inclusive." },
                    "to": { "type": "integer" },
                    "actor": { "type": "string" },
                    "action": { "type": "string",
                                "enum": ["create", "modify", "open", "move", "delete",
                                         "search", "annotate"] },
                    "limit": { "type": "integer" }
                }),
                &[],
            )
        },
    },
    Tool {
        name: "brain_working_set",
        method: method::BRAIN_WORKING_SET,
        gated: false,
        description: "What the human has been touching lately, and how long they have been \
                      idle. Use it to work out what 'this file' or 'what I was just doing' \
                      refers to.",
        schema: || object(json!({}), &[]),
    },
    Tool {
        name: "brain_report",
        method: method::BRAIN_REPORT,
        gated: false,
        description: "What the last consolidation pass did: how many episodes it replayed, what \
                      it associated, what changed tier, what it deliberately did not do. Use it \
                      before concluding that the memory has forgotten something — it may simply \
                      never have been consolidated.",
        schema: || object(json!({}), &[]),
    },
    Tool {
        name: "brain_graph",
        method: method::BRAIN_GRAPH,
        gated: false,
        description: "The whole brain in one answer: the most salient nodes, their edges, the \
                      zones, the Working Set and the recent episodes. A large result — put it \
                      through a result gate rather than into a prompt. For 'what does this \
                      index look like', not for finding a file; use brain_search for that.",
        schema: || {
            object(
                json!({
                    "limit": { "type": "integer",
                               "description": "Nodes, by salience. Default 3000, capped at \
                                               15000 so the answer fits the transport. \
                                               Ancestors are added on top so nothing floats." },
                    "min_salience": { "type": "number" },
                    "zone": { "type": "integer", "description": "Restrict to one zone." },
                    "events": { "type": "integer", "description": "Recent episodes. Default 500." },
                    "window_secs": { "type": "integer",
                                     "description": "How far back 'recently touched' reaches." }
                }),
                &[],
            )
        },
    },
    Tool {
        name: "brain_diff",
        method: method::BRAIN_DIFF,
        gated: false,
        description: "What changed since an episode id. Keep the returned `cursor` and pass it \
                      back as `since` — this is the tool that means you never have to poll.",
        schema: || {
            object(
                json!({
                    "since": { "type": "integer", "description": "The cursor from your last call." },
                    "limit": { "type": "integer" }
                }),
                &[],
            )
        },
    },
    Tool {
        name: "brain_annotate",
        method: method::BRAIN_ANNOTATE,
        gated: false,
        description: "Attach a note to a file without touching the file. This is how you record \
                      what you worked out about something so that the next search can find it.",
        schema: || {
            object(
                json!({
                    "target": target_property(),
                    "note": { "type": "string", "description": "What you want remembered." },
                    "tags": { "type": "array", "items": { "type": "string" } }
                }),
                &["target", "note"],
            )
        },
    },
    Tool {
        name: "brain_link",
        method: method::BRAIN_LINK,
        gated: false,
        description: "Assert that two nodes belong together. Explicit links are never pruned by \
                      anything that only trusts its own inferences.",
        schema: || {
            object(
                json!({
                    "a": target_property(),
                    "b": target_property(),
                    "edge": { "type": "string",
                              "enum": ["contains", "co_access", "imports", "similar_to",
                                       "derived_from", "annotates"],
                              "description": "Default `similar_to`." },
                    "remove": { "type": "boolean", "description": "Remove the edge instead." }
                }),
                &["a", "b"],
            )
        },
    },
    Tool {
        name: "brain_pin",
        method: method::BRAIN_PIN,
        gated: false,
        description: "Mark a node as never to be forgotten. Pinned nodes are immune to decay \
                      and tier demotion.",
        schema: || object(json!({ "target": target_property() }), &["target"]),
    },
    Tool {
        name: "brain_forget",
        method: method::BRAIN_FORGET,
        gated: false,
        description: "Unpin a node and reset its salience to zero. Touches memory only — the \
                      file itself is not affected.",
        schema: || object(json!({ "target": target_property() }), &["target"]),
    },
    Tool {
        name: "fs_places",
        method: method::FS_PLACES,
        gated: false,
        description: "Where the person's own files are: home, Downloads, Documents and the \
                      rest, the mounted drives, and the filesystem root. Use this instead of \
                      guessing at `~/Documents` — the folders are named by the desktop's own \
                      configuration and may not be in English.",
        schema: || object(json!({}), &[]),
    },
    Tool {
        name: "fs_list",
        method: method::FS_LIST,
        gated: false,
        description: "Read a directory. Entries covered by the deny-list are counted but not \
                      named. Each entry says whether the brain knows it, so you can tell 'not \
                      there' from 'not indexed yet'.",
        schema: || {
            object(
                json!({
                    "path": { "type": "string", "description": "An absolute path." },
                    "depth": { "type": "integer", "description": "1 is the directory itself." },
                    "limit": { "type": "integer" }
                }),
                &["path"],
            )
        },
    },
    Tool {
        name: "fs_read",
        method: method::FS_READ,
        gated: false,
        description: "Read a file's contents, up to 1 MiB. Pass `byte_range` for a slice of a \
                      larger one.",
        schema: || {
            object(
                json!({
                    "path": { "type": "string" },
                    "byte_range": {
                        "type": "array",
                        "items": { "type": "integer" },
                        "minItems": 2, "maxItems": 2,
                        "description": "[start, end), in bytes."
                    }
                }),
                &["path"],
            )
        },
    },
    Tool {
        name: "fs_stat",
        method: method::FS_STAT,
        gated: false,
        description: "What is at this path: whether it exists, its size, whether it is indexed, \
                      and — usefully before you try — whether you are allowed to write it.",
        schema: || object(json!({ "path": { "type": "string" } }), &["path"]),
    },
    Tool {
        name: "fs_write",
        method: method::FS_WRITE,
        gated: true,
        description: "Write a file. GATED: called without `plan`, it changes nothing and \
                      returns a plan describing what it would do. Call again with that plan's \
                      id to apply it. The plan expires and is checked against the filesystem \
                      when applied, so it cannot be used after the target has changed.",
        schema: || {
            object(
                json!({
                    "path": { "type": "string" },
                    "content": { "type": "string" },
                    "mode": { "type": "string", "enum": ["create", "overwrite", "append"],
                              "description": "Default `create`, which refuses if the path exists." },
                    "plan": { "type": "string", "description": "The plan id from your dry run." }
                }),
                &["path", "content"],
            )
        },
    },
    Tool {
        name: "fs_move",
        method: method::FS_MOVE,
        gated: true,
        description: "Move a file. GATED: plan first, then apply with the plan id. Both ends \
                      must be inside the write allow-list.",
        schema: || {
            object(
                json!({
                    "from": { "type": "string" },
                    "to": { "type": "string" },
                    "plan": { "type": "string" }
                }),
                &["from", "to"],
            )
        },
    },
    Tool {
        name: "fs_trash",
        method: method::FS_TRASH,
        gated: true,
        description: "Move a file to the trash, where `undo` can still reach it. GATED: plan \
                      first, then apply. Prefer this to overwriting anything you cannot replace.",
        schema: || {
            object(
                json!({
                    "path": { "type": "string" },
                    "plan": { "type": "string" }
                }),
                &["path"],
            )
        },
    },
    Tool {
        name: "undo",
        method: method::UNDO,
        gated: false,
        description: "Reverse recent mutations, newest first. Every gated tool writes its own \
                      inverse before it acts, so this works for writes, moves and trashings \
                      alike. Use `dry_run` to see what would be reversed.",
        schema: || {
            object(
                json!({
                    "last": { "type": "integer", "description": "How many. Default 1." },
                    "plan": { "type": "string", "description": "Or: everything one plan did." },
                    "dry_run": { "type": "boolean" }
                }),
                &[],
            )
        },
    },
];

pub fn find(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.name == name)
}

/// The catalogue as MCP wants it.
pub fn catalogue() -> Value {
    Value::Array(
        TOOLS
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "inputSchema": (t.schema)(),
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_rpc::protocol::method;

    #[test]
    fn every_tool_forwards_to_a_method_the_daemon_serves() {
        for tool in TOOLS {
            assert!(
                method::ALL.contains(&tool.method),
                "{} forwards to `{}`, which is not in the protocol's method list",
                tool.name,
                tool.method
            );
        }
    }

    #[test]
    fn names_are_unique_and_use_no_dots() {
        let mut names: Vec<_> = TOOLS.iter().map(|t| t.name).collect();
        let before = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(before, names.len(), "duplicate tool name");
        assert!(TOOLS.iter().all(|t| !t.name.contains('.')));
    }

    #[test]
    fn every_tool_describes_itself_and_its_arguments() {
        for tool in TOOLS {
            assert!(
                tool.description.len() > 40,
                "{} has a description too short to choose it by",
                tool.name
            );
            let schema = (tool.schema)();
            assert_eq!(schema["type"], "object", "{}", tool.name);
            assert!(schema.get("properties").is_some(), "{}", tool.name);
        }
    }

    #[test]
    fn every_gated_tool_says_it_is_gated_and_how() {
        for tool in TOOLS.iter().filter(|t| t.gated) {
            assert!(
                tool.description.contains("GATED"),
                "{} is gated but does not say so, so an agent will call it wrongly first",
                tool.name
            );
            let schema = (tool.schema)();
            assert!(
                schema["properties"].get("plan").is_some(),
                "{} is gated but takes no plan id",
                tool.name
            );
        }
    }
}
