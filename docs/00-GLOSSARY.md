# 00 — Binding Glossary

Every agent and every document must use these terms exactly. Inconsistent vocabulary is the
most common cause of architectural drift in agent-built systems. If you need a new term,
add it here first.

| Term | Meaning |
|---|---|
| **Cortex** | The daemon on the local machine. The core of the system. Binary name `cortexd`. |
| **Main Brain** | A remote, always-on semantic memory instance the Cortex may federate with. **Optional.** The Core works fully without one. |
| **Node** | A memory entry. May be a file, a directory, a chunk, a concept or an annotation. |
| **Episode** | A timestamped event (file opened, created, searched, moved). Immutable, append-only. |
| **Working Set** | What the human is doing *right now*. Volatile, TTL-based, in memory. |
| **STM** | Short-Term Memory. Hot, fully indexed zone covering the last N days. |
| **LTM** | Long-Term Memory. Consolidated, condensed knowledge. Lives locally; may additionally live in a Main Brain. |
| **Cold** | Metadata-only tier. Path and stat, no embeddings. Read only on explicit deep search. |
| **Consolidation** | The batch process that condenses episodes into semantic knowledge and promotes it upward. |
| **Salience** | Importance score of a Node. Drives consolidation, ranking and forgetting. |
| **Human View** | The GUI view optimised for humans. |
| **Agent View** | The visualisation of the brain state. Readable by humans, but it *shows* what agents see. |
| **Agent Surface** | The actual machine interface (MCP + local JSON-RPC). Not to be confused with Agent View. |
| **Core** | The public, publishable part of the code. This repository. |
| **Extension** | Private, personal add-on. Separate repository. Never merged into Core. |
| **Ingest** | The pipeline from filesystem event to indexed chunk. |
| **Outbox** | Local append-only queue of sync events waiting for a reachable peer. |
| **Peer** | A federation counterpart. A Main Brain is a peer. The protocol does not know what kind. |

## Terms deliberately avoided

- "Database" for the index — it is a **cache**. Saying database invites treating it as authoritative.
- "AI search" — the system is a hybrid retrieval engine; models improve ranking, they do not
  produce the result set.
- "Sidecar file" — Hippocampus writes nothing next to the user's files. Ever.
