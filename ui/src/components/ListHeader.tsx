/**
 * Column headers over a directory listing: what each column is, which one orders the list, and
 * a click to change that.
 *
 * The list has always been sortable — by name, size, date and salience, from the palette. What
 * it did not have was a visible statement of *what the columns are* and *which one is in
 * charge*, which every file manager on this desk has and two of the three reviews of
 * 2026-09-05 missed. This is that statement, one line, no grid, no resizing: the row's own
 * layout is name on the left and the metadata on the right, and the header follows it.
 *
 * Not shown while searching — a result list is ordered by relevance and a header claiming
 * "Name ▲" over it would be a lie — and not on the places screen, which has no columns.
 */

import type { Order } from "../ipc";
import { AnimatePresence, motion } from "framer-motion";
import { tweenFast } from "../motion";

export type ListHeaderProps = {
  order: Order;
  reverse: boolean;
  onOrder: (order: Order) => void;
  /** Whether the metadata column shows the date too (narrow windows drop it). */
  narrow: boolean;
  /** Whether the rows carry the memory columns at all. A share with no index has none. */
  hasSalience: boolean;
};

type Column = { order: Order; label: string; className: string };

const COLUMNS: Column[] = [
  { order: "name", label: "Name", className: "hc-list-header-name" },
  { order: "salience", label: "Salienz", className: "hc-list-header-salience" },
  { order: "size", label: "Größe", className: "hc-list-header-size" },
  { order: "modified", label: "Geändert", className: "hc-list-header-modified" },
];

export function ListHeader({ order, reverse, onOrder, narrow, hasSalience }: ListHeaderProps) {
  const cell = (column: Column) => {
    const active = column.order === order;
    return (
      <button
        key={column.order}
        type="button"
        role="columnheader"
        aria-sort={active ? (reverse ? "descending" : "ascending") : "none"}
        className={
          active
            ? `hc-list-header-cell ${column.className} hc-list-header-active`
            : `hc-list-header-cell ${column.className}`
        }
        title={active ? "nochmal wählen kehrt um" : `nach ${column.label} ordnen`}
        // mousedown, so the search field keeps the keyboard.
        onMouseDown={(event) => event.preventDefault()}
        onClick={() => onOrder(column.order)}
      >
        {column.label}
        {/* The kit's HeaderCell arrow, ported: it slides in from the direction it points — down for
            descending, up for ascending — and it is keyed on the direction, so flipping the sort
            plays the slide again instead of snapping the glyph over. It used to be a plain span with
            `▲`/`▼` in it, which is the same *information* and none of the movement: the kit's arrow
            is what says „I have just changed my mind" rather than only „this is the rule now". */}
        <AnimatePresence initial={false}>
          {active && (
            <motion.span
              key={reverse ? "desc" : "asc"}
              className="hc-list-header-arrow"
              initial={{ opacity: 0, y: reverse ? -4 : 4 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0 }}
              transition={tweenFast()}
              aria-hidden="true"
            >
              {reverse ? "▼" : "▲"}
            </motion.span>
          )}
        </AnimatePresence>
      </button>
    );
  };

  // Which columns are on this window: a narrow one drops the date, a listing with no memory
  // columns has no salience to order by. Written out rather than filtered — the frontend does
  // not reshape collections (see `architecture.test.ts`), and this is a layout decision.
  const [name, salience, size, modified] = COLUMNS as [Column, Column, Column, Column];

  return (
    // The kit's strip material, as the kit's classes — `glass-flat` for the edge and `glass-weak`
    // for the fill — so the heading line is a surface above the list rather than a line in it
    // (app.css has the why, and what it took to see it).
    <div className="hc-list-header glass-flat glass-weak" role="row" aria-label="Spalten">
      {cell(name)}
      <span className="hc-list-header-meta">
        {hasSalience && cell(salience)}
        {cell(size)}
        {!narrow && cell(modified)}
      </span>
    </div>
  );
}
