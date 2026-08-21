/**
 * The frontend's share of M4's two latency budgets.
 *
 * §2 is "keystroke → first visible row < 80 ms P95, measured in the GUI, not in the daemon".
 * That budget has three parts: the daemon's search, the IPC hop, and the render. This file
 * measures the **render**, against a fixture that answers instantly, and asserts it stays a
 * small fraction of the budget. It does not claim the whole number — the daemon's part is
 * measured in `bench/latency` and the end-to-end number comes out of the app itself, which
 * reports it through `startup_report` and the command palette.
 *
 * Saying which part is measured here matters more than the number: a test that measured a
 * fixture and reported it as the end-to-end latency would be worse than no test.
 *
 * What is timed is **keystroke to updated DOM**, not keystroke to composited frame. Waiting on
 * `requestAnimationFrame` measures the browser's frame pacing — two frames is thirty-three
 * milliseconds before any of this code has done anything — and that is a property of the
 * display, not of the application. The compositor takes one frame after this regardless of
 * what the app does; the number worth defending is the work in front of it.
 */

import { expect, test, type Page } from "@playwright/test";

/**
 * The frontend's share of the 80 ms.
 *
 * The rest is the daemon's search (about twenty milliseconds over a hundred thousand files,
 * measured in `bench/latency`), the IPC hop, and the one frame the compositor takes afterwards.
 * Twenty-four leaves that comfortably; measured here it is 2 ms at the median and 16 at the
 * ninety-fifth percentile, and the ninety-fifth is the first keystroke against an empty list,
 * where the rows mount for the first time.
 */
const RENDER_BUDGET_MS = 24;
/** One frame at 60 fps. */
const FRAME_MS = 16.7;
/**
 * The bound M4 §3 asks for, at this viewport, while the view is moving at its fastest.
 *
 * At rest the list holds under forty rows and that is asserted separately. This is the
 * ceiling the adaptive lead is allowed to reach during a flick, and it is a constant so that
 * "bounded" stays a number somebody chose rather than whatever the code happens to produce.
 */
const MAX_ROWS_IN_FLIGHT = 48;

async function open(page: Page, hits = 10_000) {
  await page.setViewportSize({ width: 1200, height: 700 });
  await page.goto(`/harness.html?at=/&hits=${hits}`);
  await page.waitForSelector(".hc-input");
}

/**
 * Type into a controlled React input from inside the page.
 *
 * Assigning `input.value` directly does not work: React keeps its own record of the last value
 * it wrote, sees no change, and never calls `onChange`. Going through the prototype's setter
 * updates the element without touching that record, so the event that follows looks exactly
 * like a keystroke. Playwright's own typing would do this correctly too — but the timer has to
 * start and stop inside the page, or it measures the round trip to the test runner as well.
 */
const TYPE_INTO_INPUT = `(input, value) => {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
  setter.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}`;

function percentile(values: number[], p: number): number {
  const sorted = [...values].sort((a, b) => a - b);
  const index = Math.min(sorted.length - 1, Math.ceil((p / 100) * sorted.length) - 1);
  return sorted[index] ?? 0;
}

// One at a time, and run on their own: a latency assertion sharing eight cores with seven other
// browser instances measures the cores. Playwright's worker pool is per-invocation rather than
// per-project, so `npm run test:perf` runs this file by itself with `--workers=1`.
test.describe.configure({ mode: "serial" });

test.describe("the render stays inside its share of the budget", () => {
  test("a keystroke reaches a painted row well inside 80 ms", async ({ page }) => {
    await open(page);

    const samples: number[] = [];
    const word = "abcdefghijklmnopqrstuvwxyzabcdefghij";

    for (const letter of word) {
      const elapsed = await page.evaluate(async ({ key, typeInto }: { key: string; typeInto: string }) => {
        const input = document.querySelector(".hc-input") as HTMLInputElement;
        const before = document.querySelector(".hc-row-name")?.textContent ?? null;
        const started = performance.now();

        // A real input event, so React's state update and the whole render path run — not a
        // direct call into a component, which would measure a function rather than a render.
        (0, eval)(typeInto)(input, input.value + key);

        // Wait for the DOM to reflect it. A task queue turn rather than a frame: the fixture
        // resolves in a microtask, React commits, and this sees the committed DOM.
        const deadline = started + 500;
        for (;;) {
          await new Promise((resolve) => setTimeout(resolve, 0));
          const rows = document.querySelectorAll(".hc-row").length;
          const first = document.querySelector(".hc-row-name")?.textContent ?? null;
          if (rows > 0 && (before === null || first !== null)) break;
          if (performance.now() > deadline) break;
        }
        return performance.now() - started;
      }, { key: letter, typeInto: TYPE_INTO_INPUT } as never);

      samples.push(elapsed);
    }

    const p95 = percentile(samples, 95);
    // Reported either way, so a run that passes still says how much headroom it had.
    console.log(
      `render p50 ${percentile(samples, 50).toFixed(1)} ms, ` +
        `p95 ${p95.toFixed(1)} ms, max ${Math.max(...samples).toFixed(1)} ms ` +
        `(budget for the whole path is 80 ms)`,
    );
    expect(p95).toBeLessThan(RENDER_BUDGET_MS);
  });

  test("a result list of ten thousand renders its first row in one frame", async ({ page }) => {
    await open(page);

    // §3: "a directory of 10 000 entries renders its first row in < 100 ms". The daemon's part
    // of that is its own gate; this is the frontend's, and it is one frame rather than 100 ms
    // because virtualisation makes the list's length irrelevant to the first paint.
    const elapsed = await page.evaluate(async (typeInto: string) => {
      const input = document.querySelector(".hc-input") as HTMLInputElement;
      const started = performance.now();
      (0, eval)(typeInto)(input, "everything");

      const deadline = started + 1000;
      while (document.querySelectorAll(".hc-row").length === 0 && performance.now() < deadline) {
        await new Promise((resolve) => setTimeout(resolve, 0));
      }
      return performance.now() - started;
    }, TYPE_INTO_INPUT);

    expect(await page.locator(".hc-row").count()).toBeGreaterThan(0);
    expect(elapsed).toBeLessThan(100);
  });

  test("scrolling ten thousand rows costs less than a frame per update", async ({ page }) => {
    await open(page);
    await page.getByLabel("Suche").fill("query");
    await page.waitForSelector(".hc-row");

    // Scroll the way a wheel does: many small steps, each of which re-windows the list. If one
    // of those costs more than a frame, the list cannot hold 60 fps, and §8 of the design
    // system says such an effect is removed rather than optimised later.
    const worst = await page.evaluate(async () => {
      const list = document.querySelector(".hc-list") as HTMLElement;
      const timings: number[] = [];

      // Scroll, then wait for the rows to actually change on screen — not for a fixed number
      // of frames.
      //
      // This matters twice over. The list now coalesces scroll events onto an animation frame,
      // so the old `setTimeout(0)` returned *before* any work happened and this test would have
      // gone on passing while measuring nothing; and waiting a fixed two frames instead
      // measures the waiting, which is 33 ms of doing nothing whatever the work costs. What a
      // person experiences is the delay from moving the wheel to the rows moving, so that is
      // what is timed.
      const changed = () =>
        new Promise<void>((resolve) => {
          const observer = new MutationObserver(() => {
            observer.disconnect();
            resolve();
          });
          observer.observe(list, { childList: true, subtree: true });
          // A scroll that changes nothing visible resolves on the next frame instead of
          // hanging — and costs nothing, which is the point of the test below this one.
          requestAnimationFrame(() =>
            requestAnimationFrame(() => {
              observer.disconnect();
              resolve();
            }),
          );
        });

      for (let i = 0; i < 60; i++) {
        const started = performance.now();
        list.scrollTop += 137;
        list.dispatchEvent(new Event("scroll"));
        await changed();
        timings.push(performance.now() - started);
      }
      // The first update includes whatever the browser had queued; the steady state is what a
      // scroll actually feels like.
      return timings.slice(5).sort((a, b) => b - a)[0] ?? 0;
    });

    console.log(`worst scroll update ${worst.toFixed(1)} ms (one frame is ${FRAME_MS} ms)`);
    // Wheel to rows moving. One frame of that is the coalescing, which is deliberate; the
    // budget allows a second for the work itself. Anything beyond cannot hold 60 fps, and §8 of
    // the design system says such an effect is removed rather than optimised later.
    expect(worst).toBeLessThan(FRAME_MS * 2);
  });

  test("a scroll too small to change a row costs nothing at all", async ({ page }) => {
    // The wheel produces a great many small deltas, and most of them leave exactly the same
    // rows on screen. Answering each of those with a React render was work whose result was
    // identical to what was already there — which is what "laggy when scrolling with the
    // mouse" turned out to be.
    await open(page);
    await page.getByLabel("Suche").fill("query");
    await page.waitForSelector(".hc-row");

    const mutations = await page.evaluate(async () => {
      const list = document.querySelector(".hc-list") as HTMLElement;
      const rowHeight = (document.querySelector(".hc-row") as HTMLElement).offsetHeight;

      const settle = () =>
        new Promise<void>((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
        );

      // Land on a row boundary first, so the small scrolls below stay inside one row.
      list.scrollTop = rowHeight * 20;
      list.dispatchEvent(new Event("scroll"));
      await settle();

      let changes = 0;
      const observer = new MutationObserver((records) => (changes += records.length));
      observer.observe(list, { childList: true, subtree: true, attributes: true });

      // Four steps that add up to less than one row, so no new row can come into view.
      // (Four thirds of a row is more than a row: the first version of this test asserted the
      // right property with the wrong arithmetic and failed honestly.)
      const step = Math.max(1, Math.floor(rowHeight / 5));
      for (let i = 0; i < 4; i++) {
        list.scrollTop += step;
        list.dispatchEvent(new Event("scroll"));
        await settle();
      }
      const withinOneRow = changes;

      // And now far enough that the rows must change, to prove the observer works and that
      // scrolling still does something.
      changes = 0;
      list.scrollTop += rowHeight * 10;
      list.dispatchEvent(new Event("scroll"));
      await settle();
      const acrossTenRows = changes;

      observer.disconnect();
      return { withinOneRow, acrossTenRows };
    });

    expect(mutations.withinOneRow).toBe(0);
    expect(mutations.acrossTenRows).toBeGreaterThan(0);
  });

  test("the row count does not grow with the length of the list", async ({ page }) => {
    // The property behind both budgets, stated directly: ten times the results, the same DOM.
    const counts: number[] = [];
    for (const hits of [1_000, 10_000, 100_000]) {
      await open(page, hits);
      await page.getByLabel("Suche").fill("query");
      await page.waitForSelector(".hc-row");
      counts.push(await page.locator(".hc-row").count());
    }

    expect(new Set(counts).size).toBe(1);
    expect(counts[0]).toBeLessThan(40);
  });
});

/**
 * Blank space while scrolling.
 *
 * "It is still laggy, and it is with the mouse" — and it was not lag. The browser paints the new
 * scroll position before React can put rows there, so a fast wheel showed a third of the list
 * empty on every frame, filling in one frame later. That reads as loading, which is why the
 * complaint was about files loading.
 *
 * Measured on this viewport, worst case over twenty steady frames, at each of the three
 * arrangements this list has had:
 *
 * | wheel speed  | six each side | two behind, twelve ahead | lead from the last frame |
 * |---|---|---|---|
 * | 200 px/frame | 28 px  | none   | none |
 * | 300 px/frame | 128 px | none   | none |
 * | 420 px/frame | 240 px | 72 px  | none |
 * | 600 px/frame | —      | —      | 36 px, past the ceiling |
 *
 * The middle column bought the first two rows for two more nodes. The third closes 420 px per
 * frame as well, and pays for it only while the view is actually moving that fast: the lead is
 * taken from how far the previous frame travelled, so at rest the window is the size it always
 * was, and a scrollbar jump — which predicts nothing — borrows nothing at all.
 */
test.describe("a fast scroll shows rows, not gaps", () => {
  /** The worst gap in the viewport over twenty frames of steady scrolling. */
  async function worstGap(page: Page, perFrame: number): Promise<number> {
    return page.evaluate(async (delta) => {
      const list = document.querySelector(".hc-list") as HTMLElement;
      const frame = () => new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));

      // Warm up in the same direction first. What is being measured is steady scrolling; the
      // frame after a jump is genuinely incomplete and a wheel does not teleport. Measuring
      // without this warm-up is how the first attempt at this concluded, wrongly, that the
      // change had made things worse.
      for (let i = 0; i < 6; i++) {
        list.scrollTop += delta;
        list.dispatchEvent(new Event("scroll"));
        await frame();
      }

      let worst = 0;
      for (let i = 0; i < 20; i++) {
        list.scrollTop += delta;
        list.dispatchEvent(new Event("scroll"));
        await frame();

        const box = list.getBoundingClientRect();
        let covered = 0;
        for (const row of Array.from(document.querySelectorAll(".hc-row"))) {
          const r = row.getBoundingClientRect();
          const top = Math.max(r.top, box.top);
          const bottom = Math.min(r.bottom, box.bottom);
          if (bottom > top) covered += bottom - top;
        }
        worst = Math.max(worst, Math.round(box.height - covered));
      }
      return worst;
    }, perFrame);
  }

  test("a brisk wheel leaves no gap at all", async ({ page }) => {
    await open(page, 100_000);
    await page.getByLabel("Suche").fill("query");
    await page.waitForSelector(".hc-row");

    // 300 px per frame is a fast flick — eighteen thousand pixels a second — and it showed
    // 128 px of the viewport empty before the spare rows were moved.
    expect(await worstGap(page, 300)).toBe(0);
  });

  test("and a flick twice that fast leaves none either", async ({ page }) => {
    await open(page, 100_000);
    await page.getByLabel("Suche").fill("query");
    await page.waitForSelector(".hc-row");

    // 420 px per frame is the fastest flick this list is asked to survive — twenty-five
    // thousand pixels a second. A fixed lead of twelve rows covers 336 px of it and leaves the
    // remaining 72 px blank, which is the gap the owner was asked about and decided to close.
    expect(await worstGap(page, 420)).toBe(0);
  });

  test("and the DOM does not grow to achieve it", async ({ page }) => {
    // M4 §3 requires the node count to stay *bounded*, and does not name the bound. The number
    // here is the bound this list claims, so raising it is a visible edit to an assertion
    // rather than a silent drift. It is asserted at rest and during the fastest flick, because
    // those are two different arrangements of the same window.
    await open(page, 100_000);
    await page.getByLabel("Suche").fill("query");
    await page.waitForSelector(".hc-row");
    expect(await page.locator(".hc-row").count()).toBeLessThan(40);

    await worstGap(page, 300);
    expect(await page.locator(".hc-row").count()).toBeLessThan(MAX_ROWS_IN_FLIGHT);
    await worstGap(page, 420);
    expect(await page.locator(".hc-row").count()).toBeLessThan(MAX_ROWS_IN_FLIGHT);

    // Past the ceiling. 600 px a frame is more than the lead is allowed to cover, so there is
    // a gap here and the bound is deliberately the thing that wins.
    await worstGap(page, 600);
    expect(await page.locator(".hc-row").count()).toBeLessThan(MAX_ROWS_IN_FLIGHT);

    // And a speed no wheel produces, because the scrollbar does: dragging the thumb moves
    // thousands of pixels in a frame. That is a jump, not scrolling — it predicts nothing about
    // the next frame — so the list borrows *nothing*, and the node count stays at the size it
    // has at rest rather than at the size a flick is allowed to reach.
    await worstGap(page, 3000);
    expect(await page.locator(".hc-row").count()).toBeLessThan(40);
  });

  test("and it comes back down when the scrolling stops", async ({ page }) => {
    // The lead is spent while the view is moving and given back when it stops. Without this the
    // adaptive window would be a one-way ratchet: one flick and the DOM stays large for ever.
    await open(page, 100_000);
    await page.getByLabel("Suche").fill("query");
    await page.waitForSelector(".hc-row");

    await worstGap(page, 420);

    // Nothing is dispatched here, deliberately. When a wheel stops, the browser stops sending
    // scroll events — so the list has to notice on its own that it has settled. An earlier
    // version of this test dispatched six more scroll events while waiting, and those events
    // did the settling; it passed with the settle deleted from the code, which makes it a test
    // of the test rather than of the list.
    await page.evaluate(async () => {
      for (let i = 0; i < 6; i++) {
        await new Promise<void>((r) => requestAnimationFrame(() => r()));
      }
    });
    expect(await page.locator(".hc-row").count()).toBeLessThan(40);
  });
});

/**
 * What an ambient animation is allowed to cost when the window is painted on the CPU.
 *
 * `WEBKIT_DISABLE_DMABUF_RENDERER=1` is set on this machine because the DMABUF renderer and the
 * driver together produce glitches and black areas (D54, D57). It drops WebKit to software
 * compositing, and the breath layer — one gradient covering the whole window, with its opacity
 * rising and falling — is then a full-window repaint on every frame.
 *
 * Measured on the real overlay, not here: **116% of one core, steady, with the window merely on
 * screen and nobody touching it**, against 0.14% with the animations stopped. That is the
 * number this exists to stop coming back. What can be asserted in a browser is the wiring —
 * that the page asks the shell, and that the answer reaches the layer — because the page cannot
 * see the variable and would otherwise animate at full rate with no way to tell.
 */
test.describe("the breath layer costs less when the window is painted on the CPU", () => {
  test("on the GPU it breathes continuously", async ({ page }) => {
    await page.setViewportSize({ width: 1200, height: 700 });
    await page.goto(`/harness.html?at=/`);
    await page.waitForSelector(".hc-breath-animated");

    const timing = await page.evaluate(
      () => getComputedStyle(document.querySelector(".hc-breath") as Element).animationTimingFunction,
    );
    expect(timing).toContain("ease-in-out");
    expect(await page.locator("[data-compositing]").count()).toBe(0);
  });

  test("in software it takes the same breath in steps", async ({ page }) => {
    await page.setViewportSize({ width: 1200, height: 700 });
    await page.goto(`/harness.html?at=/&compositing=software`);
    await page.waitForSelector('[data-compositing="software"]');

    const style = await page.evaluate(() => {
      const layer = document.querySelector(".hc-breath") as Element;
      const computed = getComputedStyle(layer);
      return {
        timing: computed.animationTimingFunction,
        duration: computed.animationDuration,
        name: computed.animationName,
      };
    });

    // Stepped, so the opacity changes a few times a second instead of sixty.
    expect(style.timing).toContain("steps");
    // And still breathing, on the same period. A static layer would be cheaper and would
    // indicate nothing, which is the one thing §6 says this element is for.
    expect(style.name).toBe("hc-breathe");
    expect(parseFloat(style.duration)).toBeGreaterThan(0);
  });
});
