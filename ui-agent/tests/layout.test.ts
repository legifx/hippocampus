/**
 * The two properties the picture depends on and that are cheap to check without a GPU:
 * the layout is a pure function of the snapshot, and a malformed snapshot is refused at the
 * door rather than drawn as `undefined`.
 */

import { describe, expect, it } from "vitest";
import { layout } from "../src/layout";
import { assertSnapshot, tierRank } from "../src/snapshot";
import type { GraphNode, GraphSnapshot } from "../src/snapshot";

function node(id: number, path: string, parent: number | null, over: Partial<GraphNode> = {}): GraphNode {
  return {
    id,
    path,
    name: path.split("/").pop() ?? path,
    kind: "file",
    parent,
    tier: "stm",
    salience: 1,
    pinned: false,
    zone: 1,
    size: 10,
    mtime: 0,
    degree: 0,
    recent_touches: 0,
    ...over,
  };
}

const TREE: GraphNode[] = [
  node(1, "/w", null, { kind: "dir" }),
  node(2, "/w/a", 1, { kind: "dir" }),
  node(3, "/w/a/one.md", 2),
  node(4, "/w/a/two.md", 2),
  node(5, "/w/b.md", 1, { tier: "cold", salience: 0 }),
];

describe("layout", () => {
  it("is the same on every run", () => {
    const a = layout(TREE);
    const b = layout(TREE);
    expect(Array.from(a.tree)).toEqual(Array.from(b.tree));
    expect(Array.from(a.shells)).toEqual(Array.from(b.shells));
  });

  it("puts a child further from the origin than nothing and gives every node a position", () => {
    const { tree } = layout(TREE);
    for (let i = 0; i < TREE.length; i++) {
      const x = tree[i * 3];
      const y = tree[i * 3 + 1];
      const z = tree[i * 3 + 2];
      expect(Number.isFinite(x) && Number.isFinite(y) && Number.isFinite(z)).toBe(true);
    }
  });

  it("orders the shells by tier, hottest innermost", () => {
    const nodes = (["working", "stm", "ltm", "cold"] as const).map((tier, i) =>
      node(i + 1, `/w/${tier}.md`, null, { tier, salience: 0 }),
    );
    const { shells } = layout(nodes);
    const radii = nodes.map((_, i) =>
      Math.hypot(shells[i * 3], shells[i * 3 + 1], shells[i * 3 + 2]),
    );
    for (let i = 1; i < radii.length; i++) {
      expect(radii[i]).toBeGreaterThan(radii[i - 1]);
    }
  });

  it("does not overflow the stack on a deep tree", () => {
    const deep: GraphNode[] = [node(1, "/w", null, { kind: "dir" })];
    for (let i = 2; i <= 20_000; i++) deep.push(node(i, `/w/${i}`, i - 1, { kind: "dir" }));
    expect(() => layout(deep)).not.toThrow();
  });

  it("places a node whose parent was not sent as a root rather than dropping it", () => {
    const orphan = [node(9, "/w/x/y.md", 404)];
    const { tree } = layout(orphan);
    expect(Number.isFinite(tree[0])).toBe(true);
  });
});

describe("concepts", () => {
  const withConcept: GraphNode[] = [
    node(1, "/w/a.md", null),
    node(2, "/w/b.md", null),
    node(9, "parser · grammar", null, { kind: "concept", zone: null }),
  ];
  const edges = [
    { src: 9, dst: 1, type: "derived_from", weight: 1, source: "inferred" },
    { src: 9, dst: 2, type: "derived_from", weight: 1, source: "inferred" },
  ];

  it("places a concept among the files it abstracts", () => {
    const { tree } = layout(withConcept, edges);
    const mid = [0, 1, 2].map((axis) => (tree[axis] + tree[3 + axis]) / 2);
    // Same place as its members in x and z, lifted clear of them in y.
    // The concept is the third node, so its offset is 6 — not 18, which is what a first pass
    // at this test used and which reads NaN off the end of the buffer rather than failing.
    expect(Math.abs(tree[6] - mid[0])).toBeLessThan(0.001);
    expect(Math.abs(tree[8] - mid[2])).toBeLessThan(0.001);
    expect(tree[7]).toBeGreaterThan(mid[1]);
  });

  it("leaves a concept whose members were not sent where it was", () => {
    const orphan = [node(9, "lonely", null, { kind: "concept" })];
    expect(() => layout(orphan, [{ src: 9, dst: 404, type: "derived_from", weight: 1, source: "inferred" }]))
      .not.toThrow();
  });
});

describe("snapshot validation", () => {
  const valid: GraphSnapshot = {
    generated_at: 1,
    nodes: TREE,
    edges: [],
    zones: [],
    events: [],
    working_set: [],
    totals: { nodes: 5, edges: 0, episodes: 0, tiers: {}, last_consolidation: null },
    basis: "test",
    report: null,
  };

  it("accepts what the daemon sends", () => {
    expect(assertSnapshot(structuredClone(valid)).nodes).toHaveLength(5);
  });

  it("names the field that is wrong instead of failing later", () => {
    const broken = { ...structuredClone(valid), edges: undefined };
    expect(() => assertSnapshot(broken)).toThrow(/edges/);
  });

  it("treats a missing report as no report", () => {
    const without = structuredClone(valid) as Partial<GraphSnapshot>;
    delete without.report;
    expect(assertSnapshot(without).report).toBeNull();
  });

  it("ranks tiers hottest first", () => {
    expect(tierRank("working")).toBeLessThan(tierRank("cold"));
  });
});
