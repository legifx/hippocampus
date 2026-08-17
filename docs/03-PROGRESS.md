# 03 — Progress

**This is the living state file. Read it first, update it last, every session.**
If you finish work without editing this file, the next agent starts from nothing.

---

## Current position

| | |
|---|---|
| **Active milestone** | M1 — Perception |
| **Status** | M1 largely done: watcher, coalescer, reconcile, path index, benchmarks. Three items outstanding (below). |
| **Last updated** | 2026-08-17 |
| **Blocked on** | nothing |

## Outstanding in M1

Everything else in the milestone is implemented and measured. These three are not:

1. **Zones (D15).** The `zones` table exists and `nodes.zone_id` is wired through the model and
   the protocol, but nothing populates them. Needs: root detection from XDG user directories and
   detected project roots, plus stable hue assignment. The GUI in M4 is the first consumer, so
   this is not blocking anything yet.
2. **`parent_id` threading.** Always `None`. The directory graph therefore has no edges, which
   `structural` queries will need in M2. `Store::set_parent` exists and is tested; the walker
   and the watcher simply do not call it.
3. **The 24-hour soak test.** The M1 definition of done requires bounded RSS and file
   descriptors across 24 hours of synthetic churn. Not yet written. It belongs in `bench/` as a
   nightly CI job, not in the unit suite.

## Then M2 — Meaning

Do not start M2 before the three items above are closed, or they never will be.

1. Classifier: MIME by magic bytes, never by extension.
2. Extractors with wall-clock and memory caps, in a way that cannot take the daemon down.
3. Structure-aware chunker with per-chunk content hashing — the largest single performance
   lever in the whole pipeline.
4. Embedder: `multilingual-e5-small` via ONNX Runtime, CUDA with CPU fallback (D6).
5. FTS5 (contentless, D16) and the usearch ANN index.
6. RRF fusion across trigram, BM25 and vector, then the async cross-encoder rerank.
7. The 50-query German/English goldset in `bench/goldset/`.

### Things M1 deliberately left rough

- `Store` still wraps one connection behind a mutex, so queries serialise against ingest
  writes. Replace with a reader pool plus a single writer in M2, when extraction makes the
  writer busy enough for it to matter.
- The path index is rebuilt wholesale after any bulk change (reconcile, directory rename,
  directory removal). Correct and simple; if it ever shows up in a profile, patch incrementally
  instead — but only then.
- The ingest loop polls on a 250 ms timer rather than blocking on the inotify descriptor.
  Measured idle cost is 0.000 % of one core, so this is not worth changing without a reason.

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

## Still open

| # | Question | Needed by |
|---|---|---|
| D13 | License for the public Core (recommendation: Apache-2.0) | M9 |
| — | Concrete peer protocol for federation | M7 |

---

## Session log

Newest entry on top. One entry per working session. Keep entries short and factual — what
changed, what was learned, what the next agent needs to know.

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

Path-search figures come from `bench-latency` against the standard corpus, release build.
Index, CPU and memory figures are from a live daemon; the index time is taken from the
daemon's own log rather than from an external poller.

Measured with a **debug** build; the release build will be faster. These are hand-timed runs,
not yet a benchmark harness — M1 must replace them with `bench/` so they become a CI gate
rather than a note.

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
| Watcher survives 24 h without leaking descriptors or memory | **not yet tested** — see outstanding items at the top |


## Goldset recall

Empty until M2.

| Date | Model | Recall@10 (EN) | Recall@10 (DE) | Notes |
|---|---|---|---|---|
| — | — | — | — | — |
