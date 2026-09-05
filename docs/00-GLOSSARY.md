# 00 — Binding Glossary

Every agent and every document must use these terms exactly. Inconsistent vocabulary is the
most common cause of architectural drift in agent-built systems. If you need a new term,
add it here first.

| Term | Meaning |
|---|---|
| **Cortex** | The daemon on the local machine. The core of the system. Binary name `cortexd`. |
| **Main Brain** | A remote, always-on semantic memory instance the Cortex may federate with. **Optional.** The Core works fully without one. |
| **Node** | A memory entry. May be a file, a directory, a chunk, a concept, an annotation or a **Memory**. |
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
| **Memory** | A Node with `kind = 'memory'`: something an agent or the human knows that is **not in a file**. Has text, a Scope and a memory kind, no path. Indexed, scored, decayed and consolidated exactly like a file's chunks (D74). |
| **Scope** | Who a Memory belongs to: `session:<id>`, `agent:<name>`, `project:<zone_id>` or `global`. Writes default to the narrowest; reads never cross into another agent's private scope. |
| **Context Pack** | The answer `brain.context` gives: a token-budgeted, deduplicated, attributed block of memory, addressable by `pack_id` so an outcome can be reported against it (D77). |
| **Usefulness** | The learned component of a Memory's salience, moved by `brain.outcome`. Distinct from **confidence**, which is the writer's own claim and is never treated as evidence (D76). |
| **Agent Session** | One agent's run against the Surface. Every episode, audit row and Memory written during it carries its id, which is what makes "what did this task do" answerable. |
| **Agent Inspector** | The panel in the explorer that shows the Agent area: live sessions, the memory list, policy, the undo journal, and the Agent View embedded. Renders; does not decide (D4). |
| **Peer** | A federation counterpart. A Main Brain is a peer. The protocol does not know what kind. |

## Added by M12

- **Origin** — who caused a node's bytes to exist: `human`, `agent`, `tool`, `file`, `peer`.
  Recorded in the episode, projected to the node, never changed afterwards (D98).
- **Authority** — what a Memory may be used *as*: `instruction`, `fact`, `observation`,
  `untrusted`. Inherited downward from a memory's basis, raised only by a human or by two
  independent trusted origins. Not a score (D98).
- **State** — `pending`, `current`, `historical`, `retracted`, `deleted`. A pure function of the
  episode log, never a column of record (D99). *Pending* is the admission state for anything a
  file, a tool or a peer wrote.
- **Validity** — `valid_from` / `valid_to`: when a memory was true, as distinct from
  `created_at`, when it was recorded. Never silently filled (D99).
- **Supersede, with a reason** — `changed` (the world moved on; the old memory is *historical*)
  or `wrong` (the old memory was mistaken; it is *retracted*). One word carrying two questions
  was the mistake (D99).
- **Gold set** — frozen query → node pairs from the owner's own re-finding episodes, hashed;
  what `bench-quality` runs (D101).
- **Forgetting suite** — command-driven cases scored by the absence of a string across every
  surface, after a rebuild, after a re-remember (D102).

## Terms deliberately avoided

- "Database" for the index — it is a **cache**. Saying database invites treating it as authoritative.
- "AI search" — the system is a hybrid retrieval engine; models improve ranking, they do not
  produce the result set.
- "Sidecar file" — Hippocampus writes nothing next to the user's files. Ever.
- "Agent memory" as a *separate system* — there is one memory. An agent's Memory is a Node in it,
  distinguished by Scope, not by living somewhere else (D74). Saying "the agent memory" invites
  building the second store that decision exists to prevent.
- "Vector database" as a component to be chosen — the vector index is one arm of retrieval and a
  derived cache of the `embeddings` table, not a system of record (D75).
- "Trust score" — authority is an enum with a propagation rule. A continuous trust number is the
  malleable signal the poisoning literature proves unsound (D98).
