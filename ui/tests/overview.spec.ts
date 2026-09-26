/**
 * The start screen, in a real layout engine.
 *
 * Three claims are made here that vitest cannot make. That the window *opens* on the start screen
 * and can be left again — which is a claim about the window and not about a component. That the
 * start screen survives 400 px, where a rail beside a stage has the least room to be wrong in.
 * And that a daemon which does not answer costs the window its numbers and nothing else.
 *
 * The numbers asserted here are the harness fixture's. They are meant to be: this test asks
 * whether a number that arrived is printed, and the one thing it must never do is work out a
 * number of its own (D4).
 */

import { expect, test, type Page } from "@playwright/test";

const WIDTHS = [400, 600, 900, 1400];

/** The fixture's headline, and the only sentence on this screen the window did not write itself. */
const HEADLINE = /38 Aufrufe in den letzten 24 Stunden, drei davon abgelehnt/;

async function openStart(page: Page, query = "") {
  await page.goto(`/harness.html?section=overview${query}`);
  await page.waitForSelector(".hc-ov-headline");
}

test.describe("the start screen", () => {
  test("is where the window opens, and says what it is showing", async ({ page }) => {
    await openStart(page);

    await expect(page.getByRole("region", { name: "Übersicht" })).toBeVisible();
    await expect(page.locator(".hc-ov-headline")).toHaveText(HEADLINE);

    // The daemon's own sentence about what the numbers are and are not. Shown untouched.
    await expect(page.locator(".hc-ov-basis")).toContainText("Gezählt über die ganze Datenbank");

    // Counts, in German grouping, exactly as the daemon sent them.
    await expect(page.locator(".hc-tile-value").first()).toHaveText("12.480");
    await expect(page.getByRole("button", { name: /Dateien/ }).first()).toContainText("12.480");
  });

  test("names the reading it came from", async ({ page }) => {
    await openStart(page);

    await expect(page.locator(".hc-ov-when")).toContainText("Stand");
    await expect(page.locator(".hc-ov .hc-chip", { hasText: "cortexd" })).toContainText("0.4.1");
    // Scoped to the screen: the rail's footer says the same two things about the same daemon, and
    // a locator that could be either would pass without saying which screen was read.
    await expect(page.locator(".hc-ov .hc-chip", { hasText: "Agenten an" })).toBeVisible();
    await expect(page.locator(".hc-ov .hc-chip", { hasText: "Indexierung läuft" })).toBeVisible();
  });

  test("the rail marks the section the window is on", async ({ page }) => {
    await openStart(page);

    const current = page.locator('.hc-rail .hc-nav[aria-current="page"] .hc-nav-label');
    await expect(current).toHaveText("Übersicht");

    // The footer carries the daemon's own state, not a copy of the screen's.
    await expect(page.locator(".hc-rail-foot")).toContainText("2 h 14 min");
  });

  test("leaving for the browser lands in the browser", async ({ page }) => {
    await openStart(page);

    await page.locator('.hc-rail .hc-nav', { hasText: "Dateien" }).click();

    await expect(page.getByRole("region", { name: "Übersicht" })).toHaveCount(0);
    await expect(
      page.locator('.hc-rail .hc-nav[aria-current="page"] .hc-nav-label'),
    ).toHaveText("Dateien");

    // And the browser is live, not merely visible: a search returns rows.
    await page.getByLabel("Suche").fill("query");
    await page.waitForSelector(".hc-row");
  });

  test("typing wins over the start screen", async ({ page }) => {
    await openStart(page);

    // M6's shape: the field is where looking starts, so the first character typed on the start
    // screen leaves it rather than searching behind it.
    await page.getByLabel("Suche").fill("query");

    await expect(page.getByRole("region", { name: "Übersicht" })).toHaveCount(0);
    await page.waitForSelector(".hc-row");
  });

  test("the window switcher re-reads over the window that was picked", async ({ page }) => {
    await openStart(page);

    // `exact` because the tile beside it says the same window in its meta line ("letzte 24
    // Stunden"), and a loose name would match the tile as well.
    const hours24 = page.getByRole("button", { name: "24 Stunden", exact: true });
    const days7 = page.getByRole("button", { name: "7 Tage", exact: true });

    await expect(hours24).toHaveAttribute("aria-pressed", "true");

    await days7.click();

    // The daemon echoes the window it was asked for; the segment follows the answer, not the
    // click — which is what keeps a refused window from being displayed as an accepted one.
    await expect(days7).toHaveAttribute("aria-pressed", "true");
    await expect(hours24).toHaveAttribute("aria-pressed", "false");
  });

  test("a daemon that does not answer costs the numbers and nothing else", async ({ page }) => {
    await page.goto("/harness.html?section=overview&nostats=1");

    await expect(page.locator(".hc-ov-error")).toContainText("Der Daemon antwortet nicht");
    await expect(page.locator(".hc-ov-headline")).toHaveCount(0);

    // The way out is still there, and still works.
    await page.locator('.hc-rail .hc-nav', { hasText: "Dateien" }).click();
    await page.getByLabel("Suche").fill("query");
    await page.waitForSelector(".hc-row");
  });

  for (const width of WIDTHS) {
    test(`lays out without overflow at ${width} px`, async ({ page }) => {
      await page.setViewportSize({ width, height: 700 });
      await openStart(page);

      const overflow = await page.evaluate(() => {
        const el = (selector: string) => document.querySelector(selector) as HTMLElement;
        const box = (selector: string) => {
          const found = el(selector);
          return { w: found.clientWidth, own: found.scrollWidth };
        };
        return {
          body: document.body.scrollWidth - document.body.clientWidth,
          overlay: box(".hc-overlay").own - box(".hc-overlay").w,
          screen: box(".hc-ov").own - box(".hc-ov").w,
          // The rail is a fixed width; what is left for the screen is what matters.
          stage: box(".hc-ov").w,
        };
      });

      expect(overflow.body, `body overflows by ${overflow.body}px at ${width}px`).toBeLessThanOrEqual(
        0,
      );
      expect(overflow.overlay).toBeLessThanOrEqual(0);
      expect(overflow.screen).toBeLessThanOrEqual(0);

      // Neither the rail nor the tiles may take the whole width: at 400 px the screen still has
      // more room than the rail beside it.
      expect(overflow.stage, `only ${overflow.stage}px left for the screen at ${width}px`)
        .toBeGreaterThan(120);
    });

    test(`no tile is clipped at ${width} px`, async ({ page }) => {
      await page.setViewportSize({ width, height: 700 });
      await openStart(page);

      const clipped = await page.evaluate(() =>
        Math.max(
          ...Array.from(document.querySelectorAll(".hc-tile") as NodeListOf<HTMLElement>).map(
            (tile) => tile.scrollWidth - tile.clientWidth,
          ),
        ),
      );

      expect(clipped).toBeLessThanOrEqual(0);
    });
  }
});
