/**
 * M6's shape: one list, two modes.
 *
 * "Search as an **overlay**, not a separate page — results replace the list live." That is a
 * claim about what happens to the *same* list when somebody types, so it is tested by typing
 * and looking at the list rather than by checking that two components exist.
 */

import { expect, test, type Page } from "@playwright/test";

async function open(page: Page, width = 1200) {
  await page.setViewportSize({ width, height: 700 });
  await page.goto("/harness.html");
  await page.waitForSelector(".hc-row");
}

test.describe("browsing", () => {
  test("an empty query shows a directory rather than an invitation to type", async ({ page }) => {
    await open(page);

    await expect(page.locator(".hc-crumbs")).toBeVisible();
    // The fixture's directory: two folders and three files.
    await expect(page.locator(".hc-row")).toHaveCount(5);
    await expect(page.locator(".hc-row-directory")).toHaveCount(2);
  });

  test("typing replaces the list and clearing brings it back", async ({ page }) => {
    await open(page);
    const crumbs = page.locator(".hc-crumbs");
    await expect(crumbs).toBeVisible();

    await page.getByLabel("Suche").fill("query");
    // The same list, different contents — and the breadcrumb goes, because a result list is
    // not in a directory.
    await expect(crumbs).toHaveCount(0);
    await expect(page.locator(".hc-row").first()).toBeVisible();

    await page.getByLabel("Suche").fill("");
    await expect(crumbs).toBeVisible();
    await expect(page.locator(".hc-row")).toHaveCount(5);
  });

  test("Enter descends into a directory and Backspace comes back up", async ({ page }) => {
    await open(page);

    const first = page.locator(".hc-row").first();
    const name = await first.locator(".hc-row-name").textContent();
    await page.keyboard.press("Enter");

    // The breadcrumb is the evidence: the last segment is the directory just entered.
    const crumb = page.locator('.hc-crumb[aria-current="page"]');
    await expect(crumb).toHaveText(name!.trim());

    await page.keyboard.press("Backspace");
    await expect(page.locator('.hc-crumb[aria-current="page"]')).toHaveText("/");
  });

  test("Backspace does not navigate while there is something to delete", async ({ page }) => {
    await open(page);
    await page.getByLabel("Suche").fill("quer");

    await page.keyboard.press("Backspace");
    await expect(page.getByLabel("Suche")).toHaveValue("que");
    // Still searching, still not somewhere else.
    await expect(page.locator(".hc-crumbs")).toHaveCount(0);
  });

  test("a breadcrumb segment is a way back", async ({ page }) => {
    await open(page);
    await page.keyboard.press("Enter");
    await expect(page.locator('.hc-crumb[aria-current="page"]')).not.toHaveText("/");

    await page.locator(".hc-crumb").first().click();
    await expect(page.locator('.hc-crumb[aria-current="page"]')).toHaveText("/");
  });

  test("Alt+Left goes back through where you have been", async ({ page }) => {
    await open(page);
    await page.keyboard.press("Enter");
    const descended = await page.locator('.hc-crumb[aria-current="page"]').textContent();

    await page.keyboard.press("Backspace");
    await expect(page.locator('.hc-crumb[aria-current="page"]')).toHaveText("/");

    // Back is history, not the parent: it returns to the directory that was left.
    await page.keyboard.press("Alt+ArrowLeft");
    await expect(page.locator('.hc-crumb[aria-current="page"]')).toHaveText(descended!.trim());
  });

  test("the deny-list's hidden entries are counted and not named", async ({ page }) => {
    await open(page);
    // The fixture reports one hidden entry. It must appear as a number and nowhere else.
    await expect(page.locator(".hc-status")).toContainText("+1");
  });

  test("a directory reads as a place, with a mark rather than only a colour", async ({ page }) => {
    await open(page);
    const directories = page.locator(".hc-row-directory");
    await expect(directories).toHaveCount(2);
    // D15: colour is never the only signal.
    await expect(directories.first().locator(".hc-row-dirmark")).toHaveCount(1);
  });

  test("the layout still holds at 400 px while browsing", async ({ page }) => {
    await open(page, 400);
    const overflow = await page.evaluate(
      () => document.body.scrollWidth - document.body.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);
    // The breadcrumb is the new thing that could push the window sideways.
    const crumbs = await page.evaluate(() => {
      const el = document.querySelector(".hc-crumbs") as HTMLElement;
      return el.scrollWidth - el.clientWidth;
    });
    expect(crumbs).toBeLessThanOrEqual(0);
  });
});
