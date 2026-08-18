/**
 * The app, wired to a fixture instead of a daemon.
 *
 * This exists so that the layout tests run against a real layout engine rather than a DOM
 * emulation. M4 §4 says resizing from full width to 400 px must never produce horizontal
 * overflow or a clipped control — that is a statement about a browser's box model, and happy-dom
 * has no box model to be wrong about.
 *
 * It is a separate entry point rather than a flag in the app, so that nothing about the fixture
 * can end up in the shipped bundle.
 */

import { createRoot } from "react-dom/client";
import { App } from "../App";
import { setTransport, type SearchHit } from "../ipc";
import "../tokens.css";
import "../app.css";

const params = new URLSearchParams(location.search);
const HIT_COUNT = Number(params.get("hits") ?? "10000");

function hits(count: number): SearchHit[] {
  return Array.from({ length: count }, (_, i) => ({
    node_id: i + 1,
    // Long enough to overflow a 400 px column if anything forgets to elide.
    path: `/home/user/Projects/some-rather-long-project-name-${i}/src/deeply/nested/module-${i}.ts`,
    kind: "file",
    size: 4096 * (i + 1),
    mtime: 1_700_000_000 - i * 3600,
    zone_id: i % 4 === 0 ? 1 : null,
    tier: i % 50 === 0 ? "cold" : "stm",
    score: { path_rank: 1, lexical_rank: 3, vector_rank: null, fused: 0.42, salience_boost: 1.1, rerank: null, final_score: 0.46 },
    snippet: `…a snippet with enough text in it to threaten the layout of a narrow column, ${i}…`,
  }));
}

const ALL = hits(HIT_COUNT);

setTransport({
  async invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    switch (command) {
      case "search": {
        const query = String(args?.query ?? "");
        // The fixture is a daemon, so it answers with everything: the frontend must not be the
        // thing that decides which rows match.
        return { hits: query ? ALL : [], mode: "auto", elapsed_ms: 3, partial: false } as T;
      }
      case "zones":
        return { zones: [{ id: 1, name: "Projects", root: "/home/user/Projects", hue: 210, nodes: 9 }] } as T;
      case "extract_status":
        return { pending: 0, done: 10, failed: 0, skipped: 0 } as T;
      case "neighbors":
        return {
          target_id: 1,
          neighbors: [
            { node_id: 2, path: "/home/user/Projects/a/related-file-with-a-long-name.ts", kind: "file", edge: "derived_from", source: "explicit", weight: 1, depth: 1, note: null },
          ],
          annotations: [
            { node_id: 3, target_id: 1, tags: ["decision", "storage"], note: "eine Notiz, die lang genug ist, um in einer schmalen Spalte umzubrechen", actor: "agent:claude", ts: 1_700_000_000 },
          ],
        } as T;
      case "read_text":
        return {
          path: "x",
          content: "const answer = 42;\n".repeat(40),
          offset: 0,
          bytes: 760,
          total_bytes: 760,
          truncated: false,
          note: null,
        } as T;
      case "startup_report":
        return { connected_ms: 12, interactive_ms: 90, budget_ms: 300, within_budget: true } as T;
      case "mark_interactive":
        return 90 as T;
      default:
        return null as T;
    }
  },
  async listen() {
    return () => {};
  },
});

const root = document.getElementById("root");
if (!root) throw new Error("the harness document has no #root");
createRoot(root).render(<App />);
