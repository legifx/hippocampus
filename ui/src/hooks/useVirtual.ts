/**
 * Fixed-height windowing.
 *
 * D4 attaches this as a condition on using a web stack at all: every list is virtualised, only
 * visible rows exist in the DOM, and without it the view collapses at ten thousand files. It is
 * sixty lines rather than a dependency because that is all a fixed row height needs, and
 * because the DOM-bound assertion in M4 §3 is easier to trust when the thing being asserted is
 * in the same repository as the assertion.
 *
 * The row height is fixed by the design system (§4: 28 px compact, 34 px comfortable), which is
 * what makes the arithmetic exact — a variable-height virtualiser has to measure, and measuring
 * during a scroll is what makes those libraries janky at the sizes this has to hold.
 */

import { useCallback, useEffect, useState } from "react";

export type Window = {
  /** First index to render. */
  start: number;
  /** One past the last index to render. */
  end: number;
  /** Pixels of empty space standing in for the rows above `start`. */
  padTop: number;
  /** Total height of the scrollable content. */
  totalHeight: number;
};

export type VirtualOptions = {
  count: number;
  rowHeight: number;
  /** Rows rendered beyond each edge, so a fast scroll does not show blank space. */
  overscan?: number;
};

export const DEFAULT_OVERSCAN = 6;

/** The pure part, so it can be tested without a DOM and asserted against directly. */
export function windowFor(
  scrollTop: number,
  viewportHeight: number,
  { count, rowHeight, overscan = DEFAULT_OVERSCAN }: VirtualOptions,
): Window {
  const totalHeight = count * rowHeight;
  if (count === 0 || viewportHeight <= 0 || rowHeight <= 0) {
    return { start: 0, end: 0, padTop: 0, totalHeight };
  }

  const clampedScroll = Math.min(Math.max(scrollTop, 0), Math.max(totalHeight - viewportHeight, 0));
  const firstVisible = Math.floor(clampedScroll / rowHeight);
  const visibleCount = Math.ceil(viewportHeight / rowHeight);

  const start = Math.max(0, firstVisible - overscan);
  const end = Math.min(count, firstVisible + visibleCount + overscan);

  return { start, end, padTop: start * rowHeight, totalHeight };
}

/**
 * The bound M4 §3 asserts: however long the list is, the DOM holds at most a viewport's worth
 * of rows plus the overscan at both edges.
 */
export function maxRenderedRows(
  viewportHeight: number,
  rowHeight: number,
  overscan = DEFAULT_OVERSCAN,
): number {
  return Math.ceil(viewportHeight / rowHeight) + overscan * 2 + 1;
}

export function useVirtual(options: VirtualOptions) {
  // A callback ref rather than `useRef`, because the scrollable element does not exist on
  // every render: an empty list renders a message instead, and a `useRef` measured once at
  // mount would still be holding `null` when the first results arrive. That produced a list
  // with the right scroll height and no rows in it, which is a silent failure — the container
  // looks scrollable and is empty.
  const [element, setElement] = useState<HTMLDivElement | null>(null);
  const ref = useCallback((node: HTMLDivElement | null) => setElement(node), []);

  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(0);

  useEffect(() => {
    if (!element) {
      setViewportHeight(0);
      return;
    }

    setViewportHeight(element.clientHeight);
    if (typeof ResizeObserver === "undefined") return;

    // The window is resized constantly under a tiling compositor, and the row window has to
    // follow it. A resize listener on `window` would miss a column-width change that leaves the
    // window size alone.
    const observer = new ResizeObserver(() => setViewportHeight(element.clientHeight));
    observer.observe(element);
    return () => observer.disconnect();
  }, [element]);

  const onScroll = useCallback((event: React.UIEvent<HTMLDivElement>) => {
    setScrollTop(event.currentTarget.scrollTop);
  }, []);

  /** Bring a row into view. Keyboard navigation is the only caller, and it is mandatory (M4 §7). */
  const scrollToIndex = useCallback(
    (index: number) => {
      if (!element) return;

      // The list is padded, and the rows live inside that padding. `clientHeight` includes it,
      // so scrolling to `bottom - clientHeight` leaves the last row sitting exactly one bottom
      // padding below the edge — visible in the DOM, cut off on screen, and invisible to any
      // assertion that only checks the row exists.
      const style = getComputedStyle(element);
      const padTop = parseFloat(style.paddingTop) || 0;
      const padBottom = parseFloat(style.paddingBottom) || 0;

      const top = index * options.rowHeight;
      const bottom = top + options.rowHeight;

      if (top < element.scrollTop) {
        element.scrollTop = Math.max(0, top - padTop);
      } else if (bottom + padTop + padBottom > element.scrollTop + element.clientHeight) {
        element.scrollTop = bottom + padTop + padBottom - element.clientHeight;
      }
    },
    [element, options.rowHeight],
  );

  return {
    ref,
    onScroll,
    scrollToIndex,
    viewportHeight,
    window: windowFor(scrollTop, viewportHeight, options),
  };
}
