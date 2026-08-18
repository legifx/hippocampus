/**
 * Presentation only.
 *
 * The line D4 draws is between *processing* data and *displaying* it: turning 1 048 576 into
 * "1.0 MB" is display, and deciding which rows to show is not. Nothing in this file takes a
 * collection, and that is the rule that keeps it on the right side of the line.
 */

const UNITS = ["B", "kB", "MB", "GB", "TB"] as const;

export function formatSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "";
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < UNITS.length - 1) {
    value /= 1000;
    unit += 1;
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  return `${value.toFixed(digits)} ${UNITS[unit]}`;
}

/**
 * How long ago, in words. Relative rather than absolute because the question a result list
 * answers is "is this the recent one", and a date makes the reader do that arithmetic.
 */
export function formatAge(unixSeconds: number, now = Date.now() / 1000): string {
  const age = Math.max(0, now - unixSeconds);
  if (age < 90) return "just now";
  const minutes = age / 60;
  if (minutes < 90) return `${Math.round(minutes)} min`;
  const hours = minutes / 60;
  if (hours < 36) return `${Math.round(hours)} h`;
  const days = hours / 24;
  if (days < 14) return `${Math.round(days)} d`;
  const weeks = days / 7;
  if (weeks < 9) return `${Math.round(weeks)} w`;
  const months = days / 30.44;
  if (months < 18) return `${Math.round(months)} mo`;
  return `${Math.round(days / 365.25)} y`;
}

/** The file name, and the directory that holds it, split for two-tone rendering. */
export function splitPath(path: string): { dir: string; name: string } {
  const cut = path.lastIndexOf("/");
  if (cut < 0) return { dir: "", name: path };
  return { dir: path.slice(0, cut + 1), name: path.slice(cut + 1) };
}

/**
 * Shorten a directory from the left, keeping the end.
 *
 * The end is what distinguishes two results; the beginning is usually the same home directory
 * over and over. Cutting from the left is the opposite of what an ellipsis usually does and is
 * right for exactly this reason.
 */
export function elideStart(text: string, max: number): string {
  if (text.length <= max) return text;
  return `…${text.slice(text.length - max + 1)}`;
}

/** The tier badge, or nothing. Cold results are marked so the tiering model is legible (M4). */
export function tierLabel(tier: string): string | null {
  switch (tier) {
    case "cold":
      return "cold";
    case "ltm":
      return "archive";
    default:
      return null;
  }
}
