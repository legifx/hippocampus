/**
 * The result list: virtualised, unstaggered, and it does no thinking.
 *
 * Two rules from the design system are visible in the markup and both are performance rules
 * that override aesthetics (§8): no `box-shadow` on a row — selection is a background change
 * and a border — and no stagger when results update, because staggering two hundred rows costs
 * the entire keystroke budget.
 *
 * The rows are given in the order the daemon returned them. This component never sorts.
 */

import { memo, useLayoutEffect } from "react";
import type { SearchHit } from "../ipc";
import { elideStart, formatAge, formatSize, splitPath, tierLabel } from "../format";
import { useVirtual } from "../hooks/useVirtual";
import { zoneBorder, zoneTint } from "../zone";

export type ResultListProps = {
  hits: SearchHit[];
  selected: number;
  onSelect: (index: number) => void;
  onOpen: (hit: SearchHit) => void;
  rowHeight: number;
  /** Hue per zone id, as the daemon derived it (D15). */
  zoneHues: Map<number, number>;
  emptyMessage: string;
};

export function ResultList({
  hits,
  selected,
  onSelect,
  onOpen,
  rowHeight,
  zoneHues,
  emptyMessage,
}: ResultListProps) {
  const virtual = useVirtual({ count: hits.length, rowHeight });

  // Keeping the selected row visible is the whole of keyboard navigation's feel, and it belongs
  // here rather than in the key handler: this is the component that knows where the row is.
  // `useLayoutEffect`, not `useEffect` — the latter runs after paint, which would show the
  // selection off-screen for one frame on every arrow key.
  const { scrollToIndex } = virtual;
  useLayoutEffect(() => {
    if (selected >= 0) scrollToIndex(selected);
  }, [selected, scrollToIndex]);

  if (hits.length === 0) {
    return (
      <div className="hc-empty" role="status">
        {emptyMessage}
      </div>
    );
  }

  const { start, end, padTop, totalHeight } = virtual.window;
  const visible = hits.slice(start, end);

  return (
    <div
      className="hc-list"
      ref={virtual.ref}
      onScroll={virtual.onScroll}
      role="listbox"
      aria-label="Ergebnisse"
      aria-activedescendant={`hc-row-${selected}`}
      tabIndex={-1}
    >
      <div style={{ height: totalHeight, position: "relative" }}>
        <div style={{ transform: `translateY(${padTop}px)` }}>
          {visible.map((hit, offset) => {
            const index = start + offset;
            return (
              <Row
                key={hit.node_id}
                index={index}
                hit={hit}
                selected={index === selected}
                hue={hit.zone_id === null ? null : zoneHues.get(hit.zone_id) ?? null}
                rowHeight={rowHeight}
                onSelect={onSelect}
                onOpen={onOpen}
              />
            );
          })}
        </div>
      </div>
    </div>
  );
}

type RowProps = {
  index: number;
  hit: SearchHit;
  selected: boolean;
  hue: number | null;
  rowHeight: number;
  onSelect: (index: number) => void;
  onOpen: (hit: SearchHit) => void;
};

const Row = memo(function Row({
  index,
  hit,
  selected,
  hue,
  rowHeight,
  onSelect,
  onOpen,
}: RowProps) {
  const { dir, name } = splitPath(hit.path);
  const tier = tierLabel(hit.tier);

  return (
    <div
      id={`hc-row-${index}`}
      role="option"
      aria-selected={selected}
      className={selected ? "hc-row hc-row-selected" : "hc-row"}
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
      onDoubleClick={() => onOpen(hit)}
    >
      <span className="hc-row-name hc-row-text">{name}</span>
      <span className="hc-row-dir hc-meta">{elideStart(dir, 48)}</span>
      {hit.snippet && <span className="hc-row-snippet hc-meta">{hit.snippet}</span>}
      <span className="hc-row-meta hc-meta">
        {tier && <span className="hc-pill hc-tier">{tier}</span>}
        <span className="hc-num">{formatSize(hit.size)}</span>
        <span className="hc-num">{formatAge(hit.mtime)}</span>
      </span>
    </div>
  );
});
