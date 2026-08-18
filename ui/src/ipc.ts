/**
 * The only way data enters this frontend.
 *
 * D4 attaches two hard conditions to using a web stack at all, and the second is *no data
 * processing in the frontend*: it renders what the daemon sends, and does not filter, sort or
 * search. That condition is only enforceable if there is one door, so this is it — every
 * component reads from here and nothing constructs a result of its own.
 *
 * The transport is injectable so that tests are real. A test that stubbed `@tauri-apps/api`
 * would be testing the stub; a test that swaps the transport exercises the same code the app
 * runs, with a daemon that answers from a fixture.
 */

import { convertFileSrc, invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";

export type Transport = {
  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  listen(event: string, handler: (payload: unknown) => void): Promise<() => void>;
  /** A local file the webview may load, as a URL it is allowed to load it from. */
  assetUrl(path: string): string;
};

/** True when running inside the Tauri shell rather than a bare browser or a test. */
export function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

const realTransport: Transport = {
  invoke: (command, args) => tauriInvoke(command, args),
  listen: async (event, handler) => {
    const unlisten = await tauriListen(event, (e) => handler(e.payload));
    return unlisten;
  },
  // The asset protocol is scoped to the thumbnail cache in `tauri.conf.json`: the webview can
  // display a rendered thumbnail and nothing else. It has no business reading the files those
  // thumbnails are of.
  assetUrl: (path) => convertFileSrc(path),
};

/** A transport that fails every call, for a browser tab opened by mistake. */
const absentTransport: Transport = {
  invoke: async (command) => {
    throw new Error(`${command} needs the Tauri shell; this is a plain browser window`);
  },
  listen: async () => () => {},
  assetUrl: (path) => path,
};

let transport: Transport = inTauri() ? realTransport : absentTransport;

export function setTransport(next: Transport): void {
  transport = next;
}

// ---------------------------------------------------------------------------
// The wire shapes. These mirror the daemon's types; they are not remodelled on the way in,
// because a frontend model that differs from the protocol is a place for the two to drift.
// ---------------------------------------------------------------------------

export type QueryMode = "auto" | "locate" | "recall" | "reference";

export type ScoreBreakdown = {
  /** Rank in each arm, when that arm found it at all. */
  path_rank: number | null;
  lexical_rank: number | null;
  vector_rank: number | null;
  /** The fused rank before salience, and the multiplier salience applied to it. */
  fused: number;
  salience_boost: number;
  /** The cross-encoder's score, once it has arrived (D7). */
  rerank: number | null;
  final_score: number;
};

export type SearchHit = {
  node_id: number;
  path: string;
  kind: string;
  size: number;
  mtime: number;
  zone_id: number | null;
  tier: string;
  score: ScoreBreakdown;
  snippet: string | null;
};

export type SearchResult = {
  hits: SearchHit[];
  mode: QueryMode;
  elapsed_ms: number;
  /** True when a better order is on its way as a `search:update` event (D7). */
  partial: boolean;
};

export type SearchUpdate = {
  request_id: unknown;
  hits: SearchHit[];
  elapsed_ms: number;
  model: string;
};

export type Zone = {
  id: number;
  name: string;
  root: string;
  hue: number;
  nodes: number;
};

export type Neighbor = {
  node_id: number;
  path: string | null;
  kind: string;
  edge: string;
  source: string;
  weight: number;
  depth: number;
  note: string | null;
};

export type Annotation = {
  node_id: number;
  target_id: number;
  tags: string[];
  note: string;
  actor: string;
  ts: number;
};

export type NeighborsResult = {
  target_id: number;
  neighbors: Neighbor[];
  annotations: Annotation[];
};

export type Episode = {
  id: number;
  ts: number;
  actor: { kind: string; name?: string } | string;
  action: string;
  node_id: number | null;
  metadata: Record<string, unknown>;
};

export type Status = {
  version: string;
  files: number;
  dirs: number;
  chunks: number;
  embeddings: number;
  episodes: number;
  agent_surface_enabled: boolean;
  indexed_paths: number;
};

export type ExtractStatus = {
  pending: number;
  done: number;
  failed: number;
  skipped: number;
};

export type ReadResult = {
  path: string;
  content: string;
  offset: number;
  bytes: number;
  total_bytes: number;
  truncated: boolean;
  note: string | null;
};

/** What the thumbnailer produced. The PNG lives in the shared XDG thumbnail cache. */
export type Preview = {
  path: string;
  png: string;
  width: number;
  height: number;
  kind: "image" | "pdf" | "video" | "audio" | "text" | "none";
  size: "normal" | "large" | "x-large" | "xx-large";
  cached: boolean;
  elapsed_ms: number;
};

/** The smaller thumbnail that arrives before the real one (D14). */
export type PreviewPartial = {
  path: string;
  png: string;
  size: Preview["size"];
};

export type StartupReport = {
  connected_ms: number;
  interactive_ms: number;
  budget_ms: number;
  within_budget: boolean;
};

// ---------------------------------------------------------------------------
// Calls.
// ---------------------------------------------------------------------------

export const ipc = {
  search: (query: string, limit?: number) =>
    transport.invoke<SearchResult>("search", { query, limit }),

  status: () => transport.invoke<Status>("status"),

  extractStatus: () => transport.invoke<ExtractStatus>("extract_status"),

  zones: () => transport.invoke<{ zones: Zone[] }>("zones"),

  neighbors: (target: string | number, depth = 1, edgeTypes?: string[]) =>
    transport.invoke<NeighborsResult>("neighbors", { target, depth, edgeTypes }),

  timeline: (limit = 20) =>
    transport.invoke<{ episodes: Episode[]; total: number }>("timeline", { limit }),

  nodeGet: (path: string) => transport.invoke<unknown>("node_get", { path }),

  readText: (path: string, bytes?: number) =>
    transport.invoke<ReadResult>("read_text", { path, bytes }),

  preview: (path: string, pixels?: number) =>
    transport.invoke<Preview>("preview", { path, pixels }),

  onPreviewPartial: (handler: (partial: PreviewPartial) => void) =>
    transport.listen("preview:partial", (payload) => handler(payload as PreviewPartial)),

  /** Turn a path in the thumbnail cache into something an `<img>` may load. */
  assetUrl: (path: string) => transport.assetUrl(path),

  explain: (query: string, path: string) =>
    transport.invoke<unknown>("explain", { query, path }),

  activityPing: () => transport.invoke<unknown>("activity_ping"),

  openPath: (path: string) => transport.invoke<void>("open_path", { path }),

  dismiss: () => transport.invoke<void>("dismiss"),

  /**
   * Stop the cold-start timer, and hand back what only the page knows.
   *
   * `pageMs` is time since the webview started navigating and `domReadyMs` is when the document
   * was parsed. Together with the process-side number they say which of the three parts —
   * process startup, webview startup, or this bundle — is spending the budget.
   */
  markInteractive: () => {
    const timing =
      typeof performance === "undefined"
        ? { pageMs: undefined, domReadyMs: undefined }
        : {
            pageMs: Math.round(performance.now()),
            domReadyMs: Math.round(
              (performance.getEntriesByType("navigation")[0] as PerformanceNavigationTiming)
                ?.domContentLoadedEventEnd ?? 0,
            ),
          };
    return transport.invoke<number>("mark_interactive", timing);
  },

  startupReport: () => transport.invoke<StartupReport>("startup_report"),

  /** The reranked order, arriving after the answer it improves (D7). */
  onSearchUpdate: (handler: (update: SearchUpdate) => void) =>
    transport.listen("search:update", (payload) => handler(payload as SearchUpdate)),

  onDaemonUnavailable: (handler: (message: string) => void) =>
    transport.listen("daemon:unavailable", (payload) => handler(String(payload))),
};
