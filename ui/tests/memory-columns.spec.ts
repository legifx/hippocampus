/**
 * M10 step 2, in the window: what a row carries from the brain, and how it shows.
 *
 * The daemon's tests assert that the columns arrive. What only a browser can answer is the
 * encoding: **tier is brightness, not a badge** (§4.4) — the same encoding the three-dimensional
 * view uses, so somebody who has looked at one can read the other — and a row that somebody
 * pinned is the exception to it.
 */

import { expect, test, type Page } from "@playwright/test";

async function open(page: Page, at = "/") {
  await page.setViewportSize({ width: 1200, height: 700 });
  await page.goto(`/harness.html?at=${at}`);
  await page.waitForSelector(".hc-row");
}

/** The rendered opacity of a row's text, which is what "dimmer" means here. */
async function brightness(page: Page, name: string): Promise<number> {
  return page
    .locator(".hc-row", { hasText: name })
    .first()
    .locator(".hc-row-name")
    .evaluate((el) => Number(getComputedStyle(el).opacity));
}

test.describe("the memory on a row", () => {
  test("a cold row is dimmer, and nothing says the word cold", async ({ page }) => {
    await open(page);

    const cold = await brightness(page, "kurz.txt");
    const ordinary = await brightness(page, "notes-with-a-long");
    expect(cold).toBeLessThan(ordinary);

    // No badge, no icon, no column — the encoding is the brightness and only the brightness.
    await expect(page.locator(".hc-tier")).toHaveCount(0);
    await expect(page.locator(".hc-row", { hasText: "kurz.txt" }).first()).not.toContainText(
      "cold",
    );
  });

  test("a row somebody pinned does not fade", async ({ page }) => {
    await open(page);
    // `bild.png` is pinned in the fixture and in the working tier: held against forgetting, and
    // an interface that dimmed it would be disagreeing with the person who held it.
    expect(await brightness(page, "bild.png")).toBe(1);
  });

  test("the rows still carry what stat says as well as what the brain says", async ({ page }) => {
    await open(page);
    const row = page.locator(".hc-row", { hasText: "kurz.txt" }).first();
    // The point of one row shape: the memory arrived without costing the permissions.
    await expect(row).toContainText("-rw-r--r--");
    await expect(row).toContainText("12 B");
  });

  test("salience is offered as an order, and asking for it asks the daemon for it", async ({
    page,
  }) => {
    await open(page);
    await page.keyboard.press("Control+k");
    const command = page.getByText("Sortieren: Salienz", { exact: true });
    await expect(command).toBeVisible();
    await command.click();

    await page.waitForTimeout(150);
    const asked = (await page.evaluate(
      () =>
        (window as unknown as { __hcCalls?: { command: string; args: Record<string, unknown> }[] })
          .__hcCalls ?? [],
    )) as { command: string; args: Record<string, unknown> }[];

    const opened = asked.filter((c) => c.command === "view_open");
    // The frontend never sorts (D4): it asks for an order and renders what comes back.
    expect(opened[opened.length - 1]!.args.order).toBe("salience");
  });
});
