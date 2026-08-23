/**
 * Where each node is drawn.
 *
 * Two layouts, because they answer different questions and the animation between them is the
 * clearest thing in this whole view:
 *
 * - **Tree.** Containment as it is: directories branch, files are the leaves. This is what the
 *   index *is*, and it is the layout in which a person can find something they recognise.
 * - **Shells.** Concentric shells by tier — Working at the centre, Cold on the outside. This is
 *   what the index *believes*, and it is the only layout in which forgetting is visible: a
 *   demoted node physically moves outward.
 *
 * Both are deterministic functions of the snapshot. Nothing here is iterative and nothing is
 * random: the same snapshot lays out identically every time, on every machine, which is what
 * makes a screenshot of it comparable to the next one. A force-directed layout would look more
 * alive and would be a different picture on every reload.
 */

import type { GraphEdge, GraphNode, Tier } from "./snapshot";

/** Radius of each tier's shell, in scene units. */
export const SHELL_RADIUS: Record<Tier, number> = {
  working: 34,
  stm: 78,
  ltm: 124,
  cold: 170,
};

const BRANCH_BASE = 62;
const BRANCH_DECAY = 0.68;
/** The angle a subtree is allowed to open into, in radians. */
const SPREAD = 1.15;
const GOLDEN_ANGLE = Math.PI * (3 - Math.sqrt(5));

export interface Positions {
  /** `xyz` per node, in the order the nodes were given. */
  tree: Float32Array;
  shells: Float32Array;
  /** The radius that contains everything, for framing the camera. */
  extent: number;
  /** Depth in the containment tree, per node. */
  depth: Int32Array;
}

/** FNV-1a. Stable across machines and reloads, which `Math.random` is not. */
export function hash(text: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

/** A unit vector from a hash, evenly distributed over the sphere. */
function unitFromHash(h: number, out: [number, number, number]): void {
  const z = ((h & 0xffff) / 0xffff) * 2 - 1;
  const theta = ((h >>> 16) / 0xffff) * Math.PI * 2;
  const r = Math.sqrt(Math.max(0, 1 - z * z));
  out[0] = Math.cos(theta) * r;
  out[1] = z;
  out[2] = Math.sin(theta) * r;
}

/** Any vector perpendicular to `v`, chosen without a branch on the degenerate case. */
function perpendicular(v: [number, number, number]): [number, number, number] {
  const a: [number, number, number] = Math.abs(v[1]) < 0.9 ? [0, 1, 0] : [1, 0, 0];
  const c: [number, number, number] = [
    v[1] * a[2] - v[2] * a[1],
    v[2] * a[0] - v[0] * a[2],
    v[0] * a[1] - v[1] * a[0],
  ];
  const len = Math.hypot(c[0], c[1], c[2]) || 1;
  return [c[0] / len, c[1] / len, c[2] / len];
}

/**
 * Concepts have no parent and no path, so the tree has nowhere to put them. Placing them at the
 * centre of the files they abstract is the only position that means anything: the concept sits
 * among its material, and the edges to its members are short instead of crossing the scene.
 */
function placeConcepts(
  edges: GraphEdge[],
  index: Map<number, number>,
  positions: Float32Array,
  lift: number,
): void {
  const members = new Map<number, number[]>();
  for (const edge of edges) {
    if (edge.type !== "derived_from") continue;
    const list = members.get(edge.src);
    if (list) list.push(edge.dst);
    else members.set(edge.src, [edge.dst]);
  }
  for (const [concept, ids] of members) {
    const at = index.get(concept);
    if (at === undefined) continue;
    let x = 0;
    let y = 0;
    let z = 0;
    let counted = 0;
    for (const id of ids) {
      const member = index.get(id);
      if (member === undefined) continue;
      x += positions[member * 3];
      y += positions[member * 3 + 1];
      z += positions[member * 3 + 2];
      counted++;
    }
    if (counted === 0) continue;
    // Lifted out of the plane of its members so it is not buried inside the cluster it names.
    positions[at * 3] = x / counted;
    positions[at * 3 + 1] = y / counted + lift;
    positions[at * 3 + 2] = z / counted;
  }
}

export function layout(nodes: GraphNode[], edges: GraphEdge[] = []): Positions {
  const count = nodes.length;
  const tree = new Float32Array(count * 3);
  const shells = new Float32Array(count * 3);
  const depth = new Int32Array(count);

  const index = new Map<number, number>();
  for (let i = 0; i < count; i++) index.set(nodes[i].id, i);

  // Children, and the roots of whatever forest this snapshot happens to be. A snapshot is a
  // sample, so a node whose parent was not sent is a root here even though it has one in the
  // index — which is correct for a picture of the sample.
  const children: number[][] = Array.from({ length: count }, () => []);
  const roots: number[] = [];
  for (let i = 0; i < count; i++) {
    const parent = nodes[i].parent;
    const p = parent === null ? undefined : index.get(parent);
    if (p === undefined || p === i) roots.push(i);
    else children[p].push(i);
  }

  // Zones become lobes: every root is pushed out along its zone's direction, so a project ends
  // up as a recognisable limb rather than being interleaved with every other project.
  const zoneDirection = new Map<number, [number, number, number]>();
  const zones = [...new Set(nodes.map((n) => n.zone).filter((z): z is number => z !== null))].sort(
    (a, b) => a - b,
  );
  zones.forEach((zone, i) => {
    const y = zones.length === 1 ? 0 : 1 - (i / (zones.length - 1)) * 2;
    const radius = Math.sqrt(Math.max(0, 1 - y * y));
    const theta = GOLDEN_ANGLE * i;
    zoneDirection.set(zone, [Math.cos(theta) * radius, y * 0.55, Math.sin(theta) * radius]);
  });

  const stack: Array<{ i: number; axis: [number, number, number]; depth: number }> = [];
  roots.forEach((root, order) => {
    const node = nodes[root];
    const zoneAxis = node.zone !== null ? zoneDirection.get(node.zone) : undefined;
    const axis: [number, number, number] = [0, 0, 0];
    if (zoneAxis) {
      axis[0] = zoneAxis[0];
      axis[1] = zoneAxis[1];
      axis[2] = zoneAxis[2];
    } else {
      unitFromHash(hash(node.path) ^ (order * 2654435761), axis);
    }
    const spread = roots.length > 1 ? BRANCH_BASE * 1.6 : 0;
    tree[root * 3] = axis[0] * spread;
    tree[root * 3 + 1] = axis[1] * spread;
    tree[root * 3 + 2] = axis[2] * spread;
    stack.push({ i: root, axis, depth: 0 });
  });

  // Iterative, not recursive: a deep directory tree is exactly the shape that overflows a
  // JavaScript stack, and an index has no depth limit.
  let extent = 1;
  while (stack.length > 0) {
    const { i, axis, depth: d } = stack.pop()!;
    depth[i] = d;
    const kids = children[i];
    if (kids.length === 0) continue;

    const length = BRANCH_BASE * Math.pow(BRANCH_DECAY, d) + 12;
    const u = perpendicular(axis);
    const v: [number, number, number] = [
      axis[1] * u[2] - axis[2] * u[1],
      axis[2] * u[0] - axis[0] * u[2],
      axis[0] * u[1] - axis[1] * u[0],
    ];

    // A deterministic cone around the parent's own direction: children fan outwards rather
    // than doubling back through their parent, which is what makes the shape readable.
    for (let k = 0; k < kids.length; k++) {
      const child = kids[k];
      const jitter = (hash(nodes[child].name) % 1000) / 1000;
      const angle = GOLDEN_ANGLE * (k + jitter);
      const t = kids.length === 1 ? 0.5 : k / (kids.length - 1);
      const tilt = SPREAD * (0.35 + 0.65 * t);
      const sin = Math.sin(tilt);
      const dir: [number, number, number] = [
        axis[0] * Math.cos(tilt) + (u[0] * Math.cos(angle) + v[0] * Math.sin(angle)) * sin,
        axis[1] * Math.cos(tilt) + (u[1] * Math.cos(angle) + v[1] * Math.sin(angle)) * sin,
        axis[2] * Math.cos(tilt) + (u[2] * Math.cos(angle) + v[2] * Math.sin(angle)) * sin,
      ];
      const len = Math.hypot(dir[0], dir[1], dir[2]) || 1;
      dir[0] /= len;
      dir[1] /= len;
      dir[2] /= len;

      const stretch = length * (0.75 + 0.5 * jitter);
      const x = tree[i * 3] + dir[0] * stretch;
      const y = tree[i * 3 + 1] + dir[1] * stretch;
      const z = tree[i * 3 + 2] + dir[2] * stretch;
      tree[child * 3] = x;
      tree[child * 3 + 1] = y;
      tree[child * 3 + 2] = z;
      extent = Math.max(extent, Math.hypot(x, y, z));
      stack.push({ i: child, axis: dir, depth: d + 1 });
    }
  }

  // The shells keep each node's direction from the tree layout, so switching between the two
  // is a slide along one line rather than a reshuffle. The eye can follow a single node
  // through the transition, which is the entire point of having the transition.
  const scratch: [number, number, number] = [0, 0, 0];
  for (let i = 0; i < count; i++) {
    const x = tree[i * 3];
    const y = tree[i * 3 + 1];
    const z = tree[i * 3 + 2];
    let len = Math.hypot(x, y, z);
    let dx: number;
    let dy: number;
    let dz: number;
    if (len < 1e-6) {
      unitFromHash(hash(nodes[i].path), scratch);
      [dx, dy, dz] = scratch;
    } else {
      dx = x / len;
      dy = y / len;
      dz = z / len;
    }
    // Salience pulls a node slightly in from its shell, so the front of each shell is its most
    // important members. The offset is smaller than the gap between shells: a node's tier must
    // never be ambiguous.
    const shell = SHELL_RADIUS[nodes[i].tier] ?? SHELL_RADIUS.cold;
    const pull = Math.min(14, nodes[i].salience * 3);
    const jitter = ((hash(nodes[i].path + "shell") % 1000) / 1000 - 0.5) * 9;
    const radius = shell - pull + jitter;
    shells[i * 3] = dx * radius;
    shells[i * 3 + 1] = dy * radius;
    shells[i * 3 + 2] = dz * radius;
  }

  // After both layouts exist, so a concept lands among its members in each of them.
  placeConcepts(edges, index, tree, 26);
  placeConcepts(edges, index, shells, 12);

  return { tree, shells, extent: Math.max(extent, SHELL_RADIUS.cold), depth };
}
