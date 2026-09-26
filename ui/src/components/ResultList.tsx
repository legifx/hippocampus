/**
 * The list: virtualised, unstaggered, and it does no thinking.
 *
 * It renders [`Row`]s, which is what makes the directory and the search results the same list
 * rather than two pages (M6). Two rules from the design system are visible in the markup and
 * both are performance rules that override aesthetics (§8): no `box-shadow` on a row —
 * selection is a background change and a border — and no stagger when the list updates, because
 * staggering two hundred rows costs the entire keystroke budget.
 *
 * The rows arrive in the order the daemon chose. This component never sorts.
 */

import { memo, useLayoutEffect, type ReactNode } from "react";
import { elideStart, fileUri, formatAge, formatSize } from "../format";
import { useThumbnails } from "../hooks/useThumbnails";
import { useVirtual } from "../hooks/useVirtual";
import type { Row } from "../row";
import { zoneVars } from "../zone";

export type ResultListProps = {
  rows: Row[];
  /// Where the keyboard is. Exactly one row, always, and not the same thing as the selection:
  /// a shift-extension moves the cursor through rows that are all selected, and a person needs
  /// to see which one the next key will act from.
  cursor: number;
  /// Every selected row. Usually contains the cursor.
  selected: Set<number>;
  onSelect: (index: number, modifiers: { shift: boolean; ctrl: boolean }) => void;
  /** Enter, or a double click: descend into a directory, open a file. */
  onActivate: (row: Row) => void;
  rowHeight: number;
  /**
   * Whether anybody can see the window.
   *
   * Passed in rather than subscribed to here: the component above already knows, and a hidden
   * overlay that asked the thumbnailer about its rows would be exactly the kind of periodic
   * work for nobody that D59 measured and removed.
   */
  visible: boolean;
  /** Hue per zone id, as the daemon derived it (D15). */
  zoneHues: Map<number, number>;
  emptyMessage: string;
  /**
   * The glyph over the empty state, when there is an empty state to draw.
   *
   * The circle is the kit's (`EmptyState`, and `.hc-empty-mark` in app.css). Which glyph belongs in
   * it depends on *why* the list is empty — a search that found nothing, a folder with nothing in
   * it — and this component does not know that, so the caller passes it. Absent means no circle:
   * a list that is loading and a share that did not answer are not empty states, and a glass circle
   * over a diagnosis is decoration.
   */
  emptyIcon?: ReactNode;
  /**
   * Every selected path, for dragging out of the window.
   *
   * Passed in rather than derived from `selected`, because deriving it would mean walking the
   * rows here — which is the data processing D4 keeps out of this frontend, and which the
   * component above has already done once for every other operation.
   */
  selectedPaths: string[];
};

export function ResultList({
  rows,
  cursor,
  selected,
  onSelect,
  onActivate,
  rowHeight,
  visible: windowVisible,
  zoneHues,
  emptyMessage,
  emptyIcon,
  selectedPaths,
}: ResultListProps) {
  const virtual = useVirtual({ count: rows.length, rowHeight });

  // Keeping the selected row visible is the whole of keyboard navigation's feel, and it belongs
  // here rather than in the key handler: this is the component that knows where the row is.
  // `useLayoutEffect`, not `useEffect` — the latter runs after paint, which would show the
  // selection off-screen for one frame on every arrow key.
  const { scrollToIndex } = virtual;
  useLayoutEffect(() => {
    if (cursor >= 0) scrollToIndex(cursor);
  }, [cursor, scrollToIndex]);

  // Above the empty case on purpose: a hook may not be called conditionally, and the slice is
  // what the thumbnails are asked about.
  const { start, end, padTop, totalHeight } = virtual.window;
  const visible = rows.slice(start, end);
  const thumbnails = useThumbnails(visible, windowVisible);

  if (rows.length === 0) {
    return (
      // The kit's empty state: a glass circle with one glyph in it, and the sentence under it. The
      // circle is `aria-hidden` in the icon itself — the state already says itself in words, and a
      // screen reader announcing „open folder" before the sentence says nothing a person can act on.
      <div className="hc-empty" role="status">
        {emptyIcon !== undefined && (
          <span className="hc-empty-mark glass glass-sheen">{emptyIcon}</span>
        )}
        <p>{emptyMessage}</p>
      </div>
    );
  }

  return (
    <div
      className="hc-list"
      ref={virtual.ref}
      onScroll={virtual.onScroll}
      role="listbox"
      aria-label="Einträge"
      aria-multiselectable="true"
      aria-activedescendant={`hc-row-${cursor}`}
      tabIndex={-1}
    >
      <div style={{ height: totalHeight, position: "relative" }}>
        <div style={{ transform: `translateY(${padTop}px)` }}>
          {visible.map((row, offset) => {
            const index = start + offset;
            return (
              <ListRow
                key={row.key}
                index={index}
                row={row}
                selected={selected.has(index)}
                cursor={index === cursor}
                hue={row.zoneId === null ? null : zoneHues.get(row.zoneId) ?? null}
                rowHeight={rowHeight}
                thumbnail={thumbnails.get(row.path) ?? null}
                onSelect={onSelect}
                onActivate={onActivate}
                selectedPaths={selectedPaths}
              />
            );
          })}
        </div>
      </div>
    </div>
  );
}

type ListRowProps = {
  index: number;
  row: Row;
  selected: boolean;
  cursor: boolean;
  hue: number | null;
  rowHeight: number;
  /** The picture for this row, when the cache has one. `null` is the ordinary case. */
  thumbnail: string | null;
  onSelect: (index: number, modifiers: { shift: boolean; ctrl: boolean }) => void;
  onActivate: (row: Row) => void;
  selectedPaths: string[];
};

const ListRow = memo(function ListRow({
  index,
  row,
  selected,
  cursor,
  hue,
  rowHeight,
  thumbnail,
  onSelect,
  onActivate,
  selectedPaths,
}: ListRowProps) {
  return (
    <div
      id={`hc-row-${index}`}
      role="option"
      aria-selected={selected}
      className={rowClass(row, selected, cursor)}
      // Tier is brightness, not a badge (M10 §4.4): a cold row is dimmer, which is the same
      // encoding the three-dimensional view uses, so the two windows teach one visual language.
      // A badge said "cold" on one row in fifty and said nothing at all about the other
      // forty-nine.
      data-tier={row.tier ?? undefined}
      data-pinned={row.pinned ? "yes" : undefined}
      style={{
        height: rowHeight,
        // The zone travels as data; every rule that paints it is in `app.css` (`zoneVars`). The
        // three colours used to be written here as an inline `borderLeftColor` and `background`,
        // which silently beat the row's own recipe: the selected row computed to
        // `background: rgba(0, 0, 0, 0)` and the kit's fill never appeared.
        ...zoneVars(hue),
      }}
      onMouseDown={(e) => {
        // `mousedown` rather than `click`: the input keeps focus, so typing continues to work
        // after a pointer selection. Losing focus to a row would break keyboard-first operation
        // the moment somebody touched the mouse once.
        e.preventDefault();
        onSelect(index, { shift: e.shiftKey, ctrl: e.ctrlKey || e.metaKey });
      }}
      onDoubleClick={() => onActivate(row)}
      draggable
      onDragStart={(event) => {
        // Dragging a row that is part of the selection drags the whole selection; dragging one
        // that is not drags only it. That is what every file manager does, and the alternative
        // — always dragging the selection — moves files somebody was not pointing at.
        const paths = selected && selectedPaths.length > 0 ? selectedPaths : [row.path];
        let uris = "";
        for (const path of paths) {
          uris += `${fileUri(path)}\r\n`;
        }
        // `text/uri-list` is what the rest of the desktop reads; the plain text is for the
        // things that only take text, which is most editors and every terminal.
        event.dataTransfer.setData("text/uri-list", uris);
        event.dataTransfer.setData("text/plain", paths.join("\n"));
        event.dataTransfer.effectAllowed = "copy";
      }}
    >
      <span className="hc-row-name hc-row-text">
        {row.isDir && <span className="hc-row-dirmark" aria-hidden="true" />}
        {thumbnail !== null && (
          // Only when there is one. Reserving a column for every row would indent every name in
          // a directory of text files for the sake of the one picture in it — and this list is
          // names, not icons. `aria-hidden`, because the name beside it already says which file
          // this is and a screen reader repeating "image" per row says nothing.
          <img className="hc-row-thumb" src={thumbnail} alt="" aria-hidden="true" loading="lazy" />
        )}
        {row.name}
        {row.linkTarget !== null && (
          // The arrow and the target, inline: a link that showed only its own name would be
          // indistinguishable from a copy, which is the difference that matters when somebody
          // decides to delete one of them.
          <span className={row.broken ? "hc-row-link hc-row-link-broken" : "hc-row-link"}>
            {" → "}
            {elideStart(row.linkTarget, 40)}
            {row.broken && <span className="hc-pill hc-row-broken">fehlt</span>}
          </span>
        )}
      </span>
      {row.dir && <span className="hc-row-dir hc-meta">{elideStart(row.dir, 48)}</span>}
      {row.snippet && <span className="hc-row-snippet hc-meta">{row.snippet}</span>}
      <span className="hc-row-meta hc-meta">
        {!row.indexed && (
          // „neu" says what the entry *is* rather than what it measures, but it stays here: the name
          // element is what the window, the tests and a screen reader read as the name, and
          // „neu content.png" is not one. It sits in front of the first fixed track, so it cannot
          // move the columns behind it.
          <span className="hc-pill hc-unindexed" title="Der Index kennt diese Datei noch nicht">
            neu
          </span>
        )}
        {row.mode && (
          <span className="hc-num hc-row-mode" title={row.owner ?? undefined}>
            {row.mode}
          </span>
        )}
        {/* The three columns the header names, in the header's order and in the header's tracks
            (`.hc-row-salience` / `-size` / `-age`, app.css). The salience cell used to be missing
            entirely: the heading said „Salienz" while the first cell under it was the permission
            string, so the column was a claim about a value that never arrived. */}
        {typeof row.salience === "number" && (
          <span className="hc-num hc-row-salience" title="Salienz">
            {row.salience.toFixed(2)}
          </span>
        )}
        {/* A directory has no size of its own in a listing; the dash is what this window puts where a
            number cannot be had (`Rang —` in the inspector), so a blank cell is not mistaken for a
            zero. */}
        <span className="hc-num hc-row-size">{row.isDir ? "—" : formatSize(row.size)}</span>
        <span className="hc-num hc-row-age">{formatAge(row.mtime)}</span>
      </span>
    </div>
  );
});

function rowClass(row: Row, selected: boolean, cursor: boolean): string {
  const classes = ["hc-row"];
  if (selected) classes.push("hc-row-selected");
  // The cursor is drawn even when the row is not selected, which happens after Escape narrows
  // a selection: the keyboard is still somewhere and has to be visible.
  if (cursor) classes.push("hc-row-cursor");
  if (row.isDir) classes.push("hc-row-directory");
  return classes.join(" ");
}