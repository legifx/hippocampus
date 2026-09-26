/**
 * The icons the ports of the kit's components need.
 *
 * The kit imports them from `lucide-react`; this window may not add that dependency
 * (`ui/src/test/architecture.test.ts` allows three runtime dependencies), so they are
 * written out here. The geometry is lucide's — a 24-grid, round caps and joins, the same
 * stroke weight the kit passes (`1.75`) — and every icon takes its size from the control it
 * sits in, because `.gb svg` / `.gi-icon svg` / `.gm-item-icon svg` size them in `em` (§3).
 * The one exception is `FolderOpenIcon`, which the kit draws at `strokeWidth={1.4}` in its
 * empty state, where it is a 28 px glyph and not a 16 px one.
 */

interface IconProps {
  className?: string;
}

export function SearchIcon({ className }: IconProps) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      className={className}
    >
      <circle cx="11" cy="11" r="7" />
      <path d="M16.5 16.5 21 21" />
    </svg>
  );
}

export function XIcon({ className }: IconProps) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      className={className}
    >
      <path d="M6 6l12 12M18 6 6 18" />
    </svg>
  );
}

export function CheckIcon({ className }: IconProps) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      className={className}
    >
      <path d="M4.5 12.5 9.5 17.5 19.5 6.5" />
    </svg>
  );
}

export function ChevronDownIcon({ className }: IconProps) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      className={className}
    >
      <path d="m6 9.5 6 6 6-6" />
    </svg>
  );
}

/**
 * An open folder, for the empty state of a listing that is empty rather than wrong.
 *
 * The kit's `EmptyState` defaults to this glyph at `strokeWidth={1.4}`, which is the weight to keep
 * here: it is drawn at 28 px in the 72 px circle, and 1.75 on a glyph that size is a heavy outline
 * where the kit has a drawing.
 */
export function FolderOpenIcon({ className }: IconProps) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.4}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      className={className}
    >
      <path d="m6 14 1.45-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.55 6a2 2 0 0 1-1.94 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.93a2 2 0 0 1 1.66.9l.82 1.2a2 2 0 0 0 1.66.9H18a2 2 0 0 1 2 2v2" />
    </svg>
  );
}
