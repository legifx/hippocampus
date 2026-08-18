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
