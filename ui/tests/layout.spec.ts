/**
 * M4 §3 and §4, in a real layout engine.
 *
 * §3 asks that the DOM node count stay bounded and be asserted rather than assumed; §4 asks
 * that the layout survive 400 px, which under niri is an ordinary column width and not an edge
 * case. Both are checked against ten thousand results, because the failure they describe only
 * appears at that size.
 */

import { expect, test, type Page } from "@playwright/test";

/** The widths §4 names, plus the minimum the window config allows. */
const WIDTHS = [400, 600, 900, 1400];

async function open(page: import("@playwright/test").Page, width: number, hits = 10_000) {
  await page.setViewportSize({ width, height: 700 });
  await page.goto(`/harness.html?at=/&hits=${hits}`);
  await page.getByLabel("Suche").fill("query");
  await page.waitForSelector(".hc-row");
}

test.describe("the layout holds from 400 px to full width", () => {
  for (const width of WIDTHS) {
    test(`no horizontal overflow at ${width} px`, async ({ page }) => {
      await open(page, width);

      // The page itself must never scroll sideways. Wide content scrolls inside its own pane
      // or elides; it does not push the window.
      const overflow = await page.evaluate(() => ({
        body: document.body.scrollWidth - document.body.clientWidth,
        overlay: (() => {
          const el = document.querySelector(".hc-overlay") as HTMLElement;
          return el.scrollWidth - el.clientWidth;
        })(),
      }));

      expect(overflow.body, `body overflows by ${overflow.body}px at ${width}px`).toBeLessThanOrEqual(0);
      expect(overflow.overlay).toBeLessThanOrEqual(0);
    });

    test(`no row is clipped at ${width} px`, async ({ page }) => {
      await open(page, width);

      // Every rendered row must sit inside the list's box. A row wider than its container is
      // the clipped control §4 forbids, and it is invisible until somebody looks for it.
      const widest = await page.evaluate(() => {
        const list = document.querySelector(".hc-list") as HTMLElement;
        const rows = Array.from(document.querySelectorAll(".hc-row")) as HTMLElement[];
        return Math.max(...rows.map((row) => row.scrollWidth - list.clientWidth));
      });
      expect(widest).toBeLessThanOrEqual(0);
    });

    test(`the search input is usable at ${width} px`, async ({ page }) => {
      await open(page, width);
      const input = page.getByLabel("Suche");
      await expect(input).toBeVisible();

      const box = await input.boundingBox();
      expect(box, "the input has no box at all").not.toBeNull();
      // An input squeezed to nothing by a status line that refused to shrink is the other way
      // §4 fails.
      expect(box!.width).toBeGreaterThan(80);
      expect(box!.x + box!.width).toBeLessThanOrEqual(width);
    });
  }

  test("panes appear as the window gets wider, and only then", async ({ page }) => {
    await open(page, 400);
    expect(await page.locator(".hc-preview").count()).toBe(0);
    expect(await page.locator(".hc-context").count()).toBe(0);

    await page.setViewportSize({ width: 700, height: 700 });
    await expect(page.locator(".hc-preview")).toHaveCount(1);
    expect(await page.locator(".hc-context").count()).toBe(0);

    await page.setViewportSize({ width: 1400, height: 700 });
    await expect(page.locator(".hc-preview")).toHaveCount(1);
    await expect(page.locator(".hc-context")).toHaveCount(1);
  });

  test("shrinking back to 400 px leaves nothing overflowing behind it", async ({ page }) => {
    await open(page, 1400);
    await expect(page.locator(".hc-context")).toHaveCount(1);

    // The resize §4 actually describes: full width down to a narrow column, in one go.
    await page.setViewportSize({ width: 400, height: 700 });
    await page.waitForFunction(() => document.querySelectorAll(".hc-context").length === 0);

    const overflow = await page.evaluate(
      () => document.body.scrollWidth - document.body.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);
  });
});

test.describe("the DOM stays bounded", () => {
  test("ten thousand results render a viewport's worth of rows", async ({ page }) => {
    await open(page, 1400);

    const counts = await page.evaluate(() => ({
      rows: document.querySelectorAll(".hc-row").length,
      total: document.querySelectorAll("*").length,
    }));

    // A 700 px viewport at 28 px per row is 25 rows, plus six of overscan at each edge.
    expect(counts.rows).toBeLessThan(40);
    // The whole document, not just the list: a bounded list inside an unbounded wrapper would
    // pass the first assertion and fail the milestone.
    expect(counts.total).toBeLessThan(400);
  });

  test("the count stays bounded after scrolling deep into the list", async ({ page }) => {
    await open(page, 1400);

    await page.evaluate(() => {
      const list = document.querySelector(".hc-list") as HTMLElement;
      list.scrollTop = 5000 * 28;
      list.dispatchEvent(new Event("scroll"));
    });
    await page.waitForTimeout(100);

    const rows = await page.locator(".hc-row").count();
    expect(rows).toBeLessThan(40);

    // And the rows shown are the ones that belong there, not the first ones re-rendered.
    const firstName = await page.locator(".hc-row-name").first().textContent();
    expect(firstName).not.toBe("module-0.ts");
  });

  test("the scrollable height is the full list, so the scrollbar does not lie", async ({ page }) => {
    await open(page, 1400);
    const height = await page.evaluate(
      () => (document.querySelector(".hc-list > div") as HTMLElement).offsetHeight,
    );
    expect(height).toBe(10_000 * 28);
  });
});

/**
 * A resident window spends most of its life hidden, and a hidden GTK window is not a suspended
 * page: WebKit keeps running the document. Whatever this interface does periodically, it does
 * forever, for nobody — which measured, on this machine, as 99% of a CPU core with nothing on
 * screen.
 */
test.describe("while nobody is looking", () => {
  async function conceal(page: Page) {
    await page.evaluate(() => {
      (window as unknown as { __hcEmit?: (e: string, p?: unknown) => void }).__hcEmit?.(
        "window:visible",
        false,
      );
    });
  }

  test("nothing animates once the window is put away", async ({ page }) => {
    // With work to report, so there is an animation for hiding the window to stop. The idle
    // window does not animate at all any more (D59), and asserting this rule from that state
    // would pass with the rule deleted.
    await page.setViewportSize({ width: 1200, height: 700 });
    await page.goto("/harness.html?at=/&pending=500");
    await page.waitForSelector(".hc-row");

    const breath = page.locator(".hc-breath");
    await expect
      .poll(() => breath.evaluate((el) => getComputedStyle(el).animationName))
      .toBe("hc-breathe");

    await conceal(page);
    await expect(page.locator(".hc-overlay")).toHaveAttribute("data-visible", "no");
    await expect
      .poll(() => breath.evaluate((el) => getComputedStyle(el).animationName))
      .toBe("none");
  });

  test("the daemon is left alone until the window comes back", async ({ page }) => {
    await page.setViewportSize({ width: 1200, height: 700 });
    await page.goto("/harness.html?at=/");
    await page.waitForSelector(".hc-row");
    await conceal(page);

    const asked = () =>
      page.evaluate(
        () =>
          ((window as unknown as { __hcCalls?: { command: string }[] }).__hcCalls ?? []).filter(
            (c) => c.command === "extract_status",
          ).length,
      );
    const before = await asked();

    // Longer than the poll's own interval would be if it were still running.
    await page.waitForTimeout(1200);
    expect(await asked()).toBe(before);

    // And it resumes when the window does — a heartbeat that never came back would be worse
    // than one that costs something.
    await page.evaluate(() => {
      (window as unknown as { __hcEmit?: (e: string, p?: unknown) => void }).__hcEmit?.(
        "window:visible",
        true,
      );
    });
    await expect.poll(asked).toBeGreaterThan(before);
  });
});
