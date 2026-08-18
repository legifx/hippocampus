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
 * The picture path is progressive: the smaller cached thumbnail arrives as an event and is
 * shown at once, and the full render replaces it when it is ready. Both are `<img>` elements
 * with the same dimensions, so the replacement does not move anything.
 */

import { useEffect, useRef, useState } from "react";
import { ipc, type Preview, type SearchHit } from "../ipc";
import { formatAge, formatSize, splitPath } from "../format";

/** How much of a text file the pane asks for. A preview is a first impression, not a reader. */
const PREVIEW_BYTES = 32 * 1024;

/** The long edge the pane asks the thumbnailer for, before the device pixel ratio. */
const PREVIEW_PIXELS = 512;

const TEXTUAL = new Set([
  "md", "txt", "rs", "ts", "tsx", "js", "jsx", "py", "go", "c", "h", "cpp", "hpp", "java", "kt",
  "rb", "sh", "bash", "zsh", "fish", "toml", "yaml", "yml", "json", "xml", "html", "css", "scss",
  "sql", "csv", "tsv", "ini", "conf", "cfg", "log", "nix", "kdl", "lua", "vim", "el",
]);

const PICTORIAL = new Set([
  "png", "jpg", "jpeg", "webp", "gif", "avif", "bmp", "tif", "tiff",
  "pdf", "mp4", "mkv", "webm", "mov", "avi", "m4v",
]);

type PreviewState =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "text"; content: string; truncated: boolean }
  | { kind: "picture"; url: string; width: number; height: number; sharp: boolean }
  | { kind: "binary" }
  | { kind: "error"; message: string };

export function PreviewPane({ hit }: { hit: SearchHit | null }) {
  const [state, setState] = useState<PreviewState>({ kind: "idle" });

  // Which path the partial event belongs to. A selection can move faster than a render
  // finishes, and a thumbnail of the previous row appearing over the current one is worse than
  // no thumbnail at all.
  const wanted = useRef<string | null>(null);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    ipc
      .onPreviewPartial((partial) => {
        if (disposed || partial.path !== wanted.current) return;
        setState((previous) => {
          // Never replace a sharp picture with a blurry one: the events can arrive in either
          // order when a render finishes quickly.
          if (previous.kind === "picture" && previous.sharp) return previous;
          return {
            kind: "picture",
            url: ipc.assetUrl(partial.png),
            width: 0,
            height: 0,
            sharp: false,
          };
        });
      })
      .then((off) => (disposed ? off() : (unlisten = off)))
      .catch(() => {});

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!hit) {
      wanted.current = null;
      setState({ kind: "idle" });
      return;
    }
    if (hit.kind === "dir") {
      wanted.current = null;
      setState({ kind: "binary" });
      return;
    }

    wanted.current = hit.path;
    const extension = extensionOf(hit.path);
    let current = true;
    setState({ kind: "loading" });

    const dpr = typeof window === "undefined" ? 1 : window.devicePixelRatio || 1;

    const request = PICTORIAL.has(extension)
      ? ipc
          .preview(hit.path, Math.round(PREVIEW_PIXELS * dpr))
          .then((preview: Preview) => {
            if (!current) return;
            setState({
              kind: "picture",
              url: ipc.assetUrl(preview.png),
              width: preview.width,
              height: preview.height,
              sharp: true,
            });
          })
      : TEXTUAL.has(extension)
        ? ipc.readText(hit.path, PREVIEW_BYTES).then((result) => {
            if (!current) return;
            setState({
              kind: "text",
              content: result.content,
              truncated: result.truncated || result.bytes < result.total_bytes,
            });
          })
        : Promise.resolve().then(() => current && setState({ kind: "binary" }));

    request.catch((e: unknown) => {
      if (!current) return;
      setState({ kind: "error", message: messageOf(e) });
    });

    return () => {
      current = false;
    };
  }, [hit?.path, hit?.kind]);

  if (!hit) {
    return (
      <section className="hc-preview" aria-label="Vorschau">
        <p className="hc-faint">Nichts ausgewählt</p>
      </section>
    );
  }

  const { name } = splitPath(hit.path);

  return (
    <section className="hc-preview" aria-label="Vorschau">
      <header className="hc-preview-head">
        <span className="hc-title">{name}</span>
        <span className="hc-meta">
          <span className="hc-num">{formatSize(hit.size)}</span>
          <span className="hc-num">{formatAge(hit.mtime)}</span>
        </span>
      </header>
      <div className="hc-preview-body">{renderBody(state)}</div>
    </section>
  );
}

function renderBody(state: PreviewState) {
  switch (state.kind) {
    case "idle":
      return <p className="hc-faint">Nichts ausgewählt</p>;
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

function extensionOf(path: string): string {
  const name = splitPath(path).name;
  const dot = name.lastIndexOf(".");
  return dot <= 0 ? "" : name.slice(dot + 1).toLowerCase();
}

function messageOf(error: unknown): string {
  if (typeof error === "string") return error;
  if (error && typeof error === "object" && "message" in error) {
    return String((error as { message: unknown }).message);
  }
  return "die Vorschau ist fehlgeschlagen";
}
