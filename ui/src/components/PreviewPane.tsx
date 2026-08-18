/**
 * The preview pane.
 *
 * Text and code are shown here, read through the daemon's `fs.read` so that the deny-list still
 * applies and the read is audited — a GUI that opened files itself would be a way to read what
 * the rest of the system refuses to.
 *
 * Images, PDFs and video posters are **not** rendered here. D14 puts them in a separate
 * thumbnailer process: preview generation touches untrusted file content with third-party
 * decoders, which is the most likely place for a crash, and isolating it means a malformed
 * image kills a helper rather than the daemon. Until that process exists this pane says what it
 * cannot show rather than showing an empty box.
 */

import { useEffect, useState } from "react";
import { ipc, type SearchHit } from "../ipc";
import { formatAge, formatSize, splitPath } from "../format";

/** How much of a file the pane asks for. A preview is a first impression, not a reader. */
const PREVIEW_BYTES = 32 * 1024;

const TEXTUAL = new Set([
  "md", "txt", "rs", "ts", "tsx", "js", "jsx", "py", "go", "c", "h", "cpp", "hpp", "java", "kt",
  "rb", "sh", "bash", "zsh", "fish", "toml", "yaml", "yml", "json", "xml", "html", "css", "scss",
  "sql", "csv", "tsv", "ini", "conf", "cfg", "log", "nix", "kdl", "lua", "vim", "el",
]);

const RENDERABLE_LATER = new Set([
  "png", "jpg", "jpeg", "webp", "gif", "avif", "svg", "bmp", "tiff", "pdf", "mp4", "mkv", "webm",
  "mov", "mp3", "flac", "wav", "ogg",
]);

type PreviewState =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "text"; content: string; truncated: boolean }
  | { kind: "deferred"; extension: string }
  | { kind: "binary" }
  | { kind: "error"; message: string };

export function PreviewPane({ hit }: { hit: SearchHit | null }) {
  const [state, setState] = useState<PreviewState>({ kind: "idle" });

  useEffect(() => {
    if (!hit) {
      setState({ kind: "idle" });
      return;
    }
    if (hit.kind === "dir") {
      setState({ kind: "binary" });
      return;
    }

    const extension = extensionOf(hit.path);
    if (RENDERABLE_LATER.has(extension)) {
      setState({ kind: "deferred", extension });
      return;
    }
    if (!TEXTUAL.has(extension)) {
      setState({ kind: "binary" });
      return;
    }

    // The selection can move faster than a read returns; a late answer for a row nobody is on
    // any more is dropped rather than painted over the current one.
    let current = true;
    setState({ kind: "loading" });
    ipc
      .readText(hit.path, PREVIEW_BYTES)
      .then((result) => {
        if (!current) return;
        setState({
          kind: "text",
          content: result.content,
          truncated: result.truncated || result.bytes < result.total_bytes,
        });
      })
      .catch((e: unknown) => {
        if (!current) return;
        setState({ kind: "error", message: e instanceof Error ? e.message : String(e) });
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
    case "deferred":
      return (
        <p className="hc-faint">
          Vorschau für <code>.{state.extension}</code> rendert der separate Thumbnailer-Prozess
          (D14) — noch nicht gebaut.
        </p>
      );
    case "binary":
      return <p className="hc-faint">Kein Textinhalt</p>;
    case "error":
      return <p className="hc-error">{state.message}</p>;
  }
}

function extensionOf(path: string): string {
  const name = splitPath(path).name;
  const dot = name.lastIndexOf(".");
  return dot <= 0 ? "" : name.slice(dot + 1).toLowerCase();
}
