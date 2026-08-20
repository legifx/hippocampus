/**
 * Where you are, and how you got there.
 *
 * The overlay has two modes and one list. With an empty query it shows a directory; with
 * something typed it shows results. That is the shape M6 asks for — "search as an overlay, not
 * a separate page, results replace the list live" — and it is why there is no separate search
 * screen to switch to.
 *
 * The directory itself comes from `fs.list`, ordered by the daemon. This hook keeps the path,
 * the history and the ordering *choice*; it never touches the entries.
 */

import { useCallback, useEffect, useState } from "react";
import { ipc, type FsEntry, type SortBy } from "../ipc";
import { PLACES } from "./usePlaces";

export type BrowseState = {
  path: string;
  entries: FsEntry[];
  /** Entries the deny-list covers. Counted, never named (R1). */
  hidden: number;
  truncated: boolean;
  loading: boolean;
  error: string | null;
  sort: SortBy;
  reverse: boolean;
  /** True when there is somewhere to go back to. */
  canGoBack: boolean;
};

export function useBrowse(initialPath: string) {
  const [path, setPath] = useState(initialPath);
  /// Bumped to read the directory again after something changed it.
  const [generation, setGeneration] = useState(0);
  const [history, setHistory] = useState<string[]>([]);
  const [sort, setSort] = useState<SortBy>("name");
  const [reverse, setReverse] = useState(false);
  const [entries, setEntries] = useState<FsEntry[]>([]);
  const [hidden, setHidden] = useState(0);
  const [truncated, setTruncated] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    // The places screen is not a directory, so there is nothing to list. Asking the daemon for
    // the contents of `places:` would be asking a question with no answer.
    if (path === PLACES) {
      setEntries([]);
      setHidden(0);
      setTruncated(false);
      setLoading(false);
      setError(null);
      return;
    }

    let current = true;
    setLoading(true);
    setError(null);

    ipc
      .list(path, { sort, reverse })
      .then((result) => {
        if (!current) return;
        setEntries(result.entries);
        setHidden(result.hidden);
        setTruncated(result.truncated);
        setLoading(false);
      })
      .catch((e: unknown) => {
        if (!current) return;
        // A directory that cannot be read is a fact about the directory, not a broken window.
        // Saying which one and why is the difference between an error and a dead end.
        setEntries([]);
        setError(e instanceof Error ? e.message : String(e));
        setLoading(false);
      });

    return () => {
      current = false;
    };
  }, [path, sort, reverse, generation]);

  /// Read this directory again. What a finished copy, move, delete or rename calls.
  const reload = useCallback(() => setGeneration((g) => g + 1), []);

  const goTo = useCallback(
    (next: string) => {
      setHistory((h) => [...h, path]);
      setPath(next);
    },
    [path],
  );

  const goBack = useCallback(() => {
    setHistory((h) => {
      const previous = h[h.length - 1];
      if (previous === undefined) return h;
      setPath(previous);
      return h.slice(0, -1);
    });
  }, []);

  /**
   * The directory above this one.
   *
   * Above `/` is the places screen — which is not a parent directory in any filesystem sense,
   * and is the right answer to "up" all the same: it is where you came from and where
   * everything else is reachable from.
   */
  const parentOf = useCallback((of: string): string | null => {
    if (of === PLACES) return null;
    if (of === "/" || of === "") return PLACES;
    const trimmed = of.endsWith("/") ? of.slice(0, -1) : of;
    const cut = trimmed.lastIndexOf("/");
    if (cut < 0) return null;
    return cut === 0 ? "/" : trimmed.slice(0, cut);
  }, []);

  const goUp = useCallback(() => {
    const parent = parentOf(path);
    if (parent) goTo(parent);
  }, [path, parentOf, goTo]);

  /** Clicking the same column again reverses it, which is what every file manager does. */
  const orderBy = useCallback(
    (next: SortBy) => {
      if (next === sort) {
        setReverse((r) => !r);
      } else {
        setSort(next);
        setReverse(false);
      }
    },
    [sort],
  );

  const state: BrowseState = {
    path,
    entries,
    hidden,
    truncated,
    loading,
    error,
    sort,
    reverse,
    canGoBack: history.length > 0,
  };

  return { state, goTo, goBack, goUp, orderBy, parentOf, reload };
}
