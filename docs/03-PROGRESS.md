# 03 — Progress

**This is the living state file. Read it first, update it last, every session.**
If you finish work without editing this file, the next agent starts from nothing.

---

## Current position

| | |
|---|---|
| **Active milestone** | **M12 — The Governed Brain, specified 2026-09-05 (D98–D102), not built.** It is the next thing to build; its first step is origin + authority on every memory. M9 — Release, built on 2026-09-05 (D96, D97) as far as code can build it; its scorecard is the first section below. M7 built the same day (D93–D95). M10 done (D85–D88). M11, M8 and M5 as before. |
| **Status** | **2026-09-15 (second session), the installation caught up to the tree: five binaries and both units were dated 2026-09-06, so the window was asking a daemon that has no `stats.overview` for one — rebuilt, installed, store backed up (1.1 GB) and migrated to schema 9. `cortex overview` answers in real numbers now (2 657 files, 184 754 vectors at 100 % coverage, 19 186 episodes), and the memory is empty: 0 memories, 0 agent sessions, 2 tool calls. The consolidation pass had no trigger but a person and had not run for 23 days; it is a nightly systemd timer now (D116), verified. M6's mandatory list was checked live where the surface allows it (trash, undo, permissions, symlinks, xdg-mime, places); mounts, thumbnails, archives, drag and drop and SMB/SFTP are not claimed. M12's first step — origin and authority — is next.** **2026-09-15, the explorer's kit pass: the file rows, the column strip, the places column, the empty states and the materials are the kit's now. Two bugs came out of the pictures: every material class had been rendering *no* shadow at all (a token spelled out on top of a token — D112), and the material numbers were this window's at roughly half the kit's dark strength (D113). tsc clean, 76 vitest, build clean, browser suite 187/187 and the perf suite green — the material guard this pass was about had never run in the gate at all, because `test:ui` kept its own list of spec files (it is `playwright test` now). The kit's lit canvas was built, measured and taken out again: its drift cost four composited layers and 48 fps against 60 (D114), and its colours cost the minimalism the concept is built on (D115).** **2026-09-14, interface session two: the showcase's design kit is ported and the UI moved onto it (D110, D111).** **M9 is built as far as it can be without a second person: `cortex setup`, a `.deb` and an AppDir from scripts that need only what Debian ships, `docs/09-INSTALL.md`, and the hygiene gate run over every tracked file *and every patch in the history* under `cargo test` — see its scorecard. What waits on a human: the timed install by somebody else (§1), and the license (D13).** **M7 is built: the wire format, a derived outbox, two transports, a reference peer, and every one of its five criteria asserted by a test — see its scorecard below. What it does not have is a real peer, which is the Extension's to write as a `sync` plugin.** M10 landed in full on 2026-08-24 — all four steps: `view.open` with one row shape from every source (D85), the memory columns on every row — measured before they were built, and cheaper than the per-row queries they sit beside (D86) — and the places the index makes (D87). Step 2 is complete: its by-hand criterion was checked by the owner against their own index and the salience order held. M6's mandatory list is complete as of the same day: archives (`fs.extract`, D81/D82), thumbnails in the list (D83) and network locations (D84, staged — what is mounted is offered, mounting is not built). What is left of M6 is its own §3, a week of exclusive use by the owner. **1060 Rust tests (2026-09-06, all green — the 2026-09-05 suite was 1052/2 on a hygiene false positive, see the reviews section), 60 vitest (green whatever `NODE_ENV` the shell exports), 165 functional browser tests and 16 perf (run separately, on an idle machine — see the step 3 entry).** **M11 is ten of its twelve criteria in — see its own scorecard below before anything else, and every benchmark gate is green including the new one.** M4 complete. M6's mandatory list is done; its acceptance now needs a week of use rather than more code. **M5 complete except its M7 half, and M8 on top of it:** salience with explainable components, decay, tiering, co-access edge formatio... [truncated]
| **Last updated** | 2026-09-27 — the brain view (Gehirn), `brain.ask` with a local curator, backups at login/shutdown, federation live through the Extension's bridge, 44 projects filed (D119–D123; see the session log). Before that: 2026-09-15, second session (the installed system caught up to the tree; the consolidation pass has a trigger at last — D116); before that the same day, the explorer's kit pass — D112 the invisible shadows, D113 the material numbers, D114 the drift that was declined, D115 the flat canvas |
| **Blocked on** | nothing |

## Start here

**If you are picking this up after the second session of 2026-09-15, the one thing to know
first is that the installation was nine days behind the tree** — everything the window had gained
since 2026-09-06 (the start screen, the Inspector, the memory columns) was in the source and not in
`~/.local/bin`, which is the whole reason the window looked broken. `scripts/install-user.sh` closes
that gap; it builds the window through the Tauri CLI, so do not rebuild it with a bare `cargo build`.
Two files this session added: `packaging/systemd/hippocampus-consolidate.service` and `.timer` — without
the timer nothing consolidates, which is D116 and was worth 23 days of a stale index. Then read the
2026-09-15 (second session) entry in the log and M12's `Order of work`: origin and authority first.

**If you are picking this up after the kit pass of 2026-09-15, read three things in this
order:** the 2026-09-15 entry in the session log below (what the port of the rows, the strip and the
materials changed, the two bugs it turned up, and the canvas that was built and taken out again),
D112–D115 in `02-DECISIONS.md` (a material token is listed as an item, never spelled out; the
material numbers are the kit's dark theme; the canvas does not drift; and the canvas stays flat,
because a showcase's backdrop costs the minimalism this interface is built on), and §"Ein Kit
benutzen, nicht nachbauen" of the Hippocampus-UI skill. **The gate itself was the other finding:**
`test:ui` named its spec files one by one and `tests/materials.spec.ts` — the guard over exactly
those material numbers — was never in it, so the gate stayed green while the guard never ran; it is
`playwright test` now. **Nothing is left with the owner from that session**, and one thing is
deliberately open: whether the file view still reads as "not 100 % the kit" now that the coloured
canvas is gone — that verdict is the owner's eye, not a measurement. Pictures of the state as it
stands: `/tmp/hippocampus-shots/v11/` (flat canvas, the state that stayed) and `v10/` (the kit's
coloured glows, the state that did not).

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


## Where the interface stands — rebuilt 2026-09-14 (D108, D109)

The owner asked for the interface to be made better and clearer, and for a start screen that
summarises what is in the index and how the agents have been using it. Both halves are built and
tested. What has *not* happened is the owner looking at it — this entry is code and measurements,
not a review.

**The start screen.** One method, `stats.overview`, answers with everything the screen shows: the
index counts, the memory facts, how the agents have been using the brain, the last consolidation
pass, the daemon's own state, and two sentences the daemon writes itself (`headline`, `basis`).
`cortex overview` prints the same numbers in the same order from the same call, so a number that
looks wrong can be checked from a terminal. New: `crates/cortex-core/src/overview.rs`,
`crates/cortex-store/src/overview.rs` (622 lines of counting, ordered deterministically — the store
test found a real tie-order bug: `ORDER BY count(*) DESC` alone returns rows in arbitrary order),
the method and its types in `cortex-rpc/src/protocol.rs`, the daemon arm in `cortexd/src/server.rs`,
the Tauri command and `ipc.statsOverview` in the window.

**The shell.** `ui/src/shell.css` is new and loads after `app.css`, which was not rewritten: the
overlay keeps its grid and gains one column for the rail. `NavRail.tsx`, `Overview.tsx` and
`useOverview.ts` are new. The rail names two destinations (*Übersicht*, *Dateien*) and the Inspector
panel; the places list stays the path it always was, because a rail item for it would be a second
navigation model beside `browse.state.path` (D109). The window opens on the start screen, and typing
in the field leaves it on the first character (M6). The rail collapses to 58 px under 720 px and
keeps its glyphs.

The screen arrives on the window's own springs: one element after another on `spring("content")`
and `stagger(index)` from `motion.ts`, the same pair the panels use (§5). It was a CSS keyframe
first, which would have been a second motion system beside the one that already knows about reduced
motion — `spring()` and `stagger()` collapse on their own, so the screen needs no media query and
the animation cannot run at all for somebody who asked for less of it.

**The MCP test caught the new method.** `stats.overview` was served by the daemon and was neither an
MCP tool nor on `NOT_A_TOOL`, and `every_method_is_a_tool_or_deliberately_not_one` refused to let
that pass silently. The reason is now written down beside it: it is the window's summary, its two
sentences are written for a reader rather than a caller, it carries the daemon's own paths and
uptime, and `brain_report` is what an agent should read.

**The German was wrong, and running the thing is what said so.** `cortex overview` against a
throwaway root printed „Kein Agent hat die Oberfläche letzte 24 Stunden benutzt" and „Das Fenster ist
letzte 24 Stunden" — a label that reads fine above a number does not survive being put into a clause.
There is now `Overview::window_phrase`, built from `window_label`'s own arms, and a test that pins the
two to the same window length: 72 hours stays „in den letzten 72 Stunden" instead of becoming „in den
letzten 3 Tagen" beside a label that says hours. It did exactly that in the first version, and the
test was written to catch it.

**The file manager came next, and the type stopped being a fixed number.** The owner's second
reading of the shell was that it had been built *around* the file manager — the rail, the start
screen and the animation were new while the list, the panels and the place column still wore their
original sizes — and that the text did not fit its boxes: „in der suchleiste der text von der
größe nicht an das Feld angepasst“, and the face should be something like SF Pro Display.

- **One scale instead of forty-odd sizes.** Seven steps, each clamped to the window width (§3 of the
  design system has the numbers), replacing 43 fixed `px` declarations across `app.css`, `shell.css`
  and `tokens.css`. The four relative sizes left (`0.75em` and its neighbours) stayed relative on
  purpose: a mark printed inside a row should follow the row, not the window. At about 1024 px every
  step is the size it used to be; a narrow window gets the same relationships smaller and a wide one
  larger, which is what „responsive" has to mean for type rather than for boxes alone.
- **The field is a field.** It was a bare 15.5 px input on a transparent line — 21 px tall inside a
  48 px strip — because the text and the box were two independent numbers, which is exactly why
  they agreed at one window width and nowhere else. It is a recessed pill now (`--hc-well-fill`,
  `--hc-well-rim`, the material already in §2b) with a magnifier, a „Strg K" cap while it is empty and
  a way out of a query once one is typed; every measurement inside it is `em` against its own step, so
  the pill and its text are one decision. The strip's padding is `em` too and follows.
- **The header wraps at 400 px instead of hiding things.** Three `display: none`s (the sort label,
  the shortcut cap, the clipboard pill) were buying room for the field by subtracting what the header
  said; the room they bought was measured — 79 px of input, under the width the layout test calls
  usable — and the honest version is that the header takes two lines: the field first, the counts
  under it, right-aligned. Nothing is hidden, nothing is clipped, and the field is 234 px wide at
  400 px instead of 79.
- **The face is the one that was asked for.** `SF Pro Display` is first in the stack, so a machine
  that has it uses it. It cannot be shipped — SF is Apple's and not redistributable — so the stack
  falls back to `Inter Display`/`Inter`, which is installed here and is what the pictures show. The
  fallback is written down in §3 so a screenshot is not read as a Mac.
- **The file manager itself.** Rows carry a hover state and a selection that is a fill *plus* a 2 px
  accent bar (hover and selection used to be the same faint tint, so neither could be read); they are
  set one step below the panels' prose, so a name is no longer the same size as the sentence about it;
  and their rhythm is `em`, so the pitch follows the type. The place column is the same step and the
  same pitch — the sidebar and the list are two columns of one screen and the eye compares their
  rhythm whether or not the code does. The inspector's values are no longer the same dim grey as the
  captions they sit under.

**The third defect was in the columns, and only a picture could show it.** Photographing the file
manager (`/tmp/hippocampus-shots/v3`, throwaway spec, nothing asserted) turned up a heading that was
not over its own values: `Salienz` stood above `drwxr-xr-x`. Two causes, both real:

- The row never rendered the salience value. The heading named a column whose cell did not exist, and
  the permission string slid up into the gap it left — the heading was a claim about a value that
  never arrived.
- `hasSalience` was `rows.some((row) => row.salience !== null)`, and `!== null` is *true* for
  `undefined`: the column appeared even when no row carried a number. It is `typeof … === "number"`
  now, the value is rendered in the header's own order, and heading and values are given the same
  `em` tracks. Measured at 1440 px: each heading sits within 0.5 px of the right edge of its values.
  A test in `memory-columns.spec.ts` measures that, and also that the metadata cannot push out of its
  row — it sat in the row's `auto` track sized to its own max-content and ran out over the panel
  beside it once the columns had widths.
- A directory has no size in a listing, so the cell says `—`, the mark the inspector already uses for
  a number it does not have; a blank cell reads as a zero.

An attempt in between — moving the „neu" badge into the name cell, on the theory that it pushed the
permission column sideways — was wrong twice over and is recorded because it was reverted: the
metadata is right-aligned with fixed tracks, so a badge in front of the first track cannot move the
ones behind it, and `.hc-row-name` is what the tests, the screen reader and the row's accessible name
read as *the name*, where „neu content.png" is not one. `browse.spec.ts` caught it.

**Rendering it found two things the tests could not.** The screen was photographed at 1440, 1024,
700 and 400 px (`/tmp/hippocampus-shots`, made with a throwaway spec that asserted nothing and was
deleted again — a picture is not a test), and reading the pictures turned up two defects:

- The header's status line printed the daemon's *key* for the sort — „5 name" and „0 name" on the
  start screen — which is one English identifier in the middle of a German sentence on the screen
  that is looked at most. It now prints `SORT_LABELS[...]`, the same map the palette's action uses
  („Sortieren: Name"), so the two cannot drift into two words for one thing. One browser test
  (`browse.spec.ts`) pins it.
- At 400 px the 58 px rail was still printing its footer *values* with their keys hidden, which
  squeezed „0.4.1 · 2 h 14 m" and the write path into `/ho….` — noise where a glance wants a shape.
  The values go with their keys now, the agent chip keeps its dot and loses its word (the word
  stays as its `title` and its accessible name, so nothing is carried by colour alone), and the
  header drops its sort label so the search field keeps its width.

**Tests.** 181 functional browser tests — 15 new in `tests/overview.spec.ts` plus one in
`browse.spec.ts` for the sort label, the other 165
unchanged — 60 vitest, and 1081 Rust tests across the workspace, all green (`cargo test --workspace`:
1081 passed, 0 failed — 1079 before this work plus the two that pin label and phrase together). The
harness now opens on the browser unless `?section=overview` is asked for, for the same reason `?at=`
exists: a test about the list should not begin by leaving a summary.

The path was also walked end to end for real, not only under fixtures: a throwaway daemon
(`cortexd --root /tmp/hc-live2`), five indexed entries, `cortex overview` — which printed
`4 files, 1 directories, 0 chunks, 0 vectors (0 % covered), 5 episodes (5 in den letzten 24 Stunden)`,
the daemon's headline and basis, and `daemon 0.0.1 schema 9, up 8s`. That is the RPC method, the
daemon's counting and the CLI's printing, all three exercised against a real database. The
*temporary* root was thrown away afterwards; the installed daemon on this machine (`~/.local/bin`,
built 2026-09-06) is untouched and predates this work, so it does not yet answer `stats.overview`.

**What the new browser tests found,** all three of them real and all three in `shell.css`:

| Symptom | Cause | Fix |
|---|---|---|
| The screen scrolled sideways at 400 px, by 128 px | a card track `minmax(320px, 1fr)` in a 316 px column | `minmax(min(320px, 100%), 1fr)` |
| A session line ran 58 px past its card | `.hc-session-meta` was a nowrap flex row | it wraps, with a 2 px row gap |
| The allow-list chip ran 140 px off the screen | a path in a chip that could not break | the long chip wraps at the slashes instead of being cut off |

## Where the reviews of 2026-09-05 stand — answered 2026-09-06

Three reviews of the same day (`review/README.md`: a survey, a claims-verified second pass, and a
third by another model) named twelve findings between them. This is what happened to each. Every
"done" below was verified in this session by the command named; the numbers are in the last
column of the table and were taken on the reference machine, not read from a document.

| # | finding (review) | what was done | verified by |
|---|---|---|---|
| 1 | No CI; gates run only when somebody types them (2nd §1.3, 3rd F04) | `.github/workflows/ci.yml` — fmt, clippy, `cargo test` counted by summing every `test result:` line, `npm test`, Playwright; `bench-latency` on demand (D106) | the file exists; its commands are the ones run below |
| 2 | Rust suite red: hygiene gate flags `debian.org` (2nd §1.1, 3rd F05) | allow-list widened with the documentation hosts it lacked; error text no longer invites `--no-verify` (D106) | `cargo test --workspace --no-fail-fast` → **1060 passed, 0 failed** |
| 3 | `npm test` red under `NODE_ENV=production` (2nd §1.2, 3rd F08) | `NODE_ENV=test` pinned in `vite.config.ts` and the `test` script (D106) | `NODE_ENV=production npm test` → **60 passed** |
| 4 | Daemon `RssAnon` 1.5–1.6 GB against 250 MB, CUDA runtime resident (all three) | `embedding.unload_after_idle_secs` (default 600): the ONNX session is put down after idling, the index stays; a query while it is down is answered by two arms and triggers the reload in the background; the spec row now names its configuration (D103) | unit tests for the release/reload; `cortex model reload` measured at 1.58 s (why a query must not wait). **The effect on `RssAnon` is not yet measured — it needs the new binary running for ten idle minutes; see below** |
| 5 | Same-name disambiguation untested; no side-by-side; preview capped at 512 px, no zoom (all three) | `ComparePane` (two selected rows → both, differences marked), `QuickLook` (Space / Ctrl+Space, screen-sized up to 2048 px, fit / 1:1), `disambiguation.spec.ts` with six tests (D105) | `npm run test:ui` → **165 passed** (159 + 6) |
| 6 | Benchmark corpus inside the live index: 293 k synthetic files (2nd §4.4, 3rd F06) | `.brainignore` marker honoured by the first walk, the reconcile and the watcher; the generator writes it; written to `bench/corpus/data/` on this machine (D106) | walker tests in `indexer.rs` and `reconcile.rs`, including "a marker added later removes the tree on the next scan". **The live index clears at the next hourly reconcile of the new binary, or `cortex reconcile`** |
| 7 | Contention gate's budget derived from its own measurement (2nd §4.2, 3rd F10) | fixed 45 ms = 1.5 × the specified BM25 budget; the ratio to the run's own baseline is printed as information (D104) | `bench-latency` → 28.73 ms against 45 ms, 1.24× own baseline, PASS |
| 8 | Query embedding and rerank never measured (2nd §4.2, 3rd F11) | the bench looks in the daemon's model store when the environment is silent; query embedding gated at 40 ms CPU (D104); the rerank stays "not measured" on this machine because no cross-encoder has been fetched | `bench-latency` → **3.98 ms P95** (onnx, cpu, 8 threads), all 9 gates PASS |
| 9 | MCP tool table can drift silently from the RPC registry (2nd §3.2, 3rd F09) | `NOT_A_TOOL`: every method is a tool or excluded with a reason, asserted (D106) | `cargo test -p cortex-mcp` → 5 passed |
| 10 | No details view / column headers, no grouping; neither recorded as a decision (2nd §2.3, 3rd F07) | column headers over a listing, clickable, showing the order (`ListHeader`); grouping, Miller columns and grid **deferred and recorded** (D105) | `disambiguation.spec.ts` "the listing has column headers" |
| 11 | The brain has never been used: 0 memories, 0 sessions (all three) | **not code.** Still open; only the owner can do it | `cortex memories --json; cortex sessions` |
| 12 | 159 browser tests run against a fixture (2nd §2.4, 3rd F12) | **correct as designed**, nothing changed; the real-daemon coverage is `ui/src-tauri/tests/against_daemon.rs` and the Rust integration tests | — |

**Found on the first deployment, 2026-09-06, and fixed the same night (D107):** the embedding
stage rebuilt the whole vector index every five minutes (`expected=49257 present=49258`, twelve
seconds each) because a re-extracted file's deleted chunks left their vectors in the index and
nothing removed them; deleted ids are now noted in `dead_chunks` and their vectors dropped on the
next idle pass. And the idle release of D103 was never reached from the settled path the stage
takes on every idle pass — fixed; the first deployment ran twenty idle minutes without a release.
One measurement from that deployment is worth keeping: with the model resident on CUDA and only
84 chunks embedded since start, `RssAnon` was **213 MiB** — inside the 250 MiB budget. The
1.5 GB of the reviews was the runtime's pools after embedding 160 000 chunks, so the pools grow
with work done and the idle release is what gives them back.

Also fixed while in there: `respect_gitignore` was documented as honoured and was not read by
any walk. The config comment and `09-INSTALL.md` now say so; the key stays so configs keep loading.

**What waits on the owner, in order:**

1. **Restart the daemon on the D107 build, let it embed something, then measure.** After ten
   minutes with no search and nothing to index: `grep RssAnon /proc/$(pgrep -x cortexd)/status`,
   and `journalctl --user -u hippocampus-cortexd -o cat | grep -i "released after idling"` for
   the release line. Also confirm the rebuild loop is gone: no `rebuilding the vector index`
   lines after an edit under `~/Projects`.
2. ~~`cortex reconcile`~~ — done 2026-09-06: the index went from 102 900 to 2 191 files.
3. The two thirty-second checks that have been open since August: drag a row into another
   application; copy in the overlay, hide it, `wl-paste --list-types`.
4. **Use the brain for one real task through Claude Code, twice** (M11 §12). Still the one
   criterion that decides whether the most novel half of the project earns its budget.
5. Then M6 §3's week, the Inspector layout, the Extension bridge — the owner's order.

## Where M12 stands — specified 2026-09-05, not built

**Read `docs/paper/WHAT-2026-CHANGED.md` first, then M12 in `docs/01-BUILD-PROMPT.md`, then
D98–D102.** The research behind it: a three-round literature search over 2025–2026 (938
candidates, 230 fetched as text, final-round novelty still 79 % — the field is larger than what
was read). The library lives *outside* this repository, in a sibling directory
`hippocampus-research/agent-memory-2026/` whose `manifest.json` lists every source with its URL;
nothing from it is vendored here, and paths into it must not be committed (R9).

### What the reading changed, in one table

| finding | consequence here | decision |
|---|---|---|
| memory poisoning needs one write; content and lineage defences are malleable; origin-bound non-malleable authority is necessary and sufficient; consolidation collapses authority in 48/49 configurations | `origin` on every node, `authority` on every memory, inherited downward, raised only by a human or two independent origins; the policy engine refuses a gated write motivated by anything below `instruction` | D98 |
| "newer wins" presupposes a conflict key, valid-time = transaction-time, and newer ≥ older in trust; *changed* ≠ *wrong*; labelled state in the evidence packet is the one measured fix (A-TMA); update is five ops, not two (TARL) | `supersede` takes a reason; `state` derived from the log (`pending · current · historical · retracted · deleted`); `valid_from/valid_to`; file/tool/peer memories land `pending` | D99 |
| uniform decay is wrong across memory types; type-conditioned decay was the necessary mechanism (ScrubJay-MEM) | half-life per memory kind, defaults to be corrected only by the cold-tier-retrieval falsifier | D100 |
| most published memory gains were embedding confounds (MemDelta); retrieval dominates writing 14–23 pt (Yuan et al.); a local-first post-fusion frequency+decay boost failed to help NDCG (vstash) | a frozen gold set from real episodes, `bench-quality` one variable at a time, the four falsifiers run; **the salience boost leaves the ranking if it loses** | D101 |
| production failures are forgetting failures; recall benchmarks do not test being commanded to forget | the forgetting suite, non-revival through rebuild and re-remember; hash-chained audit log | D102 |
| no model in the write path matches or beats extraction and summarisation (Yuan et al.); the index-as-cache / lossless log shape is what the best context managers converged on (Scroll) | confirmed; D75 and R2 unchanged, now with external numbers | — |
| agent-organised markdown stores erode; organisation does not convert into answers (Zhou et al.) | confirmed; the daemon organises, the agent writes nodes (D74) | — |

### Order of work, and why

1. **Origin + authority** (D98): two columns, one propagation rule, one policy check, labels in
   the pack. Cheapest change, strongest evidence. Criteria 1–4 of M12.
2. **Reason + state + validity** (D99): criteria 5–8. Together with step 1 this is the
   "governed" in the milestone's name.
3. **The gold set and `bench-quality`** (D101): criteria 11–12. Needs the owner's episodes, so it
   needs the owner to have *used* the thing — the same dependency M6 §3 and M11 §12 have.
4. **Forgetting suite + audit chain** (D102): criteria 7, 10.
5. **Per-kind half-lives** (D100) and **adaptive fusion**: criteria 9, 13 — *after* 3, because
   3 decides whether they stay.
6. **Fitted weights** (criterion 14) last; it is advice, and it needs the log to have history.

### Step 1 — origin and authority: the file map, written before the build
*Written 2026-09-15, at the start of the build. D98 is the decision; criteria 1–4 of M12 are the
target. Read this before touching anything, so the migration is designed once.*

**What exists.** `nodes` carries `path, parent_id, mime, size, mtime, ctime, inode, dev,
content_hash, tier, salience, pinned, zone_id, first_seen, last_seen, extract_state, extract_note,
label`. `memories` carries `node_id, scope, …` and **no** authority, basis or validity. The schema
files are numbered `0001`–`0009`, so step 1 is **`0010_origin_authority.sql`**. Nothing in
`cortex-*` matches `authority`, `motivated_by` or `untrusted` outside comments — the patch is as
unbuilt as the documents say.

**The pattern to follow, already in this codebase.** `cortex-store/src/graph.rs` says it for
`annotations`: *"The row in `annotations` is derived; the authority is the `annotate` episode it came
from."* Origin and authority are the same shape: written into the episode, projected onto the node
and the memory, and reproducible by `cortex rebuild` (R10, criterion 4's second half, criterion 6).

**Where each piece goes.**
| piece | file |
|---|---|
| two enums, `Origin` and `Authority`, with `Ord` so "lowest wins" is one call | `crates/cortex-core/src/model.rs` |
| `origin` on `nodes`, `authority` + `basis` on `memories`, `valid_from`/`valid_to` for step 2 | `crates/cortex-store/src/schema/0010_origin_authority.sql` |
| the projection: episode → node, and the rebuild path that reproduces it | `crates/cortex-store/src/memories.rs`, `crates/cortex-store/src/graph.rs` |
| the write path: defaults per origin, the imperative detector, inheritance from `basis` | `crates/cortex-store/src/memories.rs` + `crates/cortex-agent/src/memory_tools.rs` |
| the tool surface: `origin`/`basis`/`motivated_by` fields, and the MCP descriptions that say to send them | `crates/cortex-core/src/tools.rs`, `crates/cortex-mcp/src/tools.rs` |
| the refusal: a gated write whose `motivated_by` memory is below `instruction` | `crates/cortex-core/src/denylist.rs` and the gated path in `cortex-core/src/tools.rs` |
| the label in the pack, plus the `omitted` accounting | `crates/cortex-agent/src/brain_tools.rs` (`brain.context`) |
| the human path: `--as human` on the CLI, the Inspector's confirm | `crates/cortex-cli/src/main.rs`, `ui/src-tauri/src/lib.rs`, the Inspector |

**Four things that will bite, in the order they will bite.**
1. **`origin = human` must be unforgeable from the socket** (criterion 1). The socket knows its
   actor — `--as` and the MCP server's `--actor` — so the rule is: `origin` is derived from the
   *connection's* actor, never from the request body. A request that asks for `human` gets the
   policy error, not a quiet downgrade, because a silent downgrade is indistinguishable from a bug.
2. **The imperative detector is a heuristic and must be named as one.** "Anything that parses as an
   imperative aimed at an agent" (D98) decides `untrusted`. It will have false positives; it must
   therefore only ever *lower* authority, never raise it, and its verdict belongs in `why` where a
   human can overrule it. A false positive costs a promotion step, not a wrong action.
3. **`basis` is what makes criterion 2 testable**, and it is the *only* input to inheritance —
   derived from `supersedes`, from the `about` path, and from anything an agent names. If a memory
   cites a `file` chunk, the file's `observation` ceiling applies whatever the request said. Three
   levels deep is the test (criterion 2's chain of three).
4. **Rebuild is the criterion nobody remembers to test.** `cortex rebuild` must reproduce both the
   labels and any elevation, because the elevation is an episode and the label is a projection. If
   the derivation lives anywhere else, criterion 4's second half fails and nothing else notices.

**First slice, and what "green" means for it:** the two enums, the migration, and
`origin = f(connection actor)` with a test per surface (MCP, socket, CLI-as-human, Inspector).
Criterion 1 is that slice, and it is worth finishing alone rather than half of four things.

### What waits on the owner

- Real use, so that the gold set is real (step 3). A seeded gold set is the fallback and says so.
- Thirty seconds in the Inspector confirming one elevation, once step 1 exists — the first
  `actor = human` authority episode.
- Whether `wrong` is a tool argument or an Inspector button (D99's revisit trigger) — decide
  after the first month of supersessions, not before.

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

### 2026-11-02 — The embedder spent eight times its share, and nobody told it the owner was there (D129, D130, D131)

Third defect, found by measuring after the first two: the queue was blocked at its head (D131).
`model.status` showed `embedded_this_run: 0`, `stale` rising in steps of 64, and 65 % of a core —
every claim was a 3.2 s scan for the same 64 chunks of one binary file, which a bounded read cut
back to its first malformed byte so they could never be read back. Fixed at both ends: a cursor
through the queue, and `read_text` drops only an incomplete trailing character.

The owner's desktop stuttered several times a day; one of three causes was `cortexd`. Two defects
together: the embedding stage's duty cycle counted wall time while the ONNX pool ran eight threads
(D22 asked for the share to be divided, it was not — D129), and only the overlay and interactive
requests signalled activity, so gaming or watching a video ran the idle budget (0.8) under the
person (D130). Fix one: `Embedder::cpu_threads` + `throttle::share_per_worker`. Fix two: a new
binary `cortex-presence` (crate of its own, `wayland-client` + `wayland-protocols`, pure Rust) that
pings while `ext-idle-notify-v1` reports input, with `hippocampus-presence.service`; install,
uninstall, `.deb` and AppDir scripts list it. Tests: two throttle tests, four presence tests.

Measured on the owner's index on the way, for whoever reads `cortex overview` next: "940 444
vectors (61 % covered)" compares rows of `embeddings` (one per distinct text, D26, and kept for
texts no chunk has any more) with chunks. The real backlog was 94 890 chunks over 46 429 distinct
texts. Copies of a repository (git worktrees) are *not* embedded again — D26 holds. The overview's
coverage figure is misleading and not fixed here. Also seen: the nightly pass last ran 16 days ago
(`last pass` in the overview) although the timer is installed — not investigated in this session.

### 2026-10-12 — A deleted directory no longer rebuilds the whole vector index (D128)

The owner's machine lagged with btop showing little: `cortexd` at one core and 43 MB/s, IO pressure
11–16 % full, swap thrashing. The log showed `rebuilding the vector index from the store` every four
to five minutes since login, `expected` shrinking by 2–4 k each time (825 148 → 814 950). Cause: a
file or directory removed from disk goes through `remove_path_on`, whose `DELETE FROM nodes`
cascades the chunks away without the `dead_chunks` note of 0009 (and leaves their contentless FTS
rows behind); the same held for the destination row a rename overwrites. The reconcile then saw
`present > expected` and refilled ~820 k vectors. A build directory churning (a Fabric gametest)
kept it going. Fix: `bury_chunks_on` notes the chunks and drops their FTS rows before both deletes.
Tests: `removing_a_directory_notes_its_chunks_as_dead`, `a_rename_onto_an_existing_file_notes_the_overwritten_chunks`
(the first fails without the fix); cortex-store 192/192. FTS rows orphaned before this stay until
a `--rebuild-index`; they match nothing a search can show, since the join to `chunks` drops them.

### 2026-10-07 — `brain.ask` asks the peer too (D126)

The owner wants the peer to be the main memory and this Cortex its local part. New plugin kind
`recall` (`cortex-core::plugins`), `cortex_sync::recall::Recall`, and `plugin::exchange` pulled out
of the sync transport so both kinds share one process/deadline rule. `federation.recall_plugin`
(unset by default) names the declaration; `brain.ask` starts it before searching and appends up
to three `peer` items, rendered as `peer: <location> — <title>`. Not stored, not given to the
curator, `brain_context` unchanged. Tests: four in `recall.rs` (kind check, limit and size cut,
question reaches the plugin, deadline), a config default test. The Extension ships the plugin.
Same day, D127: `cortex-mcp` reconnects after a daemon restart instead of answering "Broken pipe"
for the rest of the session (one retry, only when the request was never written). Installed:
`cortexd` and `cortex-mcp` only (the UI in ~/.local/bin is still the 2026-09-27 build).

### 2026-09-27 (later) — Liquid: the interface redone at the owner's request

"Das GUI ist noch sehr eintönig und hässlich — liquid glass, kleine Animationen, minimalistisch."
One layer, `ui/src/liquid.css` + `tokens.css` §2c (D125, design system §10): a lit static canvas,
glass islands with gaps, liquid pills, a sliding nav pill (framer-motion `layoutId`), tinted status
chips, one-line result rows, opaque inspector sheet, one-shot motion only. Fixed on the way: result
rows printed two lines over each other; rounded rows bent their zone border into a bracket; the
inspector let the list show through. Tests: vitest 77, Playwright 194/194 (twice), perf 16/16.
Not measured yet: the real window's idle CPU with the new surfaces — the cgroup snippet above,
with the window open and still, should read near the 4.8–5.1 % of D59.

### 2026-09-27 — The brain view, the curator, backups, and an index that was one third data

The owner's brief: finish the explorer, make the agent half actually work, a second tab with the
brain in 3D, live logs and a heatmap that cost nothing in the background, a local embedding plus a
small decision model as curator (and an honest evaluation of whether that saves tokens), a robust
load-aware pipeline to the Main Brain, backups at every boot and shutdown, and every harness on the
machine told to use the brain. This entry is the Core half; the private half is in the Extension.

**What was broken, found by looking rather than assumed.** The root disk was 100 % full (632 MB
free) — `target/debug` alone was 79 GB of build cache; removed. Four sessions of work (2026-09-06 to
09-17) sat uncommitted; tested (1095 Rust, 77 vitest) and committed as a checkpoint. The installed
daemon was schema 9 against a schema-10 tree. The MCP server was *disabled* for the home directory
in Claude Code and pointed at a build path, and no other harness had it at all — hence 0 agent
sessions and 1 memory. The reranker had never been fetched. And a third of the index was data
(D122).

**Built.**
- `brain.activity` — the live pulse (D119); `cortex pulse` prints it.
- `brain.ask` / `brain_ask` / `cortex ask` — the curator (D120), with `[curator]` config (loopback
  only), `scripts/hippocampus-curator` and `packaging/systemd/hippocampus-curator.service`.
- `store.backup` / `cortex backup [--full]` — light and full backups with rotation (D121).
- `brain.services` — curator awake/asleep, backup ages, federation state, for the status line.
- Retrieval hygiene: data extensions metadata-only, start-up requeue of content the rules forbid,
  entry files as project markers, every child of a projects folder a zone (D122).
- The window: **Gehirn** as the rail's third destination (`BrainView.tsx`, `brain.css`,
  `ui-agent/src/brain3d.ts`) — status line, 3D brain rendered on demand (D123), heatmap, agents,
  live log, and a box to ask the curator. `hippocampus --brain` / `--files` open the window on a
  section and toggle it away when it is already showing there.

**The curator, measured** (12 questions, expected file named per question, `cortex ask`):

| run | hit@1 | hit@3 | latency | answer size |
|---|---|---|---|---|
| ranked, before D122 fixes | 4/12 | 5/12 | 151 ms | ~370 tokens |
| model (Qwen3-4B), before | 3/12 | 4/12 | 1157 ms | ~305 tokens |
| ranked, after | 5/12 | 8/12 | 66 ms | ~341 tokens |
| model, after | 6/12 | 7/12 | 1201 ms | ~259 tokens |

Reading only the one right file by hand averages ~9 400 tokens on this set; an agent that greps
first reads more. A `brain_ask` answer is ~260–340 tokens and names the file. So the token saving is
real *when retrieval finds the file* — which is now two thirds of the time in the top three — and
the model's contribution is a sentence of answer and a shorter list, not better recall. Idle cost:
252 MB VRAM and ~0 CPU (the model unloads after five minutes); awake, ~3.5 GB VRAM for ~1.2 s per
question. `run_eval.py` is not in the repository (the questions name private projects); the method
is: `cortex --json ask` per question, compare the first item's path.

**Evidence.** `cargo test --workspace` 1121 passed, 0 failed. `ui`: tsc clean, vitest 77 passed,
Playwright 192 + 5 new (`tests/brain.spec.ts`: numbers as sent, log grows, curator from the window,
polling stops when the view is left, 400 px without overflow).

**Autostart and access, same day.** `packaging/systemd/hippocampus-backup.service` (light backup
at login, and in `ExecStop` at shutdown — ordered after the daemon so the daemon is still up when
it runs), `hippocampus-backup-full.{service,timer}` (daily full, ten minutes after login),
`hippocampus-curator.service`; `install-user.sh` installs and names all of them. Backups are 0600.
`packaging/hippocampus.desktop` has actions for Dateien and Gehirn; `packaging/agent-instructions.md`
is the short block every harness on the owner's machine now carries (Claude Code, Hermes,
CommandCode, Antigravity), and the MCP server is registered in all four.

**Later the same night: the index cleaned, the encoder moved, the projects filed.** After D122's
requeue the chunks fell from 206 469 to ~132 000. Embedding and the cross-encoder now run on the
CPU on the owner's machine (`[embedding] device = "cpu"`, CPU runtime fetched): **cortexd holds
0 MB of video memory instead of 2.2 GB**, which the 27B model needs. `brain.ask` now reads the
project memories of the zones the question names and of its best documents (without that, every
project card was invisible to the question it answers). Final measurement on the same twelve
questions: **ranked hit@1 6/12, hit@3 10/12 at ~0.54 s; with the model hit@1 9/12 at ~1.5 s.**
44 projects were filed by the Extension's `hc-ingest-projects`: `~/Projects/INDEX.md`, 160
project memories (actor `ingest`) and 36 entity cards on the Main Brain. Federation delivered its
first memory end to end (a journal line on the server within a minute of `cortex sync now`).
`.dart_tool` is a bulky directory now (Flutter build output was being indexed).

**"Es lädt nicht" — the owner, and three bugs behind it.** (1) **Every event listener in the
window had failed since the first release**: no Tauri capability allowed `plugin:event|listen`, so
`window:visible`, `window:shown`, `transfer:progress`, `daemon:unavailable` and the new section
events never reached the page — silently, the error was swallowed by a `.catch`. Found only once
the page could log into the journal (`ui_log`). `ui/src-tauri/capabilities/default.json` grants
listen/unlisten and nothing else. This also means `useVisibility` had never seen the window hidden,
so everything gated on it polled in the background — worth re-measuring. (2) Under niri a window
scrolled out of view reports itself visible, so the launcher's toggle hid what nobody could see and
the section flag switched a window nobody could see; a summon now remaps unless the window has the
focus. (3) niri opens a remapped window unfocused without an activation token — the owner's config
now has a window rule `open-focused true` for `app-id="^hippocampus$"`.

**Correction (D124).** Asleep, llama-server kept 2.7 GB of RAM; the curator is now socket-activated
and stopped after five idle minutes — idle it costs nothing, a cold question takes 2.9 s instead of
1.6 s. Idle base load measured: cortexd 0.05 % of a core and 0 MB VRAM, the hidden window 0.08 %.

**Not done here, deliberately.** M12 steps 2–4 are untouched. The brain view's CPU cost in the real
WebKitGTK window has not been measured yet — the snippet in "How to answer a report of slowness"
is the way, with the view open and agents working.

### 2026-09-17 — M12 step 1: the origin is projected from the episode, and the red test was a test

The session in the order the work demanded: the previous agent's two open problems first, then the
last of the design-kit port, then M12 step 1.

**The red test in `ui` was a test bug, and the wrong half of a decision was documented.**
`cargo test -p hippocampus-ui` failed on the connect test because `bridge.rs` now sends
`identify` on connect (D117) and the fake daemon answered *every* request with a notification — the
test saw two messages and its assertion "the notification was the answer" had nothing left to point
at. Fixed in the fake daemon (it records what it heard, so the property is still checked) rather
than by weakening the bridge. `cortex-rpc/src/protocol.rs` also still said an unclaimed connection
is `human` — the comment and the default outlived D117 and would have taught the next reader exactly
the thing that decision forbids. It now says `unknown`. `cargo test -p hippocampus-ui`: 36 green.

**The design-kit port is finished.** Every hard-coded colour left `app.css`/`shell.css`
(`tokens.css` gained the last of them), QuickLook's zoom control — the last button in the
application that owned its own fill — became a `GlassButton`, and ConflictDialog's two sides sit on
the kit's `well`. The port is now *enforced*: `ui/src/test/architecture.test.ts` fails on any hex or
`rgba()` outside `tokens.css` ("a hex colour outside tokens.css means the design document stopped
being the contract"). Falsified with a probe stylesheet before being believed.

**The Playwright flake is found and fixed — in the test.** Reproduced with
`npx playwright test --repeat-each=3`: 561 executions, 1 failure, in
`sidebar.spec.ts › a note can be written without leaving the window or touching the file`. The note
went to `/a-rather-long-directory-name-for-testing` — the *directory* the list had shown before the
query — while the assertion wants the row the query produced. Playwright's failure snapshot showed
the window in the right state (module-0 selected), so the note was written *before* the window
caught up. The cause is in `App.tsx`, not in the daemon: this sidebar is painted from
`useDeferred(focus)`, so the list highlight moves a frame or two before the panel does, and the note
is written from the panel (`ipc.annotate(hit.path, …)` in `ContextSidebar`). Displayed subject and
acted subject are the same object — that is coherent — but the test waited on `.hc-row-selected`
(the list) and the app could not answer that question yet. The wait is now
`expect(page.locator(".hc-context-title")).toHaveText(/module-/)`: the panel, not the list. The
property is unchanged and is the point of the test.

The same `--repeat-each=3` run found *two more* of the same class, in tests nothing had touched:
`layout.spec.ts › the button and Ctrl+Shift+A are the same control` waited 30 s for a panel that
never opened, because the keystroke went into the gap between `page.goto` and the app's focus effect
— the shortcut is a React handler on the window's element, so a key pressed while `<body>` has the
focus bubbles past it and is gone, and nothing presses twice. It now waits for
`expect(page.getByLabel("Suche")).toBeFocused()` first. And `disambiguation.spec.ts › Quick Look
shows the file as large as the window` read `after[after.length - 1]` and caught the *pane's* 512
request: the pane behind the overlay is still mounted and may re-request, so "the last request" is a
race. The property is "exactly one request is larger than the pane's, and it is this file", which is
now what it asserts. `--repeat-each=3` afterwards: **561 passed, 0 failed** (56 s), and `tsc --noEmit`
clean before it.

One residual, deliberately not claimed as fixed: in a *pre-fix* tree the same sidebar test also
failed twice in six suite passes as a 30 s test timeout *inside* `page.evaluate` (`calls(page)`) —
the renderer's main thread not answering, not the wrong subject. It has not reappeared in 561
post-fix executions, and the plausible cause is load rather than a wait: `playwright.config.ts` runs
`fullyParallel` with one worker per core, each building a 10 000-row fixture in-page, and this box
had load 17 on 16 cores with 1 GB free RAM and a 99 %-full root disk while it ran. If it comes back,
measure before believing a wait is at fault: `npx playwright test --workers=4` on the same tree
separates "the test asks the wrong question" from "the box cannot hold sixteen of these pages at
once". Reducing the default worker count is a change to how everybody runs the suite, so it is left
as a decision rather than made quietly here.

**M12 step 1 (criteria 1 and 15).** Migration v10 `0010_origin_authority.sql` — five columns and one
index, additive, `rebuilds_tables: false`; `nodes.origin`, `memories.authority`, `basis_json`, and
the two validity columns. The derivation lives in *one* place, `Store::remember`: origin is a
function of the connection's actor, authority is that origin's ceiling, and no request type in
`cortex-rpc` has either field, so criterion 1 is a property of the protocol's shape. `replay_memories`
re-derives both from the episode's actor instead of copying the projection (R10), and `canonical_dump`
now carries them, so a rebuild that derived differently fails the dump comparison. `Memory` and every
pack `Basis` carry both fields and a pack item serialises them unconditionally; the CLI's `memories`
and `recall` print them as `as instruction (human)`. The backfill rule and its reasons are D118.

Evidence: 3 new store tests (the backfill keeps the least: an old chunk is `file`, an old community
node takes its episode's actor, an old agent memory row is `observation` and *not* a fact; the
validity columns stay empty rather than being filled with `ts`), a rebuild test that writes as
`agent:a` and earns `agent`/`fact` through a replay, and the per-surface test
`every_surface_writes_the_origin_its_connection_implies` in `cortexd/tests/three_surfaces.rs`, which
writes the same sentence over MCP, over the CLI as the human, over the socket the window uses, over
an unidentified socket and over a CLI with no `--as`, then reads all five back. Criterion 15: the
per-surface test was falsified by making `Origin::from_actor` return `Human`, and it failed on the
first assertion — with every memory in the pack coming back as `authority: instruction`.

What step 1 does *not* do, so nobody reads the green as more than it is: the pack **renderer** still
prints no labels (the fields are on the items; the text an agent reads does not yet say them — the
open design question is where a label goes in a token budget, and the honest rule is probably
"whenever the item is not the human's"); `basis_json` is written by nobody yet; criteria 2 to 4
(inheritance, the imperative detector and `pending`/`untrusted`, elevation as an episode) are
untouched; the Inspector does not show the two fields yet; and the validity columns are deliberately
empty until step 2 gives `remember` a field for them.

For the next agent: `cargo build --workspace --bins` before judging a `three_surfaces` failure —
that test shells out to `target/debug/cortex`, and a stale binary serialises the pack without the
newest fields and looks exactly like a surface disagreeing (the assertion message now says so).
`migrate_to(&mut conn, 9)` in `cortex-store/src/migrations.rs`'s test module is how a pre-migration
store is built. And `canonical_dump` is the evidence for R10: if a column matters to a rebuild, it
belongs in that dump or the rebuild tests are not testing it.

### 2026-09-15 (second session) — the installation was nine days behind the tree, and the pass had no trigger

The owner's order for this session: *"Fang jetzt an das ganze GUI komplett funktionsfähig und
effizient ist. Und mache die Agent Pipeline noch fertig. Heißt: Agents anbinden, Hippocampus Brain
vollständig einbauen und laufen lassen, Page zur visualisierung über die GUI, direkte Verbindung mit
dem Main Brain auf dem server, nachtragen aller daten in das Brain."* The first hour went into
finding out what "not functional" actually meant, and it was not the window.

**What was installed was from 2026-09-06.** All five binaries and both units still had that date:
`cortexd`, `cortex`, `cortex-mcp`, `cortex-preview` and `hippocampus` itself. Everything the last
two sessions built for the window — the start screen's `stats.overview`, the Agent Inspector and its
five tabs, the whole memory column — was written *after* that binary was installed, so the window on
this desktop was asking a nine-day-old daemon for methods it does not have. Nothing in the window's
code was broken; the installation was. Rebuilt and installed with `scripts/install-user.sh` (which
builds the window through the Tauri CLI on purpose — a plain `cargo build` bakes the dev server's
URL into it), with the store backed up first (1.1 GB, `~/.local/state/hippocampus/backups/`), the
daemon stopped and started, and the migrations ran through to schema 9 without a complaint.

**What the daemon says about itself now**, `cortex overview`, 2026-09-15: 2 657 files, 564
directories, 53 247 chunks, **184 754 vectors, 100 % covered**, 19 186 episodes (1 093 of them in
the last 24 hours), 1 concept, 80 associations, 14 zones, 15 annotations, 193 duplicate files, and
last pass — 23 days ago. The memory is empty: **0 memories, 0 human-confirmed, 0 superseded**; the
agent surface is nearly untouched: 0 sessions, 2 calls, both `fs.places`. That is the honest state of
"the brain runs": the index is alive and watching, the memory layer is built and has never been fed.

**The pass had exactly one trigger, and it was a person.** Nothing in `cortexd` schedules
consolidation and no timer existed, so it ran when somebody typed `cortex consolidate` — hence 23
days. An index that is never consolidated has no tier movement and nothing decays, which makes
salience, pinning and tiering look like decoration. Fixed as infrastructure rather than core work:
**D116**, `packaging/systemd/hippocampus-consolidate.service` and `.timer` (03:00, `Persistent=true`
— the machine is not awake at 03:00 every night, and a job that silently did not happen is
indistinguishable from a quiet index). The timer calls `cortex consolidate`, which is a *client*
call into the daemon (`brain.consolidate` over the socket), so the single-writer rule holds and
nothing here needs the store. Verified: the timer is enabled and scheduled for 2026-09-16 03:00,
`systemd-analyze --user verify` is clean, and starting the service by hand returned
`Result=success` with a fresh pass in `cortex report` (138 ms). `install-user.sh` and
`uninstall-user.sh` know about four units now, and the install note says out loud that the timer is
what makes the brain run by itself.

**M6's mandatory list, checked live against the running daemon** (`/tmp/hc-audit/`, not the home
directory): trash ✓ and `cortex undo` ✓ — and the file landed in `/tmp/.Trash-1000/files/`, which is
the spec's per-filesystem top-level trash rather than `~/.local/share/Trash`, so that item is worth
checking on a second filesystem for exactly that reason; permissions, ownership and symbolic links ✓
(`cortex ls` prints `lrwxrwxrwx legi legi 5 link.txt -> a.txt`); `xdg-mime` open-with ✓ (`cortex
apps` on a `.txt`: DMS Notepad as default, Vim listed); `places` ✓; `stat` ✓. Not yet checked live,
and therefore not claimed: mounts (no device was plugged in), thumbnails, archives, drag and drop,
SMB/SFTP.

**The gate, run after the swap:** the Rust workspace suite is green — **1 081 tests, exit 0** —
and the window's gates are where the last session left them (76 vitest, 187 browser tests, the perf
suite 16/16).

**The efficiency baseline, taken while nothing was happening** (hidden overlay, no search open):
`hippocampus` 69 MB RSS, 30 threads, **0.10 % of one core**; `cortexd` 89 MB RSS, 48 threads,
**0.40 %**; the store 1.1 GB plus 47 MB of vectors. So the resident window is not the problem it is
often assumed to be, and "efficient" in this session's order is about the *store* first: 193 files
have a byte-identical twin in the index, 2 366 touched nodes have nothing linking to them, and
184 754 vectors cover 53 247 chunks. Those are the numbers to move.

**The first slice of M12 step 1 is in the tree.** `Origin` (human · agent · tool · file · peer)
and `Authority` (untrusted · observation · fact · instruction) live in
`crates/cortex-core/src/memory.rs`, each with the three rules that are decisions rather than
conveniences: `Origin::ceiling()` (only a human origin may create an instruction),
`Authority::may_motivate_a_write()` (only an instruction may be the stated reason for a gated write)
and `Origin::is_trusted()` (a file and a peer are not two witnesses). The variant order of
`Authority` *is* the inheritance rule, so `min()` is the whole arithmetic of criterion 2. Six new
unit tests; `cargo test --release -p cortex-core --lib memory` → **16 passed, 0 failed**. Next in
this step: `0010_origin_authority.sql`, then `origin = f(connection actor)` (criterion 1), then
`motivated_by` in the gated path (criterion 3).

**Criterion 1 has its teeth, and they were not where the plan expected them.** The socket could
not name its own origin — but it did not have to: `Actor::Unknown` did not exist, so an un-identified
connection *was* the human by default (`actor_from_client(None)` and the daemon's initial `ctx.actor`
both said `Human`, justified by a comment that read "the one that grants nothing"). Measured: every
read and every pass this session ran was written into the episode log as `human`, including the ones
an agent ran — the log could not tell us apart. **D117** closes it: un-identified is the weakest
origin (`peer`, authority capped at `observation`), the window says `human` explicitly when it
connects, the CLI does it with `--as human`, and the daemon's search episode stops claiming to be a
person. One consequence for the person at the terminal, written down because it will be noticed:
`cortex remember "…"` without `--as human` stores an observation, not an instruction. That is the
point of the label, not a side effect. `cargo test --release -p cortex-core -p cortex-agent --lib`
→ **146 + 114 green**.

**What this session did not touch:** M12 itself. Its first step — origin and authority (D98,
criteria 1–4) — is still the next thing to build, and the plan's own order (`Where M12 stands`) says
it goes first. The two other blocks behind the owner's order are M7 to a real peer (the Core's half
is built and green; what is missing is the Main Brain on the server, which does not exist there yet
— `jlhome6353` has no cortex binary, no unit and no store), and the backfill: 28 projects' worth of
documents and insights into a memory that currently holds zero.

### 2026-09-15 — the file explorer in the kit's clothes, and two bugs the materials had been hiding

The kit port of 2026-09-14 named three places still in the hand-written skin: the file rows, the
panels' inner lists, the places column. This session ported them — and then found out why the result
still looked flat.

**The rows.** A row carries the kit's separator now: 1 px, at the row's bottom edge, inset past the
icon column and 12 px short of the right edge, and it is *gone* on a selected row so no seam runs
through the fill D15 paints. The rule lives on the row's own `::after`, which is why `.hc-row` grew
a `position: relative`. Measured on a crop: the rule reads 57.1 against a row fill of 41.1. Held by
`tests/layout.spec.ts`, which asks the engine rather than the eye —
`getComputedStyle(row, "::after")` is 1 px, `left` past the border, `right` 12 px, `opacity` 0 on
the selected row.

**The column header is the kit's strip.** It had been a heading row with a rule under it and, as the
probe showed, a *two-row* grid: `.hc-list-header-meta` asked for `grid-area: meta` in a strip that
declared no such area, so the grid invented two 0 px columns and a second row, and the three
headings stood half a line below „Name". Existing coverage could not see it because it compares the
*right* edges, which were correct. The strip declares `grid-template-areas: "name meta"` now,
carries the kit's material (`glass-flat glass-weak`), and is 30 px tall where it was 52. Headings and
their values are set in `--hc-text-dim` (0.55) — measured 158.1, the same step as the metadata they
head.

**The empty state** is the kit's: a 72 px glass circle with the glyph of the search it belongs to
and the sentence under it — and *no* circle where there is nothing to search with (a dead share, a
refused read), because a circle there would decorate an error. **The breadcrumb** hovers like the
kit's (`accent-soft` plus primary text), and so does the rail's current entry.

**Then the two bugs, both found by measuring rather than by looking.**

1. **Every material class had no shadow at all.** `getComputedStyle` said `box-shadow: none` on
   `.glass`, `.glass-flat`, `.raised*`, `.well`, `.solid` and `.hc-pill` — every material this
   window has. The ported recipes spell out the kit's `inset 0 1px 0 0` in front of tokens that
   already *are* that phrase (`--hc-glass-rim: inset 0 1px 0 rgba(…)`), which makes the declaration
   invalid, and the browser drops the whole list without a word. The fill is a separate property and
   still rendered, so the surfaces looked flat rather than broken: the header strip's fill was
   measurably there (45.1 against the panel's 41.1) and its rim was not there at all — 41.1 → 53.1
   once the recipe parsed. Now `tests/materials.spec.ts`: on two screens, every element carrying a
   material class must come back with a shadow, and the walk must have found something to check.
   D112.
2. **The numbers were this window's, at about half the kit's strength.** Rim 0.07 against the kit's
   dark 0.16, fill 0.035 against 0.06, sheen 0.05 against 0.24, and the raised pair a translucent
   white wash against the kit's two *solid* greys (`#3b3b44` over `#2a2a31`). With §8.1's ban on
   backdrop blur those three ingredients *are* the material, and at 44 % and 21 % a surface measures
   as slightly lighter than its background and reads as a rectangle. The values are the kit's now;
   `--hc-glass-seam` rings a surface with the kit's dark seam; the rail's gradient starts at
   `--hc-glass-fill` because the kit's `strong` is a nearly opaque panel fill for menus, not „a bit
   more fill". Measured after: the rail's current entry spans 52 levels with an 82.1 rim row, where
   it spanned 19 before. `--hc-glass-drop` and `--hc-glass-edge-strong` were read by no rule and
   went; the window shadows gained the kit's 1 px white ring. D113.

**Then the backdrop — built, measured, and taken out again by the owner.** The last visible
difference to the kit was its lit canvas (D113), so the kit's own was built: three very large glows in
its unchanged dark canvas colours on one `inset: 0` layer, as radial gradients with a solid core
because §8.1 keeps the kit's `blur(90px)` out. **Measured:** the file list's median 44.5 → 49.5, the
rail 48.8 → 59.6, and the spread of tile medians across the window 12.0 → 25.2 — a canvas that is lit
and uneven, which is what the panels had been missing. **The drift did not survive being measured:**
counted over CDP (`LayerTree.enable`) and timed as frame deltas in one run, drift on against off, 15
composited layers against 11 and 21.0 ms against 16.7 ms per frame — at rest *and* under a scroll, a
permanent 60 → 48 fps in a window that never closes, for light to move. So it was kept still (7 layers
with it, 7 without) and shown to the owner. The verdict took the colour out too: *„Die glows nehmen
irgendwie die Minimalistic raus aus dem GUI Konzept."* The numbers had been fine; the concept had
not, and that is an argument no measurement answers. The glows are gone, `Ambient.tsx` paints one
gradient again, and the only light this window has is the breath, which is lit because there is work
to report. Measured after the removal: median 42.5, blue-over-red bias +2.4, spread 12.9. D114 (the
drift) and D115 (the colour) hold the two halves.

**Green, and two things that were not.** tsc clean, **76 vitest**, build clean, and the browser suite
is **187 tests, 187/187 green serially** (55 s). It was 185 when this session started and should
have been 187 the moment `tests/materials.spec.ts` was written: that file — the guard on the very
material numbers D113 is about — was written last pass and **never ran in the gate**, because `test:ui`
named its spec files one by one and that list was a second truth nobody checked. A guard that does not
run is not a guard, and the numbers it holds are the ones this session had just measured by hand.
`test:ui` is `playwright test` now, with the reason written into `package.json`; a new spec under
`tests/` is in the gate the moment it exists. **The second thing is that the parallel gate is
load-sensitive, and this machine was busy:** with a load average of 20 on 16 cores one run came back
`1 failed, 186 passed` in `operations.spec.ts`, while `operations.spec.ts` alone is 49/49 and the same
suite serially is 187/187. That is the shape the skill's *Spec-Zuverlässigkeit* section describes, and
nothing to do with what changed here — **a red parallel run under load is a reason to re-run it
serially, not to go looking for a regression.** The perf numbers still have to come from an idle
machine: `npm run test:perf` is 16/16, the keystroke budgets and the ten-thousand-row scroll where they
were, and "with nothing being indexed the heartbeat does not animate at all" still holds. The six scratch specs used for the pass
(`tests/tmp-*`) are deleted; the measurement scripts live in `/tmp/hippocampus-shots/`.

### 2026-09-14 — the interface is rebuilt from the showcase's design kit

The showcase (`ui/inspo/minimalist-glassmorphism-file-explorer`) was no longer imitated; its
design kit was ported and the interface moved onto it. `src/glass.css` carries the kit's material
vocabulary once (`glass`, `glass-flat`, `glass-strong`, `glass-sheen`, `glass-specular`, `raised`,
`raised-bezel`, `well`, `solid`), `src/kit.css` the class bodies of the kit's controls, and
`src/components/glass/` the primitives: `GlassButton`, `GlassSurface`, `GlassInput`/`GlassSearch`,
`GlassSegmented`, `GlassToggle`, `GlassCheckbox`, `GlassMenu`, the four lucide icons redrawn by
hand, `Kbd`, `CountBadge`, `StatusDot`. `src/cn.ts` and `src/useSpecular.ts` are the kit's
helpers without new dependencies; `motion.ts` maps the kit's spring names onto the six springs.
The search field, the sort word, the segments (time windows, inspector tabs), the panel cards and
the header buttons are kit parts now; `shell.css`/`app.css` gave up their six private copies of
the materials (D110, D111).

**Gate:** tsc clean, **73/73 vitest** (13 new: `src/components/glass/glass.test.tsx`), build
green, **184/184 browser tests** — after four findings the kit port surfaced: two tests still
spoke the old field class names (`hc-field-hint/clear` → the kit's `gi-shortcut/gi-clear`), the
sort word's title moved from a span to its button, `GlassInput`'s shell was a `<label>` and so
named an already-labelled field twice (now a `<div>` with click-to-focus written in, caught by
the suite's strict locator), and the controls test's luminance read ignored alpha — it
pre-multiplies now, or every translucent glass fill would read as "light on dark" forever.

**Not yet, in the order agreed with the owner:** the file rows (`.hc-row`) still carry their own
hand-written hover/selection skin; after them the panels' inner lists and the places column. Each
step ends with the gate and a fresh set of pictures — the owner steers by pictures, not prose.

**Port pitfalls this session added to the skill** (so the next agent does not rediscover them):
a kit component's shell element can carry an implicit accessible name (`GlassInput` shipped as a
`<label>` and named an already-labelled field twice — the shell is a `<div>` now, with
click-to-focus written in); a test helper sent to the page as a template string (`page.evaluate`)
must not contain backticks, not even in comments — it dies in the scanner, not the browser; and a
luminance check that divides rgba by 255 without pre-multiplying by alpha reads every translucent
glass fill as "light on dark" — pre-multiply, then compare.

**Session-log rule for tomorrow:** one gate per finished step, numbers into this file, pictures
into `/tmp/hippocampus-shots/v5/` (or higher), and the D-log only for decisions, not for steps.

### 2026-09-05 (evening) — the brain, read against the 2025–2026 field; M12 specified

A scientific deep-research pass on agent memory (salience, forgetting, consolidation, retrieval
feedback, provenance, evaluation): three search rounds, 230 papers fetched as text into a
research library kept outside this repository, forty-odd read in full. The result is
`docs/paper/WHAT-2026-CHANGED.md` — what the field confirms (no model in the write path; the
index as a cache; the closed feedback loop; explainability), what it corrects (one half-life for
six memory kinds; "newer wins"; a comparison table that was an argument), and what it adds
(memory poisoning and authority collapse as the open problem; controlled evaluation as the
missing experiment with a protocol now).

**What changed in the repository:** the new paper; §2.4, §12, §13.7, §14 and the references
pointer in the design paper; M12 in both build prompts with fifteen criteria and a bracket rule
(every mechanism has a test that fails when it is disabled); D98–D102; the glossary's M12 terms;
the architecture's agent-brain section; the agent-tools note; this file. **No code changed.** M12
is specified, not built, and the order of work above is deliberate: provenance first because the
evidence is strongest and the change is smallest.

**What was learned that is not in the paper:** the reading agents that were meant to digest the
corpus in parallel all died on a session rate limit; the digest was made by hand from abstracts
plus targeted full-text reads, which is why forty papers are read closely and not two hundred.
The library's `INDEX.md` says which; a later agent with budget should read the benchmark folder
(thirty papers) before building `bench-quality`, because the protocols there are the ones to copy.

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