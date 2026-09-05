# 03 — Progress

**This is the living state file. Read it first, update it last, every session.**
If you finish work without editing this file, the next agent starts from nothing.

---

## Current position

| | |
|---|---|
| **Active milestone** | **M9 — Release**, built on 2026-09-05 (D96, D97) as far as code can build it; its scorecard is the first section below. M7 built the same day (D93–D95). M10 done (D85–D88). M11, M8 and M5 as before. |
| **Status** | **M9 is built as far as it can be without a second person: `cortex setup`, a `.deb` and an AppDir from scripts that need only what Debian ships, `docs/09-INSTALL.md`, and the hygiene gate run over every tracked file *and every patch in the history* under `cargo test` — see its scorecard. What waits on a human: the timed install by somebody else (§1), and the license (D13).** **M7 is built: the wire format, a derived outbox, two transports, a reference peer, and every one of its five criteria asserted by a test — see its scorecard below. What it does not have is a real peer, which is the Extension's to write as a `sync` plugin.** M10 landed in full on 2026-08-24 — all four steps: `view.open` with one row shape from every source (D85), the memory columns on every row — measured before they were built, and cheaper than the per-row queries they sit beside (D86) — and the places the index makes (D87). Step 2 is complete: its by-hand criterion was checked by the owner against their own index and the salience order held. M6's mandatory list is complete as of the same day: archives (`fs.extract`, D81/D82), thumbnails in the list (D83) and network locations (D84, staged — what is mounted is offered, mounting is not built). What is left of M6 is its own §3, a week of exclusive use by the owner. 1054 Rust tests (2026-09-05), 60 vitest, 159 functional browser tests and 12 perf (run separately, on an idle machine — see the step 3 entry).** **M11 is ten of its twelve criteria in — see its own scorecard below before anything else, and every benchmark gate is green including the new one.** M4 complete. M6's mandatory list is done; its acceptance now needs a week of use rather than more code. **M5 complete except its M7 half, and M8 on top of it:** salience with explainable components, decay, tiering, co-access edge formation, a consolidation pass with a human-readable report, `brain.graph`, and a three-dimensional read-only view of the result that runs live against the daemon and as a single self-contained HTML file. The semantic abstraction step is now built too (D68): concept nodes clustered from co-access and named from what their members' paths have in common. Only promotion into the sync outbox is left, and that belongs to M7. The embedding model is resident on the GPU and the vector arm of search is filling in. **A note on counting, kept because it cost a session:** this file once said 399 tests and accused the previous entry's 812 of being unreproducible. The 399 was wrong — the counting command piped `cargo test` through `tail -60` before summing, so it added up the last few test binaries and silently dropped the rest. The previous entry was right. Count by summing every `test result:` line. Check the measuring tool before disbelieving the measurement; this file already says that about latency, and it applies to counting. |
| **Last updated** | 2026-09-05 (M7 built — D93–D95; M9 built — D96, D97) |
| **Blocked on** | nothing |

## Start here

**If you are picking this up after the M11 session of 2026-08-24, read three things in this
order:** the M11 scorecard below (what is in and what is not), D74–D80 (why the agent brain is
shaped the way it is), and `docs/07-AGENT-TOOLS.md` (what an agent can actually ask for). The
two criteria still open both need the owner rather than more code: a real task done twice through
Claude Code, and thirty seconds with the cgroup snippet below while the Inspector is streaming.

**The owner's stated priority, in their words: everything stable and working first, the look of
the interface afterwards.** They are not happy with the appearance yet and have deliberately
deferred it. Do not spend a session on visual design unless they ask for it.

**Four things are waiting on the owner rather than on code.** Ask; do not assume, and do not
re-derive them by reasoning:

**Two of the four were answered on 2026-08-20 and acted on; two are still open.**

1. **Does the overlay render correctly, and what does it cost? — Both answered, and the second
   answer was not the one three sessions in a row expected.**
   `WEBKIT_DISABLE_DMABUF_RENDERER=1` stays, as a drop-in at
   `~/.config/systemd/user/hippocampus-overlay.service.d/dmabuf.conf`, because without it the
   glitches and black areas come back. **It costs nothing: 111% of one core with it, 111%
   without** — measured on 2026-08-21 with the window on screen and idle.

   What cost a core was the breath layer, and not for the reason D54 or D57 gave. Eight variants
   were measured: the renderer makes no difference, nor does the backdrop filter, nor the
   gradient, nor the size of the animated element — a three-pixel strip still cost 71%. **Any
   running animation recomposites the whole window every frame.** The heartbeat now runs only
   while the daemon has something to index, which is both the cheap answer and the honest one,
   and the visible idle window measures **4.8–5.1%** against 111% before, over four consecutive
   samples. D59 has the table.

2. **Does the system clipboard survive the window being hidden? — Still open, not tested.** Copy
   in the overlay, then `wl-paste --list-types` in a terminal. Wayland only lets a focused
   window take the selection; whether the compositor keeps it after the surface unmaps is not
   something the documentation settles. If it does not, `gtk_clipboard_set_can_store` or
   delaying the hide are the levers.
3. **Does a drag out of the window reach another application? — Still open, not tested.** The
   page offers `text/uri-list` and the browser tests prove that much; they run in Chromium and
   the overlay runs in WebKitGTK, so whether the webview forwards it to the compositor can only
   be found out by dragging a row into something.
4. **Is the scrolling right now? — Answered: close it.** Done, and measured: **the gap at
   420 px per frame is now zero**, for 42 rows in the DOM while moving that fast and 36 at rest,
   which is what it was before. The lead is taken from how far the previous frame actually
   travelled instead of being fixed, so nothing is paid while the view is still. D57 has the
   table, the three rules that make it safe, and the two tests that had to be repaired before
   any of it could be believed.

**How to answer a report of slowness.** Not by reasoning about it. The bridge logs any daemon
call over 50 ms and any preview over 50 ms, and every one of the five causes was found in the
journal:

```
journalctl --user -u hippocampus-overlay --since "-1d" -o cat \
  | sed -E 's/\x1b\[[0-9;]*m//g' | grep -E "slow call|slow preview"
```

Equal durations across different methods finishing together is a queue, not slowness. A single
call that is genuinely slow is a different problem. And check what the processes are actually
doing before believing either — the largest cause of all was a hidden window burning a core, and
no amount of reading the code would have shown it:

```
cg=$(systemctl --user show -p ControlGroup --value hippocampus-overlay.service)
a=$(awk '/usage_usec/{print $2}' /sys/fs/cgroup${cg}/cpu.stat); sleep 30
b=$(awk '/usage_usec/{print $2}' /sys/fs/cgroup${cg}/cpu.stat)
awk -v a=$a -v b=$b 'BEGIN{printf "%.2f%% of one core\n",(b-a)/1e6/30*100}'
```

Hidden and idle, that is **under a tenth of a percent**. Anything in the tens is the bug.
`ps`'s `%CPU` will not show it: that column is an average over the whole life of the process, so
a core burnt for the last minute of a four-hour run reads as almost nothing.

**And check the measuring tool before believing the measurement.** The first version of the
snippet above reported **0.0% for a process spinning a full core**, because `$!` had captured a
subshell and `utime + stime` is a process's own time and not its children's. It agreed with
every good result and could not be distinguished from one. Point it at `yes > /dev/null` first;
it must read 100%.

## Where M9 stands — built 2026-09-05, read this first

### Definition of Done for M9, honestly

| # | criterion | state |
|---|---|---|
| 1 | A second person installs it on a clean machine, indexes their home and searches, unaided, under ten minutes | **not done — needs a second person.** Everything they would use exists: the `.deb`, `cortex setup` (three questions, then the first index), `docs/09-INSTALL.md`. Record the time here when it happens |
| 2 | Running without any Main Brain is a documented, tested, first-class configuration | **met** — it is the default; `09-INSTALL.md` §3 says so; `cortexd/tests/federation.rs` asserts the daemon serves with federation on and its bridge missing |
| 3 | `git log -p` over the entire public history contains no secret and no private identifier | **met** — `scripts/pre-commit --history` runs the gate's patterns over every patch ever committed, and `tests/hygiene.rs` runs it under `cargo test`; the gate itself is verified against a seeded fake credential, a private home path and an endpoint, assembled at runtime so the test's own source is clean |
| 4 | Every benchmark gate is green on the release commit | **met for the daemon's gates, this commit, release build, idle machine:** fuzzy path 1.19 ms / 10; keystroke path 5.88 / 10; abbreviation 4.57 / 25; BM25 top-100 24.14 / 30, and 28.64 / 36.21 while the indexer writes; vector ANN 1.05 / 40; fusion 0.01 / 5; context pack 10.36 / 120. Query embedding and reranking were not measured (the test model variables were unset); the browser perf suite was not re-run this session — its twelve tests last passed on 2026-08-24 |

### Scope, item by item

| item | state |
|---|---|
| single binary + desktop entry; `.deb`; AppImage or Flatpak | `.deb` built and inspected (`packaging/deb/build.sh`, dpkg-deb only); AppDir built, AppImage when `appimagetool` is present; no Flatpak (D96) |
| onboarding: folders, model, first index with progress, under ten minutes | `cortex setup` (D97); a throwaway instance under `--root` went from nothing to indexed in under a second on an empty home |
| configuration separated into defaults, user file, extension | was already so: `Config::default()`, `config.toml`, `plugins/*.toml`; `cortexd --check` prints the effective result; documented in `09-INSTALL.md` §4 |
| README, installation, configuration reference, MCP tool reference, troubleshooting, limitations | README §Installing; `09-INSTALL.md` §1, §4, §5, §6; `07-AGENT-TOOLS.md` was already the MCP reference |
| pre-commit secret scanner and the CI gate from R9, verified against a seeded fake secret | `--all` and `--history` modes; three tests in `cortex-sync/tests/hygiene.rs` |
| license decided (D13) | **open — the owner's.** `Cargo.toml` has declared Apache-2.0 since M0 and the package's copyright file repeats it, but there is no `LICENSE` file, and adding one is the decision, not a formality |

## Where M7 stands — built 2026-09-05

**The shape.** Nothing writes to the outbox. A worker *harvests* it: `episodes` from a cursor, the
concept nodes by identity, each into an event whose identifier is a hash of this index's origin and
the row's identity — never its content (D93). One batch at a time goes through a transport that
knows no address: `file-drop` writes one document per event into a directory the peer already
watches; `plugin` starts a declared program once per batch and speaks JSON-RPC to it over stdio
(D95). The ack names what was accepted and what was refused; silence about a row means it goes
again; an unreachable peer counts against nothing and the worker backs off (D94). The peer may send
its taxonomy down inside the ack, and the Core stores the latest and shows its version.

`docs/08-FEDERATION.md` is the wire format. `hc-test-peer` is the reference peer, forty lines that
keep a set of ids and a log of arrivals and do nothing clever — the peer D18 assumes.

### Definition of Done for M7, honestly

| # | criterion | state |
|---|---|---|
| 1 | Offline, then a clean reconcile: no duplicates, no lost events, no reordering damage | **met** — `tests/federation.rs`: 72 hours down at one event per 30 s, 8 640 events, every one arrives once, in Lamport order, and Lamport order is history order |
| 2 | Replaying the entire outbox twice changes nothing on the peer | **met** — the peer's knowledge (ids, payloads, clocks on first sight) is byte-identical after two replays; only its arrival log grows, which is the peer *observing* the duplicate its knowledge did not absorb |
| 3 | 72 hours of events sync without exceeding the configured rate or the memory budget | **met** — never more than `batch_size`, never closer than `batch_interval_ms`, asserted per batch on a synthetic clock; the test process grows by under 64 MiB draining the backlog, and the worker holds at most one batch at a time by construction |
| 4 | A grep of the public repository for endpoint patterns, private paths and credentials returns nothing, in CI | **met** — `scripts/pre-commit --all` gained an endpoint pattern (a URL whose host is a real domain outside a short public list), and `tests/hygiene.rs` runs it under `cargo test` |
| 5 | With the Extension absent every Core test passes and the daemon starts normally | **met** — `cortexd/tests/federation.rs` starts the daemon with federation on and the plugin missing; it serves, and `sync.status` says why nothing moves |

**Not done, and not the Core's to do:** the bridge to a concrete Main Brain. It is a program that
reads one `sync.push` request and answers one ack, declared in `plugins/<name>.toml` in the private
Extension. `hc-test-peer push` is the shape to copy.

### What was tried that did not work

- **Deleting a row on ack.** The first concept sent went out again on the next tick, because a
  concept is harvested by identity and a missing row is a new identity. Migration 0008 adds
  `acked_at` and `kind`; an ack marks (D94).
- **Counting an unreachable peer as an attempt.** The schema's partial index dead-letters a row at
  a hundred attempts; a weekend offline at a one-second interval would have buried the whole
  outbox in two minutes. Attempts count refusals only.
- **A 72-hour test at one event every ten seconds** ran 38 seconds: 871 unreachable ticks each
  starting a process, and a reference peer that rewrites its whole state per batch. Halving the
  rate and raising the backoff ceiling in the test to a quarter of an hour brought it to six
  seconds without changing what it proves. `publish` also stopped counting the outbox on ticks
  that changed nothing, which is correct as well as cheaper: nothing else writes that table.

### The numbers

| what | value |
|---|---|
| `Worker::tick` on a waiting tick, 8 640 pending | one query for new episodes, nothing else |
| 8 640 events drained through the plugin transport | 18 batches, 18 synthetic seconds, 4.6 real seconds including the peer rewriting 1.6 MB of state per batch |
| Rust tests added | 6 outbox, 22 sync (unit), 4 federation, 1 hygiene, 3 daemon, 3 plugins, 1 config |

## Where M8 and M5 stand

**The order was forced by the data.** M8 draws concept nodes, salience, tiers and consolidation
events. Before this session the real index had **24 edges, `salience = 0.0` on all 108 000 nodes
and every node in `stm`**, because M5 had not been built. A three-dimensional view of that is a
directory tree with extra steps, so M5 came first — see D62 for what was built and what was
deliberately left.

### Abstraction: the half that was missing

D62 recorded that M5 shipped without step 2 of the consolidation loop, and every report printed
the omission. It is built now. Label propagation over the `co_access` edges gives communities;
a community of three to sixty files becomes a **concept node**, named after the terms its
members' paths share and the rest of the index does not, with `derived_from` edges to its
members and a salience taken from its three best ones. D68 has the reasoning, including why
there is no language model in it.

**Three things this step taught, all of them by failing:**

- **A name is an identity, so it has to be unique** — `upsert_concept` keys on it. Two attempts
  at disambiguation produced `kettle (5)` three times and then `kettle #1230`, which is unique,
  stable and meaningless. A cluster with no term the others lack is now **not named at all**.
- **The index root is not vocabulary.** The first concept the real index produced was called
  `axon · legi · projects`. The tokens of the longest common prefix of every indexed path are
  excluded now, and it is called `axon`.
- **Two SQL shapes fail without an error** — `json_extract` on a column that is not JSON, and
  `NOT IN` over an empty list. D69.

On the real index this produces one concept, `axon`, over 16 files. That is not a disappointing
number: there are 39 co-access edges in an index five days old whose episode log is mostly the
watcher's own modify events. The simulator, with sixty days of invented human sessions, produces
eight.

### What two generations of somebody else's memory taught this one

Two systems by the same owner, one the successor of the other, were read for what a memory gets
wrong in practice. **The first — file-based, curated, a year old, 1 147 notes — is the abandoned
generation.** The second is a running daemon with two tiers, a vector store and an LLM curator,
and its opening design sentence is this paper's central claim arrived at independently: *Markdown
is the single source of truth, everything else is reconstructible derived state.* Five findings
are now code here. D70–D73; §9a and §10.5 of the paper.

- **Copies outrank originals.** Its search is `grep -r` including backup directories, so a query
  returns the same note from three backup generations before the canonical one. `noise_score`
  now penalises `backup`, `snapshot`, `archive`, `old` as path segments.
- **Orphans and byte-identical twins** are now counted in every consolidation report. The orphan
  count is restricted to nodes somebody has *touched* — without that it reports 102 584 on this
  machine, which is true and useless; with it, 1 468. Duplicates: 24 408 of 108 342, 22 %.
- **The older one's forgetting mechanism has never fired.** Seven identical weekly reports saying
  "none", beside a health file listing 75 orphans, because the prune criteria are four conditions
  with AND and its own intake gate makes two of them unreachable. Report-only anyway. That is the
  correct response to a store that is authoritative — which is exactly why this index is a cache.
- **A threshold sitting on a reachable maximum, and the same bug latent here.** The successor's
  demotion threshold equalled its recency weight, so every unused note sat on the edge by
  construction and its short memory ran at one note of eighty. Here, a file merely *opened* once
  reached 1.676 against a Working threshold of **1.60** — four per cent of clearance, and no test
  would have caught the day somebody lowered the recency weight. Threshold now 1.15, and
  `single_touch_ceiling()` plus two invariant tests hold the numbers up against each other rather
  than against cases. **D72, and it is the most valuable thing in this whole section.**
- **A collapse must not be committed.** The successor lost 939 of ~4 900 derived edges twice in
  one day. `write_concepts` here had the same shape — it deleted every concept a pass did not
  propose. It now refuses when a pass proposes under half of what exists. D73.

### What M5 does now

`cortex consolidate [--dry-run]`, `cortex report`, `cortex graph`. One pass, in the order the
components depend on each other:

1. **Replay** the episode log — the whole of it, every pass, so the derived graph is a pure
   function of the log rather than of how often consolidation happened to run (D65).
2. **Associate.** Bursts of activity become `co_access` edges. A burst touching more than twelve
   nodes is discarded whole: that is a build, not a person, and letting builds form edges turns
   the graph into a hairball in which everything relates to everything.
3. **Score.** Seven bounded components, weighted, summing to a number that can explain itself.
   Stored breakdowns sum to the score actually used, and there is a test that says so.
4. **Abstract.** Communities become concept nodes, before scoring, because a file that belongs
   to a named concept is more connected than one that does not and centrality is a component.
5. **Tier.** Promotion is immediate, demotion is one tier per pass, pins are exempt rather than
   heavily weighted.
6. **Report.** Including a `not done in this pass` section, which now names the one honest
   remaining limit — the labels are lexical, not written by a summariser.

Measured on the real index — 108 139 nodes, 9 500 episodes: **dry run 109 ms, full pass 1 516 ms.**
On the simulator — 3 069 nodes, 31 120 episodes: **200–240 ms.**

### Two bugs that only real data could find

**Episodes outlive the files they are about; `edges` has a foreign key.** `episodes.node_id` is
deliberately not a foreign key (R10) so that the log survives a deletion. Deriving an association
between two nodes that have since been deleted therefore fails the entire transaction with
`FOREIGN KEY constraint failed`, and the first real consolidation did exactly that. The replay
query now joins `nodes`. The simulator could never have found it — nothing in it is ever deleted.
Regression test: `an_episode_about_a_file_that_no_longer_exists_does_not_break_the_pass`.

**A count that never decays means nothing is ever forgotten.** `w1·log(1+access_count)` is
monotonic, so any file ever touched keeps a permanent floor of salience, and in the first
thirty-day simulation **not one node ever reached Cold**. Both counts are now age-weighted folds
using the specification's own decay law. See D63 — the law was already written down, in a
different paragraph from the sum, and the two had never been connected.

**A fourth, found while measuring the third criterion above.** `brain.graph --limit 50000`
produced a reply of about 19 MB. The transport is newline-delimited JSON with an 8 MB line limit,
so the daemon could not write it — and the concurrent response path discarded the write error, so
the caller waited for an answer that was never coming. Both processes sat at **0.0 % of one core**,
which is what a deadlock looks like from the outside and is a worse failure than an error because
it does not localise itself. Two fixes, at two layers: a response that will not fit now comes back
as `RESULT_TOO_LARGE` naming its size and telling the caller to ask for less, and `brain.graph`
caps `limit` at 15 000, which is where a real index's answer still fits (20 000 nodes measured at
7.55 MB). A request for 50 000 now returns 15 022 nodes in 0.27 s. Regression tests:
`a_reply_that_does_not_fit_comes_back_as_an_error` and its ordinary-case twin.

**And one thing a test found rather than a person:** `graph_snapshot` held a pooled connection
while calling helpers that ask for one. An in-memory store has exactly one connection, so it
waited for itself. No error, no CPU load, the suite simply stopped. Everything that needs its own
connection now happens before the long-lived one is taken.

### What M8 is

`ui-agent/` — its own Vite app, three.js, read-only, German by default. Hue is the zone,
brightness the tier, size the salience, halo the Working Set. Two deterministic layouts: the
containment tree, and concentric shells by tier in which a demoted node visibly moves outward.
**Play consolidation** replays the last pass in five captioned steps using the daemon's own
numbers — nothing in the animation is computed in the frontend (D4, D66).

It runs three ways: `npm run dev` against a snapshot, live against the daemon, and as one
self-contained HTML file produced by `scripts/bake.mjs` that needs no server at all.

### Definition of Done for M8, honestly

| # | criterion | state |
|---|---|---|
| 1 | consolidation is visually comprehensible | **met** — five captioned steps, the daemon's numbers |
| 2 | 5 000 nodes stay interactive | **met, measured** — see the table below. |
| 3 | read-only with respect to the filesystem | **met** — the app has no write path at all; `brain.graph` is a read |

### Criterion 2, measured

Headless Chromium with `--use-angle=vulkan`, which reaches the real GPU without opening a window
(the default and `--use-gl=egl` both fall back to SwiftShader — check the reported renderer before
believing any frame rate). The frame rate is pinned to 60 by the browser's own animation clock, so
the honest number is the **time spent inside each frame** against the 16.67 ms a 60 Hz frame has.
`requestAnimationFrame` was wrapped before the app loaded, so every frame it drew is timed from
the inside.

| nodes | idle p50 | rotating p95 | layout transition p95 | worst frame seen |
|---|---|---|---|---|
| 3 000 | 0.30 ms | 0.40 ms | 0.80 ms | 2.10 ms |
| 6 022 | 0.40 ms | 1.10 ms | 3.30 ms | 4.30 ms |
| 20 019 | 0.40 ms | 1.10 ms | 6.30 ms | 9.50 ms |

60 fps held in every mode at every size, including during the layout transition — the expensive
case, which rewrites every instance matrix and every edge vertex on each frame. At four times the
required node count the worst frame still used well under two thirds of its budget.

**What this does not measure:** the wrapper times the work inside the frame callback, which
includes issuing the draw calls but not the GPU finishing them. The frame rate holding at the cap
is the evidence for the GPU side.

### The explorer's next shape is written down now

`docs/paper/A-DIRECTORY-IS-JUST-ONE-QUERY.md` argues the restructure the window needs, and §M10 of
`docs/paper/BUILD-PROMPT.md` is its scope and Definition of Done. The short version: the row
renderer already does not know whether its rows came from a directory or from a search, and that
convenience is actually a statement about architecture — **a directory is one query among
several, not the ground truth of the view.** Following it gives concepts, the working set and the
cold tier the same navigation directories already have, in four shippable steps.

**It waited behind M6's mandatory list, and that list is now done** (2026-08-24: archives,
thumbnails, network locations). What is still open there is one built-but-unverified thing —
dragging a row *out* of the window — and M6's own §3, a week of exclusive use. The largest scope
risk in this project was that the memory features are more interesting to build than the file
manager's basics are; the basics went in.
`docs/paper/EXPLORER-KICKOFF-PROMPT.md` is the session prompt when it is time.

### What to do next here

- **The abstraction step (D62).** It needs `embeddings` to stop being 0, which needs the model
  the daemon has not been asked to load.
- **The grace window shows up as a flat legend.** A five-day-old index has every node in `stm`
  and the tier legend reads `working 0 / ltm 0 / cold 0`. The snapshot's `basis` now explains
  this in words, but it is worth watching whether the explanation is enough.
- M6's archive item is done (2026-08-24). What is left of its mandatory list: network locations
  and thumbnails in the list.

## Where M6 stands

**Browsing works.** An empty query shows a directory; typing replaces the list with results and
clearing brings the directory back. That is M6's shape — "search as an overlay, not a separate
page" — and it is one list rather than two pages, which is why the row renderer does not know
which mode produced its rows.

- Enter descends into a directory, Backspace goes up (only when there is nothing to delete),
  Alt+Left goes back through history, and the breadcrumb segments are clickable.
- **Ordering is the daemon's**: `fs.list` gained `sort` (name, size, modified, none) and
  `reverse`, with directories first in every order and the file name as the tie-break so two
  listings of one directory never disagree. A frontend that sorted would be the data processing
  D4 forbids.
- Entries the deny-list covers are counted in the status line and never named (R1).
- A file the index has not seen yet is marked `neu`, so "not there" and "not indexed" look
  different.

Nine browser tests cover it, including the 400 px case, because the breadcrumb is the new thing
that could push the window sideways.

### What works now

**Selecting.** Shift extends from an anchor that stays put, Ctrl toggles, Ctrl+A takes the
directory, Escape narrows back to one. The cursor and the selection are drawn differently
because they are different things. The selection is forgotten whenever the list changes, so
"delete these three" can never mean whatever landed in those three positions.

**Operating.** Ctrl+X / Ctrl+C / Ctrl+V, Delete to the trash, F2 to rename, Ctrl+Shift+N for a
folder, Ctrl+Z to undo. Copying shows a bar with a real denominator and Escape cancels it.

**Underneath.** `fs.transfer` answers with a handle and reports through notifications throttled
to ten a second; `fs.mkdir` and `fs.rename` answer immediately. Every one of them writes an undo
entry, and a batch writes one per source so a partly-failed batch is still undoable.

**No more crash at login.** Both services start together and the daemon's socket appears a
moment after the daemon does, so the overlay lost the race, aborted and was restarted by
systemd — a core dump in the journal at every login, for a race. It now waits for the socket,
and a run that waited reports `comparable=false` instead of a budget verdict, because that run
measured two services racing rather than this one starting (D51). §1 re-measured after the
change: 277 ms median, ten of ten inside the budget.

**The lag had one big cause and two small ones, and all of them are measured.**

The owner reported that everything felt slow. The journal answered it, because a previous
session had put the logging there for exactly this:

| call | n | avg | max |
|---|---|---|---|
| `extract.status` | 155 | 142 ms | **988 ms** |
| `search` | 5 | 417 ms | **932 ms** |
| `activity.ping` | 2 | 582 ms | 933 ms |

None of those calls is slow. They were waiting. What was found, in order of size:

1. **The overlay was burning a whole CPU core while hidden.** `WEBKIT_DISABLE_DMABUF_RENDERER=1`
   in the service unit drops WebKit to software compositing, and the breath layer then repaints
   a full-window gradient on the CPU at sixty frames a second — for a *resident* window that is
   off screen almost all of the time. Measured: 99% of a core hidden, 0% with the renderer
   enabled. The variable is gone, and the window now stops every animation and every poll while
   it is not on screen (D54).
2. **One SQLite connection behind one mutex** (D53), so every reader waited for the indexer,
   which is exactly what the WAL pragma three lines above it exists to prevent. The store now
   keeps a small pool. A new gate in `bench/latency` measures a search *while a writer works*,
   which no gate did before — every other one measures a store nothing else is touching, and the
   daemon is never in that state.
3. **A four-second poll, round the clock, for an animation nobody was looking at**, whose query
   scanned a hundred thousand rows because the only index on `extract_state` was a partial one
   built for a different question. Migration 5 adds the right one: 26.8 ms to 4.8 ms.

**What the new gate says.** Within one run, BM25 top-100 quiet against the same query while a
writer works: **+70% before the pool, +13% after**, and the writer got two and a half times as
much done in the same wall time because it was no longer fighting the reader. The *absolute*
numbers from before and after are not comparable, and saying so is the point: the first version
of that gate wrote changed rows into the benchmark corpus, so every run left the database
different from the one the next run would measure. It writes back the values already there now,
and two consecutive runs agree to within a millisecond. A benchmark that alters what it measures
is worse than no benchmark, because it produces numbers.

**A hazard the pool created, and the fix.** Two `batch` calls on two connections are two
writers. Rusqlite's default `DEFERRED` transaction begins as a reader and takes the write lock
on its first write — and SQLite then returns `SQLITE_BUSY` *immediately* rather than waiting,
because a reader that blocked while upgrading could deadlock. `busy_timeout` does not apply.
Every transaction is now `IMMEDIATE`, which takes the lock up front so contention is a wait; a
test runs four threads writing eighty times and asserts all eighty land.

**4. The RPC read loop answered one request at a time.** Left as a noted limit after the first
three were fixed — and then the owner said scrolling was still slow, and the journal gave the
signature exactly:

```text
18:39:22.084861  slow call method="search"          ms=152
18:39:22.085244  slow call method="brain.neighbors" ms=152
18:39:22.085253  slow call method="fs.read"         ms=152
```

Three different calls, the same duration to the millisecond, finishing within 0.4 ms of each
other. A queue, not slowness. Reads now run on their own tasks, eight at a time per connection,
from an allow-list so anything new stays serialised; writes keep their order because their order
means something (D55). The regression test was checked against the old behaviour and fails
there.

**5. And it still felt slow with the mouse, which was none of the above.** Wheel scrolling moves
no selection, so nothing was being asked of the daemon at all. Two things in the list itself
(D56): every scroll event re-rendered it, including the many that leave the same rows on screen;
and the spare rows were spread evenly on both sides, so a fast wheel painted a third of the
viewport empty and filled it a frame later. That reads as loading, which is what it was reported
as. Scroll events are coalesced onto a frame and compared before anything is stored, and the
spare rows now go where the eye is going — 300 px per frame went from 128 px of gap to none, for
two extra rows in the DOM.

**6. The last 72 px of that gap is closed too, and it cost nothing at rest (D57).** D56 left
420 px per frame showing 72 px of empty list, and the owner was asked whether to push further,
since the obvious way to do it is a bigger DOM. The lead is now **measured** rather than fixed:
it is sized from how far the previous frame actually travelled, capped at twenty rows, and given
back when the view stops.

| wheel speed | before D56 | after D56 | now |
|---|---|---|---|
| 150–300 px/frame | up to 128 px | none | none, 38 rows |
| **420 px/frame** | 240 px | **72 px** | **none, 42 rows** |
| 600 px/frame | 428 px | 260 px | 36 px, 46 rows — past the ceiling, by design |
| scrollbar jump | — | — | full height, 38 rows — a jump predicts nothing, so it borrows nothing |
| **at rest** | 36 rows | 36 rows | **36 rows** |

Three rules keep it bounded, and none of them is decoration — each was left out first, and each
broke something a test caught: a ceiling on the lead, a jump is not a scroll, and the lead
follows the *slower* of the last two frames so one frame alone can never buy rows. The settle
hands back the lead and deliberately not the direction. D57 has the reasoning.

**And two of the tests around this did not survive being checked.** The 420 px gap — the exact
defect the owner was asked about — **had no test at all**; the suite asserted 300 px, where D56
already reached zero. And "it comes back down when the scrolling stops" dispatched scroll events
while it waited, so those events did the settling: it passed with the settle deleted from the
code. Both are fixed, and every rule above was then verified by breaking it and watching a named
test fail. An existing bound assertion in `layout.spec.ts` that began failing only under parallel
load was not flakiness either — it was a real one-frame transient, and fixing the cause removed
it.

**It opens somewhere a person recognises.** The first screen is a list of places, not `/`:
Persönlich (home and the XDG user folders that exist, plus the trash), Lesezeichen (read from
the file Nautilus and Thunar use), Laufwerke (mounted volumes with their label and how full they
are), and System — one row, called "Dateisystem", holding everything technical. Built by the
daemon, so `cortex places` prints the same list and a test asserts all three surfaces agree
(D52). It is not the result list: you do not cut a drive, and a test says so.

**Cut and copy that the desktop can hear.** The system clipboard, in the four formats anything
reads: `x-special/gnome-copied-files` (the only one carrying the verb), `text/uri-list`,
`application/x-kde-cutselection` and plain text. Copy here, paste in Nautilus; cut in Thunar,
paste here, and it stays a cut. It lives in the shell rather than the daemon, which is headless
and has to stay that way (D49). A copy the compositor refuses — Wayland only lets a focused
window take the selection — still works inside the window and says so.

**Dragging.** Files dropped in from another application are copied into the directory being
looked at; a drop onto a result list is refused, because a result list is not a place. A drop
copies rather than moves, and D50 says why: the event carries neither the modifier keys nor the
source's filesystem, and a wrong guess in the move direction takes somebody else's file. Rows
offer `text/uri-list` when dragged out — the page's half is tested, the webview's half needs
trying against a real application.

**Saying what an entry is.** A listing now carries permissions, owner and group, and — for a
symbolic link — where it points and what is there. The rows show `drwxr-xr-x` in a wide window,
`name → target` for a link, and a struck-through target with `fehlt` when the target is gone.
The sidebar names the owner, and `cortex ls` prints the same three columns, in `ls`'s order.

Two details are load-bearing rather than cosmetic:

- **A link stays a link.** `kind` describes the entry itself; the target has its own field.
  Reporting a link as its target makes it indistinguishable from a copy, which is exactly the
  difference somebody needs before deciding to delete one of them.
- **`target_kind` makes a link to a directory openable.** Without it the row renders as a file
  and Enter tries to preview a directory instead of descending into it.

Names come from `/etc/passwd` rather than `getpwuid`, once per listing (D46) — NSS can block on
the network and a listing has a latency budget. The `drwxr-xr-x` formatter is written twice, in
Rust and in TypeScript, tested against the same strings (D48); the reasoning about links is
D47.

**Asking before overwriting.** `fs.transfer` takes `dry_run`, which answers *which* names are
already taken instead of moving anything. The interface uses it on every paste: no conflicts and
the transfer starts; conflicts and a dialog asks, showing both sides — what is there and what
would arrive, with sizes and ages — because "overwrite?" is not a question anybody can answer.
The safest choice is first and Enter takes it, each option has a letter, Escape moves nothing.

**Öffnen mit.** Ctrl+Enter offers the applications that claim the file's type, the desktop's own
default first; Umschalt+Enter records the choice for that type from now on, in the file
`xdg-mime default` writes to, so the rest of the session hears it. `A` widens the list to
everything installed, for the file nothing has registered for. Plain Enter is unchanged: one key,
no dialog.

Nothing in it is invented — the type comes from shared-mime-info, the applications from
`mimeapps.list` and `mimeinfo.cache`, and `cortex apps <file>` prints exactly what the window
shows. **Checked against the machine and not only against the specification:** twenty-five file
types compared with `xdg-mime query filetype` and with `gio mime`, which is what GTK applications
use — 25 of 25 agree on the type, on the default and on the order of the list. Three corrections
came out of that comparison and are in D58; the sharpest is that `NoDisplay=true` means "not in
the applications menu" rather than "never offer this", and filtering those entries had hidden the
machine's own default text editor.

An agent may not set a default, whatever the allow-list says, and the red-team suite asserts it
in both directions. The reasoning is in D58: it is the one call here whose effect is felt the
next time a *person* opens a file.

**Drives that are not mounted yet.** A stick that has just been plugged in is in the places list
now, with its size and the words "nicht eingehängt"; Enter mounts it and goes to wherever udisks2
put it, and Ctrl+E unmounts — and cuts the power, when it is something that can be unplugged.
Unmounted Windows partitions are offered the same way, which is how somebody opens the disk they
keep their videos on without going near a terminal.

The listing needs no privileges and no D-Bus: `/sys/class/block`, the disk above a partition, and
udev's own database. Only the mounting shells out to `udisksctl`, always with
`--no-user-interaction`, because a daemon waiting on an authentication dialog nobody can see has
stopped answering. Verified against this machine — a stick, three Windows partitions, an EFI
partition and six snap loop devices — and every case reproduced in tests against a `/sys` made
for the purpose. **Ejecting a disk that cannot be unplugged is refused**, and no agent may mount,
unmount or eject anything at all. D60 has the reasoning.

### What M6 still needs

**Nothing from the mandatory list.** Archives, thumbnails and network locations all landed on
2026-08-24; what is left of M6 is its Definition of Done §3 — a week of exclusive daily use by the
owner — and two things that need a person rather than more code: whether the clipboard survives
the window being hidden, and whether a drag *out* of the window reaches another application.

**Network locations are staged, and the stage boundary is deliberate** (D84). What is mounted —
by the desktop's gvfs or by `/etc/fstab` — is offered as its own section, browsable and
searchable. **Mounting one is not built**, because it means credentials, and a headless daemon
that stored them would be a second, worse keyring. `fs.list` knocks on a share with a two-second
deadline before reading it, so a server that has gone produces a sentence rather than a frozen
window, and the indexer stops at a network mount point rather than crawling somebody else's disk.

**Thumbnails in the list are done** (2026-08-24). The previewer answers `thumbnails(paths)` for a
whole viewport out of the shared XDG cache and renders nothing; the list renders the misses one
at a time, on screen only, after the scrolling stops, and asks nothing while the window is hidden
(D83). A row with a picture is exactly as tall as one without, which a browser test asserts,
because the virtualiser is told the row height rather than measuring it.

**Archives are done** (2026-08-24). `fs.extract` unpacks zip, tar and tar.gz, answers with a
handle and reports on the transfer notifications (D81); Ctrl+E in the window, and the command
palette offers "Entpacken" for an archive and for nothing else. The trust rules are D82 and each
has a test against a live daemon: an entry that would escape, a link, a cancellation that leaves
nothing behind, an undo that takes back what was written and not the file a person added
afterwards. **Not covered:** `.tar.bz2` and `.tar.xz` (the suffixes are stripped for naming but
the formats are not read), archives with a password, and *creating* one — M6 asks for extraction,
and packing is a different gesture with a different dialog.

**The stepped breath layer was measured, and it bought nothing** — 116% before, 110% after. The
explanation behind it was wrong, and what replaced it is in D59: the heartbeat runs only while
there is work. The visible idle window is now 4.8–5.1% of one core. What is still unmeasured is
the *busy* case: during an ingest the animation runs and the core comes back, which is worth a
number the next time a long ingest happens.

**And the scrolling in WebKitGTK rather than Chromium** is still only judged by arithmetic. The
gap at 420 px a frame is zero in the browser tests, and Chromium is not the renderer this ships
on. The DOM work is the same either way; whether it *feels* right is the owner's judgement.

One thing is built but unverified: dragging a row *out* of the window offers `text/uri-list`,
and whether WebKitGTK forwards that to the compositor cannot be tested from here — the browser
tests run in Chromium. It needs one drag into another application to know. Dropping *in* is
tested and works.

And two holes the reorder creates, which should stay visible rather than be filled with a
plausible number (D43): the sidebar cannot show salience, and the cold-tier indicator has
nothing to indicate, because both are M5.

## Where M4 stands

| DoD | State | Where it is proven |
|---|---|---|
| §1 Cold start < 300 ms | met — **265 ms** | ten runs of the installed release binary on an idle machine, min 260, max 280 (D41); the 418 ms first recorded here was measured under concurrent build load |
| §2 Keystroke → row < 80 ms P95 | **frontend share met** | `tests/perf.spec.ts` — 2 ms median, under 16 ms p95 against a 24 ms allowance; the daemon's ~20 ms is measured in `bench/latency` |
| §3 10 000 rows, bounded DOM, 60 fps | met | `tests/layout.spec.ts`, `useVirtual.test.ts` — under 40 rows, under 400 nodes, worst scroll update 9 ms |
| §4 400–1400 px, no overflow | met | `tests/layout.spec.ts` — four widths plus the full-width-to-400 shrink, in a real browser |
| §5 Preview of a 40 MP image < 50 ms | met | `cortex-preview/tests/process.rs` — **146 µs** for the cached thumbnail of a 40 MP photograph |
| §6 Killing the thumbnailer is harmless | met | `cortex-preview/tests/process.rs` and `ui/src-tauri/tests/against_daemon.rs` — killed mid-render, search unaffected, nothing corrupt left in the shared cache |
| §7 Everything reachable without a mouse | met | `tests/keyboard.spec.ts` — 12 checks, real key presses |
| §8 No data processing in the frontend | met | `src/test/architecture.test.ts` — dependencies, source, one door, one palette |

D4's two conditions are met and its GTK4 fallback is retired (D37).

### §1: the first number was wrong

It was recorded here as 418 ms and failing, and that measurement was taken while the same
machine compiled the workspace and ran three test suites. Ten runs on an idle machine give
**265 ms, all ten inside the budget**; the decomposition and the conditions are in D41.

Two things follow. The GTK4 question is closed — the stack was never the problem. And the
resident model, where the overlay starts hidden at login and is summoned rather than launched,
stays: not as a mitigation for a budget it turned out to meet, but because that is what a
launcher is.

### Summoning is the compositor's job on Wayland

The global hotkey never worked and reported that it had (D42). A Wayland client cannot grab a
key — the plugin falls back to an X11 grab that lands in XWayland and never fires. Under Wayland
the overlay now registers nothing and logs the line to paste into the compositor's
configuration; summoning goes through `hippocampus --show`, which reaches the resident process
through `tauri-plugin-single-instance`. Only an explicit flag moves the window: a bare second
invocation does nothing, because a stray one otherwise puts a search box in front of whoever is
at the machine.

### What is there

A Tauri v2 window with a query input, a virtualised result list, a preview pane for text, a
context sidebar showing what the brain knows about the selected file, a command palette on
Ctrl+K, and the breath layer whose period is the indexing load. Summoned with
`ctrl+alt+space`; Escape clears, then closes.

The Rust half is a bridge over a **split socket** (D38): a reader task dispatches by id, so
calls pipeline and the reranked order arrives as a window event rather than as something the
next keystroke has to wait behind.

### Four bugs the tests found, none of them visible by looking

1. **The list measured itself once, while it did not exist.** `useRef` was null on the first
   render — an empty list renders a message, not a scroller — so the row window stayed at zero
   height for ever. The scroll height was right and the list was empty: the failure looks like
   "no results" and is not.
2. **A grid item will not shrink below its min-content width.** `1fr` instead of
   `minmax(0, 1fr)` on the overlay let the list's scrollbar push the whole window ten pixels
   sideways at 400 px, which is exactly the overflow §4 forbids.
3. **`scrollToIndex` ignored the list's padding**, so the last row sat six pixels below the
   visible edge — present in the DOM, cut off on screen.
4. **A cold start too fast to measure read as "never started".** Zero was the sentinel for
   "has not happened yet", and zero milliseconds is the best case.

And one the daemon owed the frontend: `SearchHit` carried no `size` or `mtime`, so a result row
could not say how big a file was or when it changed without opening it. Both are on the node
already; they are on the hit now.

### The thumbnailer

`cortex-preview` is a process whose whole design goal is to be killable (D14). It shares the
desktop's XDG thumbnail cache — same directory, same MD5-of-URI naming, same `Thumb::MTime`
invalidation — so a thumbnail it renders is one Nautilus does not have to, and the other way
round.

Three things in it are worth knowing:

- **A JPEG is never fully decoded for a small preview.** `image` 0.25 decodes JPEG with
  zune-jpeg, which has no DCT scaling, so the previewer uses `jpeg-decoder` directly and asks
  for a scaled image. A test asserts a 128 px preview costs strictly less than a full one; the
  40 MP case is where that is the difference between six hundred kilopixels and forty million.
- **PDFs and video frames are rendered by whatever the machine already has** — `pdftoppm` or
  `mutool`, and `ffmpeg`. Bundling a PDF engine would mean shipping two of the largest attack
  surfaces in desktop software. Where the tool is absent the pane names it.
- **A failed render is remembered** under `fail/cortex-preview/`, so no file manager on the
  machine keeps retrying a file that cannot be decoded — and a file that *changes* is retried,
  because the marker carries an mtime like everything else here.

### Not done

1. **Audio shows nothing yet.** The scope says "waveform metadata only"; the previewer refuses
   audio with a reason rather than rendering a waveform, and the pane prints that reason.
2. **`fs.list` has a `glob` parameter nothing implements.** It is in the wire format and the
   MCP schema; no filter is applied.
3. **The overlay is not packaged.** It runs from `cargo build`; a desktop entry and a bundle are
   M9.
4. **Zone accents are wired but untested against real zones** — the hue comes from the daemon
   and is rendered as a 2 px left border, but no test asserts what a user sees for a zone tree.

## Where M2 stands — read this first

Everything M2 asked for exists and is measured: extraction, chunking, a lexical index, embeddings
on the GPU, a vector index, three-arm fusion, snippets, explainability, a cross-encoder reranker
delivered as a result update, a goldset, and a rebuild that reproduces the index from the
filesystem.

A query finds documents that share no word with it, in either language. The proof is in
`cortex explain`, not in a claim: for *"how fast did storage expenses grow after moving the
archive"* the German answer comes back with **0 path candidates, 0 lexical candidates, vector
rank 3**.

### The one criterion that is not met

**German queries are 14.8 % behind English ones** (85.2 % against 100.0 % recall@10), where the
Definition of Done allows 10 %. The build prompt names the consequence: *"the embedding model is
wrong and D6 must be revisited"*.

Before revisiting it, know what the number is made of. **The entire gap is three queries out of
twenty-seven**, and all three are German questions whose answer is an English document:

| direction | queries | found |
|---|---|---|
| German → German | 24 | 24 |
| German → English | 3 | 0 |
| English → English | 20 | 20 |
| English → German | 3 | 3 |

So it is not that German is handled worse. It is that **German → English retrieval fails while
English → German works**, and the cause is measured: same-language similarity carries an offset
of roughly 0.08–0.12 over cross-language similarity for this model, which is more than the gap
content opens up on short texts. `goldset --explain q43` shows a German query returning twenty
German *noise* documents ahead of the English document that answers it.

Three things could close it, none of them tried, all measurable with the instrument that now
exists:

1. **Per-language score normalisation** in the vector arm — needs a language guess per document.
2. **Query-side expansion**: embed the query in both languages. Doubles query embedding cost
   (1.1 ms on the GPU, so affordable) and needs translation, which needs a model.
3. **A different model.** `bge-m3` and `jina-embeddings-v3` are both stronger cross-lingually and
   both several times the size. This is what "revisit D6" would mean in practice.

### What the goldset says

Measured with the real model against 1 029 documents (29 judged, 1 000 generated distractors):

| arm | recall@10 | German | English | MRR | top-1 | by name | lexical | semantic | cross-language |
|---|---|---|---|---|---|---|---|---|---|
| **fused** | **92.0 %** | 85.2 % | 100.0 % | 0.878 | 86.0 % | 100 % | 100 % | 96 % | 50 % |
| fused + rerank | 92.0 % | 85.2 % | 100.0 % | 0.878 | 86.0 % | 100 % | 100 % | 96 % | 50 % |
| lexical only | 36.0 % | 33.3 % | 39.1 % | 0.360 | 36.0 % | 100 % | 86 % | 0 % | 0 % |

**The model is worth 56 points.** Without it every query whose answer does not repeat its words
fails, which is the case this project exists for.

**The reranker changes nothing, and that is a fact about the goldset rather than about the
reranker** (D30). Its distractors are Zipfian nonsense: plausible in shape, empty in meaning. A
cross-encoder earns its keep against documents that are plausible *and* irrelevant, and there are
none here. Verified separately that the model itself works — on clean pairs it separates the
right answer from the wrong one by eighteen points of logit, in both languages.

### Not done

1. **PDF extraction.** `Handling::Pdf` is classified and skipped. When it lands, note that
   `snippet` re-runs the *text* reader for every handler; a PDF's byte offsets are into extracted
   text that no plain read reproduces, so the handling will have to be stored per node.
2. **Tree-sitter chunking**, still staged behind the heuristic.
3. **A wall-clock cap per extractor.** The byte budget bounds memory and the stage's own thread
   bounds latency for everything else, but one file on a stalled network mount can hold the stage
   indefinitely.
4. **The 24-hour soak test**, deferred by the owner since M1.
5. **Real distractors for the goldset** — a few hundred written documents. It is the only thing
   that would make reranking measurable, and it would make every other number more trustworthy.

### Open questions the goldset can now answer

- **Ties in fusion are broken by node id.** Should a path hit beat a content hit at equal score?
- **Arm weights are all 1.0**; `rrf_k` is 60 for retrieval and 10 for the reranked order (D29).
- **Prefix matching** is on for interactive queries and off for the goldset.
- **The mode heuristic** (D24) may mislabel queries.

### Limitations worth knowing

FTS5 folds `ä` to `a` but not `ß` to `ss`, and has **no stemmer configured**, so "verworfen" does
not match "verwirft" — two of the goldset's lexical queries fail on the lexical arm for exactly
that reason. Both are query-side expansion problems.

The GPU is shared with whatever else the machine is running (D31). Every number here was measured
while a local language model held 11.7 GB of the 16 GB card.

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
- The reranked order is fused with the retrieved one, never substituted (D29)
- Generated distractors must be irrelevant by construction (D30)
- The GPU is shared, so every model runs under a memory cap (D31)

## Still open

| # | Question | Needed by |
|---|---|---|
| D13 | License for the public Core (recommendation: Apache-2.0) | M9 |
| — | ~~Concrete peer protocol for federation~~ — `docs/08-FEDERATION.md`, 2026-09-05. What is still open is the *bridge* to a concrete peer, in the Extension, as a `sync` plugin | M7 → Extension |

---

## Session log

Newest entry on top. One entry per working session. Keep entries short and factual — what
changed, what was learned, what the next agent needs to know.

### 2026-09-05 — M9: what a second person would need, built; the second person, not yet

Same day as M7, and the milestone the order left next. What can be built without another human
is built: `cortex setup` (three questions, every default the safe one — D97), a Debian package
from a script that needs only `dpkg-deb`, an AppDir, a desktop entry (D96), `docs/09-INSTALL.md`
for the person who has never seen this repository, and the hygiene gate extended to every
tracked file and every patch in the history, run by `cargo test` and verified against a seeded
fake secret.

**What the first run found:** `cortex setup --root /tmp/x` enabled the *real* daemon's systemd
unit, because "start the daemon" did not know it was talking about a throwaway. Under `--root`
it now leaves systemd alone. And a backtick inside an unquoted heredoc in the package script ran
`cortex setup` while writing the control file — the shell was helping.

**Still the owner's:** the timed install by somebody else (§1), and the license (D13). The
`.deb` is at `target/packaging/` after `packaging/deb/build.sh`; the honest test is handing it to
someone.

### 2026-09-05 — M7: a peer the Core never names

The next milestone in the order that was left open, and the shortest one so far: everything the
build prompt asks for was already decided in D11, D17, D18 and D19, and the work was to make those
sentences true in code.

**The one decision the documents did not already contain** is D93: the outbox is *derived*. No
write path in the daemon knows the peer exists; a worker harvests `episodes` from a cursor and
concept nodes by identity, and the identifier of every event is a function of the row and this
index's origin, never of the payload. Replay is "clear the cache and harvest again"; the same
identifiers come out, and the peer that keeps them sees nothing new. That is the criterion, and it
is a property of a hash rather than of a peer.

**Two mistakes found by the tests before anything ran.** A row deleted on ack made every concept a
new event on the next tick (D94, migration 0008); an unreachable peer counted as an attempt would
have dead-lettered a weekend's outbox in two minutes (D94, the other half).

**Built:** `crates/cortex-sync` — wire format, `Transport`, `file-drop` and `plugin` transports,
the worker, `hc-test-peer`; `cortex-store::outbox`; `cortex-core::plugins`; `federation.*`
settings; the sync stage in the daemon; `sync.status`, `sync.now`, `sync.replay`; `cortex sync`;
`scripts/pre-commit --all` with an endpoint pattern, run by a test; `docs/08-FEDERATION.md`.

**What the next session should know.** The Extension's bridge is the only thing between this and
a real second brain, and it is a forty-line program. The owner's home server was surveyed for
D17–D19 (a directory it already watches, no ingest API, nightly deduplication with a cap) — the
`file-drop` transport was built for exactly that peer, and `hc-test-peer ingest` is the consuming
half to adapt. Nothing in this repository may name that server; the hygiene gate now refuses a
URL that would.

### 2026-08-25 — And then the loop froze the window

The render loop from the entry below was right and unbudgeted, and the first thing it did on the
owner's machine was hang the overlay. Nothing was spinning afterwards — both processes idle at
0.1% — which fits a freeze that lasted while the Architektur tab was open and ended when it was
closed.

Three limits now, all from one fact: this window is WebKitGTK with the dmabuf renderer disabled
(D59), so **software OpenGL**, where "as fast as possible" means "as slow as this frame takes"
with the keyboard unanswered in between. No glow pass in the panel (the composer runs every pass
it holds; the standalone view keeps its glow and its GPU); thirty frames a second as a ceiling;
and nothing at all while the window is hidden. D92 has the reasoning.

**The frame rate is on screen for the same reason it is in a test**: a black canvas says nothing,
"0 Bilder/s — es wird nichts gezeichnet" says which half is broken.

**Still unknown, and cheap to check now:** whether the picture appears at all under software GL.
The panel says whether frames are being drawn, so the next report can distinguish "nothing is
drawing" from "something is drawing and it is invisible" — and only the second one is about the
renderer.

### 2026-08-24 — The 3D tab was black because nobody drew it

The sizing fix (D90) was real and not the cause. `cortex graph --limit 4000` answers with 4 023
nodes; the canvas was built and sized; the renderer started. **Nothing called `render()`.**

`BrainScene.render()` draws one frame and returns — the standalone view drives it from its own
`requestAnimationFrame` loop, and the panel never did. What kept it hidden is worth reading in
D91: `ui/src/agent-view.d.ts` declares the boundary between the window and `ui-agent` by hand,
narrowly, and did not declare `render` at all. The panel therefore *could not* call it, and a
missing call is not a failure the tab can report — the file's own comment promised that this tab
"fails loudly rather than silently", which a boundary cannot promise about a method it does not
mention.

**The panel prints its frame rate now**, and says "es wird nichts gezeichnet" at zero. That is the
diagnostic whose absence cost the evening. The test reads it, and lives in the serial config
beside the perf suite — it is the only test that compiles shaders, and five software renderers at
once make "is anything being drawn" fail for reasons unrelated to the answer.

### 2026-08-24 — Two defects the owner found by looking: unreadable controls, and an empty 3D tab

Reported as "die Buttons sind verbuggt und hässlich" and "bei Architektur sieht man nichts". Both
turned out to be one-line causes with wide blast radii, and both now have a test.

**Every control in the window was painted by the platform.** There was no `button` reset: a
`<button>` keeps `buttonface`, which is light, and the classes set only a text colour. The
Inspector's tabs, kill switch, memory actions and its `select` were light-on-light. One rule in
`tokens.css` fixes all of them, and `controls.spec.ts` now walks every control in the window, the
palette and each Inspector tab and fails on any background lighter than the midpoint (D89).
Unselected tabs also lighten on hover now — nothing had said they were pressable.

**The 3D view measured the window and lived in a panel.** `BrainScene` read its container's size
once and then listened to `window.resize`; inside the Inspector the container is a panel that is
mounted mid-animation, so the measurement was zero and nothing ever corrected it — 0×0 canvas,
black box, no error, with 4 023 nodes waiting behind it. It observes its container now, and
`architecture.spec.ts` asserts the canvas fills its box (D90).

**What is not yet known**: whether that was the whole of it on the owner's machine. The test runs
in Chromium and the window is WebKitGTK with the dmabuf renderer disabled (D59). If the tab is
still empty after this build, the renderer is the next suspect — and the tab's failed state does
print its reason, so an empty box means the renderer started and drew nothing.

**Still open from the same report:** *"im Allgemeinen nicht übersichtlich"* — the Inspector's
layout. Not a defect, a design question, and the first one worth answering now that its controls
are legible: five tabs, a live list, a memory list with three actions per row, and no hierarchy
between them.

### 2026-08-24 — M10 step 4: the sidebar's new job, and M10 is done

The last of the four steps. **1007 Rust tests**, **60 vitest**, **156 functional browser tests**
(4 new) and 12 perf. Green.

`node.why(path)` answers the sidebar's questions in one call: the parts of the salience score as
the consolidation pass stored them, the file's episodes, the notes attached to it, and the memory
columns. The panel shows *Warum* — named parts, signed, adding up to the score — *Verlauf* in the
log's own verbs, and a one-line field that attaches a note without touching the file. "Verbunden"
is gone from the panel and is a button that goes to the `neighbours` place instead. D88 has the
reasoning, including the two things it decided not to do: recompute the score for display, and
show an annotation twice.

**Three things this session got wrong first.**

1. **Enter in the note field opened a file.** The window handles Enter for the selected row and
   sees the key bubbling out of the input. The field stops it now.
2. **A test asserted an annotation appears in the file's history.** It does not, and should not:
   the episode belongs to the annotation node so a rebuild replays it as what it was. The note
   shows once, in *Notizen*.
3. **The panel's own state outlived the selection.** Half a typed sentence carried to the next
   row would attach itself to a different file at the next Enter; the field clears with the
   selection.

**M10 is complete as a plan.** What it asked for — one row stream, the memory on every row, the
new places, the sidebar — is built and tested. What it did not ask for and is now worth doing is
the owner's list from the same day: **the agent view does not show everything it could, and the
explorer's GUI is still rough in places.** Those are the next session's work, and they are not
architecture — they are the appearance pass the owner has been deferring since M4, plus whatever
using the thing for a week turns up.

### 2026-08-24 — M10 step 3: the places the index makes, and a perf gate that was measuring the machine

Step 3 of four. `concept`, `working_set`, `neighbours`, `timeline` and `tier` are sources like a
directory is: one method, one row shape, one navigation model. **1005 Rust tests** against 1001,
**60 vitest**, **152 functional browser tests** plus **12 perf** — and those two numbers are
separate now, for a reason below.

**What is reachable.** The places screen has a *Gedächtnis* section: what you are working on, the
concepts the last consolidation named (biggest first, with how many files are in each), and
"Vergessen" when the cold tier has anything in it. Enter opens one like a folder. The rows are
ordinary rows — selectable, copyable, previewable. Alt+Left walks back out. The basis line is
rendered above them, which it had not been since step 1 built it: a concept says how it was
named, and the cold view says that **nothing was deleted**.

**The window's place is a `Source` now**, which is the revisit D85 predicted, and it is what makes
Alt+Left work out of a concept. Up and back became different questions in the process: only a
directory has a parent, so `goUp` in a concept falls through to `goBack` rather than inventing a
hierarchy.

**Three things this session got wrong first, and the tests that said so.**

1. **A match arm in the wrong order.** The catch-all for the new sources was written above
   `Source::Query`, which made the query branch unreachable — `cargo check` said so as a warning
   and it would have been a silent behaviour change: every search would have gone through the
   generic path and answered nothing.
2. **`hasText: "Arbeit"` matched two rows.** A places test located the drive called *Arbeit* by
   substring, and the new row *"Woran ich gerade arbeite"* contains it. It fails as a strict-mode
   violation, which reads as a broken locator rather than as a name collision. It matches the
   exact name now.
3. **A stale binary in `target/debug`.** `the_places_are_the_same_through_all_three` failed with
   *unknown variant `view`* — the CLI could not decode a place kind that had just been added. The
   CLI shares the type; the *binary* was from before the change. The test's own message says to
   build the bins first, and that is exactly what it caught.

**And a perf gate that had been measuring the machine.** `a keystroke reaches a painted row` fails
at 31 ms against its 24 ms render share when the full Playwright suite runs it beside four other
workers driving ten-thousand-row lists. Alone it is **6.0 ms p95, 2.3 ms median**. It had been
read as a regression three times in one day. The perf suite is now excluded from the default run
and has a config of its own — `npm run test:perf`, one worker, nothing else in flight — so the
mistake is no longer available. The kickoff prompt's rule was already there in words: *measure on
an idle machine*; now the harness enforces it.

**What step 4 is, for whoever picks this up.** The sidebar's new job: `brain.explain` one
keystroke away, episodes for the selection, annotate without leaving the window, and "Verbunden"
removed from the sidebar because it is a place now (`neighbours`). The columns step 2 carries but
does not draw — salience, degree, last episode — belong there.

### 2026-08-24 — M10 step 2: the memory on every row, measured first

Step 2 of four. Salience, tier, degree, last episode, pinned and zone on every row from every
source; tier renders as brightness; `Order::Salience` exists and a directory can be opened by it.

**1001 Rust tests** against 998 — the measurement plus two against a live daemon — **60 vitest**,
**159 browser tests** (4 new). Green.

**The measurement §5 demanded, before the design.** Ten thousand rows, debug build, warm cache, a
tenth of them carrying edges and episodes:

| | |
|---|---|
| one `node_by_path` per row — what the listing already does | **286 ms** |
| one statement per 900 paths, with degree and last-episode joined in | **55 ms** |

The threshold was 15 ms of *added* cost. The columns add none: they cost a fifth of the shape
they sit beside. `cortex-store/tests/listing_cost.rs` asserts the difference rather than the
absolute number, and D86 has the reasoning, including the two things it decided against the
paper's suggestion — degree is counted rather than stored, and the lookup is keyed by path rather
than by `parent_id`.

**The `parent_id` version was written first and came back empty.** A freshly reconciled directory
has paths before it has parent links — the link pass runs separately — so every memory column was
absent on exactly the directories somebody would look at first. Keyed by path it is correct
whenever the pass last ran.

**What the window shows.** Tier is brightness in four steps, and the "cold" badge is gone: it
said something about one row in fifty and nothing about the other forty-nine. A pinned row does
not fade. The palette offers "Sortieren: Salienz", and choosing it asks the daemon for that order
— the frontend still never sorts.

**What is deliberately not shown.** Salience, degree and the last-episode timestamp are on the
row and nothing draws them. Numbers in a column would be a visual design decision, and the owner
has deferred the appearance until the thing is stable; step 4 gives them a home in the sidebar,
where "why is this here" is a question somebody is actually asking.

**Step 2's last criterion is met** (2026-08-24, by the owner, at their own index): a directory
opened with **Sortieren: Salienz** put the files they had been working with at the top. Verified
by hand, once, as the Definition of Done asks — the weights are not obviously wrong, and there is
nothing further to tune blind. What follows is the criterion as it was written and how to repeat
the check.

**How the check was made, for repeating it.** Its Definition of Done ends with
*"a directory of files you have worked with this week, ordered by salience, puts them at the top
— verified by hand, once, and written down."* That is a claim about a real index and cannot be
asserted against a fixture. Open the window in a directory you have been working in, press Ctrl+K
and choose **Sortieren: Salienz**, and write down here whether the top of the list is what you
would have picked. If it is not, the salience weights are wrong, not the view.

**Two more lost keystrokes.** The `context.spec.ts` fix from the previous session covered one
helper; two other tests in the same file pressed Ctrl+Shift+A immediately after `goto` and failed
the same way under load. All four wait for the application now.

### 2026-08-24 — M10 step 1: one row stream, and a flaky test that was a lost keystroke

M6's mandatory list finished earlier in the day, so this session started the restructure the
explorer paper argues for. **Step 1 of four**, and step 1 only: `view.open` with two sources, one
row shape, one mapper. No new capability, no behaviour change.

**998 Rust tests** against 990 — 4 new in `cortex-core::view`, 4 in `cortexd/tests/views.rs` —
**60 vitest**, **155 browser tests**. Green, including a full browser run *while*
`cargo test --workspace` had every core, which is the run that used to fail.

**What exists now.** `cortex_core::view` holds `Source`, `Order`, `ViewRow` and `View`;
`view.open` serves `dir` and `query` through the same `Surface::list` and `run_search` the old
methods use; `view.update` carries a reranked order as rows. In the window, `useBrowse` and
`useSearch` are gone and `useView` replaces both, `rowFromHit`/`rowFromEntry` are gone and
`rowFromView` replaces both. `fs.list` and `search` stay for the CLI and for agents.

**The `hit` the panes are given is now built from the row** rather than from whichever call
produced it — a `Row` carries `nodeId` and `score` now, so the "was this a search result"
question has nowhere left to be asked.

**What "no branch" honestly means** is written up in D85, because the DoD sentence is easy to
overclaim: the *row path* has no branch, and nineteen mentions of `searching` remain in `App.tsx`
for things that are about the window rather than about a row — whether a drop has a directory to
land in, whether the breadcrumb shows, which of the two held views is on screen.

**A defect in a test nobody wrote this session, found by looking at why it kept failing.**
`context.spec.ts` failed three times in full runs and never on its own. The error read "waiting
for tab Gedächtnis" for thirty seconds, which looks like a broken tab strip; the page snapshot
showed no panel at all. Its helper pressed Ctrl+Shift+A immediately after `goto`, without waiting
for the application — under load the press landed before React had attached its keydown listener
and was simply lost. It waits for a row and then for the panel now. Six repeats at six workers
pass, and so does a full run under a parallel `cargo test`. **A test that races the thing it is
testing fails as a mystery rather than as a message.**

**What step 2 needed** is done the same day, and so is step 3 — see the entries above. Step 4 is
the sidebar's new job: `brain.explain` a keystroke away, episodes for the selection, annotating
without leaving the window.

### 2026-08-24 — M6: network locations, and the calls that do not come back

The last item on M6's mandatory list. Shares that something has already mounted are their own
section of the places screen, browsable and searchable; nothing here mounts one, and D84 says
where that line is and why.

**990 Rust tests** against 979 — 5 new in `cortex-agent::places`, 3 in the new
`cortex-core::mounts`, 1 in `cortex-ingest::reconcile` — **60 vitest**, **154 browser tests**
(4 new in `ui/tests/network.spec.ts`). Green.

**The whole feature is one fact and three consequences.** What is behind the mount point is on
another machine, over a network that can be gone without anything saying so. So: the places
screen never calls `statvfs` on a share (it blocks); `fs.list` knocks once with a two-second
deadline on a thread it is willing to abandon; and the indexer stops at a network mount point
without even stat-ing it, counting what it did not enter in `network_skipped`.

**`/proc/mounts` moved to `cortex-core::mounts`.** Two crates now need the same answer for
opposite reasons — the places screen to offer a share, the indexer to avoid one — and two parsers
would be two opinions about what a share is. The octal-escape handling and its test moved with
it, which is the part that would have been reimplemented slightly differently.

**Two things that were nearly wrong.**

- **A share under `/mnt` was offered twice.** `/mnt` is one of the roots where anything mounted
  counts as removable media, so a CIFS mount there appeared as a drive *and* as a network
  location — with a capacity bar, which means the screen would have called `statvfs` on it after
  all. `is_place` refuses network filesystems first now, and there is a test.
- **A thirty-second test.** The first version of "a wedged gvfs costs the screen nothing" slept
  thirty seconds in an injected closure to prove it. It passed, and it would have added half a
  minute to every run for a property that `answers_within`'s own test already asserts in eighty
  milliseconds. Replaced with a test about the listing's order, which is what that layer actually
  decides.

**And one the screenshot found, which no test would have.** With the share section in it, the
places sidebar showed `server:/export/backup` and *no name at all* for that row: the row's third
grid column was `auto`, so a long address took its full width and the name — the thing somebody
reads — collapsed to nothing. The name column has a floor now and the address elides instead;
the capacity bar is pinned at 56 px, because making that column shrinkable squashed it to a stub
on the first attempt, and a bar that gives way is a bar showing the wrong number. Both are
asserted, at 1200 px and at 400 px.

**A note on measuring, again.** The Playwright perf suite failed once during this session on
"a keystroke reaches a painted row well inside 80 ms" — while `cargo test --workspace` was using
every core in the background. On an idle machine the full run is 154/154. A performance gate
measured against a busy machine measures the machine.

### 2026-08-24 — M6: thumbnails in the list, and the queue that is not slowness

The second item off M6's mandatory list. `thumbnails` in `cortex-preview`, `useThumbnails` in the
window, and a picture in the rows that have one.

**979 Rust tests** against 977 — 2 new in `cortex-preview/tests/process.rs` — **60 vitest**
(4 new) and **150 browser tests** (4 new). Green.

**The shape, and why.** Two calls rather than one: a batch cache lookup that provably never
renders, and the existing per-file render for the misses. The previewer is one process answering
one request at a time and the preview pane shares it — a call that *might* render turns a flick
through a directory of photographs into two hundred queued decodes with the selected file's
preview behind them. D83 has the argument and the three consequences.

**Two things this session had to be careful about, both about not breaking what is measured.**

1. **The row height.** The virtualiser is told the height rather than measuring it, so a picture
   that made a row four pixels taller would put every scroll position progressively out — worse
   the further down the list, which is the kind of bug that gets reported as "scrolling feels
   wrong". The image is `1.35em` in the line box, and a browser test compares a row with a
   picture against one without at both densities' compact value.
2. **The hidden window.** D59 measured what periodic work for nobody costs. The hook takes
   `visible` and asks for nothing when the overlay is hidden, and a unit test asserts the
   transport is never called in that state.

**Two things found while building it.**

- **The architecture test earned its keep.** `useThumbnails` was first written with `filter` and
  `find`, which D4's guard forbids anywhere in this frontend. Nothing in the hook reshapes what
  the daemon sent — these were "which paths to ask about" — so the temptation was to add an
  exception. It was rewritten as loops instead: a blunt rule with one carve-out is a rule, and a
  blunt rule with two is a habit.
- **A NUL byte in a source file.** A `join(" ")` in the new hook contained `\x00` instead of a
  space after a scripted edit. It compiled, ran and passed its tests — a separator is a
  separator — but `file` reported the source as `data`, and the repository's own `grep` skips
  binary files, so a search for `filter` in that file silently found nothing while the test that
  reads it with `readFileSync` kept failing. **Two tools disagreeing about whether a file
  contains a string is worth stopping for**; the tool that reads bytes was right.

### 2026-08-24 — M6: archives, and four things an archive is not allowed to do

The first item off M6's mandatory list since the milestone was reordered. `fs.extract` in
`cortex-agent/src/archive.rs`, dispatched by the daemon the way `fs.transfer` is, reachable from
the window with Ctrl+E.

**977 Rust tests** against 965 at the start — 9 new integration tests in `cortexd/tests/archives.rs`
and 3 in the module — **56 vitest**, and **134 browser tests** in `test:ui` including 5 new ones in
`ui/tests/archives.spec.ts` (146 with the perf suite). Everything green.

**One flake worth knowing about, and it is not new.** Under the full Playwright run at five
workers, `context.spec.ts` "states a preference outright" timed out once waiting for the memory
tab, and passed on its own and on the next full run. It is a load-dependent wait in a test that
drives the Inspector, not an extraction problem — recorded here rather than fixed blind.

**What it does.** Zip, tar and gzipped tar, identified by magic bytes with the extension only as
a fallback. A dry run answers what is inside, where it would go and whether something is already
there; the real call answers with a handle and reports progress until it is done or cancelled.
Refused entries are named in the answer rather than skipped. Undo removes exactly what was
written. The reasoning is D81 (why it reports as a transfer) and D82 (the four rules about paths
that came from a stranger).

**Three things found by building it, in the order they would have hurt.**

1. **An archive can unpack into itself.** The destination is the archive's name with its suffix
   stripped — and a file identified by its *bytes* need not have one. `bundle.bin` produced
   `bundle.bin` as its own destination, which the conflict check then reported as "already
   exists": a correct refusal for entirely the wrong reason, and one that would have read as a
   bug in the conflict handling rather than in the naming. Found by a test that was about gzip
   detection and named the file `.bin` for unrelated reasons.
2. **`tar_gz` is not a word.** `#[serde(rename_all = "snake_case")]` spelled the format `tar_gz`
   on the wire while the type's own `as_str` said `tar.gz`. Two names for one thing, and the one
   a client would have shown a person was the wrong one.
3. **A field nothing reads.** `FsExtractParams` was drafted with a `plan` field beside `dry_run`,
   copied from `FsTransferParams` — where it is also unread. It was removed rather than added to:
   an extraction is gated like a transfer, by the kill switch and the write allow-list, because
   the plan handshake fingerprints targets and an extraction's targets do not exist yet. This
   file has a section about things that were written, tested and called by nothing; the way not
   to extend it is to notice at the time.

**What is not built:** `.tar.bz2` and `.tar.xz`, encrypted archives, and creating one.

### 2026-08-24 — M11 built: the memory layer, the second collection, and the pack

Three slices, 942 tests against 881 at the start of the session, every one of them green.
`cargo test --workspace`, counted by summing `test result:` lines rather than by reading the
last one — the note above about the measuring tool applies to counting, and it is why this
number is trustworthy.

**What exists now.** Migration 7; `kind = 'memory'` as a node kind; scopes, kinds, supersession,
expiry, usefulness and the counts behind it; a second usearch collection over memory chunks with
its own embedding pass and its own reconcile; `brain.context` in `cortex-query`; and the whole
family on the socket, the CLI and MCP with the three-surface test extended to prove they agree.

**Three bugs found by building it, and all three would have shipped.**

1. **The first migration would have destroyed the index.** Widening a CHECK constraint means
   dropping and recreating the table, and `DROP TABLE nodes` with foreign keys enforced runs
   every `ON DELETE CASCADE` on the way out: edges, chunks, annotations, salience_log. The
   obvious guard — `PRAGMA foreign_keys = OFF` at the top of the migration — **does nothing**,
   because that pragma is a no-op inside a transaction and every migration runs in one. It reads
   exactly like protection. The runner now handles it around the transaction for migrations that
   declare `rebuilds_tables`, runs `foreign_key_check` before committing, and restores what it
   found. The regression test is behavioural — real children, enforcement genuinely on — and it
   was checked against the broken version before being kept: it fails with "edges were cascaded
   away".

   The same first draft also silently dropped the `extract_note` column and four indexes, which
   is why the table was dumped and compared rather than rewritten from memory. There is now a
   test that every index on a rebuilt table survives.

2. **`canonical_dump` could not see a memory.** It walks `nodes WHERE path IS NOT NULL`, so a
   rebuild that lost every memory would have compared equal and Definition of Done §2 would have
   passed while being false. It has a memory section now, keyed by `(scope, kind, text)` because
   the rebuild renumbers the ids it would otherwise be keyed by.

3. **Two vector counts could not tell the collections apart.** `indexable_vector_count` joined
   chunks to embeddings and nothing else, so the first embedded memory would have pushed the
   file collection's expected count past what the file index holds, the reconcile would have
   concluded the index had drifted, and it would have rebuilt 122 137 vectors on every pass for
   ever. Both queries are scoped by `n.path IS NOT NULL` now, with a test that fails if either
   forgets.

**And one that only an end-to-end run could find.** A task is a sentence — *"where does the
reference corpus come from"* — and `match_expression` builds a conjunction of every word in it,
deliberately, because an OR over a hundred thousand files ranks noise above the answer. Over a
memory store of a few thousand that reasoning inverts: the conjunction matched nothing, and a
context pack that returns nothing looks exactly like an empty memory. `task_expression` is a
disjunction over content words with a short bilingual stopword list. The first version of that
compared case-sensitively, so a sentence starting with "Der" or "The" kept its stopword.

**What was decided while building, and written down.** D79: the token budget is an estimate.
The Definition of Done asked for "a tokeniser rather than a divisor", which hides the question
it does not answer — *whose*. The only tokeniser this repository ships belongs to a
384-dimension retrieval model. The estimate is structural, biased upwards because the two errors
are not symmetric, and bounded below by the word count, which a byte-pair encoding cannot go
under. Both build prompts are amended.

**What is not built yet**, in the order it should be picked up:

- **The Agent Inspector** (M11 §8) — the button, the panel, the five tabs. Nothing of it exists.
- **The live 3D view** (M11 §9) — `ui-agent` still eats a static `graph.json`.
- ~~**The benchmark** for `brain.context`~~ Written, and it earned its keep twice within an hour
  of existing. See the M11 benchmark note below.
- **`brain.explain` over a memory.** It explains a file's ranking; a memory's `usefulness` and
  its counts are in the pack's basis but not in `explain`.
- **Consolidation over memories.** They are nodes, so salience and tiering reach them for free,
  but nothing has been *run* over a corpus of them and no number says whether the decay curve
  that suits files suits a memory. D76's revisit trigger is the honest place to start.
- ~~**`decay_unreported` is not called by anything.**~~ Done in the same session: the
  consolidation pass now expires session scratch and penalises memories that keep arriving in
  packs and that nobody has ever reported on. It is in the pass rather than on a timer of its
  own because it is the same question the rest of the pass asks — what still deserves to be in
  the hot part of the index — and a second scheduler is a second thing to be wrong about when
  it last ran. The dry run reports both numbers and changes neither, which is asserted.

### 2026-08-23 — M11 specified: the agent brain, and the vector question answered

Specification only; no code in this commit. The owner asked for the agent area to be built out,
for Hippocampus itself to become the memory an agent works from, and for that to go into the
build prompt and the documentation before it goes into the code. It is milestone **M11** in both
build prompts, with five decisions behind it.

**The design in one sentence: a memory is a node.** What an agent learns that is not in a file
becomes `kind = 'memory'` — no path, its own text, a scope — and every faculty the file memory
already has applies to it unchanged. The alternative, a second agent-flavoured store, was
considered and rejected in **D74** for three reasons that are one reason: two stores means two
rankings and the second is always worse; the interesting queries cross the boundary and merging
two incomparable score scales is the mistake RRF exists to avoid; and a second store holding the
only copy of what an agent learned would not satisfy R10.

**Was the vector database supposed to become Qdrant? No — D75 says why**, and the table in it is
the short answer: a network service against R5, a second install step against M9, and a measured
1.01 ms P95 against a 40 ms budget on the index that already exists. What is actually missing is
not a different engine but **a second collection**: a memory recall against one index shared with
122 137 file chunks competes for twenty slots and loses, whatever the engine is. The existing
`VectorIndex` gets instantiated twice.

**The two ideas in it that are not obvious:**

- **Retrieval without use is evidence *against* a memory (D76).** Every salience component so far
  rewards being touched, which is right for files — a human opening a file found it useful — and
  wrong for a memory, because a memory in a context pack was not chosen by anybody, it was chosen
  by the ranking being evaluated. So `brain.outcome` reports back, and a memory that keeps
  arriving in context and is never reported useful decays *faster* than one never retrieved.
- **`brain.context` returns a pack, not rows (D77).** Budget in measured tokens rather than
  characters over four; pinned preferences unconditionally, because their whole value is being
  present; MMR before truncation, because restatement is the specific failure of embedding
  retrieval over a memory store; a basis on every item, because a memory an agent cannot attribute
  is one it cannot argue with.

**M6 still gates M10 and this does not change that.** M11 was put ahead of the file-manager
basics deliberately and the exception is written into the milestone map: M6 gates the *window*,
M11 is mostly daemon, and its window component is a panel inside the existing toplevel rather
than a new surface. Archives, drag-and-drop out of the window, network locations and correct
permission display are still missing and still come before M10.

Next: migration 0007, the store layer, then `brain.context`, then the Inspector. The
Definition of Done is twelve items and item 2 — a rebuild reproducing a byte-identical dump
including usefulness counts and deletions — is the one that will find the bugs.

### 2026-08-21 — The password prompt, and a negative that was not evidence

The owner read the new refusal — an internal disk needs an administrator — and asked the obvious
question: can the explorer not just ask for the password? It can, and the change is small: the
ordinary `fs.mount` still never waits, but it now refuses with a code of its own (1004,
`NEEDS_AUTHORISATION`), and the window answers that code by asking a second time with
`interactive: true`. That second call lets polkit put a dialog in front of somebody who pressed
Enter a moment ago and is therefore still there. 120 seconds, then it gives up. D61 has the rest.

**The diagnosis before it was wrong, and the way it was wrong is the lesson.** `ps` showed no
polkit agent in the session, so the conclusion was that there was none — a package was installed
and a line added to the compositor's config to fix it. The new agent refused to start: *an
authentication agent already exists for the given subject*. It was inside the desktop shell, a
thread of a Quickshell process, invisible to a search for process names. Both changes were
reverted. **A negative from `ps` is not evidence of absence**; polkit knew, and answered in one
line. The only thing that had ever been wrong was our own `--no-user-interaction`.

Verified on the real disk: refused with 1004 without asking, mounted at `/run/media/<user>/Videos`
with `--ask`, and unmounting needs no authorisation at all.

### 2026-08-21 — What "everything is buggy and it hangs" turned out to be

The owner used the drives an hour after they shipped and reported that selecting one that is not
the stick broke everything. **The logs could not settle it and a screenshot could**: the window
was not frozen at all. It showed the places list with the cursor where he had left it, and a
banner reading `[object Object]` painted over the top of the previous message.

Three defects, all mine, all from the same morning's work:

1. **`[object Object]`** — a daemon error arrives as `{ code, message }` rather than as a string,
   which the shell documents and which the frontend already had a function for. I wrote a second
   one beside it. The two are one function now, in `format.ts`, and a browser test drives the
   failure with an object rather than an `Error` because a fixture that threw a string would have
   hidden exactly this.
2. **A failed mount left no audit entry**, so `cortex audit` showed the successful one and
   nothing else — R8 says allowed *and* refused. The red-team suite asserts it now.
3. **The message was noise.** `udisksctl` prefixes every failure with a complaint about not being
   able to open `/dev/tty`; the actual answer is on the second line. An internal disk now gets a
   sentence that says a permission is needed, that this window never asks for one, and what
   command would work.

**What is not settled: whether the window also stopped repainting.** The overlapping text in the
screenshot is the signature of a region that was written and not cleared, and "it does not react"
is what a stale image looks like from the outside — the page was alive, idle and answering. One
contributing candidate was found and changed on its own merits: the breath layer kept
`will-change: opacity` after D59 stopped animating it, so the compositor was holding a layer for
something that never changes. Whether that was the cause is unproven, and the next report is the
measurement.

### 2026-08-21 — Drives that are not mounted yet

M6's mounts, and the surprise is how little of it needs udisks2. Finding the drives is
`/sys/class/block`, the disk above a partition, and udev's own database in `/run/udev/data` —
world-readable, `E:KEY=value` a line — so the first screen stays off D-Bus and the daemon stays
off a GLib main loop. Only the mounting shells out to `udisksctl`, and always with
`--no-user-interaction`.

Written against this machine and then reproduced in a fixture: a stick plugged in and unmounted,
three unmounted Windows partitions, an EFI partition, a recovery partition and six snap loop
devices carrying squashfs images that udev reports as filesystems. The scan offers the stick and
the Windows volumes and none of the machinery. `cortex mount` put the stick at
`/run/media/<user>/ESD-USB` with no authentication prompt, and `cortex unmount` put it back.

Three things worth carrying forward:

- **The dangerous rule had no test, and the break-check is what said so.** `power-off` on a disk
  that cannot be unplugged takes it off the bus while a running system is using it — one argument
  away from the ordinary case. The check lived inside `Surface::unmount`, where deleting it broke
  nothing any test noticed. It is `plan_unmount` now: a decision with a name, and deleting it
  fails `an_internal_disk_is_never_powered_off`.
- **A refusal that misdirects is worse than no message.** Refusing an agent's mount with
  `OutsideAllowlist` told it to add `/dev/sdb1` to `agent.write_allowlist` — which would not have
  helped, because nothing permits this. `Refusal::NotForAgents` says what is true, and it fixes
  the same misdirection in yesterday's `apps.set_default`.
- **An unmounted volume's `path` is a device node**, because there is no directory yet. Opening
  it means mounting first and going to *the mount point udisks2 answers with*. A path guessed
  from the label sends the window somewhere empty, and the guess looks right on a stick called
  ESD-USB.

### 2026-08-21 — The heartbeat, and what a running animation actually costs

The measurement D57 asked the next session to take, taken: **the stepped breath layer bought
nothing.** 116% of one core before, 110% after, window on screen and untouched. So the
explanation was wrong, and eight variants were measured before a new one was believed — the full
table is in D59. The short version is that none of the suspects did it: not the renderer (111%
on WebKit's GPU path, 111% under software compositing), not the backdrop filter, not the
gradient, and not the size of the animated element, because a **three-pixel strip still cost
71%**. Any running animation recomposites the whole window every frame.

The heartbeat now runs only while the daemon has something to index. Visible and idle, the
window measures **4.8–5.1% of one core** against 111%; hidden, 0.22%. The last five per cent
has not been chased — the next suspect is the caret blinking in the search field, which is an
animation like any other. §6 reads better for it
too: the layer exists to make background work legible, so a still layer is what "no background
work" looks like. Design system §8 gains a rule — no animation runs continuously in a resident
window — and it is a performance rule with a measurement behind it rather than a taste.

**And the commit that carried all this went out without the change in it.** A break-check —
delete the rule, watch the named test fail — ended with `git checkout` on the file, which
restored it to `HEAD`, which did not yet have the rule. The docs, the tests and the design-system
entry were committed; the four lines they were about were not. What caught it was the *number*:
the shipped build measured 111.5% again. The perf gate would have caught it too and was not
re-run after the restore. **After a break-check that reverts a file, run the gate again before
committing** — the whole point of the exercise is that the file has been changed twice.

Two notes for whoever measures next. **The CPU tool read 0.0% for a process spinning a full core
the first time it was pointed at one**, again, for a different reason than last session: the
spinner's output went to the journal and it never spun. Point it at `yes > /dev/null` in a
scope of its own before believing anything it says. And **`install-user.sh` builds `dist`
without the test harness**, which is exactly the trap `package.json` documents: an `npx
playwright test` straight after an install times out on every test waiting for a row. Use
`npm run test:ui`, which rebuilds first.

### 2026-08-21 — Öffnen mit, and the desktop as the source of truth

The last session's work — the measured scroll lead and the stepped breath layer — was finished
but never committed. It is committed now, unchanged, and the suite was green before and after.

Then the next thing M6 is missing: **"Öffnen mit"**. What it does is above; what is worth
carrying forward is how it was checked. Reading the specifications gives a plausible
implementation, and a plausible implementation was wrong in three places that only a comparison
with the machine could show — `xdg-mime query filetype` for the type, `gio mime` for the default
and the order, twenty-five file types. It now agrees with both on all twenty-five. **A desktop
integration that has not been diffed against the desktop is not finished**, and the diff took ten
minutes.

Two defects found on the way, neither of them in the new code:

- **A browser test failed seven runs in ten under parallel load**, and it was not the test. The
  chooser reset its cursor in an effect, which commits one frame with the old position still
  highlighted — the same shape as the bug D56 paid for, except that Enter in that frame opens an
  application nobody pointed at. The cursor is derived during render now and carries the list it
  is a position in.
- **`the_places_are_the_same_through_all_three` compared free disk space across three calls.**
  It failed once during this session's full run and could not be reproduced afterwards, which is
  the worst way for a test to fail. Capacity is measured per call, so three identical numbers
  were never the property being asserted; it asserts that each surface reports one, and compares
  the rest exactly.

**And a defect found while measuring, not while reading:** every application opened from the
overlay — by Enter, which predates this session, as well as by "Öffnen mit" — was a child of the
overlay's systemd service, and therefore in its cgroup, and therefore killed by the next restart
of it. Launches go through `systemd-run --user --scope` now. The mechanism is measured in both
directions (a plain child dies even after `setsid`; a scope survives) and the wrapping is
asserted by three tests, but **the end-to-end proof is still to be done by hand**: open something
from the overlay, restart the overlay, and see whether it stays open.

And one thing found in passing, not fixed: **`fs.list` accepts a `glob` parameter and ignores
it.** The field is on the wire type, documented there, and no implementation reads it — so a
caller that filters a large directory with it gets the whole directory back and no error. Either
it works or it is not a parameter.

And one measurement to distrust: **the perf scroll gate fails reproducibly while the Rust suite
compiles in parallel** and passes on an idle machine. It measures frame time, so that is the
tool being right about a busy machine — but anything that runs both at once will see a red gate
that means nothing. It is the same trap §1 fell into.

Still not measured, both needing the window on screen and therefore the owner: what the stepped
breath layer costs against the 116% it replaces, and whether the scrolling feels right in
WebKitGTK. Dragging a row out into another application is still untried.

### 2026-08-20 — "es laggt": four things that were costing frames

The owner used the overlay and said it lagged. The measurements said 2 ms. When those two
disagree the measurements are wrong, so this is what they were missing.

1. **The breath layer was a JavaScript animation.** Framer Motion drives values from a
   `requestAnimationFrame` loop, so an animation with `repeat: Infinity` is a callback on every
   frame for as long as the window is open. Design system §6 says ambient layers are
   compositor-only, and this violated the rule it was implementing. It is a CSS keyframe
   animation now, and the render benchmark — which never claimed to be measuring this — dropped
   from 9.7 ms at p95 to **2.4 ms**.
2. **Holding an arrow key fired two round trips per row.** The context sidebar and the preview
   each request per selection, so thirty rows a second is sixty requests, fifty-nine of them
   about rows nobody looked at. The list still follows the key immediately; the panes follow the
   list once it settles, 120 ms later.
3. **Every keystroke pinged the daemon** to say a human was present. The first keystroke says
   that; the rest repeat it. Throttled to once every five seconds.
4. **`backdrop-filter: blur(24px)` over the whole window.** §8 permits it on the overlay
   container and nowhere else — but permission is not a requirement, and a backdrop filter is
   recomputed whenever anything behind it changes. It is behind `--hc-blur`, which is 0.

And the environment: WebKitGTK's DMABUF renderer with the proprietary NVIDIA driver is a known
bad pair. `WEBKIT_DISABLE_DMABUF_RENDERER=1` is set in the service unit, with a comment saying
to remove it and compare.

**What this does not settle.** None of the four was measured *in the real window* — three are
"obviously wasteful whatever the number is" and the fourth is a known driver problem. The bridge
now logs any daemon call over 50 ms and any preview over 50 ms, so the next report of lag can be
answered from the journal instead of from reasoning.

### 2026-08-20 — M6: selecting several things, and doing something with them

The half that decides whether this can replace a file manager. 745 tests.

**The shared trash (D45).** Deleting and overwriting put things in `~/.local/share/Trash` with
a `.trashinfo` each, or `$topdir/.Trash-$uid` for another filesystem — never a copy across a
boundary to delete something. The info file is written before the move: a crash between them
must leave an entry pointing at a file still in place, rather than a file nobody can restore.
The trash picks the name, so the journal writes a placeholder and rewrites it, which is the one
place a journal row is updated rather than appended.

**Copying (§4).** Measure first so the bar has a denominator; check for cancellation inside the
byte loop so a large file stops within a chunk; write to a temporary in the destination and
rename, so an interrupted copy is absent rather than a plausible half. Tested against a running
daemon: a cancel sent over the socket reaches a copy in flight, and nothing partial is left.

**Who the allow-list is for (D44).** It gates agents. The first version ignored the actor, which
was harmless while only agents mutated anything and absurd once the GUI grew a file manager —
with the shipped configuration the owner could not move his own file. The red-team suite is
unchanged and still passes, which is the evidence the gated path stayed gated.

**Four bugs found by tests that were about something else:**

1. `copy()` reset its progress for every file, so a batch counted from zero each time — the bar
   would have restarted per file, and the batch cancellation test failed because `files_done`
   never passed one.
2. Ctrl+A was taken by the overlay even while the caret was in the search field, so selecting a
   query to replace it selected every file instead. A browser test found it by no longer being
   able to clear the input it had just typed into.
3. Enter was taken by the overlay while the rename field was open, so the form never submitted
   and the rename silently did not happen.
4. `mkdir` at the root produced `//name`.

And one that was mine in the test rather than the code: `fill("")` does not clear a controlled
React input in this Playwright version — an empty insertion fires no event — so the test now
clears the way a person does.

### 2026-08-20 — §1 was measured wrong, and the hotkey never worked

Two corrections, both found by somebody actually using the thing.

**The cold start is 265 ms, not 418, and §1 passes.** The original measurement ran while the
same machine was compiling the workspace and running three test suites — it measured the load.
Ten runs on an idle machine: median 265, min 260, max 280, ten of ten inside the budget. D41 is
rewritten around the correction and the GTK4 question it raised is closed. The resident model
stays, because that is what a launcher is, and no longer because of a budget it turned out to
meet.

The same mistake had already appeared once that week — the frontend's render test failed in the
parallel Playwright run and passed alone — and it was diagnosed correctly there and not here.

**The global hotkey could never have worked** (D42). A Wayland client cannot grab a key; the
plugin falls back to an X11 grab that, with XWayland running, *succeeds*, lands in XWayland and
never fires. The overlay logged "summon with the hotkey" and the first person to press it found
nothing happening and nothing in the journal. It now registers nothing under Wayland and prints
the compositor line to paste; summoning is `hippocampus --show` through the single-instance
plugin.

**And a footgun found by walking into it:** a bare second invocation used to present the window,
so a measurement loop opened it ten times on somebody's desktop. Only `--show`, `--toggle` and
`--hide` move the window now.

Everything is installed as systemd user services — `scripts/install-user.sh`, the daemon on
`default.target` and the overlay on `graphical-session.target`.

### 2026-08-18 — M4 §1 measured, and it fails

**Superseded — see the entry above and D41.** The 418 ms below was measured on a machine that
was compiling at the time. On an idle machine the cold start is 265 ms and §1 is met. The entry
is left as it was written, because a progress log that quietly edits what it believed last week
cannot be trusted about what it believes today.

The overlay's cold start is **418 ms** against a 300 ms budget. It is recorded as red (D41).

Getting the number at all took finding a mistake in the instructions: `cargo build --release`
produces a Tauri binary that starts and then shows *"Could not connect to localhost"*, because
`tauri-build` decides dev-versus-production from whether the Tauri CLI is driving the build and
not from the cargo profile. A plain cargo build embeds the dev server's URL. Both READMEs now
say so; `npm run tauri build -- --no-bundle` is the command.

**The decomposition is the useful part**, and it took three instrumented builds to get:

| part | median |
|---|---|
| Tauri and GTK starting, before `setup()` runs | 171 ms |
| the webview process, until the page navigates | 145 ms |
| the page, of which 43 ms is parsing the document | 102 ms |
| connecting to the daemon | **under 1 ms** |

316 of the 418 milliseconds are gone before any line of this repository executes. Removing
Framer Motion — the largest dependency in the bundle — would buy tens of milliseconds against a
118 ms overrun that is not the frontend's. The two real options are the resident-process model
the launcher design already implies, or D4's GTK4 fallback; both are written up in D41 and
neither is chosen.

**A note on how this was measured, because it cost the owner some patience:** the overlay was
launched two dozen times in a row from this session. It turned out those windows were appearing
on his desktop, not vanishing into an unmapped void as an earlier session had recorded. Anything
that starts a GUI here is visible to whoever is sitting in front of the machine.

### 2026-08-18 — M4: the thumbnailer, in a process that is meant to die

`cortex-preview`, the XDG thumbnail cache, and the progressive contract D14 describes. 652
tests. §5 measures **146 µs** for the cached thumbnail of a forty-megapixel photograph against
a fifty-millisecond budget, and §6 kills the process mid-render and finds search unaffected and
the shared cache uncorrupted.

**The cache is shared, and that is most of the work.** Getting the naming right — MD5 of the
percent-encoded URI, under `normal`/`large`/`x-large`/`xx-large`, with `Thumb::URI` and
`Thumb::MTime` as `tEXt` chunks — is what makes a thumbnail this renders one that Nautilus finds
and does not re-render. A thumbnailer that got it wrong would not fail; it would quietly stop
sharing, which is worse.

**Two things had to be written out rather than depended on.** The `image` crate writes no text
chunks, so the PNG header is assembled here and the encoder's output spliced in after it — a PNG
is a signature followed by chunks, and inserting chunks between two of them is concatenation.
And PNG's CRC-32 is four lines of arithmetic that did not justify a compression crate.

**The JPEG path is the whole of D14's quality rule.** `image` 0.25 decodes JPEG with zune-jpeg,
which produces full-resolution pixels and nothing else — so a 128 px preview of a 40 MP
photograph would decode all forty megapixels first. `jpeg-decoder` can be told to scale in the
DCT domain, and there is a test that fails if the two paths ever cost the same.

**A test of mine was wrong and the code was right.** The MD5 I wrote into the cache test came
from memory and did not match the implementation. Pinning it against values computed outside
this code — including the empty string and "abc" — is what a hash test should have been.

For the next session:

- **§1 still needs a human.** Everything else in M4 is measured.
- **Audio previews** are refused with a reason; the scope asks for waveform metadata.
- **The handlers are still inline on the async runtime** (noted since the first M4 session), and
  M4's keystroke budget shares those threads with the GUI now.

### 2026-08-18 — M4: the overlay, and four bugs that only a real browser could show

The first GUI. A Tauri window, a virtualised list, a preview pane, a context sidebar, a command
palette, and the breath layer. 623 tests across three suites: Rust, vitest, and Playwright
against a real layout engine.

**The layout tests are in a real browser on purpose.** M4 §4 is a claim about a box model, and
jsdom has none to be wrong about. Running them properly cost a fixture entry point — the app
wired to a fake daemon — and it paid for itself immediately: three of the four bugs above are
invisible to a DOM emulation, and the grid min-content one is invisible to a person looking at
the window unless they happen to try 400 px.

**The eighty-millisecond budget is decomposed rather than claimed.** The frontend's share is
measured here (2 ms median, under 16 at p95); the daemon's is measured in `bench/latency`; the
end-to-end number comes from the app itself. A test that measured a fixture and reported it as
end-to-end latency would be worse than no test, so it says which part it measured.

**What is measured is work, not frame pacing.** The first version waited on two animation
frames and reported sixty milliseconds — thirty-three of which were the display's, before any
of this code had run. The timing now stops when the DOM reflects the keystroke; the compositor
takes its frame afterwards regardless.

For the next session:

- **The preview subsystem is the rest of M4** (§5, §6): a separate thumbnailer process, the XDG
  thumbnail spec, progressive rendering, and a cap of 2048 px on the long edge. It is
  deliberately a separate process — preview generation touches untrusted bytes with third-party
  decoders (D14).
- **§1 needs a human for five minutes.** Run the overlay, press Ctrl+K, and read the cold-start
  line. The number is already being measured; nothing here can display it.

### 2026-08-18 — M3 finished: the brain tools, MCP, and three surfaces proven identical

The `brain.*` tools, `cortex-mcp`, and the tests that turn M3's Definition of Done from a list of
claims into a list of assertions. 559 tests.

**The hardest thing in this half was rule R10, and it was not obvious that it applied.** An
annotation an agent writes is not on the filesystem, and the index is a cache — so a rebuild,
which this project advertises as a supported operation, would have quietly deleted everything an
agent ever recorded. Assertions now go into the episode log and the graph is a projection of
them (D35). The first version stored the target's *node id*, which a rebuild renumbers; it takes
a test that actually rebuilds to notice that, and `assertions survive a rebuild` is that test.

**`brain.recall` is the tool the analogy is about.** Search finds the way in and the associative
graph supplies what belongs with it, so a cue that names one document brings back the note
somebody wrote on it and the file it was derived from — neither of which contains the words in
the cue. Verified against a live daemon, not only in a unit test.

**MCP is deliberately empty.** `cortex-mcp` is a table mapping one tool name to one method, plus
stdio framing (D36). The reason is not tidiness: a bridge that interpreted anything would be a
way round the policy engine, and the policy engine's whole job is to be unavoidable. The kill
switch therefore reaches the bridge too — with the surface off it refuses to start rather than
starting and refusing every call, because a client that has been shown a tool list has been told
the surface is available.

**Three surfaces, one answer, asserted byte for byte.** `three_surfaces.rs` starts a real daemon
and drives the socket directly, the `cortex` binary, and `cortex-mcp` over stdio, then compares.
It also checks that a refusal is the *same* refusal on all three — code 1002, tag
`outside_allowlist`, and a message that names the allow-list — because a surface that turns a
policy refusal into a generic error is unusable from the other side.

For the next session:

- **§1 has a caveat and it is written down.** The end-to-end task test runs without the embedding
  model, so its search step is lexical. The semantic arm is measured in M2's goldset. A human
  check through Claude Code proper is five minutes of work nobody has done.
- **The handlers are still inline on the async runtime.** That is fine now and will not be when
  M4's 80 ms keystroke budget shares those threads.

### 2026-08-18 — M3 begun: the safety spine

The policy engine, the kill switch, the audit log, the `fs.*` tools, dry-run plans and the undo
journal. 524 tests, of which 22 are new red-team and undo tests.

**The layer is a crate with no daemon in it.** `cortex-agent` takes a policy, a store and an
actor; the daemon is a fifty-line adapter over it. That is not tidiness — it is what makes
"every gated tool refuses without an allow-list entry" a table-driven test over three tools
rather than a claim, and it is what M3 §5 will lean on when the MCP server arrives.

**Three attacks the red-team suite actually caught while it was being written:**

- A `..` in a path is resolved *before* the allow-list is consulted, not after. The naive
  version compared the literal string and let `allowed/../secret/f.txt` through.
- A symlink inside the allow-list pointing outside it is the same attack with a different
  spelling, so the resolution is `canonicalize` of the deepest existing ancestor — which also
  handles the file that does not exist yet, which is every file a write creates.
- `/data-private` is not inside `/data`. `starts_with` on a `Path` compares components, which
  is why this one is correct rather than lucky, and there is now a test that says so.

**The undo of an overwrite was wrong, and the DoD test found it.** The inverse was a single
"restore from trash" step, which lands on a path that is occupied — by the file the overwrite
just wrote. Inverse steps are now stored in the order of the forward steps they undo and applied
in reverse, so an overwrite is undone by "delete the new one, then move the old one back".

**M3 §3 passes verbatim:** 100 files moved through the surface, `undo --last 100`, and a
canonical dump of the tree hashes identical to before. The dump covers every path, its length
and its content hash.

Two things worth knowing for the next session:

- **The trash is why undo does not need the index to hold content** (D34). An overwrite moves
  the old version into the state directory and journals the path; R10 stays intact and the
  database does not grow by the size of everything an agent touches.
- **A plan is bound to its call.** It carries a fingerprint of every target, is consumed on use,
  expires in two minutes and must describe exactly the operation presenting it — so a plan for
  `scratch.txt` cannot be spent on `precious.txt` (D33).

### 2026-08-18 — M2 finished: the GPU, the reranker, and a goldset that lied twice

CUDA installed at the owner's request, the cross-encoder built and delivered as a progressive
result update, and every M2 gate green. 472 tests.

**The GPU.** The machine had the card and the driver but no CUDA runtime; `cuda-libraries-13-3`
and `libcudnn9-cuda-13` fixed that, and the CUDA build of ONNX Runtime is now a second entry in
the registry (`cortex model fetch --gpu`, 191 MB instead of 11 MB). Measured in the running
daemon: **3.11 ms per chunk against 89 ms on the CPU** — a corpus that took two and a half hours
now takes six minutes. Query embedding fell from 3.9 ms to 1.1 ms.

The card is shared, and that turned out to matter more than the speed (D31). A local language
model held 11.7 GB of the 16 GB throughout, and without a memory cap a batch of 32 chunks asked
for 273 MB it could not get and the whole pass failed. Every model now runs under
`embedding.gpu_memory_mb`, and the reranker batches eight pairs rather than thirty.

**The reranker.** `mmarco-mMiniLMv2-L12-H384-v1`, the same twelve layers and 384 hidden units as
the embedder, chosen to match it rather than to beat it. Thirty pairs in **16.5 ms against a
150 ms budget**.

It is delivered the way the build prompt asks: the search answers immediately with what retrieval
found, and the reranked order follows on the same connection as a `search.update` notification
(D7). That needed the connection handler split into a reader and a writer so a spawned task can
answer while the loop keeps reading — the mechanism M4 will want for progressive results, built
once here.

**The goldset was wrong twice, and both times it was worth catching.**

1. *It measured the reranker as harmful.* MRR fell from 0.882 to 0.746 and top-1 from 86 % to
   72 %. The first cause was mine: I had substituted the reranked order for the retrieved one, and
   when a cross-encoder finds nothing it likes its scores collapse into a narrow negative band
   where the ordering is noise. Fusing the two orders instead (D29) recovered most of it.

2. *The rest was the corpus.* `goldset --explain q21` showed the reranker promoting a distractor
   over the judged answer — so I read the file. It contained the sentence *"Die Speicherkosten
   stiegen nach der Archivmigration schneller als erwartet"*, which answers the question. The
   corpus generator had been sprinkling sixteen real sentences through its documents, and I had
   built goldset topics on those very sentences, then judged every copy irrelevant. **The reranker
   had been right and the measurement wrong.** Generated distractors are now irrelevant by
   construction, with a test that asserts no real sentence appears in generated text (D30).

With that fixed, the reranker changes nothing at all: MRR 0.878 either way. That is now a
statement about the goldset rather than the model — its distractors are nonsense, and a
cross-encoder earns its keep against documents that are plausible *and* irrelevant. On clean
pairs the model separates right from wrong by eighteen points of logit, in both languages.

**One Definition-of-Done item is not met, and it is recorded rather than argued away.** German
queries are 14.8 % behind English against a 10 % allowance. The whole gap is three queries:
German → English retrieval fails (0 of 3) while English → German works (3 of 3). The cause is the
language bias measured earlier; the candidate fixes and what "revisit D6" would mean are at the
top of this file.

Two other things worth knowing:

- **A rebuild gives the new chunks the same row ids**, so the cheap watermark the embedding stage
  uses to avoid scanning every chunk saw no change and the stage slept through it — with 1 721
  chunks waiting and their vectors just dropped. The wake-up signal was a boolean, which the
  extraction stage consumed first; it is now a generation counter, so one signal reaches both
  stages.
- **usearch refuses a duplicate key** rather than replacing it, which crashed a goldset run over
  a leftover index. Adding a vector is now idempotent.

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

## Four things that were written, tested, and called by nothing

This session found the same defect four times, which is a rate worth naming rather than four
separate notes. Each was correct code with a test beside it and no caller anywhere:

| what | what it meant in practice |
|---|---|
| `decay_unreported` | a memory retrieved fifty times and never mentioned ranked exactly as well as one that had earned it — the counter-intuitive half of D76, implemented and inert |
| `prune_packs` | a table that only ever grew, holding a record of everything an agent was once shown |
| `recalled_with` | an edge type in the schema and the enum that nothing wrote, so memories never associated with each other at all |
| `note_session_call` | **the Inspector printed "0 Aufrufe" beside every session ever opened** |

The last one is the one to learn from, because it had a *user interface* built on top of it. A
panel was written to display a number, the number was always zero, and nothing anywhere failed —
the tests for the counter passed, because they called it themselves.

**What the four have in common** is that each was written from the specification, in the order
the specification lists things, and the call site belonged to a different layer that was written
earlier or later. The tests were written at the same moment as the function and therefore called
it directly, which is exactly the shape that cannot catch this.

**The cheap defence, and it is genuinely cheap:** `scripts/uncalled.sh`, which is that grep with
its lessons in the header. Three of these four came out of it in about a minute.

Two things about it are worth knowing before running it. The first version matched only `.f(`
and `::f(`, so it missed every function imported with `use` and called by its bare name — it
reported a clean sweep over four crates that in fact had sixteen candidates in them. **A tool
that finds nothing is indistinguishable from a codebase with nothing to find**, which is the
same failure this whole section is about, one level up.

The second is that it is a prompt and not a verdict, and the header says so: a query helper whose
only user is a test assertion is test infrastructure, something reached through a trait or
`serde` has no call site to find, and a library crate's public API may be called from outside
this tree. Sorting the list is the work; producing it is the easy part.

Running that sweep over the whole of M11 afterwards found four more, and sorting them was the
useful part. **The rule is not "delete everything with no production caller"** — a query helper
whose only user is a test assertion is test infrastructure, not a bug. The rule is: *code whose
purpose is to make something happen in production, and which production never calls, is a bug.*
By that line `needs_explicit_request` and `MemoryFilter::visible_to` were duplication to be
wired — the second was a filter `brain.memories` had spelled out by hand — and
`Store::agent_session`, `Store::memory_chunk`, `Store::pack_items` and `tokens::estimate` were
weight to be dropped, with their tests rewritten to assert behaviour instead. A pack that is
still there is a pack an outcome can be reported against, which is the only thing a pack is for.

The fourth was found by reading the panel. Where a function's *purpose* is to make a number
appear somewhere, the test that matters asserts the number where it appears — which is what
`a_session_counts_the_calls_and_the_refusals_that_belong_to_it` now does, over a real socket,
counting a real refusal.

## The pack was spending its budget four times

Found by reading what an MCP client actually receives, rather than what the daemon returns.

An MCP result carries the answer **twice** by construction — pretty-printed as the text block a
model reads, and verbatim as `structuredContent`. The pack carried its own text twice as well,
in `rendered` and in an item beside it. So every memory's text reached an agent four times, and
a 2 048-token budget cost something closer to eight thousand. Nothing failed anywhere; the
feature simply did not do the one thing it exists to do.

| | |
|---|---|
| A memory's text in one MCP answer | **4 copies → 1** |
| A 2 266-byte pack, whole answer | 8 601 bytes → **5 756** |
| Ratio, and it falls as a pack grows | **2.54**, gated below 3.0 |

Four changes, each removing one copy or one repetition:

- **`PackItem.text` is skipped on the wire.** What a caller needs per item is what it takes to
  report an outcome and to argue with a memory: the id, the arm, the kind, the scope, the path,
  the basis. The words are in the block.
- **The MCP text block is the rendered pack, and the field is *moved* rather than copied** — so
  `structuredContent` does not repeat it. Declared as a named field in the tool table, so the
  bridge still interprets nothing (D36); it moves one field it was told the name of.
- **The basis omits every count that is still zero**, and `arm`, which the item beside it
  already carries. "helped 0, noise 0, retrieved 0" on every item of every pack on a fresh
  install is the absence of an argument rather than one.
- **An empty pack is not recorded at all**, and recording is idempotent on the pack id — which
  is derived from the second, the actor and the task, so two identical requests inside one
  second collide by construction and that is a retry.

**What is left is the floor.** 2 266 bytes of pack and about 3 500 of per-item metadata for
fifteen items, and that metadata does not grow with the budget — so the ratio falls as a pack
gets bigger. The test asserts it stays under 3.0, because a rise means something started
repeating again.

**Two things this broke, both mine, and both worth the note.** A CLI that required
`basis.salience`, which now goes missing when it is zero — *absent means zero* is a convention
and a convention has to be written down, so it is, in the struct and in the two tests that read
it. And a stale `target/debug/cortex`: `three_surfaces` drives real binaries and `cargo test`
does not rebuild another package's, so a wire-format change passes the unit tests and fails an
end-to-end one against yesterday's binary. `cargo build --workspace --bins` first.

## M11 — where its Definition of Done actually stands

Twelve criteria. **Nine met, two partly, one waiting on the owner.** The two partial ones are
partial in ways worth reading rather than in ways worth rounding up.

| # | Criterion | State |
|---|---|---|
| 1 | A memory written once is returned by `brain.context` on all three surfaces, identically | **met** — `three_surfaces.rs`, against a real daemon and two real binaries |
| 2 | `rebuild` reproduces a byte-identical dump including supersession, deletion and what a memory earned | **met**, at the thousand the criterion names |
| 3 | The budget is honoured at 512, 2 048 and 8 192 tokens over 5 000 memories | **met** — asserted inside the benchmark loop, not only in its report |
| 4 | `brain.context` P95 under 120 ms | **met** — 8.97 ms, a factor of thirteen |
| 5 | MMR at λ 0.7 beats λ 1.0 on duplicates without losing distinct facts | **met** — 4 restatements and 1 distinct fact at λ 1.0, against 2 and 3 at λ 0.7 |
| 6 | The learning signal moves the ranking in all three directions | **met**, including the counter-intuitive one |
| 7 | Scope isolation per surface; no unrequested promotion to `global` | **met** — and the CLI's default had to be fixed to make the second half true |
| 8 | Supersession: absent, present with history, detected without deciding | **met** |
| 9 | The kill switch reaches every memory tool | **met** — asserted per tool over the socket |
| 10 | The Inspector renders at 400 px, costs under 1 % of a core, and gives the window back | **partly — the width, the give-back and keyboard-completeness are measured; the CPU cost is not** |
| — | *Not a criterion, and the one that decides whether any of this is used:* selecting a file in the ordinary window shows what agents have learned about it | **built** — the Inspector is where somebody audits the memory; the context panel is where they meet it without going anywhere |
| 11 | A human's deletion is immediate and survives a rebuild | **met**, live as well as in a test — and superseding is wired now too, which the first version of the panel had not been |
| 12 | A real task, twice, by a human, measurably better the second time | **not done — this one needs the owner** |

**§2, at scale.** Two tests, and they are about different things. The dozen-memory one is about
*shape*: every scope, every kind, a supersession, a deletion, a correction. The thousand-memory
one is the criterion as written — a thousand across five scopes and six kinds, a supersession
chain three deep so the replay has to resolve a memory that is itself superseded, a deletion, and
an earned score including a human overruling an agent. Rebuilt, dumped, compared.

**It replays in 0.55 seconds**, which is not a gate — the build prompt gives the replay no
budget — and is recorded because it is the number that would say the replay had gone quadratic.
A thousand memories taking ten seconds would be invisible at a dozen and ruinous at a hundred
thousand, and the test fails at ten seconds for exactly that reason.

**§5's numbers, now recorded.** On a seeded store of five restatements of one fact and three
distinct facts, at a 200-token budget:

| | restatements kept | distinct facts kept |
|---|---|---|
| λ = 1.0, pure relevance | 4 | 1 |
| λ = 0.7 | 2 | 3 |

Both pairs are pinned in the test rather than only compared, so a change that makes
diversification more or less aggressive has to edit that line and say why. Two restatements
rather than one is worth noticing: λ 0.7 is a trade-off and not a deduplicator, and the second
copy costing a slot is the price of not dropping something that only *looked* like a copy.

**§10, and a third thing M4 §7 asks for that it did not do at all.** Every button in the panel
was wired to `onMouseDown`, which a keyboard never fires — a `<button>` activated with Enter
raises `click` and nothing else. The tab strip, the kill switch, the delete confirmation and the
supersede flow were all mouse-only, on a surface added to an application whose Definition of Done
says every function is reachable without a mouse. Worse, the unit tests dispatched `mousedown`
directly, so they passed against a panel no keyboard could operate.

Fixed, and the arrows now move along the tab strip with the focus following the selection, which
is what a tablist does everywhere else and therefore what somebody will try. Two Playwright cases
drive the whole panel from the keyboard alone, in a real browser — "pressing Enter on a focused
button activates it" is a statement about an event loop and not about a DOM emulation.

**The width half is measured too:** fourteen Playwright cases open the panel
at 400, 600, 900 and 1400 px on three tabs and assert that neither the body, nor the panel, nor
any row overflows its own box — plus Escape returning the caret to the search field, and the
button and `Ctrl+Shift+A` being one control. **Those tests found a real overflow at every width**:
the panel reused the existing `.hc-facts` class, whose contract is one line with an ellipsis, so
every allow-list path inherited `white-space: nowrap` and pushed its box 166 px wide. Three
attempts to fix it with `overflow-wrap` and `min-width` failed because they never mentioned the
property that was actually wrong. The panel has its own class now — a panel that wraps is a
different thing from a sidebar that elides.

**The window's own gates still hold with all of this in it.** The full browser suite — 141 cases
including the perf specs — passes: cold start, keystroke to first row, the 10 000-entry
directory, 60 fps scrolling, the bounded DOM, and the heartbeat that runs only while there is
something to report. That is not the same as measuring what the panel costs *while it streams*,
and it is the half of M11 §10 that could be checked without opening a window: adding the
Inspector, the memory section in the context panel and the compose row moved none of M4's
numbers.

**And one thing that would have made the cost unmeasurable anyway, found by reading the hook
that exists to prevent it.** The panel polled four calls every two seconds regardless of whether
the window was on screen. A hidden GTK window keeps running its page — that is D59's whole
subject and `useVisibility`'s whole reason — so somebody who opened the Inspector and then
dismissed the window left four calls a second and a half running against the daemon until the
next reboot. It listens now, and the test that says so was checked against the version without
the guard.

**What the panel asks for, before anybody measures what it costs.** Two things were wasteful in
a way that reading the code makes obvious and reading a CPU number would not have attributed.
The Live tab re-read the last eighty audit entries *and* the last eighty refusals on every
two-second poll — with `args_json`, the largest column in that table and one it never displays —
for a daemon on which usually nothing had happened. `audit.recent` takes a cursor now, which is
`brain.diff`'s own argument applied to the other thing a client watches, and the panel appends
instead of replacing. The session list cannot be cursored the same way, because the counts
inside a session are the part that changes; it asks for twenty rather than fifty, which is what a
live view needs, and `cortex sessions` is where the rest of them live.

The cost half is still not measured. The panel does not poll while it is shut and does not poll a
tab that is not showing, and both are asserted. Nothing has measured what it costs while the
Live tab streams. This project has a recorded case of a hidden window burning a full core that no
amount of reading the code would have shown (D59), and the cgroup snippet at the top of this file
settles it in thirty seconds — **but it needs the window open, and the owner has asked not to
have windows appear unasked.** It is theirs to run.

**Two more gaps that are not Definition-of-Done items and matter anyway:**

- ~~**The semantic arm of `brain.context` has never run against a real embedding model.**~~ Run
  on 2026-08-24, and it works. See below.
- ~~**Consolidation has never run over a corpus of memories.**~~ Run, and it found that
  **every memory scored exactly 0.0 and stayed in Working for ever**: the scoring pass selects
  `kind IN ('file', 'dir')`, so memories were not scored low, they were not scored. D80 has the
  whole of it, including the part where adding the component did not work either because
  `Components::sum()` lists its fields by hand. Memories are scored now, `usefulness` is a real
  component of the shared model, and a memory's tier floor is Long-term rather than Cold. What
  remains open is D76's own question — whether usefulness predicts a human calling something
  useful — and that needs reports that only use produces.

## The semantic arm, against the real model

Run on 2026-08-24 with `multilingual-e5-small` on the GPU, in a throwaway root whose `models`
directory is a symlink to the one that is already downloaded — no network, nothing written to
the real state directory, and the reason for going to the trouble is that everything else about
the second collection had only ever been exercised with the hashing embedder, which has no
semantics to test.

Four memories, then two tasks chosen to share as little wording as possible with them:

| task | what came back first |
|---|---|
| *"how long does the file system observer delay before reacting"* | "the watcher waits three hundred milliseconds before it acts on a filesystem event" |
| *"warum laufen Vorschaubilder in einem eigenen Prozess"* | "thumbnails are decoded in a separate process so a malformed image cannot take the daemon with it" |

**The second one is the one that settles it.** The German task and the English memory share no
token at all — not one — and it ranked first. The first task is weaker evidence than it looks:
`before` is not in the stopword list, so the lexical arm had something to match on, and the
result was fused rather than purely semantic. Worth knowing before quoting it.

Two things this run also produced:

- **`cortex model` reported "0 vectors in the index" while four memories were embedded.** It
  printed `loaded.vectors.len()`, which is the file collection. It now prints both, because one
  number covering two files with two rebuild paths would make a full memory index beside an
  empty file one look like a half-filled single one.
- The whole exercise took under a minute once the runtime was right, and the runtime was the
  only obstacle: `device = "cpu"` asks for an ONNX runtime build that this machine has never
  downloaded, while the CUDA one it does have is what `auto` or `cuda` selects.

## Driving it by hand, and the four things that found

Ten minutes at the command line against a real daemon, after every test was green. It found four
defects, and none of them was reachable from a unit test.

**1. A memory about "every commit" was not returned for the task "committing a change."** The
tokeniser folds case and diacritics and does nothing else, so `commit` and `committing` are
unrelated tokens. With an embedding model the semantic arm covers it — and R5 means a fresh
install has no model, so a context tool that only works after a download is one that does not
work on the day somebody tries it. Task terms of seven characters or more now also match a
sixty-per-cent prefix of themselves: `commit`* for `committing`, `reposi`* for `repository`,
`Umbene`* for `Umbenennung`. Short words are left alone, where truncation would make a wildcard
matching most of the store. It over-matches on purpose — `commit`* also finds `committee` — and
the trade is not symmetric: one extra slot in a ranked list against never seeing the memory that
answered the question.

**2. A human could not correct an agent.** The guard that stops a client retrying a failed call
from doubling its own vote was written as "refuse any second report on a pack item", which also
silenced the human — the exact opposite of the weighting D76 is about. Watching a `noise` report
from a person land on a memory an agent had called helpful, and nothing happen, is what showed
it. They are two properties and they need two rules: the same *class* of reporter may not report
twice, and a human may overrule an agent. A correction reverses the agent's weighted delta and
its count before applying the human's, so the Inspector does not go on showing a "helped" that
the human has already contradicted.

**3. The CLI defaulted every write to `global`, even for an agent.** The flag's default was set
once, for a person typing at a terminal who has no agent scope — and it applied just as happily
to `cortex --as claude remember`, which silently widened exactly what the scope rules exist to
keep narrow. The default follows who is speaking now.

**4. `cortex rebuild` waited forever when extraction was switched off.** Not an M11 defect and
it was found by M11 work: the progress loop polls until `pending` reaches zero, and a queue
nothing drains never gets there. No content root is the *default* configuration, so this is the
first thing a new user would have hit. It stops and says so.

**And R10 verified against a live daemon rather than only in a test.** Six memories including a
supersession chain and a human's deletion, `cortex rebuild`, and the two listings compare
identical — with the deleted one still deleted. The daemon logged `memory layer replayed from
the episode log memories=8`: six writes, one supersession and one deletion, all of them
assertions.

## The M11 benchmark, and the two things it found

Written after the implementation rather than before it, which is the wrong way round and is
recorded as such. It found two defects in the first run, both of which a unit test could not
have.

**A 512-token budget produced a 540-token pack.** The items were fitted to the budget and the
block was rendered afterwards, and the block carries a title, a heading per section and a bullet
per item that no per-item count knows about. The overshoot is proportional to the number of
sections, so it only appears once a pack is full enough to have several — which is why every
unit test passed. The furniture is reserved out of the budget before anything is chosen now, and
the block is measured and trimmed from the end until it fits, so the promise is true by
construction rather than by arithmetic that a later change can drift away from.

**The gate failed at 163 ms against 120, and the extra line in the benchmark said where.** The
same pack with diversification switched off measured 7 ms. MMR is quadratic in the candidate
count and the first version made that far worse than it had to be: it built both trigram sets
*inside* the comparison, so a hundred and twenty candidates meant fourteen thousand set
constructions over the same sixty strings. The sets are built once per candidate now, the
running redundancy is carried forward instead of recomputed, and the ranking stops after forty
picks because a dozen will fit.

| | before | after |
|---|---|---|
| context pack, 2 048-token budget, 5 000 memories, P95 | 163.41 ms | **8.97 ms** |
| the same with `λ = 1.0` (no diversification), P95 | 7.08 ms | 7.96 ms |
| the `cortex-query` unit tests | 10.35 s | 0.32 s |

So diversification costs about one millisecond, against a budget of a hundred and twenty. The
last row is not a benchmark result and is worth having anyway: a test suite that takes ten
seconds to run is a test suite somebody stops running.

**One measurement that is a design claim rather than a number.** The context gate seeds a store
of its own instead of writing into the cached corpus index, because `brain.context` never
touches the file chunk index at all — the memory arm is a second collection (D75). Measuring it
beside a hundred thousand file chunks would have produced the same number while implying a
dependency that is not there. That the two do not interact is the whole argument for the second
collection, and it is now a property of the benchmark rather than a claim in a decision log.

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
| 2026-08-18 | M2 | Cross-encoder rerank, top 30 (GPU) | P95 17.48 ms | 150 ms | yes |
| 2026-08-18 | M2 | Embedding one chunk (GPU / CPU 8 threads) | 1.85 ms / 75 ms | — | — |
| 2026-08-18 | M2 | Embedding one chunk, whole daemon (GPU / CPU) | 3.11 ms / 89 ms | — | — |
| 2026-08-18 | M2 | Embedding one query (GPU / CPU) | 1.1 ms / 3.9 ms | — | — |
| 2026-08-18 | M2 | Recall@10, goldset, fused | 92.0 % | recorded | — |
| 2026-08-18 | M2 | Recall@10, goldset, with reranking | 92.0 % (MRR 0.878 either way) | recorded | — |
| 2026-08-18 | M2 | Recall@10, German against English | 85.2 % / 100.0 % | within 10 % | **no** |
| 2026-08-18 | M2 | `cargo test --workspace` | 472 passed, 0 failed | green | yes |

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

The Definition of Done is in `docs/01-BUILD-PROMPT.md`. Every item is now measured, and the one
that fails says so rather than being reworded until it passes.

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
| Recall@10 on the goldset, measured and gated | 92.0 % fused, MRR 0.878, top-1 86 % |
| Re-indexing re-embeds zero chunks, by call counter | yes — `re_indexing_an_unchanged_corpus_asks_the_model_for_nothing` |
| `cortex --rebuild-index` reproduces a canonical dump | yes — byte-identical over 1 005 lines, live, with every vector kept |
| Vector index rebuildable from `embeddings` without the model | yes — 122 137 vectors in 152 ms, live |
| Cross-encoder rerank of the top 30 under 150 ms | 17.5 ms P95 on the GPU, delivered as a `search.update` notification after the first answer |
| Recall@10 gated against regression | `bench/goldset/baseline.toml`, a 3-point drop on any breakdown fails the run |
| German within 10 % of English | **not met** — 14.8 % behind. Recorded rather than argued away; see the top of this file |

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
| 2026-08-18 | multilingual-e5-small, GPU | 100.0 % | 85.2 % | fused 92.0 % overall, MRR 0.878, top-1 86 %; by-name 100 %, lexical 100 %, semantic 96 %, cross-language 50 % |
| 2026-08-18 | the same, plus the cross-encoder | 100.0 % | 85.2 % | identical — see D30 for why this goldset cannot measure a reranker |
| 2026-08-18 | lexical arm alone (control) | 39.1 % | 33.3 % | 36.0 % overall — what the index answers with no model at all |
| 2026-08-18 | hashing embedder (control) | — | — | run `goldset` without `--model` to reproduce; it is a control, not a result |

**German is 14.8 % behind English, outside the 10 % Definition-of-Done §3 allows.** The whole gap
is three queries out of twenty-seven, all of them German questions with English answers; the
reverse direction is perfect. See "The one criterion that is not met" at the top.

An earlier run of this table reported 91.0 % with a 9 % gap. It was measured against a corpus
whose distractors contained real sentences, several of which answered goldset queries without
being judged relevant (D30). The numbers are not comparable and the older ones are not
trustworthy.
