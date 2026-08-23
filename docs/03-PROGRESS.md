# 03 — Progress

**This is the living state file. Read it first, update it last, every session.**
If you finish work without editing this file, the next agent starts from nothing.

---

## Current position

| | |
|---|---|
| **Active milestone** | M8 — Agent View, and the half of M5 it needs (pulled ahead at the owner's request, D62) |
| **Status** | M4 complete. M6 begun and paused. **M5 complete except its M7 half, and M8 on top of it:** salience with explainable components, decay, tiering, co-access edge formation, a consolidation pass with a human-readable report, `brain.graph`, and a three-dimensional read-only view of the result that runs live against the daemon and as a single self-contained HTML file. The semantic abstraction step is now built too (D68): concept nodes clustered from co-access and named from what their members' paths have in common. Only promotion into the sync outbox is left, and that belongs to M7. The embedding model is resident on the GPU and the vector arm of search is filling in. **881 Rust tests pass** under `cargo test --workspace` across 40 test binaries — 44 of them in the new `cortex-memory`, 23 in `cortex-store::memory`, 2 guarding the response path — plus 11 vitest in `ui-agent`. **A note on that number:** this file briefly said 399 and accused the previous entry's 812 of being unreproducible. The 399 was wrong: the counting command piped `cargo test` through `tail -60` before summing, so it added up the last few test binaries and silently dropped the rest. The previous entry was right. Check the measuring tool before disbelieving the measurement — this file already says that about latency, and it applies to counting. |
| **Last updated** | 2026-08-23 |
| **Blocked on** | nothing |

## Start here

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

## Where M8 and M5 stand — the newest work, read this first

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

**It waits behind M6's mandatory list.** Archives, drag-and-drop out of the window, network
locations and permission display are still missing, and the largest scope risk in this project is
that the memory features are more interesting to build than those are.
`docs/paper/EXPLORER-KICKOFF-PROMPT.md` is the session prompt when it is time.

### What to do next here

- **The abstraction step (D62).** It needs `embeddings` to stop being 0, which needs the model
  the daemon has not been asked to load.
- **The grace window shows up as a flat legend.** A five-day-old index has every node in `stm`
  and the tier legend reads `working 0 / ltm 0 / cold 0`. The snapshot's `basis` now explains
  this in words, but it is worth watching whether the explanation is enough.
- M6 is where it was; nothing in this session touched it.

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

1. **Archives** — extract zip and tar at minimum.
2. **Network locations**, which may be staged.
3. **Thumbnails in the list**, not only in the preview pane.

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
| — | Concrete peer protocol for federation | M7 |

---

## Session log

Newest entry on top. One entry per working session. Keep entries short and factual — what
changed, what was learned, what the next agent needs to know.

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
