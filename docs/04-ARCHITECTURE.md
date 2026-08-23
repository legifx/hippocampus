# 04 — Architecture

## The one-sentence version

A headless daemon owns a rebuildable memory index over the filesystem; every user interface —
GUI, CLI, TUI, MCP — is an equal client speaking JSON-RPC to it over a Unix socket.

## Component map

```
┌──────────────────────── LOCAL MACHINE (not always on) ────────────────────────┐
│                                                                               │
│  CLIENTS                                                                      │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐  ┌────────────────────┐       │
│  │ Human View │  │ Agent View │  │  CLI/TUI   │  │ MCP Server         │       │
│  │   (GUI)    │  │   (Vis)    │  │            │  │ (Agent Surface)    │       │
│  └─────┬──────┘  └─────┬──────┘  └─────┬──────┘  └─────────┬──────────┘       │
│        └───────────────┴───────────────┴───────────────────┘                  │
│                                │ JSON-RPC 2.0 over Unix socket (0600)         │
│  ┌─────────────────────────────▼─────────────────────────────────────────┐    │
│  │                        CORTEX (daemon)                                │    │
│  │                                                                       │    │
│  │  Query Engine ── RRF Fusion ── Rerank ── (optional) Query Understanding│   │
│  │       │                                                               │    │
│  │  ┌────▼─────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌────────────┐   │    │
│  │  │ Working  │ │   STM    │ │   LTM    │ │ Salience │ │Consolidation│  │    │
│  │  │   Set    │ │  (hot)   │ │ (local)  │ │  Engine  │ │   Engine    │  │    │
│  │  └──────────┘ └──────────┘ └──────────┘ └──────────┘ └─────┬──────┘   │    │
│  │                                                            │          │    │
│  │  Ingest: Watcher → Classifier → Extractor → Chunker → Embedder        │    │
│  │  Stores: SQLite (meta/episodes/graph) │ FTS5 │ usearch HNSW           │    │
│  └───────────────────────────────┬───────────────────────────────────────┘    │
│                                  │ plugin protocol = same JSON-RPC            │
│  ┌───────────────────────────────▼───────────────────────────────────────┐    │
│  │  PLUGINS (separate processes)                                         │    │
│  │  · thumbnailer (Core)   · extension bridge (private)   · custom       │    │
│  └───────────────────────────────┬───────────────────────────────────────┘    │
└──────────────────────────────────┼────────────────────────────────────────────┘
                                   │  optional, additive, offline-tolerant
┌──────────────────────────────────▼────────────────────────────────────────────┐
│  PEER — a Main Brain (optional; the Core is fully functional without one)      │
└───────────────────────────────────────────────────────────────────────────────┘
```

### Where that lives in the source

The map above is the architecture; this is the current translation of it into crates, which
moves as the system grows.

| Box | Crate |
|---|---|
| Watcher, coalescing, reconciliation, resource throttling | `cortex-ingest` |
| Classifier, extractor, chunker, snippets | `cortex-extract` |
| Embedding models: registry, provisioning, inference | `cortex-embed` |
| Query engine, RRF fusion, query modes, the search orchestrator | `cortex-query` |
| SQLite, FTS5, chunk and episode storage, the usearch vector index | `cortex-store` |
| Ingest loop, extraction stage, embedding stage, activity signal, RPC server | `cortexd` |

The search orchestrator lives in `cortex-query` rather than in the daemon on purpose: a binary
cannot be depended on, and a benchmark that reimplements the search it measures eventually
measures something else. The daemon owns scheduling and policy; the library owns retrieval.

## Why the daemon owns everything

- It is testable without a display: integration tests, benchmarks, fuzzing.
- Agent access is a first-class path, not a retrofit.
- If the GUI technology turns out to be the wrong choice, 20 % is discarded instead of 100 %.
- Federation talks to the daemon, not to a running GUI — so it works with no window open.

## Process boundaries and why they are where they are

| Process | Contains | Reason for the boundary |
|---|---|---|
| `cortexd` | index, query, memory, RPC | single owner of state; no locking across processes |
| `hc-thumbnailer` | image/PDF/video decoding | untrusted content and third-party decoders — a crash must not take the memory with it, and it must be killable under load |
| `cortex-mcp` | MCP protocol adaptation | agent-facing surface can be disabled independently (kill switch) |
| plugins | extension code | language independence, crash isolation, and a runtime-enforced Core/Extension split |
| GUI | rendering only | a frozen or crashed UI must not stall indexing |

The thumbnailer boundary is the one most likely to be argued away as over-engineering. It is not:
preview generation is the single place where the system parses arbitrary, potentially hostile
binary data with large third-party libraries.

## Data flow: a keystroke

```
keystroke
   │
   ├─► [0–20 ms]   trigram path match ∥ BM25 ∥ vector ANN     measured: 1 / 15 / 1 ms
   │               → reciprocal rank fusion → salience boost   measured: 12 µs
   │               → FIRST RESULTS VISIBLE                     measured: 20 ms with snippets
   │
   ├─► [~17 ms]    cross-encoder rerank of top 30, fused with the order above (D29)
   │               → delivered as a `search.update` notification, never awaited
   │
   └─► [~400 ms]   (M5+, optional) small local model parses the query into
                   structured filters → list refines
```

The reranker's arrow starts where the first one ends: the results are already on screen when it
begins. That is the whole design (D7) — the connection is split so a background task can send a
better answer down it while the reader loop carries on.

The result is never empty and never blocked. Models *improve* the result progressively rather
than producing it. This is the difference between a tool and a demo.

## Data flow: a file changes

```
inotify event                                    (D20: inotify, not fanotify)
   │  (no content read)
   ├─► debounce/coalesce ~500 ms per (path, class)
   ├─► deny-list check ────────► rejected → metadata only, no extractor ever invoked
   ├─► episode written immediately (append-only)
   ├─► node upsert (move detected by inode, not delete+create)
   │      └─► stat changed? → extract_state = pending
   └─► priority queue + wake signal ──► extraction stage (own thread, nice, duty cycle)
          │  a just-touched file is taken before the backlog; the stage sleeps
          │  until signalled rather than polling (D22)
          └─► classify → extract → hash the text
                 ├─► text hash unchanged? → nothing to do at all
                 └─► changed? → chunk → hash per chunk
                        ├─► chunk hash unchanged? → keep the row, keep the embedding
                        └─► new? → FTS5

embedding stage (separate thread, D28)
   │  claims chunks whose *hash* has no vector for the current model
   ├─► re-reads the text through the stored offsets, refusing a stale hash
   ├─► embeds in batches → `embeddings`, keyed by hash (D26)
   └─► adds to the usearch index, keyed by chunk id
          └─► index disagrees with the table? → refilled from it, no model needed
```

The two hashes do different jobs. The **text** hash decides whether the file is worth chunking;
the **chunk** hash decides which chunks survive an edit and therefore which embeddings are kept.
A save with no edit stops at the first; editing one paragraph of a long document stops nearly
everything at the second.

## Storage layout

```
$XDG_CONFIG_HOME/hippocampus/
    config.toml              user configuration
    plugins/*.toml           plugin declarations

$XDG_STATE_HOME/hippocampus/     mode 0700
    cortex.db                SQLite, WAL — authoritative index
    cortex.db-wal
    vectors.usearch          ANN index over file chunks, memory-mapped, rebuildable
    memories.usearch         ANN index over memory chunks — a second collection, same type,
                             so a memory recall does not compete with 100 000 file chunks (D75)
    models/                  downloaded ONNX models, checksum-verified
    audit.log                append-only agent audit trail

$XDG_RUNTIME_DIR/hippocampus/
    cortex.sock              JSON-RPC socket, mode 0600
    cortex.lock              single-instance lock
```

Thumbnails go to the **shared XDG thumbnail cache**, not to a private directory. Sharing it means
other applications' thumbnails are reused and ours are reusable.

## The agent brain (M11)

There is one memory, and an agent's share of it is a **Scope**, not a second store. The reason is
D74; the shape is this:

```
                    an agent, over MCP or the socket
                                  │
             agent.session.begin ─┤ every episode, audit row and memory
                                  │ from here on carries this session id
                                  ▼
      ┌───────────────────────────────────────────────────────────────┐
      │  brain.remember(text, kind, scope, subject?, supersedes?)      │
      │      writes an episode  ──►  projects a node + chunk + vector  │
      │      R10: the episode is the authority, the rest is a cache    │
      └───────────────────────────────────────────────────────────────┘
                                  │
      ┌───────────────────────────▼───────────────────────────────────┐
      │  brain.context(task, budget_tokens)                            │
      │      pinned preferences (unconditional)                        │
      │    + working set                                               │
      │    + memories: lexical ∪ memory-ANN ─► RRF ─► MMR ─► budget    │
      │    + the files those memories point at (path + byte range)     │
      │      every item with its basis; the whole thing gets a pack_id │
      └───────────────────────────┬───────────────────────────────────┘
                                  │
      ┌───────────────────────────▼───────────────────────────────────┐
      │  brain.outcome(pack_id, helped[], noise[])                     │
      │      moves `usefulness`, which enters salience — and which     │
      │      falls when a memory is retrieved and never reported (D76) │
      └───────────────────────────────────────────────────────────────┘
```

Two things about this diagram are the whole design:

**The middle box is the only one an agent needs on a good day.** Everything else in the surface
exists so that the middle box can be explained, corrected and audited.

**The loop closes at the bottom.** Without `brain.outcome` the ranking is evaluated by the thing
being evaluated, and a memory system with no feedback path converges on whatever it happened to
rank first. That is not a refinement; it is the difference between a store and a brain.

The **Agent Inspector** in the explorer is a fifth client of the same socket. It renders the
sessions, the memory list, the policy state, the undo journal and the M8 view — and, apart from
the kill switch, the undo button and the human's right to delete a memory (D78), it writes
nothing.

## The rebuildability invariant

Everything except `episodes` is derivable from `episodes` plus the filesystem.

This is what allows the memory to forget aggressively without risk, and it is what makes the
index a cache. Any proposed feature that stores unrecoverable state fails review. The test is
`cortex --rebuild-index` producing a byte-identical canonical dump (M2 DoD §7).

**Memories are the hard case and the reason the invariant is stated as "episodes plus the
filesystem" rather than "the filesystem".** What an agent learned is not on disk anywhere. It
survives a rebuild because the `remember` assertion is an episode and the memory row is a
projection of it — the same trick annotations use (D35), with more columns to lose. A deletion is
an assertion too, for the same reason: a `forget_memory` episode, or the memory comes back the
first time the index is rebuilt and the delete button was a lie (D78).

## Federation model

Two one-way streets, never a crossing:

```
Cortex ──── events (append-only, Lamport-stamped, idempotent) ────► Peer
Cortex ◄─── ontology / taxonomy updates ──────────────────────────  Peer
```

Authority is split so that most conflicts cannot exist:

- The peer is **never** authoritative over local file state.
- The Cortex is **never** authoritative over global knowledge.

Offline is the normal case. The outbox drains when a peer is reachable, under a rate limit so
that three days of accumulated events do not flood the peer on reconnect.

**The Core ships a protocol and a reference adapter against a local test peer. It contains no
knowledge of any real endpoint.** Concrete endpoints, schemas and credentials live only in the
private Extension repository, loaded as a subprocess plugin.

## Where the models run

| Work | Where | Why |
|---|---|---|
| Embedding | local GPU | latency, offline capability, and no network dependency in ingest |
| Reranking | local GPU/CPU | asynchronous, but still local for the same reason |
| Consolidation abstraction | local or peer | not latency-critical; may be routed |
| Transcription | plugin, may be remote | expensive, batchy, genuinely offloadable |

The general rule: **anything in the hot path runs locally.** A search that needs the network is
a search that fails on a train.
