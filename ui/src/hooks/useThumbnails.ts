/**
 * A picture in the row, for the rows that can have one.
 *
 * M6 asks for thumbnails in the list and for the XDG cache to be shared rather than reinvented,
 * and the second half is what shapes this: a thumbnail Nautilus has already rendered is one
 * this window reads off the disk, and one this window renders is one Nautilus does not have to.
 *
 * **Two questions, not one.** *Which of these rows already have a picture* is a stat per path
 * and is asked for a whole viewport at once. *Render this one* costs a decode and is asked one
 * row at a time, for rows that are still on screen, after the scrolling has stopped. Collapsing
 * them into a single call would make a list of two hundred images start two hundred decodes in
 * a process that answers one at a time — which is not slowness, it is a queue, and the answer
 * for the row at the bottom would arrive after the person had scrolled past it.
 *
 * **Nothing is asked twice.** Every path ends up in one of three states — a URL, `null` for "it
 * will never have one", or queued — and a path that has a state is never asked about again.
 * Without that, every scroll back over the same rows would re-ask for the files that have no
 * thumbnail, which is most of a source tree.
 */

import { useEffect, useRef, useState } from "react";
import { ipc } from "../ipc";
import type { Row } from "../row";

/** The XDG "normal" bucket. A row is twenty pixels tall; this is already generous. */
export const ROW_THUMBNAIL_PIXELS = 128;

/**
 * How long the viewport has to hold still before anything is asked for.
 *
 * The same reasoning as the preview pane's settle delay, for the same reason: a list flung
 * through a thousand rows passes every one of them, and asking about each would be a thousand
 * questions about rows nobody saw.
 */
export const THUMBNAIL_SETTLE_MS = 140;

type Known = Map<string, string | null>;

/**
 * @param visible the rows on screen right now, in order
 * @param enabled false while nobody can see the window — a hidden overlay must cost nothing
 */
export function useThumbnails(visible: Row[], enabled = true): Known {
  const known = useRef<Known>(new Map());
  const queued = useRef<Set<string>>(new Set());
  const onScreen = useRef<Set<string>>(new Set());
  const draining = useRef(false);
  // A counter rather than a new Map per change: the map is read by two hundred rows and copying
  // it on every arrival would be the expensive half of showing a picture.
  const [, bump] = useState(0);

  // Written as loops rather than as `filter`/`find`, and not for speed: the architecture test
  // forbids those operations in this frontend on purpose (D4), so that a file which starts
  // reshaping what the daemon sent has to argue for an exception. Nothing here reshapes a list —
  // these are the paths to ask a question about — and the way to keep a blunt rule useful is to
  // not carve a hole in it for a case that does not need one.
  const paths: string[] = [];
  for (const row of visible) {
    if (!row.isDir) paths.push(row.path);
  }
  onScreen.current = new Set(paths);
  const key = paths.join(" ");

  useEffect(() => {
    if (!enabled || paths.length === 0) return;
    let disposed = false;

    const timer = setTimeout(() => {
      const unknown: string[] = [];
      for (const path of paths) {
        if (!known.current.has(path) && !queued.current.has(path)) unknown.push(path);
      }
      if (unknown.length === 0) {
        void drain();
        return;
      }

      const dpr = typeof window === "undefined" ? 1 : window.devicePixelRatio || 1;
      const pixels = Math.round(ROW_THUMBNAIL_PIXELS * dpr);

      ipc
        .thumbnails(unknown, pixels)
        .then((answer) => {
          if (disposed) return;
          for (const hit of answer.found) known.current.set(hit.path, ipc.assetUrl(hit.png));
          // "Skipped" is the daemon saying *stop asking*: on the deny-list, gone, or already
          // found un-thumbnailable and unchanged since. Recording it as `null` is what keeps a
          // directory of source files from being re-examined on every scroll.
          for (const path of answer.skipped) known.current.set(path, null);
          for (const miss of answer.missing) queued.current.add(miss.path);
          bump((n) => n + 1);
          void drain();
        })
        .catch(() => {
          // The thumbnailer is not there, or it died. The list is a list of names either way,
          // and a row with no picture is the state it was already in.
        });
    }, THUMBNAIL_SETTLE_MS);

    /**
     * Render the queued ones, one at a time, skipping whatever has scrolled away.
     *
     * One at a time because the thumbnailer is one process and the preview pane shares it: two
     * in flight would not be twice as fast, and the pane — which is showing the file somebody
     * actually selected — would wait behind both.
     */
    async function drain() {
      if (draining.current) return;
      draining.current = true;
      try {
        for (;;) {
          let next: string | undefined;
          for (const path of queued.current) {
            if (onScreen.current.has(path)) {
              next = path;
              break;
            }
          }
          if (next === undefined || disposed) break;
          queued.current.delete(next);

          const dpr = typeof window === "undefined" ? 1 : window.devicePixelRatio || 1;
          try {
            const preview = await ipc.preview(next, Math.round(ROW_THUMBNAIL_PIXELS * dpr));
            known.current.set(next, ipc.assetUrl(preview.png));
          } catch {
            // It cannot be rendered on this machine — a missing PDF tool, a broken file. The
            // previewer remembers the ones that are the file's own fault; this remembers the
            // rest for as long as the window is open, which is the right lifetime for "the
            // tool is not installed".
            known.current.set(next, null);
          }
          bump((n) => n + 1);
        }
      } finally {
        draining.current = false;
      }
    }

    return () => {
      disposed = true;
      clearTimeout(timer);
    };
    // `paths` is rebuilt every render; `key` is what actually changed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, enabled]);

  return known.current;
}
