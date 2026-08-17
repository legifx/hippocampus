# 02 — Decision Log

Append-only. Never rewrite a past entry — supersede it with a new one and mark the old one
`SUPERSEDED BY D<n>`. Each entry states the decision, the reason, and what would make us
revisit it. A decision without a "revisit trigger" is a dogma and will hurt later.

Status values: `ACCEPTED` · `PROVISIONAL` (holds until a named milestone re-evaluates it) ·
`OPEN` (must be answered before the named milestone) · `SUPERSEDED`.

---

### D1 — Headless daemon is the core; every UI is a client
**Status:** ACCEPTED

The Cortex daemon owns all state and logic. GUI, CLI, TUI and MCP server are equal clients
speaking JSON-RPC over a Unix socket.

**Why:** testability without a display, agent access as a first-class path rather than an
afterthought, and a GUI technology change costs ~20 % of the codebase instead of 100 %.

**Revisit if:** never. This is the load-bearing decision of the project.

---

### D2 — Build order: brain first, full file manager committed as M3
**Status:** ACCEPTED — *decided by the project owner*

Source spec §15 offered Option A (full file manager from the start) and Option B (brain first,
file manager later or never). Chosen: **Option B's ordering with Option A's ambition** — the
complete file manager is a firm M3 deliverable, not a "maybe".

**Why:** a full file manager is the largest time sink and the most likely reason to stall, while
delivering the least novelty. The daemon plus a search overlay delivers most of the daily value
at a fraction of the effort — and proves the core before the expensive part starts.

**Revisit if:** M0–M2 slip past their budget. Then M3 gets descoped, not delayed.

---

### D3 — Daemon language: Rust
**Status:** ACCEPTED

**Why:** the latency budgets are not reachable in Python; `fanotify`, tree-sitter, ONNX Runtime,
SQLite and HNSW bindings all exist and are mature; single-binary deployment makes the public
release trivial.

**Revisit if:** never for the daemon. Plugins may be written in any language — they are separate
processes.

---

### D4 — GUI stack: Tauri v2 + React 18 + Framer Motion
**Status:** PROVISIONAL until M3 kickoff

**Why:** the project owner's design system (`island-ui`) is a React + Framer Motion component
library with a spring-based motion model and CSS custom properties. Matching that visual
language in GTK4 would mean reimplementing its entire motion system by hand. The Agent View
(force-directed graph) is trivial in a web stack and expensive in GTK. D1 makes a later swap cheap.

**Two hard conditions attached to this decision:**
1. Every list is virtualised — only visible rows exist in the DOM. Without this the view
   collapses at 10 000 files.
2. **No data processing in the frontend.** The frontend renders what the daemon sends. It does
   not filter, sort or search.

**Revisit at:** M3 kickoff. If condition 1 cannot be met against the M3 benchmark, switch the
GUI layer to GTK4 + libadwaita. The daemon is unaffected.

---

### D5 — Storage: one SQLite file, FTS5 for lexical, usearch HNSW for vectors
**Status:** PROVISIONAL until M2 benchmarks

- Metadata, episodes, chunks, edges, salience, outbox → **SQLite in WAL mode**, one file.
- Lexical/BM25 → **SQLite FTS5** in the same file.
- Vector ANN → **usearch** HNSW index in a separate memory-mapped file, with `f16` quantisation.
  SQLite holds the authoritative embedding rows; the ANN file is rebuildable from them.

**Why:** the realistic corpus is ~78 000 files / 8 GB (measured), yielding an estimated
150 000–250 000 chunks. Brute-force cosine over that is 30–100 ms — too slow for the 40 ms
budget, so a real ANN index is required. Tantivy is not needed at this corpus size and its
separate index directory works against the "one file to back up" advantage.

**Revisit if:** the M2 vector benchmark misses 40 ms P95, or the corpus grows past ~1 M chunks.
Escalation path: LanceDB. De-escalation path: `sqlite-vec` brute force if the corpus turns out
much smaller.

---

### D6 — Embeddings run locally on the workstation GPU, never on the server
**Status:** ACCEPTED

Model: **`multilingual-e5-small`** (384 dimensions) via ONNX Runtime, CUDA execution provider,
CPU fallback. Model id and version are stored per chunk so a model change is a background
migration, not a full rebuild.

**Why:** the corpus is a German/English mix, which disqualifies English-only models outright.
The workstation has a 16 GB RTX-class GPU; the home server has none, 15 GB RAM and roughly
thirty always-on services. Pushing embedding work to the server would make it the bottleneck
and put a hard dependency on the network into the hot ingest path.

**Revisit if:** a benchmark shows recall on the German query goldset is unacceptable. Escalation
path: `bge-m3` or `jina-embeddings-v3`, both multilingual, both heavier.

---

### D7 — Reranking: cross-encoder, asynchronous, never blocking
**Status:** PROVISIONAL until M2

`bge-reranker-v2-m3` (ONNX) over the top 30 fused results, delivered as a second update after
the first result list is already on screen.

**Why:** a cross-encoder beats LLM reranking decisively on cost/benefit, and the progressive
architecture means a slow reranker degrades quality, not usability.

**Revisit if:** the 150 ms budget is missed. Fallback: skip reranking, keep RRF ordering.

---

### D8 — No LLM in the hot search path
**Status:** ACCEPTED

Query understanding by a small local model is an optional, asynchronous *third* refinement stage
(~400 ms), and is deferred to M5 at the earliest. First results are always lexical + vector.

**Why:** the benchmark for search is `fzf` and Spotlight, not a chat interface. A result list
that appears after one second loses to typing the path by hand.

**Revisit if:** never for the hot path.

---

### D9 — Two separate repositories, not a monorepo with a private submodule
**Status:** ACCEPTED

- `hippocampus` — public Core.
- `hippocampus-ext` — private Extension: sync bridge to a concrete Main Brain, personal
  ontology, project rules, secrets configuration.

The Extension is loaded as a **subprocess plugin** speaking the same JSON-RPC as every other
client. No dynamic linking, no ABI, no language binding.

**Why:** a submodule leaks the existence, name and commit history of the private repo into the
public one. Separate repos plus a process boundary means the Core cannot accidentally grow a
dependency on private code — the split is enforced by the runtime, not by discipline.

**Revisit if:** never. Retrofitting this split after the fact is the failure mode this avoids.

---

### D10 — The Core is fully functional with no Main Brain at all
**Status:** ACCEPTED — *decided by the project owner*

The second user will run Hippocampus **as his own Main Brain**, with no server behind it.
Therefore local LTM is a first-class, complete memory tier — not a write-through cache in front
of a server. Federation is strictly additive.

**Why:** it is the stated use case, and it forces the cleaner architecture. A Core that needs a
server to be useful is badly cut.

**Consequence:** consolidation, concept nodes and the knowledge graph are **Core** features and
must be implemented locally. The Extension may mirror them upward; it may not own them.

**Revisit if:** never.

---

### D11 — Federation transport: outbox, append-only events, no bidirectional merge
**Status:** ACCEPTED — refined by D17, D18 and D19 after surveying a real peer

The Cortex writes sync events into a local outbox table. A worker drains it when a peer is
reachable. Events carry stable IDs and Lamport timestamps; duplicate delivery is a no-op.
Ontology flows down, events flow up. Two one-way streets, never a crossing.

Authority split:

| Data | Authority |
|---|---|
| Files, paths, mtimes, local structure | Cortex |
| Episodes | Cortex |
| Semantic concepts, cross-project knowledge | Main Brain (if present) |
| Ontology and tag taxonomy | Main Brain (if present) |
| Salience | Both, kept separately — local and global salience are different quantities |

**Revisit at:** M7, against the actual peer. The survey that produced D17–D19 already confirmed
the shape to plan for: a peer may have no ingest API at all, no idempotent writes, and no spare
memory for a resident service. The Core assumes that peer, not a friendly one.

---

### D12 — Language: English code and docs, internationalised UI defaulting to German
**Status:** ACCEPTED — *decided by the project owner*

**Why:** the repository is intended for publication; the primary users are German-speaking.
An i18n layer added later means touching every string in the UI.

---

### D13 — License
**Status:** OPEN — must be answered before M9 (public release)

Recommendation: **Apache-2.0** — permissive, includes an explicit patent grant, and is the least
friction for a tool that others may embed.

---

### D14 — Preview subsystem: separate process, separate budget
**Status:** ACCEPTED

File previews (the side panel) run in a **separate thumbnailer process**, not in the daemon and
not in the GUI process. It obeys the XDG Thumbnail spec and shares the standard thumbnail cache.

**Why:** preview generation touches untrusted file content with third-party decoders — the most
likely place for a crash or a memory-safety bug in a dependency. Isolating it means a malformed
image kills a helper, not the memory daemon. It also lets previews be killed under load without
affecting search.

**Quality rule:** previews are rendered at the displayed size times device pixel ratio, capped
at 2048 px on the long edge, and progressive — the cached small thumbnail appears first, the
full-quality render replaces it when ready. Never decode a full-resolution image to show a
300 px preview.

---

### D15 — Colour-coded zones are derived, not decorative
**Status:** ACCEPTED

Major areas of the tree get a subtle accent colour used as a 2 px left border on rows and as the
accent token in the island-ui sense — not as a background fill. Colours are **derived
deterministically** from the zone identity (stable hash → hue), with saturation and lightness
fixed by the theme so no colour can shout. Users can override any zone's colour, and can turn
the whole system off.

**Why:** colour that changes between sessions is noise. Colour that is stable becomes a spatial
memory aid, which is precisely the point of the project. Fixed S/L guarantees no combination
becomes visually loud.

**Accessibility:** colour is never the only signal — every zone also has a name and an icon.

---

### D16 — Chunk text is never stored; the lexical index is contentless
**Status:** ACCEPTED

`chunks` stores byte offsets, a content hash and a short bounded preview. The FTS5 index is
created with `content=''` — it tokenises text for BM25 without keeping a copy. Snippets longer
than the preview are re-read from the file using the stored offsets at query time.

**Why:** this is the direct implementation of "the filesystem is the neocortex". A database
holding the full text of every indexed file is a second copy of the user's data with different
permissions, different backup semantics and different deletion semantics — exactly the lock-in
the project exists to avoid. An inverted index is not a copy.

**Consequence:** rebuilding FTS requires re-reading files, which is why `--rebuild-index` is a
first-class tested command rather than an emergency measure.

**Revisit if:** snippet generation turns out to dominate query latency. Mitigation before
revisiting: widen the stored preview, which is bounded and cheap.

---

### D17 — The federation transport is pluggable, and a file-drop transport is a first-class case
**Status:** ACCEPTED

`cortex-sync` defines the event protocol independently of how bytes reach the peer. The Core
ships at least two transports: a direct request/response transport (the obvious one) and a
**file-drop transport** that writes one document per event into a directory the peer ingests
on its own schedule.

**Why:** this is not hypothetical generality. A realistic peer is a machine that is already
running many services, has little spare memory, exposes nothing to the network, and already has
a working ingest pipeline that watches a directory. Demanding that such a peer grow a new
authenticated HTTP endpoint is asking it to take on a security surface and a resident process
for our convenience. Dropping files into a directory it already watches costs it nothing.

The file-drop transport is also the strictly safer default: it fails closed when the peer is
unreachable, it needs no credentials beyond existing transport-level access, and it inherits
whatever deduplication the peer already performs on files.

**Revisit if:** never as a removal. Additional transports may be added.

---

### D18 — The Cortex owns idempotency, and never assumes the peer provides it
**Status:** ACCEPTED

Every sync event carries a stable, content-independent event ID and a Lamport timestamp. The
outbox marks a row acknowledged only after the peer state is confirmed. Resends reuse the same
identifier and the same target name so that any deduplication the peer *does* have can engage.

**Why:** a survey of a real peer showed that its write path always creates a new record, assigns
random identifiers, and deduplicates only in a nightly batch job with a hard cap on how many
merges it performs. A sender that assumes "the peer will merge duplicates" would produce them
faster than that peer removes them. Assuming the peer is dumb costs almost nothing and is
correct against every peer, including smart ones.

**Consequence for the protocol:** rate limiting, batch size and a configurable quiet window are
**protocol features, not deployment tuning**. A peer may have periods where a bulk push is
actively harmful, and the sender must be able to respect that without code changes.

---

### D19 — Vector spaces are not shared across the federation boundary
**Status:** ACCEPTED

The Cortex embeds locally with its own model and keeps its own index. Federation sends
**condensed text and concepts**, and lets the peer embed with whatever model the peer uses.
Vectors are only ever shipped between two ends that are known to run the identical model and
dimensionality.

**Why:** sharing a vector space forces both ends onto one model. If the peer's model is remote
or API-hosted, that puts a network round trip into local ingest and breaks offline operation
(D6, R5). If the peer's model is large, it dictates the workstation's hardware. Text is the
portable format; vectors are an implementation detail of whoever searches them.

**Revisit if:** both ends are ever deliberately standardised on one model. Then shipping
vectors becomes a pure optimisation and can be added without protocol changes.
