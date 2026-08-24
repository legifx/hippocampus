/**
 * M10 step 3 in the window: the places the index makes.
 *
 * Its Definition of Done is a sentence about *use*: "a concept can be opened, its members
 * copied, and its basis line read; the cold view explains itself; and Alt+Left walks back out of
 * all of them." That is what this file asserts, in that order.
 */

import { expect, test, type Page } from "@playwright/test";

async function openAtPlaces(page: Page) {
  await page.setViewportSize({ width: 1200, height: 700 });
  await page.goto("/harness.html");
  await page.waitForSelector(".hc-place");
}

async function calls(page: Page) {
  return (await page.evaluate(
    () =>
      (window as unknown as { __hcCalls?: { command: string; args: Record<string, unknown> }[] })
        .__hcCalls ?? [],
  )) as { command: string; args: Record<string, unknown> }[];
}

test.describe("the places the index makes", () => {
  test("they are a section of their own, with what is in them", async ({ page }) => {
    await openAtPlaces(page);

    const section = page.locator(".hc-places-section", { hasText: "Gedächtnis" });
    const rows = section.locator(".hc-place");
    await expect(rows).toHaveCount(3);
    await expect(rows.nth(0)).toContainText("Woran ich gerade arbeite");
    // A concept says how many files are in it, so nobody opens three to find the one worth
    // opening.
    await expect(rows.nth(1)).toContainText("parser · grammar");
    await expect(rows.nth(1)).toContainText("4 Dateien");
    await expect(rows.nth(2)).toContainText("Vergessen");
  });

  test("a concept opens like a folder, and its rows are ordinary rows", async ({ page }) => {
    await openAtPlaces(page);
    await page.locator(".hc-place", { hasText: "parser · grammar" }).dblclick();
    await page.waitForSelector(".hc-row");

    const opened = (await calls(page)).filter((c) => c.command === "view_open");
    const source = opened[opened.length - 1]!.args.source as { kind: string; label?: string };
    expect(source.kind).toBe("concept");
    expect(source.label).toBe("parser · grammar");

    // Ordinary rows: selectable, and the operations act on them. Ctrl+C is the cheapest proof
    // that a concept's members can be copied.
    await page.locator(".hc-row").first().click();
    await page.keyboard.press("Control+c");
    await page.waitForTimeout(50);
    const copied = (await calls(page)).filter((c) => c.command === "clipboard_write");
    expect(copied).toHaveLength(1);
    expect((copied[0]!.args.paths as string[]).length).toBe(1);
  });

  test("the cold view explains itself, and says nothing was deleted", async ({ page }) => {
    await openAtPlaces(page);
    await page.locator(".hc-place", { hasText: "Vergessen" }).dblclick();
    await page.waitForSelector(".hc-row");

    // The basis line is the feature: a list of files the system "let go of" has to say, in the
    // same breath, that they are still on the disk.
    const basis = page.locator(".hc-basis");
    await expect(basis).toBeVisible();
    await expect(basis).toContainText("Gelöscht ist nichts");
    await expect(basis).toContainText("Metadaten");
  });

  test("the breadcrumb names a place that has no path, and Alt+Left walks back out", async ({
    page,
  }) => {
    await openAtPlaces(page);
    await page.locator(".hc-place", { hasText: "Woran ich gerade arbeite" }).dblclick();
    await page.waitForSelector(".hc-row");

    // One crumb, the name the daemon gave it: a concept is not inside anything, and segments
    // for it would be a hierarchy the window invented.
    await expect(page.locator(".hc-crumb-view")).toHaveText("Woran ich gerade arbeite");

    await page.keyboard.press("Alt+ArrowLeft");
    await page.waitForSelector(".hc-place");
    // Back where we came from: the places screen, not a directory guessed from nothing.
    await expect(page.locator(".hc-places-title").first()).toHaveText("Persönlich");
  });

  test("a place with no directory behind it refuses a paste rather than losing it", async ({
    page,
  }) => {
    await openAtPlaces(page);
    await page.locator(".hc-place", { hasText: "Vergessen" }).dblclick();
    await page.waitForSelector(".hc-row");

    await page.locator(".hc-row").first().click();
    await page.keyboard.press("Control+c");
    await page.keyboard.press("Control+v");
    await page.waitForTimeout(100);

    // Nothing was transferred, and the window says why instead of doing nothing.
    expect((await calls(page)).filter((c) => c.command === "transfer")).toHaveLength(0);
    await expect(page.locator(".hc-banner")).toContainText("kein Ordner");
  });
});
