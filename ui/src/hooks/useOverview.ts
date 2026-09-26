/**
 * The start screen's numbers, and when they were true.
 *
 * Read while somebody is looking at the window, then re-read on a slow timer — sixty seconds — and
 * nothing while nobody is: the numbers are real `count(*)` scans over the whole database, and a
 * hidden window that keeps asking for them is exactly the bill D59 was about. A start screen that
 * is not on screen asks for nothing.
 *
 * "While somebody is looking" includes the first paint: `useVisibility` starts true on purpose
 * (a window that assumed it was hidden would come up empty), and every change after that is the
 * shell's own report. Both the read and the timer hang off the same flag, so a window that is put
 * away does neither — including not re-reading once merely because it was hidden.
 *
 * There is deliberately no optimistic state and no merging of two answers: a statistic that is
 * half from one reading and half from another is a statistic about nothing. The previous numbers
 * stay on screen until the new ones arrive, and the timestamp beside them says which reading they
 * came from.
 */

import { useCallback, useEffect, useState } from "react";
import { ipc, type StatsOverview } from "../ipc";
import { messageOf } from "../format";

/** How often the numbers are re-read while the screen is visible. */
const REFRESH_MS = 60_000;

export type OverviewState = {
  overview: StatsOverview | null;
  loading: boolean;
  error: string | null;
  reload: () => void;
};

export function useOverview(visible: boolean, windowSecs?: number): OverviewState {
  const [overview, setOverview] = useState<StatsOverview | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [generation, setGeneration] = useState(0);

  const reload = useCallback(() => setGeneration((g) => g + 1), []);

  useEffect(() => {
    if (!visible) return;
    let current = true;
    setLoading(true);

    ipc
      .statsOverview(windowSecs)
      .then((result) => {
        if (!current) return;
        setOverview(result);
        setError(null);
        setLoading(false);
      })
      .catch((e: unknown) => {
        if (!current) return;
        // A daemon that cannot answer is a fact worth showing, not a blank screen — and the
        // numbers from the last answer stay up beside it, because they were true once.
        setError(messageOf(e));
        setLoading(false);
      });

    return () => {
      current = false;
    };
  }, [visible, windowSecs, generation]);

  useEffect(() => {
    if (!visible) return;
    const timer = window.setInterval(reload, REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [visible, reload]);

  return { overview, loading, error, reload };
}
