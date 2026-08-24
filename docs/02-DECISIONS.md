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

---

### D43 — M6 is pulled ahead of M5, at the owner's request
**Status:** ACCEPTED

The order becomes M4 → **M6 (Human View)** → M5 (Memory) → M7 …

**Why:** the owner used the overlay and said what it is: a search engine, not a file manager.
That is exactly what M4 was specified to be — "a launcher, not a file manager" — but it means
the thing cannot yet be used for the job its name promises, and M5 makes the search cleverer
rather than making it a file manager. Asked which he wanted first, he chose the explorer.

**The warning this overrides, stated rather than skipped.** M6's own text says *"Do not start it
before M0–M5 are green"*, and gives the reason: "a file manager that cannot do the basics will
not be used, no matter how good the brain is — and the basics are more work than the brain."
That risk is unchanged by reordering. What reordering costs concretely:

- **The context sidebar cannot show salience**, because the salience engine is M5. It shows
  what the brain knows — edges, notes, episodes — and says nothing where it would otherwise say
  "why this is salient".
- **Nothing decays or is tiered**, so the cold-tier indicator has nothing to indicate yet.

Neither blocks a file manager. Both are places where M6's surface will have a hole until M5
lands, and the holes should be visible in the interface rather than filled with a plausible
number.

**Consequence for M6's Definition of Done:** §3 — one week of exclusive daily use — is unchanged
and is still the real gate. §5 — all M4 performance gates still hold in the full application —
is now the gate that keeps the reorder honest, because a file manager bolted onto a launcher is
exactly where those budgets go.

---

### D44 — The write allow-list gates agents, not the person at the keyboard
**Status:** ACCEPTED

`Policy::check_write` takes the actor. An agent is held to `agent.write_allowlist` and to the
mandatory dry run; a human and the daemon itself are not. The **deny-list applies to everybody**.

**Why:** rule R6 is titled "Agent writes are gated" and its first sentence is about agents. The
first version of the policy ignored the actor, which was harmless while the only mutating client
*was* an agent — and became absurd the moment the GUI grew a file manager: with the shipped
configuration, whose allow-list is empty, the owner could not move his own file until he had
listed the directory in a configuration file. He can already use `rm`. A gate that only stops
the person holding the key is ceremony.

**Why the deny-list still applies to everybody:** nothing in this program needs to write into
`~/.ssh`, and a file manager that can do it unasked is one bug away from doing it unasked.

**Consequence:** the red-team suite is unchanged and still passes, which is the point — it
constructs its surface with an agent actor, so it proves the gated path stayed gated. Two new
tests assert the other half, and one existing end-to-end test had its premise replaced rather
than being made to pass: it asserted the GUI was refused a write outside the allow-list, which
is no longer true and should not be.

---

### D45 — Deleting puts things in the XDG trash, and so does overwriting
**Status:** ACCEPTED. Revises D34.

D34 made destructive operations reversible by moving displaced content into
`$XDG_STATE_HOME/hippocampus/trash/`. That was right about the mechanism and wrong about the
location. Both trashing and overwriting now use the **shared XDG trash**: `~/.local/share/Trash`
with `files/`, `info/` and a `.trashinfo` per entry, or `$topdir/.Trash-$uid` for a file on
another filesystem.

**Why:** M6's goal is one week of exclusive use with no fallback to another file manager. A file
this program deletes has to appear in the same trash as a file deleted by anything else, and be
restorable by anything else — otherwise stopping using it strands everything it ever deleted.
A private trash is a data-loss trap wearing a safety feature's clothes.

**What that changes about undo:** the journal's inverse for a trashing is no longer "move this
path back" but "restore this entry from that trash", so the `.trashinfo` is removed with the
file and the pair never disagrees. The entry has to be written *before* the move — a crash in
between must leave something recoverable — but the trash chooses the name, so the journal row is
written with a placeholder and rewritten once the name exists. That is the only place in this
system where a journal row is updated rather than appended, and `Store::replace_inverse` says so.

**Consequence:** overwriting a file puts the previous version somewhere the person can find
without knowing this program exists. Ten thousand agent overwrites would fill the trash, which
is the same thing that happens when a person deletes ten thousand files, and the trash is where
that is dealt with.

---

### D46 — Names come from `/etc/passwd`, not from `getpwuid`
**Status:** ACCEPTED.

A listing resolves uid and gid to names by parsing `/etc/passwd` and `/etc/group` once per
listing, rather than by calling `getpwuid`/`getgrgid`.

**Why:** those calls go through NSS, and on a machine configured with LDAP, SSSD or NIS they can
block on the network for as long as the resolver's timeout allows. R5 says no network access by
default, and a directory listing is the wrong place to discover that a directory server is
unreachable. The listing has a latency budget; a name lookup does not get to spend it.

**What it costs:** ids that live only in a network directory do not resolve, and show as
numbers. That is the same thing that would be shown if the NSS lookup timed out — it just
happens in microseconds instead of seconds. A machine whose users are all local, which is every
machine this is being built for, loses nothing.

**Once per listing, not once per entry.** Ten thousand files must not mean ten thousand reads of
`/etc/passwd`. `Names::from_system()` is constructed at the top of `fs.list` and passed down.

**A missing or unreadable file is an empty map, never an error.** Not knowing a name is a column
showing a number. It is not a reason to fail a listing.

---

### D47 — A symlink is reported as itself, and separately as what it points at
**Status:** ACCEPTED.

`FsEntry.kind` describes the entry; `target`, `target_kind` and `broken` describe where it
leads. Metadata is read with `symlink_metadata` (or `DirEntry::metadata`, which is the same
thing on Unix) everywhere, so nothing follows a link by accident.

**Why not report the target's kind as the entry's kind:** it makes a link indistinguishable from
a copy. That is precisely the difference somebody needs before they decide to delete one of two
things that look identical, and a file manager that hides it will eventually cost somebody the
original.

**Why `target_kind` exists anyway:** a link to a directory has to open like a directory. Without
it the row renders as a file and Enter tries to preview a directory. So the frontend draws the
row from `kind` and decides what Enter does from both — which is the one place the distinction
is allowed to collapse, and it collapses in the direction that does what the person meant.

**The target is verbatim.** A relative link is shown relative, because that is what the link
says; resolving it would answer "where does this end up", which is a different question and one
whose answer changes when the directory moves.

**A broken link is listed, not hidden.** It looks exactly like a working one until somebody
tries it, so the listing is the only place it can be noticed.

---

### D48 — `format_mode` exists twice, in Rust and in TypeScript
**Status:** ACCEPTED. Narrow exception to D4.

D4 forbids data processing in the frontend. Rendering `0o100644` as `-rw-r--r--` is written
twice: `cortex_core::tools::format_mode` for the CLI and any other client, and `formatMode` in
`ui/src/row.ts` for the interface.

**Why this does not reverse D4:** the rule is about *deciding* — ranking, matching, filtering,
sorting — because a frontend that decides becomes a second implementation of the search that
disagrees with the first. Formatting decides nothing. The daemon sends the same number to every
client and each renders it; two clients cannot come to different conclusions about a file
because one of them wrote out ten characters.

**Why not send the string:** the number is the fact and the string is a rendering of it. A
client that wanted to show an octal mode, or to check a single bit, would have to parse the
string back — and the wire type would have been chosen for one client's convenience.

**How the two are kept honest:** both are tested against the same expected strings, including
setuid, setgid, sticky and the capital-letter cases, and both are asserted to be exactly ten
characters wide for every mode from 0 to 0o7777.

---

### D49 — The system clipboard belongs to the shell, not to the daemon
**Status:** ACCEPTED.

Cut and copy put files on the desktop's clipboard, and paste reads whatever is on it. The code
that does this lives in `ui/src-tauri/src/clipboard.rs` and talks to GTK directly. The daemon
knows nothing about it.

**Why not the daemon:** it is headless and must stay that way. It runs as a user service, it is
meant to be usable from a terminal over a socket, and a clipboard is a property of a *session* —
one login has one, a machine may have several, and a daemon that acquired a display connection
to serve one of them would have broken the property that makes it testable and scriptable. The
shell is the process with a Wayland connection, so the clipboard sits beside the window and the
hotkey, which are the other two things a browser cannot do.

**Why GTK rather than `wl-copy`:** `wl-copy` would be four lines. It works by forking a process
that stays alive to serve the data — a second process per copy, outliving the window, holding
paths that may have moved since — and it is a package that is not installed everywhere. The
overlay is already a resident GTK process with a Wayland connection; owning the selection is
what that connection is for.

**Four formats, because there is no standard.** `x-special/gnome-copied-files` is the only one
that carries the verb, which is what makes "cut" possible at all; `text/uri-list` is the
freedesktop standard that everything reads; `application/x-kde-cutselection` is how Dolphin
learns that a cut was meant; `text/plain` is so that pasting into a terminal gives paths. All
four are offered at once — advertising costs nothing, the receiver picks. Reading goes most
specific first, so a cut from any of the three families stays a cut.

**A plain `text/uri-list` pastes as a copy.** It carries no verb, and a mover that guessed would
be a mover that deletes somebody's originals.

**A refusal is reported.** On Wayland a client may only take the selection off the back of a
recent input event, so a copy from an unfocused window does not happen. The window keeps its own
copy of what was cut — which is what paste falls back to — and says the copy is good only
inside this window. Somebody who is not told finds out later, in another application, by pasting
the wrong thing.

---

### D50 — A drop copies
**Status:** ACCEPTED.

Files dragged into the window from another application are copied into the directory being
looked at. Never moved.

**Why not the usual rule** — move within a filesystem, copy across one, Shift to override — is
that the drop event carries neither the modifier keys nor where the source lives. Both would
have to be guessed. Guessing "copy" wrongly leaves a duplicate somebody deletes; guessing "move"
wrongly takes a file out of another application's window while its user is looking at it. Ctrl+X
and Ctrl+V are still there for a move, and they say what they mean.

**A drop onto a result list is refused rather than redirected.** A list of search results is not
a place. Copying into whichever directory was browsed last would put the file somewhere nobody
was looking.

**Dragging *out*** offers `text/uri-list` and `text/plain` from the page's `dataTransfer`.
Whether that reaches the compositor is WebKitGTK's business, not this application's: the browser
tests prove the page offers it and can prove nothing beyond that, because they run in Chromium.

---

### D51 — A start that waited for the daemon is not reported against the budget
**Status:** ACCEPTED. Extends D41.

The overlay retries the daemon's socket for up to five seconds before giving up, and a run that
had to wait records how long. Its cold-start line then reports `waited_for_daemon_ms` and
`comparable=false`, and omits the budget verdict entirely.

**Why the retry:** both user services start together at login. `After=` orders the *start jobs*,
and a `Type=simple` daemon counts as started the moment it is spawned — its socket appears a
moment later. So the overlay lost the race, aborted, and was restarted by systemd. That worked,
and it left a core dump in the journal at every login. Waiting a hundred milliseconds is the
whole fix.

**Why the number is disqualified rather than corrected:** subtracting the wait would produce a
plausible figure from a run that measured two services racing. This project has already reported
a startup number twice from a run whose conditions made it meaningless (D41), and the fix then
was to state the conditions rather than adjust the number. This is the same rule, applied by the
program to itself: a run that waited says so, and says nothing about the budget.

**What it does not change:** `scripts/measure-startup.sh` is unaffected — it stops only the
overlay, so the daemon is always already listening and no run under it ever waits. Measured
after this change: **277 ms median, ten runs of ten inside the budget.**

---

### D52 — The overlay opens at a list of places, not at `/`
**Status:** ACCEPTED.

The first screen is not a directory listing. It is four sections — Persönlich, Lesezeichen,
Laufwerke, System — built by the daemon and served as `fs.places`.

**Why:** a file manager opens at `/` when nobody decided where it should open. What that shows
first is `bin boot dev etc lib lost+found opt proc run sbin srv sys tmp usr var`: sixteen
directories, of which one is ever wanted, and everything a person actually opens is four levels
below it. The screen answers the wrong question — "what is in this directory" instead of "where
would you like to go".

**Everything technical is one row.** `/` is still there, last, called "Dateisystem", and
everything above it is still reachable through it. Nothing is hidden; it is simply not in front
of somebody looking for their Downloads.

**Why the daemon decides this and not the interface:** D4. Which mounts are volumes and which
are kernel plumbing, where Downloads actually is on a German desktop, what a drive is called —
these are judgements, and a judgement made in the frontend is a judgement made twice, because
the CLI needs it too. `cortex places` prints the same list the window draws, and a test asserts
all three surfaces agree.

**Read from files, not from D-Bus.** `/proc/mounts`, `/dev/disk/by-label`,
`~/.config/user-dirs.dirs`, `~/.config/gtk-3.0/bookmarks`, and `statvfs`. No udisks2 on this
path: the daemon promises no network access and starts in milliseconds, and a D-Bus round trip
on the way to the first screen would cost both. *Mounting* and *unmounting* do need udisks2, and
that is the next piece of work rather than this one.

**Bookmarks are the desktop's, not ours.** `~/.config/gtk-3.0/bookmarks` is what Nautilus, Nemo
and Thunar use, so a bookmark made in any of them shows up here. Nothing writes to that file
yet — this program is new on the machine and the file belongs to whatever has been managing it.

**It is not the result list, and that is deliberate.** Places are not files: you do not select
three of them, you do not cut one, and there are a dozen rather than ten thousand. Reusing the
row machinery would mean explaining, in every operation, why the thing under the cursor cannot
be copied. It has its own cursor and its own two keys, and a browser test asserts that Ctrl+C
and Delete ask the daemon for nothing while it is on screen.

**Getting back:** the breadcrumb starts with "Orte" from anywhere, and Backspace at `/` goes
there — above the root is not "nowhere", it is where everything else is reachable from.

---

### D53 — The store keeps several connections, because WAL only helps if it does
**Status:** ACCEPTED. Corrects an error present since M1.

`Store` held one `Mutex<Connection>`. It now holds a small pool — up to four for a file-backed
database, exactly one for an in-memory one.

**Why:** the pragmas ask for WAL, and the comment above them says why: *"WAL lets readers run
while a writer is active, which is the entire access pattern here: one ingest writer, many short
query readers."* That is correct and the code did the opposite of it. A single connection behind
a mutex serialises every reader behind the writer, which is precisely what WAL exists to avoid.
The comment described the intended design; nothing implemented it.

**What it cost**, from a week of the journal on a real machine: searches at up to **932 ms**
against an 80 ms budget, `extract.status` — a heartbeat — at up to **988 ms**, and
`activity.ping` at 933 ms. None of those calls is slow. They were waiting.

**Why the benchmarks never saw it:** every gate in `bench/latency` measured a store that nothing
else was touching, and the daemon is never in that state. A gate has been added that measures a
search *while a writer works*, which is the only condition that matters.

**Four, and 16 MiB of page cache each.** SQLite's page cache is per connection, so four at
16 MiB come to the 64 MiB the single connection used to have to itself. Measured: the cache size
makes no difference here at all — 5.8 ms at 64 MiB against 5.9 ms at 16 MiB, because the
operating system's page cache is doing the work — so the smaller figure costs nothing and keeps
the daemon's footprint where it was.

**An in-memory store gets exactly one.** `:memory:` is not a name that can be opened twice: a
second connection would be a second, empty database. Callers therefore serialise there exactly
as they used to everywhere, which is why the test suite is unaffected.

**Do not take two handles on one thread.** `conn()` blocks when all are out, so nesting
deadlocks at capacity — and would be wrong before then, because the second handle is a different
connection and would not see the first one's open transaction. Nothing nests today: under the
old single mutex it would have hung immediately.

---

### D54 — A hidden window runs nothing
**Status:** ACCEPTED.

The shell emits `window:visible` on every show and hide. The frontend stops its polling and all
CSS animation while the answer is false.

**Why:** hiding a GTK window does not suspend the page inside it. WebKit keeps running the
document, so an infinite animation animates forever and a `setInterval` fires forever — and the
overlay is *resident*, so "forever" means from login to logout, for a window nobody can see.

**What it cost, measured:** with `WEBKIT_DISABLE_DMABUF_RENDERER=1` set — which drops WebKit to
software compositing — the breath layer alone held **99% of a CPU core with the window hidden**.
That is the whole of "everything feels slow": a resident overlay eating a core makes every other
application on the machine slower, including itself when it finally appears.

**That variable is also gone** (see the unit file). It was set against a real WebKitGTK/NVIDIA
pairing whose symptom was glitches, with a comment claiming it "costs nothing on hardware where
it works". It cost a core. If the glitches come back on some machine, the override is documented
in the unit and the visibility rule above means the cost is at least only paid while somebody is
looking.

**`document.visibilityState` is not used**, because it does not change: the document is visible
as far as WebKit is concerned whatever the window manager did with the window around it. The
shell knows, so the shell says.

**The heartbeat poll was also wrong on its own terms.** It ran every four seconds and its comment
said it "costs nothing". It is two aggregate queries over a hundred thousand rows each, and it
existed to set the period of an animation. It now runs every eight seconds, only while the
window is on screen, and the query behind it has an index (migration 5): 26.8 ms to 4.8 ms.

---

### D55 — Reads are answered concurrently; writes keep their order
**Status:** ACCEPTED. Removes the limit noted in D53's session.

A connection's read loop used to await each request before reading the next. It now spawns a
task for any method on an allow-list in `protocol::method::is_concurrent`, bounded to eight in
flight per connection. Everything else keeps the old, sequential path.

**How this was found.** Three lines from the journal of a real session, after the CPU and store
problems had already been fixed and the owner reported that scrolling was still slow:

```text
18:39:22.084861  slow call method="search"          ms=152
18:39:22.085244  slow call method="brain.neighbors" ms=152
18:39:22.085253  slow call method="fs.read"         ms=152
```

Three different calls, the same duration to the millisecond, all finishing within 0.4 ms of each
other. Nothing there is slow. They were in a queue, and scrolling a result list is what fills
it: every row the cursor rests on asks for its neighbours and its preview, and each waits behind
the last.

**Why an allow-list rather than a deny-list.** A method nobody has thought about is serialised,
which is the safe direction. Adding `fs.move` to that list would be a claim that two moves may
interleave, and the claim should be made deliberately by whoever wants it.

**Why writes stay in order.** Their order means something: an undo undoes the previous thing, a
paste follows the plan that described it. Making them concurrent is a much larger change than
"these reads are independent", and nothing measured asks for it.

**`ConnCtx` is cloned into each task, not shared.** The only thing that mutates it is
`identify`, which is not on the list and therefore still runs on the sequential path — before
any clone could observe it.

**Tested by property, not by timing.** A test sends a large listing and then a `ping`, and
asserts the ping comes back first; a shared machine makes timing assertions into tests that fail
on Tuesdays. Two more assert that writes are still answered in order and that twenty
interleaved requests are each answered exactly once. The first was checked against the old
behaviour and fails there, which is the only way to know a regression test tests the regression.

---

### D56 — The spare rows go where the eye is going, and a scroll that changes nothing renders nothing
**Status:** ACCEPTED.

Two changes to `useVirtual`, both from the same report: "still laggy, and it is with the mouse".

**Scroll events are coalesced onto an animation frame, and the *window* is the state.** It used
to be the other way round: every scroll event set `scrollTop`, so every scroll event re-rendered
the list — including the many that move the view a few pixels and leave exactly the same rows on
screen. A wheel produces a great many of those. Now the window is compared before it is stored,
so a scroll that changes nothing visible changes no state, React never renders, and the DOM is
never touched. A browser test asserts zero DOM mutations across four sub-row scrolls.

**The overscan is asymmetric while moving: two rows behind, twelve ahead.** Symmetric overscan
spends half its budget on rows that have just been scrolled away from. Worst gap in the viewport
over twenty frames of steady scrolling, on a 650 px list of 28 px rows:

| wheel speed | six each side | two behind, twelve ahead |
|---|---|---|
| 150 px/frame | none | none |
| 200 px/frame | 28 px | **none** |
| 300 px/frame | 128 px | **none** |
| 420 px/frame | 240 px | **72 px** |
| 600 px/frame | 428 px | **260 px** |

For two more rows in the DOM — 38 against 36 — so M4 §3's bound is untouched, and a test asserts
that too. This was never lag: the browser paints the new scroll position before React can put
rows there, so a fast wheel showed a third of the list empty on every frame and filled it one
frame later. That reads as loading, which is why the report was about files loading.

**Three things about measuring this, because two of them were got wrong first.**

The window is now computed *during render* when its inputs change — viewport height, row count,
row height — rather than in an effect. An effect commits one frame with the stale window first,
and that frame is visible: a list that has just been given its height renders a short window and
corrects itself. A browser test caught it by finding twelve rows where there should have been
thirty.

The existing scroll benchmark waited a `setTimeout(0)` after dispatching a scroll. With
coalescing, that turn now completes *before* the work happens — the test would have gone on
passing while measuring nothing. It waits for the rows to change instead, which is also the
quantity a person experiences.

And the gap measurement has to warm up in the direction of travel. Measured from a standing
jump, the first frame is genuinely incomplete — a wheel does not teleport — and the maximum over
the run is that one frame. Measured that way, this change looked like it had made things four
times worse.

---

### D57 — The lead is measured, not fixed; and the DMABUF renderer was never the thing that cost a core

Two findings from one session, and they are recorded together because the second corrects the
first half of D54.

**The renderer was exonerated.** D54 removed `WEBKIT_DISABLE_DMABUF_RENDERER=1` from the service
unit and attributed 99% of a CPU core to it. The owner reports that the glitches and black areas
the variable existed to suppress are back without it — WebKitGTK 2.52.3 on the NVIDIA 595.84 open
kernel module, under Wayland — so it has been restored as a drop-in. Measured with it back, the
window hidden, over thirty seconds:

| | hidden, no override | hidden, override restored |
|---|---|---|
| overlay cgroup | 0.07% of one core | **0.10% of one core** |

The core was never the renderer's. It was the breath layer animating for a window nobody could
see, and the *other* half of D54 — `[data-visible="no"] * { animation: none }`, plus stopping the
polls — is what fixed it. D54 changed two things and credited the wrong one. Correct rendering
and a quiet CPU are not a trade: both are available, and the cost of software compositing is now
paid only while the window is actually on screen.

The measuring tool was checked before its numbers were believed, and the first version of it read
**0.0% for a process spinning a full core** — `$!` had captured a subshell, and `utime + stime`
is the process's own time, not its children's. A tool that reports zero for everything agrees
with a good result and cannot be told apart from one.

**The lead follows the scroll instead of guessing at it.** D56 moved the spare rows into the
direction of travel and left 72 px of gap at 420 px per frame, which the owner was asked about
and chose to close. A larger fixed lead would have bought it with a permanently larger DOM. The
lead is instead derived from how far the previous frame actually travelled, so the rows are
borrowed only while the view is moving that fast and handed back when it stops.

| wheel speed | six each side | two behind, twelve ahead | lead from the last frame |
|---|---|---|---|
| 150–300 px/frame | up to 128 px | none | none, 38 rows |
| 420 px/frame | 240 px | 72 px | **none, 42 rows** |
| 500 px/frame | — | — | none, 45 rows |
| 600 px/frame | 428 px | 260 px | 36 px, 46 rows — past the ceiling |
| scrollbar jump | — | — | full height, **38 rows** |
| at rest | 36 rows | 36 rows | **36 rows** |

Three rules make it safe, and each of them exists because leaving it out broke something that
was measured:

1. **A ceiling.** `MAX_LEAD_ROWS = 20` covers 560 px a frame. Without it the node count would be
   a slope with the input device's speed on the x-axis, and M4 §3 asks for a bound.
2. **A jump is not a scroll.** A frame that travels further than the viewport is a scrollbar drag
   or `scrollToIndex` landing on a match: the view goes somewhere unrelated and stops, so there
   is nothing for a lead to be continuous with. Sizing a lead from that distance spends rows on
   motion that is already over.
3. **The lead follows the slower of the last two frames.** One frame on its own can never buy
   rows. Without this, a single long step put 46 rows in the DOM and gave them back a frame
   later, and that burst of DOM mutations landed exactly where D56 had got them to zero — the
   test for "a scroll too small to change a row costs nothing" went from 0 mutations to 96.

The settle gives back the lead and **not** the direction. Resetting both looked tidier and
un-weighted the window on the frame after every wheel notch, which is the same regression again.

**And the renderer's real bill arrives when the window is on screen.** The measurement above is
the *hidden* window, which is where D54 looked. With the overlay summoned and nobody touching
it, the same process reads:

| | hidden | on screen, idle |
|---|---|---|
| overlay cgroup | 0.14% of one core | **116%**, over four consecutive samples |

Same binary, same variable, same window: the only difference is D54's
`[data-visible="no"] * { animation: none }`. That is a controlled comparison rather than an
inference, and an inventory of the frontend finds exactly one animation that runs continuously —
the breath layer, one gradient covering the window with its opacity rising and falling. Under
software compositing that is a full-window repaint sixty times a second.

The layer keeps its period and its gesture and takes them in **steps**: `steps(24, end)` under
`[data-compositing="software"]`, so the opacity changes a few times a second instead of every
frame. Not a static layer, which would be cheaper still — design system §6 makes this the only
progress indicator the application has, and a still one indicates nothing.

The page cannot see `WEBKIT_DISABLE_DMABUF_RENDERER`, so the shell reports it, in
`startup::Report` alongside the cold-start numbers and for the same reason: the half that knows
says so, rather than the other half guessing. **The effect of the step change is not measured
yet** — it needs the window on screen, and this session could not open it. The number to beat is
116%.

**What the tests are worth, checked rather than assumed.** Every one of the five rules above was
verified by breaking it and watching a named test fail: ceiling, jump guard, sustained motion,
the settle, and the settle's direction. Two tests did not survive that check and were fixed
first:

- The gap at 420 px per frame **had no test at all** — the suite asserted only 300 px, where
  D56's fixed lead already reached zero. The defect the owner was asked about was outside every
  assertion.
- "It comes back down when the scrolling stops" dispatched six scroll events while waiting, and
  those events did the settling. It passed with the settle deleted from the code. A real wheel
  sends nothing once it stops, so the test now waits frames and dispatches nothing.

And an existing bound assertion in `layout.spec.ts` began failing **only under parallel load**,
which was not flakiness to be retried: it was the 46-row transient from rule 3, visible for one
frame. Fixing the cause removed it.

---

### D58 — "Öffnen mit" is read from the desktop's own databases, and the daemon is what reads them

A file manager that keeps its own idea of which program opens a `.odt` disagrees with the
session it lives in: somebody sets their editor in GNOME's settings and this window still opens
the old one, or they set it here and nothing else notices. So nothing here is invented. The type
comes from shared-mime-info's `globs2`, the applications come from `mimeapps.list` and
`mimeinfo.cache` in the precedence the Association specification gives, and setting a default
writes to the file `xdg-mime default` writes to, one line of it, leaving comments and every other
key untouched.

**Read by the daemon, run by whichever surface has a session.** The reading is judgement —
precedence across eight configuration files, glob weighting, subclass fallback — and judgement
implemented twice drifts (D4). The running needs a session, and the daemon is headless (D49).
The seam is `Application::command_line`: the `Exec` line and its field codes are expanded in one
place, in Rust, and both `cortex apps` and the window get the same argv. `cortex apps <file>`
prints what the window shows, which is the cheap way to find out which of the two is wrong.

**It was checked against the machine rather than against the specification.** Twenty-five file
types, compared with `xdg-mime query filetype` and with `gio mime`, which is what GTK
applications actually use: 25 of 25 agree on the type, on the default, and on the order of the
list. Three of the four corrections that took came from that comparison and not from reading the
standard:

- **`NoDisplay=true` does not mean "do not offer this".** It means "not in the applications
  menu". The first version filtered those entries out, and on the machine this was written on
  the desktop's own default handler for `text/plain` is exactly such an entry — so the chooser
  offered everything *except* the application a double click opens.
- **With nothing naming a default, the first association is the default.** `xdg-mime` and `gio`
  both answer that way, so it is what opening the file actually does. Leaving the list unmarked
  would have been truthful about the configuration files and wrong about the machine.
- **`defaults.list` is still read.** It is the name `mimeapps.list` had before, packages still
  ship it, and on a machine where it is the only thing naming a default it is the answer.
- And a tie has to be broken deterministically: `*.json` is claimed by `application/json` and
  `application/schema+json` at the same weight and the same length. Deciding by the order the
  files happened to be read in makes the answer a property of the machine. The type name does
  not, and it agrees with `xdg-mime`.

**The name is consulted before the content, which is the opposite of what the indexer does.**
`cortex-extract`'s first rule is that a magic number outranks the extension, because it is
deciding whether to *read* a file and a `.txt` full of JPEG bytes must not be read as text. This
is deciding what the rest of the desktop would open the file with, and the rest of the desktop
matches the name first. Disagreeing would mean this window opens a file with a different
application than a double click in Nautilus does, which is the one thing the module exists to
prevent. The content is still the fallback when the name says nothing, and it ends in the
indexer's classifier — so a machine with no shared-mime-info installed still gets `text/markdown`
rather than `some bytes`.

**An agent may never set a default, and the write allow-list is not why.** The allow-list is
about which paths may be written (D44); this call writes no path an agent would name. What it
changes is what happens the next time a *person* opens a file, in this window and in every other
application. An agent that could point `text/plain` at a program of its choosing would have
arranged for that program to be run later, by somebody else, with no further call to this
surface for anything to refuse — one innocuous audit line, and then nothing. The refusal is
unconditional for an actor that is an agent, and the red-team suite asserts both halves: refused
for an agent with nothing written, allowed for a person.

It writes no undo entry, and that is a limit rather than an oversight: the undo journal describes
filesystem operations and this is a configuration key. What the previous default was goes into
the audit line, which is what somebody putting it back needs.

**A launched application gets a scope of its own, or the next restart closes it.** A child of a
systemd user service lives in that service's cgroup, and the default kill mode takes the cgroup
down with the service — so an editor opened from the overlay was closed again by the next
`systemctl --user restart hippocampus-overlay`: an update, a crash, a login. Measured rather than
reasoned about: a child spawned the ordinary way was killed with its service **even after
`setsid`**, and one launched with `systemd-run --user --scope` survived the same stop, in a
cgroup of its own. Launches go through that now — which is what GIO does, for this reason — and
where there is no `systemd-run` the plain spawn is the only answer there is. This applied to
`open_path` before "Öffnen mit" existed; it was found while measuring something else, which is
the second time in three sessions that watching the cgroup rather than the code found the defect.

**And the chooser's cursor is derived during render, not reset in an effect.** The list under it
can be replaced — "alle anzeigen" is a longer list — and an effect commits one frame with the old
position still highlighted. D56 paid for that lesson once as a visual glitch; here it is worse,
because Enter in that frame opens an application nobody pointed at. A browser test caught it by
failing seven runs in ten under parallel load, with the rows already grown to three and the
highlight still on the second row of the old list. The cursor now carries the list it is a
position *in*, and a position in a list that is no longer on screen reads as zero.

---

### D59 — The heartbeat runs only while there is work, because a running animation costs a core whatever it animates

D57 said the breath layer was expensive *because of software compositing*, and made it stepped so
that the opacity changes a few times a second instead of sixty. The prediction was an order of
magnitude. **Measured on the overlay, window on screen and nobody touching it: 116% before,
110% after.** The remedy bought nothing, so the explanation behind it was wrong, and six variants
were measured before anything was believed:

| what was changed | cost, window visible and idle |
|---|---|
| D57's stepped timing function (`steps(24, end)`) | **110%** of one core |
| the smooth animation it replaced | 116% |
| the same, with `backdrop-filter` removed entirely | 111% |
| the same, with WebKit's **GPU** renderer instead of software compositing | 111% |
| a flat wash instead of the radial gradient | 104% |
| a **three-pixel strip** instead of the full-window gradient | 71% |
| opacity set from a timer twice a second, no CSS animation | 38% |
| **no animation at all** | **0.30%** |

Read down that column: it is not the renderer, not the backdrop filter, not the gradient, and
not the size of the thing being animated. **What costs the core is the window being recomposited
every frame while any animation is running**, and the only lever that moves the number is
whether one is running at all. A three-pixel strip costing 71% is what settles it — 0.2% of the
pixels for 64% of the cost.

So the breath now runs while the daemon has something to index and is still when it does not.
Measured on the shipped build, four consecutive samples with an empty extract queue: **4.8–5.1%
of one core** visible and idle, against 111% before, and 0.22% hidden. The remaining five per
cent has not been chased; the twenty-fold difference is the point, and the next candidate if
somebody does chase it is the caret blinking in the search field, which is an animation like any
other. That is also the truer reading of §6: the layer exists to make background work legible,
and when there is no background work, a still layer is what that means. It still breathes during
an ingest, which is the case §6 was written for, and the cost then is a core — worth knowing, and
worth measuring again if a long ingest ever feels slow.

Three consequences worth writing down:

- **`WEBKIT_DISABLE_DMABUF_RENDERER=1` is exonerated a second time, and for a different reason
  than in D57.** It costs nothing at all: 111% with it, 111% without. It buys correct rendering
  on this driver and that is the whole of its effect. D54 charged it with a core, D57 charged it
  with the visible cost, and both were wrong.
- **The `software_compositing` field survives with no consumer in the page.** The stepped CSS it
  was added for is gone. It stays in the startup report because it is logged, and "which
  renderer is this process using" was a question this session could only answer by experiment.
- **A resident window is a different animal from a page.** An infinite animation in a document
  somebody closes after a minute is ordinary; in a window that is open from login to logout it
  is a core, for ever. Design system §8 gets a rule about it.

The measuring method is worth keeping too: the cgroup, not `ps`, and the tool checked against a
process known to spin one core before any of its numbers were believed — it read 0.0% for that
spinner the first time, for the same reason it did in D57's session.

---

### D60 — Drives: the listing is files, the mounting is `udisksctl`, and neither is an agent's business

M6 asks for mounts via udisks2. Two halves, and they are not alike.

**Finding the drives needs no privileges and no D-Bus.** `/sys/class/block` says what exists and
how big it is, the disk above a partition says whether it can be unplugged, and udev's own
database — `/run/udev/data/b<major>:<minor>`, world-readable, one `E:KEY=value` per line — says
what filesystem is on it and what it is called. That is the same shape as everything else in
`places.rs` (D52), and it keeps the first screen off a D-Bus round trip and out of a dependency
on a GLib main loop inside a headless daemon.

**Mounting is the one thing here that is not a file**, and it needs privileges this daemon does
not have and must not have. `udisksctl` already knows how to ask: it talks to the system service,
which asks polkit, which says yes for a removable drive belonging to the person at the keyboard.
Shelling out to it rather than linking a D-Bus crate keeps the dependency list and the startup
time where they are, and it is the program every desktop already uses. **`--no-user-interaction`
on every call, without exception** — a daemon waiting on an authentication dialog nobody can see
is a daemon that has stopped answering.

**An unmounted volume's `path` is a device node.** There is no directory yet, so `Place::path` is
`/dev/sdb1` and `Place::volume.mounted` is false. Opening such a row is two steps — mount, then
go to *the mount point udisks2 answers with*, never to a path guessed from the label, because
udisks2 chooses where it lands and a guess sends the window somewhere empty.

**What is not offered, and why it is an allow-list.** A filesystem type nobody has thought about
is not offered rather than offered and failing; the EFI system partition, the Microsoft reserved
partition and a Windows recovery partition are machinery rather than places, and mounting an ESP
by accident is a way to damage a boot; the kernel's own loop and zram devices are not drives, and
on the machine this was written on six snap loop devices each carry a squashfs that udev reports
as a filesystem. What the rules *cannot* do is written down too: on an MBR disk a Windows system
partition and a Windows data partition are both type `0x7`, so a 579 MB volume called
"System-reserviert" is offered like any other. Nautilus offers it as well, and inventing a rule
out of a German label would be worse than one row too many.

**`power-off` on a disk that cannot be unplugged is refused, and the refusal is a tested
decision rather than a line inside the doing.** It is one argument away from the ordinary case
and it takes a disk off the bus while a running system is using it. Whether a disk can be
unplugged is read from the kernel and never inferred from the mount point. The first version had
the check inside `Surface::unmount`, where deleting it broke nothing that any test noticed —
`plan_unmount` exists so that deleting it fails a test with a name.

**No agent may mount, unmount or eject anything**, and — as with `apps.set_default` (D58) — the
allow-list is not why. No path an allow-list could name is written. What changes is the state of
somebody's hardware: mounting exposes a filesystem the machine had deliberately not exposed, and
ejecting one that a copy is running to leaves a half-written file on a stick that has already
been pulled out. The refusal comes before anything looks at the machine, so the call cannot be
used to find out what is plugged in, and neither method is on the MCP bridge at all.

That refusal needed a new kind. `Refusal::OutsideAllowlist` was reused at first and it told the
caller to *add `/dev/sdb1` to `agent.write_allowlist`* — advice that would not have worked,
because no configuration permits this. `Refusal::NotForAgents` says what is true instead, and it
corrects the same misdirection in D58's `apps.set_default`.

**Three things this got wrong, found by somebody using it rather than by a test.** The report was
"selecting a drive that is not the stick makes everything buggy and it hangs", and a screenshot of
the window settled what the logs could not:

- **`[object Object]`.** A daemon error crosses into JavaScript as an object — `{ code, message }`,
  deliberately, so the page can branch on a refusal's code — and the places screen ran `String(e)`
  over one. There was already a function in the frontend that handled this correctly; writing a
  second one next to it *was* the bug, and the two are one function now.
- **A refused mount left no audit entry.** Both new methods returned hardware failures without
  recording them, so `cortex audit` showed the successful mount and nothing else, and the only
  evidence a refusal had happened was the screenshot. R8 says allowed *and* refused.
- **The message was noise even once it was a string.** `udisksctl` prefixes every failure with a
  complaint that it cannot open `/dev/tty` to build a textual authentication agent — true, since
  a daemon has no terminal, and not the answer to anything. `NotAuthorized` in either of its two
  spellings is now its own error that says what is actually true: an internal disk needs a
  permission this window never asks for, and names the command that would work.

**Verified against hardware rather than only against a fixture.** The machine this was written on
had a stick plugged in and unmounted, three unmounted Windows partitions, an EFI partition and
six loop devices. The scan found the stick and the Windows volumes and none of the machinery;
`cortex mount` put the stick at `/run/media/<user>/ESD-USB` with no authentication prompt, the
places list then showed it with a capacity, `cortex unmount` put it back; ejecting an internal
disk was refused, and so was every one of the four calls made as an agent. The unit tests
reproduce each of those cases against a `/sys` and a `/run/udev` that were made for the purpose,
because a suite that reads the real ones asserts what happens to be plugged in today.

---

### D61 — A password is asked for by asking twice, and the second time is a client's decision

D60 said `--no-user-interaction` on every call, without exception, because a daemon waiting on a
dialog nobody can see has stopped answering. That rule left internal disks unmountable: polkit
wants `auth_admin_keep` for `filesystem-mount-system`, and the daemon had promised never to give
anybody the chance to type a password. The owner asked for the prompt, and this is how it works.

**Two requests rather than a retry inside one.** The ordinary `fs.mount` still never waits: it
refuses with a code of its own, `NEEDS_AUTHORISATION` (1004), which means "this would work if
somebody said so". A client that has a person in front of it — the window, where somebody just
pressed Enter and is therefore still there — sends a second, explicit `fs.mount` with
`interactive: true`, and *that* one lets polkit ask. The CLI spells the same thing `--ask`.

Three things make it safe rather than a hole in D60's rule:

- **It is never reached on its own.** `Udisks::asking` is a separate constructor, not a flag with
  a default, so the path that reaches out from a background stage cannot fall into it.
- **It has a deadline.** 120 seconds, then the child is killed and the answer is "nobody
  answered the authentication request" — long enough to find a password, short enough that a
  dialog in front of an empty chair does not leave a process waiting until the machine is
  switched off. A test proves it gives up.
- **The refusal travels as a number, not as prose.** A page that recognised this case by matching
  English text would break the first time the wording improved.

**And the diagnosis that led here was wrong twice, which is worth more than the fix.** The
symptom was that nothing ever asked for a password. `ps` showed no polkit agent — no
`polkit-gnome`, no `mate-polkit`, nothing — so the conclusion was "this session has no agent",
and a package was installed and an autostart line added to the compositor's configuration on the
strength of it. The new agent then refused to start: *an authentication agent already exists for
the given subject*. It was inside the desktop shell all along, as a thread of a Quickshell
process, invisible to a search for process names. **A negative from `ps` is not evidence of
absence; polkit was the thing to ask, and it answered in one line.** The package and the
configuration change were reverted.

What polkit actually wants here, for the record, since it is the whole reason the stick behaved
differently from the disks:

| action | active local session |
|---|---|
| `org.freedesktop.udisks2.filesystem-mount` (removable) | **yes** — no authentication at all |
| `org.freedesktop.udisks2.filesystem-mount-system` (internal) | `auth_admin_keep` — a password, remembered afterwards |

Unmounting needs neither: a filesystem you mounted is yours to unmount, which a test on the real
disk confirmed rather than assumed.

---

### D62 — M8 is pulled ahead of the rest of M5, and M5 ships without its language model
**Status:** ACCEPTED (owner's request, 2026-08-23)

The owner asked for the Agent View — M8 — as a finished, showable thing. M8 draws concept
nodes, salience, tiers and consolidation events, and before this session the index had **24
edges, a salience of 0.0 on all 108 000 nodes and every node in `stm`**. A three-dimensional
view of that is a directory tree with extra steps. So M5 came first, and it came in halves.

**What M5 now has:** the salience engine with explainable components, decay, tiering with
one-step demotion and immune pins, co-access edge formation from the episode log, and a
consolidation pass with a human-readable report. `cortex consolidate`, `cortex report`.

**What M5 still does not have, and says so in every report it prints:** step 2 of the
consolidation loop — one semantic concept node per cluster, which needs an embedding model and
a summariser — and step 5, promotion into the sync outbox, which is M7's job anyway.

The report lists both under "not done in this pass". A system that quietly omits half of what
its own specification promised is how a project ends up believing it is further along than it
is; printing the omission in the same paragraph as the achievement costs three lines.

**Revisit when:** the embedding model is loaded and `embeddings` stops being 0, at which point
the abstraction step becomes implementable and this entry is superseded rather than amended.

---

### D63 — Every salience component is bounded before it is weighted, and every count decays
**Status:** ACCEPTED

The formula in `01-BUILD-PROMPT.md` §M5 is a weighted sum of seven signals. Implemented
literally it has two failure modes, and the second one was found by a test rather than by
reasoning.

**Bounded components.** Each signal is squashed into `[0, 1]` before its weight is applied, so
a weight *is* the most that signal can ever contribute. Without this the weights are not
comparable with each other — `w1` would mean "per log-access" and `w5` "once" — and a file
opened ten thousand times by a build script outranks everything a person ever cared about.

**Decaying counts.** `w1·log(1+access_count)` never decreases. A file opened once in 2019 keeps
a floor of salience for ever, and in the first thirty-day simulation **nothing ever reached the
Cold tier**: every node that had ever been touched sat above the threshold permanently. The fix
is that `access_count` and `edit_count` are *effective* counts — each episode is a reinforcement
of size 1 and the running total decays between episodes by the specification's own law,
`salience(t) = base·e^(−λ·Δt) + Σ(reinforcements)`. Fifty accesses two years ago now count for
less than three accesses this week, which is what "how much is this being used" has to mean in
a memory that forgets.

The law was already in the specification. It was written down as a separate mechanism from the
sum, and the two were never connected. They are the same mechanism.

**Revisit when:** somebody wants a signal that genuinely should not decay. Pins are that case
and are handled by exemption, not by weight.

---

### D64 — The salience log keeps what changed, not every computation
**Status:** ACCEPTED — deviates from `01-BUILD-PROMPT.md` §M5 deliberately

The milestone says every computation is logged to `salience_log`. Taken literally that is one
row per node per pass: a hundred thousand rows a day on this machine, forty million a year, for
a table that is only ever read one row at a time by `brain.explain`.

What is stored instead: a row for every node whose score moved by more than 0.01 **or** whose
tier changed, and only the newest three per node. A settled index therefore writes almost
nothing, and the question the table exists to answer — "why is this ranked here" — is still
answerable for anything that has moved recently.

**Revisit when:** somebody needs to audit a score that has not changed in months. The honest
answer today is to recompute it, which is deterministic from the inputs.

---

### D65 — Consolidation replays the whole episode log every pass
**Status:** ACCEPTED

Deriving associations only from episodes since the last pass makes the resulting graph depend on
how often consolidation happened to run — an index consolidated hourly and one consolidated
weekly would disagree about what is related to what, from the same history. Definition-of-Done
item 3 asks for idempotence, and this is the cheap way to get it: replay everything, upsert the
edges, and the edge set is a pure function of the log. Run it twice, get the same graph.

The cost has a ceiling (`MAX_REPLAY_EPISODES`, 500 000 newest) and the pass is 240 ms over
31 000 episodes and 3 000 nodes in the simulator. If that ever becomes the bottleneck the fix is
an incremental derivation with a stored fold, not a smaller window.

**Revisit when:** a real index passes half a million episodes.

---

### D66 — The Agent View is its own app, and its layout is deterministic
**Status:** ACCEPTED

`ui-agent/` is a second Vite application rather than a route inside `ui/`. The file manager's
bundle is on a latency budget that a WebGL renderer and a graph layout have no business being
inside, and the two are opened for different reasons by different people.

**No force-directed layout**, although §M8 suggests reusing a force-graph component. A force
layout settles into a different arrangement on every reload, which means two screenshots of the
same brain cannot be compared and a node cannot be found twice in the same place. Both layouts
here are pure functions of the snapshot: a containment tree, and concentric shells by tier. The
transition between them is the animation in which forgetting is actually visible — a demoted
node moves outward — and it only reads as movement because the node was somewhere stable before.

Rendering: one `InstancedMesh` for all nodes and one `LineSegments` buffer for all edges. Per-node
meshes would be thousands of draw calls, which is the thing that makes such views unusable.

**Revisit when:** concept nodes exist (D62) and have no place in a containment tree — they will
need a layout of their own, probably near the nodes they abstract.

---

### D67 — A snapshot of a real index is treated like the index
**Status:** ACCEPTED

`brain.graph` answers with paths. A `graph.json` taken from somebody's own daemon is therefore
as sensitive as the index it came from, and `cortex graph` says so on the terminal when it
writes one. It is in `.gitignore`, it is never committed, and the demo that ships in this public
repository is generated by `brain-sim` from a seed — a synthetic workspace whose every salience,
tier and edge was nonetheless computed by the same engine the daemon runs (R9).

**Revisit when:** a sharing feature is wanted, at which point the snapshot needs a redaction
mode and this entry is superseded.

---

### D68 — Concepts are clustered from co-access and named lexically, not by a model
**Status:** ACCEPTED — completes the half of M5 that D62 recorded as missing

M5 step 2 asks for "one semantic node per cluster" and says an LLM is appropriate. This
implements the step and does not use one.

**How it works.** Label propagation over the `co_access` edges, run to a fixed point in a
deterministic node order, gives the communities. A community of at least three and at most sixty
files becomes a concept. Its name is the terms its members' paths have in common **and nothing
else does**: frequent inside the cluster, rare outside it. The concept is a real node with
`derived_from` edges to its members, its salience is the mean of its three most salient members,
and its membership is rewritten on every pass rather than merged.

**Why not a model.** A summariser would produce better names. It would also make the abstraction
step non-deterministic, unrunnable offline, untestable, and impossible to reproduce from a seed —
four properties this system does not trade for nicer strings. `Cluster` carries the members and
the ranked terms, so adding a summariser later is a function from a cluster to a better label and
nothing else changes.

**The label is the identity**, which is what lets a cluster that gains a file update its concept
instead of growing a rival beside it — and which forced the rule that matters most here: **a
cluster with no term the others lack is not named at all.** Two earlier attempts appended a
disambiguator, first the member count (`kettle (5)`, three times over) and then the smallest
member id (`kettle #1230`, unique, stable, and meaningless). A node that looks like knowledge and
is not is worse than an absent node.

**Terms from the index root are excluded.** The longest common prefix of every indexed path is,
on a personal machine, that person's home directory — so the first concept the real index
produced was called `axon · legi · projects`. It is now called `axon`. The prefix is found from
`MIN(path)` and `MAX(path)`, which is two rows rather than a scan.

**Revisit when:** an embedding model is resident anyway and `similar_to` edges exist. Clustering
over both edge types would find subjects that co-access alone cannot, and the same labelling
would still apply.

---

### D69 — Two SQL shapes that are wrong quietly
**Status:** ACCEPTED — recorded because both produced no error

Neither of these is a design decision so much as a pair of traps that cost a debugging session
each, and both fail *silently*, which is why they are in the log rather than only in a commit.

**`json_extract` on a column that is not JSON.** `upsert_concept` looked its concept up with
`WHERE kind = 'concept' AND json_extract(mime, '$') IS NULL AND mime = ?1`. The clause was meant
to say "concepts have no path". What it does is ask SQLite to parse the concept's name as JSON,
which fails with `malformed JSON` for every name that is not a JSON document — that is, for
every name. It had never been called with a real label before the abstraction step existed.
Migration 6 gives concepts a `label` column of their own; a MIME type is a MIME type.

**`NOT IN` over an empty list.** Pruning concepts whose cluster had dissolved rendered the empty
case as `id NOT IN (SELECT NULL)`. In SQL's three-valued logic that predicate is NULL for every
row and therefore true for none, so the delete removed nothing **exactly when there was most to
remove**. The empty case is now a separate statement.

**The pattern both share:** a condition that is wrong returns no rows rather than an error, and a
delete that removes nothing looks identical to a delete that had nothing to remove.

---

### D70 — Two health numbers and one deny-list pattern, taken from a knowledge base in daily use
**Status:** ACCEPTED

A separate, file-based knowledge base — human-curated, a year old, 1 147 notes of which 415 are
curated canon, in daily use by several agents — was examined for what a memory actually gets
wrong over time. Three of its findings changed code here.

**Copies outrank originals.** Its search is `grep -r` over the whole tree, backups included, so a
query returns the same note from three generations of backup *before* the canonical one, in
filesystem order. `noise_score` now penalises `backup`, `backups`, `snapshot`, `snapshots`,
`old` and `archive` as path segments. A copy is not merely redundant: it answers **instead of**
the original, it is the stale one, and there are several of it.

**Orphans are the most useful number a memory can print about itself.** Its health file counts
notes with no link in either direction: 75 of 415, under a rule that requires at least one link.
Manual linking does not survive contact with volume. The consolidation report now counts the
same thing — restricted to nodes somebody has actually touched, because without that restriction
a fresh index of a home directory reports a hundred thousand orphans, which is true, useless and
indistinguishable from a broken graph. On the index here it is 1 468.

**Byte-identical twins are worth counting separately.** 24 408 of 108 342 nodes here share a
content hash with another node — 22 %. The embedding table is already keyed by hash so they cost
nothing to store, but they cost something to *read*, which is the point.

**Revisit when:** the deny-list becomes configurable per zone. A directory called `archive` is
debris in one project and the subject matter in another.

---

### D71 — What a year of that system's use says about this one
**Status:** ACCEPTED — evidence, not a change

The same knowledge base has a weekly prune report. **Seven consecutive weeks produced identical
files saying "none"**, while its health file simultaneously listed 75 orphans and 4 stubs. The
cause is in the criteria:

```python
status == "draft"  AND  updated < 14 days ago  AND  len(body) < 800  AND  no links
```

Four conditions with AND — and its own intake gate *requires* at least one link and sets
`status: stable`. **The forgetting criteria are made unreachable by the intake rules of the same
system.** Even when it does find something, the report changes nothing: it is explicitly
report-only, and in seven weeks nobody acted on it.

That is not negligence. It is the correct response to a store that is **authoritative**: throwing
something away is irreversible, so nobody throws anything away, so it grows monotonically. This
is the accumulation trap of §1 of the paper, observed rather than argued, in a system that is
better curated than most.

**Why this matters here:** it is the strongest available evidence that the paper's central claim
is the load-bearing one. Not the scoring function, not the tiering thresholds — the fact that
**the index is a cache**. Everything else follows: reconstructible, therefore safe to demote,
therefore able to rank at all.

**Revisit when:** somebody demonstrates a memory that both is authoritative and forgets. We would
want to see how they made deletion reversible.

---

### D72 — Thresholds are checked against reachable maxima, not only against cases
**Status:** ACCEPTED — a latent bug, found by reading somebody else's decision log

A second-brain daemon in daily use records this failure: its demotion threshold was 0.40 and the
maximum score a note without access, focus or pin can reach is exactly its recency weight, also
0.40. **Every unused note sat on the demotion edge by construction.** Its short memory ran at one
note out of eighty and 150 of 30 000 tokens. Nothing miscalculated; the formula was right; no
unit test of the formula could fail. Their sentence for it: *a scoring system can compute
flawlessly and still be functionally dead.*

The same coupling was latent here, in the other direction. A file merely **opened** once, this
instant, with no edit, no edges and no active project, reaches

```
access  ln(2)/ln(51) · 1.0 = 0.176
recency          1.0  · 1.5 = 1.500
                             ------
                              1.676
```

against a Working threshold of **1.60**. A clearance of 0.076, four per cent. Lower `w_recency`
to 1.4 for any reason and **nothing could ever enter the Working Set again** — and every test
here would still pass, because every test used a node with several signals at once.

**Decision:** `TierPolicy::working` 1.60 → **1.15**, and — the part that matters —
`single_touch_ceiling()` is now a function, and two tests hold the thresholds up against it:

- `every_threshold_clears_the_score_that_has_to_cross_it`: `working ≤ ceiling · 0.75`, the tiers
  are ordered, the ordinary ceiling is well past Working, and a pin beats everything else summed.
- `one_fresh_touch_is_enough_to_enter_the_working_set`: the behavioural half.

Fixing the number would have fixed the bug. The invariant stops it coming back, whoever next
edits the weights.

**Revisit when:** the weights become user-configurable. Then the invariant has to run at config
load and refuse the configuration, not only in the test suite.

---

### D73 — Derived data may collapse only when the collapse is believable
**Status:** ACCEPTED

The same daemon lost 939 of its ~4 900 semantic edges twice in one day. Both times the notes
were fine — the edge cache is derived, and a rebuild that produced nothing was allowed to replace
everything with nothing. Three causes stacked: a vector store not yet ready returned `None` for
every note without raising, so the rebuild produced an empty list and committed it; a concurrent
delete raised `FileNotFoundError` in the middle of an iteration and tore down the whole cycle;
and one `try/except` wrapped every phase, so one broken phase cost all the later ones. The second
loss had a different cause and a worse one: `sqlite3.OperationalError` **inherits from**
`DatabaseError`, so a handler that treated `DatabaseError` as corruption deleted the database
file whenever a lock timed out — on a 5 400-rpm disk under load, regularly.

Their rules, adopted here:

- **Stale is better than empty.** A rebuild may replace the old set only if it is credible.
- **A lock is not damage.** Only what the database explicitly reports as corrupt counts as
  corrupt; anything else is propagated.
- **Set aside, never delete.** From a file moved aside, derived data can still be salvaged.

The same shape was live in this code: `write_concepts` deletes every concept the current pass did
not propose, so one bad pass — a truncated replay, an empty edge set — would have removed the
whole semantic layer. It now refuses to replace the concept set when a pass proposes fewer than
`CONCEPT_KEEP_RATIO` (0.5) of what exists, logs the refusal, and reports having pruned nothing.
Tests cover both directions: a collapse is held, and a pass that keeps most of them may still
remove the stray.

Hippocampus has no delete-on-error path to fix — `rebuild` is an explicit command — but the rule
is written down here so that nobody adds one.

**Revisit when:** a legitimate mass-deletion case appears, e.g. a zone being removed. Then the
guard needs an explicit `allow_collapse` rather than a lower ratio.

---

### D74 — An agent's memory is the same memory, with a scope
**Status:** ACCEPTED

The owner asked, on 2026-08-23, for the agent area to be built out and for Hippocampus to be
*the* memory an agent works from. The obvious shape — a second store, agent-flavoured, next to
the file index — was considered and rejected before any code was written, for three reasons that
are all the same reason.

**One: two memories means two rankings, and the second one is always worse.** Salience, decay,
tiering, co-access, consolidation and `explain` took M5 and most of M2 to get right, and they
were measured. A parallel agent store starts with none of that and acquires a bad copy of it over
six months.

**Two: the interesting queries cross the boundary.** "What do I know about this file" has to
return the file's chunks *and* the note an agent wrote about it *and* the failure it recorded
while working on it, ranked against each other. Two stores means merging two ranked lists whose
scores are not comparable, which is exactly the mistake RRF exists to avoid and which nothing
downstream can repair.

**Three: R10.** The file index is a cache because everything in it is derivable. A second store
holding the only copy of what an agent learned would not be, so it would need its own durability
story, its own backup, its own rebuild. There is already one: an assertion goes into `episodes`
and everything else is a projection of it (D35). Reusing it costs a migration; not reusing it
costs a second set of invariants.

So a memory is a node with `kind = 'memory'`, no path, its own text, and a scope. The scope is
the only genuinely new idea, and it is there because a shared pool without one fails in a
specific way: one agent's wrong guess becomes every agent's premise, silently. Writes default to
`agent:<name>`, `global` requires an explicit argument, and reads never cross into another
agent's private scope.

**What this costs, stated plainly.** The `nodes` table now holds rows that are not on the
filesystem and cannot be re-derived from it, so `rebuild` must replay assertions to restore them —
which it already does for annotations, but the memory layer has more columns to lose and losing
one silently is the failure mode. Definition of Done §2 exists for that and asserts a
byte-identical dump rather than a successful exit code.

**Revisit when:** memories outnumber file chunks, or a memory query's latency stops being
dominated by the file arm. Either would mean the memory layer has become the main workload and
deserves its own physical layout rather than a shared table.

---

### D75 — usearch stays; Qdrant is refused
**Status:** ACCEPTED

Asked directly whether the vector database should be Qdrant. The answer is no, and the reasoning
is worth keeping because the question will come back.

| | usearch, as built | Qdrant |
|---|---|---|
| process model | a library in the daemon | a separate server, HTTP or gRPC |
| R5 (no network by default) | satisfied by construction | needs a loopback exception and a story for why that is not network |
| M9 (a stranger installs in ten minutes) | one binary | a binary, a service, a port, a version to keep in step |
| measured ANN cost | **1.01 ms P95 over 122 137 vectors** against a 40 ms budget | unmeasured here, and it starts with a serialisation hop |
| failure mode | a file that can be deleted and rebuilt from the table in 152 ms | an external service that can be down while the daemon is up |
| filtered search | in SQL, against a store that already holds every attribute | in a second query language over a second copy of the attributes |

The last row is the one that decides it. Qdrant's real advantage over an embedded HNSW is
payload filtering at scale — and this system already holds every attribute in SQLite next to the
data it filters on, so buying that advantage would mean duplicating the attributes into the
vector store and keeping two copies honest. That is a synchronisation problem in exchange for a
latency win over a budget that is already met by a factor of forty.

**What is actually missing is not a different engine, it is a second collection.** A memory
recall against one index shared with 122 000 file chunks competes for its twenty slots and loses,
whatever the engine. M11 §6 instantiates the existing `VectorIndex` a second time over memory
chunks. Same type, same file format, same rebuild path, no new dependency.

**Revisit when:** one of three things is true — a single collection passes ten million vectors
and the HNSW build time becomes the bottleneck; filtered search needs an attribute the store does
not hold; or the daemon has to serve more than one machine, at which point a server process is
the point rather than the cost.

---

### D76 — Retrieval without use is evidence against a memory
**Status:** ACCEPTED

Every salience model in this project so far rewards being touched: access count, recency, degree.
That is right for files, because a human opening a file is a human finding it useful. It is
wrong for an agent's memory, because a memory retrieved into a context pack was not chosen by
anybody — it was chosen by the ranking that is being evaluated. Rewarding it for having been
retrieved closes the loop: whatever ranked highly first goes on ranking highly, and the system
has no way to find out it was wrong.

So `brain.outcome` reports back which items of a pack helped and which were noise, and
`usefulness` enters salience as a component with its own sign:

- reported **helpful** → usefulness rises, weighted by who said so;
- reported **noise** → usefulness falls;
- **retrieved repeatedly and never reported at all** → usefulness falls slowly. This is the
  counter-intuitive one and it is the one that makes the signal work. A memory that keeps
  arriving in context and never earns a mention is costing tokens for nothing.

Three guards, because a feedback signal that can be gamed will be:

1. **The reporter is recorded, and weighted.** `human_confirmed > agent_reported > unobserved`.
   An agent reporting on its own success is an unreliable narrator; it is not ignored, it is
   discounted, and `brain.explain` prints which of the three moved the number.

   **A guard against double-counting silenced this, and only a live run found it.** The first
   implementation refused any second report on a pack item, which correctly stops a client
   retrying a failed call from doubling its own vote — and also stopped a human from correcting
   an agent, which is the exact opposite of the weighting this decision is about. They are two
   properties and they need two rules: the same *class* of reporter may not report twice, and a
   human may overrule an agent. A correction reverses the agent's weighted delta and its count
   before applying the human's, so the human's verdict is what stands whether or not they agree
   — and the Inspector does not go on showing a "helped" the human has already contradicted.
2. **A negative report is not a delete.** It lowers a component. Forgetting stays with the decay
   engine, so one bad session cannot erase a year, and a pin still beats everything.
3. **An outcome is an episode, so what a memory has earned survives a rebuild.** This was
   nearly missed. `usefulness` is derived, everything derived has to be reconstructible from
   the log or R10 is a slogan, and the first implementation wrote the report's node ids into
   the episode — which a rebuild renumbers. An agent's entire record of what has been useful
   would have reset the first time somebody ran `cortex rebuild`, and nothing would have
   failed. Reports name their memories by what they say, like every other assertion here, and
   a human's correction records what it took back so that the replay takes it back too.
4. **`helped` and `noise` are counted separately, not netted.** A memory that is strongly useful
   in one context and noise in another is a memory that needs splitting, and netting to zero
   hides that. The Inspector shows both counts.

**Revisit when:** enough outcome reports exist to measure whether they predict anything. If a
memory's usefulness score does not correlate with a human calling it useful, the signal is
decoration and should be removed rather than tuned.

---

### D77 — A context pack, not a result list
**Status:** ACCEPTED

`brain.search` returns rows. An agent does not have rows, it has a context window, and every
agent built on this surface would otherwise write the same four hundred lines: fetch k results,
drop the near-duplicates, decide what fits, format it, and remember where each piece came from.
Done in the agent that is done badly — it has neither the scores nor the embeddings — done in
the daemon it is done once, with both.

`brain.context(task, budget_tokens)` returns a budgeted, deduplicated, attributed block.
Four properties, each of which is there because omitting it is a known failure:

- **The budget is in tokens and it is measured.** Characters divided by four is wrong by up to
  40 % on code and on German, and the direction of the error decides between wasting money and
  truncating a system prompt mid-structure.
- **Preferences are unconditional.** They are tiny and their entire value is being present. If
  they do not fit, the call reports that the budget is too small rather than dropping them
  quietly, because an agent that silently loses "never run this without `--dry-run`" is worse
  than one that errors.
- **MMR before truncation.** Restatement is the specific failure mode of embedding retrieval over
  a memory store, because people and agents restate. Three phrasings of one fact cost three slots
  and are worth one.
- **Every item carries its basis.** Which arm found it, what it scored, who wrote it and when.
  A memory an agent cannot attribute is a memory it cannot argue with, and an agent that cannot
  argue with its memory will follow it off a cliff.

The pack has an id, and that id is what `brain.outcome` reports against — which is the only
reason the pack is a stored object rather than a formatted string.

**Revisit when:** an agent framework arrives with its own context assembly that is better than
this one. Then the pack becomes optional and the arms are exposed individually.

---

### D78 — The human can read and delete what an agent believes
**Status:** ACCEPTED

The Agent Inspector's Memory tab can edit and delete. That is the one place in the window, apart
from the kill switch and the undo journal, where a read-only surface writes, and it was not added
for convenience.

A system that lets a machine form durable beliefs about a person's work, and does not let the
person read them, is a system keeping secrets. It is also, more prosaically, a system with no
repair path: the first wrong memory an agent forms — a stale build command, a misread convention —
is repeated confidently for as long as it survives, and nothing in the decay model removes a
memory that keeps being retrieved.

So: every memory is listed with its scope, kind, text, provenance, usefulness and supersession
chain; the human can supersede it, correct it, or delete it. A deletion writes a `forget_memory`
episode with `actor = human`, which is what makes it survive a rebuild — replaying the assertions
must replay the deletion too, or the memory returns the first time the index is rebuilt and the
human has no reason to believe the button did anything.

**Three verbs, and the panel shipped with two.** "Supersede it, correct it, or delete it" was
written here and the first version of the Inspector could supersede with a memory that already
existed and could delete. *Correcting* one means writing the replacement, and there was nowhere
in the window to write anything — which also meant the human could not state a preference at
all, and a preference is the kind whose author is a person rather than a machine and which every
context pack carries unconditionally. There is a compose row in the Memory tab now, and the
per-row button that fills it in with what is being replaced.

**Revisit when:** the memory count makes a flat list useless. Then it needs the same treatment as
the result list — the daemon decides the order, the window renders it.

---

### D79 — The token budget is an estimate, and says so
**Status:** ACCEPTED

M11's Definition of Done said the context budget would be measured "with a tokeniser, not a
divisor". Building it made clear that the sentence hides a question it does not answer: *whose*
tokeniser.

Claude, GPT and Llama tokenise the same sentence differently. The daemon cannot know which one
the caller uses, and the only tokeniser this repository ships belongs to the embedding model — a
384-dimension WordPiece vocabulary for a retrieval model, which has nothing to do with any
language model's context window. Budgeting with it would have been *precise about the wrong
number*, which is worse than being honestly approximate about the right one, and it would have
looked rigorous while being wrong.

So `cortex_core::tokens` is a structural estimate, deliberately biased upwards, and every
surface that reports a token count reports it as an estimate:

- a run of ASCII letters or digits costs one token per 3.5 characters, rounded up, minimum one;
- punctuation costs one per character;
- a newline costs one; a run of spaces costs nothing, because a leading space merges into the
  word after it;
- anything outside ASCII costs one per character — accented Latin is about half that and CJK
  about one, which is where the upward bias does the most good, since German with umlauts is
  exactly the case `chars / 4` gets worst.

**The bias is not a hedge, it is the asymmetry.** Over-estimating wastes a little of the budget.
Under-estimating truncates a system prompt at a byte boundary. There is no symmetric answer.

**One property makes it safe to budget with, and it is asserted rather than assumed:** a
byte-pair encoding does not merge across a word boundary, so the true count is never below the
word count — and neither is the estimate. The tests also hold it under one token per character,
which no tokeniser exceeds on text. Between those two bounds it is calibration, and
`chars_per_token` is a parameter so that somebody who has measured their own model can say so
instead of being told a number this crate invented.

**Definition of Done §3 is amended to match**: the budget is honoured against the estimate, the
estimate is bounded by word count below and character count above, and the pack reports
`used_tokens` as an estimate rather than as a measurement.

**Revisit when:** the caller can declare its tokeniser and ship the vocabulary — an MCP client
could — or when a measurement of the estimate against a real tokeniser on this corpus exists.
Until then, adding a second tokeniser would be adding a second wrong answer.

---

### D80 — "It gets salience for free" was a claim about effort, not about fit
**Status:** ACCEPTED

D74's argument for making a memory a node rather than a second store was that salience, decay,
tiering and consolidation would then apply to it without being written twice. That argument is
still right, and the sentence *"so it gets them for free"* was not: nobody had run the
consolidation pass over a corpus of memories, and when somebody finally did, **every memory in
the index scored exactly 0.0 and sat in the Working tier for ever.**

The reason was one line. `raw_nodes`, which feeds the scoring pass, selects
`WHERE path IS NOT NULL AND kind IN ('file', 'dir')` — written before there was any node kind
that legitimately had no path. Memories were not scored low. They were not scored.

Three things changed, and the third is the one worth remembering.

**One: memories are in the pass.** Same query, same scoring function, same tier assignment. Not
a second engine — that is the thing D74 refuses, and a second engine is what this would have
grown into the moment somebody wanted memories ranked at all.

**Two: `usefulness` is a component of the shared model.** D76 said it "enters salience", and it
did not enter anything: the column was written, printed by the Inspector, and read by nothing.
It is `w8` now, zero for every file because a file cannot be reported useful, squashed by `tanh`
so that the first report to say a memory helped moves it noticeably and the fortieth does not
move it at all. Weighted below `recency` on purpose: a memory that has proved useful should
outrank one that has not, and it should not outrank being about what is happening right now.
The measured result, on a store of three memories and a two-month gap: fresh 1.68 in Working,
ignored-for-two-months 0.13 in Short-term, pinned 10.13 in Working.

**Three: a memory's tier floor is Long-term, never Cold.** Cold means "path and stat, no
embeddings, read only on an explicit deep search". That is a coherent thing to say about a file
and an incoherent one about a memory, whose text is the whole of it — there is nothing left to
be metadata-only about. Nothing breaks today, because memory retrieval does not filter on tier;
it is a word that would mean the wrong thing the first time anything acted on it.

**And a fourth thing, which is the actual lesson.** Adding the component did not work either,
and for a while it was worse than not adding it: `Components::sum()` lists its fields by hand,
so the new one was set, stored, printed — and contributed nothing. Two memories with entirely
different histories scored identically to sixteen decimal places, which is a suspicious enough
result to notice only because a test happened to compare them. There is now a test that assigns
each component a distinct power of two and asserts the total, so a missing term is not only
detected but *named* by the difference. It was checked against the broken version before being
kept.

**And a fifth, found by the test suite two minutes later.** Scoring memories meant the pass now
held ids for nodes that the memory housekeeping — expiry — was about to *delete*, and the write
in step 5 failed the entire consolidation with `FOREIGN KEY constraint failed`. Housekeeping runs
before the scoring now, which is also where it belongs on its own merits: scoring something that
is about to be deleted is wasted work. The order of the steps in a pass is part of its
specification, not an implementation detail.

**Revisit when:** enough outcome reports exist to ask D76's own question — whether a memory's
usefulness predicts a human calling it useful. If it does not, `w8` is decoration and should be
removed rather than tuned.

---

### D81 — An extraction is a transfer, and reports as one

**Status:** accepted · 2026-08-24 · M6

An archive is unpacked with `fs.extract`, which answers with an operation handle and then reports
on `transfer.progress` and `transfer.done` — the same two notifications a copy uses, and the same
`operation.cancel` stops it.

The alternative was a notification pair of its own, `extract.progress` / `extract.done`. It was
rejected because the difference it would encode is not one anybody has: to the person watching,
unpacking a 400 MB archive and copying a 400 MB directory are the same event — a bar, a name
going past, a cancel button — and the window would have had to grow a second subscription, a
second reducer and a second "what is running" state to render the same thing twice. The client
that already knew how to draw a transfer now draws an extraction for free.

What is *not* shared is the sentence at the end. `summariseExtraction` says "entpackt nach
bilder — 1 Eintrag nicht: is a symbolic link"; a transfer's wording would have said "1
übertragen" and dropped the refusal, because a transfer's `moved` is a list of files and an
extraction's is the one directory it made. One ref in `useOperations` remembers which of the two
was started, and it is the only thing that has to.

**The cost, stated:** a client that subscribes to `transfer.done` now receives events for
operations it did not start with `fs.transfer`. The window already filtered by operation id — it
has to, because two clients share a daemon — so nothing changed there, but a future client that
filtered by *method* instead would be surprised. That is the reason `TransferDone` carries the
`operation` and not the tool.

### D82 — Every path in an archive comes from a stranger

**Status:** accepted · 2026-08-24 · M6

The paths in a transfer came off the filesystem and through the policy engine. The paths in an
archive came from whoever built it. Four rules follow, and they are why this is 600 lines of Rust
rather than a call to `bsdtar`:

1. **Nothing lands outside the destination.** Entry paths are resolved component by component;
   an absolute path, a `..` or a Windows prefix is refused. Refused, not sanitised: `foo/../bar`
   rewritten to `foo/bar` silently changes what the archive said, and a rule that quietly edits
   hostile input is a rule nobody can reason about.
2. **Links are refused, not followed.** A symbolic link inside an archive is a path of the
   archive's choosing, and a hard link is worse. Neither is created. This costs something real —
   a source tarball with a symlink in it unpacks incomplete — and the entry is *named* in the
   answer for exactly that reason, rather than being skipped in silence.
3. **A declared size is a claim.** Both formats state an uncompressed size. It is what the
   progress bar counts against, and it is enforced while writing: an entry that produces more
   than it declared is stopped at the byte it crosses. That is what a zip bomb looks like from
   inside the write loop, and it needs no configured limit to catch.
4. **The default destination is a new directory beside the archive.** Not the current one. An
   archive with forty files at its root emptying itself over a directory somebody is standing in
   is the oldest surprise in file management, and the fix costs one directory. The name is the
   archive's, with its suffix stripped — the compound one, so `notes.tar.gz` gives `notes`. An
   archive identified by its magic bytes need not be named for its format at all, and the first
   one that was not (`bundle.bin`) unpacked into itself: a name with nothing left to strip now
   gets `-extracted`, and that case has a test.

**Undo takes back what was written and nothing else.** One journal entry per extraction, holding
one inverse step per path created: `Delete` for each file, `RemoveDir` for each directory. The
single-step alternative — "remove the destination recursively" — was rejected because it would
also remove whatever the person had put in that directory since. The price is a large row for a
large archive, roughly a hundred kilobytes of JSON per thousand files, which is the cheaper of
the two mistakes.

**And it leaves nothing behind when it stops.** The same list is the cleanup list: a cancellation
or a mid-archive failure removes every file and every directory the extraction created, in
reverse order, and directories that existed before are not touched because they were never
recorded. M6 §4 asks for this of a copy; an extraction that left half a tree looking like a
finished one would be the same failure with a different verb.

### D83 — A thumbnail is two questions, and only one of them is expensive

**Status:** accepted · 2026-08-24 · M6

Rows show a picture when there is one. Getting there is two calls, deliberately:

- **`thumbnails(paths, pixels)`** — one call for a whole viewport, answering which of those paths
  the shared XDG cache already holds. It is a stat per path and a PNG header read per hit, and
  **it never renders**. Its cost therefore does not depend on what is in the directory, which is
  the property that makes it safe to call while somebody scrolls.
- **`preview(path, pixels)`** — the existing call, one row at a time, for the misses that are
  still on screen after the scrolling stops.

The single-call alternative — "give me thumbnails for these two hundred rows" — was rejected on
the shape of the thing behind it. The previewer is one process answering one request at a time
(D14), shared with the preview pane. A call that might render turns a flick through a directory
of photographs into two hundred queued decodes, and the pane showing the file somebody actually
selected waits behind all of them. The queue is invisible from the caller's side: every request
looks slow, and none of them is.

Three consequences worth stating:

1. **Nothing is asked twice.** Every path ends in one of three states — a URL, `null`, or queued
   — and the answer names the paths that are not worth asking about again (`skipped`: denied,
   gone, or already found un-thumbnailable). Without that, scrolling back over a source tree
   re-asks about every file in it, forever.
2. **The row does not change size.** The image is `1.35em` inside a line of text, not a grid
   column: the virtualiser is *told* the row height rather than measuring it, so a row that grew
   by four pixels would put every scroll position progressively out. A browser test asserts a row
   with a picture and a row without are both exactly 28 px.
3. **No icon column.** A reserved column would indent every name in a directory of text files
   for the sake of the one picture in it, and this list is names rather than icons. The cost is a
   ragged left edge where pictures and plain rows alternate. That is a visual decision the owner
   has deferred along with the rest of the appearance; it is recorded here so that whoever does
   the visual pass knows it was a choice and not an oversight.

**And it stops when nobody is looking.** The hook takes the window's visibility and asks for
nothing while the overlay is hidden — the same rule D59 arrived at the expensive way, applied
before it could be measured again.

### D84 — A share is somebody else's disk, and everything follows from that

**Status:** accepted · 2026-08-24 · M6

Network locations are in, and the shape of them is decided by one fact: what is behind the mount
point is on another machine, reached over a network that can be gone without anything saying so.

**What this does not do: mount one.** A share appears in the places screen because the desktop
mounted it (gvfs, from any file manager or the portal) or because `/etc/fstab` did. M6 allows
this item to be staged and this is the stage boundary, drawn where the credentials are: mounting
an SMB share means asking for a password, storing it, and answering a re-authentication prompt
hours later in a process with no window. A daemon that did that would be a second, worse keyring
— and R5 says no secrets in this repository, which a credential store would immediately test.
What is built is the half that has no such problem: what is already mounted is offered, browsable
and searchable.

**Three places the difference shows, and what each one does about it.**

1. **The places screen never measures a share.** `statvfs` on a mount whose server has gone
   blocks in the kernel, and the first screen is not where somebody should discover that. A drive
   shows how full it is; a share shows its address — `smb://nas/fotos`, rebuilt from the name
   gvfs gave the mount, because two shares called `fotos` are otherwise the same row twice.
2. **`fs.list` knocks first.** A path on a network mount gets one `stat` on a thread of its own
   with a two-second deadline; if it does not answer, the call returns *"…did not answer within
   2000 ms — the share may be gone"* instead of holding a thread until the mount's own timeout
   expires, which for a default NFS mount is minutes. The probe is per call, not per row, and it
   is skipped entirely for local paths — the cost is a thread, and it is paid only where
   something can hang. **The stuck thread is left behind**: nothing in user space can cancel a
   blocked syscall, so it holds one stack until the kernel gives up and then ends. That is the
   honest cost, and it is smaller than a frozen file manager.
3. **The indexer stops at the mount point.** Not even a `stat` on it — the root of a dead share
   blocks like everything inside it. A share inside an indexed home directory would otherwise
   turn "index my files" into days of traffic against a server nobody mentioned. A root that *is*
   on a share is indexed anyway: configuring one is saying so on purpose, and a tool that refused
   would be overruling its owner. `ReconcileReport::network_skipped` counts what was not entered,
   because a number that is silently zero is how this gets broken later.

**What counts as a network filesystem is a list of types, not a pattern.** `fuse.sshfs` and
`fuse.gvfsd-fuse` are network filesystems; `fuse.exfat` on a stick in this machine's own USB port
is not. A rule like "anything with fuse in it" would refuse to index a memory card, which is the
kind of near-miss that only shows up on somebody else's machine.

**Where the parsing lives.** `/proc/mounts` is now read in `cortex-core::mounts`, because two
crates need the same answer for opposite reasons — the places screen to *offer* a share, the
indexer to *avoid* one — and two parsers would be two opinions about what a share is.

### D85 — One row stream, and what "no branch" honestly means

**Status:** accepted · 2026-08-24 · M10 step 1

`view.open(source, order?, limit?)` answers with a `View` — rows, the origin echoed back, the
order the daemon chose, a basis in words, and the counts. Two sources ship in step 1, `dir` and
`query`, and they are the same two calls underneath: a directory view is `Surface::list` with the
same deny-list, the same ordering and the same audit entry, and a query view is `run_search` with
the same activity ping, the same episode and the same reranking. Nothing about listing a
directory or running a search was reimplemented; what changed is that both answer in one shape.

**Why this was worth doing at all.** The window had two row-producing calls with different shapes
and a mapper per call — `rowFromHit` and `rowFromEntry` — plus a component that chose between
them. That mapping *is* the frontend deciding what a row is, which is the decision D4 keeps on
the daemon's side, and the visible consequence was that a file's memory (its tier, its zone, its
score) existed in one list and not the other. There is now one mapper, `rowFromView`, fed by one
method.

**What "the frontend contains no branch on browsing or searching" means, precisely.** The row
path has none: `rows` is a single array, mapped once, rendered by a component that cannot tell
where they came from. What remains — and what the paper does not ask to be removed — is
window-state: whether a drop has a directory to land in, whether "new folder" makes sense,
whether the breadcrumb is showing, and which of the two held views is on screen. Those are
questions about *this window*, not about what a row is, and calling them the same thing would be
overclaiming.

**Both views are held at once, on purpose.** M6 asks for search that replaces the list live and
gives the directory back the moment the query is cleared — instantly, not after a round trip. So
`useView` keeps the place and the overlay, and clearing the query shows the place again from
cache. A single-view hook would have made "clear the search" a network call, which is the
behaviour M6 explicitly rejected.

**The reranked order needed a second notification.** `search.update` carries hits; a client of
the view layer must never have to know what a hit is. `view.update` carries rows, and which of
the two the daemon sends is decided where the request arrived rather than by the reranker, which
is the same for both.

**The basis is in German.** It is a sentence composed by the daemon for display in a window whose
default locale is de-DE, and the places screen already sets that precedent ("Persönlich",
"Laufwerke"). The alternative — an English sentence in a German window, as `brain.graph`'s basis
currently is — is the inconsistency this chooses against.

**Revisit when:** step 3 adds sources that are neither spatial nor a query (`concept`,
`working_set`, `tier`). If `useView`'s two slots — a place and an overlay — cannot hold them,
the stack of views the paper describes in §4.2 replaces them, and this entry is where to say so.

### D86 — The memory columns cost less than the queries they replace

**Status:** accepted · 2026-08-24 · M10 step 2

Every row from every source now carries what the index knows about it: salience, tier, degree,
when it was last touched, whether somebody pinned it, and which zone it is in. §5 of the explorer
paper demanded the measurement *before* the design, with a stated threshold — **more than 15 ms
on ten thousand rows and the columns belong in a deferred second message instead.**

**Measured, on ten thousand rows** (`cortex-store/tests/listing_cost.rs`, debug build, warm page
cache, a tenth of the rows carrying edges and episodes):

| | |
|---|---|
| One `node_by_path` per row — what the listing already does, to mark a row as "new" | **286 ms** |
| One statement per chunk of 900 paths, with degree and last-episode joined in | **55 ms** |

The columns do not cost 15 ms. They cost **negative** time against the shape they sit next to,
and the test asserts the difference rather than the absolute number, because the absolute number
is a property of the machine and the ratio is a property of the design.

**A consequence worth naming rather than quietly enjoying:** the expensive half of a large
listing is now the per-row lookup inside `Surface::list`, not the memory. Replacing it with the
same batched statement is a follow-up, deliberately not done in this step — `fs.list` serves
agents and the CLI as well, and changing it belongs in its own change with its own test.

**Keyed by path, not by `parent_id`.** The first version selected the children of a directory
node. It came back empty against a freshly reconciled directory: the parent link is filled in by
a pass of its own, so a file has a path before it has a parent, and a listing that waited for the
link would show a directory with no memory in it for as long as that took. The view knows exactly
which paths it is showing; asking about those is both correct and independent of when the link
pass last ran.

**Degree is counted, not stored.** The paper suggests the consolidation pass leave a count
behind. It is not stored, for two reasons: the measurement says counting it is a fraction of a
listing's cost, and a stored count is a second copy of a fact that goes stale between passes —
`edges` is the fact. **Revisit when** a corpus appears where the count is the slow part: either an
index with a node of very high degree, or the batched statement crossing 15 ms in the test above.
The column is the answer then, and it belongs in the consolidation pass rather than in the
listing path.

**Tier is brightness, and pinned is the exception to it** (§4.4). The badge that used to say
"cold" on one row in fifty said nothing at all about the other forty-nine; four steps of one
dimension say something about every row, and it is the encoding the three-dimensional view
already uses, so the two windows teach one visual language. Applied to the row's *text* rather
than to the row, so a dim row is still visibly selected. A pinned row does not fade: somebody
said "keep this", and an interface that faded it would be disagreeing with them.

**Salience ordering happens in the daemon, on stored numbers.** The frontend asks for an order
and renders what comes back (D4). A directory has no salience of its own, so the listing arrives
by name and is reordered once the columns are attached, with the path as the tie-break — two
listings of one directory must never disagree, which is the same rule `fs.list` already has for names.
Nothing in the listing path waits on a model, which is §7's rule and the reason salience is
arithmetic rather than a ranking call.

### D87 — A concept is a place, and a place without a directory has to say so

**Status:** accepted · 2026-08-24 · M10 step 3

Five sources join the two from step 1: `concept`, `working_set`, `neighbours`, `timeline` and
`tier`. Each is a variant of `Source`, each answers `view.open` in the same row shape, each
appears in the places screen, and each says in words what rule it applied.

**They differ only in which nodes and in what order.** Concept membership is a `derived_from`
walk, the working set is the episode log asked "what lately", neighbours is the graph one step
out, a timeline is the episode log windowed, and a tier is `nodes` ordered by salience. After
that they all go through one function that turns a set of ids into rows — the alternative was a
row builder per source, which is five more places for "what a row is" to drift.

**No `stat` in these views.** The size and the modification time come from the index. That is
what a view of things you touched last week is *about*, and going to the disk for fifty rows
scattered across a home directory would be fifty seeks to tell somebody something they can
already see. A directory listing still stats, because there the disk is the subject.

**The window's place is a `Source` now, not a path.** This is the revisit D85 predicted. A
concept is somewhere you can *be*: it has a name, rows you can act on, and a way back out. The
history stack holds sources, so Alt+Left walks out of a concept exactly as it walks out of a
folder — which is this step's Definition of Done and the thing a path-shaped stack could not do.

**Up and back are different questions.** Only a directory has a parent. From a concept, "up" is
the way you came, so `goUp` falls through to `goBack` rather than inventing a hierarchy the
index does not have.

**A place without a directory refuses what needs one.** Paste, drop and "new folder" have
nowhere to land in a concept or a tier. They say so — *"dieser Ort ist kein Ordner"* — rather
than doing nothing, and there is one implementation of that refusal for the key and for the
palette entry. There were two for about an hour, and they had already drifted: the command said
so and the key silently did nothing, which is the worse half of the same bug.

**The basis line is rendered, and only where it earns its place.** A directory's basis repeats
what the status line says in three characters; a concept's says how the cluster was named and a
tier's says that nothing has been deleted, and there is nowhere else to learn either. So it shows
for the places the index made and not for a directory or a query.

**A tier is a subject in exactly one view.** Everywhere else it is brightness (D86). The cold
view is the exception because "what has been forgotten" is a question, and its basis line answers
the one that follows immediately: **nothing was deleted** — the files are on the disk, unchanged,
and opening or pinning one brings it back.

### D88 — Why a row is here, and what happened to it, are one question

**Status:** accepted · 2026-08-24 · M10 step 4

The sidebar's new job (§4.5). `node.why(path)` answers with the parts of the salience score, the
file's episodes, the notes attached to it, and the memory columns — in one call, because a person
looking at a row asks all of it in one glance. The panel used to make two calls for a fraction of
this and show a third thing it should not have been showing.

**The breakdown is read, not computed.** `salience_log` holds what the consolidation pass
calculated, component by component, and the panel prints those numbers. Recomputing them for
display would be a second implementation of the score, and the two would disagree the first time
a weight changed. When no pass has scored the node yet the panel says so — *"noch nicht bewertet"*
— rather than drawing a row of zeroes, because a fresh index holds no judgements and a zero that
means "unknown" is the most expensive kind of lie in an interface about memory.

**The parts are named in words a person can argue with**: *zuletzt benutzt*, *wie oft benutzt*,
*wie verbunden*, *Rauschen*. The noise term is signed and shown signed, so the parts visibly add
up to the number above them.

**"Verbunden" left the panel** and became a button that goes to the `neighbours` view (D87). A
panel can show the first three of a graph; a place can show all of it, sort it, and let you act
on it. What was lost is that the neighbours are no longer visible without a keystroke; what was
gained is that they are no longer *only* visible as three names in a corner.

**A note is written in one line, in the panel.** No dialog: a dialog for a sentence is a dialog
nobody opens, and the whole point of an annotation is that it costs less than editing the file.
The panel re-reads after writing rather than pushing the note into its own list — what it shows
has to be what the daemon stored, or the two drift the first time one of them rejects something.

**An annotation's episode belongs to the annotation, not to the file.** It is recorded against
the annotation node, so a rebuild replays it as what it was. The file's history therefore does
not contain "notiert", and the note appears once — in *Notizen*, with its own timestamp — rather
than twice. This was found by a test that asserted the opposite; the test was wrong and the
comment explaining why is now in it.

**Enter in the note field is not Enter in the window.** The window opens the selected row on
Enter and sees the key on its way up from the input. The field stops it, and Escape clears the
field rather than closing the window. Typing a note and pressing Enter used to open a file.

### D89 — Nothing in this window is painted by the platform

**Status:** accepted · 2026-08-24 · M4/M11 defect

A `<button>` with no `background` keeps the user agent's `buttonface`, which is **light**. Every
class in this window sets a text colour and most set no background, so every control that did not
explicitly paint itself came out light-grey text on a light-grey button — unreadable, in a dark
window. It hit the Agent Inspector's tab strip, its kill switch, the actions on a memory
(*Korrigieren · Ersetzen · Löschen*), *Schließen (Esc)* and the one `select` in the panel.

**It was reported as three separate bugs** — "the buttons are broken", "the buttons are ugly",
"the panel is hard to read" — and it was one missing rule. There is now a reset in `tokens.css`
for `button`, `select`, `input` and `textarea`, and the classes paint on top of it. `font:
inherit` is part of it: a control that keeps the platform font is a different size from the text
beside it at every zoom level.

**The rule is asserted rather than remembered.** `ui/tests/controls.spec.ts` walks every
`button`, `select` and `input` in the ordinary window, in the palette and on each Inspector tab,
computes the perceived lightness of its background, and fails on anything above the midpoint. A
dark window draws nothing light; that is checkable, so it is checked.

**And a tab that is not selected now looks pressable** — it lightens on hover and has a focus
ring. Before, an unselected tab was indistinguishable from a label, which is the other half of
"the buttons are broken": nothing said they could be pressed.

### D90 — A view inside a panel measures the panel, not the window

**Status:** accepted · 2026-08-24 · M8/M11 defect

The Agent Inspector's *Architektur* tab drew nothing on the owner's machine: a black box, no
error, correct data underneath — `cortex graph --limit 4000` answered with 4 023 nodes.

`BrainScene` measured `container.clientWidth/Height` once, at construction, and then listened to
**`window.resize`**. In the standalone `ui-agent` page the container *is* the window, so that is
right. Inside the Inspector the container is a panel that is mounted while it is still animating
in — measured at zero — and nothing resizes the window afterwards, so nothing ever corrects it.
A renderer sized 0×0 draws nothing and reports nothing.

The scene now also observes its container with a `ResizeObserver`, and `onResize` ignores a width
of zero rather than committing it. `ui/tests/architecture.spec.ts` asserts the canvas ends up the
size of the box it sits in.

**What this does not prove.** The test runs in Chromium; the window is WebKitGTK with
`WEBKIT_DISABLE_DMABUF_RENDERER=1` (D59), and whether WebGL works there is a separate question
with a separate answer. If the tab is still empty after this, the next thing to check is the
renderer, and the honest way to check it is to make the failure say which one it was — the tab
already has a failed state and shows the message, so an empty box means the renderer started.
