/**
 * One keystroke, one search, and the newest answer wins.
 *
 * There is no debounce, and that is a decision rather than an oversight. The budget is 80 ms
 * from keystroke to first visible row (M4 §2), and a 150 ms debounce spends twice that budget
 * doing nothing. The daemon answers a hybrid query in about twenty milliseconds over a hundred
 * thousand files, so the right thing to do with a keystroke is to ask.
 *
 * What is needed instead is **staleness handling**: answers can arrive out of order, and an
 * older one landing last would show results for a query the person has already finished typing
 * past. Every search carries a sequence number and anything older than what is already on
 * screen is dropped.
 *
 * The reranked order (D7) arrives later still, as an event, and is applied only if it belongs
 * to the query currently on screen.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { ipc, type SearchHit, type SearchResult, type SearchUpdate } from "../ipc";

export type SearchState = {
  query: string;
  hits: SearchHit[];
  mode: string;
  elapsedMs: number;
  /** True while a better order is expected. */
  refining: boolean;
  /** True when a request is in flight and there is nothing on screen yet. */
  loading: boolean;
  error: string | null;
};

const EMPTY: SearchState = {
  query: "",
  hits: [],
  mode: "auto",
  elapsedMs: 0,
  refining: false,
  loading: false,
  error: null,
};

export function useSearch(limit = 200) {
  const [state, setState] = useState<SearchState>(EMPTY);

  // Two counters rather than one: `issued` numbers the requests, `applied` records which one is
  // on screen. Comparing them is what makes a late answer droppable without a timer.
  const issued = useRef(0);
  const applied = useRef(0);
  const currentQuery = useRef("");

  const run = useCallback(
    (query: string) => {
      currentQuery.current = query;

      if (query.trim() === "") {
        issued.current += 1;
        applied.current = issued.current;
        setState(EMPTY);
        return;
      }

      const seq = ++issued.current;
      setState((prev) => ({ ...prev, query, loading: prev.hits.length === 0, error: null }));

      ipc
        .search(query, limit)
        .then((result: SearchResult) => {
          // An answer to a query the person has typed past is not wrong, it is late.
          if (seq < applied.current) return;
          applied.current = seq;
          setState({
            query,
            hits: result.hits,
            mode: result.mode,
            elapsedMs: result.elapsed_ms,
            refining: result.partial,
            loading: false,
            error: null,
          });
        })
        .catch((error: unknown) => {
          if (seq < applied.current) return;
          applied.current = seq;
          setState({
            ...EMPTY,
            query,
            error: messageOf(error),
          });
        });
    },
    [limit],
  );

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    ipc
      .onSearchUpdate((update: SearchUpdate) => {
        // The improved order belongs to a specific query. Applying it to a later one would
        // reorder results the person is already reading, which is worse than not reranking.
        setState((prev) => {
          if (!prev.refining || prev.query !== currentQuery.current) return prev;
          return { ...prev, hits: update.hits, refining: false };
        });
      })
      .then((off) => {
        if (disposed) off();
        else unlisten = off;
      })
      .catch(() => {
        // No event channel means no reranking, which is a degradation and not a failure: the
        // first answer was already complete.
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  return { state, run };
}

function messageOf(error: unknown): string {
  if (typeof error === "string") return error;
  if (error && typeof error === "object" && "message" in error) {
    return String((error as { message: unknown }).message);
  }
  return "the daemon did not answer";
}
