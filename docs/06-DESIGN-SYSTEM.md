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

## 3. Typography

Stack: `Inter Variable`, then system sans. Variable weight axis is used for arrival animation.

| Role | Size | Weight | Tracking | Line height |
|---|---|---|---|---|
| Title | 15.5px | 600 | −0.012em | 1.2 |
| Subtitle | 13.5px | 450 | — | 1.3 |
| Body | 14px | 520 | −0.006em | 1.38 |
| Row (file name) | 13.5px | 500 | −0.004em | 1.35 |
| Meta (size, date) | 12px | 500 | — | 1.3 |
| Button | 13.5px | 550 | −0.005em | — |
| Pill / badge | 12.5px | 500 | — | — |

Numbers always use `font-variant-numeric: tabular-nums` — file sizes and dates must not jitter
while a list updates.

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
   opacity 0.4 → 1 → 0.4. Period is derived from **indexing load**: 6 s idle, down to 2.4 s at
   full ingest. This is the system's heartbeat and it makes background work legible without a
   progress bar.
2. **Glow** — accent hairline plus inner glow on the focused panel.
3. **Rim** — specular highlight tracking the cursor, 150 × 80.

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
