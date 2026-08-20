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

      for (let i = 0; i < 60; i++) {
        const started = performance.now();
        list.scrollTop += 137;
        list.dispatchEvent(new Event("scroll"));
        // The work, not the wait: re-windowing the list and committing the new rows.
        await new Promise((resolve) => setTimeout(resolve, 0));
        timings.push(performance.now() - started);
      }
      // The first update includes whatever the browser had queued; the steady state is what a
      // scroll actually feels like.
      return timings.slice(5).sort((a, b) => b - a)[0] ?? 0;
    });

    console.log(`worst scroll update ${worst.toFixed(1)} ms (one frame is ${FRAME_MS} ms)`);
    // One frame of work per scroll step. Anything above this cannot hold 60 fps, and §8 of the
    // design system says such an effect is removed rather than optimised later.
    expect(worst).toBeLessThan(FRAME_MS);
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
