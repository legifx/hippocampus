/**
 * The preview pane.
 *
 * Two paths, and which one a file takes is a security decision rather than a rendering one:
 *
 * - **Text and code** are read through the daemon's `fs.read`, so the deny-list applies and the
 *   read is audited. A pane that opened files itself would be a way to read what the rest of
 *   the system refuses to.
 * - **Everything with pixels** — images, PDFs, video posters — goes to the thumbnailer, a
 *   separate process (D14). Those decoders read untrusted bytes, and a malformed image killing
 *   a helper is an inconvenience where a malformed image killing this window is a crash.
 *
 * The loading lives in `usePreview`, shared with the compare view and Quick Look; this file is
 * the pane's shape. The picture path is progressive: the smaller cached thumbnail arrives as an
 * event and is shown at once, and the full render replaces it when it is ready. Both are
 * `<img>` elements with the same dimensions, so the replacement does not move anything.
 */

import type { SearchHit } from "../ipc";
import { formatAge, formatSize, splitPath } from "../format";
import { usePreview, type PreviewState } from "../hooks/usePreview";

export function PreviewPane({ hit }: { hit: SearchHit | null }) {
  const state = usePreview(hit);

  if (!hit) {
    return (
      <section className="hc-preview glass-flat" aria-label="Vorschau">
        <p className="hc-faint hc-empty-note">Nichts ausgewählt</p>
      </section>
    );
  }

  const { name } = splitPath(hit.path);

  return (
    <section className="hc-preview glass-flat" aria-label="Vorschau">
      <header className="hc-preview-head">
        <span className="hc-title">{name}</span>
        <span className="hc-meta">
          <span className="hc-num">{formatSize(hit.size)}</span>
          <span className="hc-num">{formatAge(hit.mtime)}</span>
        </span>
      </header>
      <div className="hc-preview-body">
        <PreviewBody state={state} />
      </div>
    </section>
  );
}

/** The body of a preview, from its state. Shared by the pane, the compare view and Quick Look. */
export function PreviewBody({ state }: { state: PreviewState }) {
  switch (state.kind) {
    case "idle":
      return <p className="hc-faint hc-empty-note">Nichts ausgewählt</p>;
    case "loading":
      return <p className="hc-faint">…</p>;
    case "text":
      return (
        <>
          <pre className="hc-preview-text">{state.content}</pre>
          {state.truncated && <p className="hc-faint">… gekürzt</p>}
        </>
      );
    case "picture":
      return (
        <img
          className={state.sharp ? "hc-preview-image" : "hc-preview-image hc-preview-blurry"}
          src={state.url}
          alt=""
          // No width or height attribute while the picture is still the placeholder: the
          // cached thumbnail has different dimensions, and stating them would make the pane
          // jump when the full render lands.
          {...(state.sharp && state.width > 0
            ? { width: state.width, height: state.height }
            : {})}
        />
      );
    case "binary":
      return <p className="hc-faint">Keine Vorschau für diesen Dateityp</p>;
    case "error":
      return <p className="hc-error">{state.message}</p>;
  }
}
