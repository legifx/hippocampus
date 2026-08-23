# A Directory Is Just One Query

### Restructuring a file manager around the memory underneath it

**Status.** A design paper for work not yet done, written against a running implementation. The
current behaviour described in §2 is measured or read from the code; everything from §4 onward is
a proposal with acceptance criteria. The companion papers are
[`FORGETTING-IS-THE-FEATURE.md`](FORGETTING-IS-THE-FEATURE.md), which argues the memory design,
and [`BUILD-PROMPT.md`](BUILD-PROMPT.md), which specifies it. This one is about the window.

---

## 1. The claim

The explorer today is a **file manager that can ask a memory questions**. It should be a
**memory browser that can show a directory**, and the difference is not framing — it changes what
the row list is, what the sidebar is for, what the address bar addresses, and which component is
allowed to decide anything.

The whole argument fits in one observation about code that already exists. The row renderer in
this project does not know whether its rows came from a directory listing or from a search:

> An empty query shows a directory; typing replaces the list with results and clearing brings the
> directory back. That is one list rather than two pages, which is why the row renderer does not
> know which mode produced its rows.

That was written as an interface convenience. It is actually a statement about architecture, and
following it to its conclusion is the entire content of this paper:

> **A directory is one query among several. It is not the ground truth of the view; it is a
> filter that happens to be spatial.**

Once that is true, everything the memory knows becomes navigable by the same machinery that
already shows folders — and the file manager stops being a separate application that consults a
memory, which is the shape that makes every "smart file manager" feel bolted together.

---

## 2. Where the explorer actually is

Honest inventory, because a restructure proposed against an imagined codebase is worthless.

**What works.** An overlay summoned by the compositor, resident, cold start 265 ms. Browsing with
Enter/Backspace/Alt+Left and a clickable breadcrumb. Selection with an anchor that stays put.
Copy, move, rename, trash, undo, a conflict dialog, transfer progress with a real denominator and
cancellation. Places, mounts, open-with from the desktop's own databases. A preview pane with a
separate thumbnailer process. A command palette. Search as an overlay over the same list.
Keyboard-complete. 400 px to full width without overflow.

**What the memory contributes today.** One panel. `ContextSidebar` calls `brain.neighbors` for the
selected row and renders what comes back — related files, annotations, why it is salient. Its own
header comment is unusually candid about its role:

> *This is the panel that makes the application worth having over `fzf`.*

**And that is the problem this paper is about.** The memory is a *panel*. It is consulted about
the row you have already found, in a directory you already knew to open. Everything the daemon
computes — salience, four tiers, co-access edges, concepts, the working set, an episode log going
back to the first index run — reaches the user through one sidebar on one selected file, or not
at all.

**Three concrete symptoms:**

1. **The ordering is spatial only.** `fs.list` sorts by name, size or mtime, directories first.
   The daemon knows which three of these forty files you actually worked with this week. The list
   does not use it.
2. **The tiers are invisible.** Four tiers exist and are computed every night. In the window they
   appear as a "cold-tier indicator" on search results, and nowhere else. A user cannot see what
   the system has quietly stopped keeping close, which is exactly the thing they would want to
   check before trusting it.
3. **Concepts have nowhere to live.** Consolidation names clusters of files that keep being used
   together. That is arguably the most valuable thing in the index, and there is no view in which
   a concept is a thing you can open.

None of these is a bug. They are the consequence of an architecture in which the directory is the
subject and the memory is an annotation on it.

---

## 3. Why the obvious fix is the wrong one

The tempting move is to add features: a salience column, a tier filter, a concepts panel, a
"recent" sidebar. Four more surfaces, each with its own state, its own fetch, its own empty state,
its own keyboard handling.

That is how file managers accumulate panes until nothing is discoverable. It also violates the
rule this project already has (**D4: no data processing in the frontend**) in a subtle way: the
moment the window holds four different row sources, the window is the thing that decides which of
them you are looking at, how they combine, and what happens when two disagree. That is a query
planner, in TypeScript, in a component tree.

The restructure below adds **no new panes**. It generalises the one that exists.

---

## 4. The restructure: one row stream, many origins

### 4.1 The idea

Today the window has two row-producing calls with different shapes — `fs.list` and `search` — plus
three more that produce something row-like and are rendered by hand: `brain.working_set`,
`brain.neighbors`, `brain.timeline`.

Replace all five, **at the view layer**, with one:

```
view.open(source, order?, limit?, cursor?) -> View
```

where a **source** is any of:

| Source | Reads | "Where am I?" |
|---|---|---|
| `dir(path)` | a directory | `~/Projects/atlas` |
| `query(text)` | hybrid search | `parser bug` |
| `concept(id)` | a named cluster | `parser · grammar` |
| `zone(id)` | a project | `atlas` |
| `working_set` | recent episodes | *what I am doing* |
| `neighbours(node, types)` | the graph | *used with `parser.rs`* |
| `timeline(from, to)` | episodes in a window | *Tuesday afternoon* |
| `tier(cold)` | the tier index | *what has been forgotten* |

and a **View** is:

```
View {
  rows:   [Row]          // one uniform shape, whatever the source
  origin: Source         // what produced this, echoed back
  order:  Order          // what the daemon sorted by
  basis:  String         // in words: what this list is and is not
  total:  usize          // how many exist; rows may be a page of them
  cursor: Option<String>
}
```

Three properties make this a restructure rather than a rename.

**The row shape does not depend on the source.** A row from a directory and a row from a concept
carry the same fields, including the memory ones — salience, tier, degree, last episode, zone,
pinned, and which concepts it belongs to. The renderer already does not care where rows came from;
now it *cannot* care, because there is nothing to distinguish.

**Ordering is the daemon's, for every source.** `fs.list` already refuses to let the frontend
sort, and the reason given — two listings of one directory must never disagree — applies with more
force once ranking is involved. `Order` gains `salience` beside `name`, `size`, `modified`, and
salience is the default for every non-spatial source.

**Every view states its own basis in words.** This already exists in the graph snapshot and it is
the cheapest honesty mechanism in the system: a view of a fresh index says that nothing has been
consolidated yet, a cold-tier view says that these are metadata-only and why, a concept view says
how the cluster was named. A list that cannot say what it is invites the reader to invent it.

### 4.2 The address bar addresses views, not paths

The breadcrumb today shows path segments. It becomes a **location bar** over `Source`:

```
~/Projects/atlas/src              a directory
parser bug                        a query
▸ parser · grammar                a concept
◷ Tuesday, 14:00–18:00            a slice of time
● what I am working on            the working set
❄ forgotten                       the cold tier
```

Each is a place. Each can be bookmarked, navigated back from, and appear in Places. **Alt+Left
already walks a history stack**; it now walks a history of views, and the only change required is
that the stack holds a `Source` instead of a `PathBuf`.

This is the smallest possible interface change for the largest gain: the user learns nothing new.
They already know how to be in a location and go back.

### 4.3 Concepts become places

A concept is a node with `derived_from` edges to its members, named after what those members share
and the rest of the index does not. Opening it is `view.open(concept(id))` and the rows are its
members. It belongs in Places under its zone, and it is the answer to the question a directory
tree structurally cannot answer: *what was this work?*

**A concept view must show its own provenance.** The basis line says which terms named it, how
many bursts of activity produced it, and — the honest part — that the naming is lexical rather
than a summary. A place that looks like a folder but was inferred has to say so, or the user will
assume somebody filed it there.

### 4.4 The tiers become visible, in one place

Not a badge on rows, not a filter dropdown. **One view**: `tier(cold)`, reachable from Places,
whose basis line explains what "cold" means and that nothing has been deleted. Rows in it are
ordinary rows and can be opened, pinned, and pulled back.

Everywhere else, tier is *brightness* — the same encoding the 3D view already uses, so the two
windows teach the same visual language. A cold row is dimmer. That is all. No badge, no icon, no
column.

### 4.5 The sidebar stops being the memory

Once the list itself carries the memory, `ContextSidebar` sheds most of its job and keeps the part
that genuinely belongs to a single selection:

- **why this ranked here** — the salience breakdown from `brain.explain`, which is the trust
  mechanism and deserves to be one keystroke away rather than buried;
- **what happened to this** — its episodes;
- **annotations**, and — new — the ability to *write* one without leaving the window.

"Related files" moves out of the sidebar and becomes `neighbours(node)`, a place you can go, with
the full row list and every operation available. A related file you cannot copy is a teaser.

---

## 5. What this costs, and the one number that decides it

Memory columns are not free. Today `fs.list` returns a directory; the proposal has it return a
directory **plus** a salience, a tier, a degree and a last-episode timestamp per row.

The budget is unchanged and non-negotiable: **a directory of 10 000 entries renders its first row
in under 100 ms and scrolls at 60 fps.** Three consequences:

1. **The memory columns come from the same query**, as a join, not as a second round trip per row.
   A per-row `brain.explain` would be ten thousand calls and is the obvious way to fail this.
2. **Degree and concept membership are precomputed**, not counted at listing time. The
   consolidation pass already touches every node; it can leave the counts behind.
3. **If the join cannot hold the budget, the columns are dropped, not the budget.** A file manager
   that hesitates is a file manager nobody uses, whatever it knows.

Measure before building: run `fs.list` against a 10 000-entry directory today, then the same
listing with the join, on a cold page cache. If the difference is over 15 ms, the design is wrong
and the columns belong in a deferred second message that arrives after the rows are on screen —
the same pattern the reranker already uses, where better results arrive 17 ms after the list is
already visible.

---

## 6. Doing it in four steps

Each step is shippable on its own and leaves the application working. None of them is a rewrite.

### Step 1 — One view type, existing sources

Introduce `View` and `Source`, and reimplement `fs.list` and `search` as `view.open(dir(...))` and
`view.open(query(...))`. No new capability. The frontend's history stack holds `Source`.

**Done when:** every existing behaviour is unchanged, the row shape is identical across both
sources, and the frontend contains no branch on "am I browsing or searching".

### Step 2 — The memory columns

Salience, tier, degree, last episode, zone, pinned, concepts, on every row from every source.
Tier renders as brightness. `Order::Salience` exists and is the default for query views.

**Done when:** the 10 000-entry gate still holds, measured; and a directory of files you have
worked with this week, ordered by salience, puts them at the top — verified by hand, once, and
written down.

### Step 3 — The new places

`concept`, `working_set`, `neighbours`, `tier(cold)`, `timeline`. Each appears in Places, each has
a basis line, each supports the full row operations.

**Done when:** a concept can be opened, its members copied, and its basis line read; the cold view
explains itself; and Alt+Left walks back out of all of them.

### Step 4 — The sidebar's new job

`brain.explain` on one keystroke, episodes for the selection, annotate without leaving the window.
"Related" is removed from the sidebar because it is a place now.

**Done when:** a person can ask "why is this here" about any row and get the component breakdown,
and can attach a note to a file without touching the file.

---

## 7. What must not happen

- **No second index in the window.** The frontend holds no rows it did not receive, merges no two
  sources, and computes no ordering. D4 exists because a frontend that filters is a frontend that
  will eventually disagree with the daemon, and the user will believe the window.
- **No model in the listing path.** Ranking by salience is arithmetic on stored numbers. Nothing
  in a directory listing may wait on an embedding, a rerank or a language model.
- **No new toplevels.** The niri constraints are requirements: internal panes yes, additional
  windows never, 400 px must work.
- **The file-manager basics keep priority.** The largest scope risk in this project is that the
  memory features are more interesting to build than archives, drag-and-drop and network
  locations — and that a file manager which cannot do the basics will not be used, however good
  the brain is. This restructure is worth doing **because it is small**; if it grows past four
  steps, it is competing with work that matters more.
- **Nothing here is a reason to touch the daemon's guarantees.** The index stays a cache, the
  episode log stays append-only, and a view is a read.

---

## 8. What would show this was wrong

- **If salience ordering does not beat modification time** for finding a file the user knows they
  have, in their own directories, the columns are decoration. This is testable with a stopwatch
  and ten files.
- **If nobody ever opens a concept twice**, concepts are not places, and they belong back in a
  panel.
- **If the cold view is opened once out of curiosity and never again**, then visible forgetting is
  a designer's idea rather than a user's need, and the tier encoding can go back to being a badge
  on search results.
- **If the 10 000-entry budget cannot be held with the join**, §5's fallback applies, and if the
  deferred variant feels worse than no columns at all, the honest answer is to keep the columns
  in search views only.

Each of these is cheap to check and none has been checked. That is the state of this document.

---

## 9. Starting the work

[`EXPLORER-KICKOFF-PROMPT.md`](EXPLORER-KICKOFF-PROMPT.md) is a session prompt for an agent
picking this up: what to read first, what is already built, what the four steps are, and the rules
that outrank anything in this paper.
