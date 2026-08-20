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

### D21 — Path search has two tiers, and the second is only reached when the first finds nothing
**Status:** ACCEPTED

Tier one is trigram-filtered and finds every path containing the query **literally**. Tier two
is a full scan that also accepts scattered subsequence matches, and it runs **only when tier one
returned nothing at all**.

**Why two tiers:** a trigram index cannot accelerate subsequence matching. Nothing can — that is
why `fzf` simply scans everything. Contiguous matching is the case that indexes are good at, and
it is also the overwhelmingly common case.

**Why the condition is "nothing at all" rather than "fewer than the limit":** if thirty files
literally contain what the user typed, showing them two hundred loosely related subsequence
matches makes the result worse, and paying for a full scan to produce that dilution would be the
worst of both outcomes. Searching harder is what you do when the obvious answer is empty, not
what you do routinely.

The practical effect is that abbreviations still work — `prj3` shares no trigram with
`project3`, so tier one returns nothing and tier two finds it — while the common case never
leaves the fast path.

**Revisit if:** measurement shows the full-scan tier being entered often enough to matter. The
fix would be to parallelise the scan across cores, not to widen the condition.

---

### D20 — `inotify` is the primary watcher; `fanotify` is an optional privileged upgrade
**Status:** ACCEPTED — supersedes the source specification's watcher choice

The source spec (§5.1) mandates `fanotify` with `FAN_REPORT_FID` and rejects `inotify` outright
on the grounds that per-directory watch limits do not scale to a home directory. Both halves of
that turned out to be wrong for this system.

**Measured, not assumed:**

| | |
|---|---|
| `fs.inotify.max_user_watches` | 65 536 |
| Directories in the reference home directory | 5 109 |
| Headroom | ~12× |

`inotify` needs one watch per *directory*, not per file. Five thousand watches against a budget
of sixty-five thousand is not a scaling problem, and the deny-list removes the directories that
would actually explode the count (`node_modules`, `.git`, caches) before a watch is ever placed.

**The decisive argument is privilege, not scale.** `fanotify` with `FAN_MARK_FILESYSTEM` and
`FAN_REPORT_FID` requires `CAP_SYS_ADMIN`. Asking a user to grant `CAP_SYS_ADMIN` to a daemon
whose entire job is to read their home directory is a far larger security concession than the
feature is worth, and it contradicts rules R5 and R6. A tool that demands near-root to watch
files will not be installed by the second user, and should not be.

**Consequences:**

1. `cortex-ingest` uses `inotify` directly, not the `notify` abstraction — the coalescing window
   and the move-detection semantics are ours to control, and `notify` uses `inotify` underneath
   in any case.
2. Move detection gets *better*, not worse: `IN_MOVED_FROM` and `IN_MOVED_TO` carry a matching
   cookie, which identifies a rename exactly. Inode comparison remains the fallback for moves
   that only the reconcile scan sees.
3. A watch budget is enforced with a reported degradation, so exhaustion is visible in
   `cortex status` instead of silently producing a stale index.
4. `fanotify` may be added later as an opt-in path for users who *choose* to grant the
   capability. It is not required for correctness, because the reconcile scan is the backstop
   for dropped events either way.

**Revisit if:** a target system is measured with an unfavourable ratio of directories to
available watches — for example a home directory past ~50 000 directories on a default kernel
configuration. The escalation path is raising `max_user_watches` (a documented, unprivileged-
to-request sysctl) before it is `fanotify`.

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

---

### D22 — User activity is signalled by clients, and the expensive stages run on a duty cycle
**Status:** ACCEPTED

The extraction stage — and later the embedder — must yield when the user is active (M2 scope,
resource budgets). Two parts to that: knowing when the user is active, and yielding.

**Knowing.** Activity is *signalled*, never sniffed. Any client that knows a human is present
calls `activity.ping`; an interactive request such as `search` counts as its own signal, because
somebody typed it. The daemon reads no input devices, connects to no compositor and infers
nothing from filesystem traffic.

**Why:** reading `/dev/input` needs a privilege this daemon must never hold and exposes
keystrokes it must never see. Asking the compositor makes a headless daemon depend on a
graphical session. Inferring activity from filesystem events mistakes a background download for
a person. A signal is the only source that is both accurate and free of privilege.

**Consequence:** a daemon nobody has spoken to treats the machine as idle and uses the idle
budget. That is correct for a headless install and is corrected the moment an interactive client
connects. The GUI (M4) must ping on interaction, and that is now a requirement on it rather than
an optimisation.

**Yielding.** A duty cycle, not a scheduler priority: work runs in bounded slices with a pause
of `worked × (1 − share) / share` between them. `nice` is applied as well but is not sufficient
on its own — it yields CPU while doing nothing about page cache pressure or disk queue depth,
which is what the user actually notices during a first index run.

The share is a share **of one core**, because the stage is single-threaded. On the reference
workstation that makes the default active share about one per cent of the machine. When
extraction or embedding becomes parallel, the share must be divided across the workers rather
than reinterpreted.

**Revisit if:** a Wayland idle protocol (`ext-idle-notify-v1`) becomes available to a client that
is *already* running for other reasons — the GUI. It would then ping on the compositor's behalf,
which is the same signal from a better source, and needs no change here.

---

### D23 — The benchmark corpus must obey Zipf's law, and its content is seeded per path
**Status:** ACCEPTED

The generated corpus writes real content by default, drawn from a 20 000-word vocabulary sampled
Zipfian, with the searchable domain vocabulary placed at rank 1 000 and function words at the
head. Content is generated from a generator seeded by the file's own path, independent of the
stream that names files.

**Why:** the M1 corpus filled every file with sentences drawn from a list of sixteen, which was
right for a benchmark that only indexed paths and actively misleading for one that measures
retrieval. Every term occurred in almost every document, so a BM25 query had to score a third of
the corpus: **44 ms against a 30 ms budget, caused entirely by the input not being text.** With a
realistic distribution the same code measures 15 ms. Neither number was a property of the
retrieval code; the first was a property of the corpus.

Seeding content per path keeps the tree identical whether or not content is written. Without
that, enabling content consumed draws from the naming generator and produced a different corpus,
so the path numbers from one mode could not be compared with the other.

**Consequence:** benchmark numbers from M0 and M1 were measured against a different corpus and
are not directly comparable with M2's. The path gates were re-measured rather than carried
forward. The corpus is now ~450 MB on disk instead of ~45 MB.

**Also:** the pathological case did not disappear, it stopped being the *typical* case. A query
for a stopword still scores every match — measured and reported by `bench-latency` on every run
so that it stays visible rather than being hidden by a better corpus.

**Revisit if:** retrieval ever needs to be tuned against a distribution this generator cannot
produce. The answer then is a second preset, not a change to this one — a corpus that changes is
a benchmark that cannot be compared with itself.

---

### D24 — `Auto` is the default query mode, and an unbuilt mode degrades visibly
**Status:** ACCEPTED

`QueryMode::Auto` means "the daemon decides", and it is what every client sends unless the user
forced a mode. Detection is a heuristic over the query text, overridable by a prefix
(`locate:`, `recall:`, `when:`, `graph:`, `deep:`). Modes whose machinery does not exist yet —
`temporal`, `structural`, `deep` — run as `recall`, and the response states the mode that
actually ran.

**Why:** the heuristic has to live somewhere, and one implementation in the daemon is better than
one per client that each drift. Reporting the effective mode is what keeps a degradation from
becoming a lie: a client can show "searched content" rather than claiming it searched episodes.

**The heuristic itself:** a query is `locate` when it looks like a location — a path separator, a
leading dot or tilde, or a single token with a plausible file extension. Everything else is
`recall`, which searches paths *and* content. Erring towards content is deliberate: a single
remembered word from inside a document is exactly the case where a file manager's search
normally gives up.

**Revisit if:** the M2 goldset shows the heuristic mislabelling queries. It is one function with
its own tests, and the goldset is the instrument that would show it.

---

### D25 — The model and the ONNX runtime are provisioned like data, not like dependencies
**Status:** ACCEPTED

`multilingual-e5-small` and `libonnxruntime.so` are both fetched by an explicit user action
(`cortex model fetch`), verified against a SHA-256 pinned in `cortex_embed::registry`, and stored
under the state directory. The daemon loads the runtime *by path* at startup (`ort` built with
`load-dynamic`, without `download-binaries`).

**Why not a build-time download:** `ort`'s default feature fetches prebuilt binaries while
compiling, through a client that needs system OpenSSL. That makes `cargo build` require the
network and `libssl-dev`, which is a second user's first failure. Nothing in this project
downloads anything at build time now.

**Why not a system package:** Ubuntu has `libonnxruntime-dev`, and requiring it means asking for
root before somebody can try the project. A 10 MB tarball into the state directory asks for
nothing.

**Why the same treatment for both:** the runtime loads the model into the process. If one is
checksum-verified and the other is whatever happened to be on the machine, the verification is
decoration. `embedding.runtime_path` still points at a system library for anyone who wants that.

**Consequence:** a fresh install has no vector arm and says so, which is rule R5 working rather
than a fault. The hashing embedder covers the pipeline in the meantime.

**Revisit if:** a distribution ships an ONNX Runtime whose ABI this project can rely on across
releases, or if `ort` gains a build mode that neither downloads nor links a system library.

---

### D26 — Embeddings are keyed by the hash of the text, not by the chunk row
**Status:** ACCEPTED

`embeddings` has primary key `(chunk_hash, model_id, model_version)` (migration 0003). The usearch
index still keys by chunk id, because a search must come back to a place in a file, and it is
derived from the table by joining on the hash.

**Why:** an embedding is a pure function of the text and the model. Keying it by row id tied it
to a particular `chunks` row, and three things follow from that, all discovered by building it
the other way first:

1. `--rebuild-index` re-derives chunks from the filesystem, so every vector cascaded away with
   the old rows. On the reference corpus that is hours of inference to recover something that had
   not changed — which makes a supported operation one nobody would run (rule R10).
2. Identical text was embedded once per copy. Boilerplate is the same chunk in dozens of files.
3. Any operation that recreated chunks lost their vectors.

**Consequence:** the ANN index and the table can now disagree — the table survives a rebuild, the
index does not. The embedding stage checks and refills it whenever its queue drains, which is the
same path that satisfies Definition-of-Done §8 (delete the index file, restart, no model needed).
That check has to be cheap, which is what `Store::chunk_watermark` is for.

**Revisit if:** vectors ever become large enough that storing one per distinct text rather than
one per chunk is the wrong trade. At 384 dimensions and `f32` it is 1.5 kB.

---

### D27 — The vector index is memory-mapped, and the memory budget is read as anonymous memory
**Status:** ACCEPTED

The usearch index is opened with `view` (mmap, read-only) and promoted to a heap copy only while
something is writing to it; the embedding stage maps it again once it has caught up.

**Why:** measured on the reference corpus, 122 137 vectors are 104 MiB on disk and about 145 MiB
of heap when loaded. Loaded, the daemon's resident memory was **307 MiB against a 250 MB budget**.
Mapped, the same daemon measures:

| | fresh start | after ten searches |
|---|---|---|
| RssAnon (heap) | 178 MiB | **182 MiB** |
| RssShmem (the mapped index) | 0 | 107 MiB |
| VmRSS (what `top` shows) | 206 MiB | 299 MiB |

**So the budget is restated, not raised:** *idle RSS below 250 MB* now means **anonymous** memory,
because that is the memory the daemon genuinely holds and the only part the kernel cannot take
back. The mapped index is page cache attributed to the process; under pressure it is evicted and
searches get slower rather than the machine swapping.

Stating it any other way would have left two bad options: pretend a 299 MiB process is inside a
250 MB budget, or delete a feature the milestone requires.

**Also worth knowing:** `view` maps the file and trusts its header. Pointed at a truncated one it
does not fail, it **segfaults** — found by the test that writes rubbish into the path. The header
is now read with an ordinary file read first, and only then is anything mapped.

**Revisit if:** the anonymous half approaches the budget on its own. The path index (69.6 MiB for
105 273 paths) is the next largest item and the one with the most obvious slack.

---

### D28 — Embedding is a separate stage from extraction
**Status:** ACCEPTED

Extraction and embedding are two threads over two queues, not one pipeline. Extraction has the
chunk text in hand and could embed it there, saving a file read; the embedding stage re-reads
each chunk through its stored offsets instead.

**Why:** measured on the reference corpus, extracting 100 000 files costs **26 seconds** and
embedding the 122 137 chunks it produces costs **hours** on a CPU. One pipeline would hold the
cheap, high-value work behind the expensive one: lexical search would arrive hours late instead of
in half a minute. The re-read costs a few hundred microseconds against tens of milliseconds of
inference — under one per cent of what it decouples.

**It is also not optional in general.** A model fetched *after* the corpus was indexed has no
other way to see it, and that is the normal case: the model is a deliberate download, so every
first install is exactly that situation.

**Revisit if:** embeddings ever become cheap enough that the two stages have comparable cost —
a GPU changes the ratio by one to two orders of magnitude, but not the direction.

---

### D29 — The reranked order is fused with the retrieved one, never substituted for it
**Status:** ACCEPTED

The cross-encoder scores the top thirty results, and its ranking is combined with retrieval's by
reciprocal rank fusion (`k = 10`) rather than replacing it.

**Why:** substituting was the first implementation, and the goldset rejected it — MRR fell from
0.882 to 0.746 and top-1 from 86 % to 72 %. The cause is visible in `goldset --explain`: when a
cross-encoder finds nothing it likes, its scores collapse into a narrow negative band where the
ordering carries no signal. Replacing a good order with that is strictly worse than leaving it
alone, and it happens exactly when the query is hard — which is when the ranking matters most.

Fusing keeps both opinions. A result the reranker loves rises, one it dislikes falls, and neither
can be discarded by a model having a bad day. It is also the mechanism this project already uses
for scores on incomparable scales, which is precisely what a BM25 score, a cosine similarity and
a cross-encoder logit are.

`k = 10` rather than the 60 used across retrieval arms: there the arms are peers, here the two
orders are opinions about the same short list, and a large `k` would flatten the difference
between rank one and rank thirty into nothing.

**Revisit if:** a goldset with plausible distractors shows the reranker's own order to be
reliably better. See D30 — the current one cannot show that either way.

---

### D30 — Generated distractors must be irrelevant by construction
**Status:** ACCEPTED

The corpus generator writes no real sentences. Earlier it sprinkled sixteen genuine German and
English sentences through generated documents so that a goldset would have memorable phrases to
search for.

**Why it had to go:** the goldset now brings its own documents, and several of its topics were
built on those very sentences. So the "distractors" contained real answers that no judgement had
marked relevant — and a measurement whose distractors contain unjudged answers measures the
opposite of what it claims. It did exactly that: the reranker was scored as having *made
retrieval worse*, when what it had actually done was find a passage that answered the question.
Diagnosing it took `goldset --explain` and reading the file it had promoted.

**Consequence, and the limit it creates:** the distractors are now Zipfian nonsense — plausible
in shape, empty in meaning. That makes recall measurable and reranking *unmeasurable*: a
cross-encoder earns its keep against documents that are plausible **and** irrelevant, and there
are none. Measured on this goldset the reranker changes nothing at all (MRR 0.878 either way),
which is a statement about the corpus rather than about the model.

**Revisit when:** somebody writes a few hundred real distractor documents. That is what would
turn "the reranker changes nothing here" into a fact about the reranker.

---

### D31 — The GPU is shared, so every model runs under a memory cap
**Status:** ACCEPTED

The CUDA execution provider is configured with `embedding.gpu_memory_mb` (1 GB by default), and
the reranker batches eight pairs at a time rather than thirty.

**Why:** the reference workstation has a 16 GB card of which a local language model held 11.7 GB
during this work. Without a cap, a batch of 32 chunks asked for 273 MB it could not get and the
whole pass failed; the reranker later failed the same way at 377 MB. A background indexer that
takes whatever the card has, and dies when it cannot, is a bad neighbour twice over.

Batching is the other half: attention is quadratic in sequence length, and a cross-encoder's
sequences are roughly twice an embedder's because query and passage share one. Eight pairs is
about 100 MB of working set.

**Consequence:** measured throughput on a busy card — 1.85 ms per chunk against 75 ms on the CPU,
and 16.5 ms for thirty reranked pairs against a 150 ms budget. An idle card would be faster; the
point is that these numbers hold while somebody else is using it.

**Revisit if:** the daemon ever needs to *reserve* rather than borrow. It should not: indexing is
background work by construction (D22), and background work that evicts a foreground application
from the GPU has misunderstood what it is.

---

### D32 — Configuration is a hard off; the kill switch is runtime state
**Status:** ACCEPTED

`agent.enabled = false` in the configuration file cannot be lifted over RPC. `agent.enabled =
true` merely permits the runtime switch to decide, and that switch — together with the indexing
pause — lives in the `meta` table, not in the configuration file.

**Why:** the two directions are not symmetrical. Somebody who disabled the Agent Surface in a
file has said something an agent must not be able to talk them out of; somebody who disabled it
at the terminal has said something that must survive the daemon crashing (R7). Writing the
runtime state back into the user's configuration file would be worse than either: a temporary
pause is not an edit to a file the user maintains by hand.

**Consequence:** `agent.status` reports both, `enabled` and `configured_enabled`, so a client can
tell "switched off just now" from "switched off in a file, and your RPC will not change that".

---

### D33 — The dry run is a plan with a fingerprint and an expiry, held only in memory
**Status:** ACCEPTED

A gated tool called without a plan id mutates nothing and returns a plan. The plan records what
was at each target when it was made — size and mtime, not a hash. Applying it re-checks that,
and refuses if it changed. Plans expire after two minutes, are bound to the actor that made
them, are consumed on use, and must describe exactly the call that presents them.

**Why the fingerprint:** a confirmation flag confirms an intention. What needs confirming is an
*effect*, and the effect of "write this file" is different if somebody else created that file in
the meantime. Without the check, an approval for `create` silently becomes an `overwrite`.

**Why not persisted:** a plan that survived a restart would describe a filesystem the daemon has
not looked at since. That is precisely the situation the staleness check exists to catch, so
there is nothing to gain and a false sense of continuity to lose.

**Why size and mtime rather than a hash:** hashing every target to plan a move makes planning
cost what the move costs. The question is "did this change since I looked", and mtime answers it.

**Revisit if:** a client wants to plan a thousand-file reorganisation and confirm it a minute
later. Then the TTL is a configuration value rather than a constant.

---

### D34 — Destructive mutations are made reversible by a trash directory, never by the index
**Status:** ACCEPTED

An overwrite moves the previous version into `$XDG_STATE_HOME/hippocampus/trash/<slot>/` before
writing, and a trashing is that move and nothing else. The undo journal stores the *path* it went
to, and the inverse is a rename back.

**Why:** the alternative is storing the old content in the database, and rule R10 forbids exactly
that — the index is a cache, and a cache that holds the only copy of something is not one. It
would also make the database grow by the size of every file an agent ever touched.

**Consequence:** the trash is bounded by `agent.undo_journal_depth`, and the journal's inverse can
fail honestly — if something else has taken the original path in the meantime, undo reports a
failure rather than overwriting a third party's work. The rename may cross a filesystem boundary
(a home directory on one disk, `~/.local/state` on another), so the move falls back to copy plus
delete on `EXDEV`.

---

### D35 — An agent's knowledge lives in the episode log; the graph is a projection of it
**Status:** ACCEPTED

Annotations, links and pins are written to `episodes` as *assertions* carrying the **path** of
their target, not only its node id. The annotation node, the edge and the `annotations` row are
derived, and `cortex rebuild` replays them with `Store::replay_assertions`.

**Why:** rule R10 says everything must be reconstructible from the filesystem plus the episode
log. An annotation is not on the filesystem — the filesystem has never heard of it — so if it
lived only in `nodes` and `edges`, a rebuild would silently delete everything an agent ever
recorded. That is not a theoretical loss: `rebuild` is a supported operation this project
advertises, not a repair of last resort.

**Why paths and not ids:** a rebuild renumbers every node. An assertion that can only name its
target by a number is an assertion a rebuild cannot replay, and the first version of this stored
exactly that.

**Consequence:** an assertion whose target no longer exists is dropped and *counted* —
`rebuild` reports `assertions_dropped` — rather than resurrecting a node for a file that is gone.

---

### D36 — MCP is a bridge with no policy in it
**Status:** ACCEPTED

`cortex-mcp` is a table mapping one MCP tool to one RPC method, plus stdio framing. It contains
no policy, no tool logic, and no state beyond the connection.

**Why:** M3 §5 requires the three surfaces to be provably one implementation. A bridge that
interpreted anything would be a second implementation, and the interesting case is not that it
would drift — it is that it would be a way *round* the policy engine, which is the one component
whose whole job is to be unavoidable.

**Consequence:** the kill switch reaches the bridge too. With the surface off, `cortex-mcp`
refuses to start rather than starting and refusing every call — a client that sees a tool list
has been told the surface is available, and it must not be told that when it is not. There is an
end-to-end test per claim: same operation through all three surfaces, byte-identical answers.

---

### D37 — D4's conditions are met; the GUI stack stays
**Status:** ACCEPTED (D4 was PROVISIONAL until M3 kickoff)

D4 chose Tauri v2 + React with two conditions attached and a stated fallback to GTK4 if the
first could not be met: every list virtualised, and no data processing in the frontend. Both
are met and both are asserted rather than claimed.

**Virtualisation.** Ten thousand results render under forty rows, and the whole document stays
under four hundred nodes — checked in a real browser at four viewport widths, and again after
scrolling five thousand rows in. The bound is arithmetic (`maxRenderedRows`) and it is asserted
directly as well as through the rendered component.

**No data processing.** An architectural test reads the source: the runtime dependency list may
contain only React, React DOM and Framer Motion; no file may use `filter`, `sort`, `reduce` or
`find`; only `ipc.ts` may import the Tauri API; and no colour may appear outside `tokens.css`.
The one exemption is the virtualiser, which slices a list to what fits on screen — that is
layout, and it is the thing D4 requires.

The rule had teeth immediately: the context sidebar was written to filter annotation edges out
of a neighbour list, and the fix was to ask the daemon for the edge types it wanted instead.

**Consequence:** the fallback in D4 is retired. A future move to GTK4 remains cheap (D1), but it
is no longer a scheduled decision.

---

### D38 — The overlay speaks to the daemon over a split socket, not the request/response client
**Status:** ACCEPTED

`cortex-rpc::client::Client` holds the socket while it waits for an answer. The overlay uses its
own connection instead: a reader task owns the read half and dispatches by id, so calls are
pipelined and unsolicited messages become window events.

**Why:** a search's reranked order arrives about seventeen milliseconds after its first answer,
on the same connection (D7). With a request/response client the only way to receive it is to
wait for it, which puts those seventeen milliseconds in front of the next keystroke — and the
keystroke budget for the entire round trip is eighty.

**Consequence:** the frontend needs no notion of "waiting for the better answer"; it renders
what arrives. Measured: the frontend's own share of the budget is 2 ms at the median and under
16 at the ninety-fifth percentile, against a 24 ms allowance.

---

### D39 — The thumbnailer decodes JPEG with `jpeg-decoder`, not with `image`
**Status:** ACCEPTED

`cortex-preview` depends on both. Everything except JPEG goes through the `image` crate; JPEG
goes through `jpeg-decoder` directly, with `scale()` called before `decode()`.

**Why:** `image` 0.25 decodes JPEG with zune-jpeg, which has no DCT scaling — it produces
full-resolution pixels and nothing else. D14's quality rule says a small preview must never
decode a full-resolution image, and for a forty-megapixel photograph that is the difference
between reading six hundred kilopixels and reading forty million. Without this the rule would
be a comment rather than a behaviour.

**Consequence:** a second decoder in the tree, and a test that fails if a 128 px preview ever
costs the same as a full-size one. Revisit if `image` gains a scaled JPEG path; the rest of the
crate is written so that deleting `decode_jpeg_scaled` would be the whole change.

---

### D40 — PDF and video previews use whatever the machine already has
**Status:** ACCEPTED

`pdftoppm` or `mutool` for PDFs, `ffmpeg` for a video frame, found on `PATH` at render time.
Where none is present the previewer answers with the name of the missing program and the pane
prints it.

**Why:** a PDF engine and a demuxer are two of the largest attack surfaces in desktop software,
and bundling them would mean shipping them, updating them, and owning their vulnerabilities —
for a side panel. Every desktop that shows PDF thumbnails already has one of these installed.

**Why the absence is reported rather than silently skipped:** a preview that never appears looks
like a bug in this program. "pdftoppm is not installed" is something a person can act on.

**Consequence:** a missing tool is *not* recorded as a thumbnail failure. Marking the file would
leave it unpreviewable after the tool was installed, and the marker is meant to record facts
about files rather than about the machine.

---

### D41 — M4 §1 is met, and the first measurement of it was wrong
**Status:** ACCEPTED. Supersedes an earlier version of this entry that recorded §1 as failing.

**The correction first.** This decision was originally written up as "§1 is missed: 418 ms
against a 300 ms budget, and 316 ms of it is the toolkit". That number was measured while the
same machine was compiling the workspace and running three test suites. It was measuring the
load, not the application.

Measured again on an idle machine, ten consecutive runs of the installed release binary:

| part | median |
|---|---|
| toolkit init, before `setup()` runs | 122 ms |
| the page, until interactive | 75 ms |
| **total** | **265 ms** — min 260, max 280 |

**Ten of ten runs are inside the 300 ms budget. §1 is met.**

The conditions belong in the record, because "cold start" has more than one meaning:

- **Idle machine, binary in the page cache: 265 ms.** This is normal operating conditions and
  it is what the gate is measured against.
- **Immediately after a fresh build, cold page cache: 551 ms.** Every application on the machine
  pays this the first time it is run after being written; it is a property of the disk.
- **While the machine compiles: 418 ms.** The original, wrong number.

**What this costs the previous conclusions.** The GTK4 question raised here is closed: the
stack is not the problem, and D37 stands without a caveat. The resident-process model was
adopted *because* of the bad number — see below.

**The resident model stays, for a different reason.** The overlay still starts hidden at login
and is summoned rather than launched, and that is now a statement about what a launcher is
rather than a mitigation for a missed budget: a thing summoned by a key should be there when
the key is pressed, and 265 ms of it is 265 ms nobody should spend. Keeping the design while
deleting its original justification is the honest outcome, and it is written here rather than
left implicit.

**The lesson, which is the part worth keeping:** a latency measurement taken on a machine doing
something else measures the something else. The same mistake bit the frontend's render test in
the same week — it failed in the parallel Playwright run and passed alone — and both are now
run deliberately quiet.

---

### D42 — On Wayland the compositor owns the key binding, so the overlay does not register one
**Status:** ACCEPTED

`tauri-plugin-global-shortcut` is registered only when the session is not Wayland. Under
Wayland the overlay logs the line to paste into the compositor's configuration and summoning
happens through `hippocampus --show`, which reaches the resident process through
`tauri-plugin-single-instance`.

**Why:** a Wayland client cannot grab a key globally — the compositor dispatches input and only
it can decide that a combination means something. The library behind the plugin falls back to
an X11 grab, which under a Wayland session with XWayland running *succeeds*, lands in XWayland,
and never fires. The overlay logged "summon with the hotkey" for a hotkey that could not work,
and the first person to press it found nothing happening and no error anywhere.

**Consequence:** a compositor binding is one line, and the log prints it in full rather than
saying "not supported". For niri:

```kdl
Ctrl+Alt+Space hotkey-overlay-title="Hippocampus" { spawn "hippocampus" "--show"; }
```

**And one thing learned the hard way:** a bare second invocation does *nothing*. Only `--show`,
`--toggle` and `--hide` move the window. Somebody who wants it types one more word; a stray
invocation — a script, a completion, a restart race — otherwise puts a search box in front of
whoever is sitting at the machine.
