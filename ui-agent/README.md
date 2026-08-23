# Agent View

The brain, drawn. Milestone **M8**: a three-dimensional view of what the memory currently
believes — which nodes matter, what is connected to what, what has been forgotten, and what the
last consolidation pass actually did.

It is read-only. It never touches a file and never writes to the index.

## What you are looking at

| channel | variable |
|---|---|
| hue | zone — the daemon's own colour for the project a node lives in |
| brightness | tier: Working, Short-term, Long-term, Cold |
| size | salience (square root, because area is what the eye compares) |
| halo | in the Working Set |
| edge colour | edge type: containment, co-access, similarity, annotation |

Two layouts, and the transition between them is the point:

- **Tree** — containment as it is. Directories branch, files are the leaves. This is what the
  index *is*.
- **Shells** — concentric shells by tier, Working innermost, Cold outermost. This is what the
  index *believes*, and it is the only layout in which forgetting is visible: a demoted node
  moves outward.

Both are deterministic functions of the snapshot. The same data lays out identically every time
on every machine, which is what makes two screenshots comparable (D66).

**Play consolidation** replays the last pass in five steps — replay, discard, associate, score,
tier — with the daemon's own numbers in the caption. Nothing in the animation is computed here.

Keys: `l` switches layout, `c` plays the consolidation, `Escape` stops and deselects.

## Running it

The view needs a snapshot: `graph.json` next to `index.html`. There are two ways to get one.

**Synthetic, from a seed** — this is what the repository ships with, because it can be published:

```sh
cargo run --release -p brain-sim -- --out ui-agent/public/graph.json --days 60
npm install && npm run dev
```

`brain-sim` invents a workspace and sixty days of plausible use over it, then runs the **real**
`Store::consolidate` once per simulated day. The history is fiction; every salience, tier and
edge in it was computed by the same engine the daemon runs.

**From your own daemon** — this is the one that shows your actual index:

```sh
cortex consolidate                       # if it has never run, everything scores 0
cortex graph --out ui-agent/public/graph.json --limit 6000
npm run dev
```

## The single-file demo

```sh
node scripts/bake.mjs public/graph.json ~/agent-view.html
```

One HTML file, no server, no daemon, no network. Open it by double-clicking.

**A baked file made from a real index contains every path in it.** It is exactly as sensitive as
the index. `dist-demo/` and every `graph.json` are in `.gitignore` and neither is ever committed
(D67, R9).

## Layout of the source

| file | what is in it |
|---|---|
| `src/snapshot.ts` | the shape of what the daemon sends, and the only door data enters through |
| `src/layout.ts` | both layouts — pure, deterministic, iterative rather than recursive |
| `src/palette.ts` | the visual encoding, in one table the legend also reads |
| `src/scene.ts` | three.js: one instanced mesh for the nodes, one line buffer for the edges |
| `src/consolidation.ts` | the five-step playback of a pass |
| `src/hud.ts` | the panels |
| `src/i18n.ts` | interface strings, German by default |

## What it does not do

- **No force-directed layout.** It would settle differently on every reload; see D66.
- **No filtering, sorting or searching.** D4 forbids data processing in a frontend: the view
  draws the snapshot it was given. Which nodes are in it is the daemon's decision, and the
  reason is printed in the panel under "Grundlage".
- **No concept nodes yet.** They are step 2 of consolidation and need an embedding model
  (D62). Every consolidation report says so under "not done in this pass".
