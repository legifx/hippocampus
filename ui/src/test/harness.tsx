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
        // A share whose server has gone: the daemon knocks with a deadline and answers with
        // this rather than hanging. `?deadshare=1` is how a test asks for that machine.
        if (
          params.get("deadshare") === "1" &&
          String(args?.path ?? "").startsWith("/mnt/backup")
        ) {
          throw new Error(
            "server:/export/backup did not answer within 2000 ms — the share may be gone",
          );
        }
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
          // Every real listing carries these, so the fixture does too — a permissions column
          // that only ever appeared against a live daemon would be a column nothing tests.
          mode: kind === "dir" ? 0o040755 : 0o100644,
          owner: { id: 1000, name: "someone" },
          group: { id: 1000, name: "someone" },
        });
        // Links live one level down, so the root listing keeps the five entries the layout
        // tests are measured against.
        const links =
          base === ""
            ? []
            : [
                {
                  ...entry("verweis-auf-ordner", "symlink", 12, 5),
                  mode: 0o120777,
                  target: "/home/user/anderswo",
                  target_kind: "dir",
                },
                {
                  ...entry("kaputter-verweis", "symlink", 9, 6),
                  mode: 0o120777,
                  target: "/home/user/weg.txt",
                  broken: true,
                },
                {
                  ...entry("relativer-verweis", "symlink", 8, 7),
                  mode: 0o120777,
                  target: "kurz.txt",
                  target_kind: "file",
                },
                // An archive, for the extraction tests. One level down like the links, because
                // the root listing is what the layout tests count.
                entry("bilder.zip", "file", 3_000, 8),
                // Not in the cache: the row it belongs to is the one that has to be rendered.
                entry("foto.jpg", "file", 900_000, 9),
              ];
        return {
          path: at,
          entries: [
            entry("a-rather-long-directory-name-for-testing", "dir", 4096, 0),
            entry("zweiter-ordner", "dir", 4096, 1),
            entry("notes-with-a-long-enough-name-to-elide.md", "file", 4096, 2),
            entry("bild.png", "file", 2_400_000, 3),
            entry("kurz.txt", "file", 12, 4),
            ...links,
          ],
          truncated: false,
          hidden: 1,
        } as T;
      }
      case "mount_volume": {
        // `?mountfails=1` is an internal disk: polkit refuses it without a prompt. **The
        // rejection is an object and not an `Error`**, because that is what the shell sends —
        // `BridgeError` serialises as `{ code, message }` so the page can branch on the code —
        // and a fixture that threw a string would have hidden the bug this reproduces.
        if (params.get("mountfails")) {
          // eslint-disable-next-line no-throw-literal
          throw {
            code: -32603,
            message:
              "/dev/sda2 needs an administrator's permission, and this window never asks for one",
          };
        }
        // `?needsauth=1` is an internal disk: the ordinary attempt is refused with the code that
        // means "a password would do it", and the second one — the one that lets polkit ask —
        // works. Both halves, because the interesting part is that the window makes the second
        // call at all.
        if (params.get("needsauth") && args?.interactive !== true) {
          // eslint-disable-next-line no-throw-literal
          throw {
            code: 1004,
            message: "this needs an administrator's permission",
          };
        }
        (window as unknown as HarnessWindow).__hcMounted = String(args?.device ?? "");
        return {
          device: String(args?.device ?? ""),
          mount_point: "/run/media/user/ESD-USB",
          mounted_now: true,
        } as T;
      }
      case "unmount_volume": {
        (window as unknown as HarnessWindow).__hcEjected = {
          device: String(args?.device ?? ""),
          eject: args?.eject === true,
        };
        return { device: String(args?.device ?? ""), ejected: args?.eject === true } as T;
      }
      case "places":
        // The shape a real machine gives back: personal folders, a stick with a label and a
        // capacity, and one row for everything technical.
        return {
          sections: [
            {
              title: "Persönlich",
              places: [
                { path: "/home/user", name: "Persönlicher Ordner", kind: "home" },
                { path: "/home/user/Downloads", name: "Downloads", kind: "downloads" },
                { path: "/home/user/Dokumente", name: "Dokumente", kind: "documents" },
                {
                  path: "/home/user/.local/share/Trash/files",
                  name: "Papierkorb",
                  kind: "trash",
                },
              ],
            },
            {
              title: "Laufwerke",
              places: [
                {
                  path: "/run/media/user/Arbeit",
                  name: "Arbeit",
                  kind: "drive",
                  device: "/dev/sdc1",
                  capacity: { total_bytes: 31_400_000_000, free_bytes: 21_000_000_000 },
                  volume: { mounted: true, fstype: "ext4", size_bytes: 31_400_000_000, removable: true },
                },
                // Plugged in and not mounted: its path is a device node, and opening it means
                // mounting it first. `__hcMounted` records that the window did.
                {
                  path: "/dev/sdb1",
                  name: "ESD-USB",
                  kind: "drive",
                  device: "/dev/sdb1",
                  volume: { mounted: false, fstype: "vfat", size_bytes: 31_400_000_000, removable: true },
                },
              ],
            },
            {
              // Shares the desktop mounted, as the daemon reports them: no capacity, because
              // measuring one means a call that can block, and an address rather than a
              // mount path.
              title: "Netzwerk",
              places: [
                {
                  path: "/run/user/1000/gvfs/smb-share:server=nas,share=fotos",
                  name: "fotos auf nas",
                  kind: "network",
                  device: "smb://nas/fotos",
                },
                {
                  path: "/mnt/backup",
                  name: "backup auf server",
                  kind: "network",
                  device: "server:/export/backup",
                },
              ],
            },
            {
              title: "System",
              places: [
                {
                  path: "/",
                  name: "Dateisystem",
                  kind: "filesystem",
                  device: "/dev/nvme0n1p2",
                  capacity: { total_bytes: 423_000_000_000, free_bytes: 275_000_000_000 },
                },
              ],
            },
          ],
        } as T;
      case "zones":
        return { zones: [{ id: 1, name: "Projects", root: "/home/user/Projects", hue: 210, nodes: 9 }] } as T;
      case "extract_status":
        // `?pending=N` is how a test says the daemon is busy, and `__hcPending` is how it
        // changes its mind afterwards — the query string is read once when this module loads,
        // so rewriting the URL would look like a change and be none. Nothing pending is the
        // default and the ordinary state, and the breath layer is still while it holds.
        return {
          pending: Number(
            (window as unknown as HarnessWindow).__hcPending ?? params.get("pending") ?? "0",
          ),
          done: 10,
          failed: 0,
          skipped: 0,
        } as T;
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
      case "transfer": {
        // A dry run answers what would happen. The fixture reports a conflict when the sources
        // include the file the tests use for exactly that, so the dialog can be driven.
        const sources = (args?.sources as string[]) ?? [];
        if (args?.dryRun) {
          const clashing = sources.filter((s) => s.includes("kurz.txt"));
          return {
            outcome: "planned",
            total_files: sources.length,
            total_bytes: 1000,
            conflicts: clashing.map((source) => ({
              source,
              destination: source,
              existing: { path: source, kind: "file", size: 12, mtime: 1_600_000_000, indexed: true },
              incoming: { path: source, kind: "file", size: 4096, mtime: 1_700_000_000, indexed: false },
            })),
          } as T;
        }
        return { outcome: "started", operation: 1, total_files: 2, total_bytes: 1000 } as T;
      }
      case "thumbnails": {
        // The shared cache, as the previewer would report it: the picture is already there, the
        // second one has to be rendered, and everything else is not worth asking about again.
        const asked = (args?.paths as string[]) ?? [];
        return {
          found: asked
            .filter((path) => path.endsWith("bild.png"))
            .map((path) => ({ path, png: `${path}.thumb.png`, width: 128, height: 96, size: "normal" })),
          missing: asked
            .filter((path) => path.endsWith("foto.jpg"))
            .map((path) => ({ path, kind: "image" })),
          skipped: asked.filter(
            (path) => !path.endsWith("bild.png") && !path.endsWith("foto.jpg"),
          ),
        } as T;
      }
      case "extract_archive": {
        // What the daemon answers, in the two halves the window asks for: what is inside, and
        // then the handle. Where it would go is the daemon's decision, so the fixture makes it
        // the same way — a new directory beside the archive, named after it.
        const archive = String(args?.archive ?? "");
        const destination = archive.replace(/\.zip$/, "");
        if (args?.dryRun) {
          return {
            outcome: "planned",
            format: "zip",
            destination,
            total_files: 3,
            total_bytes: 3000,
            entries: [
              { path: "eins.png", kind: "file", size: 1000 },
              { path: "zwei.png", kind: "file", size: 1000 },
              { path: "unterordner/drei.png", kind: "file", size: 1000 },
            ],
            truncated: false,
            occupied: false,
          } as T;
        }
        return {
          outcome: "started",
          operation: 7,
          destination,
          total_files: 3,
          total_bytes: 3000,
        } as T;
      }
      case "cancel_operation":
        return { cancelled: true } as T;

      // "Öffnen mit": in the shell these come from the desktop's association files, which a
      // browser has none of. What the tests can check is the window's half — which application
      // it offers, which one it calls `open_with` with, and that a remembered choice is
      // recorded before the file is opened rather than after.
      case "apps_for_path": {
        const all = args?.all === true;
        const path = String(args?.path ?? "");
        const claiming = [
          { id: "editor.desktop", name: "Editor", comment: "Text bearbeiten", exec: "edit %F", terminal: false, is_default: true, path: "/usr/share/applications/editor.desktop" },
          { id: "viewer.desktop", name: "Betrachter", exec: "view %f", terminal: false, is_default: false, path: "/usr/share/applications/viewer.desktop" },
        ];
        const everything = [
          ...claiming,
          { id: "vim.desktop", name: "Vim", exec: "vim %F", terminal: true, is_default: false, path: "/usr/share/applications/vim.desktop" },
        ];
        // `bild.png` in the fixture listing stands for the case a chooser exists for: a type
        // that nothing on the machine has registered for.
        const empty = path.endsWith(".png");
        return {
          path,
          mime: empty ? "application/octet-stream" : "text/plain",
          applications: all ? everything : empty ? [] : claiming,
        } as T;
      }
      case "open_with":
        (window as unknown as HarnessWindow).__hcOpenedWith = {
          id: String((args?.application as { id?: string })?.id ?? ""),
          paths: (args?.paths as string[]) ?? [],
        };
        return undefined as T;
      case "apps_set_default":
        (window as unknown as HarnessWindow).__hcDefaultSet = {
          mime: String(args?.mime ?? ""),
          application: String(args?.application ?? ""),
        };
        return {} as T;

      // The system clipboard, which in the shell is GTK and here is a variable. The tests care
      // about *which calls the window makes* — that a copy reaches the desktop, that a paste
      // asks the desktop first — and a fixture is the only place that can be observed without
      // a compositor and a second file manager.
      case "clipboard_write": {
        const forced = (window as unknown as HarnessWindow).__hcClipboardRefuses;
        if (forced) throw new Error("the compositor refused the selection");
        (window as unknown as HarnessWindow).__hcClipboard = {
          mode: String(args?.mode ?? "copy"),
          paths: (args?.paths as string[]) ?? [],
        };
        return undefined as T;
      }
      case "clipboard_read":
        return ((window as unknown as HarnessWindow).__hcClipboard ?? null) as T;
      case "clipboard_clear":
        (window as unknown as HarnessWindow).__hcClipboard = null;
        return undefined as T;
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
        return {
          connected_ms: 12,
          interactive_ms: 90,
          budget_ms: 300,
          within_budget: true,
          waited_for_daemon_ms: 0,
          // Reported by the shell and read by nothing in the page any more: the renderer turned
          // out not to be what decided the cost of an animation (D59). It is still in the
          // report because the journal answers "which renderer is this process using" with it.
          software_compositing: params.get("compositing") === "software",
        } as T;
      case "mark_interactive":
        return 90 as T;

      // The Agent Inspector. Enough shape for the layout tests to measure a full panel — a
      // memory long enough to threaten a 400 px column, a session with a task, and a refused
      // call — and no more: what the panel *does* with these is asserted in vitest against its
      // own fixture, and this one exists to be laid out.
      case "agent_sessions":
        return {
          sessions: [
            {
              id: "s-1",
              name: "claude",
              task: "tidy a directory whose name is long enough to threaten a narrow column",
              started_at: 1_700_000_000,
              calls: 42,
              refusals: 3,
              memories: 7,
            },
          ],
          open: 1,
          basis: "the 1 most recent agent sessions, newest first; 1 still running",
        } as T;
      case "audit_recent": {
        const refused = args?.refusedOnly === true;
        return {
          entries: Array.from({ length: refused ? 3 : 20 }, (_, i) => ({
            id: i + 1,
            ts: 1_700_000_000 - i * 30,
            actor: "agent:claude",
            tool: refused ? "fs.write" : "brain.context",
            args_json: "{}",
            result_summary: refused
              ? "/home/user/elsewhere/deeply/nested/file.txt is not inside the write allow-list"
              : `${i + 3} items, 1840 of 2048 tokens`,
            allowed: !refused,
          })),
        } as T;
      }
      case "agent_status":
        return {
          enabled: true,
          indexing_paused: false,
          write_allowlist: ["/home/user/Projects/a-directory-with-a-long-name/scratch"],
          require_dry_run: true,
        } as T;
      case "memories":
        // The context panel asks for what is known about one path; the Inspector asks for
        // everything. The fixture answers the narrow question narrowly, so a layout test of the
        // sidebar measures one or two memories rather than thirty.
        if (args?.about) {
          return {
            memories: [
              {
                node_id: 901,
                scope: { kind: "agent", id: "claude" },
                kind: "observation",
                text:
                  "this file is generated on every build, so editing it by hand is lost work " +
                  "the next time anybody runs the pipeline",
                confidence: 0.8,
                usefulness: 0.35,
                helped_count: 1,
                noise_count: 0,
                retrieved_count: 3,
                human_confirmed: true,
                actor: "agent:claude",
                ts: 1_700_000_000,
                salience: 0.4,
                pinned: false,
              },
            ],
            basis: "memories about this path",
          } as T;
        }
        return {
          memories: Array.from({ length: 30 }, (_, i) => ({
            node_id: i + 1,
            scope: i % 3 === 0 ? { kind: "global" } : { kind: "agent", id: "claude" },
            kind: ["fact", "preference", "procedure", "decision", "failure", "observation"][i % 6],
            text:
              `memory ${i}: a sentence long enough to threaten the layout of a narrow column, ` +
              "with no spaces-free run in it that could be blamed instead",
            confidence: 0.5,
            usefulness: i % 4 === 0 ? -0.15 : 0.35,
            helped_count: i % 4 === 0 ? 0 : 1,
            noise_count: i % 4 === 0 ? 2 : 0,
            retrieved_count: i,
            human_confirmed: i % 5 === 0,
            superseded_by: i % 7 === 0 ? 99 : undefined,
            actor: "agent:claude",
            ts: 1_700_000_000 - i * 600,
            salience: 0.2,
            pinned: false,
          })),
          basis: "every memory in the index, in every scope, including superseded and expired ones",
        } as T;
      case "brain_diff":
        return { changes: [], cursor: 1, more: false } as T;
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
  /** What the desktop is holding. A test can put something here as another application would. */
  __hcClipboard?: { mode: string; paths: string[] } | null;
  /** Wayland refuses the selection to an unfocused window; this is how that is rehearsed. */
  __hcClipboardRefuses?: boolean;
  /** Which application the window asked the shell to start, and with what. */
  __hcOpenedWith?: { id: string; paths: string[] };
  /** A remembered choice, as the daemon would have written it into `mimeapps.list`. */
  __hcDefaultSet?: { mime: string; application: string };
  /** How much the daemon still has to index, when a test wants to change that mid-run. */
  __hcPending?: number;
  /** Which device the window asked to mount. */
  __hcMounted?: string;
  /** Which device it asked to unmount, and whether it asked for the power to be cut. */
  __hcEjected?: { device: string; eject: boolean };
};

(window as unknown as HarnessWindow).__hcEmit = (event, payload) => {
  (window as unknown as HarnessWindow).__hcListeners
    ?.get(event)
    ?.forEach((handler) => handler(payload));
};

const root = document.getElementById("root");
if (!root) throw new Error("the harness document has no #root");
// `?at=/a/path` opens there instead of the places screen. Most of the browser tests are about
// browsing a directory, and starting each of them at the places screen would mean every one
// began by navigating out of it.
const at = params.get("at");
createRoot(root).render(<App startPath={at ?? undefined} />);
