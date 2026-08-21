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

/**
 * The ceiling on the lead, and therefore on the DOM.
 *
 * Twelve rows covers 336 px of travel per frame and leaves a 420 px flick with a gap. The lead
 * is now taken from how far the view actually moved on the last frame instead of being fixed,
 * which closes that gap — but an unbounded lead is a list that renders more the faster somebody
 * flicks, and M4 §3 requires a bound. Twenty rows covers 560 px per frame, which is a third
 * again beyond the fastest flick this is asked to survive.
 *
 * The cost is paid only while the view is moving. When it stops, the lead goes back to
 * `LEAD_ROWS` and the resting window is the size it was before any of this — the *direction*
 * stays, deliberately, because un-weighting it on the frame after every wheel notch is a second
 * render for the same rows. Only a new list or a resize puts the spare rows back on both sides.
 */
export const MAX_LEAD_ROWS = 20;

/**
 * How many rows to keep ahead, given how far the last frame moved.
 *
 * Prediction from the previous frame, which is the only measurement available at the moment the
 * window has to be decided. It is right for steady scrolling, which is what a wheel and a
 * trackpad produce.
 *
 * **A frame that travels more than a viewport is not scrolling and predicts nothing.** Dragging
 * the scrollbar, or `scrollToIndex` jumping to a match, moves the view somewhere unrelated and
 * then stops; there is no continuity with what was on screen, so there is nothing for a lead to
 * be continuous with. Sizing the lead from that distance borrows twenty rows to cover motion
 * that has already finished — a measurable transient in the node count with nothing to show for
 * it, which is how this was found: a jump left the list at 46 rows for one frame and made an
 * existing bound assertion fail under load.
 */
export function leadFor(perFrame: number, rowHeight: number, viewportHeight: number): number {
  if (!(perFrame > 0) || rowHeight <= 0) return LEAD_ROWS;
  if (viewportHeight > 0 && perFrame > viewportHeight) return LEAD_ROWS;
  // Plus one for the row the scroll position lands inside. A frame that travels an exact
  // multiple of the row height still exposes a partial row at the edge unless the position
  // happens to be row-aligned, and being one row short is a visible band of empty list.
  const needed = Math.ceil(perFrame / rowHeight) + 1;
  return Math.min(MAX_LEAD_ROWS, Math.max(LEAD_ROWS, needed));
}

/** The pure part, so it can be tested without a DOM and asserted against directly. */
export function windowFor(
  scrollTop: number,
  viewportHeight: number,
  { count, rowHeight, overscan = DEFAULT_OVERSCAN }: VirtualOptions,
  direction: Direction = "still",
  lead: number = LEAD_ROWS,
): Window {
  const totalHeight = count * rowHeight;
  if (count === 0 || viewportHeight <= 0 || rowHeight <= 0) {
    return { start: 0, end: 0, padTop: 0, totalHeight };
  }

  const clampedScroll = Math.min(Math.max(scrollTop, 0), Math.max(totalHeight - viewportHeight, 0));
  const firstVisible = Math.floor(clampedScroll / rowHeight);
  const visibleCount = Math.ceil(viewportHeight / rowHeight);

  const ahead = Math.min(MAX_LEAD_ROWS, Math.max(LEAD_ROWS, Math.floor(lead)));
  const [above, below] =
    direction === "down"
      ? [TRAIL_ROWS, ahead]
      : direction === "up"
        ? [ahead, TRAIL_ROWS]
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
  // The larger of the two arrangements: symmetric while still, weighted while moving. The
  // weighted one is no longer the same size as the symmetric one — the lead grows with the
  // speed of the scroll — so this is the worst case, which is the lead at its ceiling.
  const spare = Math.max(overscan * 2, MAX_LEAD_ROWS + TRAIL_ROWS);
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
  /** The scroll position at the previous recompute, so a frame's travel can be measured. */
  const previous = useRef(0);
  /** How far the frame before that one travelled, so the lead follows sustained motion. */
  const before = useRef(0);
  /** How many rows to keep ahead, derived from that travel. */
  const lead = useRef(LEAD_ROWS);

  // Everything except the scroll position that decides the window. When one of these changes
  // the window is wrong *now*, not on the next frame.
  const inputs = `${viewportHeight}|${count}|${rowHeight}|${overscan}`;
  const [state, setState] = useState<{ window: Window; inputs: string }>(() => ({
    window: windowFor(0, 0, { count, rowHeight, overscan }),
    inputs: "",
  }));

  if (state.inputs !== inputs) {
    // A resize or a new list is not a scroll: the spare rows go back to both sides, because
    // there is no direction of travel to favour, and the lead goes back to its floor with them.
    direction.current = "still";
    lead.current = LEAD_ROWS;
    previous.current = scrollTop.current;
    before.current = 0;
    // Adjusting state during render, which React documents for exactly this: the alternative is
    // an effect, and an effect commits one frame with the old window first. That frame is
    // visible — a list that has just been given its height renders a short window and then
    // corrects itself — and it is what a browser test caught by finding twelve rows where there
    // should have been thirty.
    setState({
      window: windowFor(
        scrollTop.current,
        viewportHeight,
        { count, rowHeight, overscan },
        "still",
        LEAD_ROWS,
      ),
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

  /**
   * Book the one frame, from wherever. A ref because `recompute` books its own follow-up and
   * would otherwise have to refer to itself while it is still being defined.
   */
  const recomputeRef = useRef<() => void>(() => {});
  const schedule = useCallback(() => {
    if (frame.current !== null) return;
    frame.current = requestAnimationFrame(() => {
      frame.current = null;
      recomputeRef.current();
    });
  }, []);

  const recompute = useCallback(() => {
    // How far the view travelled since the last time this ran, which is one frame ago whenever
    // scrolling is under way. This is the measurement the lead is sized from; asking the wheel
    // how fast it is going would answer in the wheel's units, not the list's.
    const position = scrollTop.current;
    const travelled = Math.abs(position - previous.current);
    previous.current = position;

    // The lead follows the *slower* of the last two frames, so one frame on its own can never
    // buy rows. A single long step is `scrollToIndex` landing on a match, or a click in the
    // scrollbar trough, or the first frame of a flick — and only the third of those continues.
    // Borrowing on the strength of one frame put 46 rows in the DOM for a jump that was already
    // over, and gave them back a frame later, which is a burst of DOM mutations in exactly the
    // place D56 got them down to zero.
    const sustained = Math.min(travelled, before.current);
    before.current = travelled;

    if (travelled === 0) {
      // Settled: give back what was borrowed, and *only* that.
      //
      // The direction stays. Resetting it here as well looked tidier and broke the first of
      // D56's two fixes: a slow scroll would weight the window towards its travel on one frame
      // and un-weight it on the next, so the rows changed twice per wheel notch and the test
      // for "a scroll too small to change a row costs nothing" went from zero DOM mutations to
      // ninety-six. A settled view has still been travelling one way, and the spare rows belong
      // where it was going.
      lead.current = LEAD_ROWS;
    } else {
      lead.current = leadFor(sustained, rowHeight, viewportHeight);
    }

    const next = windowFor(
      position,
      viewportHeight,
      { count, rowHeight, overscan },
      direction.current,
      lead.current,
    );
    // The functional form, because this runs from an animation frame and the state it compares
    // against must be the current one rather than the one captured when the frame was booked.
    setState((prior) => (same(prior.window, next) ? prior : { window: next, inputs: prior.inputs }));

    // One more frame after a moving one, so the settle above is reached without waiting for a
    // scroll event that will never come — the browser stops sending them the moment the view
    // stops, which is exactly when the borrowed rows should be handed back.
    if (travelled !== 0) schedule();
  }, [viewportHeight, count, rowHeight, overscan, schedule]);

  recomputeRef.current = recompute;

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
      schedule();
    },
    [schedule],
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
