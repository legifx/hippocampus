/**
 * Zone accents (D15).
 *
 * The hue arrives from the daemon, already derived from the zone's identity and already snapped
 * to a twelve-step wheel — deriving it again here would be a second implementation of a
 * decision that has to be stable across clients. Saturation and lightness are the theme's, so
 * no zone can be louder than another.
 *
 * Colour is never the only signal: every zone has a name, and the row shows it.
 */

import type { CSSProperties } from "react";

export const ZONE_SATURATION = "58%";
export const ZONE_LIGHTNESS = "62%";
const BORDER_ALPHA = 0.85;
const SELECTED_TINT_ALPHA = 0.06;

/**
 * The row's zone, handed to the stylesheet as custom properties.
 *
 * What this replaces was two inline declarations on the row — `borderLeftColor` and `background`.
 * An inline declaration beats every rule in every stylesheet, so the row's own recipe in `app.css`,
 * which is where the ported row skin lives, was never painted on a selected row at all: the
 * stylesheet said `background-color: var(--hc-accent-soft)` and the element computed to
 * `rgba(0, 0, 0, 0)`, bar included. A custom property cannot win that way — nothing else sets one —
 * so the hue travels as data and both D15 (the zone is this row's left border) and the kit's row
 * recipe stay in force, each in its own rule.
 *
 * A row without a zone gets no properties at all, so `app.css`'s fallbacks apply: no bar, and the
 * selection bar takes the accent colour.
 */
export function zoneVars(hue: number | null | undefined): CSSProperties {
  if (hue === null || hue === undefined) return {};
  const vars: Record<string, string> = {
    "--hc-zone": `hsl(${hue}, ${ZONE_SATURATION}, ${ZONE_LIGHTNESS})`,
    "--hc-zone-border": `hsla(${hue}, ${ZONE_SATURATION}, ${ZONE_LIGHTNESS}, ${BORDER_ALPHA})`,
    "--hc-zone-soft": `hsla(${hue}, ${ZONE_SATURATION}, ${ZONE_LIGHTNESS}, ${SELECTED_TINT_ALPHA})`,
  };
  return vars as CSSProperties;
}

export function zoneAccent(hue: number | null | undefined): string {
  if (hue === null || hue === undefined) return "var(--hc-info)";
  return `hsl(${hue}, ${ZONE_SATURATION}, ${ZONE_LIGHTNESS})`;
}
