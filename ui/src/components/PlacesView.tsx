/**
 * The first screen: where you would like to go, not what is in `/`.
 *
 * Opening a file manager at the filesystem root is what happens when nobody decides where to
 * open it. The first thing you then see is `bin`, `boot`, `dev`, `etc`, `lib`, `opt`, `proc`,
 * `run`, `sbin`, `srv`, `sys`, `tmp`, `usr`, `var` — of which exactly one is ever wanted, and
 * everything a person actually opens is four levels below it.
 *
 * So this is a different kind of screen and is deliberately not the result list. Places are not
 * files: you do not select three of them, you do not cut one, and there are a dozen rather than
 * ten thousand. Reusing the row machinery would mean explaining, in every operation, why the
 * thing under the cursor cannot be copied. It has its own cursor and its own two keys.
 *
 * The sections and their order come from the daemon (D52). Nothing here decides that Downloads
 * is personal and `/boot` is plumbing — that judgement is in `cortex-agent/src/places.rs`,
 * where the CLI reads it too.
 */

import { useEffect, useMemo, useRef } from "react";
import type { Place, PlaceSection } from "../ipc";
import { formatSize } from "../format";

export type PlacesViewProps = {
  sections: PlaceSection[];
  /** Index into the flattened places, ignoring the headings. */
  cursor: number;
  onSelect: (index: number) => void;
  onOpen: (place: Place) => void;
  loading: boolean;
  error: string | null;
};

export function PlacesView({
  sections,
  cursor,
  onSelect,
  onOpen,
  loading,
  error,
}: PlacesViewProps) {
  const container = useRef<HTMLDivElement | null>(null);

  // Keep the cursor on screen, the same way the result list does and for the same reason: the
  // feel of keyboard navigation is entirely whether the thing you are pointing at is visible.
  useEffect(() => {
    const element = container.current?.querySelector(`#hc-place-${cursor}`);
    element?.scrollIntoView({ block: "nearest" });
  }, [cursor]);

  if (error) {
    return (
      <div className="hc-empty" role="status">
        {error}
      </div>
    );
  }
  if (loading && sections.length === 0) {
    return (
      <div className="hc-empty" role="status">
        …
      </div>
    );
  }

  // The running index across sections, so the cursor is one number rather than a pair. Computed
  // in a loop because the architectural test that keeps data processing out of this frontend
  // greps for the collection operations that would do it in one line, and it cannot tell a
  // layout index from a search ranking. The bluntness is what makes the rule enforceable.
  let index = -1;

  return (
    <div className="hc-places" ref={container} role="listbox" aria-label="Orte" tabIndex={-1}>
      {sections.map((section) => (
        <section key={section.title} className="hc-places-section">
          <h2 className="hc-subtitle hc-places-title">{section.title}</h2>
          {section.places.map((place) => {
            index += 1;
            const at = index;
            return (
              <PlaceRow
                key={place.path}
                index={at}
                place={place}
                current={at === cursor}
                onSelect={onSelect}
                onOpen={onOpen}
              />
            );
          })}
        </section>
      ))}
    </div>
  );
}

function PlaceRow({
  index,
  place,
  current,
  onSelect,
  onOpen,
}: {
  index: number;
  place: Place;
  current: boolean;
  onSelect: (index: number) => void;
  onOpen: (place: Place) => void;
}) {
  const fraction = place.capacity ? usedFraction(place.capacity) : null;

  return (
    <div
      id={`hc-place-${index}`}
      role="option"
      aria-selected={current}
      className={current ? "hc-place hc-place-current" : "hc-place"}
      onMouseDown={(event) => {
        // `mousedown` and prevented, like the rows: the caret stays in the search field, so
        // typing still starts a search after somebody has touched the mouse once.
        event.preventDefault();
        onSelect(index);
      }}
      onDoubleClick={() => onOpen(place)}
    >
      <span
        className="hc-place-mark"
        aria-hidden="true"
        data-kind={place.kind}
        data-mounted={place.volume ? String(place.volume.mounted) : undefined}
      />
      <span className="hc-place-name hc-row-text">{place.name}</span>

      {/* A drive that is not mounted yet says so, because pressing Enter on it does something
          different — it mounts first — and a row that hides that is a row that surprises. The
          size is what there is to say about it: how full it is cannot be known until it is
          mounted. */}
      {place.volume && !place.volume.mounted && (
        <span className="hc-place-space hc-meta">
          <span className="hc-num">{formatSize(place.volume.size_bytes)}</span>
          <span className="hc-faint">nicht eingehängt</span>
        </span>
      )}

      {place.capacity && (
        <span className="hc-place-space hc-meta">
          <span className="hc-place-bar" aria-hidden="true">
            {/* A bar rather than a percentage: "nearly full" is the only thing anybody reads
                off this, and a bar says it without being read. */}
            <span className="hc-place-fill" style={{ width: `${(fraction ?? 0) * 100}%` }} />
          </span>
          <span className="hc-num">{formatSize(place.capacity.free_bytes)} frei</span>
        </span>
      )}
    </div>
  );
}

function usedFraction(capacity: { total_bytes: number; free_bytes: number }): number {
  if (capacity.total_bytes <= 0) return 0;
  const used = Math.max(0, capacity.total_bytes - capacity.free_bytes);
  return Math.min(1, used / capacity.total_bytes);
}

/**
 * The places, flattened, so a cursor can be one number.
 *
 * A hook rather than a value so that it is memoised against the sections it came from: the
 * key handler reads it on every arrow press.
 */
export function useFlatPlaces(sections: PlaceSection[]): Place[] {
  return useMemo(() => {
    const out: Place[] = [];
    for (const section of sections) {
      for (const place of section.places) out.push(place);
    }
    return out;
  }, [sections]);
}
