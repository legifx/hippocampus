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
    // Every call is recorded, so a test can assert that five selected rows became one request
    // naming five paths. What happens to those paths afterwards is the daemon's own tests.
    ((window as unknown as HarnessWindow).__hcCalls ??= []).push({
      command,
      args: args ?? {},
    });

    switch (command) {
      case "search": {
        const query = String(args?.query ?? "");
        // The fixture is a daemon, so it answers with everything: the frontend must not be the
        // thing that decides which rows match.
        return { hits: query ? ALL : [], mode: "auto", elapsed_ms: 3, partial: false } as T;
      }
      case "list": {
        // A directory the layout tests can navigate: two folders, three files, and names long
        // enough to threaten a 400 px column.
        const at = String(args?.path ?? "/");
        const base = at === "/" ? "" : at.replace(/\/$/, "");
        const entry = (name: string, kind: string, size: number, i: number) => ({
          path: `${base}/${name}`,
          kind,
          size,
          mtime: 1_700_000_000 - i * 86_400,
          indexed: i % 3 !== 0,
        });
        return {
          path: at,
          entries: [
            entry("a-rather-long-directory-name-for-testing", "dir", 4096, 0),
            entry("zweiter-ordner", "dir", 4096, 1),
            entry("notes-with-a-long-enough-name-to-elide.md", "file", 4096, 2),
            entry("bild.png", "file", 2_400_000, 3),
            entry("kurz.txt", "file", 12, 4),
          ],
          truncated: false,
          hidden: 1,
        } as T;
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
      case "preview": {
        // A 1×1 PNG as a data URL: the harness has no thumbnail cache and the layout tests care
        // about the box the image sits in, not about what is inside it.
        return {
          path: String(args?.path ?? ""),
          png: "/fixture/thumbnail.png",
          width: 320,
          height: 240,
          kind: "image",
          size: "x-large",
          cached: true,
          elapsed_ms: 1,
        } as T;
      }
      case "transfer":
        return { operation: 1, total_files: 2, total_bytes: 1000 } as T;
      case "cancel_operation":
        return { cancelled: true } as T;
      case "mkdir":
      case "rename_entry":
        return { path: String(args?.path ?? "") } as T;
      case "trash":
        return {} as T;
      case "undo_last":
        return { undone: [{}] } as T;
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
  // The harness records listeners and exposes a way to fire them, so a test can drive the
  // events the Rust side would send — `window:shown` in particular, which has no other way in.
  async listen(event: string, handler: (payload: unknown) => void) {
    const listeners = ((window as unknown as HarnessWindow).__hcListeners ??= new Map());
    const forEvent = listeners.get(event) ?? new Set();
    forEvent.add(handler);
    listeners.set(event, forEvent);
    return () => forEvent.delete(handler);
  },
  // A 1×1 transparent PNG, so an `<img>` in the harness loads rather than showing a broken
  // icon and changing the layout being measured.
  assetUrl: () =>
    "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
});

type HarnessWindow = {
  __hcListeners?: Map<string, Set<(payload: unknown) => void>>;
  __hcEmit?: (event: string, payload?: unknown) => void;
  __hcCalls?: { command: string; args: Record<string, unknown> }[];
};

(window as unknown as HarnessWindow).__hcEmit = (event, payload) => {
  (window as unknown as HarnessWindow).__hcListeners
    ?.get(event)
    ?.forEach((handler) => handler(payload));
};

const root = document.getElementById("root");
if (!root) throw new Error("the harness document has no #root");
createRoot(root).render(<App />);
