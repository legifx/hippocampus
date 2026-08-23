/**
 * The visual encoding, in one place.
 *
 * Every channel carries exactly one variable, and the mapping is written down here so that the
 * legend in the interface and the picture on screen cannot drift apart — they read the same
 * table.
 *
 * | channel     | variable                    |
 * |-------------|-----------------------------|
 * | hue         | zone (the daemon's own hue) |
 * | brightness  | tier                        |
 * | size        | salience                    |
 * | halo        | in the Working Set          |
 * | ring        | pinned                      |
 * | edge colour | edge type                   |
 *
 * Nothing encodes file type, and that is deliberate: this view is about what the brain thinks,
 * not about what the filesystem holds.
 */

import type { Tier } from "./snapshot";

export interface Rgb {
  r: number;
  g: number;
  b: number;
}

export const BACKGROUND = 0x05070c;

/** How bright and how saturated a node is drawn, per tier. */
export const TIER_STYLE: Record<Tier, { lightness: number; saturation: number; alpha: number }> = {
  working: { lightness: 0.72, saturation: 0.95, alpha: 1.0 },
  stm: { lightness: 0.55, saturation: 0.75, alpha: 0.92 },
  ltm: { lightness: 0.38, saturation: 0.45, alpha: 0.72 },
  cold: { lightness: 0.24, saturation: 0.18, alpha: 0.5 },
};

/** Edge colours by type. Association is warm, containment is structural and stays quiet. */
export const EDGE_STYLE: Record<string, { color: Rgb; alpha: number }> = {
  contains: { color: { r: 0.32, g: 0.42, b: 0.62 }, alpha: 0.22 },
  co_access: { color: { r: 1.0, g: 0.62, b: 0.28 }, alpha: 0.85 },
  similar_to: { color: { r: 0.66, g: 0.5, b: 1.0 }, alpha: 0.7 },
  annotates: { color: { r: 0.4, g: 0.92, b: 0.66 }, alpha: 0.8 },
  derived_from: { color: { r: 0.9, g: 0.45, b: 0.6 }, alpha: 0.7 },
  imports: { color: { r: 0.45, g: 0.8, b: 0.95 }, alpha: 0.6 },
};

export const EDGE_FALLBACK = { color: { r: 0.6, g: 0.6, b: 0.6 }, alpha: 0.4 };

/** The hue a node with no zone gets: a neutral steel, so "unsorted" is visibly not a project. */
const NO_ZONE_HUE = 215;
const NO_ZONE_SATURATION = 0.12;

export function hslToRgb(h: number, s: number, l: number): Rgb {
  const hue = ((h % 360) + 360) % 360 / 360;
  if (s === 0) return { r: l, g: l, b: l };
  const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
  const p = 2 * l - q;
  const channel = (t: number): number => {
    let v = t;
    if (v < 0) v += 1;
    if (v > 1) v -= 1;
    if (v < 1 / 6) return p + (q - p) * 6 * v;
    if (v < 1 / 2) return q;
    if (v < 2 / 3) return p + (q - p) * (2 / 3 - v) * 6;
    return p;
  };
  return { r: channel(hue + 1 / 3), g: channel(hue), b: channel(hue - 1 / 3) };
}

/** The colour of one node: zone for hue, tier for everything else. */
export function nodeColor(hue: number | undefined, tier: Tier): Rgb {
  const style = TIER_STYLE[tier] ?? TIER_STYLE.cold;
  if (hue === undefined) {
    return hslToRgb(NO_ZONE_HUE, NO_ZONE_SATURATION * style.saturation, style.lightness * 0.9);
  }
  return hslToRgb(hue, style.saturation, style.lightness);
}

/**
 * Radius from salience.
 *
 * A square root, not a linear scale: area is what the eye compares, so a linear radius would
 * make a node twice as salient look four times as important. Bounded at both ends — a node
 * with no salience must still be visible, or an unconsolidated index looks empty.
 */
export function nodeRadius(salience: number, isDir: boolean): number {
  const base = isDir ? 0.85 : 0.5;
  return base + Math.sqrt(Math.max(0, salience)) * 1.05;
}

export function toHex(c: Rgb): number {
  const clamp = (v: number) => Math.max(0, Math.min(255, Math.round(v * 255)));
  return (clamp(c.r) << 16) | (clamp(c.g) << 8) | clamp(c.b);
}

export function css(c: Rgb, alpha = 1): string {
  const clamp = (v: number) => Math.max(0, Math.min(255, Math.round(v * 255)));
  return `rgba(${clamp(c.r)}, ${clamp(c.g)}, ${clamp(c.b)}, ${alpha})`;
}
