# 01 — Build Prompt

**This document is the plan of record.** It is written so that an agent with no memory of any
previous session can pick up a milestone and execute it. Each milestone section is intended to
be read as a standalone prompt, together with `00-GLOSSARY.md`, `05-HARD-RULES.md` and the
current state in `03-PROGRESS.md`.

**Maintenance rule:** when reality forces a change to the plan, edit this document *and* append
an entry to `02-DECISIONS.md` in the same commit. Do not accumulate undocumented deviations,
and do not rewrite the plan for every small obstacle — a plan that changes weekly is not a plan.

---

## Standing context (prepend mentally to every milestone)

A filesystem is already a memory — just a very bad one. It stores content but no **episodes**
(when, why, in what context), no **salience** (what matters) and no **associations** (what
belongs with what). That is why people *search* for files instead of *remembering* them.

Hippocampus inverts the priority: **the file manager is not the application. It is one of
several views onto a local memory daemon.**

The governing analogy, which produces concrete design decisions rather than decoration:
the hippocampus does not store memories, it stores indices into neocortical patterns.
Therefore **the brain stores no file contents. The filesystem is the neocortex.** The index
holds pointers, relationships, episodes and condensed representations — and is at all times
fully rebuildable. It is a cache, not a data store.

Biological mapping that carries real weight:

| Structure | Function | Component |
|---|---|---|
| Sensory input | raw perception | `inotify` watcher (see D20) |
| Entorhinal cortex | gateway, spatial map | ingest router + directory graph |
| Dentate gyrus | pattern separation | content hashing, chunk boundaries, near-duplicate detection |
| CA3 | pattern completion | vector ANN + associative graph traversal |
| CA1 | expectation vs. reality | reconciliation, re-ranking, scoring |
| Prefrontal cortex | working memory | Working Set, TTL ≈ session |
| Amygdala / dopamine tags | salience marking | salience engine |
| Sharp-wave ripples | replay at rest | idle-time indexer — expensive work only when idle |
| Sleep consolidation | hippocampus → neocortex | nightly consolidation job |
| Synaptic pruning | active forgetting | decay + tier demotion. **Not optional.** |

**Forgetting is a feature, and it is the one every second-brain project omits.** A system that
only accumulates is large, slow and full of noise after eighteen months, and its owner stops
trusting it. Decay, tiering and eviction are built in M5, not "later". Pins are immune.
Forgetting is safe precisely because the index is reconstructible.

---

## Milestone map

Order follows decision **D2**: brain first, full file manager committed rather than optional.
The numbering differs from the source spec because the order does; the mapping is given.

| Milestone | Content | Source-spec equivalent |
|---|---|---|
| **M0** | Skeleton: workspace, schema, RPC socket, CLI | M0 |
| **M1** | Perception: watcher, metadata index, episodes, path search | M1 |
| **M2** | Meaning: extraction, chunking, embeddings, hybrid search | M2 |
| **M3** | Agent Surface: MCP, policy engine, undo journal, audit | M4 |
| **M4** | Search Overlay: first GUI, launcher-style, preview pane | (new — Option B core deliverable) |
| **M6** | Human View: the full file manager | M3 |
| **M5** | Memory: salience, decay, tiering, consolidation, local graph | M5 |
| **M7** | Federation: outbox protocol in Core, bridge in Extension | M6 |
| **M8** | Agent View: brain-state visualisation | M7 |
| **M9** | Release: packaging, onboarding, second-user install | M8 |
| **M10** | The explorer as a memory browser — see below | (new) |
| **M11** | The Agent Brain: memories as nodes, context packs, the learning signal, the Inspector | (new — D74) |

**M0–M3 are headless.** That is deliberate. If the search does not convince in the CLI, it will
not convince in a GUI.

**M10 is specified elsewhere**, because it is an argument before it is a task list:
[`paper/A-DIRECTORY-IS-JUST-ONE-QUERY.md`](paper/A-DIRECTORY-IS-JUST-ONE-QUERY.md) restructures
the window around the memory — one row stream with many origins, a directory being one of them —
in four shippable steps with acceptance criteria. Its scope and Definition of Done are §M10 of
[`paper/BUILD-PROMPT.md`](paper/BUILD-PROMPT.md), and
[`paper/EXPLORER-KICKOFF-PROMPT.md`](paper/EXPLORER-KICKOFF-PROMPT.md) is the session prompt for
picking it up. **It waits behind M6's mandatory list**: a file manager that cannot do the basics
will not be used, however good the brain is.

**M11 does not wait behind M6**, and that is a deliberate exception rather than an oversight.
M6 gates the *window*; M11 is mostly daemon, and its one window component — the Agent Inspector —
is a panel inside the existing toplevel rather than a new surface. The owner asked for it on
2026-08-23 and the reasoning is in D74. M6's mandatory list is still unfinished and still gates
M10.

---

## Benchmarks — the actual specification

These are CI gates. Write the benchmark before the implementation. A milestone is not done
while its benchmarks are red. Measurements are P95 on the reference corpus, on the target
workstation class (see `03-PROGRESS.md` §Environment).

| Path | Budget | Gate from |
|---|---|---|
| Keystroke → first visible row | **80 ms** | M4 |
| Fuzzy path match (in-memory trigram) | 10 ms | M1 |
| BM25 top-100 | 30 ms | M2 |
| Vector ANN top-100 | 40 ms | M2 |
| RRF fusion + salience boost | 5 ms | M2 |
| Cross-encoder rerank top-30 (async, GPU) | 150 ms | M2 |
| Open a directory with 10 000 entries → first row | 100 ms | M4 |
| Daemon cold start → socket accepting | 200 ms | M0 |
| GUI cold start → interactive | 300 ms | M4 |
| Watcher event → per-event processing cost | 5 ms | M1 |
| Idle RSS, daemon, 100 k files indexed | 250 MB (anonymous memory — see D27) | M1 |
| Idle CPU, daemon, no filesystem activity | < 0.5 % of one core | M1 |
| `brain.context`, 2 048-token budget, 5 000 memories | **120 ms** | M11 |
| Memory ANN top-100 in the memory collection | 5 ms | M11 |
| Agent Inspector open and streaming | < 1 % of one core | M11 |

The reference corpus is generated by `bench/corpus/generate.sh` — synthetic, checked in as a
generator not as data, reproducible from a seed, ~100 000 files with a realistic mix of
text, code, PDF, images and binary noise, and a realistic directory shape.

Its **contents** are as much part of the specification as its shape: word frequencies follow
Zipf's law, because a corpus in which every term occurs in every document makes BM25 measure a
full scan rather than retrieval (decision D23). Numbers measured before that change are not
comparable with numbers measured after it.

---

## Data model

One SQLite file, WAL mode, at `$XDG_STATE_HOME/hippocampus/cortex.db`, mode `0600`.

```sql
-- The neocortex pointer: what exists, where
nodes(id, kind, path, parent_id, mime, size, mtime, ctime, inode, dev,
      content_hash, tier, salience, pinned, zone_id, first_seen, last_seen)
      -- kind: file | dir | chunk | concept | annotation | memory   (memory: M11)
      -- tier: working | stm | ltm | cold

-- The hippocampal index: associative connections
edges(src, dst, type, weight, created_at, source)
      -- type: contains | co_access | imports | similar_to | derived_from | annotates
      --     | supersedes | recalled_with                                    (M11)
      -- source: observed | inferred | explicit

-- Episodic memory: append-only, never mutated
episodes(id, ts, actor, action, node_id, session_id, metadata_json)
      -- actor: human | agent:<name> | system
      -- action: create | modify | open | move | delete | search | annotate
      --       | remember | outcome | supersede | forget_memory              (M11)

-- Pattern separation units
chunks(id, node_id, ordinal, byte_start, byte_end, hash, text_preview)

-- Vectors, versioned by model
embeddings(chunk_id, model_id, model_version, dim, vector, created_at)

-- Lexical index
chunks_fts USING fts5(text, content='chunks', content_rowid='id')

-- Spatial zones (see D15)
zones(id, root_path, name, color_hue, icon, user_override)

-- Federation
sync_outbox(id, event_id, lamport, payload_json, created_at, attempts, last_error)
sync_state(peer, last_acked_lamport, last_sync_at)

-- Explainability
salience_log(node_id, computed_at, score, components_json)

-- What an agent knows that is not in a file (M11). Derived from `remember` episodes the
-- same way `annotations` is; see R10 and D35.
memories(node_id, scope, kind, text, subject_id, confidence,
         usefulness, helped_count, noise_count, human_confirmed,
         expires_at, superseded_by, session_id, episode_id, actor, ts)
         -- scope: session:<id> | agent:<name> | project:<zone_id> | global
         -- kind:  fact | preference | procedure | decision | failure | observation

-- One agent's connection to the surface, so attribution can answer "this task" (M11)
agent_sessions(id, name, task, started_at, ended_at, calls, refusals)

-- What a context pack contained, so an outcome report can be attributed to items (M11)
context_packs(id, session_id, task, budget_tokens, used_tokens, created_at)
context_pack_items(pack_id, node_id, rank, arm, score, reported)

-- Agent safety
undo_journal(id, ts, actor, operation_json, inverse_json, applied, undone_at)
audit_log(id, ts, actor, tool, args_json, result_summary, allowed)
```

`episodes` is the only table that is never mutated. Everything else is derivable from it plus
the filesystem. This is the technical implementation of "the index is a cache" and it is what
makes aggressive forgetting risk-free.

---

# M0 — Skeleton

**Goal:** a daemon that starts, owns a database, accepts RPC, and answers a trivial search from
a seeded corpus. No watching, no extraction, no models.

### Scope
- Cargo workspace with the crates listed in `CLAUDE.md`.
- Config: layered `system defaults → user config → environment`, TOML at
  `$XDG_CONFIG_HOME/hippocampus/config.toml`. A missing config file is valid and yields
  working defaults. Config schema lives in `cortex-core`.
- SQLite schema above, with a forward-only migration runner. Migration `0001` creates
  everything. WAL, `synchronous=NORMAL`, `foreign_keys=ON`, busy timeout.
- JSON-RPC 2.0 over a Unix socket at `$XDG_RUNTIME_DIR/hippocampus/cortex.sock`, mode `0600`.
  Methods for M0: `ping`, `status`, `search`, `node.get`, `episode.append`, `shutdown`.
- Daemon lifecycle: single-instance lock, clean shutdown on SIGTERM, crash-safe (the database
  survives `kill -9`).
- `cortex` CLI: `ping`, `status`, `search <query>`, `index <path>` (manual, recursive, metadata
  only), `db path`, `db vacuum`.
- Structured logging (`tracing`) to stderr, level from config, no log file by default.
- `cortex-rpc` exposes typed protocol structs shared by daemon and clients — never hand-rolled
  JSON on either side.

### Out of scope
Watcher, extraction, embeddings, FTS, GUI, MCP.

### Definition of Done
1. `cargo test --workspace` green.
2. `cortex index ./bench/corpus/small && cortex search foo` returns matching rows, ranked,
   in a stable order.
3. Daemon cold start to accepting connections < 200 ms, asserted by a benchmark.
4. `kill -9` on the daemon mid-index, restart, `cortex status` reports a consistent database
   and no corruption. Asserted by a test.
5. Two daemons cannot run against the same database — the second exits with a clear message.
6. Running the daemon with no config file and no state directory works and creates both.

---

# M1 — Perception

**Goal:** the daemon sees the filesystem in real time, remembers what happened, and can find
any path instantly.

### Scope
- **Watcher:** `inotify`, one watch per directory, managed by us rather than through an
  abstraction layer. See **D20** — the source spec's `fanotify` mandate was measured and
  rejected: it requires `CAP_SYS_ADMIN`, and the watch-limit argument against `inotify` does not
  hold at this corpus size (5 109 directories against a 65 536 watch budget).
  A **watch budget** is enforced and any degradation is reported in `cortex status`, so
  exhaustion is visible rather than silently producing a stale index.
- **Debouncing:** editors write a file three to five times per save. Coalescing window ~500 ms,
  configurable. Coalescing is per (path, event-class).
- Each event captures path, event type, mtime, size, inode. **No content is read.**
- Every event writes an **episode** immediately. Episodes are the raw material for everything.
- **Deny-list enforcement** per `05-HARD-RULES.md` §R1, applied at classification time, before
  anything opens a file. Plus `.gitignore` and `.brainignore` semantics (§R2).
- **Metadata indexer:** populates `nodes` for files and directories, maintains `parent_id`,
  and detects moves rather than delete+create. Two mechanisms, in this order: `IN_MOVED_FROM`
  and `IN_MOVED_TO` paired by their cookie, which identifies a rename exactly; and `(dev, inode)`
  identity as the fallback for moves that only the reconcile scan ever sees.
- **Reconcile scan:** a periodic and on-demand full walk comparing mtime and size against the
  index, repairing drift from dropped events. This is not optional — watcher events *are* lost
  under load, and an index that silently drifts is worse than no index.
- **Trigram path index:** in-memory, built at startup from `nodes`, incremental thereafter.
  Serves fuzzy path matching. Must survive a 100 k-file corpus inside the memory budget.
- **Zones:** detection of major roots (configurable, with sensible defaults derived from XDG
  user dirs and detected project roots), stable hue assignment per D15.
- `cortex` CLI additions: `watch status`, `reconcile`, `episodes [--since]`, `zones`.

### Out of scope
Reading file contents in any form.

### Definition of Done
1. 100 000 files indexed from the reference corpus. Full initial index < 60 s.
2. Fuzzy path search P95 < 10 ms over that corpus, asserted by benchmark.
3. Per-event watcher processing cost P95 < 5 ms.
   **This is a cost budget, not an end-to-end latency budget.** Coalescing deliberately delays
   emission by the debounce window, because one save is one event however many times the editor
   wrote the file. End to end, a change becomes searchable in roughly `debounce_ms` plus the
   loop's poll interval — under a second at the defaults. Do not "optimise" the delay away;
   it is the feature.
4. 24-hour soak with synthetic filesystem churn: RSS growth < 5 % from hour 1 to hour 24,
   no file-descriptor growth. Asserted by a soak test runnable in CI nightly.
5. Idle CPU below 0.5 % of one core with no filesystem activity.
6. Deny-list test: a file matching each built-in pattern is present in the corpus; the test
   asserts that no extractor was ever invoked on it and that its content never entered any
   table. This test must be impossible to pass accidentally — assert on an instrumented
   extractor call counter, not on absence of output.
7. Move detection: `mv` of a 10 000-file directory produces move episodes, not 10 000
   delete+create pairs.
8. Reconcile after killing the watcher for 10 minutes of churn restores an index identical to
   a from-scratch rebuild. Compared by hash of a canonical dump.

---

# M2 — Meaning

**Goal:** hybrid retrieval that is fast, multilingual, and able to explain itself.

### Scope
- **Classifier:** MIME by magic bytes, never by extension. Decides `metadata_only` | `extract`
  | `deep_extract`. Content extraction only inside configured roots (§R3).
- **Extractors:** text and code directly; PDF via pdfium; office formats via their parsers;
  images: EXIF always, OCR opt-in. Audio/video transcription is **out of scope for M2** — it is
  an optional plugin later. Every extractor runs with a wall-clock and memory cap and cannot
  take down the daemon.
- **Chunker:** structure-aware, not naive character splitting. Markdown by heading, code by
  function via tree-sitter, PDF by page and section. Target 300–600 tokens, ~15 % overlap.
  **Content hash per chunk** — unchanged chunks are never re-embedded. This is the single
  largest performance lever in the whole ingest pipeline.
- **Embedder:** `multilingual-e5-small`, ONNX Runtime, CUDA execution provider with CPU
  fallback, batched. Model id and version stored per chunk (D6). The model file is *not*
  vendored in the repo; it is fetched on first use by an explicit user action (§R5) and
  verified by checksum. The runtime library is provisioned the same way and for the same
  reasons (D25). Vectors are keyed by the hash of the text rather than by the chunk row, which
  is what keeps `--rebuild-index` affordable (D26).
- **Idle-priority queue:** extraction and embedding are the "replay" stage — expensive work runs
  when the machine is idle. Hard resource budgets: nice level, a configurable CPU share, a GPU
  memory cap, and immediate yield when the user becomes active. A first index run must never
  make the machine feel slow. How "active" is determined, and why it is a signal from clients
  rather than something the daemon detects, is decision D22.
- **Lexical index:** FTS5 over chunk text, BM25 ranking.
- **Vector index:** usearch HNSW, `f16`, memory-mapped, rebuildable from `embeddings`.
- **Fusion:** parallel trigram + BM25 + vector, combined by Reciprocal Rank Fusion, then a
  salience boost. First results are never blocked on the reranker.
- **Reranker:** cross-encoder over top-30, asynchronous, delivered as a result update. Two
  things it needs that nothing else in M2 did: a second model, and a protocol that can deliver a
  *second* answer to one request — the socket is request/response today, and server-initiated
  updates are also what M4's progressive results will want. Its 150 ms budget assumes a GPU. Its
  ranking is *fused* with the retrieved one rather than replacing it (D29), and note before
  measuring it that a goldset whose distractors are meaningless cannot say whether it helps
  (D30).
- **Query modes** (heuristically detected, forceable by prefix):
  `locate` (path/name, lexical — the 90 % case) · `recall` (semantic) · `temporal` (episodes
  only, no vectors) · `structural` (graph traversal) · `deep` (includes cold tier and peers).
- **Explainability:** every result carries a score breakdown. `cortex explain <node>` prints
  why it ranked where it did. Without this the search is neither debuggable nor trustworthy.
- **Goldset:** 50 queries, German and English mixed, with judged relevant results, checked in.
  `bench/goldset/` — this is how recall regressions are caught.

### Definition of Done
1. Every benchmark in the table above that is gated at M2 is green in CI.
2. Recall@10 on the goldset is measured, recorded in `03-PROGRESS.md`, and gated against
   regression: a drop of more than 3 points fails CI.
3. German queries perform within 10 % of English queries on the goldset. If not, the embedding
   model is wrong and D6 must be revisited.
4. Re-indexing an unchanged corpus re-embeds **zero** chunks. Asserted by an embedder call counter.
5. Idle RSS < 250 MB with 100 k files indexed.
6. First index run of the reference corpus while the user is "active" (simulated input) does not
   exceed the configured CPU share. Asserted.
7. `cortex --rebuild-index` reproduces a byte-identical canonical dump. This is the §R10 gate.
8. Deleting the usearch index file and restarting rebuilds it from `embeddings` without
   re-running the model.

---

# M3 — Agent Surface

**Goal:** agents can use the brain productively, and cannot damage the filesystem.

### Scope
- **MCP server** (`cortex-mcp`) plus JSON-RPC over the Unix socket, plus the CLI. All three
  expose the same capabilities through one internal API — no drift between surfaces.

**Read / remember**

| Tool | Purpose |
|---|---|
| `brain.search(query, mode, filters, k)` | main search, all modes |
| `brain.recall(cue, depth)` | pattern completion — fragment to context |
| `brain.neighbors(node_id, edge_types, depth)` | associative traversal |
| `brain.timeline(from, to, filter)` | episodic retrieval |
| `brain.working_set()` | what the human is doing right now |
| `brain.explain(node_id)` | why this is relevant; salience breakdown |
| `brain.diff(since)` | what changed — for agents that should not poll |

**File access**

| Tool | Purpose |
|---|---|
| `fs.list(path, glob, depth)` | read a directory |
| `fs.read(path, byte_range?)` | read content |
| `fs.stat(path)` | metadata |
| `fs.write(path, content, mode)` | **gated** |
| `fs.move` / `fs.trash` | **gated** |

**Write to memory (never touches files)**

| Tool | Purpose |
|---|---|
| `brain.annotate(target, tags, note)` | attach knowledge without touching the file |
| `brain.pin(target)` / `brain.forget(target)` | set salience manually |
| `brain.link(a, b, type)` | explicit edge |

- **Policy engine.** Agents get read access to files and write access to the brain, by default.
  Filesystem mutation requires: an **allow-list** of paths (never a deny-list), a **mandatory
  dry-run** that returns a plan which must be confirmed, an **episode with actor attribution**
  per mutation, and an entry in the **undo journal**.
  The reason is not paranoia: an agent with unchecked `fs.move` over an indexed home directory
  can do more damage in 200 ms than a backup undoes.
- **Undo journal:** every agentic mutation stores its inverse. `cortex undo [--last n]` applies it.
- **Audit log** (§R8) and **kill switch** (§R7).

### Definition of Done
1. Claude Code, configured against the MCP server, completes a real task end to end:
   find a file by vague description, read it, annotate it, and link it to another.
2. Red-team test suite: for every gated tool, a test proves it refuses without an allow-list
   entry, refuses without a confirmed dry-run, and produces an audit entry on refusal.
3. Undo test: an agent moves 100 files, `cortex undo` restores the exact prior state,
   verified by hash of a canonical filesystem dump.
4. Kill switch: with the Agent Surface disabled, every agent tool returns a refusal and the
   socket does not accept MCP connections. State survives a daemon restart.
5. The three surfaces (MCP, RPC, CLI) are proven to share one implementation by a test that
   exercises the same operation through all three and asserts identical results.

---

# M4 — Search Overlay

**Goal:** the first GUI. A launcher, not a file manager. This is the milestone that delivers
daily value, and it is the honest test of whether the daemon is good.

### Scope
- Tauri v2 shell, React 18, Framer Motion. Design language per `06-DESIGN-SYSTEM.md`.
- **One window**, summoned by a hotkey, dismissed by Escape. No additional toplevels ever —
  niri owns the layout (see niri constraints below).
- Query input, virtualised result list, **preview pane** on the side.
- **Preview subsystem** per D14: separate thumbnailer process, XDG thumbnail spec, shared cache,
  progressive rendering (cached small first, full-quality replacement), long edge capped at
  2048 px, never decodes full resolution for a small preview. Image, text, code, PDF first page,
  video poster frame. Audio shows waveform metadata only.
- **Context sidebar:** for the selected file, what the brain knows — related files, recent
  episodes, tags, and why it is salient.
- **Cold-tier indicator:** a discreet marker when a result comes from the cold tier, so the
  human can understand the tiering model rather than being confused by it.
- Keyboard-only operation is complete. Every action has a binding. A command palette (Ctrl+K)
  is the primary entry point.
- Zone accent colours per D15.

### niri constraints (these are requirements, not preferences)
- Column width is variable and often narrow. The layout must work from **400 px to full width**.
  Responsive breakpoints, not a fixed desktop layout.
- **No window management of our own.** No popups, no our-own tabs. Internal panes yes,
  additional toplevels no.
- **Keyboard-first is mandatory.** People who run niri dislike reaching for the mouse.
- **Pure Wayland.** No XWayland fallback, no X11 assumptions. Handle CSD correctly or let the
  compositor decorate.
- Startup matters: tiling users open and close windows constantly. The daemon is already
  running; the GUI only has to connect.

### Definition of Done
1. GUI cold start to interactive < 300 ms, measured with a real timer in the real app.
2. Keystroke to first visible row < 80 ms P95, measured in the GUI, not in the daemon.
3. A directory of 10 000 entries renders its first row in < 100 ms and scrolls at 60 fps.
   DOM node count stays bounded — asserted, not assumed.
4. Resizing the window from full width to 400 px never produces horizontal overflow or
   clipped controls. Tested at 400, 600, 900, 1400 px.
5. Preview of a 40-megapixel image appears (cached thumbnail) in < 50 ms and reaches full
   quality without blocking the UI thread.
6. Killing the thumbnailer process mid-preview does not affect search or the daemon.
7. Every function is reachable without a mouse. Verified by a checklist test.
8. Frontend performs no filtering, sorting or searching. Enforced by review and by an
   architectural test asserting the frontend has no data-processing dependencies.

---

# M5 — Memory

**Goal:** the system stops being a search index and becomes a memory — including forgetting.

### Scope
- **Salience engine:**
  ```
  salience = w1·log(1+access_count)
           + w2·recency_decay(last_access)
           + w3·edit_frequency
           + w4·graph_centrality
           + w5·explicit_pin        (dominant, overrides everything)
           + w6·project_context     (inside an active project root)
           − w7·noise_penalty       (build artefacts, caches, download debris)
  ```
  Weights are configurable. Every computation is logged to `salience_log` with its components,
  and `brain.explain` renders them. A ranking that cannot explain itself is not trustworthy.
- **Decay:** `salience(t) = base · e^(−λ·Δt) + Σ(reinforcements)`. Below threshold, a node
  falls out of the hot index.
- **Tiering, not deletion:** STM → LTM → Cold. Cold is read only on explicit deep search.
  **Pins are immune** and never decay.
- **Consolidation engine**, nightly and on extended idle:
  1. **Replay** — load the period's episodes, cluster by session and project.
  2. **Abstraction** — one semantic node per cluster. An LLM is appropriate here: asynchronous,
     not latency-critical, may be the larger model or routed to a peer.
  3. **Edge formation** — co-access, code imports, semantic similarity, shared session →
     weighted edges.
  4. **Salience recomputation** across all nodes.
  5. **Promotion** — nodes above threshold enter the sync outbox (consumed in M7).
  6. **Pruning** — nodes below threshold are demoted a tier.
  7. **Report** — a short, human-readable summary of what was consolidated. This matters:
     a memory whose work is invisible is not believed.
- **Local knowledge graph** — concept nodes and edges live in Core (D10), not only in a peer.
- `cortex consolidate [--dry-run]`, `cortex report [--since]`, `cortex forget <node>`,
  `cortex pin <node>`.

### Definition of Done
1. Simulated 30 days of usage against the reference corpus: search latency at day 30 is within
   10 % of day 1, and index size growth is sub-linear in episode count.
2. Pinned nodes are still in the hot tier after the simulation. Asserted.
3. Consolidation is idempotent — running it twice over the same period produces no duplicate
   concept nodes.
4. The consolidation report is generated and readable, and a human can tell from it what the
   system did without reading logs.
5. `brain.explain` returns a component breakdown for any ranked result, and the numbers sum to
   the score actually used.
6. Full rebuild after aggressive pruning restores every pruned node from episodes plus the
   filesystem (§R10).

---

# M6 — Human View

**Goal:** the full file manager. The honest test is one week of exclusive daily use.

**A file manager that cannot do the basics will not be used, no matter how good the brain is —
and the basics are more work than the brain.** This is the largest scope risk in the project.

*This milestone originally read "do not start it before M0–M5 are green". It was pulled ahead of
M5 at the owner's request after using M4 (D43). The scope risk is unchanged; what the reorder
costs is written up there.*

### Scope — mandatory, none of it optional
- Copy / move / rename with progress, cancellation and a conflict dialog
- XDG Trash specification — a trash can, never `rm`
- Permissions, ownership and symbolic links displayed correctly
- Mounts via `udisks2` (USB sticks, external drives)
- Thumbnails per XDG Thumbnail specification — share the cache, do not reinvent it
- `xdg-mime` for "open with"
- Wayland drag and drop; copy/paste through `wl-clipboard`-compatible mechanisms
- Archives: at minimum extract zip and tar
- Network locations (SMB/SFTP) — may be staged, but a home server makes them relevant

### Scope — the part that makes it Hippocampus
- Dual pane optional, Miller columns optional, list as default
- Search as an **overlay**, not a separate page — results replace the list live
- The M4 context sidebar, now permanent
- Zone colours as a 2 px left border per D15

### Definition of Done
1. Every item in the mandatory list works and has a test where testable.
2. Trash behaviour is spec-conformant, verified against a reference implementation's layout.
3. One week of exclusive daily use by the project owner with no fallback to another file
   manager. This is a real gate, recorded in `03-PROGRESS.md`, not a formality.
4. A copy of 10 GB across filesystems shows accurate progress, can be cancelled, and leaves no
   partial file behind on cancellation.
5. All M4 performance gates still hold in the full application.

---

# M7 — Federation

**Goal:** the Cortex can federate with a Main Brain — and the Core still knows nothing about any
specific one.

### Scope — Core (public repository)
- **Outbox pattern.** Sync events are written to `sync_outbox`. A worker drains it when a peer
  is reachable. **Offline is the normal case, not the exception.**
- **Append-only events with Lamport timestamps.** No bidirectional state merging. The peer
  consumes events; the Cortex consumes ontology updates. Two one-way streets rather than a
  crossing.
- **Idempotence.** Stable event IDs. Duplicate delivery must be harmless.
- **No file content leaves the machine** unless explicitly released. By default only metadata,
  summaries and vectors flow upward.
- **Backpressure.** After three days offline the first sync must not flood the peer. Batch
  limits and rate limiting belong in the first version, not the second.
- A documented wire protocol plus a **reference adapter** that talks to a local test peer
  shipped in the repo. The Core contains no knowledge of any real endpoint.

### Scope — Extension (private repository)
- The concrete bridge: endpoints, schemas, authentication, credential handling.
- Personal ontology and tag taxonomy.
- Project-specific rules and folder semantics.
- Runs as a subprocess plugin (D9) loaded from `$XDG_CONFIG_HOME/hippocampus/plugins/*.toml`.

### Definition of Done
1. One day offline, then a clean reconcile: no duplicates, no lost events, no reordering
   damage. Asserted against the test peer.
2. Replaying the entire outbox twice changes nothing on the peer.
3. Backpressure test: 72 hours of accumulated events sync without exceeding the configured
   rate, and without the daemon exceeding its memory budget.
4. A grep of the public repository for endpoint patterns, private paths and credentials
   returns nothing. This runs in CI.
5. With the Extension plugin absent, every Core test still passes and the daemon starts
   normally. The Core must never notice that the Extension exists.

---

# M8 — Agent View

**Goal:** make the brain state visible so a human can understand what agents see.

Deliberately low priority. Content: graph of concept nodes and their edges, salience as size or
brightness, tier as depth or colour, consolidation events as animation, current Working Set
highlighted. Prefer embedding or reusing an existing force-graph component over building one.

### Definition of Done
1. The consolidation process is visually comprehensible — a human watching it can describe what
   the system did.
2. Rendering 5 000 nodes stays interactive.
3. The view is read-only with respect to the filesystem.

---

# M9 — Release

**Goal:** the second user installs it alone, without help and without access to any private
repository.

### Scope
- Packaging: single binary plus a desktop entry; a `.deb` and an AppImage or Flatpak.
- **Onboarding flow:** which folders to index, which embedding model, first index run with a
  progress display. Under ten minutes to a working system.
- Configuration clearly separated into system defaults, user configuration and extension.
- Documentation: README, installation, configuration reference, MCP tool reference,
  troubleshooting, and an honest limitations section.
- Pre-commit secret scanner and the CI gate from §R9, verified against a seeded fake secret.
- License decided (D13).

### Definition of Done
1. A second person installs Hippocampus on a clean machine, indexes their home directory and
   performs a successful semantic search — with no help and no access to the private repo.
   Timed; the target is under ten minutes to first search.
2. Running entirely without any Main Brain is a documented, tested, first-class configuration.
3. `git log -p` over the entire public history contains no secret and no private identifier.
4. Every benchmark gate is green on the release commit.

# M11 — The Agent Brain

**Goal:** an agent that works through the Agent Surface remembers what it learned, gets it back
at the start of the next session inside a context budget it can afford, and is measurably better
for having done so. The human can see every part of that and correct it.

*Added on the owner's instruction, 2026-08-23, after M8. It is a milestone rather than an
extension of M3 because it changes the data model, and §"What to do when the plan is wrong"
requires that to be written down rather than absorbed. See D74.*

### The one thing this milestone is about

M3 gave agents a surface onto the human's memory. It did not give them a memory. An agent using
Hippocampus today can read what the index knows about files and write notes attached to files —
and that is all. Everything an agent learns that is **not about a file** has nowhere to go: that
a build needs a flag, that an approach was tried and failed, that the owner prefers one library
over another, that a directory called `research` is actually output rather than input. At the end
of the session it is gone, and the next session pays to rediscover it or, worse, repeats the
failure.

The fix is not a second memory system bolted to the side. It is one line:

> **A memory is a node. Everything the file memory already does to files, it does to memories.**

Chunking, embedding, salience, decay, tiering, co-access edges, consolidation into concepts,
`brain.explain`, the graph, the three surfaces and the audit log all apply unchanged. This
milestone adds a node kind, a scope, a retrieval primitive shaped like a context window, and one
feedback signal. It does not add a second brain, a second index, a second ranking model or a
second set of vocabulary — and a design that starts to require any of those has gone wrong.

### Scope

**1. `memory` becomes a node kind.**

A memory has no path, holds its own text, and is indexed like a file's chunk. Its columns beyond
the `nodes` row:

| field | meaning |
|---|---|
| `scope` | `session:<id>` · `agent:<name>` · `project:<zone_id>` · `global` |
| `kind` | `fact` · `preference` · `procedure` · `decision` · `failure` · `observation` |
| `text` | the memory itself, held here because there is no file to re-read it from |
| `subject_id` | optional node the memory is *about*; an `annotates` edge is written with it |
| `confidence` | 0–1, the writer's own claim, never treated as ground truth |
| `usefulness` | learned; see §4 below. Starts at 0, is not the same thing as confidence |
| `expires_at` | NULL for everything except `session:` scope, which is TTL'd |
| `superseded_by` | NULL, or the memory that replaced this one |

**Rule R10 applies and is the hardest part of this milestone.** A memory is not derivable from
the filesystem — the filesystem has never heard of it. So the memory is written as a `remember`
assertion into `episodes`, and the node, the row, the chunk and the embedding are a *projection*
of that episode, exactly as annotations are (D35). `replay_assertions` must rebuild the whole
memory layer, and a test that rebuilds and compares is the acceptance criterion, not a comment.
An assertion carries the target's **path**, never only its node id, because a rebuild renumbers.

**2. Scope is how a shared brain stays usable.**

Every memory is written into the narrowest scope that can hold it, and promoted explicitly.
The default write scope is `agent:<name>`; `session:` is for scratch that must not outlive the
task; `project:` is keyed to the zone the work happened in, which the daemon already computes;
`global` requires an explicit argument and is the only scope a human is asked to confirm.

Retrieval reads `session ∪ agent ∪ project(current) ∪ global` by default and never silently
crosses into another agent's `agent:` scope. The reason is not privacy between robots. It is
that a shared pool with no scoping degrades in exactly one way — one agent's wrong guess becomes
every agent's premise — and that failure is invisible until it is expensive.

**3. `brain.context` — the primitive the whole milestone exists for.**

Every other tool returns rows. An agent does not have rows, it has a context window, and the
work of turning twenty results into something worth spending 2 000 tokens on is work the daemon
can do once, well, with the scores in front of it, instead of every agent doing it badly.

```
brain.context(task, budget_tokens, scope?, kinds?, include_files?) -> ContextPack
```

It returns, in this order and inside the budget:

1. **Preferences and pinned memories** — always, because they are small and their whole value is
   being present. If they do not fit the budget, the budget is too small and the call says so
   rather than silently dropping them.
2. **The working set** — what the human is doing right now, which M3 already computes.
3. **Memories**, ranked by the existing fusion and then **diversified by MMR** (§6), because
   three restatements of one fact are worth one fact and cost three.
4. **Files those memories point at**, as path plus byte range plus one snippet — not content.
   An agent that wants the content has `fs.read` and now knows exactly which bytes to ask for.

Every item carries a **basis**: which arm retrieved it, its score components, and its provenance
(who wrote it, when, from which episode). This is `brain.explain` per item, and it is mandatory —
a memory an agent cannot attribute is a memory it cannot argue with.

The pack has a `pack_id`, and the budget is honoured by **measured tokens, not by an estimate of
characters divided by four**. A budget that is wrong in the agent's favour wastes money and a
budget that is wrong the other way truncates a system prompt at a byte boundary.

**4. `brain.outcome` — the learning signal, and the only one.**

```
brain.outcome(pack_id, helped: [memory_id], noise: [memory_id], note?)
```

This adjusts a `usefulness` component that enters salience. The point of it is a property no
recency-and-frequency model has: **a memory that is retrieved often and never helps must decay
faster than one that is never retrieved at all.** Retrieval without use is evidence *against* a
memory. Without this signal, a memory system's ranking converges on whatever it happened to rank
highly first, and the loop is closed and invisible.

Two honesty constraints, both of them recorded in the schema rather than argued in a comment:

- **An agent reporting on its own success is an unreliable narrator.** Weights are
  `human_confirmed > agent_reported > unobserved`, they are stored separately, and
  `brain.explain` prints which one moved the number.
- **A negative report is not a delete.** `noise` lowers usefulness; forgetting remains the
  decay engine's decision, so one bad session cannot erase a year.

**5. Contradiction is resolved, not accumulated.**

`brain.remember(..., supersedes: [memory_id])` writes a `supersedes` edge and sets
`superseded_by` on the old memory. Retrieval excludes superseded memories unless
`include_history` is set. Additionally the daemon **detects** the case it can detect cheaply: a
new memory whose embedding is above a similarity threshold to an existing one in the same scope
and kind is reported back in the write result as a possible supersession, with the ids — it does
not decide. Deciding would be a language model's job and there is no language model in this
daemon (D68).

A memory system that only appends is a memory system that is wrong more every week. This is the
single most common failure of agent-memory bolt-ons and it is why this is scope, not a nice-to-have.

**6. The vector layer, done properly for this.**

Memories are few and files are many. A single ANN index over both means a memory recall competes
with 122 000 file chunks for twenty slots and loses. Therefore:

- **A second collection.** `memories.usearch`, same `VectorIndex` type, its own file, its own
  keyspace over memory chunk ids. This is not a new dependency and not a new abstraction: it is
  the existing index instantiated twice. Memory recall then searches thousands of vectors rather
  than millions, and the scope filter is applied in SQL over a result set small enough that
  over-fetching is sound — with the over-fetch factor **measured and recorded**, not guessed.
- **MMR over the fused list**, λ configurable, default 0.7. Redundancy is the specific failure of
  embedding retrieval over a memory store, because people and agents restate things.
- **usearch stays; Qdrant is refused.** See D75. Briefly: it is a network service, R5 forbids
  network by default, M9 requires one binary a stranger installs in ten minutes, and the
  measured ANN cost is 1.01 ms P95 against a 40 ms budget. There is nothing to buy.

**7. Sessions, so that attribution is real.**

`agent.session.begin(name, task?) -> session_id` and `agent.session.end(session_id)`. Every
episode, audit row and memory written during it carries the session id. Without this the
`actor` column says `agent:claude` for a year of unrelated work and no question about *this
task* can be answered. Sessions are also what `session:` scope expires against, and what the
Agent Inspector lists.

**8. The Agent Inspector — the button.**

One control in the explorer, and one binding (`Ctrl+Shift+A`), opening a full-height panel.
It obeys D4 like everything else in the window: it renders, it does not decide.

| tab | what it shows | writes? |
|---|---|---|
| **Live** | connected sessions, their calls, latency, refusals, as they happen | no |
| **Memory** | every memory: scope, kind, salience, usefulness, supersession chain, provenance. Filterable. **Editable and deletable by the human.** | yes — deliberately |
| **Policy** | the allow-list, the kill switch, gated tools, and what was refused and why | kill switch only |
| **Undo** | the journal, newest first, one click to undo | yes — deliberately |
| **Architecture** | the M8 three-dimensional view, live, embedded | no |

The Memory tab's write path is not a convenience. A memory an agent formed about the human's
work that the human cannot read and correct is a machine keeping secrets, and the whole project
is an argument against that.

**9. The three-dimensional view goes live.**

M8 renders a snapshot from a file. It gains a live mode over the same IPC door the rest of the
window uses: `brain.graph` on a cursor, memory writes appearing as they are written, a context
pack drawing the edges it pulled, consolidation animating when it runs. Layout stays
deterministic (D66) — live means new data, not a jittering scene.

### Out of scope

- Any language model inside the daemon. Naming, summarising and deciding supersession stay
  outside; the daemon computes, the caller reasons. (D68)
- Cross-machine memory sharing. That is M7 and the outbox already exists for it.
- Automatic memory extraction from an agent's transcript. The agent decides what is worth
  remembering; a daemon guessing that from a conversation it cannot see is a fiction.
- Memory of the *human's* conversations. Out of scope forever, not just for this milestone.

### Definition of Done

1. **Round trip.** An agent writes a memory in one session and `brain.context` returns it in the
   next, with the right basis, through all three surfaces, asserted identically (M3 §5's test,
   extended).
2. **R10 holds over the memory layer.** `cortex rebuild` on an index containing 1 000 memories
   across every scope and kind reproduces a byte-identical canonical dump, including
   supersession chains and usefulness. The dump is the assertion; a passing rebuild that lost a
   column is the failure this criterion exists to catch.
3. **The budget is honoured.** `brain.context(budget_tokens = N)` returns a pack whose token
   count is ≤ N for every N in {512, 2048, 8192} against a store of 5 000 memories, and never
   truncates an item mid-structure. The count is a **structural estimate, biased upwards, and
   reported as an estimate** — not a divisor, and not a tokeniser belonging to a different
   model. It is bounded below by the word count and above by the character count, both
   asserted. D79 says why there is no honest third option.
4. **Latency.** `brain.context` P95 **under 120 ms** for a 2 048-token budget over 5 000
   memories and 100 000 file chunks, warm. Written as a benchmark before the implementation,
   and added to the gate table.
5. **Diversity is worth its cost.** On a seeded store containing deliberate near-duplicates, MMR
   at λ = 0.7 returns strictly fewer duplicate facts than λ = 1.0 at the same budget, and the
   recall of the distinct facts does not fall. Both numbers recorded.
6. **The learning signal moves the ranking, in the right direction.** A memory reported as noise
   ten times ranks below an identically-scored memory reported as helpful once, and a memory
   retrieved fifty times with no outcome reported at all ranks below one never retrieved. The
   third is the counter-intuitive one and it is the one that must be tested.
7. **Scope isolation.** A memory in `agent:a` is never returned to `agent:b`, asserted per
   surface. Promotion to `global` from an agent surface without the explicit argument is refused
   and audited.
8. **Supersession.** A superseded memory does not appear in a context pack; it does appear with
   `include_history`; and the detection of a probable supersession reports candidate ids without
   changing anything.
9. **Kill switch reaches all of it.** With the Agent Surface off, every memory tool refuses, no
   memory is written, and the Inspector says so rather than showing an empty list.
10. **The Inspector is complete and cheap.** Every tab renders at 400 px width, the Live tab
    costs under 1 % of one core while streaming, and closing the panel returns the window to the
    idle cost measured in D59. A panel that costs a core is a panel that gets switched off.
11. **The human can correct the machine.** Deleting a memory in the Inspector writes a `forget`
    episode with `actor = human`, removes it from retrieval immediately, and survives a rebuild
    as a deletion rather than coming back.
12. **A real task, end to end, by a human.** Claude Code against the MCP server: session one
    discovers something non-obvious and remembers it; session two, with no transcript from
    session one, is measurably faster or more correct because of `brain.context`. Recorded in
    `03-PROGRESS.md` with what was asked and what happened — including if it did not help.

---


## What to do when the plan is wrong

It will be, somewhere. The rule is:

- A **small** obstacle (a crate does not work, an API differs) — solve it, note it in
  `03-PROGRESS.md`, move on. Do not amend this document.
- A **decision-level** change (a budget is unreachable, a component must be replaced) — append
  to `02-DECISIONS.md`, edit the affected milestone here, and say so in the commit message.
- A **scope-level** change (a milestone must be split, reordered or dropped) — do not do it
  silently. Write the proposal into `03-PROGRESS.md` under "Open questions for the owner"
  and ask. Reordering milestones without agreement is how a plan dies.

Never carry an undocumented deviation into the next session. The next agent has only these files.
