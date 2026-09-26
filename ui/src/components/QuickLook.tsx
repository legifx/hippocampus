/**
 * Quick Look: the selected file, as large as the window allows.
 *
 * The preview pane answers "is this the one" at 512 px, which is enough for a photo and not
 * enough for a screenshot with a typo in it. Space (with an empty search field) or Ctrl+Space
 * opens the same file over the whole window, rendered at the screen's own pixel count up to the
 * thumbnailer's cap (D14: 2048 px). Space, Escape or a click on the backdrop closes it.
 *
 * Two zoom states and no more: *fit* (the whole picture visible) and *1:1* (one image pixel per
 * device pixel, scrollable). A slider would be a third thing to learn; "is it sharp at 100 %"
 * and "what is on it" are the two questions this exists to answer. Toggle with a click on the
 * picture or with `1` / `0`.
 *
 * Text files are shown with eight times the pane's budget. It is still a preview: the deny-list
 * and the audit apply, because the bytes come through `fs.read` like everywhere else.
 */

import { useEffect, useState } from "react";
import type { SearchHit } from "../ipc";
import { formatAge, formatSize, splitPath } from "../format";
import { GlassButton } from "./glass";
import { QUICK_LOOK_BYTES, QUICK_LOOK_MAX_PIXELS, usePreview } from "../hooks/usePreview";
import { PreviewBody } from "./PreviewPane";

export function QuickLook({ hit, onClose }: { hit: SearchHit; onClose: () => void }) {
  const [actualSize, setActualSize] = useState(false);
  // The long edge of the window in CSS pixels, once per mount. A resize while it is open does
  // not re-request: the render already covers the screen, and asking again would only spend
  // the thumbnailer's time on a picture that is already sharp.
  const [pixels] = useState(() =>
    typeof window === "undefined"
      ? QUICK_LOOK_MAX_PIXELS
      : Math.min(QUICK_LOOK_MAX_PIXELS, Math.max(window.innerWidth, window.innerHeight)),
  );
  const state = usePreview(hit, { pixels, bytes: QUICK_LOOK_BYTES });

  // Zoom keys, here rather than in the app's handler: they mean nothing anywhere else.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "1" && event.key !== "0") return;
      // Capture phase, and the default prevented: the search field has the focus the whole
      // time, and without this the digit would be typed into it.
      event.preventDefault();
      setActualSize(event.key === "1");
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  const { name, dir } = splitPath(hit.path);
  const isPicture = state.kind === "picture";

  return (
    <div
      className="hc-quicklook"
      role="dialog"
      aria-label="Schnellansicht"
      data-zoom={actualSize ? "actual" : "fit"}
      onMouseDown={(event) => {
        // The backdrop closes; the picture toggles. Both on mousedown so the input keeps focus
        // and the next keystroke still goes to the search field.
        event.preventDefault();
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <header className="hc-quicklook-head">
        <span className="hc-title">{name}</span>
        <span className="hc-meta hc-quicklook-dir">{dir}</span>
        <span className="hc-meta">
          <span className="hc-num">{hit.kind === "dir" ? "" : formatSize(hit.size)}</span>
          <span className="hc-num">{formatAge(hit.mtime)}</span>
          {isPicture && state.kind === "picture" && state.width > 0 && (
            <span className="hc-num">
              {state.width} × {state.height}
            </span>
          )}
          {isPicture && (
            /*
             * The kit's button, not a chip of this window's own drawing. This was the last
             * control in the application that owned its fill — `--hc-surface-hi`, a border and a
             * pill radius written out by hand — while `.gb` sat in `kit.css` doing the same job
             * with the kit's hover, its press and its disabled state. `onMouseDown` still
             * prevents the default so the search field keeps the focus (the zoom keys are
             * handled on the window anyway).
             */
            <GlassButton
              size="sm"
              variant="glass"
              shape="pill"
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => setActualSize((z) => !z)}
            >
              {actualSize ? "einpassen (0)" : "1:1 (1)"}
            </GlassButton>
          )}
        </span>
      </header>
      <div
        className="hc-quicklook-body"
        onMouseDown={(event) => {
          event.preventDefault();
          if (isPicture) setActualSize((z) => !z);
        }}
      >
        <PreviewBody state={state} />
      </div>
    </div>
  );
}
