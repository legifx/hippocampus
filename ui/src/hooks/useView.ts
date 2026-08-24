/**
 * Where you are, what is in it, and how you got there — for every kind of place.
 *
 * This replaces the two hooks it grew out of. `useBrowse` held a directory and `useSearch` held
 * results; each fetched its own shape and the component above mapped them into a third one with
 * a branch on which mode it was in. The daemon now answers `view.open` in one shape whatever the
 * source was (M10 step 1), so there is one hook, one mapper, and one row stream.
 *
 * **The overlay is still an overlay.** M6 asks for search that replaces the list live and gives
 * the directory back when the query is cleared — instantly, without a round trip. So two views
 * are held at once: the *place*, which is spatial and has a history, and the *overlay*, which is
 * whatever is typed. Which of them is showing is a question about the window's state, not about
 * what a row is: nothing downstream of `rows` knows the difference.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { ipc, type Order, type Source, type View, type ViewUpdate } from "../ipc";
import { rowFromView, type Row } from "../row";
import { PLACES } from "./usePlaces";

export type SortChoice = Order;

export type ViewState = {
  /** What is showing: the overlay if there is one, the place otherwise. */
  source: Source;
  /** The directory the window is *in*, whatever is showing over it. */
  path: string;
  /** True when a query is showing. A fact about the window, never about a row. */
  searching: boolean;
  basis: string;
  order: SortChoice;
  reverse: boolean;
  /** Entries the deny-list covers. Counted, never named (R1). */
  hidden: number;
  truncated: boolean;
  loading: boolean;
  error: string | null;
  /** True while a better order is expected. */
  refining: boolean;
  elapsedMs: number;
  canGoBack: boolean;
};

const NOTHING = { rows: [] as Row[], basis: "", hidden: 0, truncated: false, refining: false, elapsedMs: 0 };

type Held = typeof NOTHING;

function heldFrom(view: View, commonDir: string | null): Held {
  return {
    rows: view.rows.map((row) => rowFromView(row, commonDir)),
    basis: view.basis,
    hidden: view.hidden,
    truncated: view.truncated,
    refining: view.refining,
    elapsedMs: view.elapsed_ms,
  };
}

export function useView(startPath: string) {
  const [path, setPath] = useState(startPath);
  const [history, setHistory] = useState<string[]>([]);
  const [generation, setGeneration] = useState(0);
  const [order, setOrder] = useState<SortChoice>("name");
  const [reverse, setReverse] = useState(false);
  const [query, setQuery] = useState("");

  const [place, setPlace] = useState<Held>(NOTHING);
  const [overlay, setOverlay] = useState<Held>(NOTHING);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Two counters rather than one: `issued` numbers the requests, `applied` records which one is
  // on screen. Comparing them is what makes a late answer droppable without a timer — an older
  // query landing last would otherwise show results for something already typed past.
  const issued = useRef(0);
  const applied = useRef(0);
  const currentQuery = useRef("");

  const searching = query.trim() !== "";

  // The place. Not fetched for the places screen, which is not a directory: asking the daemon
  // to list `places:` would be asking a question with no answer.
  useEffect(() => {
    if (path === PLACES) {
      setPlace(NOTHING);
      setLoading(false);
      setError(null);
      return;
    }

    let current = true;
    setLoading(true);
    setError(null);

    ipc
      .viewOpen({ kind: "dir", path }, { order, reverse })
      .then((view) => {
        if (!current) return;
        // The directory every row would otherwise repeat — so a listing shows bare names.
        setPlace(heldFrom(view, path.endsWith("/") ? path : `${path}/`));
        setLoading(false);
      })
      .catch((e: unknown) => {
        if (!current) return;
        // A place that cannot be read is a fact about the place, not a broken window. Saying
        // which one and why is the difference between an error and a dead end.
        setPlace(NOTHING);
        setError(e instanceof Error ? e.message : String(e));
        setLoading(false);
      });

    return () => {
      current = false;
    };
  }, [path, order, reverse, generation]);

  // The overlay. No debounce, deliberately: the budget is 80 ms from keystroke to first row and
  // a 150 ms debounce spends twice that doing nothing (M4 §2).
  useEffect(() => {
    const text = query.trim();
    currentQuery.current = text;
    if (text === "") {
      setOverlay(NOTHING);
      return;
    }

    const sequence = ++issued.current;
    let current = true;
    setError(null);

    ipc
      .viewOpen({ kind: "query", text }, { limit: 200 })
      .then((view) => {
        if (!current || sequence < applied.current) return;
        applied.current = sequence;
        // Every row says where it lives: they come from everywhere.
        setOverlay(heldFrom(view, null));
      })
      .catch((e: unknown) => {
        if (!current) return;
        setOverlay(NOTHING);
        setError(e instanceof Error ? e.message : String(e));
      });

    return () => {
      current = false;
    };
  }, [query]);

  // The reranked order, when it arrives. Applied only to the query it belongs to: reordering a
  // list somebody is already reading is worse than not reranking at all.
  useEffect(() => {
    let disposed = false;
    let off: (() => void) | null = null;

    ipc
      .onViewUpdate((update: ViewUpdate) => {
        if (disposed) return;
        setOverlay((previous) => {
          if (!previous.refining) return previous;
          return {
            ...previous,
            rows: update.rows.map((row) => rowFromView(row, null)),
            refining: false,
          };
        });
      })
      .then((unlisten) => {
        if (disposed) unlisten();
        else off = unlisten;
      })
      .catch(() => {
        // No event channel means no reranking, which is a degradation rather than a failure:
        // the first answer was already complete.
      });

    return () => {
      disposed = true;
      off?.();
    };
  }, []);

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
   * The place above this one.
   *
   * Above `/` is the places screen — not a parent directory in any filesystem sense, and the
   * right answer to "up" all the same: it is where you came from and where everything else is
   * reachable from.
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
    (next: SortChoice) => {
      if (next === order) {
        setReverse((r) => !r);
      } else {
        setOrder(next);
        setReverse(false);
      }
    },
    [order],
  );

  const reload = useCallback(() => setGeneration((g) => g + 1), []);

  const showing = searching ? overlay : place;
  const state: ViewState = {
    source: searching ? { kind: "query", text: query.trim() } : { kind: "dir", path },
    path,
    searching,
    basis: showing.basis,
    order,
    reverse,
    hidden: showing.hidden,
    truncated: showing.truncated,
    loading,
    error,
    refining: showing.refining,
    elapsedMs: showing.elapsedMs,
    canGoBack: history.length > 0,
  };

  return {
    state,
    rows: showing.rows,
    query,
    setQuery,
    goTo,
    goBack,
    goUp,
    parentOf,
    orderBy,
    reload,
  };
}
