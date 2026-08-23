# Session prompt: continue the explorer

Copy everything below the line into a fresh agent session, in a checkout of this repository.

---

You are continuing work on **Hippocampus**, a memory daemon with a file manager on top. Your job
this session is the **explorer** — the window, not the daemon.

## Read these first, in this order, before writing anything

1. `CLAUDE.md` — the entry point and the non-negotiable rules.
2. `docs/03-PROGRESS.md` — **the current state.** What is done, what the last session left rough,
   and why. Read it first, update it last.
3. `docs/paper/A-DIRECTORY-IS-JUST-ONE-QUERY.md` — **the plan for this work.** It argues that the
   explorer should be a memory browser that can show a directory rather than a file manager that
   consults a memory, and it breaks the change into four shippable steps.
4. `docs/06-DESIGN-SYSTEM.md` — the visual and motion contract. Implement these tokens; do not
   invent new ones. If you need a new token, add it there first.
5. `docs/01-BUILD-PROMPT.md` §M4 and §M6 — the milestone scope and the Definition of Done you are
   working against. §M4's niri constraints are requirements, not preferences.
6. `docs/02-DECISIONS.md` — skim the entries about the window: D4 (no data processing in the
   frontend), D14 (the preview process), D15 (zone colours), D41–D43, D53, D56, D57 (the scroll
   lead, and the measurement that corrected it), D59 (why a running animation costs a core).

Then look at the code you will actually touch: `ui/src/ipc.ts` (the one door data enters through),
`ui/src/row.ts`, `ui/src/components/ResultList.tsx`, `ui/src/components/ContextSidebar.tsx`, and
`ui/src/components/Breadcrumb.tsx`.

## What you are building

The four steps are in §6 of the paper. **Do step 1 completely before starting step 2.** Each step
ships on its own and leaves the application working.

1. One `View` type and one `Source` type; `fs.list` and `search` become two sources of the same
   thing. No new capability, no behaviour change.
2. The memory columns on every row — salience, tier, degree, last episode, zone, pinned, concepts.
   Tier renders as brightness, not as a badge.
3. The new places: a concept, the working set, the neighbours of a node, the cold tier, a slice of
   time. Each with a basis line, each supporting the full row operations.
4. The sidebar's new job: `brain.explain` one keystroke away, episodes for the selection, and
   annotating without leaving the window.

## Rules that outrank the paper

- **The frontend renders and does not decide.** No filtering, no sorting, no searching, no merging
  of two sources. If you need a different order, the daemon provides it. This is D4 and it is
  enforced by an architectural test.
- **Performance gates are the specification**, not aspirations: 300 ms cold start, 80 ms P95 from
  keystroke to first row, first row of a 10 000-entry directory in 100 ms, 60 fps while scrolling,
  bounded DOM node count. §5 of the paper says what to do if the memory columns threaten the last
  two — drop the columns, never the budget.
- **Measure on an idle machine.** This project has a recorded case of a latency number taken while
  the machine was compiling, and three sessions spent optimising something that was never slow.
- **400 px to full width, keyboard-complete, no additional toplevels.** niri owns the layout.
- **Never read a secret.** The deny-list is checked before anything is opened, and that is the
  daemon's job — but do not add a code path that works around it.
- **This repository is public.** No usernames, no absolute home paths, no hostnames — in code,
  tests, fixtures, documentation *or commit messages*. A pre-commit hook enforces it; do not use
  `--no-verify`.

## What must not be sacrificed to make this interesting

The largest scope risk in the whole project is written into §M6: **a file manager that cannot do
the basics will not be used, however good the brain is, and the basics are more work than the
brain.** Archives, drag-and-drop out of the window, network locations and correct permission
display are still missing. The restructure in the paper is worth doing because it is small. If it
grows past four steps, stop and say so in `docs/03-PROGRESS.md` rather than continuing.

## Before you finish

- `cargo test --workspace` and `npm test` in `ui/` are green, and you ran them.
- `docs/03-PROGRESS.md` is updated in the same commit as the code: what you did, what you
  measured (with numbers and a date), **what you tried that did not work**, and what the next
  session should pick up.
- Anything you decided that the documents do not already cover is a new entry in
  `docs/02-DECISIONS.md`, with a reason and a revisit trigger. A decision without a revisit
  trigger is a dogma.
- If you found a defect in something you did not write, say so plainly and write the regression
  test before the fix.

## The question to judge your own work by

> **Can somebody find a file they know they have, faster than before, without learning anything
> new?**

Every part of this plan is meant to be reachable with what a person already knows: they are in a
place, they can go back, they can open things. If a step requires teaching the user a new concept
before it pays off, that step is wrong, and saying so is more useful than shipping it.
