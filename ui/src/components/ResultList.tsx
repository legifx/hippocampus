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

import { memo, useLayoutEffect } from "react";
import { elideStart, formatAge, formatSize, tierLabel } from "../format";
import { useVirtual } from "../hooks/useVirtual";
import type { Row } from "../row";
import { zoneBorder, zoneTint } from "../zone";

export type ResultListProps = {
  rows: Row[];
  selected: number;
  onSelect: (index: number) => void;
  /** Enter, or a double click: descend into a directory, open a file. */
  onActivate: (row: Row) => void;
  rowHeight: number;
  /** Hue per zone id, as the daemon derived it (D15). */
  zoneHues: Map<number, number>;
  emptyMessage: string;
};

export function ResultList({
  rows,
  selected,
  onSelect,
  onActivate,
  rowHeight,
  zoneHues,
  emptyMessage,
}: ResultListProps) {
  const virtual = useVirtual({ count: rows.length, rowHeight });

  // Keeping the selected row visible is the whole of keyboard navigation's feel, and it belongs
  // here rather than in the key handler: this is the component that knows where the row is.
  // `useLayoutEffect`, not `useEffect` — the latter runs after paint, which would show the
  // selection off-screen for one frame on every arrow key.
  const { scrollToIndex } = virtual;
  useLayoutEffect(() => {
    if (selected >= 0) scrollToIndex(selected);
  }, [selected, scrollToIndex]);

  if (rows.length === 0) {
    return (
      <div className="hc-empty" role="status">
        {emptyMessage}
      </div>
    );
  }

  const { start, end, padTop, totalHeight } = virtual.window;
  const visible = rows.slice(start, end);

  return (
    <div
      className="hc-list"
      ref={virtual.ref}
      onScroll={virtual.onScroll}
      role="listbox"
      aria-label="Einträge"
      aria-activedescendant={`hc-row-${selected}`}
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
                selected={index === selected}
                hue={row.zoneId === null ? null : zoneHues.get(row.zoneId) ?? null}
                rowHeight={rowHeight}
                onSelect={onSelect}
                onActivate={onActivate}
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
  hue: number | null;
  rowHeight: number;
  onSelect: (index: number) => void;
  onActivate: (row: Row) => void;
};

const ListRow = memo(function ListRow({
  index,
  row,
  selected,
  hue,
  rowHeight,
  onSelect,
  onActivate,
}: ListRowProps) {
  const tier = row.tier ? tierLabel(row.tier) : null;

  return (
    <div
      id={`hc-row-${index}`}
      role="option"
      aria-selected={selected}
      className={rowClass(row, selected)}
      style={{
        height: rowHeight,
        borderLeftColor: zoneBorder(hue),
        background: selected ? zoneTint(hue) : undefined,
      }}
      onMouseDown={(e) => {
        // `mousedown` rather than `click`: the input keeps focus, so typing continues to work
        // after a pointer selection. Losing focus to a row would break keyboard-first operation
        // the moment somebody touched the mouse once.
        e.preventDefault();
        onSelect(index);
      }}
      onDoubleClick={() => onActivate(row)}
    >
      <span className="hc-row-name hc-row-text">
        {row.isDir && <span className="hc-row-dirmark" aria-hidden="true" />}
        {row.name}
      </span>
      {row.dir && <span className="hc-row-dir hc-meta">{elideStart(row.dir, 48)}</span>}
      {row.snippet && <span className="hc-row-snippet hc-meta">{row.snippet}</span>}
      <span className="hc-row-meta hc-meta">
        {tier && <span className="hc-pill hc-tier">{tier}</span>}
        {!row.indexed && (
          <span className="hc-pill hc-unindexed" title="Der Index kennt diese Datei noch nicht">
            neu
          </span>
        )}
        <span className="hc-num">{row.isDir ? "" : formatSize(row.size)}</span>
        <span className="hc-num">{formatAge(row.mtime)}</span>
      </span>
    </div>
  );
});

function rowClass(row: Row, selected: boolean): string {
  const classes = ["hc-row"];
  if (selected) classes.push("hc-row-selected");
  if (row.isDir) classes.push("hc-row-directory");
  return classes.join(" ");
}
