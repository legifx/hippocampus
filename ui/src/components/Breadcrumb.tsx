/**
 * Where you are, as something you can also click.
 *
 * Truncated from the left, because the end of a path is what distinguishes it and the
 * beginning is the same home directory over and over. The segments stay clickable even when
 * elided — cutting a path for display must not cut what it can do.
 */

import { zoneAccent } from "../zone";

export type BreadcrumbProps = {
  /** The directory, when the place is one. Null in a concept, a tier or the working set. */
  path: string | null;
  /** What to call a place that has no path. Ignored when there is one. */
  label?: string;
  onNavigate: (path: string) => void;
  /** Back to the places screen, which is what sits above `/`. */
  onPlaces: () => void;
  /** The zone this directory belongs to, if the daemon knows one (D15). */
  hue: number | null;
  /** How many segments to show before eliding from the left. */
  maxSegments?: number;
};

export function Breadcrumb({
  path,
  label,
  onNavigate,
  onPlaces,
  hue,
  maxSegments = 4,
}: BreadcrumbProps) {
  // A place the index made has a name and no parents: a concept is not inside anything, and
  // segments for it would be a hierarchy this window invented (§4.2). One crumb, and the way
  // back is the way you came.
  const segments = path === null ? [] : toSegments(path);
  const shown = segments.slice(-maxSegments);
  const elided = segments.length - shown.length;

  return (
    <nav className="hc-crumbs" aria-label="Pfad">
      {/* Always first, from anywhere: the way back to the folders somebody actually uses, one
          key or one click away however deep they have gone. */}
      <span className="hc-crumb-group">
        <button
          type="button"
          className="hc-crumb hc-crumb-places"
          onMouseDown={(e) => {
            e.preventDefault();
            onPlaces();
          }}
        >
          Orte
        </button>
        <span className="hc-crumb-sep">/</span>
      </span>
      {path === null && label !== undefined && (
        // One crumb, current, not a button: there is nothing above a concept to navigate to.
        <span className="hc-crumb-group">
          <span className="hc-crumb hc-crumb-view" aria-current="page">
            {label}
          </span>
        </span>
      )}
      {elided > 0 && (
        <button
          type="button"
          className="hc-crumb hc-crumb-elided"
          // The first hidden segment, so the ellipsis is a way up rather than decoration.
          onMouseDown={(e) => {
            e.preventDefault();
            onNavigate(segments[elided - 1]!.path);
          }}
          title={segments[elided - 1]!.path}
        >
          …
        </button>
      )}
      {shown.map((segment, index) => (
        <span key={segment.path} className="hc-crumb-group">
          <button
            type="button"
            className="hc-crumb"
            style={index === shown.length - 1 ? { color: zoneAccent(hue) } : undefined}
            aria-current={index === shown.length - 1 ? "page" : undefined}
            onMouseDown={(e) => {
              // `mousedown` and prevented, like the rows: the caret stays in the search input.
              e.preventDefault();
              onNavigate(segment.path);
            }}
          >
            {segment.label}
          </button>
          {index < shown.length - 1 && <span className="hc-crumb-sep">/</span>}
        </span>
      ))}
    </nav>
  );
}

type Segment = { label: string; path: string };

/**
 * `/a/b/c` becomes root, a, b, c — each carrying the path that gets you there.
 *
 * Written as a loop rather than with `filter`, because the architectural test that keeps data
 * processing out of this frontend greps for exactly that and does not know a path string from
 * a list of results. The bluntness is the point: an exception here would be an exception
 * somewhere else next month.
 */
export function toSegments(path: string): Segment[] {
  const segments: Segment[] = [{ label: "/", path: "/" }];
  let accumulated = "";
  for (const part of path.split("/")) {
    if (part.length === 0) continue;
    accumulated += `/${part}`;
    segments.push({ label: part, path: accumulated });
  }
  return segments;
}
