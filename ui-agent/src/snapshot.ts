/**
 * The shape of what the daemon sends, and the only place this app gets data from.
 *
 * These types mirror `cortex-store/src/memory.rs` exactly. They are hand-written rather than
 * generated because a generator would be a build step for four hundred lines that change once
 * a milestone — but they are not free-form: `assertSnapshot` checks the shape at the door, so a
 * daemon that changed its mind about a field name produces a clear error instead of a scene
 * full of `undefined`.
 *
 * Decision D4 forbids data processing in a frontend: this view renders what it is given and
 * never filters, sorts or searches. Geometry is not processing — where a node is drawn is a
 * property of the picture, not of the brain — but *which* nodes exist, what they score and
 * what they are connected to is the daemon's answer and is used as it arrives.
 */

export type Tier = "working" | "stm" | "ltm" | "cold";

export interface GraphNode {
  id: number;
  path: string;
  name: string;
  kind: "file" | "dir" | "concept" | "memory" | string;
  parent: number | null;
  tier: Tier;
  salience: number;
  pinned: boolean;
  zone: number | null;
  size: number;
  mtime: number;
  degree: number;
  recent_touches: number;
}

export interface GraphEdge {
  src: number;
  dst: number;
  type:
    | "contains"
    | "co_access"
    | "similar_to"
    | "annotates"
    | "supersedes"
    | "recalled_with"
    | string;
  weight: number;
  source: "observed" | "inferred" | "explicit" | string;
}

export interface GraphZone {
  id: number;
  name: string;
  hue: number;
  /** The daemon's icon name for the zone, which is also how it classifies the place. */
  icon: string | null;
  nodes: number;
}

export interface GraphEvent {
  ts: number;
  actor: string;
  action: string;
  node: number | null;
}

export interface TierMovement {
  from: Tier;
  to: Tier;
  count: number;
}

export interface Highlight {
  node_id: number;
  path: string;
  salience: number;
  tier: Tier;
  reason: string;
}

export interface ConsolidationReport {
  ran_at: number;
  window_from: number;
  window_to: number;
  episodes_replayed: number;
  bursts: number;
  bursts_discarded: number;
  edges_written: number;
  edges_unchanged: number;
  nodes_scored: number;
  movements: TierMovement[];
  tier_totals: Record<string, number>;
  pinned_held: number;
  top: Highlight[];
  demoted: Highlight[];
  dry_run: boolean;
  duration_ms: number;
  not_done: string[];
}

export interface GraphTotals {
  nodes: number;
  edges: number;
  episodes: number;
  tiers: Record<string, number>;
  last_consolidation: number | null;
}

export interface GraphSnapshot {
  generated_at: number;
  nodes: GraphNode[];
  edges: GraphEdge[];
  zones: GraphZone[];
  events: GraphEvent[];
  working_set: number[];
  totals: GraphTotals;
  basis: string;
  report: ConsolidationReport | null;
}

/** Where a snapshot came from. Shown in the header, because it changes what the view means. */
export type Origin = "live" | "baked" | "generated";

export interface Loaded {
  snapshot: GraphSnapshot;
  origin: Origin;
}

/**
 * A snapshot baked into the page at build time, if there is one.
 *
 * The demo build writes this so the page is a single file that needs no daemon and no server.
 */
declare global {
  interface Window {
    __HC_SNAPSHOT__?: GraphSnapshot;
    __TAURI_INTERNALS__?: unknown;
  }
}

export function assertSnapshot(value: unknown): GraphSnapshot {
  const s = value as GraphSnapshot;
  const problems: string[] = [];
  if (!s || typeof s !== "object") throw new Error("snapshot is not an object");
  if (!Array.isArray(s.nodes)) problems.push("nodes");
  if (!Array.isArray(s.edges)) problems.push("edges");
  if (!Array.isArray(s.zones)) problems.push("zones");
  if (!Array.isArray(s.events)) problems.push("events");
  if (!Array.isArray(s.working_set)) problems.push("working_set");
  if (!s.totals || typeof s.totals.nodes !== "number") problems.push("totals");
  if (typeof s.basis !== "string") problems.push("basis");
  if (problems.length > 0) {
    throw new Error(`the daemon sent something this view does not understand: ${problems.join(", ")}`);
  }
  if (s.report === undefined) s.report = null;
  return s;
}

/**
 * Get a snapshot from wherever this build can reach one.
 *
 * The order is deliberate: a baked snapshot wins over the network, because the demo build must
 * never silently show live data if it happens to be opened next to a running daemon.
 */
export async function load(fetchImpl: typeof fetch = fetch): Promise<Loaded> {
  if (window.__HC_SNAPSHOT__) {
    return { snapshot: assertSnapshot(window.__HC_SNAPSHOT__), origin: "baked" };
  }
  const response = await fetchImpl("graph.json", { cache: "no-store" });
  if (!response.ok) {
    throw new Error(`graph.json: ${response.status} ${response.statusText}`);
  }
  const snapshot = assertSnapshot(await response.json());
  // A snapshot fetched from disk in the browser is a file somebody baked; only the Tauri shell
  // and the dev server proxy speak to a running daemon.
  return { snapshot, origin: window.__TAURI_INTERNALS__ ? "live" : "baked" };
}

/** Index nodes by id once, so nothing else has to scan the array. */
export function byId(nodes: GraphNode[]): Map<number, GraphNode> {
  const map = new Map<number, GraphNode>();
  for (const node of nodes) map.set(node.id, node);
  return map;
}

export const TIERS: Tier[] = ["working", "stm", "ltm", "cold"];

/** Rank a tier for anything that needs an order. Lower is hotter. */
export function tierRank(tier: Tier): number {
  const index = TIERS.indexOf(tier);
  return index < 0 ? TIERS.length : index;
}
