# 06 — Design System

The visual and motion language is derived from the **island-ui** design system (a
Dynamic-Island-style React + Framer Motion component library). Hippocampus is not an overlay
notification surface, so the components do not transfer directly — **the tokens, the motion
model and the layering discipline do.**

This document is the contract. The GUI implements these values; it does not invent new ones.
If a new token is needed, add it here first.

## 1. Principles inherited from island-ui

1. **Dark, near-black ground.** The surface is a single dark plane; hierarchy comes from
   elevation and stroke, not from competing background colours.
2. **One accent at a time.** Accent colour is per-event and per-context, never a permanent
   rainbow. This is what makes D15's zone colours viable — they are accents, not fills.
3. **Motion is spring-based, never linear.** Every state change is a spring with named physics.
   Duration-based easing is reserved for content, not for geometry.
4. **Ambient layers are compositor-only.** They animate `opacity` and `transform` exclusively.
   Nothing that triggers layout or paint animates.
5. **Exit is faster than entry.** Content leaves in ~140 ms and arrives in ~420 ms. The interface
   feels responsive because it gets out of the way quickly.

## 2. Colour tokens

```css
--hc-bg:            #09090b;   /* ground */
--hc-surface:       #0e0e11;   /* raised rows, panels */
--hc-surface-hi:    #141418;   /* hover / selected row */
--hc-stroke:        rgba(255, 255, 255, 0.075);
--hc-stroke-lift:   rgba(255, 255, 255, 0.10);
--hc-highlight:     inset 0 1px 0 rgba(255, 255, 255, 0.09);

--hc-text:          rgba(255, 255, 255, 0.96);
--hc-text-dim:      rgba(255, 255, 255, 0.55);
--hc-text-faint:    rgba(255, 255, 255, 0.32);

--hc-info:          #0a84ff;
--hc-success:       #30d158;
--hc-warning:       #ffb86b;
--hc-error:         #ff453a;
--hc-neutral:       #ffffff;
```

Elevation:

```css
--hc-shadow-rest:   0 12px 40px rgba(0,0,0,0.55), 0 2px 8px rgba(0,0,0,0.4),
                    inset 0 1px 0 rgba(255,255,255,0.09);
--hc-shadow-lift:   0 20px 56px rgba(0,0,0,0.62), 0 4px 14px rgba(0,0,0,0.45),
                    inset 0 1px 0 rgba(255,255,255,0.16);
```

### Zone accents (D15)

Zone hue is derived deterministically: `hue = stable_hash(zone_id) mod 360`, then snapped to a
12-step wheel to avoid near-identical neighbours. **Saturation and lightness are fixed by the
theme**, so no zone can be louder than another:

```css
--hc-zone-sat:      58%;
--hc-zone-light:    62%;
--hc-zone-alpha:    0.85;   /* border */
--hc-zone-alpha-bg: 0.06;   /* only for the selected row's tint */
```

A zone renders as a **2 px left border** on the row, plus the accent token for that row's focus
ring. Never a background fill on unselected rows. Colour is never the only signal — every zone
also carries a name and an icon.

**The materials come from the showcase's design kit, and each one is written down once.**
Since 2026-09-14 (D110, D111) the vocabulary above is carried by `ui/src/glass.css` — `glass`,
`glass-flat`, `glass-strong`, `glass-sheen`, `glass-specular`, `raised`, `raised-bezel`, `well`,
`solid` — ported once from `ui/inspo/minimalist-glassmorphism-file-explorer/src/styles/glass.css`
onto these tokens, and `ui/src/kit.css` holds the class bodies of the kit's controls. The
primitives live in `ui/src/components/glass/` and are used, not re-drawn: a panel that needs a
fill takes a kit material, it does not mix its own. Where a rule in this document and the kit
disagree, the kit wins for materials and this document wins for arrangement; and anything the kit
was extended to do is written down in D111, not added silently.

## 3. Typography

Stack: `SF Pro Display` first — the owner asked for it, so the window uses it wherever it exists on
the machine — then `SF Pro Text`, `-apple-system`, `Inter Display`, `Inter`, then the system sans. SF
cannot be *shipped*: it is Apple's and its licence does not allow redistribution. `Inter` is what the
window falls back to on Linux, and what the pictures in this repository show; the fallback is stated
here so nobody reads a screenshot as a Mac. The variable weight axis is used for arrival animation.

**The steps are fluid.** Seven of them, each clamped to the window width — a floor, a ceiling and a
shallow slope between. This replaced forty-odd fixed sizes chosen one at a time, any two of which
could disagree; the disagreement the owner could see was the search field, whose text was 15.5 px in
a field that grew with the window. The slope is shallow on purpose: type is not a layout that can be
stretched, and a step that grows as fast as the window turns a wide window into a poster. The numbers
are set so that at about 1024 px every step is what it used to be.

| Token | 400 px | ~1024 px | 1440 px | What it is for |
|---|---|---|---|---|
| `--hc-text-2xs` | 10 | 11 | 12.5 | the small tracked label over a number |
| `--hc-text-xs` | 11 | 12 | 13.5 | metadata, status, a table's headings |
| `--hc-text-sm` | 12 | 13 | 15 | controls, counts, a row of the list |
| `--hc-text-md` | 13 | 14 | 16 | body: the prose in the panels |
| `--hc-text-lg` | 14 | 15.5 | 19 | the title, and the search field's own text |
| `--hc-text-xl` | 17 | 20 | 24 | a number on a card |
| `--hc-text-2xl` | 21 | 25 | 32 | the daemon's sentence as a headline |

**A box is measured in its own type.** Padding and height of a control are in `em` against the step
the control is set in, so the search field's height *is* its text with room around it instead of a
second, independent number. That is what made the field and its text agree only at one window width.

Weights and tracking are unchanged, and belong to the role rather than to the step: Title 600 /
−0.012em, Subtitle 450, Body 520 / −0.006em, Row 500 / −0.004em, Meta 500, Button 550 /
−0.005em, Pill 500.

Numbers always use `font-variant-numeric: tabular-nums` — file sizes and dates must not jitter
while a list updates.

### The columns

A heading is a claim about what is under it, so a heading and its values are given the same track:
the widths are `em` against the same step (the heading row and a row's metadata are both set in
`--hc-text-xs`), and both groups are right-aligned against the same inset. When the list has no room
for the table, the numbers give way rather than the metadata pushing out over the panel beside it
(`min-width: 0` on the group, a floor and `flex-shrink` on the cells). The permission string is the
only cell with no heading — there is no order called „rechte“ — and it gives way first, clipping
with its full text in its `title`. A directory has no size in a listing, so the cell carries `—`,
the same mark the inspector uses for a number it does not have; a blank cell reads as a zero.

## 4. Geometry

Radii follow the island-ui geometry scale rather than a generic 4/8/12 ramp:

| Element | Radius |
|---|---|
| Row / list item | 13 |
| Panel / card | 20 |
| Overlay container | 26 |
| Drawer / large surface | 26 top, 0 bottom |
| Pill / badge | 999 |

Layout constants:

- Left alignment gutter: `--hc-lead: 50px` (icon socket + gutter), matching island-ui's `--lead`
- Icon grid: 24 × 24, stroke 1.6 px, round caps and joins
- Row height: 28 px compact, 34 px comfortable — configurable, both must pass the 10 000-row
  benchmark
- Max preview panel width: 460 px (island-ui's max card width) at narrow columns; it may grow
  proportionally above 900 px viewport width

### Panels inside the one toplevel

niri owns the layout and this project ships no second window (M4's constraints), so anything
that would be a dialog elsewhere is a panel here. Two of them exist — the command palette and
the Agent Inspector — and they follow one shape:

| | |
|---|---|
| Position | `absolute; inset: 0` inside `.hc-overlay`, which is `position: relative` for exactly this |
| Cover or beside | **cover.** At 400 px there is no beside, and a panel that is a column at 1400 px and a sliver at 400 is two designs |
| Radius | the overlay's 26, because it is the overlay's own edge that is showing |
| Two panes | `grid-template-columns: minmax(0, 1fr)` below 900 px and two columns above; the second pane's separator moves from a top border to a left one |
| Escape | closes the panel and returns focus to the search field. It does *not* reach the window's own Escape |
| Focus | moves into the panel when it opens. Without that the keyboard is still talking to what is behind it |
| Keys | **every control is reachable and activatable without a mouse (M4 §7).** A `<button>` wired to `onMouseDown` is not: a keyboard raises `click` and nothing else, which made the Inspector entirely mouse-operable until a browser test said so |
| Cost | nothing polls while a panel is shut, nothing polls a tab that is not showing, and **nothing polls while the window is hidden** (§8.6). A hidden GTK window keeps running its page: the shell says whether anybody can see it, and `useVisibility` is how a component listens |

**Do not reuse a class from another component to get its shape.** `.hc-facts` is a definition
list that elides on one line, and the Inspector borrowed it for a list that has to wrap: every
allow-list path inherited `white-space: nowrap` and pushed its box 166 px wide at every width.
Three attempts to fix it with `overflow-wrap` and `min-width` failed because none of them
mentioned the property that was actually wrong. A panel that wraps is a different component from
a sidebar that elides, and it needs a class of its own.

## 5. Motion

Named springs, used verbatim from island-ui:

| Name | Stiffness | Damping | Mass | Use |
|---|---|---|---|---|
| `expand` | 380 | 30 | 1.10 | opening a panel, overlay in |
| `collapse` | 260 | 33 | 1.15 | closing |
| `pulse` | 700 | 22 | 0.60 | brief impulse, e.g. result count changed |
| `dock` | 440 | 32 | 0.90 | hover arrival |
| `undock` | 240 | 32 | 1.00 | hover departure |
| `content` | 420 | 34 | 0.90 | row animation |

Easing (for content, not geometry):

```css
--hc-ease-island: cubic-bezier(0.16, 1, 0.3, 1);
--hc-ease-sheet:  cubic-bezier(0.32, 0.72, 0, 1);
```

Timing constants:

| Event | Duration |
|---|---|
| Content exit | 140 ms |
| Row stagger | 30 ms |
| Icon crossfade | 240 ms |
| Sheen pass (overlay open) | 500 ms |
| Accent sweep | 450 ms |
| Hover grace after pointer leaves | 900 ms |

**Result list rule:** when search results update, rows do **not** stagger. Staggering 200 rows
costs the 80 ms budget. Stagger applies only to the context sidebar and to panels — surfaces
with fewer than about twenty elements.

## 6. Ambient layers

At most three, all `position: absolute; inset: 0; pointer-events: none`, animating only
`opacity` and `transform`:

1. **Breath** — `radial-gradient(90% 150% at 50% -35%, rgba(255,255,255,0.06), transparent 70%)`,
   opacity 0.4 → 1 → 0.4. Period is derived from **indexing load**: 2.4 s at full ingest,
   lengthening towards 6 s as the queue drains. This is the system's heartbeat and it makes
   background work legible without a progress bar. **With nothing to index it does not beat at
   all** — see §8.6 for what a continuous animation costs a resident window, and note that a
   still layer is also the honest answer when nothing is happening.
2. **Glow** — accent hairline plus inner glow on the focused panel.
3. **Rim** — specular highlight tracking the cursor, 150 × 80.

**The canvas under all of it is flat, and that is a rule and not an omission.** A kit's backdrop
belongs to the kit's showcase: the `minimalist-glassmorphism` kit paints three very large coloured
glows behind its glass, and this window carried them for one session (D114 is about the drift that
came with them). They went again because of the concept, not the cost — the owner's verdict was
„Die glows nehmen irgendwie die Minimalistic raus aus dem GUI Konzept": colour in the background is
the first thing a tool does not need, and the glass here is something the panels *are*, not something
to be admired. Measured, the difference was small and all of it in the colour: the file list's median
50.0 → 42.5 and its blue-over-red bias +8.4 → +2.4 when the glows left. **What lights this window is
the breath and nothing else** (§6.1), and it lights it because there is work to report. D115 has the
decision and the two calm levers that remain if a surface ever measures too flat again.

The mic ring, sheen and sweep layers from island-ui are not used in the explorer except for the
overlay's opening sheen.

## 7. Reduced motion

When `prefers-reduced-motion: reduce`:

- springs collapse to `{ duration: 0.12 }`
- breath becomes a static 0.7 opacity
- row arrival loses rise and blur
- hover lift is disabled

This is a hard requirement, not a nicety.

## 8. Performance rules that override aesthetics

1. No `backdrop-filter` on any element that appears more than once. Blur on a 10 000-row list
   destroys the frame budget. Blur is permitted on the overlay container only.
2. No `box-shadow` on list rows. Selection is a background change plus a border, never a shadow.
3. No layout-triggering property is ever animated. `transform` and `opacity` only.
4. Every list is virtualised. The DOM node count is bounded and asserted by a test (M4 DoD §3).
5. If a visual effect cannot hold 60 fps at 10 000 rows, it is removed — not optimised later.
6. **No animation runs continuously in a resident window.** An ambient layer animates while it
   has something to say and stops when it does not. This is not a preference: with any animation
   running, the overlay costs about **one CPU core** while it is on screen, and stopping it
   brings the same window to 0.3% — independently of the renderer, the size of the animated
   element, or what is being drawn (D59). A window that is open from login to logout pays that
   for as long as it is looked at.

## 9. The shell: the rail and the start screen

Added 2026-09-14, in `src/shell.css`, loaded after `app.css`. Nothing in `app.css` was rewritten:
the overlay keeps its grid and gains one column for the rail.

**The rail.** Two destinations and one panel — *Übersicht*, *Dateien*, *Inspector*. `--hc-rail`
(200 px) beside a `minmax(0, 1fr)` stage, full window height. Under 720 px it collapses to
`--hc-rail-narrow` (58 px): the glyphs stay, the words and the counts go, and the rail keeps its
height — an icon column, not a squeezed sidebar. The footer's *values* go with its keys, and the
agent chip keeps its dot and loses its word: at 58 px a value is not shortened by ellipsis, it
becomes `/ho….`, and the word survives as the chip's `title` and accessible name rather than as a
colour alone. In the header, the sort label goes the same way — the search field is what has to keep
its width at 400 px.

**The start screen.** One column, at most 1180 px wide: the daemon's own sentence, then tiles, then
cards, then the daemon's state. Cards are a `minmax(min(320px, 100%), 1fr)` grid, so at 400 px
there is one per row and the screen never scrolls sideways. Every number on it is printed as the
daemon sent it, the two sentences included — the daemon counted, the window repeats (D4).

**Material.** Three surfaces, all from `--hc-glass-*` — and every number in them is the kit's dark
theme, not a value this window arrived at by looking (D113): the fill 0.06, the rim `inset 0 1px 0
rgba(255, 255, 255, 0.16)`, the bottom shade `inset 0 -1px 0 rgba(0, 0, 0, 0.45)`, the sheen 0.24
under `--hc-sheen-opacity: 0.4`, the drop shadow 0.45 black, and — because the kit separates a glass
surface from its neighbours with a dark seam rather than a light outline — a `0 0 0 1px` ring of
`--hc-glass-seam` (`rgba(0, 0, 0, 0.35)`). `--hc-glass-edge` keeps this window's light 0.055 white
for the one job the kit does not give it: it is the colour of every panel divider in `shell.css`. A
convex surface (`.raised`, `.raised-flat`, `.raised-bezel`) is the kit's pair of *solid* greys,
`#3b3b44` over `#2a2a31`, under the kit's 1 px light top rim — a translucent white wash over
`#101012` is 26 levels lighter at one end and 4 at the other, which is a tint and not an object. The
overlay is the only place a blur is allowed (§8.1); the rail is a gradient fill with a 1 px rim on
its right edge; the cards keep the outer drop shadow: a handful of surfaces painted once, not rows
that repeat (§8.2).

**A material is a list, and a token in it is listed rather than spelled out.** Each of these tokens
carries its own geometry — `--hc-glass-rim` *is* `inset 0 1px 0 …` — so a recipe writes
`var(--hc-glass-rim)` and never `inset 0 1px 0 0 var(--hc-glass-rim)`. The second form is invalid,
and a declaration that is invalid at computed-value time is not partially applied: the browser drops
the entire shadow. It had happened to every material class in `glass.css` (D112); what holds it now
is `ui/tests/materials.spec.ts`.

**Two shapes to keep.** A card is a panel with a header and a body — the header says what the card
is about, the body holds at most one list or one bar. And a chip is a state, not a number: it is
read at a glance and never the only thing that carries its meaning.
## §10 Liquid (D125, 2026-09-27)

The owner's brief after the brain view landed: "still very monotonous and ugly — liquid glass,
small animations, minimalist". Implemented as one layer, `ui/src/liquid.css`, loaded last, with its
values in `tokens.css` §2c. It changes surfaces and motion, not layout decisions the tests measure.

| | what | why |
|---|---|---|
| Canvas | `--hc-canvas` plus four static radial glows (`--hc-glow-1…4`) on `.hc-overlay` | glass needs something behind it; painted once, never moves (D114) |
| Islands | rail, search field, list, preview, side panel, cards, brain panels: translucent fill lightening to the top, rim light, dark lower seam, soft drop (`--hc-lq-*`); gaps instead of hairlines | the showcase's material, without `backdrop-filter` (§8.1) |
| Pills | nav, buttons, segmented thumbs: convex gradient with a bright top rim | the showcase's "liquid" controls |
| Active place | one `.hc-nav-pill` with a shared framer-motion `layoutId`, sliding between destinations | the one continuous motion, and only on a click |
| Accents | a free-standing bar or a dot, never a left border on a rounded box (the radius bent those into brackets) | |
| Status | tinted chips (`--hc-tint-*`) for on/warn/off | as in the showcase |
| Sheets | the inspector is opaque (`--hc-lq-fill-solid`, alpha 1) — even 1.6 % transparency showed the list through | |
| Motion | one-shot only: sections rise (`hc-rise`), the file list fades, sheets slide in, rows and cards light on hover, presses sink to 0.96; `prefers-reduced-motion` turns all of it off, a hidden window already has `animation: none` | D59 — nothing loops |
| Rows | one line: name, folder (faint, gives way first), numbers (give way last, from the left) | two stacked lines in a 28 px row overlapped |
