/**
 * M6's thumbnails, in a real layout engine.
 *
 * The unit tests cover which calls the list makes. What only a browser can answer is the part
 * that would make this feature unshippable: whether a picture in a row changes the row. The
 * virtualiser is told the row height rather than measuring it, so a row that grew by four pixels
 * would put every scroll position slightly out — visibly, and worse the further down the list.
 */

import { expect, test, type Page } from "@playwright/test";

type Recorded = { command: string; args: Record<string, unknown> };

const ROW_COMPACT = 28;

async function open(page: Page, width = 1200) {
  await page.setViewportSize({ width, height: 700 });
  await page.goto("/harness.html?at=/zweiter-ordner");
  await page.waitForSelector(".hc-row");
}

async function calls(page: Page): Promise<Recorded[]> {
  return page.evaluate(
    () => (window as unknown as { __hcCalls?: Recorded[] }).__hcCalls ?? [],
  ) as Promise<Recorded[]>;
}

test.describe("a picture in the row", () => {
  test("the cached one appears, and the uncached one is rendered and appears too", async ({
    page,
  }) => {
    await open(page);

    // bild.png is in the fixture's cache; foto.jpg is not and has to be rendered.
    const cached = page.locator(".hc-row", { hasText: "bild.png" }).locator(".hc-row-thumb");
    const rendered = page.locator(".hc-row", { hasText: "foto.jpg" }).locator(".hc-row-thumb");
    await expect(cached).toBeVisible();
    await expect(rendered).toBeVisible();

    const asked = await calls(page);
    // One lookup for the viewport, and exactly one render — for the one row that needed it.
    expect(asked.filter((c) => c.command === "thumbnails")).toHaveLength(1);
    const rendering = asked.filter((c) => c.command === "preview");
    expect(rendering).toHaveLength(1);
    expect(String(rendering[0]!.args.path)).toContain("foto.jpg");
  });

  test("a row with a picture is exactly as tall as one without", async ({ page }) => {
    await open(page);
    await expect(
      page.locator(".hc-row", { hasText: "bild.png" }).locator(".hc-row-thumb"),
    ).toBeVisible();

    const withPicture = await page.locator(".hc-row", { hasText: "bild.png" }).boundingBox();
    const without = await page.locator(".hc-row", { hasText: "notes-with-a-long" }).boundingBox();

    expect(withPicture?.height).toBe(ROW_COMPACT);
    expect(without?.height).toBe(ROW_COMPACT);
  });

  test("nothing overflows at 400 px with pictures in the rows", async ({ page }) => {
    await open(page, 1200);
    await expect(
      page.locator(".hc-row", { hasText: "bild.png" }).locator(".hc-row-thumb"),
    ).toBeVisible();
    // Narrowed afterwards, so the row has to survive being squeezed rather than being built
    // narrow — the case M4 §4 is actually about.
    await page.setViewportSize({ width: 400, height: 700 });
    await page.waitForTimeout(150);

    const overflow = await page.evaluate(() => ({
      body: document.body.scrollWidth - document.body.clientWidth,
      worst: Math.max(
        0,
        ...Array.from(document.querySelectorAll(".hc-row")).map(
          (el) => (el as HTMLElement).scrollWidth - (el as HTMLElement).clientWidth,
        ),
      ),
    }));
    expect(overflow.body).toBeLessThanOrEqual(0);
    expect(overflow.worst).toBeLessThanOrEqual(0);
  });

  test("browsing away and back does not ask about the same rows again", async ({ page }) => {
    await open(page);
    await expect(
      page.locator(".hc-row", { hasText: "bild.png" }).locator(".hc-row-thumb"),
    ).toBeVisible();
    const first = (await calls(page)).filter((c) => c.command === "thumbnails").length;

    // Up one level and back down: the same rows, freshly rendered components.
    await page.keyboard.press("Backspace");
    await page.waitForTimeout(200);
    await page.locator(".hc-row", { hasText: "zweiter-ordner" }).first().click();
    await page.keyboard.press("Enter");
    await expect(
      page.locator(".hc-row", { hasText: "bild.png" }).locator(".hc-row-thumb"),
    ).toBeVisible();
    await page.waitForTimeout(300);

    const asked = (await calls(page)).filter((c) => c.command === "thumbnails");
    // The root listing has rows of its own, so one more lookup is expected — what must not
    // happen is a lookup for rows this window has already been told about.
    const repeated = asked
      .slice(first)
      .flatMap((c) => (c.args.paths as string[]) ?? [])
      .filter((path) => path.endsWith("bild.png") || path.endsWith("kurz.txt"));
    expect(repeated.filter((p) => p.startsWith("/zweiter-ordner"))).toHaveLength(0);
  });
});
