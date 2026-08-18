# 03 — Progress

**This is the living state file. Read it first, update it last, every session.**
If you finish work without editing this file, the next agent starts from nothing.

---

## Current position

| | |
|---|---|
| **Active milestone** | M2 — Meaning |
| **Status** | First half of M2 done: content is extracted, chunked, indexed and searchable end to end, with fusion, snippets and explainability. The models are not started. |
| **Last updated** | 2026-08-18 |
| **Blocked on** | nothing |

## Where M2 stands — read this first

M2 is being built in two halves. The first half is **meaning without models**: everything that
makes content searchable using no machine learning at all. The second half is the models. The
ordering is deliberate — if lexical search over extracted content is not good, no embedding will
rescue it, and the models are by far the heaviest dependency in the project.

**The first half is done and running.** On the reference corpus the daemon extracts 100 000
files into 122 137 chunks in about 26 seconds of work, and a word that appears only *inside* a
file is findable in roughly 15 ms. Everything below is what remains.

### Done, tested, and green (377 tests across the workspace)

- **`cortex-extract`** — `classify`, `extract`, `chunk` from the previous session, plus
  `pipeline` (one file in, one decision out, including the content-hash short circuit) and
  `snippet` (re-reads a chunk from the file and refuses to quote it if the hash disagrees).
- **`cortex-store::chunks`** — chunk writes that *diff* rather than replace: a chunk whose hash
  is unchanged keeps its row, its id and therefore its future embedding. Also the extraction
  queue, the state counters, and the BM25 query.
- **`cortex-query`** — `lexical` (FTS5 expression building that user input cannot break),
  `mode` (heuristic detection with prefix override, D24), `fusion` (RRF, salience boost,
  best-chunk-per-document) and `search` (the orchestrator, which lives here rather than in the
  daemon so the benchmark measures the code that ships).
- **`cortex-ingest::throttle`** — the duty-cycle arithmetic, clock-free and exactly testable.
- **`cortexd`** — the extraction stage on its own thread, the activity signal (D22), and the
  RPC surface for all of it.
- **CLI** — `cortex search` now shows snippets, `cortex explain` says why something ranked where
  it did or why it did not appear, `cortex extract` shows the queue and the CPU budget.
- **Benchmarks** — BM25 and fusion are gated; the corpus generator writes realistic prose (D23).

### Not done — the second half of M2

1. **The embedder.** `multilingual-e5-small` via ONNX Runtime, CUDA with CPU fallback, batched,
   model fetched by an explicit user action and checksum-verified (D6, R5). Model id and version
   stored per chunk. The chunk-hash short circuit already exists and is measured, so the
   embedder inherits "unchanged chunks are never re-embedded" for free — assert it with a call
   counter, which is M2 DoD §4.
2. **The vector index.** usearch HNSW, `f16`, memory-mapped, rebuildable from `embeddings`
   without re-running the model (DoD §8).
3. **The vector arm.** `fusion` already takes an arbitrary list of rankings; adding it is one
   more `Ranking` and one more branch in `search::run`. Nothing else should need to change —
   if it does, that is a design smell worth stopping for.
4. **The cross-encoder reranker.** Top-30, asynchronous, delivered as a result update. The
   protocol already carries `partial` and `ScoreBreakdown::rerank` for exactly this.
5. **The goldset.** 50 queries, German and English, judged, in `bench/goldset/`. Also the
   instrument for the open questions listed further down. Recall@10 gets recorded in this file
   and gated at −3 points.
6. **`cortex --rebuild-index`** (DoD §7, rule R10). The pieces exist — the extraction stage
   already re-chunks correctly when chunks are missing but hashes are not — but the command
   itself does not, and neither does the canonical dump it must reproduce.

Smaller things still open from the first half:

- **PDF extraction.** `Handling::Pdf` is classified and skipped. `pdf-extract` is available.
  When it lands, note that `snippet` currently re-runs the *text* reader for every handler; a
  PDF's byte offsets are into extracted text that no plain read reproduces, so the handling will
  have to be stored per node and passed to the snippet path.
- **Tree-sitter chunking**, still staged behind the heuristic (unchanged from last session).
- **A wall-clock cap per extractor.** The byte budget bounds memory and the stage's own thread
  bounds latency for everything else, but a single file on a stalled network mount can hold the
  stage indefinitely. Cheap to add with the text reader; genuinely needed once an extractor
  exists whose cost is not linear in bytes.
- **A reader pool.** `Store` is still one connection behind a mutex, so a search serialises
  against extraction writes. Now measurable: during a first index run, queries visibly wait.

### Deferred, still outstanding from M1

- **The 24-hour soak test.** Deliberately deferred by the project owner. Everything it would
  guard is currently unguarded: a descriptor leak in the watcher, or unbounded growth in the
  coalescer, would not be caught by anything else.

### Open questions the goldset should answer, not opinion

Recorded here rather than guessed at, because the goldset is the instrument that decides them:

- **Ties in fusion are broken by node id.** When a path hit and a content hit score identically,
  which should a person see first? Preferring the path arm is defensible ("a name is stronger
  evidence than a mention") but it is a tuning decision, and tuning without measurement is how a
  ranking becomes folklore.
- **Prefix matching is on by default.** It is what makes results appear while typing; it also
  makes "budge" match "budgetieren". `--exact` exists for measuring both.
- **Arm weights are all 1.0.** RRF needs no calibration, which is its virtue. Whether the
  content arm deserves more weight than the path arm for `recall` queries is measurable.
- **The mode heuristic** (D24) may mislabel queries. One function, its own tests, easy to fix
  once there is evidence.

### A limitation worth knowing before writing the goldset

FTS5's `remove_diacritics 2` folds `ä` to `a`, so a query typed without umlauts matches. It does
**not** fold `ß` to `ss`, because ß carries no diacritic — it is a distinct letter. So "grosse"
does not find "große". There is a test asserting this gap so that it is recorded rather than
rediscovered as a recall mystery. Closing it needs either a custom tokenizer or query-side
expansion, and it will affect the German half of the M2 goldset.

### Things M1 deliberately left rough

- The path index is rebuilt wholesale after any bulk change (reconcile, directory rename,
  directory removal). Correct and simple; if it ever shows up in a profile, patch incrementally
  instead — but only then.
- `assign_zones` runs one `UPDATE ... LIKE` per zone over the whole index. Linear in
  zones × nodes, which is why the zone budget exists. If zones ever need to be numerous, this
  is the thing to replace, not the budget.
- The ingest loop polls on a 250 ms timer rather than blocking on the inotify descriptor.
  Measured idle cost is 0.033 % of one core, so this is not worth changing without a reason.

---

## Environment (reference target)

Recorded because benchmark numbers are meaningless without it.

| | |
|---|---|
| Workstation OS | Ubuntu 26.04, Wayland, niri (scrollable tiling) |
| CPU | 10-core / 16-thread desktop class (i5-13400F) |
| RAM | 30 GB |
| GPU | RTX 5070 Ti, 16 GB, driver 595.x |
| Rust | 1.93.1 |
| Measured home directory | 78 621 files, 5 109 directories, 8.0 GB |
| Corpus bucket | 50 k–500 k (lower end) — local embeddings are comfortably realistic |

Note: the reference corpus for benchmarks is **synthetic and generated**, never the real home
directory. Benchmarks must be reproducible on a machine that has never seen this system.

---

## Decisions already made

See `02-DECISIONS.md` for the full log with reasoning. Summary of what is settled:

- Headless daemon core, all UIs are clients (D1)
- Brain first; full file manager is a committed milestone, not optional (D2)
- Rust daemon (D3)
- Tauri v2 + React + Framer Motion, provisional until M4 (D4)
- One SQLite file + FTS5 + usearch HNSW, provisional until M2 benchmarks (D5)
- `multilingual-e5-small` embeddings on the local GPU, never on a server (D6)
- No LLM in the hot search path, ever (D8)
- Two repositories: public Core, private Extension; extension loaded as a subprocess plugin (D9)
- The Core is fully functional with no Main Brain at all (D10)
- English code and docs, i18n UI defaulting to German (D12)
- Previews in a separate process, progressive, size-capped (D14)
- Zone colours derived deterministically, fixed saturation/lightness, never the only signal (D15)
- `inotify` rather than `fanotify`; the spec's mandate was measured and rejected (D20)
- Path search has two tiers; the full scan runs only when the literal search finds nothing (D21)
- User activity is signalled by clients, and expensive stages run on a duty cycle (D22)
- The benchmark corpus obeys Zipf's law, and its content is seeded per path (D23)
- `Auto` is the default query mode, and a mode that is not built yet degrades visibly (D24)

## Still open

| # | Question | Needed by |
|---|---|---|
| D13 | License for the public Core (recommendation: Apache-2.0) | M9 |
| — | Concrete peer protocol for federation | M7 |

---

## Session log

Newest entry on top. One entry per working session. Keep entries short and factual — what
changed, what was learned, what the next agent needs to know.

### 2026-08-18 — M2, first half: content is searchable

The extraction layer is wired in, and the search it feeds is complete without any model in it.
377 tests green, every gate passing, no daemon left running.

What exists now, end to end: a file is classified, read within a byte budget, split at its
structure, hashed per chunk, written into a contentless FTS index, retrieved by BM25, fused with
the path arm by RRF, boosted by salience, and displayed with a snippet re-read from the file. If
it does not show up, `cortex explain` says which of those steps is the reason.

Five things worth knowing, four of which cost real time:

1. **A benchmark can fail because of its input.** BM25 top-100 measured 44 ms against a 30 ms
   budget. The cause was not retrieval: the corpus filled every file with sentences drawn from a
   list of sixteen, so every term occurred in almost every document, and ranking a query meant
   scoring a third of the index. With a Zipfian vocabulary the same code measures 15 ms. The
   corpus generator now writes realistic prose (D23) — and the pathological case is still
   measured and printed on every run, because a corpus good enough to pass a gate must not be
   allowed to hide the case that does not.

2. **The queue query cost more than the work it fed.** Draining the extraction queue
   newest-first needs `ORDER BY mtime DESC`, which cannot use the partial index on
   `extract_state`: every batch read the whole backlog, evaluated a chunk count per row, and
   sorted it. **11 ms per batch with 50 000 files pending, against 6 µs in index order.** Recency
   is now handled where it costs nothing — the watcher pushes just-touched files onto a small
   priority queue that the stage drains first. A file saved during an 85 000-file backlog is
   searchable by content within about six seconds; before, it waited for the backlog.

3. **A duty cycle that measures the wrong thing is not a duty cycle.** The first version timed
   only the extraction, leaving the queue read and the commit — which dominate a batch of small
   files — outside the slice. It also accumulated microsecond work in a millisecond counter, so
   after extracting 100 000 files the stage reported that it had never done any work. Both
   fixed; the share now measures at exactly 15.0 % of one core while the user is active, against
   a configured 15 %.

4. **Polling a database "just in case" is not free.** With the queue empty, asking once a second
   whether anything was pending cost 0.65 % of one core on a 100 000-file index — more than the
   daemon's entire idle budget of 0.5 %. The stage now backs off to 30 s and is woken by a signal
   from the watcher. Idle cost afterwards: 0.100 % for the whole daemon.

5. **The content hash cannot be trusted on its own.** `--rebuild-index` drops chunks and keeps
   hashes, so a stage comparing only hashes would decide there was nothing to do and leave the
   corpus permanently unsearchable. The queue therefore reports the chunk count alongside the
   hash, and a file with a matching hash and no chunks is re-chunked. Rule R10 has a test.

Verified live against the reference corpus, with a real daemon:

- 100 000 files extracted into 122 137 chunks, 26 s of work at the idle budget.
- A re-extraction of the entire corpus wrote **zero** new chunks — the hash short circuit
  recognised every file as unchanged. That is M2 DoD §4 minus the embedder that does not exist
  yet, and the number it asserts is exactly the number of embedding calls a re-index will make.
- Deny-listed files (`.aws/credentials`, `.env`, `*.kdbx`, `id_ed25519`) are absent from the
  index entirely, not merely unextracted.
- A file renamed to a deny-listed name loses its chunks: withdrawn content stops being
  searchable rather than lingering.
- Idle RSS 170.3 MiB with 100 000 files and 122 137 chunks indexed, against a 250 MB budget.

One structural change worth noting for anyone looking for the code: the search orchestrator
moved from `cortexd` into `cortex-query::search`. The daemon binary cannot be depended on, and a
benchmark that reimplements the search it measures is a benchmark that eventually measures
something else.

### 2026-08-18 — M2 begun: the extraction layer

Stopped deliberately at a clean boundary so the next session has a well-defined starting point.
275 tests green, workspace builds, no daemon left running.

Built `cortex-extract` (classify, extract, chunk) and migration 0002. **None of it is wired
in** — see "Where M2 stands" at the top of this file for exactly what to do next and in what
order.

Three things worth knowing:

1. **"MIME by magic bytes, never by extension" is only half a rule.** Binary formats have magic
   numbers; plain text does not. There is no byte sequence meaning "this is Markdown". So the
   actual rule is: deny-list first, then magic number if present, then content sniffing to
   decide whether it is text at all, and only then the extension — used solely to pick which
   text handler runs. The extension can never make an unreadable file readable, and never
   overrides a magic number. A JPEG named `.txt` is a JPEG, and there is a test saying so.
2. **A byte budget must bound the read itself, not check the size afterwards.** `take()` on the
   reader means a file that grows while being read still cannot exceed the budget. Checking
   `metadata.len()` first would be a race.
3. **Slicing a string at a byte offset derived from arithmetic will panic sooner or later.** The
   chunker's hard-split searched for a line break inside `text[cursor..cursor + MAX_CHARS]`, and
   that endpoint is not a character boundary in German text. Caught by the test written for
   exactly that case. Every offset that reaches a slice now goes through `floor_char_boundary`.

### 2026-08-18 — M1: zones and the directory graph

The two structural gaps M1 left behind are closed. 225 tests green, every latency gate still
passing, RSS 150 MiB.

**Zones (D15).** `cortex_core::zones` derives a stable hue from a zone's path; detection lives
in `cortex_ingest::zone_detect`. Two sources qualify as a zone: conventional areas directly
under a configured root (`Documents`, `Projects`, `Pictures`, …) and project roots anywhere
within three levels, identified by markers like `.git` or `Cargo.toml`. The deepest containing
zone wins, so a repository inside `Documents` reads as that repository. Observable through
`cortex zones`, overridable with `cortex zones set-color`.

**The directory graph.** `parent_id` is populated: in bulk by `Store::link_parents` after a
reconcile, and directly on incremental inserts, where the parent is one cached lookup away.
`cortex node` now shows both parent and zone.

Three things worth knowing:

1. **A zone budget was necessary, and it is a real constraint rather than a safety valve.**
   Zone assignment costs one pass over the index *per zone*. The benchmark corpus produces 567
   candidates, because its generator scatters `.git` directories as noise and those are genuine
   project markers — the initial index went from 4.1 s to 7.1 s before the cap. Capped at 64 by
   default, with the drop logged. A real home directory is far below that; the corpus is the
   pathological case, which is exactly what it is for.
2. **Truncating to a budget needs a total order.** Sorting candidates by depth alone leaves ties
   to directory iteration order, so which zones survived the cap — and therefore every colour —
   could differ between runs on an unchanged tree. That defeats the entire premise of D15.
   Now sorted by depth, then path.
3. **Bulk assignment is for when the zone *set* changes, not when a file appears.** Re-running
   it per new directory would make creating a handful of folders take longer than the initial
   index. Incremental inserts read a cached zone list instead, and full re-detection is
   triggered only by a conventional directory name or a project-marker file appearing.

The `NodeUpsert` zone field distinguishes "no opinion" from "no zone": a bulk metadata refresh
passes `None` and the stored assignment is kept, because otherwise every size change would blank
the colour of the file that changed.

### 2026-08-17 — M1: perception

Three new crates (`cortex-query`, `cortex-ingest`, plus the `bench/` tooling), 92 → 185 tests,
all green. Every measurable M1 gate passes; three items remain and are listed at the top of
this file.

What exists now:

- **Reproducible benchmark corpus.** `bench/corpus/generate.sh` builds 100 000 indexable files
  plus 190 000 skippable ones from a fixed seed, with a hand-rolled PRNG so the corpus is
  byte-identical forever — a benchmark whose input can drift with a dependency version is not a
  benchmark.
- **Path index** (`cortex-query`): trigram postings over a single concatenated byte buffer,
  fzf-style scoring with smart case, tombstoned removal, compaction.
- **Watcher** (`cortex-ingest`): `inotify` with a watch budget and reported degradation.
- **Coalescer**: clock-free and filesystem-free, so every timing rule is tested exactly rather
  than with sleeps.
- **Reconciliation**: two-pass, rename-aware, batched.
- Daemon integration, plus `cortex watch status` and `cortex reconcile`.

Four things worth knowing that cost real time:

1. **Renaming a directory deleted its subtree from the index.** `IN_MOVE_SELF` fires on the
   moved directory's *own* watch in addition to the parent's `IN_MOVED_FROM`/`IN_MOVED_TO` pair.
   It carries no destination, so it was mapped to a deletion — and a deletion supersedes a
   pending rename in the coalescer. Net effect: a rename erased the subtree. Fixed by dropping
   `IN_MOVE_SELF` from the watch mask entirely; the parent's pair is exact and reconciliation
   covers the case where the parent is not watched. Regression test added.
2. **Reconciliation had to be two passes.** Resolving renames in the same loop that handles
   adds and removals means a directory rename rewrites its descendants' paths while the loop is
   still holding the pre-rename map. The stale entries it then tried to move clobbered the rows
   the first move had just written. Renames are now resolved first, against a snapshot, and the
   index is re-read before anything else runs. `rename_node` additionally refuses to act when
   its source is absent, so a stale caller can no longer delete a good row.
3. **My own measurement script was wrong, twice.** `grep '"files":[0-9]*'` never matches
   pretty-printed JSON, because there is a space after the colon — so the polling loop always
   ran to its timeout and I read 240 s and then 132 s as index times. The daemon log had the
   real number all along: **4.07 s**. Parse JSON with a JSON parser. Do not trust a timing
   number that comes from a shell loop you wrote in the same breath as the thing it measures.
4. **Sorting, not matching, dominated the keystroke path.** A one-character query matches
   nearly every path, and fully sorting 100 000 candidates to display 50 cost more than finding
   them. `select_nth_unstable_by` plus a parallel scan took the P95 from 21 ms to 4.75 ms.

### 2026-08-17 — M0 implemented and accepted

Five crates, 92 tests, all green. Every M0 acceptance criterion verified by hand as well as by
test — the numbers are in the benchmark table below.

What exists now:

- `cortex-core` — config (layered, validating, safe by default), paths (XDG, `0700`/`0600`),
  domain model, ID newtypes, and the **deny-list**. The deny-list landed early rather than in
  M1 because rule R1 is not something to retrofit.
- `cortex-store` — the full schema as migration `0001`, a forward-only migration runner that
  refuses a database from a newer build, and the M0 query surface.
- `cortex-rpc` — JSON-RPC 2.0 types with the version enforced at the type level, newline-delimited
  JSON framing, and a client used by every other component.
- `cortexd` — lifecycle, `flock`-based single-instance lock, socket at `0600`, signal handling,
  metadata walker.
- `cortex-cli` — `ping`, `status`, `search`, `index`, `node`, `episodes`, `shutdown`, each with
  `--json`.

Two things worth knowing that cost time:

1. **Partial unique indexes need their `WHERE` clause repeated in `ON CONFLICT`.** The index on
   `nodes(path)` is partial because concept and annotation nodes have no path. Without repeating
   the predicate, every upsert failed with "ON CONFLICT clause does not match any PRIMARY KEY or
   UNIQUE constraint" — and because the walker treats an upsert failure as a skip, it looked
   like an over-eager deny-list rather than a SQL bug. If indexing silently skips everything,
   check the SQL before the classifier.
2. **The hygiene gate caught our own test fixtures**, which used made-up single-letter home
   directories instead of the documented placeholder. Fixed the fixtures rather than widening
   the exemption list. Keep resolving it in that order — the gate is only worth having while
   it is stricter than we are.

### 2026-08-17 — Planning and documentation

- Read the source specification (`docs/99-SOURCE-SPEC.md`) and answered its §16 open questions
  with the project owner. Answers recorded as D2, D10, D12.
- Measured the real corpus: 78 621 files / 8.0 GB. This is the low end of the 50 k–500 k bucket
  and it is what makes local embedding and a single SQLite file the right call (D5, D6).
- Extracted the `island-ui` design system into `06-DESIGN-SYSTEM.md`: tokens, geometry scale,
  named springs, timing constants, layer model. This is what makes D4 (web-based GUI) the
  cheaper option — the motion system already exists in React + Framer Motion.
- Established the documentation set: glossary, build prompt, decision log, architecture,
  hard rules, design system, this file.
- Created the two repositories: public Core and private Extension.
- Consulted an agent on the owner's home server about the Main Brain and the federation path.
  Findings are recorded in the **private** Extension repository, not here — the public Core
  must contain nothing about a specific peer (§R9).
- Verified the toolchain: Rust 1.93.1, cargo can fetch crates.

**Key correction to the source spec discovered during the survey:** the spec suggests offloading
expensive model work to the home server. The survey showed the server has **no GPU**, 15 GB RAM
and roughly thirty always-on services, while the workstation has a 16 GB RTX-class GPU. The
direction is therefore reversed — embeddings and reranking stay local. Recorded as D6.

---

## Benchmark results

Empty until M0 lands its first benchmark. Record P95 values here per milestone so regressions
are visible across sessions.

| Date | Milestone | Metric | Value | Budget | Pass |
|---|---|---|---|---|---|
| 2026-08-17 | M0 | Daemon cold start → socket accepting | 12 / 14 / 23 ms | 200 ms | yes |
| 2026-08-17 | M0 | `cargo test --workspace` | 92 passed, 0 failed | green | yes |
| 2026-08-17 | M1 | Fuzzy path match, literal (tier one) | P95 1.25 ms | 10 ms | yes |
| 2026-08-17 | M1 | Keystroke path, every prefix incl. 1–2 chars | P95 4.75 ms | 10 ms | yes |
| 2026-08-17 | M1 | Abbreviation, full-scan tier | P95 4.60 ms | 25 ms | yes |
| 2026-08-17 | M1 | Full initial index, 105 273 entries | 4.07 s | 60 s | yes |
| 2026-08-17 | M1 | Watch placement, 5 273 directories | 62 ms | — | — |
| 2026-08-17 | M1 | Idle CPU, no filesystem activity | 0.000 % of one core | 0.5 % | yes |
| 2026-08-17 | M1 | Idle RSS, 105 273 paths indexed | 176.3 MiB | 250 MB | yes |
| 2026-08-17 | M1 | Path index footprint | 69.6 MiB / 105 273 paths | — | — |
| 2026-08-17 | M1 | `cargo test --workspace` | 185 passed, 0 failed | green | yes |
| 2026-08-18 | M1 | Full initial index incl. zones and parents | 4.32 s reconcile + ~3.2 s structure | 60 s | yes |
| 2026-08-18 | M1 | Idle RSS, 105 273 paths, zones assigned | 150.2 MiB | 250 MB | yes |
| 2026-08-18 | M1 | Fuzzy path match, literal | P95 1.23 ms | 10 ms | yes |
| 2026-08-18 | M1 | Keystroke path | P95 5.23 ms | 10 ms | yes |
| 2026-08-18 | M1 | `cargo test --workspace` | 225 passed, 0 failed | green | yes |
| 2026-08-18 | M2 | Fuzzy path match, literal (tier one) | P95 1.11 ms | 10 ms | yes |
| 2026-08-18 | M2 | Keystroke path, every prefix incl. 1–2 chars | P95 6.61 ms | 10 ms | yes |
| 2026-08-18 | M2 | Abbreviation, full-scan tier | P95 4.75 ms | 25 ms | yes |
| 2026-08-18 | M2 | BM25 top-100 | P95 14.74 ms | 30 ms | yes |
| 2026-08-18 | M2 | RRF fusion + salience boost | P95 12.75 µs | 5 ms | yes |
| 2026-08-18 | M2 | Hybrid query, both arms + salience + snippets from disk | P95 20.96 ms | (80 ms from M4) | — |
| 2026-08-18 | M2 | Stopword query matching 120 692 of 122 137 chunks | P95 78.7 ms | — | — |
| 2026-08-18 | M2 | Content index build, 100 000 files → 122 137 chunks | 23.5 s | — | — |
| 2026-08-18 | M2 | Extraction of the whole corpus, live daemon, idle budget | 25.9 s busy | — | — |
| 2026-08-18 | M2 | Re-extraction of an unchanged corpus | 0 chunks written | 0 | yes |
| 2026-08-18 | M2 | CPU share while the user is active | 15.0 % of one core | 15 % | yes |
| 2026-08-18 | M2 | Idle CPU, 122 137 chunks indexed | 0.100 % of one core | 0.5 % | yes |
| 2026-08-18 | M2 | Idle RSS, 100 000 files, 122 137 chunks | 170.3 MiB | 250 MB | yes |
| 2026-08-18 | M2 | `cargo test --workspace` | 377 passed, 0 failed | green | yes |

Search figures come from `bench-latency` against the standard corpus, **release build**. Index,
CPU and memory figures are from a live daemon, and the CPU share is measured from the stage's own
busy counter against wall-clock time rather than from `top`.

**The M0 and M1 rows above were measured against a different corpus** and are not directly
comparable with the M2 rows. The generator now writes realistic content (D23), which changed both
the tree slightly and the work per file entirely. Where the M2 rows repeat an M1 metric, that is
a re-measurement, not a regression: the keystroke path moved from 5.23 ms to 6.61 ms, still
inside its 10 ms budget, and the cause is the corpus rather than the code. It also varies by a
couple of milliseconds between runs, which is worth knowing before reading a small change in it
as a regression.

The stopword row has no budget on purpose. Nobody searches for "und", but FTS5 scores every
match before it can rank them, so a query that matches almost the whole index costs the whole
index. It is measured on every run so that the corpus cannot quietly hide it.

### M2 acceptance so far — the half that has no models in it

The full Definition of Done for M2 is in `docs/01-BUILD-PROMPT.md`; these are the items that can
be judged before the embedder exists.

| Criterion | Result |
|---|---|
| BM25 top-100 under 30 ms | 14.7 ms P95 |
| RRF fusion + salience boost under 5 ms | 12.8 µs P95 |
| Re-indexing an unchanged corpus re-embeds zero chunks | verified live over 100 000 files: 0 chunks written, every file recognised by hash |
| Idle RSS below 250 MB with 100 k files indexed | 170.3 MiB with 122 137 chunks |
| First index run stays inside the configured CPU share | 15.0 % of one core measured against a configured 15 % while the user is active |
| Every result carries a score breakdown, and `cortex explain` prints it | yes, including why a file did *not* appear |
| Content extraction only inside configured roots (R3) | verified live: a file outside them is recorded as skipped, with the reason |
| The deny-list holds before any content access (R1) | verified live: sensitive files are absent from the index entirely |
| Withdrawing content withdraws it from the index | verified live: renaming a file to a deny-listed name deletes its chunks |
| Recall@10 on the goldset | **not measured** — the goldset is the second half of M2 |
| German within 10 % of English | **not measured** — same |
| `cortex --rebuild-index` reproduces a canonical dump | **not built** — the extraction side of it is tested (a hash without chunks re-chunks) |
| Vector index rebuildable from `embeddings` | **not built** — no embedder yet |

### M0 acceptance, verified

| Criterion | Result |
|---|---|
| `cargo test --workspace` green | 92 tests pass |
| `cortex index` then `cortex search foo` returns ranked rows | yes — finds `report-foo.md`, ranks stably |
| Cold start under 200 ms | 12–23 ms |
| `kill -9` mid-flight, restart, consistent database | yes — index intact, search works, no corruption |
| Second daemon refused with a clear message | yes — names the lock file and the holding pid |
| Works with no config file and no state directory | yes — creates both, safe defaults |
| Deny-list holds end to end | yes — `.ssh`, `id_ed25519`, `.env` and `node_modules` absent from the index |

### M1 acceptance, verified

| Criterion | Result |
|---|---|
| 100 000 files indexed, initial index under 60 s | 4.07 s for 105 273 entries |
| Fuzzy path search P95 under 10 ms | 1.25 ms literal, 4.75 ms including 1–2 character prefixes |
| Idle CPU below 0.5 % of one core | 0.000 % over a 30 s sample |
| Idle RSS under 250 MB with 100 k files | 176.3 MiB |
| Deny-list: no sensitive path reaches the index | verified live against a corpus containing `.ssh/id_ed25519`, `.aws/credentials`, `.env`, `*.pem`, `*.kdbx` |
| Move detection: `mv` of a directory is not a mass delete plus create | file and directory counts unchanged; the new path and its children are searchable |
| Live watcher picks up a new file | searchable within ~2 s at a 300 ms debounce |
| Reconcile after a blind period equals a from-scratch rebuild | asserted by unit test against a canonical path dump |
| Watcher survives 24 h without leaking descriptors or memory | **not tested** — deferred by the owner; see outstanding items at the top |
| Zones detected, coloured stably and assigned deepest-first | verified live: conventional areas and project roots, distinct hues, a repository inside an area wins over it |
| A new project becomes a zone without a restart | verified live: detected ~3 s after its `package.json` appeared |
| Directory graph is complete | verified live: a file walks up through every ancestor to the indexed root, which correctly has no parent |


## Goldset recall

Empty until M2.

| Date | Model | Recall@10 (EN) | Recall@10 (DE) | Notes |
|---|---|---|---|---|
| — | — | — | — | — |
