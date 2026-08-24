/**
 * The two questions the list asks about pictures, and the one it must never ask twice.
 *
 * The expensive failure here is not a missing thumbnail — it is a list that asks about the same
 * two hundred rows every time somebody scrolls back over them, in a process that answers one
 * request at a time and is also serving the preview pane.
 */

import { render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ResultList } from "../components/ResultList";
import { setTransport } from "../ipc";
import type { Row } from "../row";

const ROW = 28;

/** happy-dom reports zero for every measurement, so the viewport is stated explicitly. */
function withViewport(height: number) {
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get() {
      return this.classList?.contains("hc-list") ? height : 0;
    },
  });
}

function row(name: string, kind: "file" | "dir" = "file"): Row {
  return {
    key: `/${name}`,
    path: `/${name}`,
    name,
    dir: "",
    isDir: kind === "dir",
    size: 10,
    mtime: 1_700_000_000,
    snippet: null,
    zoneId: null,
    indexed: true,
    tier: null,
    mode: null,
    owner: null,
    linkTarget: null,
    broken: false,
    nodeId: null,
    score: null,
    salience: null,
    degree: null,
    lastEpisode: null,
    pinned: false,
  };
}

/**
 * A transport that answers like the previewer and counts what it was asked.
 *
 * `cached` names the paths the shared cache already holds; everything else is either
 * renderable — which is a second, more expensive call — or not worth asking about again.
 */
function transport(cached: string[], renderable: string[] = []) {
  const asked: { command: string; paths: string[] }[] = [];
  setTransport({
    async invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
      if (command === "thumbnails") {
        const paths = (args?.paths as string[]) ?? [];
        asked.push({ command, paths });
        return {
          found: paths
            .filter((p) => cached.includes(p))
            .map((p) => ({ path: p, png: `${p}.png`, width: 128, height: 128, size: "normal" })),
          missing: paths
            .filter((p) => renderable.includes(p))
            .map((p) => ({ path: p, kind: "image" })),
          skipped: paths.filter((p) => !cached.includes(p) && !renderable.includes(p)),
        } as T;
      }
      if (command === "preview") {
        const path = String(args?.path ?? "");
        asked.push({ command, paths: [path] });
        return { path, png: `${path}.rendered.png`, width: 128, height: 128 } as T;
      }
      return null as T;
    },
    async listen() {
      return () => {};
    },
    assetUrl: (path: string) => `asset://${path}`,
  });
  return asked;
}

function list(rows: Row[], visible = true) {
  withViewport(560);
  return render(
    <ResultList
      rows={rows}
      cursor={0}
      selected={new Set([0])}
      onSelect={() => {}}
      onActivate={() => {}}
      rowHeight={ROW}
      visible={visible}
      zoneHues={new Map()}
      emptyMessage="nichts"
      selectedPaths={[]}
    />,
  );
}

afterEach(() => vi.useRealTimers());

describe("thumbnails in the list", () => {
  it("shows the cached one and asks about the whole viewport in one call", async () => {
    const asked = transport(["/bild.png"]);
    const { container } = list([row("bild.png"), row("notes.md"), row("ordner", "dir")]);

    await waitFor(() => expect(container.querySelector(".hc-row-thumb")).not.toBeNull());
    const image = container.querySelector(".hc-row-thumb") as HTMLImageElement;
    expect(image.getAttribute("src")).toBe("asset:///bild.png.png");

    const lookups = asked.filter((a) => a.command === "thumbnails");
    expect(lookups).toHaveLength(1);
    // A directory has nothing to render and is never asked about.
    expect(lookups[0]!.paths).toEqual(["/bild.png", "/notes.md"]);
  });

  it("renders a miss, and does it through the call that costs something", async () => {
    const asked = transport([], ["/foto.jpg"]);
    const { container } = list([row("foto.jpg")]);

    await waitFor(() => expect(container.querySelector(".hc-row-thumb")).not.toBeNull());
    const image = container.querySelector(".hc-row-thumb") as HTMLImageElement;
    expect(image.getAttribute("src")).toBe("asset:///foto.jpg.rendered.png");
    expect(asked.filter((a) => a.command === "preview")).toHaveLength(1);
  });

  it("never asks a second time about a row that has no picture", async () => {
    const asked = transport(["/bild.png"]);
    const { rerender, container } = list([row("bild.png"), row("notes.md")]);
    await waitFor(() => expect(container.querySelector(".hc-row-thumb")).not.toBeNull());

    // The same rows again, as a re-render or a scroll back would produce.
    for (let i = 0; i < 3; i += 1) {
      rerender(
        <ResultList
          rows={[row("bild.png"), row("notes.md")]}
          cursor={i}
          selected={new Set([i])}
          onSelect={() => {}}
          onActivate={() => {}}
          rowHeight={ROW}
          visible={true}
          zoneHues={new Map()}
          emptyMessage="nichts"
          selectedPaths={[]}
        />,
      );
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
    expect(asked.filter((a) => a.command === "thumbnails")).toHaveLength(1);
  });

  it("asks nothing at all while nobody can see the window", async () => {
    const asked = transport(["/bild.png"]);
    list([row("bild.png")], false);

    await new Promise((resolve) => setTimeout(resolve, 250));
    expect(asked).toHaveLength(0);
  });
});
