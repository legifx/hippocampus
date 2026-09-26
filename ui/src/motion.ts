/**
 * The named springs from `docs/06-DESIGN-SYSTEM.md` §5, verbatim.
 *
 * They are named after what they are for rather than after their physics, because a call site
 * that says `spring.expand` stays right when the numbers are tuned and a call site that says
 * `{ stiffness: 380 }` does not.
 *
 * Reduced motion (§7) is a hard requirement, so every spring goes through `withMotion`, which
 * collapses it to a 120 ms tween. Reading the preference once at module load would be wrong —
 * a person can change it while the app is open — so it is read per call, which is a media query
 * lookup and costs nothing at the rate these are used.
 */

import type { Transition } from "framer-motion";

export type SpringName =
  | "expand"
  | "collapse"
  | "pulse"
  | "dock"
  | "undock"
  | "content";

const SPRINGS: Record<SpringName, Transition> = {
  expand: { type: "spring", stiffness: 380, damping: 30, mass: 1.1 },
  collapse: { type: "spring", stiffness: 260, damping: 33, mass: 1.15 },
  pulse: { type: "spring", stiffness: 700, damping: 22, mass: 0.6 },
  dock: { type: "spring", stiffness: 440, damping: 32, mass: 0.9 },
  undock: { type: "spring", stiffness: 240, damping: 32, mass: 1.0 },
  content: { type: "spring", stiffness: 420, damping: 34, mass: 0.9 },
};

/** Timing constants from §5, in seconds — Framer Motion's unit. */
export const TIMING = {
  contentExit: 0.14,
  rowStagger: 0.03,
  iconCrossfade: 0.24,
  sheen: 0.5,
  accentSweep: 0.45,
  hoverGrace: 0.9,
} as const;

export function prefersReducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

/** A named spring, or a flat 120 ms tween when the person asked for less motion. */
export function spring(name: SpringName): Transition {
  if (prefersReducedMotion()) return { duration: 0.12 };
  return SPRINGS[name];
}

/**
 * The design kit's four spring names (`ui/inspo/.../src/lib/motion.ts`), mapped onto the six
 * above.
 *
 * The kit's components are written against `springSnappy`, `springSoft`, `spring` and
 * `springBouncy`. Keeping that vocabulary at the call sites is worth four aliases; keeping the
 * physics in *one* table is worth more, so each kit name resolves to a named spring here instead
 * of carrying its own numbers — otherwise the window would have two springs called "fast" that
 * drift apart the first time one of them is tuned. The mapping is in
 * `docs/06-DESIGN-SYSTEM.md` §5, next to the springs it points at.
 */
export type KitSpringName = "spring" | "springSoft" | "springSnappy" | "springBouncy";

const KIT_SPRINGS: Record<KitSpringName, SpringName> = {
  spring: "content",
  springSoft: "collapse",
  springSnappy: "pulse",
  springBouncy: "dock",
};

/**
 * A kit-named spring, reduced motion included.
 */
export function kitSpring(name: KitSpringName): Transition {
  return spring(KIT_SPRINGS[name]);
}

/**
 * The kit's `tweenFast` — `{ duration: 0.18, ease: easeOutQuint }` — which is what its header
 * arrow, its row hover labels and its small icon swaps move on. They are *content*, not geometry,
 * and §5 says so out loud: the named springs move a box, the easing curves move what is written in
 * it. The curve is `--hc-ease-out` in `tokens.css`, which is the kit's `easeOutQuint` verbatim, so
 * the one curve is still chosen in one place; only the duration is paired with it here.
 *
 * Reduced motion gets no duration at all rather than a short one: an arrow that slides in 0 ms is
 * the arrow simply being there, which is what the person asked for.
 */
export function tweenFast(): Transition {
  if (prefersReducedMotion()) return { duration: 0 };
  return { duration: 0.18, ease: [0.22, 1, 0.36, 1] };
}

/**
 * The kit's `pop` — what its menus and toasts enter on. `springBouncy` above, as a function
 * rather than a constant so the reduced-motion check happens at render time and not at import
 * time (a constant would freeze whichever preference was set when the module first loaded).
 */
export function pop(): Transition {
  return kitSpring("springBouncy");
}

/**
 * The result list does **not** stagger (§5, "Result list rule"): staggering two hundred rows
 * costs the whole 80 ms keystroke budget. Stagger belongs to surfaces with fewer than about
 * twenty elements — the sidebar and the panels.
 */
export function stagger(index: number, enabled: boolean): number {
  if (!enabled || prefersReducedMotion()) return 0;
  return Math.min(index, 20) * TIMING.rowStagger;
}

/**
 * The breath layer's period, derived from indexing load (§6).
 *
 * Six seconds when idle, down to 2.4 at full ingest. This is the only place background work is
 * made visible, and it replaces a progress bar on purpose: a heartbeat says "something is
 * happening" without asking to be read.
 */
export function breathPeriod(load: number): number {
  const clamped = Math.min(Math.max(load, 0), 1);
  return 6 - clamped * 3.6;
}
