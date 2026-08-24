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

// ---------------------------------------------------------------------------
// The Agent Inspector (M11 §8). Read-only, apart from the three writes that are the point of
// it: the kill switch, undo, and the human's right to delete what a machine believes.
// ---------------------------------------------------------------------------

export type MemoryKind =
  | "fact"
  | "preference"
  | "procedure"
  | "decision"
  | "failure"
  | "observation";

/** The daemon serialises a scope as `{ kind, id }`, with `id` absent for `global`. */
export type MemoryScope = { kind: "session" | "agent" | "project" | "global"; id?: string | number };

export type Memory = {
  node_id: number;
  scope: MemoryScope;
  kind: MemoryKind;
  text: string;
  subject_id?: number;
  subject_path?: string;
  confidence: number;
  /** Earned by outcome reports. Negative means it has cost more than it has given. */
  usefulness: number;
  helped_count: number;
  noise_count: number;
  retrieved_count: number;
  human_confirmed: boolean;
  expires_at?: number;
  superseded_by?: number;
  session_id?: string;
  actor: string;
  ts: number;
  salience: number;
  pinned: boolean;
};

export type MemoriesResult = {
  memories: Memory[];
  /** What this list is, and what it is not. Every view states its own basis. */
  basis: string;
};

export type SessionsResult = {
  sessions: AgentSession[];
  /** How many are still running. Counted by the daemon, like every other decision. */
  open: number;
  basis: string;
};

export type AgentSession = {
  id: string;
  name: string;
  task?: string;
  started_at: number;
  ended_at?: number;
  calls: number;
  refusals: number;
  memories: number;
};

export type AuditEntry = {
  id: number;
  ts: number;
  actor: string;
  tool: string;
  args_json: string;
  result_summary?: string;
  allowed: boolean;
};

export type AuditRecentResult = { entries: AuditEntry[] };

export type BrainEpisode = {
  id: number;
  ts: number;
  actor: string;
  action: string;
  node_id: number | null;
  session_id: string | null;
};

export type BrainDiff = {
  changes: BrainEpisode[];
  cursor: number;
  more: boolean;
};

export type AgentStatusResult = {
  enabled: boolean;
  indexing_paused: boolean;
  write_allowlist: string[];
  require_dry_run: boolean;
};

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

export type FsKind = "file" | "dir" | "symlink" | "other";

/** A user or a group: the number, and the name where the system could give one. */
export type Owner = {
  id: number;
  name?: string | null;
};

export type FsEntry = {
  path: string;
  /** What the entry *is*. A symlink is a symlink; where it leads is `target`. */
  kind: FsKind;
  size: number;
  mtime: number;
  /** Whether the brain knows this path — "not there" and "not indexed yet" look alike without it. */
  indexed: boolean;
  /** Permission bits as `stat` gives them, already formatted by the daemon. */
  mode?: number | null;
  owner?: Owner | null;
  group?: Owner | null;
  /** Where a symlink points, verbatim — relative if it was written relative. */
  target?: string | null;
  /** What the target turns out to be, so a link to a directory can be opened like one. */
  target_kind?: FsKind | null;
  /** A link whose target is not there. */
  broken?: boolean;
};

export type PlaceKind =
  | "home"
  | "desktop"
  | "documents"
  | "downloads"
  | "pictures"
  | "music"
  | "videos"
  | "trash"
  | "drive"
  | "filesystem"
  | "bookmark"
  | "network"
  /** A place the index made: a concept, the working set, a tier. `source` says how to open it. */
  | "view";

export type Capacity = {
  total_bytes: number;
  free_bytes: number;
};

/** Somewhere worth starting from. */
/**
 * What a drive is, beyond being somewhere to go. Only drives carry it.
 *
 * **`mounted: false` means the place has no directory yet** and its `path` is a device node.
 * Opening such a row means mounting it first and going to the answer — never listing the path.
 */
/**
 * The refusal a client can do something about: polkit would allow it after a password.
 *
 * A code rather than a message, so that recognising it is not a matter of reading English prose
 * that changes.
 */
export const NEEDS_AUTHORISATION = 1004;

export type VolumeInfo = {
  mounted: boolean;
  fstype: string;
  size_bytes: number;
  removable: boolean;
};

export type Place = {
  path: string;
  name: string;
  kind: PlaceKind;
  capacity?: Capacity | null;
  device?: string | null;
  /** Present exactly for a `view` place: how to open it. */
  source?: Source | null;
  /** Drives only. See {@link VolumeInfo} — an unmounted one's `path` is a device node. */
  volume?: VolumeInfo | null;
};

/**
 * One heading and what is under it.
 *
 * The grouping is the daemon's, like every other ordering decision (D4). A frontend that sorted
 * places into sections would be a second opinion about which of them is personal and which is
 * plumbing, and the two would drift.
 */
export type PlaceSection = {
  title: string;
  places: Place[];
};

export type FsPlacesResult = {
  sections: PlaceSection[];
};

/** The orders the daemon can put a listing in. The frontend never sorts (D4). */
export type SortBy = "name" | "size" | "modified" | "none";

export type FsListResult = {
  path: string;
  entries: FsEntry[];
  truncated: boolean;
  /** Entries the deny-list covers: counted, never named (R1). */
  hidden: number;
};

export type TransferMode = "copy" | "move";

/** Files on the system clipboard, whoever put them there. */
export type SystemClipboard = {
  mode: TransferMode;
  paths: string[];
};
export type ConflictChoice = "fail" | "replace" | "keep" | "skip";

/** One destination that already exists, with both sides of the trade. */
export type Conflict = {
  source: string;
  destination: string;
  existing: FsEntry;
  incoming: FsEntry;
};

/**
 * An installed application that can open a file, as the daemon read it from its desktop entry.
 *
 * Passed back to `openWith` unchanged. The window never builds a command line: what an `Exec`
 * line means is decided once, in Rust, and the shell that has a session spawns it (D58).
 */
export type Application = {
  id: string;
  name: string;
  comment?: string;
  exec: string;
  icon?: string;
  terminal: boolean;
  /** What a double click does. Exactly one of the list has it. */
  is_default: boolean;
  path: string;
};

export type AppsForPath = {
  path: string;
  mime: string;
  applications: Application[];
};

/** What `transfer` answers: a dry run's findings, or the handle of a started one. */
export type TransferAnswer =
  | { outcome: "planned"; total_files: number; total_bytes: number; conflicts: Conflict[] }
  | { outcome: "started"; operation: number; total_files: number; total_bytes: number };

// ---------------------------------------------------------------------------
// The view layer (M10). One row stream, several sources.
// ---------------------------------------------------------------------------

/** Where a view's rows come from. More sources arrive in step 3; the shape does not change. */
export type Source =
  | { kind: "dir"; path: string }
  | { kind: "query"; text: string }
  /** A named cluster the consolidation pass found. */
  | { kind: "concept"; id: number; label: string }
  /** What the human has touched lately. */
  | { kind: "working_set" }
  /** What is connected to one node, and how. */
  | { kind: "neighbours"; path: string; edges?: string[] }
  /** A slice of time, from the episode log. Seconds since the epoch. */
  | { kind: "timeline"; from: number; to: number }
  /** One tier, most salient first — the only place a tier is a subject, not a brightness. */
  | { kind: "tier"; tier: string };

/** What the daemon sorted by. The frontend never sorts (D4). */
export type Order = "name" | "size" | "modified" | "salience" | "none";

/**
 * One row, from any source.
 *
 * Every field only one source can fill is optional, and a renderer that draws an absent field as
 * nothing needs no branch on where the row came from. That is the whole of M10 step 1.
 */
export type ViewRow = {
  path: string;
  kind: FsKind;
  size: number;
  mtime: number;
  indexed: boolean;
  node_id?: number | null;
  zone_id?: number | null;
  tier?: string | null;
  snippet?: string | null;
  score?: ScoreBreakdown | null;
  /** What the brain thinks it is worth keeping. Absent for a path the index has never seen. */
  salience?: number | null;
  /** How many edges touch it, in either direction. */
  degree?: number | null;
  /** When something last happened to it. Absent means nothing ever has. */
  last_episode?: number | null;
  /** Held by a person against forgetting. */
  pinned?: boolean;
  mode?: number | null;
  owner?: Owner | null;
  group?: Owner | null;
  target?: string | null;
  target_kind?: FsKind | null;
  broken?: boolean;
};

export type View = {
  rows: ViewRow[];
  /** Echoed back, so a history stack can hold what it asked for. */
  origin: Source;
  order: Order;
  /** What this list is and is not, in words. */
  basis: string;
  total: number;
  truncated: boolean;
  hidden: number;
  /** True while a better order is expected as a `view:update`. */
  refining: boolean;
  elapsed_ms: number;
  cursor?: string | null;
};

export type ViewUpdate = {
  request_id: number;
  rows: ViewRow[];
  elapsed_ms: number;
  model?: string | null;
};

/** What the shared thumbnail cache holds for a viewport's worth of rows. */
export type ThumbnailsAnswer = {
  found: { path: string; png: string; width: number; height: number; size: string }[];
  /** Not cached, and worth rendering. The kind is what it would cost. */
  missing: { path: string; kind: "image" | "pdf" | "video" | "audio" }[];
  /** Not cached and not worth asking about again: denied, gone, or un-thumbnailable. */
  skipped: string[];
};

/** What `extractArchive` answers: what is inside, or the handle of an unpacking. */
export type ExtractAnswer =
  | {
      outcome: "planned";
      format: ArchiveFormat;
      destination: string;
      total_files: number;
      total_bytes: number;
      entries: ArchiveEntry[];
      /** True when `entries` is a prefix rather than the whole archive. */
      truncated: boolean;
      /** Something is already where it would go. */
      occupied: boolean;
    }
  | {
      outcome: "started";
      operation: number;
      destination: string;
      total_files: number;
      total_bytes: number;
    };

export type ArchiveFormat = "zip" | "tar" | "tar.gz";

export type ArchiveEntry = {
  /** Relative to the destination, and already checked: nothing here escapes it. */
  path: string;
  kind: FsKind;
  size: number;
};

export type TransferProgress = {
  operation: number;
  files_done: number;
  bytes_done: number;
  total_files: number;
  total_bytes: number;
  fraction: number;
  current: string | null;
};

export type TransferDone = {
  operation: number;
  moved: string[];
  /** Sources that did not make it, each with why. A batch never stops at the first problem. */
  failed: { source: string; reason: string }[];
  cancelled: boolean;
  undo_ids: number[];
  elapsed_ms: number;
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
  /**
   * How long this run waited for the daemon's socket.
   *
   * Nonzero only at a login, where both services start together. A run that waited measured two
   * services racing rather than this one starting, and its number does not belong next to the
   * budget — which is why it is reported rather than dropped.
   */
  waited_for_daemon_ms: number;
  /**
   * Whether WebKit is compositing in software rather than on the GPU.
   *
   * True when the process was launched with `WEBKIT_DISABLE_DMABUF_RENDERER=1`, which some
   * driver pairings need to render without glitches (D54, D57). The page cannot see that
   * variable, and it changes what an ambient animation costs by two orders of magnitude, so the
   * shell reports it.
   */
  software_compositing: boolean;
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

  list: (
    path: string,
    options: { sort?: SortBy; reverse?: boolean; depth?: number; limit?: number } = {},
  ) =>
    transport.invoke<FsListResult>("list", {
      path,
      sort: options.sort ?? "name",
      reverse: options.reverse ?? false,
      depth: options.depth ?? 1,
      limit: options.limit,
    }),

  /** Start a copy or a move. Answers with a handle; progress arrives as events. */
  transfer: (
    sources: string[],
    destination: string,
    mode: TransferMode,
    conflict: ConflictChoice = "fail",
    dryRun = false,
  ) =>
    transport.invoke<TransferAnswer>("transfer", {
      sources,
      destination,
      mode,
      conflict,
      dryRun,
    }),

  /**
   * Open a view over a source.
   *
   * The one call that produces rows. `fs.list` and `search` are still there for the CLI and for
   * agents; this is what the window asks, and it answers in one shape whatever the source was.
   */
  viewOpen: (
    source: Source,
    options: { order?: Order; reverse?: boolean; limit?: number } = {},
  ) =>
    transport.invoke<View>("view_open", {
      source,
      order: options.order ?? null,
      reverse: options.reverse ?? false,
      limit: options.limit ?? null,
    }),

  /** A better order for a view already on screen, once the reranker has run. */
  onViewUpdate: (handler: (update: ViewUpdate) => void) =>
    transport.listen("view:update", (payload) => handler(payload as ViewUpdate)),

  /**
   * Which of these rows already have a thumbnail on the disk, and which could have one.
   *
   * Never renders — see the hook that calls it. One call per viewport; the answer names the
   * hits, the misses worth rendering, and the paths not worth asking about again.
   */
  thumbnails: (paths: string[], pixels = 128) =>
    transport.invoke<ThumbnailsAnswer>("thumbnails", { paths, pixels }),

  /**
   * Unpack an archive. Answers with a handle; progress arrives on the same events a transfer
   * uses. A dry run says what is inside and where it would go, and writes nothing.
   */
  extractArchive: (
    archive: string,
    destination: string | null = null,
    conflict: ConflictChoice = "fail",
    dryRun = false,
  ) =>
    transport.invoke<ExtractAnswer>("extract_archive", {
      archive,
      destination,
      conflict,
      dryRun,
    }),

  /**
   * Whether the window is on screen.
   *
   * A hidden GTK window does not stop the page inside it: WebKit keeps running the document, so
   * a CSS animation keeps animating and a `setInterval` keeps firing. For a *resident* overlay
   * that is forever. Measured before this existed, with software compositing: 99% of a core
   * with nothing on screen.
   */
  onVisibility: (handler: (visible: boolean) => void) =>
    transport.listen("window:visible", (payload) => handler(payload === true)),

  /** Where a person would like to start, rather than where the filesystem starts. */
  places: () => transport.invoke<FsPlacesResult>("places"),

  cancelOperation: (operation: number) =>
    transport.invoke<{ cancelled: boolean }>("cancel_operation", { operation }),

  onTransferProgress: (handler: (progress: TransferProgress) => void) =>
    transport.listen("transfer:progress", (payload) => handler(payload as TransferProgress)),

  onTransferDone: (handler: (done: TransferDone) => void) =>
    transport.listen("transfer:done", (payload) => handler(payload as TransferDone)),

  mkdir: (path: string) => transport.invoke<{ path: string }>("mkdir", { path }),

  rename: (path: string, name: string) =>
    transport.invoke<{ path: string }>("rename_entry", { path, name }),

  trash: (path: string) => transport.invoke<unknown>("trash", { path }),

  /**
   * The *system* clipboard, which is the shell's rather than the daemon's (D49).
   *
   * These are the only three calls in this file that do not reach the daemon: it is headless,
   * and a clipboard belongs to a session. Everything else here is forwarding.
   */
  clipboardWrite: (paths: string[], mode: TransferMode) =>
    transport.invoke<void>("clipboard_write", { paths, mode }),

  /** What is on the clipboard, if it is files. Null for text, an image, or nothing. */
  clipboardRead: () => transport.invoke<SystemClipboard | null>("clipboard_read"),

  clipboardClear: () => transport.invoke<void>("clipboard_clear"),

  /**
   * Files dragged in from another application.
   *
   * The shell intercepts the drop before the page sees it, which is why this is an event rather
   * than a DOM handler: a webview is handed a list of paths, not a list of files it could read.
   * That is the right way round — the daemon does the reading, under the deny-list.
   */
  onFilesDropped: (handler: (paths: string[]) => void) =>
    transport.listen("tauri://drag-drop", (payload) => {
      const dropped = payload as { paths?: string[] } | null;
      if (dropped?.paths && dropped.paths.length > 0) handler(dropped.paths);
    }),

  /** Something is being dragged over the window. For showing that it can be let go of. */
  onDragOver: (handler: (over: boolean) => void) =>
    transport
      .listen("tauri://drag-enter", () => handler(true))
      .then((offEnter) =>
        Promise.all([
          transport.listen("tauri://drag-leave", () => handler(false)),
          transport.listen("tauri://drag-drop", () => handler(false)),
        ]).then((offs) => () => {
          offEnter();
          for (const off of offs) off();
        }),
      ),

  undo: (last = 1) => transport.invoke<{ undone: unknown[] }>("undo_last", { last }),

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

  /**
   * What can open this file, the desktop's default first.
   *
   * `all` asks for every installed application instead — for the times the type's own list is
   * empty or has nothing anybody wants, which is the whole reason a chooser exists.
   */
  appsForPath: (path: string, all = false) =>
    transport.invoke<AppsForPath>("apps_for_path", { path, all }),

  /**
   * Mount a drive that is plugged in but not mounted, and answer with where it landed.
   *
   * `interactive` lets polkit ask for a password. Only ever after an ordinary attempt came back
   * with {@link NEEDS_AUTHORISATION}, because that is the only moment a person is standing in
   * front of the dialog that appears.
   */
  mount: (device: string, interactive = false) =>
    transport.invoke<{ device: string; mount_point: string; mounted_now: boolean }>("mount_volume", {
      device,
      interactive,
    }),

  /** Unmount one, optionally powering it off so it can be unplugged. */
  unmount: (device: string, eject = false) =>
    transport.invoke<{ device: string; ejected: boolean }>("unmount_volume", { device, eject }),

  /** Open these files with a named application rather than with the desktop's choice. */
  openWith: (application: Application, paths: string[]) =>
    transport.invoke<void>("open_with", { application, paths }),

  /** Make this application what the whole desktop opens the type with, from now on. */
  appsSetDefault: (mime: string, application: string) =>
    transport.invoke<unknown>("apps_set_default", { mime, application }),

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

  // ---- The Agent Inspector -------------------------------------------------------------

  agentSessions: (limit = 50) =>
    transport.invoke<SessionsResult>("agent_sessions", { limit }),

  /**
   * Every memory in the index, in every scope, including superseded ones.
   *
   * The agent surface refuses this listing and the window is permitted it, and that asymmetry
   * is the whole point of the panel (D78): a system that lets a machine form durable beliefs
   * about a person's work and does not let the person read them is a system keeping secrets.
   */
  memories: (limit = 500, includeSuperseded = true) =>
    transport.invoke<MemoriesResult>("memories", {
      limit,
      includeSuperseded,
    }),

  /**
   * What is known about one file and is not in it.
   *
   * The same call with the daemon's own subject filter. Not a different list narrowed here —
   * "which memories are about this path" needs the node the path resolves to, which is the
   * daemon's to know.
   */
  memoriesAbout: (path: string, limit = 20) =>
    transport.invoke<MemoriesResult>("memories", {
      about: path,
      limit,
      includeSuperseded: false,
    }),

  /**
   * Write a memory as the human.
   *
   * `global` scope, because the window has no agent identity and because a person stating a
   * preference states it for everything. `supersedes` carries the memory being corrected, which
   * is what makes this a correction rather than a second opinion sitting beside the first.
   */
  remember: (text: string, kind: string, supersedes: number[] = []) =>
    transport.invoke<{ memory_id: number; scope: string }>("remember", {
      text,
      kind,
      supersedes,
    }),

  forgetMemory: (memoryId: number) =>
    transport.invoke<{ changed: boolean }>("forget_memory", { memoryId }),

  supersedeMemory: (oldId: number, newId: number) =>
    transport.invoke<{ changed: boolean }>("supersede_memory", { old: oldId, new: newId }),

  agentStatus: () => transport.invoke<AgentStatusResult>("agent_status"),

  agentSetEnabled: (enabled: boolean) =>
    transport.invoke<AgentStatusResult>("agent_set_enabled", { enabled }),

  /**
   * The audit log, newest first (R8).
   *
   * `refusedOnly` is the daemon's filter and not this window's. D4: the frontend renders what
   * it is sent, so "only the refused ones" is a different call rather than a different view of
   * one answer — and there is an architectural test that fails if that slips.
   */
  auditRecent: (limit = 100, refusedOnly = false, after: number | null = null) =>
    transport.invoke<AuditRecentResult>("audit_recent", { limit, refusedOnly, after }),

  /** The whole brain, for the three-dimensional view. Large; asked for only when it is shown. */
  brainGraph: (limit = 4000) => transport.invoke<unknown>("brain_graph", { limit }),

  /**
   * What has happened since `since`, and a new cursor.
   *
   * The live tracking in the Inspector's three-dimensional tab asks this instead of fetching
   * the graph again: `brain.diff` exists precisely so that a client does not poll for a large
   * answer to a small question, and the graph is the one call the protocol documents as large.
   */
  brainDiff: (since: number | null, limit = 200) =>
    transport.invoke<BrainDiff>("brain_diff", { since, limit }),

  brainReport: () => transport.invoke<unknown>("brain_report"),

  startupReport: () => transport.invoke<StartupReport>("startup_report"),

  /** The reranked order, arriving after the answer it improves (D7). */
  onSearchUpdate: (handler: (update: SearchUpdate) => void) =>
    transport.listen("search:update", (payload) => handler(payload as SearchUpdate)),

  /**
   * The window was summoned again.
   *
   * A launcher starts empty every time. Leaving the previous query on screen would mean every
   * summon begins by deleting somebody else's search — and worse, the first keystroke would
   * append to it.
   */
  onWindowShown: (handler: () => void) => transport.listen("window:shown", () => handler()),

  onDaemonUnavailable: (handler: (message: string) => void) =>
    transport.listen("daemon:unavailable", (payload) => handler(String(payload))),
};
