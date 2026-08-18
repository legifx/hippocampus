/**
 * M4 §3: "DOM node count stays bounded — asserted, not assumed."
 *
 * This is the assertion. It is on the pure windowing function rather than on a rendered
 * component, because the property being claimed is arithmetic — a component test would prove
 * the same thing about one viewport size and leave the general case to hope.
 */

import { describe, expect, it } from "vitest";
import { DEFAULT_OVERSCAN, maxRenderedRows, windowFor } from "./useVirtual";

const ROW = 28;
const VIEWPORT = 560;

describe("windowFor", () => {
  it("renders a viewport's worth of rows out of ten thousand", () => {
    const window = windowFor(0, VIEWPORT, { count: 10_000, rowHeight: ROW });
    const rendered = window.end - window.start;

    expect(rendered).toBeLessThanOrEqual(maxRenderedRows(VIEWPORT, ROW));
    expect(rendered).toBeLessThan(40);
    expect(window.totalHeight).toBe(10_000 * ROW);
  });

  it("stays bounded wherever the scroll is", () => {
    const options = { count: 10_000, rowHeight: ROW };
    const bound = maxRenderedRows(VIEWPORT, ROW);

    // Every position from the top to well past the end, including the clamped overscroll a
    // trackpad produces.
    for (let scroll = 0; scroll <= 10_000 * ROW + 2000; scroll += 137) {
      const window = windowFor(scroll, VIEWPORT, options);
      expect(window.end - window.start).toBeLessThanOrEqual(bound);
      expect(window.start).toBeGreaterThanOrEqual(0);
      expect(window.end).toBeLessThanOrEqual(10_000);
    }
  });

  it("covers what is on screen, with overscan on both sides", () => {
    const scroll = 100 * ROW;
    const window = windowFor(scroll, VIEWPORT, { count: 10_000, rowHeight: ROW });

    const firstVisible = 100;
    const lastVisible = firstVisible + Math.ceil(VIEWPORT / ROW);

    // Nothing visible may be missing: that is the failure a virtualiser must never have.
    expect(window.start).toBeLessThanOrEqual(firstVisible);
    expect(window.end).toBeGreaterThanOrEqual(lastVisible);
    expect(window.start).toBe(firstVisible - DEFAULT_OVERSCAN);
  });

  it("puts the rendered rows where the scroll expects them", () => {
    const window = windowFor(50 * ROW, VIEWPORT, { count: 1000, rowHeight: ROW });
    expect(window.padTop).toBe(window.start * ROW);
  });

  it("renders nothing rather than crashing on an empty or unmeasured list", () => {
    expect(windowFor(0, VIEWPORT, { count: 0, rowHeight: ROW }).end).toBe(0);
    // Before the first layout the viewport has no height. A window computed then must be empty,
    // not a full list: rendering ten thousand rows once is exactly the collapse D4 warns about.
    expect(windowFor(0, 0, { count: 10_000, rowHeight: ROW }).end).toBe(0);
  });

  it("holds the bound at the comfortable row height too", () => {
    // Both densities have to pass the ten-thousand-row benchmark (design system §4).
    const window = windowFor(3000, VIEWPORT, { count: 10_000, rowHeight: 34 });
    expect(window.end - window.start).toBeLessThanOrEqual(maxRenderedRows(VIEWPORT, 34));
  });

  it("does not run off the end when the last rows are on screen", () => {
    const count = 10_000;
    const window = windowFor(count * ROW - VIEWPORT, VIEWPORT, { count, rowHeight: ROW });
    expect(window.end).toBe(count);
    expect(window.start).toBeGreaterThan(count - maxRenderedRows(VIEWPORT, ROW));
  });
});
