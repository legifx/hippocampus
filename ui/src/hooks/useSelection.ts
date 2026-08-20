/**
 * Which rows are selected.
 *
 * A file manager without multiple selection is a file viewer, so this is the thing that had to
 * exist before copy, move and delete were worth building. The model is the one every file
 * manager uses and every person already knows:
 *
 * - a plain click or an arrow key selects one and moves the anchor;
 * - **Shift** extends from the anchor, which stays put, so shift-clicking twice picks a range
 *   rather than growing one;
 * - **Ctrl** toggles one and moves the anchor to it;
 * - Ctrl+A takes everything, Escape takes nothing.
 *
 * Selection is by *index* rather than by path, because the list a person is looking at is
 * ordered and "everything between here and there" is a statement about that order. It is reset
 * whenever the list changes underneath it — a selection that survived a directory change would
 * be pointing at whatever happened to land in those positions.
 */

import { useCallback, useEffect, useMemo, useState } from "react";

export type SelectionState = {
  /** The row the cursor is on. Always exactly one, and always in range. */
  cursor: number;
  /** Every selected index. Contains the cursor except in an empty list. */
  indices: Set<number>;
  /** Where a shift-extension measures from. */
  anchor: number;
};

export type SelectionMove = "up" | "down" | "pageUp" | "pageDown" | "home" | "end";

/** How far a page key moves. Ten rows is what fits in the smallest sensible window. */
export const PAGE = 10;

export function useSelection(count: number, resetKey: unknown) {
  const [cursor, setCursor] = useState(0);
  const [anchor, setAnchor] = useState(0);
  const [indices, setIndices] = useState<Set<number>>(() => new Set([0]));

  // A new list is a new selection. The alternative — keeping index 3 selected — would mean
  // deleting whatever happens to be third in the directory somebody just navigated into.
  useEffect(() => {
    setCursor(0);
    setAnchor(0);
    setIndices(count > 0 ? new Set([0]) : new Set());
  }, [resetKey, count]);

  const clamp = useCallback((index: number) => Math.min(Math.max(index, 0), Math.max(count - 1, 0)), [count]);

  /** Move the cursor, replacing the selection or extending it from the anchor. */
  const move = useCallback(
    (how: SelectionMove, extend: boolean) => {
      // Nothing to move to, and nothing to select. Without this, moving in an empty directory
      // clamps to zero and selects a row that is not there — which then gets copied or deleted
      // by whatever acts on the selection next.
      if (count === 0) return;

      setCursor((current) => {
        const next = clamp(
          how === "up"
            ? current - 1
            : how === "down"
              ? current + 1
              : how === "pageUp"
                ? current - PAGE
                : how === "pageDown"
                  ? current + PAGE
                  : how === "home"
                    ? 0
                    : count - 1,
        );

        if (extend) {
          setIndices(rangeBetween(anchor, next));
        } else {
          setAnchor(next);
          setIndices(new Set([next]));
        }
        return next;
      });
    },
    [anchor, clamp, count],
  );

  /** A plain click: this row and nothing else. */
  const select = useCallback(
    (index: number) => {
      if (index < 0 || index >= count) return;
      setCursor(index);
      setAnchor(index);
      setIndices(new Set([index]));
    },
    [count],
  );

  /** Shift: everything from the anchor to here. The anchor does not move. */
  const extendTo = useCallback(
    (index: number) => {
      if (index < 0 || index >= count) return;
      setCursor(index);
      setIndices(rangeBetween(anchor, index));
    },
    [anchor, count],
  );

  /** Ctrl: this row in or out, and the anchor comes with it. */
  const toggle = useCallback((index: number) => {
    if (index < 0 || index >= count) return;
    setCursor(index);
    setAnchor(index);
    setIndices((current) => {
      const next = new Set(current);
      if (!next.delete(index)) next.add(index);
      return next;
    });
  }, [count]);

  const selectAll = useCallback(() => {
    setIndices(new Set(Array.from({ length: count }, (_, i) => i)));
  }, [count]);

  const clear = useCallback(() => {
    setIndices(new Set(cursor >= 0 && count > 0 ? [cursor] : []));
    setAnchor(cursor);
  }, [cursor, count]);

  const state: SelectionState = useMemo(
    () => ({ cursor, indices, anchor }),
    [cursor, indices, anchor],
  );

  return { state, move, select, extendTo, toggle, selectAll, clear };
}

/** Inclusive both ends, in either direction. */
export function rangeBetween(from: number, to: number): Set<number> {
  const low = Math.min(from, to);
  const high = Math.max(from, to);
  const out = new Set<number>();
  for (let i = low; i <= high; i++) out.add(i);
  return out;
}
