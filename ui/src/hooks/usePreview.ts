/**
 * One preview, as state: what the pane, the side-by-side compare and the Quick Look all show.
 *
 * Extracted from `PreviewPane` on 2026-09-06 when two more places needed exactly its behaviour
 * — two files next to each other, and one file filling the window. Three copies of the
 * "text through `fs.read`, pixels through the thumbnailer" rule would be three places for it
 * to drift, and the rule is a security decision (see `PreviewPane`), not a rendering detail.
 *
 * The picture path is progressive: the smaller cached thumbnail arrives as an event and is
 * shown at once, and the full render replaces it when it is ready. Never the other way round.
 */

import { useEffect, useRef, useState } from "react";
import { ipc, type Preview, type SearchHit } from "../ipc";
import { messageOf, splitPath } from "../format";

/** How much of a text file the pane asks for. A preview is a first impression, not a reader. */
export const PREVIEW_BYTES = 32 * 1024;

/** The long edge the pane asks the thumbnailer for, before the device pixel ratio. */
export const PREVIEW_PIXELS = 512;

/**
 * The long edge Quick Look asks for. The thumbnailer caps its render at 2048 (D14), and above
 * the screen's own pixel count nothing is sharper — so this is min(screen, cap), per call.
 */
export const QUICK_LOOK_MAX_PIXELS = 2048;

/** How much of a text file Quick Look shows: enough to read, still not the whole disk. */
export const QUICK_LOOK_BYTES = 256 * 1024;

const TEXTUAL = new Set([
  "md", "txt", "rs", "ts", "tsx", "js", "jsx", "py", "go", "c", "h", "cpp", "hpp", "java", "kt",
  "rb", "sh", "bash", "zsh", "fish", "toml", "yaml", "yml", "json", "xml", "html", "css", "scss",
  "sql", "csv", "tsv", "ini", "conf", "cfg", "log", "nix", "kdl", "lua", "vim", "el",
]);

const PICTORIAL = new Set([
  "png", "jpg", "jpeg", "webp", "gif", "avif", "bmp", "tif", "tiff",
  "pdf", "mp4", "mkv", "webm", "mov", "avi", "m4v",
]);

export type PreviewState =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "text"; content: string; truncated: boolean }
  | { kind: "picture"; url: string; width: number; height: number; sharp: boolean }
  | { kind: "binary" }
  | { kind: "error"; message: string };

export function extensionOf(path: string): string {
  const name = splitPath(path).name;
  const dot = name.lastIndexOf(".");
  return dot <= 0 ? "" : name.slice(dot + 1).toLowerCase();
}

export function isPictorial(path: string): boolean {
  return PICTORIAL.has(extensionOf(path));
}

export function isTextual(path: string): boolean {
  return TEXTUAL.has(extensionOf(path));
}

export type PreviewOptions = {
  /** Long edge in CSS pixels; multiplied by the device pixel ratio before asking. */
  pixels?: number;
  /** Bytes of a text file to fetch. */
  bytes?: number;
};

/**
 * The preview for one hit. `null` clears it.
 *
 * Every instance listens for the partial event and keeps only the one for its own path, so
 * two instances side by side do not swap pictures — which is the failure the compare view
 * would otherwise have had on its first day.
 */
export function usePreview(hit: SearchHit | null, options: PreviewOptions = {}): PreviewState {
  const pixels = options.pixels ?? PREVIEW_PIXELS;
  const bytes = options.bytes ?? PREVIEW_BYTES;
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
    let current = true;
    setState({ kind: "loading" });

    const dpr = typeof window === "undefined" ? 1 : window.devicePixelRatio || 1;

    const request = isPictorial(hit.path)
      ? ipc
          .preview(hit.path, Math.round(pixels * dpr))
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
      : isTextual(hit.path)
        ? ipc.readText(hit.path, bytes).then((result) => {
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
  }, [hit?.path, hit?.kind, pixels, bytes]);

  return state;
}
