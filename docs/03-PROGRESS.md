# 03 — Progress

**This is the living state file. Read it first, update it last, every session.**
If you finish work without editing this file, the next agent starts from nothing.

---

## Current position

| | |
|---|---|
| **Active milestone** | M2 — Meaning |
| **Status** | Everything in M2 except the cross-encoder reranker. Content is extracted, chunked, embedded and searchable by meaning; recall is measured against a checked-in goldset; the index rebuilds itself. |
| **Last updated** | 2026-08-18 |
| **Blocked on** | nothing — but see "One question for the owner" below |

## Where M2 stands — read this first

M2 was built in two halves: meaning without models, then the models. **Both are done.** What is
left of the milestone is one component, one protocol change, and one hardware question.

A query now finds documents that share no word with it, in either language. Verified live on the
reference corpus: the English query *"how fast did storage expenses grow after moving the
archive"* returns a German document — zero candidates from the path arm, zero from the lexical
arm, vector rank 3.

### One question for the owner

**Should CUDA be installed?** This machine has an RTX-class GPU and its driver, but no CUDA
runtime and no cuDNN, so the embedder falls back to the CPU and says so. That costs:

- 75 ms per chunk instead of the one to two milliseconds a GPU would take. Embedding the
  reference corpus is ~2.5 hours of CPU where it would be a couple of minutes.
- The **cross-encoder reranker**, which is the last M2 component, has a 150 ms budget for thirty
  pairs. On this CPU that is an order of magnitude out of reach. The reranker is therefore
  waiting on this answer rather than on anything in the code.

Installing CUDA plus cuDNN is several gigabytes and needs root, which is why it has not been
done. Everything else works without it.

### Done, tested, and green (466 tests across the workspace)

The first half, from the previous session: `cortex-extract`, chunk storage that diffs rather than
replaces, BM25, RRF fusion, snippets, `cortex explain`, the extraction stage.

This session:

- **`cortex-embed`** — the model registry (everything this project will ever download, pinned by
  SHA-256), provisioning with verification, `multilingual-e5-small` through ONNX Runtime with
  mean pooling and L2 normalisation, and a hashing embedder that needs no download at all.
- **Vectors in the store** — keyed by content hash (D26), plus a usearch HNSW index that is
  memory-mapped (D27) and rebuildable from the table without the model.
- **The embedding stage** — its own thread, its own queue, the same duty cycle and kill switch as
  extraction (D28).
- **The vector arm** — a third `Ranking` into the same fusion. `cortex explain` reports why it did
  not run, when it did not.
- **`cortex model fetch` / `status` / `reload`** — provisioning without a daemon, and a model that
  can arrive while one is running.
- **`cortex rebuild` and `cortex dump`** — rule R10 as a command and as a comparison.
- **The goldset** — 29 judged documents, 50 judged queries, a runner that reports recall@10 by
  arm and by what each query tests, and a checked-in baseline that fails the run on a drop of
  more than three points.

### What the goldset says

Measured with the real model against 1 029 documents (29 judged, 1 000 generated distractors):

| arm | overall | German | English | by name | lexical | semantic | cross-language |
|---|---|---|---|---|---|---|---|
| **fused** | **91.0 %** | 87.0 % | 95.7 % | 100 % | 100 % | 98 % | 33 % |
| lexical only | 36.0 % | 33.3 % | 39.1 % | 100 % | 86 % | 0 % | 0 % |
| vector only | 91.0 % | 87.0 % | 95.7 % | 100 % | 100 % | 98 % | 33 % |

Four things worth taking from that table:

1. **The model buys 55 points.** Without it, every query whose answer does not repeat its words
   fails. That is the case this project exists for.
2. **German is 9 % behind English**, inside the 10 % Definition-of-Done §3 allows — but only
   just, and the whole gap is cross-language queries.
3. **Cross-language retrieval is the weak spot at 33 %**, and the cause is measured rather than
   guessed: same-language similarity carries an offset of roughly 0.08–0.12 over cross-language
   similarity for this model. `goldset --explain q43` shows a German query returning twenty
   German *noise* documents ahead of the English document that answers it. Candidate mitigations,
   none of them tried: per-language score normalisation, query-side translation, or a reranker
   (which cannot help — it reorders candidates and this failure is that the candidate never
   appears).
4. **The lexical arm adds nothing the vector arm does not already find**, at recall@10 on this
   goldset. It is still worth its 15 ms: it answers with no model at all, it is what a fresh
   install has, and precision at rank 1 is not what recall@10 measures.

A caveat that belongs with the number: the distractors are semantically empty but linguistically
German, which is the worst case for cross-language retrieval. On a corpus of real documents the
33 % would be better. Fixing that means writing a thousand meaningful distractors.

### Not done

1. **The cross-encoder reranker.** See the question above. It also needs the protocol to deliver
   a second answer to one request — the socket is request/response today. That same mechanism is
   what M4 wants for progressive results, so it is worth designing once.
2. **PDF extraction.** `Handling::Pdf` is classified and skipped. When it lands, note that
   `snippet` re-runs the *text* reader for every handler; a PDF's byte offsets are into extracted
   text that no plain read reproduces, so the handling will have to be stored per node.
3. **Tree-sitter chunking**, still staged behind the heuristic.
4. **A wall-clock cap per extractor.** The byte budget bounds memory and the stage's own thread
   bounds latency for everything else, but one file on a stalled network mount can hold the stage
   indefinitely.
5. **A reader pool.** `Store` is still one connection behind a mutex.
6. **The 24-hour soak test**, deferred by the owner since M1 and still the only thing that would
   catch a descriptor leak in the watcher.

### Open questions the goldset can now answer

It is built, so these stop being opinions the moment somebody spends an afternoon on them:

- **Ties in fusion are broken by node id.** Should a path hit beat a content hit at equal score?
- **Arm weights are all 1.0.** Is the vector arm worth more than the lexical arm for `recall`?
- **`rrf_k` is 60**, the value from the paper, never measured here.
- **Prefix matching is on by default** for interactive queries and off for the goldset.
- **The mode heuristic** (D24) may mislabel queries.

### A limitation worth knowing before extending the goldset

FTS5's `remove_diacritics 2` folds `ä` to `a`, so a query typed without umlauts matches. It does
**not** fold `ß` to `ss`, because ß carries no diacritic. There is a test asserting the gap.
Separately, and now visible in the goldset: FTS5 has **no stemmer configured**, so
"verworfen" does not match "verwirft" and two of the lexical queries fail on the lexical arm
alone. Both are query-side expansion problems and both would show up as a recall change.

### Things earlier milestones deliberately left rough

- The path index is rebuilt wholesale after any bulk change. Correct and simple.
- `assign_zones` runs one `UPDATE ... LIKE` per zone over the whole index, which is why the zone
  budget exists.
- The ingest loop polls on a 250 ms timer rather than blocking on the inotify descriptor.
  Measured idle cost is 0.033 % of one core.

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
- The model and the ONNX runtime are provisioned like data, not like dependencies (D25)
- Embeddings are keyed by the hash of the text, not by the chunk row (D26)
- The vector index is memory-mapped; the memory budget is read as anonymous memory (D27)
- Embedding is a separate stage from extraction (D28)

## Still open

| # | Question | Needed by |
|---|---|---|
| D13 | License for the public Core (recommendation: Apache-2.0) | M9 |
| — | Concrete peer protocol for federation | M7 |

---

## Session log

Newest entry on top. One entry per working session. Keep entries short and factual — what
changed, what was learned, what the next agent needs to know.

### 2026-08-18 — M2, second half: the model, and what it is worth

Embeddings, the vector index, the semantic arm, the goldset that measures them, and a rebuild
that survives all of it. 466 tests green, every gate passing, no daemon left running.

The reranker is the one component not built, and it is waiting on a question rather than on code
— see the top of this file.

**What is now true that was not:** a query finds documents that share no word with it, in either
language. The proof is in `cortex explain`, not in a claim: for *"how fast did storage expenses
grow after moving the archive"* the German answer comes back with **0 path candidates, 0 lexical
candidates, vector rank 3.**

Seven things worth knowing, most of which cost time:

1. **A benchmark that cannot fail is not a gate, and a goldset that cannot be wrong is not a
   measurement.** The first version of the cross-language queries asked a German query to find
   the English copy of a document whose German copy was *also in the corpus*. That measures
   obedience, not retrieval. Rewritten so every cross-language target exists in one language
   only — which dropped the score from 50 % to 33 % and made it mean something.

2. **The language bias is a number, not a feeling.** Same-language similarity sits about
   0.08–0.12 above cross-language similarity for this model, and on short texts that is more than
   the gap content opens up. `goldset --explain q43` shows the consequence plainly: twenty German
   noise documents ahead of the English one that answers the question. Recorded rather than
   worked around; the goldset is now the instrument for anyone who tries to fix it.

3. **A vector keyed by a row id is a vector you throw away.** `--rebuild-index` re-derives chunks
   from the filesystem, so the old rows — and every embedding that cascaded from them — went with
   it. Hours of inference to recover text that had not changed. Migration 0003 keys embeddings by
   the hash of the text (D26); a rebuild now keeps all of them, and identical boilerplate across
   files is embedded once.

4. **The first rebuild refilled the vector index with nothing, and nothing noticed.** The refill
   ran immediately after the walk — before extraction had re-created a single chunk — so it wrote
   an empty index over a full table. Semantic search then returned nothing at all, silently,
   because the embedding queue was *also* empty: every hash already had a vector. The index is now
   reconciled against the table by the embedding stage whenever its queue drains, which is the
   same path that satisfies Definition-of-Done §8. Confirmed live: deleting the index file and
   restarting rebuilt 122 137 vectors in 152 ms without the model.

5. **`view` maps a file and trusts its header.** Pointed at a truncated one it does not return an
   error — it segfaults, which the existing "a corrupt index is a rebuild" test found the moment
   loading became mapping. The header is now read with an ordinary file read first.

6. **Asking "is there anything to do" cost more than doing it.** With every chunk embedded, the
   embedding queue's left join scans all 122 137 chunks to find nothing, and doing that on a timer
   measured **0.84 % of one core** against a whole-daemon idle budget of 0.5 %. Gated behind a
   two-aggregate watermark that changes exactly when the answer could have: idle cost is now
   0.067 %.

7. **An instrument that can lie is worse than no instrument.** A goldset run that crashed
   part-way through embedding left a cache whose counts all matched — 1 197 chunks, 1 197 vectors
   — and which measured **34 % recall where the same corpus measures 91 %**. The completeness of
   a cache is now recorded explicitly, keyed by model and schema version, and written last; a run
   that does not finish leaves a cache that the next run refuses. The same crash also showed that
   usearch refuses a duplicate key rather than replacing it, so adding a vector for a chunk that
   already has one is now idempotent.

**On memory.** Loading the vector index put the daemon at 307 MiB against a 250 MB budget. Mapping
it (D27) splits the number honestly: 182 MiB of heap the daemon genuinely holds, plus 107 MiB of
mapped index the kernel can reclaim. The budget is restated as anonymous memory rather than
raised, and both numbers are in the table below.

**On the GPU.** D6 puts embeddings on the local GPU. The machine has the card and the driver but
no CUDA runtime, so the CUDA execution provider fails to register and the session falls back to
the CPU — reported in `cortex model`, not hidden. Every number in this session is therefore a CPU
number, and the reranker's 150 ms budget is unreachable until that changes.

Verified live against the reference corpus, with a real daemon:

- `cortex model fetch` downloads 487 MB, verifies both checksums, and a daemon that was already
  running picks the model up without a restart.
- 1 721 chunks embedded at 89 ms each; the whole corpus of 122 137 chunks at the idle budget.
- `cortex dump` before and after `cortex rebuild` are byte-identical over 1 005 lines, with all
  1 721 vectors kept and the model never asked for anything (rule R10, Definition-of-Done §7).
- Idle CPU 0.067 %, anonymous memory 182 MiB with 100 000 files, 122 137 chunks and 122 137
  vectors indexed.

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
| 2026-08-18 | M2 | Vector ANN top-100, 122 137 vectors | P95 1.01 ms | 40 ms | yes |
| 2026-08-18 | M2 | Embedding one query (CPU, 8 threads) | P95 4.33 ms | — | — |
| 2026-08-18 | M2 | Embedding one chunk (CPU: 1 / 8 / 16 threads) | 279 / 75 / 86 ms | — | — |
| 2026-08-18 | M2 | Recall@10, goldset, fused | 91.0 % | recorded | — |
| 2026-08-18 | M2 | Recall@10, goldset, lexical arm alone | 36.0 % | — | — |
| 2026-08-18 | M2 | Recall@10, German against English | 87.0 % / 95.7 % | within 10 % | yes |
| 2026-08-18 | M2 | Re-index of an unchanged corpus, embedder calls | 0 | 0 | yes |
| 2026-08-18 | M2 | Vector index rebuilt from the store, 122 137 vectors | 152 ms, no model | — | — |
| 2026-08-18 | M2 | `cortex rebuild`, canonical dump before/after | identical, 1 005 lines | identical | yes |
| 2026-08-18 | M2 | Idle CPU, 122 137 vectors indexed | 0.067 % of one core | 0.5 % | yes |
| 2026-08-18 | M2 | Idle anonymous memory, 100 k files + 122 137 vectors | 182 MiB | 250 MB | yes |
| 2026-08-18 | M2 | Idle total RSS, same daemon after searching | 299 MiB (107 MiB mapped) | see D27 | — |
| 2026-08-18 | M2 | `cargo test --workspace` | 466 passed, 0 failed | green | yes |

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

### M2 acceptance

The Definition of Done is in `docs/01-BUILD-PROMPT.md`. Every item except the reranker's budget is
now measurable, and the one that is not says why.

| Criterion | Result |
|---|---|
| BM25 top-100 under 30 ms | 14.7 ms P95 |
| RRF fusion + salience boost under 5 ms | 12.8 µs P95 |
| Re-indexing an unchanged corpus re-embeds zero chunks | verified live over 100 000 files: 0 chunks written, every file recognised by hash |
| Idle RSS below 250 MB with 100 k files indexed | 182 MiB anonymous with 122 137 chunks and 122 137 vectors (D27) |
| First index run stays inside the configured CPU share | 15.0 % of one core measured against a configured 15 % while the user is active |
| Every result carries a score breakdown, and `cortex explain` prints it | yes, including why a file did *not* appear |
| Content extraction only inside configured roots (R3) | verified live: a file outside them is recorded as skipped, with the reason |
| The deny-list holds before any content access (R1) | verified live: sensitive files are absent from the index entirely |
| Withdrawing content withdraws it from the index | verified live: renaming a file to a deny-listed name deletes its chunks |
| Vector ANN top-100 under 40 ms | 1.01 ms P95 over 122 137 vectors |
| Recall@10 on the goldset, measured and gated | 91.0 % fused, recorded in `baseline.toml`, a 3-point drop fails the run |
| German within 10 % of English | 87.0 % against 95.7 % — 9 % behind, inside the allowance |
| Re-indexing re-embeds zero chunks, by call counter | yes — `re_indexing_an_unchanged_corpus_asks_the_model_for_nothing` |
| `cortex --rebuild-index` reproduces a canonical dump | yes — byte-identical over 1 005 lines, live, with every vector kept |
| Vector index rebuildable from `embeddings` without the model | yes — 122 137 vectors in 152 ms, live |
| Cross-encoder rerank of the top 30 under 150 ms | **not built** — needs a second model and a protocol that can send a second answer; the budget assumes a GPU this machine cannot use yet |

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

Measured by `bench/goldset` against 29 judged documents and 1 000 generated distractors, with the
50 checked-in queries. The baseline in `bench/goldset/baseline.toml` fails the run on a drop of
more than three points against any row of the breakdown.

| Date | Model | Recall@10 (EN) | Recall@10 (DE) | Notes |
|---|---|---|---|---|
| 2026-08-18 | multilingual-e5-small, CPU | 95.7 % | 87.0 % | fused 91.0 % overall; by-name 100 %, lexical 100 %, semantic 98 %, cross-language 33 % |
| 2026-08-18 | lexical arm alone (control) | 39.1 % | 33.3 % | 36.0 % overall — what the index answers with no model at all |
| 2026-08-18 | hashing embedder (control) | — | — | run `goldset` without `--model` to reproduce; it is a control, not a result |

German is 9 % behind English, inside the 10 % Definition-of-Done §3 allows, and the entire gap is
cross-language queries. See "What the goldset says" at the top for why, and for the caveat that
the distractors are linguistically German and semantically empty — the worst case for exactly
that measurement.
