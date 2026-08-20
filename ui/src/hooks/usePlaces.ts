/**
 * The places, and where the cursor is in them.
 *
 * Fetched once and again whenever something might have changed them — a drive plugged in, a
 * bookmark added elsewhere. Not polled: the list is read when the window is summoned, which for
 * a resident overlay is exactly as often as anybody looks at it.
 */

import { useCallback, useEffect, useState } from "react";
import { ipc, type PlaceSection } from "../ipc";

/**
 * The path that means "the places screen".
 *
 * A sentinel rather than a real path, because it is not one: there is no directory whose
 * contents are "your Downloads and the drives". Pretending it were `/` would mean the
 * breadcrumb, the history and every `fs.list` call had to know about a special case.
 */
export const PLACES = "places:";

export function usePlaces(active: boolean) {
  const [sections, setSections] = useState<PlaceSection[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [cursor, setCursor] = useState(0);
  const [generation, setGeneration] = useState(0);

  useEffect(() => {
    if (!active) return;
    let current = true;
    setLoading(true);
    setError(null);

    ipc
      .places()
      .then((result) => {
        if (!current) return;
        setSections(result.sections);
        setLoading(false);
      })
      .catch((e: unknown) => {
        if (!current) return;
        // A daemon that cannot answer is a fact worth showing, not a blank screen.
        setError(e instanceof Error ? e.message : String(e));
        setLoading(false);
      });

    return () => {
      current = false;
    };
  }, [active, generation]);

  /** Read the list again — a drive was plugged in, or the window was summoned. */
  const reload = useCallback(() => setGeneration((g) => g + 1), []);

  /**
   * Move the cursor, clamped.
   *
   * `count` is passed in rather than derived here: this hook holds the position and the view
   * holds the flattening, and a second copy of the flattening would be a second answer to
   * "how many places are there".
   */
  const move = useCallback((delta: number, count: number) => {
    setCursor((at) => {
      if (count === 0) return 0;
      return Math.min(count - 1, Math.max(0, at + delta));
    });
  }, []);

  const select = useCallback((index: number) => setCursor(Math.max(0, index)), []);

  return { sections, loading, error, cursor, move, select, reload };
}
