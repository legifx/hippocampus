# Review 2026-09-05 (second pass) — a code-level review against the original goal

**Commit reviewed:** `8ae1ff8` (the first review of this date, at `bbef90d`, is its parent).
**Reviewer:** Claude, in session with the owner.
**Companion files:** `2026-09-05-code-review-evidence.json` (every number below),
`2026-09-05-review.md` (the first pass — **read it first**, this paper does not repeat it).

> **Kurzfassung für den Besitzer.** Dieser zweite Durchgang liest den Code und misst selbst,
> statt die Dokumentation zu glauben. Ergebnis: die Architektur hält, die Latenz-Gates sind
> reproduzierbar grün, das Agent-Gating funktioniert am lebenden Daemon (ein Schreibversuch
> wurde verweigert und erzeugte keine Datei). **Aber drei Behauptungen der vorhandenen
> Dokumentation sind falsch, und zwei davon sind falsch im selben Moment, in dem das erste
> Review sie als grün notierte:** die Rust-Suite ist nicht grün (2 Tests rot), die Unit-Suite des
> Frontends ist nicht grün (28 von 56 rot, abhängig von der Umgebung), und es gibt **keine CI**,
> die irgendetwas davon ausführen würde. Dazu kommt ein echter funktionaler Befund: genau die
> Fähigkeit, die Ihr eigenes Beispiel verlangt — zwei gleichnamige Dateien unterscheiden —, ist
> nicht als Test festgehalten und hat keine Seitenvergleichs-Ansicht. Der Speicher des Daemons
> ist mit 1 563 MiB weiter über dem Budget und seit dem Vormittag gewachsen.

---

## 0. How to read this, and how it differs from the first pass

The first review of this date established **what exists and what it measures**. This pass asks a
different question: **is the evidence true?** It re-runs every command, reads the code behind the
claims, and treats a divergence between a document and a measurement as a finding in its own
right — because a project whose self-description is wrong will eventually make a wrong decision
on the strength of it.

Three rules were applied:

1. **Nothing is accepted because a document says it.** Every number below was produced in this
   session, on the owner's machine, with the commands in §7.
2. **A red test is a finding, not noise.** Both failures found here were reproduced twice.
3. **The absence of a gate is a finding.** A budget nothing enforces is a comment.

Verdict scale: **met** · **met with a gap** (true for what was checked; a named check is missing) ·
**built, unproven** (code and tests exist; the criterion needs a human or real use) ·
**not met** · **contradicted** (the project asserts X; measurement or code shows otherwise).

---

## 1. The three claims that do not hold

These come first because they change how the other sections should be read.

### 1.1 `cargo test --workspace` is not green — it is 1052 passed, 2 failed

`docs/03-PROGRESS.md` (`Current position`) says **"1054 Rust tests (2026-09-05)"**. The first
review's evidence file records `rust: {passed: 1054, failed: 0}`. Measured today, twice:

```
test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out
error: test failed, to rerun pass `-p cortex-sync --test hygiene`
```

The two failures are both in `crates/cortex-sync/tests/hygiene.rs`:

- `the_public_repository_names_no_endpoint_no_private_path_and_no_credential`
- `the_entire_history_names_no_secret_and_no_private_identifier`

Both fail on the **same false positive**: the hygiene gate's endpoint pattern
(`scripts/pre-commit:93`) flags `https://www.debian.org` in `packaging/deb/build.sh:56` — the
Debian copyright-format URL, a documentation host — as "a host the Core must not know". The gate
has an exclusion list of documentation and registry hosts (`w3.org`, `ietf.org`, `freedesktop.org`,
`sqlite.org`, `example.com`, …) and `debian.org` is simply not on it.

Three things make this worth a section rather than a footnote:

1. **It is not new.** `git show bbef90d:packaging/deb/build.sh | grep -c debian.org` returns 1.
   The commit the first review recorded as 1054/0 **already contained this failure**. The first
   pass did not re-run the suite; it read the number out of the progress document. That is the
   exact failure mode `03-PROGRESS.md` warns about in its own "note on counting" paragraph —
   "check the measuring tool before disbelieving the measurement" — applied to itself.
2. **It is a false positive, not a leak.** Nothing sensitive is exposed. The fix is one entry in
   an exclusion list, not a security remediation. But a gate that cries wolf on every commit is a
   gate that gets `--no-verify`'d, and the script's own error text invites exactly that:
   *"If you are certain, commit with `--no-verify` — but understand that history is forever."*
3. **It is self-inflicted by an over-broad pattern.** Any `https://` URL with a dot in the host is
   an "endpoint" unless explicitly allow-listed. That is a deny-by-default rule over an
   allow-list, so every new documentation link is a future failure. Inverting the rule — flag a
   URL whose host is not on a known-good list **and** which appears in a `peer =`/`endpoint =`
   assignment — would keep the guarantee and stop punishing prose.

**Verdict: contradicted.** Fix: add `debian\.org` to the exclusion at `scripts/pre-commit:94`
(five minutes), or restructure the pattern as above (an hour, and it stops the class).

### 1.2 The frontend unit suite is not green — 28 of 56 fail as `npm test` is written

`docs/03-PROGRESS.md` says "60 vitest". The first review recorded `vitest_ui: {passed: 60,
failed: 0}`. Measured by running the documented command, `npm test`:

```
 Test Files  5 failed | 2 passed (7)
      Tests  28 failed | 28 passed (56)
```

Every one of the 28 failures is the same error:

```
act(...) is not supported in production builds of React.
```

The cause is environmental and **not a defect in the code under test**: this shell exports
`NODE_ENV=production`, which makes Vite resolve React to its production build, and React Testing
Library's `act` is stripped from that build. Re-running with the environment set to the value the
tests were written for:

```
NODE_ENV=test npx vitest run   →  Test Files 7 passed (7) · Tests 60 passed (60)
```

So the suite is genuinely green under the right environment — but **the command the project
documents and the number the project publishes are only correct if the caller happens to have
`NODE_ENV` unset or set to `test`**. `ui/vite.config.ts:45` configures the vitest block and sets
nothing, and `ui/package.json` defines `"test": "vitest run"` with no environment. Nobody made a
mistake here; the gap is that the dependency on ambient state was never made explicit.

Two consequences worth naming:

- A contributor (or an agent) whose shell exports `NODE_ENV` — common on machines that build
  release bundles — sees 28 red tests and has no signal that the fault is not theirs.
- Any future CI would inherit whatever the runner exports. Since there is no CI (§1.3), nothing
  has ever pinned this down.

**Verdict: met with a gap.** The tests pass; the invocation is not self-contained. Fix: set
`process.env.NODE_ENV = "test"` in `ui/vite.config.ts` (or `"test": "NODE_ENV=test vitest run"` in
`package.json`). Five minutes, and it removes a trap that costs a newcomer an afternoon.

### 1.3 There is no CI

`docs/01-BUILD-PROMPT.md:93` states: **"These are CI gates. Write the benchmark before the
implementation. A milestone is not done while its benchmarks are red."** `CLAUDE.md` rule 3:
**"Latency budgets are the specification. The numbers in `docs/01-BUILD-PROMPT.md` §Benchmarks are
CI gates, not aspirations."**

There is no `.github/`, no `.gitlab-ci.yml`, no pipeline configuration of any kind in the
repository. `cargo test`, `npm test`, the Playwright suites and `bench-latency` all run **only when
a human or an agent remembers to type them**.

This is the structural finding of this review, and it explains the other two: a gate that nothing
runs is a gate that can be red for an unknown length of time without anybody noticing — which is
precisely what happened to the hygiene gate. It also means every "done" in the milestone table is
attested by a session's memory rather than by a machine.

The project is aware of the shape of the problem — `crates/cortex-sync/tests/hygiene.rs:6` says
"Running it from `cargo test` makes it a gate wherever the tests run, which is what 'in CI' means
for a project whose CI is its test suite." That is a reasonable stopgap for the hygiene gate. It
does not cover the latency budgets, which no test asserts at all: `bench/latency` is a separate
binary that must be run deliberately, and its own gate table (§4) is printed and exits non-zero,
but nothing invokes it.

**Verdict: not met, and it is the root cause of §1.1 being invisible.** A single workflow file
running `cargo test --workspace`, `cargo clippy`, `cargo fmt --check`, `npm test` and
`cargo run --release -p bench-latency` would close this. The latency job needs an idle machine to
be meaningful, so it wants either a self-hosted runner or a `workflow_dispatch` job rather than
on-push.

---

## 2. The human: find the right file fast, see everything, verify it is the right one

### 2.1 What holds

The first pass found this goal **met with one gap**. This pass confirms the substance and adds
code-level evidence:

- **Virtualisation is real.** `ui/src/components/ResultList.tsx:65` uses a purpose-built
  `useVirtual` hook (`ui/src/hooks/useVirtual.ts`, 16 kLOC-adjacent at 15 969 bytes, 12 unit
  tests) rather than a dependency. The perf suite's bound is a constant chosen by a person
  (`perf.spec.ts`, `RENDER_BUDGET_MS = 24`), not whatever the code happens to produce.
- **One row shape from every source** (M10 step 1) is genuinely implemented: `ui/src/row.ts`
  maps a `ViewRow` from the daemon with no per-source branching, and the component comment
  records that two mappers used to disagree. That is the right fix for the right problem.
- **The preview pane makes a security decision, not a rendering one.** `PreviewPane.tsx:1-16`:
  text goes through `fs.read` so the deny-list and audit apply; pixels go to `cortex-preview`, a
  separate killable process. The comment says why, and the reason is correct — a malformed image
  should cost a helper, not the window.
- **Keyboard coverage is broad**: Enter, arrows, Page keys, Home/End, Escape, Ctrl+K (palette),
  Ctrl+D (density), Ctrl+C/X/V/Z/N, Ctrl+E (extract / eject), Ctrl+Shift+A (Inspector),
  Ctrl+Shift+N. Sorting by name/size/date/salience via the palette (`App.tsx:498`).
- **The breadcrumb is right about elision**: truncated from the left, segments stay clickable
  when elided (`Breadcrumb.tsx:1-6, 33-34`). Most file managers break one of those two.

### 2.2 The gap the owner's own example names

> "ist jetzt content.png oder content(1).png das richtige?"

`grep -rniE "content\(1\)|disambigu|same.name|duplicate"` over `ui/tests/` and `ui/src/` returns
**nothing**. There is no test that puts two same-named files side by side.

What the code does provide, and why it is probably sufficient but unproven:

- Rows carry `dir` (the directory, suppressed when every row shares it — `row.ts:70`), `size`,
  `mtime`, `mode`, `owner`, `linkTarget` and a `broken` flag.
- The preview pane header shows size and age (`PreviewPane.tsx`, `hc-preview-head`), and the
  picture path renders at 512 px × DPR.
- `brain.diff` exists on the agent surface and could back a human "compare these two".

So the information is on the row. What is missing is:

1. **A test.** `disambiguation.spec.ts` — two files of the same base name in different
   directories, one an image; assert the rows show different directories/sizes/ages and the
   preview shows the selected one. The first review already recommended this (§5.3) and it is
   still not there. It is ten minutes and it makes the owner's own example a regression gate.
2. **A side-by-side compare.** Nothing in the window calls `brain.diff`. For the exact case the
   owner describes — two images with nearly identical names — reading a directory string is
   slower than looking at both. This is the one place where the human surface has a tool in the
   daemon it does not offer.
3. **Quick Look.** There is no spacebar preview, no full-screen zoom. The preview pane exists but
   is a pane; `PREVIEW_PIXELS = 512` caps image previews below the screen size, so "is this the
   right screenshot" is answered at 512 px.

**Verdict: met with a gap** (unchanged from the first pass, and the gap is now located precisely).

### 2.3 Two structural gaps in "see and survey everything"

- **There is no multi-column or details view.** The list is one line per file at every width.
  Size, date, permissions, salience and tier are all carried on that one line.
  `grep -rniE "columns|detail view|split|dual pane|miller"` across `ui/src` finds only the Agent
  Inspector's internal split and unrelated `splitPath` helpers. For a "human sees everything at a
  glance" goal, a single-line list with the density toggle as the only lever is the thinnest
  possible answer — and the row already carries more columns than a line can show legibly.
- **There is no grouping.** Sorting by name/size/date/salience exists; grouping by project, type
  or date does not. The daemon knows zones, tiers and concepts, all of which are natural groups.

Both are defensible as deliberate scope decisions (the owner's stated priority is "stability
before appearance"). Neither is recorded as a decision in `docs/02-DECISIONS.md`, so a future
session cannot tell "cut on purpose" from "never thought of".

**Verdict: met with a gap.**

### 2.4 Test reality: 159 browser tests run against a fixture, not a daemon

This is the most important methodological point about the human layer, and the first pass did not
surface it.

`ui/src/test/harness.tsx:2` — *"The app, wired to a fixture instead of a daemon."* Every
Playwright test navigates to `/harness.html`, which mounts the real `App` with
`setTransport(...)` replaced by an in-page fixture (`harness.tsx`, 700+ lines covering listings,
search, previews, conflicts, thumbnails, dry runs, dead shares).

This is a **good** decision, and it is honestly documented: the tests assert layout against a real
box model, they are hermetic, they run in milliseconds, and `perf.spec.ts:11-14` explicitly says
which share of the 80 ms budget it measures and warns that *"a test that measured a fixture and
reported it as the end-to-end latency would be worse than no test."*

The consequence is worth stating plainly, because the test count invites the wrong inference:
**159 passing browser tests prove the UI behaves correctly against a fixture that behaves the way
the authors believe the daemon behaves.** Nothing in the Playwright suite would notice the daemon
and the fixture disagreeing. The real-daemon coverage is elsewhere and thinner:
`ui/src-tauri/tests/against_daemon.rs` and the Rust integration tests. `architecture.spec.ts`
guards one specific real-world regression (the canvas measuring a still-animating container).

**Verdict: met with a gap** — the gap being that "159 browser tests" and "the human surface is
verified against the daemon" are different claims, and only the first is true.

---

## 3. The agent: the explorer as a brain and a database

### 3.1 What holds, verified rather than believed

- **29 MCP tools**, enumerated from `crates/cortex-mcp/src/tools.rs` (`TOOLS`, a single
  hand-maintained table). **65 RPC methods** enumerated from `crates/cortex-rpc`. The 29 are a
  deliberate subset, and the table's doc comment explains the constraint that matters:
  *"There is no per-tool code — a tool that needed some is a tool whose logic has escaped the
  daemon, and then the three surfaces would no longer share one implementation"* (`tools.rs:5-6`).
  That is the right architecture, stated as an invariant.
- **Three-surface parity is tested, not asserted.** `crates/cortexd/tests/three_surfaces.rs`
  boots a real daemon and compares answers over the Unix socket, the CLI and MCP stdio for a
  stat, a listing, places, open-with, and — importantly — a *gated* tool
  (`a_gated_tool_plans_identically_on_all_three`, line 360).
- **The gating works on the live daemon.** This pass tried it rather than reading about it:

  ```
  $ cortex --as reviewer write /tmp/hc_probe.txt "hello"
  Error: daemon returned an error: the write allow-list is empty, so no filesystem
         mutation is permitted (code 1002)
  $ ls /tmp/hc_probe.txt → No such file or directory
  ```

  Refused with a code, no file created.
- **The red-team suite is serious.** `crates/cortex-agent/tests/red_team.rs` (766 lines, ~45
  tests) covers refusal outside the allow-list, missing plan ids, made-up plan ids, the surface
  being off, **plan replay** (`a_plan_cannot_be_replayed`), **plan reuse on a different file**
  (`a_plan_for_one_file_cannot_be_spent_on_another`), **staleness when the target changes**
  (`a_plan_goes_stale_when_the_target_changes_underneath_it`), symlink escape, and the deny-list
  not being lifted by the allow-list. That is TOCTOU and confused-deputy coverage most tools do
  not write.
- **The audit log is live and working**: `cortex audit` showed this session's own calls
  (`agent.sessions`, `brain.memories`, timestamps, actor `human`, parameters) — the log records
  reads, not only mutations.
- **Reconstructability is tested**: `embed_stage::tests::the_memory_collection_rebuilds_from_the_table_without_the_model`
  passed in the run above, alongside `three_surfaces`' rebuild replay.

### 3.2 The parity risk nobody has gated

`crates/cortex-mcp/src/tools.rs` is a **hand-maintained table** of 29 entries mapping
`name → method`. The RPC registry has 65 methods. There is no test asserting that:

- every method in `TOOLS` names a method the daemon actually implements, and
- every tool an agent needs has a row.

The first direction fails loudly (the daemon would reject the call). The second fails **silently**:
a new brain tool added to the daemon with no row in `tools.rs` simply does not exist for an agent
using MCP, while working fine from the CLI and socket. `three_surfaces.rs:287`
(`the_session_reports_the_tools_it_actually_has`) checks that a *fixed list* of required names is
offered, which guards against deletion, not against drift.

**Verdict: met with a gap.** A test that walks `cortex_rpc`'s method table and asserts each
mutating method is either in `TOOLS` or deliberately excluded (with the exclusion named) is
perhaps thirty lines and closes a class of silent divergence.

### 3.3 The brain has still never been used

```
$ cortex memories --json   →  {"memories": [], ...}
$ cortex sessions          →  no agent has opened a session on this daemon
```

Unchanged from the first pass: **0 memories, 0 sessions**, after 1 h 49 m of daemon uptime in this
session's case. `brain.context` was exercised live and works — it returned a pack in tens of
milliseconds with a `basis` on each item naming the arm, the actor and the confidence:

```
pack_id "pack-1788631281-reviewer-e1f36839" · budget 2048 · used 393 tokens
items: working_set arms with basis {confidence 1.0, actor "human", written_at …}
```

It is fast and it is honest about provenance. It is also answering from an empty memory layer —
every item above is a `working_set` file, because there are no memories to rank.

So M11's twelfth criterion (a real task twice, measurably better the second time) remains the
single open question that decides whether the most novel half of this project is worth its budget,
and it cannot be answered by writing more code.

**Verdict: built, unproven** — and the unproven part is the part that matters.

---

## 4. Performance: budgets, and what they actually constrain

### 4.1 The gates, reproduced

`cargo run --release -p bench-latency -- --corpus bench/corpus/data/standard`, 100 000 paths,
122 137 chunks:

| gate | measured (P95) | budget | state |
|---|---|---|---|
| fuzzy path match (literal, tier one) | 1.19 ms | 10 ms | PASS |
| keystroke path (every prefix, incl. 1–2 chars) | 4.35 ms | 10 ms | PASS |
| abbreviation (full-scan tier) | 4.29 ms | 25 ms | PASS |
| BM25 top-100 | 23.20 ms | 30 ms | PASS (tightest) |
| BM25 while the indexer writes | 26.86 ms | 34.80 ms | PASS |
| vector ANN top-100 | 0.83 ms | 40 ms | PASS |
| RRF fusion + salience boost | 0.015 ms | 5 ms | PASS |
| context pack, 2 048 tokens, 5 000 memories | 9.65 ms | 120 ms | PASS |

Also printed: worst-case stopword query P95 92.2 ms; hybrid search both arms with snippets P50
21.9 / P95 27.2 / P99 29.2 ms against the 80 ms keystroke→row budget; path index 50.2 MiB for
100 000 paths; index construction 146 ms.

The harness itself is better than most: it warms up before sampling (`main.rs:585`), sorts and
takes true percentiles (`main.rs:597-600`), asserts every benchmark query actually matches
something so it cannot measure an empty index and call it fast (`main.rs:107-111`, `186`,
`499-514`), and exits non-zero when a gate fails (`main.rs:573-575`).

### 4.2 Three things about the gate that deserve criticism

1. **One budget is derived from the measurement it is supposed to constrain.**
   `main.rs:287`: `budget: b95 + b95 / 2` for "BM25 while the indexer writes", where `b95` is the
   P95 measured in this same run with nothing else running. The comment defends this on the
   grounds that a budget derived from the same run cannot be confounded by a corpus that grew —
   which is true, and also means **that gate can only fail if contention makes BM25 50 % slower
   than it was thirty seconds earlier in the same process.** A regression in baseline BM25 raises
   `b95` and therefore raises the budget that gates it. This is one gate the suite cannot fail on
   its own terms. It is a valid design for a *relative* contention check and should be labelled
   one rather than sitting in a table of absolute budgets.
2. **Two budgets in the specification are not measured at all.** The run prints:
   `query embedding not measured: set HIPPOCAMPUS_TEST_RUNTIME and HIPPOCAMPUS_TEST_MODEL` and
   `reranking not measured: set HIPPOCAMPUS_TEST_RERANKER`. So the query-embedding path and the
   150 ms cross-encoder rerank budget (§Benchmarks, `01-BUILD-PROMPT.md:104`) have **no number**
   on this machine. The daemon has 160 996 embeddings and `cortex model` is populated, so the
   semantic arm is live in production while being absent from the gate that would catch it
   regressing. `docs/01-BUILD-PROMPT.md:93` says a milestone is not done while its benchmarks are
   red; these are not red, they are unrun, and the distinction is invisible in any summary.
3. **The 80 ms keystroke→row budget is explicitly not gated.** `main.rs:465` prints
   *"the M4 keystroke budget is 80 ms; not gated until then"* and the hybrid number is reported
   rather than asserted. This is honest and documented — but it means the one budget a user
   actually feels is assembled from three separately measured parts (daemon, IPC, render) that
   nothing ever measures together end to end.

### 4.3 Memory: still over budget, and it grew

| what | first pass (morning) | this pass | budget |
|---|---|---|---|
| daemon `RssAnon` | 1 524 MiB | **1 563 MiB** | 250 MiB |
| daemon `VmRSS` | 1 789 MiB | 1 682 MiB | — |
| `RssFile` | 255 MiB | 108 MiB | reclaimable |

Attribution by mapping:

```
1061 MB [anon]
 487 MB [heap]
   4 MB libcublasLt.so.13 (CUDA)
   4 MB libcudnn_graph.so.9
   2 MB libnvidia-gpucomp.so
   1 MB libcuda.so
```

The first pass identified this correctly: the anonymous total is dominated by the CUDA runtime's
host-side allocations, not by the index. Unchanged finding, slightly worse number, and it has
been growing across the day — worth watching for a leak rather than only for a budget breach. The
first pass's recommendation stands and is the right one: measure `RssAnon` with `device = "cpu"`
and with `"cuda"`, attribute it, then either restate the budget per configuration or release the
ONNX session when the embedding stage has nothing to do (the stage already runs on a duty cycle
per D22).

### 4.4 One measurement-hygiene finding

`content_roots = ["/home/user/Projects"]` (`~/.config/hippocampus/config.toml`) and the benchmark
corpus lives at `Projects/hippocampus/bench/corpus/data/standard` — **293 344 generated files**.
The daemon's live index therefore contains its own test data: `cortex status` reports 102 916
files, and a search for "report" returns `bench/corpus/data/standard/Projects/ember-164/assets/camera_857.py`
as the first hit, above anything the owner actually wrote.

Consequences: every live search number on this machine is partly a measurement of synthetic
Zipf-distributed content, and the working-set arm of a real context pack offers the corpus
database (seen in §3.3). It does not invalidate the benchmark suite, which uses its own isolated
store — but it does mean "the daemon's live index" and "the owner's files" are not the same set,
and any manual quality judgement made by looking at live results is being made against a corpus
that is 74 % generated.

**Recommendation:** exclude `bench/corpus` from `metadata_roots`/`content_roots`, or move the
corpus outside the indexed tree.

### 4.5 Verdict

**Met on every latency gate that is actually run; two budgets unrun; one memory budget exceeded
and drifting upward; no mechanism that runs any of it.**

---

## 5. Findings, ranked by what they cost

1. **No CI** (§1.3). Root cause of §1.1 being invisible. Makes every "done" in the milestone
   table an act of memory rather than a machine-checked fact. One workflow file.
2. **The hygiene gate is red on a false positive** (§1.1). Two tests failing at the commit the
   first review recorded as clean. The gate's own error message invites `--no-verify`, which
   would disable the public-repo protection entirely.
3. **The brain has never been used** (§3.3). 0 memories, 0 sessions. M11 §12 (a real task twice)
   is the criterion that decides whether the most novel half of the project earns its budget, and
   only the owner can close it.
4. **`npm test` is red as written** (§1.2). Environment-dependent; trivially fixable; costs the
   next contributor an afternoon of doubt.
5. **The daemon's anonymous memory is 6× its budget and rising** (§4.3). Not a lag. A false claim
   in the specification, in the configuration actually run.
6. **No same-name disambiguation test, and no side-by-side compare** (§2.2). The owner's own
   example is not a gate, and `brain.diff` exists in the daemon with nothing calling it.
7. **The benchmark corpus is inside the live index** (§4.4). 293 k synthetic files. Pollutes
   every live judgement and the working-set arm of every context pack.
8. **Two budgets are never measured** (§4.2): query embedding and the 150 ms rerank.
9. **One gate's budget is derived from its own measurement** (§4.2): the contention gate cannot
   fail on its own terms.
10. **MCP tool table drift is ungated** (§3.2). A new brain tool can be invisible to agents while
    working from the CLI.
11. **No multi-column/details view, no grouping** (§2.3), and neither is recorded as a decision.
12. **159 browser tests run against a fixture** (§2.4). Correct choice, honestly documented — but
    the test count does not attest daemon integration.

Carried over from the first pass and still open: drag-out to another application, clipboard
survival across hide, one week of exclusive daily use (M6 §3), Inspector CPU while streaming,
Inspector layout, the second person's timed install (M9 §1), the license (D13), the 3D view under
WebKitGTK software GL, and the Extension's bridge to a real peer.

---

## 6. What to do next, in order

The first five are all under an hour each and four of them are one-line-to-one-file changes.

1. **Add `debian.org` to the hygiene exclusion** (`scripts/pre-commit:94`). Makes the Rust suite
   green again. Consider inverting the endpoint rule so documentation URLs are not "endpoints"
   by default.
2. **Pin the test environment** (`ui/vite.config.ts` or `package.json`). Makes `npm test` green
   regardless of the caller's shell.
3. **Write the CI workflow.** `cargo test --workspace`, `cargo clippy -- -D warnings`,
   `cargo fmt --check`, `NODE_ENV=test npm test`, Playwright, and a `workflow_dispatch` (or
   self-hosted) job for `bench-latency` on an idle machine. Nothing else in this list stays fixed
   without it.
4. **Exclude `bench/corpus` from the indexed roots**, then rebuild or reconcile. The live index
   stops being 74 % synthetic.
5. **Add `ui/tests/disambiguation.spec.ts`**: `content.png` and `content(1).png` in different
   directories; assert differing directory/size/age on the rows and the preview showing the
   selected one. Ten minutes, and it makes the owner's example a regression gate.
6. **Attribute the daemon's anonymous memory** with `device = "cpu"` and `"cuda"`, per the first
   pass's §5.1, and open a decision on the 250 MiB budget.
7. **Use the brain for one real task through Claude Code, twice.** Closes M11 §12.
8. **Gate the MCP tool table** against the RPC registry.
9. Then, in the owner's order: M6 §3's week, the Inspector layout, the two manual checks, the
   Extension bridge.

---

## 7. How the numbers were produced

```sh
# §1.1 — the failing suite (run twice; both times 1052 passed, 2 failed)
cargo test --workspace --no-fail-fast 2>&1 | grep -E "^test result" \
  | awk '{p+=$4; f+=$6} END {print "passed:",p,"failed:",f}'
cargo test -p cortex-sync --test hygiene         # the two failures
git show bbef90d:packaging/deb/build.sh | grep -c debian.org   # → 1: present at the reviewed commit

# §1.2 — the frontend suite
cd ui && npm test                     # 28 failed, 28 passed
NODE_ENV=test npx vitest run          # 60 passed

# §1.3 — CI
ls .github/workflows                  # No such file or directory

# §3.1 — the agent gates, live
cortex --as reviewer write /tmp/hc_probe.txt "hello"    # code 1002, no file created
cortex audit | head
cortex -p cortex-mcp 2>/dev/null; grep -oE 'name: "[a-z_.]+"' crates/cortex-mcp/src/tools.rs | sort -u | wc -l   # 29
grep -rnoE '"[a-z_]+\.[a-z_]+"' crates/cortex-rpc/src/*.rs | grep -oE '"[a-z_]+\.[a-z_]+"' | sort -u | wc -l     # 65

# §3.3 — real usage
cortex memories --json; cortex sessions

# §4.1 — the gates
cargo run --release -q -p bench-latency -- --corpus bench/corpus/data/standard

# §4.3 — memory
p=$(pgrep -x cortexd); grep -E "RssAnon|RssFile|VmRSS" /proc/$p/status
awk '/^[0-9a-f]+-/{n=$6;if(n=="")n="[anon]"} /^Anonymous:/{a[n]+=$2}
     END{for(n in a)printf "%d MB %s\n",a[n]/1024,n}' /proc/$p/smaps | sort -rn | head

# §4.4 — corpus inside the index
find bench/corpus/data -type f | wc -l           # 293 344
cortex search "report" --json | head -c 300      # first hit is the synthetic corpus
grep -A4 '\[index\]' ~/.config/hippocampus/config.toml

# scale
for d in crates/*/; do find "$d" -name '*.rs' -not -path '*/target/*' -exec cat {} + | wc -l; done
```

---

## 8. Scale, for the record

| | value |
|---|---|
| Rust, 14 crates | ~68 000 lines across 166 source files |
| `#[test]` / `#[tokio::test]` attributes | 999 |
| tests executed by `cargo test --workspace` | 1 054 (1 052 passed, 2 failed) |
| TypeScript/TSX, `ui/src` | 8 991 lines, 36 files |
| `ui/src-tauri/src` | 2 864 lines, 7 files |
| `ui-agent/src` (3D view) | 2 088 lines, 8 files |
| Playwright specs | 13 files (`test:ui`), plus `perf.spec.ts` |
| vitest | 7 files, 60 tests (green only with `NODE_ENV=test`) |
| `TODO`/`FIXME`/`HACK`/`unimplemented!` across `crates/`, `ui/src`, `ui-agent/src` | **0** |
| `unsafe` blocks in `crates/` | 6, all FFI (`flock`, `setpriority`, `statvfs`, `getuid`) |
| largest files | `ui/src/App.tsx` 1 140 · `AgentInspector.tsx` 1 103 · `cortex-store` 11 590 |
| commits | 112+ between 2026-08-17 and 2026-09-05 |

Zero TODO markers across 68 000 lines is genuinely unusual and worth saying: this project does
not leave debris behind. The two red tests and the missing CI are not symptoms of sloppiness; they
are symptoms of a gate being trusted rather than run — which is exactly the failure the project's
own progress document warns about, in its own words, in the paragraph it keeps "because it cost a
session".
