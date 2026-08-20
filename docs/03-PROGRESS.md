# 03 — Progress

**This is the living state file. Read it first, update it last, every session.**
If you finish work without editing this file, the next agent starts from nothing.

---

## Current position

| | |
|---|---|
| **Active milestone** | M6 — Human View (pulled ahead of M5 at the owner's request, D43) |
| **Status** | M4 **complete**, all eight Definition-of-Done items met. M6 begun: the overlay browses directories, which is the part that makes it a file manager rather than a launcher. §1 was recorded here as failing at 418 ms; that measurement was taken while the machine was compiling, and on an idle machine it is **265 ms, ten runs out of ten inside the budget** (D41). 856 tests. |
| **Last updated** | 2026-08-20 |
| **Blocked on** | nothing |

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

**What is not fixed and is known.** The RPC read loop handles one request at a time per
connection, so a slow call still blocks whatever is queued behind it on the same socket. With
the three above fixed there is no longer a call slow enough for that to show, so it stays as a
noted limit rather than a speculative rewrite of request handling.

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

### What M6 still needs

1. **Mounting and unmounting via udisks2** — the drives are *listed* now, but a stick that is
   not mounted yet does not appear, and there is no way to eject one — and `xdg-mime` for
   "open with".
2. **Archives** — extract zip and tar at minimum.
3. **Network locations**, which may be staged.
4. **Thumbnails in the list**, not only in the preview pane.

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

1. **§1 has no number yet.** The instrumentation is in and reports through the command palette;
   a window that never maps cannot report it. One run on a real session settles it.
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
4. **A reader pool.** `Store` is still one connection behind a mutex.
5. **The 24-hour soak test**, deferred by the owner since M1.
6. **Real distractors for the goldset** — a few hundred written documents. It is the only thing
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
