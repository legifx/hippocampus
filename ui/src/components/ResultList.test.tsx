/**
 * The rendered half of M4 §3: ten thousand results, and the DOM holds a viewport's worth.
 *
 * The arithmetic is asserted in `useVirtual.test.ts`; this asserts that the component actually
 * uses it, which is the part that regresses when somebody adds a feature to a row.
 */

import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ResultList } from "./ResultList";
import { rowFromView, type Row } from "../row";
import type { ViewRow } from "../ipc";

const ROW = 28;
const VIEWPORT = 560;

/** The rows a search of `count` results would produce. */
function sampleRows(count: number): Row[] {
  // A query view: rows come from everywhere, so each one says where it lives.
  return hits(count).map((row) => rowFromView(row, null));
}

function hits(count: number): ViewRow[] {
  return Array.from({ length: count }, (_, i) => ({
    node_id: i + 1,
    path: `/home/user/Projects/thing-${i}/notes-${i}.md`,
    kind: "file" as const,
    size: 1024 * (i + 1),
    mtime: 1_700_000_000 - i * 60,
    indexed: true,
    zone_id: i % 3 === 0 ? 1 : null,
    tier: i % 97 === 0 ? "cold" : "stm",
    score: { path_rank: 1, lexical_rank: null, vector_rank: 2, fused: 0.5, salience_boost: 1, rerank: null, final_score: 0.5 },
    snippet: i % 2 === 0 ? `a snippet for ${i}` : null,
  }));
}

/** happy-dom reports zero for every measurement, so the viewport is stated explicitly. */
function withViewport(height: number) {
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get() {
      return this.classList?.contains("hc-list") ? height : 0;
    },
  });
}

describe("ResultList", () => {
  it("renders a bounded number of rows for ten thousand results", () => {
    withViewport(VIEWPORT);
    const { container } = render(
      <ResultList
        rows={sampleRows(10_000)}
        cursor={0}
        selected={new Set([0])}
        onSelect={() => {}}
        onActivate={() => {}}
        rowHeight={ROW}
        visible={true}
        zoneHues={new Map([[1, 210]])}
        emptyMessage="nichts"
        selectedPaths={[]}
      />,
    );

    const rows = container.querySelectorAll(".hc-row");
    expect(rows.length).toBeGreaterThan(0);
    expect(rows.length).toBeLessThan(40);

    // And the whole list is still scrollable to its true length, or the scrollbar would lie.
    const spacer = container.querySelector(".hc-list > div") as HTMLElement;
    expect(spacer.style.height).toBe(`${10_000 * ROW}px`);
  });

  it("marks exactly one row as selected, and it is the one it was told", () => {
    withViewport(VIEWPORT);
    const { container } = render(
      <ResultList
        rows={sampleRows(50)}
        cursor={3}
        selected={new Set([3])}
        onSelect={() => {}}
        onActivate={() => {}}
        rowHeight={ROW}
        visible={true}
        zoneHues={new Map()}
        emptyMessage="nichts"
        selectedPaths={[]}
      />,
    );

    const selected = container.querySelectorAll('[aria-selected="true"]');
    expect(selected.length).toBe(1);
    expect(selected[0]?.id).toBe("hc-row-3");
  });

  it("keeps the order the daemon gave it", () => {
    withViewport(VIEWPORT);
    // D4's second condition: the frontend does not sort. The input here is deliberately not in
    // any order a sort would produce, and it must come out unchanged.
    const scrambled = [sampleRows(3)[2]!, sampleRows(3)[0]!, sampleRows(3)[1]!];
    const { container } = render(
      <ResultList
        rows={scrambled}
        cursor={0}
        selected={new Set([0])}
        onSelect={() => {}}
        onActivate={() => {}}
        rowHeight={ROW}
        visible={true}
        zoneHues={new Map()}
        emptyMessage="nichts"
        selectedPaths={[]}
      />,
    );

    const names = Array.from(container.querySelectorAll(".hc-row-name")).map((n) => n.textContent);
    expect(names).toEqual(["notes-2.md", "notes-0.md", "notes-1.md"]);
  });

  it("marks a cold result as brightness rather than as a word", () => {
    withViewport(VIEWPORT);
    const cold = sampleRows(1).map((r) => ({ ...r, tier: "cold" }));
    const { container } = render(
      <ResultList
        rows={cold}
        cursor={0}
        selected={new Set([0])}
        onSelect={() => {}}
        onActivate={() => {}}
        rowHeight={ROW}
        visible={true}
        zoneHues={new Map()}
        emptyMessage="nichts"
        selectedPaths={[]}
      />,
    );
    // M10 §4.4: tier is brightness everywhere except the view that is about tiers. The badge
    // this test used to assert said "cold" on one row in fifty and said nothing about the other
    // forty-nine; the encoding now carries every tier, and it is the one the 3D view uses.
    expect(screen.queryByText("cold")).toBeNull();
    const row = container.querySelector(".hc-row");
    expect(row?.getAttribute("data-tier")).toBe("cold");
  });

  it("says so when there is nothing, rather than showing an empty box", () => {
    withViewport(VIEWPORT);
    render(
      <ResultList
        rows={[]}
        cursor={0}
        selected={new Set([0])}
        onSelect={() => {}}
        onActivate={() => {}}
        rowHeight={ROW}
        visible={true}
        zoneHues={new Map()}
        emptyMessage="Nichts gefunden"
        selectedPaths={[]}
      />,
    );
    expect(screen.getByText("Nichts gefunden")).toBeTruthy();
  });

  it("hands its zone to the stylesheet rather than painting itself", () => {
    withViewport(VIEWPORT);
    const { container } = render(
      <ResultList
        rows={sampleRows(3)}
        cursor={0}
        selected={new Set([0])}
        onSelect={() => {}}
        onActivate={() => {}}
        rowHeight={ROW}
        visible={true}
        zoneHues={new Map([[1, 210]])}
        emptyMessage="nichts"
        selectedPaths={[]}
      />,
    );

    // `hits` puts every third row in zone 1, so row 1 is outside every zone.
    const zoned = container.querySelectorAll(".hc-row")[0] as HTMLElement;
    const plain = container.querySelectorAll(".hc-row")[1] as HTMLElement;
    const zonedStyle = zoned.getAttribute("style") ?? "";

    expect(zonedStyle).toContain("--hc-zone: hsl(210, 58%, 62%)");
    expect(zonedStyle).toContain("--hc-zone-border: hsla(210, 58%, 62%, 0.85)");
    expect(zonedStyle).toContain("--hc-zone-soft: hsla(210, 58%, 62%, 0.06)");

    // The regression this exists for: the row used to write `background` and `border-left-color`
    // inline, and an inline declaration beats every rule in every stylesheet — so the selected
    // row's fill and its bar, both of which live in `app.css`, never painted. A custom property
    // travels the same way but cannot win anything: this window sets none of these three.

    expect(zonedStyle).not.toContain("background");
    expect(zonedStyle).not.toContain("border-left-color");
    expect(plain.getAttribute("style") ?? "").not.toContain("--hc-zone");
  });

  it("selects on mousedown without stealing focus from the input", () => {
    withViewport(VIEWPORT);
    const onSelect = vi.fn();
    const { container } = render(
      <ResultList
        rows={sampleRows(5)}
        cursor={0}
        selected={new Set([0])}
        onSelect={onSelect}
        onActivate={() => {}}
        rowHeight={ROW}
        visible={true}
        zoneHues={new Map()}
        emptyMessage="nichts"
        selectedPaths={[]}
      />,
    );

    const row = container.querySelectorAll(".hc-row")[2] as HTMLElement;
    const event = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
    row.dispatchEvent(event);

    // The modifiers come with it: the same handler serves a plain click, a shift-extension
    // and a ctrl-toggle, and the row is the only thing that knows which was held.
    expect(onSelect).toHaveBeenCalledWith(2, { shift: false, ctrl: false });
    // Preventing the default is what keeps the caret in the search input; without it, typing
    // stops working the moment somebody clicks a row once.
    expect(event.defaultPrevented).toBe(true);
  });

  it("draws the kit's empty state, with the glyph it was handed", () => {
    withViewport(VIEWPORT);
    const { container } = render(
      <ResultList
        rows={[]}
        cursor={0}
        selected={new Set()}
        onSelect={() => {}}
        onActivate={() => {}}
        rowHeight={ROW}
        visible={true}
        zoneHues={new Map()}
        emptyMessage="Nichts gefunden"
        emptyIcon={<span data-testid="glyph" />}
        selectedPaths={[]}
      />,
    );

    // The kit's `EmptyState`: a circle, a glyph inside it, the sentence under it. The circle is the
    // element that carries the kit's material classes, so the test asks for both.
    const empty = container.querySelector(".hc-empty") as HTMLElement;
    const mark = empty.querySelector(".hc-empty-mark") as HTMLElement;
    expect(mark.classList.contains("glass")).toBe(true);
    expect(mark.classList.contains("glass-sheen")).toBe(true);
    expect(mark.querySelector("[data-testid='glyph']")).not.toBeNull();
    // And the sentence is still the element's text, because that is what a screen reader gets.
    expect(empty.textContent).toBe("Nichts gefunden");
  });

  it("leaves the circle off when there is no glyph for it", () => {
    withViewport(VIEWPORT);
    const { container } = render(
      <ResultList
        rows={[]}
        cursor={0}
        selected={new Set()}
        onSelect={() => {}}
        onActivate={() => {}}
        rowHeight={ROW}
        visible={true}
        zoneHues={new Map()}
        emptyMessage="…"
        selectedPaths={[]}
      />,
    );

    // A list that is loading, or a share that did not answer: not empty states, and a glass circle
    // over either of them would dress a failure up as an empty folder.
    const empty = container.querySelector(".hc-empty") as HTMLElement;
    expect(empty.querySelector(".hc-empty-mark")).toBeNull();
    expect(empty.textContent).toBe("…");
  });
});
