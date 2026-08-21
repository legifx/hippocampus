/**
 * M4 §3: "DOM node count stays bounded — asserted, not assumed."
 *
 * This is the assertion. It is on the pure windowing function rather than on a rendered
 * component, because the property being claimed is arithmetic — a component test would prove
 * the same thing about one viewport size and leave the general case to hope.
 */

import { describe, expect, it } from "vitest";
import {
  DEFAULT_OVERSCAN,
  LEAD_ROWS,
  MAX_LEAD_ROWS,
  leadFor,
  maxRenderedRows,
  windowFor,
} from "./useVirtual";

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

/**
 * The lead, which is the only thing in here that predicts rather than measures.
 *
 * These are asserted on the pure function rather than through the browser, because both guards
 * are invisible at the speeds the browser tests scroll at: a missing ceiling and a missing jump
 * guard both leave every gap assertion green. A test that cannot fail on the defect it is named
 * after is worse than no test.
 */
describe("leadFor", () => {
  const ROW_H = 28;
  const VIEW = 590;

  it("keeps the old fixed lead when the view is barely moving", () => {
    expect(leadFor(0, ROW_H, VIEW)).toBe(LEAD_ROWS);
    expect(leadFor(100, ROW_H, VIEW)).toBe(LEAD_ROWS);
  });

  it("covers the distance the last frame actually travelled", () => {
    // 420 px is fifteen rows; the sixteenth is the partial row the position lands inside.
    expect(leadFor(420, ROW_H, VIEW)).toBe(16);
    expect(leadFor(420, ROW_H, VIEW) * ROW_H).toBeGreaterThanOrEqual(420);
  });

  it("stops at the ceiling, so the DOM has a bound and not a slope", () => {
    // A tall window, deliberately. The ceiling covers 560 px a frame and the jump guard takes
    // over above the height of the list, so on a short list the guard fires first and the
    // ceiling is never reached — asserting it against `VIEW` would test the guard twice and
    // leave the ceiling free to be deleted. The band where the ceiling is the binding rule is
    // 533 px a frame up to the height of the list, and it widens as the window gets taller.
    const TALL = 1200;
    expect(leadFor(560, ROW_H, TALL)).toBe(MAX_LEAD_ROWS);
    expect(leadFor(1000, ROW_H, TALL)).toBe(MAX_LEAD_ROWS);
    expect(leadFor(TALL, ROW_H, TALL)).toBeLessThanOrEqual(MAX_LEAD_ROWS);
  });

  it("borrows nothing for a jump, because a jump predicts nothing", () => {
    // Further than a viewport in one frame is a scrollbar drag or a programmatic jump: the view
    // lands somewhere unrelated and stops. Sizing a lead from that distance spends rows on
    // motion that is already over.
    expect(leadFor(VIEW + 1, ROW_H, VIEW)).toBe(LEAD_ROWS);
    expect(leadFor(140_000, ROW_H, VIEW)).toBe(LEAD_ROWS);
  });

  it("is what the window actually uses", () => {
    const moving = windowFor(10_000, VIEW, { count: 100_000, rowHeight: ROW_H }, "down", 20);
    const still = windowFor(10_000, VIEW, { count: 100_000, rowHeight: ROW_H }, "still", 20);
    expect(moving.end - moving.start).toBeGreaterThan(still.end - still.start);
    expect(moving.end - moving.start).toBeLessThanOrEqual(maxRenderedRows(VIEW, ROW_H));
  });
});
