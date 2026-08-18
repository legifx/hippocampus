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

export const ZONE_SATURATION = "58%";
export const ZONE_LIGHTNESS = "62%";
const BORDER_ALPHA = 0.85;
const SELECTED_TINT_ALPHA = 0.06;

export function zoneBorder(hue: number | null | undefined): string {
  if (hue === null || hue === undefined) return "transparent";
  return `hsla(${hue}, ${ZONE_SATURATION}, ${ZONE_LIGHTNESS}, ${BORDER_ALPHA})`;
}

/** Only ever applied to the selected row (D15: never a background fill on unselected rows). */
export function zoneTint(hue: number | null | undefined): string {
  if (hue === null || hue === undefined) return "transparent";
  return `hsla(${hue}, ${ZONE_SATURATION}, ${ZONE_LIGHTNESS}, ${SELECTED_TINT_ALPHA})`;
}

export function zoneAccent(hue: number | null | undefined): string {
  if (hue === null || hue === undefined) return "var(--hc-info)";
  return `hsl(${hue}, ${ZONE_SATURATION}, ${ZONE_LIGHTNESS})`;
}
