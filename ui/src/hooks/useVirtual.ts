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

import { useCallback, useEffect, useRef, useState } from "react";

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
  /** Rows rendered beyond each edge while still, so a small scroll shows no blank space. */
  overscan?: number;
};

/** Which way the view is moving, so the spare rows can be put where they are needed. */
export type Direction = "up" | "down" | "still";

export const DEFAULT_OVERSCAN = 6;

/**
 * Spare rows in the direction of travel, and behind it.
 *
 * Symmetric overscan spends half its budget on rows that have just been scrolled *away* from
 * and will not be looked at again. Measured on a 650 px viewport of 28 px rows, with six rows
 * spare on each side:
 *
 * | wheel speed | blank space per frame |
 * |---|---|
 * | 150 px/frame | none |
 * | 200 px/frame | 28 px |
 * | 300 px/frame | 128 px |
 * | 420 px/frame | 240 px |
 *
 * A third of the list, empty, on every frame of a brisk flick, filling in one frame later. That
 * is what "the files load while you scroll" is: not loading, but rows that do not exist yet
 * because the browser painted the new scroll position before React could put anything there.
 *
 * The same budget, weighted towards where the eye is going, covers a flick more than twice as
 * fast — and the DOM bound M4 §3 asserts is unchanged, because the total is what it was.
 */
export const LEAD_ROWS = 12;
export const TRAIL_ROWS = 2;

/** The pure part, so it can be tested without a DOM and asserted against directly. */
export function windowFor(
  scrollTop: number,
  viewportHeight: number,
  { count, rowHeight, overscan = DEFAULT_OVERSCAN }: VirtualOptions,
  direction: Direction = "still",
): Window {
  const totalHeight = count * rowHeight;
  if (count === 0 || viewportHeight <= 0 || rowHeight <= 0) {
    return { start: 0, end: 0, padTop: 0, totalHeight };
  }

  const clampedScroll = Math.min(Math.max(scrollTop, 0), Math.max(totalHeight - viewportHeight, 0));
  const firstVisible = Math.floor(clampedScroll / rowHeight);
  const visibleCount = Math.ceil(viewportHeight / rowHeight);

  const [above, below] =
    direction === "down"
      ? [TRAIL_ROWS, LEAD_ROWS]
      : direction === "up"
        ? [LEAD_ROWS, TRAIL_ROWS]
        : [overscan, overscan];

  const start = Math.max(0, firstVisible - above);
  const end = Math.min(count, firstVisible + visibleCount + below);

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
  // The larger of the two arrangements: symmetric while still, weighted while moving. They are
  // deliberately the same size, so this is one number rather than a state to reason about.
  const spare = Math.max(overscan * 2, LEAD_ROWS + TRAIL_ROWS);
  return Math.ceil(viewportHeight / rowHeight) + spare + 1;
}

/** Whether two windows would render the same rows in the same places. */
function same(a: Window, b: Window): boolean {
  return (
    a.start === b.start &&
    a.end === b.end &&
    a.padTop === b.padTop &&
    a.totalHeight === b.totalHeight
  );
}

export function useVirtual(options: VirtualOptions) {
  const { count, rowHeight, overscan = DEFAULT_OVERSCAN } = options;
  // A callback ref rather than `useRef`, because the scrollable element does not exist on
  // every render: an empty list renders a message instead, and a `useRef` measured once at
  // mount would still be holding `null` when the first results arrive. That produced a list
  // with the right scroll height and no rows in it, which is a silent failure — the container
  // looks scrollable and is empty.
  const [element, setElement] = useState<HTMLDivElement | null>(null);
  const ref = useCallback((node: HTMLDivElement | null) => setElement(node), []);

  const [viewportHeight, setViewportHeight] = useState(0);

  // The scroll position is a ref and the *window* is the state.
  //
  // This used to be the other way round: every scroll event set `scrollTop`, so every scroll
  // event re-rendered the list — including the many events that move the view by a few pixels
  // and leave exactly the same rows on screen. A wheel produces a great many of those, and each
  // one cost a render whose result was identical to what was already there.
  //
  // Storing the window instead means a scroll that changes nothing visible changes no state,
  // React never renders, and the DOM is never touched.
  const scrollTop = useRef(0);
  /** The pending animation frame, so a burst of scroll events costs one recompute. */
  const frame = useRef<number | null>(null);
  /** Where the spare rows go. Reset to `still` is deliberate: see `recompute`. */
  const direction = useRef<Direction>("still");

  // Everything except the scroll position that decides the window. When one of these changes
  // the window is wrong *now*, not on the next frame.
  const inputs = `${viewportHeight}|${count}|${rowHeight}|${overscan}`;
  const [state, setState] = useState<{ window: Window; inputs: string }>(() => ({
    window: windowFor(0, 0, { count, rowHeight, overscan }),
    inputs: "",
  }));

  if (state.inputs !== inputs) {
    // A resize or a new list is not a scroll: the spare rows go back to both sides, because
    // there is no direction of travel to favour.
    direction.current = "still";
    // Adjusting state during render, which React documents for exactly this: the alternative is
    // an effect, and an effect commits one frame with the old window first. That frame is
    // visible — a list that has just been given its height renders a short window and then
    // corrects itself — and it is what a browser test caught by finding twelve rows where there
    // should have been thirty.
    setState({
      window: windowFor(scrollTop.current, viewportHeight, { count, rowHeight, overscan }, "still"),
      inputs,
    });
  }

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

  const recompute = useCallback(() => {
    const next = windowFor(
      scrollTop.current,
      viewportHeight,
      { count, rowHeight, overscan },
      direction.current,
    );
    // The functional form, because this runs from an animation frame and the state it compares
    // against must be the current one rather than the one captured when the frame was booked.
    setState((previous) =>
      same(previous.window, next) ? previous : { window: next, inputs: previous.inputs },
    );
  }, [viewportHeight, count, rowHeight, overscan]);

  /**
   * One recompute per frame, however many scroll events arrive.
   *
   * WebKit will happily deliver scroll events faster than it paints, and answering each of them
   * with a React render is work whose result is thrown away before anybody sees it. An
   * animation frame is exactly the rate at which the answer can matter.
   */
  const onScroll = useCallback(
    (event: React.UIEvent<HTMLDivElement>) => {
      const position = event.currentTarget.scrollTop;
      if (position !== scrollTop.current) {
        direction.current = position > scrollTop.current ? "down" : "up";
      }
      scrollTop.current = position;
      if (frame.current !== null) return;
      frame.current = requestAnimationFrame(() => {
        frame.current = null;
        recompute();
      });
    },
    [recompute],
  );

  useEffect(
    () => () => {
      if (frame.current !== null) cancelAnimationFrame(frame.current);
    },
    [],
  );

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

      const top = index * rowHeight;
      const bottom = top + rowHeight;

      if (top < element.scrollTop) {
        element.scrollTop = Math.max(0, top - padTop);
      } else if (bottom + padTop + padBottom > element.scrollTop + element.clientHeight) {
        element.scrollTop = bottom + padTop + padBottom - element.clientHeight;
      }
    },
    [element, rowHeight],
  );

  return {
    ref,
    onScroll,
    scrollToIndex,
    viewportHeight,
    window: state.window,
  };
}
