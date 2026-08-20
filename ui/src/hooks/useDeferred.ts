/**
 * A value that lags behind, on purpose.
 *
 * The search input is not debounced and must not be: the budget is 80 ms from keystroke to
 * first row, and a debounce spends it doing nothing. The *selection* is a different question.
 * Holding an arrow key moves through thirty rows a second, and the context sidebar and the
 * preview each fire a round trip per row — sixty requests a second, fifty-nine of which describe
 * a row nobody looked at.
 *
 * So: the list follows the key immediately, and the panes follow the list once it stops. That
 * is not a compromise on responsiveness. It is the difference between what a person is steering
 * and what they are reading.
 */

import { useEffect, useState } from "react";

/** Long enough to skip the rows an arrow key passes through, short enough not to feel held up. */
export const SELECTION_SETTLE_MS = 120;

export function useDeferred<T>(value: T, delayMs = SELECTION_SETTLE_MS): T {
  const [settled, setSettled] = useState(value);

  useEffect(() => {
    if (Object.is(settled, value)) return;
    const timer = setTimeout(() => setSettled(value), delayMs);
    return () => clearTimeout(timer);
    // `settled` is deliberately not a dependency: including it would restart the timer when the
    // deferred value catches up, which is the moment the timer is supposed to be finished.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value, delayMs]);

  return settled;
}

/**
 * Call at most once per interval, and never on a trailing edge.
 *
 * For signals where the *first* one carries the information and the rest are repetition — the
 * daemon's activity ping is exactly that. It wants to know a human is present, which the first
 * keystroke establishes; the next forty say the same thing at the cost of forty round trips.
 */
export function throttled(intervalMs: number): (action: () => void) => void {
  let last = 0;
  return (action) => {
    const now = Date.now();
    if (now - last < intervalMs) return;
    last = now;
    action();
  };
}
