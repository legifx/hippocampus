/**
 * The owner's own example, as a gate: "ist jetzt content.png oder content(1).png das richtige?"
 *
 * Three reviews on 2026-09-05 agreed on one thing about the human surface — every piece needed
 * to tell two same-named files apart existed, and nothing asserted that they do. A thing that is
 * not a test drifts. This is the test: the rows must differ visibly, the preview must follow the
 * cursor to the right one, selecting both must put them side by side with the differences
 * marked, and Quick Look must ask for more pixels than the pane does.
 */

import { expect, test, type Page } from "@playwright/test";

type Recorded = { command: string; args: Record<string, unknown> };

async function calls(page: Page): Promise<Recorded[]> {
  return page.evaluate(
    () => (window as unknown as { __hcCalls?: Recorded[] }).__hcCalls ?? [],
  ) as Promise<Recorded[]>;
}

async function searchContent(page: Page, width = 1200) {
  await page.setViewportSize({ width, height: 720 });
  await page.goto("/harness.html?at=/");
  await page.waitForSelector(".hc-row");
  await page.getByLabel("Suche").fill("content");
  await expect(page.locator(".hc-row")).toHaveCount(2);
}

test.describe("two files with the same name", () => {
  test("the rows carry what tells them apart: directory, size and age", async ({ page }) => {
    await searchContent(page);
    const rows = page.locator(".hc-row");

    const first = rows.nth(0);
    const second = rows.nth(1);
    await expect(first.locator(".hc-row-name")).toContainText("content.png");
    await expect(second.locator(".hc-row-name")).toContainText("content(1).png");

    // Different directories, spelled out on the row rather than left to the tooltip.
    const dirs = [
      await first.locator(".hc-row-dir").innerText(),
      await second.locator(".hc-row-dir").innerText(),
    ];
    expect(dirs[0]).toContain("Downloads");
    expect(dirs[1]).toContain("assets");
    expect(dirs[0]).not.toBe(dirs[1]);

    // Different sizes and ages, in the metadata column.
    const meta = [
      await first.locator(".hc-row-meta").innerText(),
      await second.locator(".hc-row-meta").innerText(),
    ];
    expect(meta[0]).not.toBe(meta[1]);
    expect(meta[0]).toMatch(/MB/);
    expect(meta[1]).toMatch(/kB|KB/i);
  });

  test("the preview shows the row under the cursor, and follows it", async ({ page }) => {
    await searchContent(page);
    const head = page.locator(".hc-preview-head");

    await expect(head.locator(".hc-title")).toHaveText("content.png");
    await expect.poll(async () =>
      (await calls(page)).filter((c) => c.command === "preview").map((c) => String(c.args.path)),
    ).toContain("/home/user/Downloads/content.png");

    const firstMeta = await head.locator(".hc-meta").innerText();
    await page.keyboard.press("ArrowDown");
    await expect(head.locator(".hc-title")).toHaveText("content(1).png");
    // The pane's size and age change with it.
    await expect(head.locator(".hc-meta")).not.toHaveText(firstMeta);
    await expect(head.locator(".hc-meta")).toContainText(/kB|KB/i);
    await expect.poll(async () =>
      (await calls(page)).filter((c) => c.command === "preview").map((c) => String(c.args.path)),
    ).toContain("/home/user/Projects/site/assets/content(1).png");
    // The rendered picture is the second one's: 800 wide, not 320.
    await expect(page.locator(".hc-preview-image")).toHaveAttribute("width", "800");
  });

  test("selecting both puts them side by side, with what differs marked", async ({ page }) => {
    await searchContent(page);
    await page.keyboard.press("Shift+ArrowDown");

    const compare = page.locator(".hc-compare");
    await expect(compare).toHaveCount(1);
    await expect(compare.locator(".hc-compare-name")).toHaveCount(2);
    await expect(compare.locator(".hc-compare-name").nth(0)).toHaveText("content.png");
    await expect(compare.locator(".hc-compare-name").nth(1)).toHaveText("content(1).png");

    // Directory, size and date all differ here; permissions are absent on a search result.
    const differing = compare.locator(".hc-compare-fact[data-differs='yes']");
    await expect(differing).toHaveCount(3);
    await expect(compare.locator(".hc-compare-fact").nth(0)).toContainText("Ordner");
    await expect(compare.locator(".hc-compare-fact").nth(0)).toContainText("Downloads");
    await expect(compare.locator(".hc-compare-fact").nth(0)).toContainText("assets");
    await expect(compare.locator(".hc-preview-head .hc-meta")).toContainText("3 Unterschiede");

    // Both pictures were rendered — one request each — and both are on screen.
    await expect(compare.locator(".hc-preview-image")).toHaveCount(2);
    const rendered = (await calls(page))
      .filter((c) => c.command === "preview")
      .map((c) => String(c.args.path));
    expect(rendered).toContain("/home/user/Downloads/content.png");
    expect(rendered).toContain("/home/user/Projects/site/assets/content(1).png");

    // Escape narrows the selection back to one, and the ordinary preview returns.
    await page.keyboard.press("Escape");
    await expect(compare).toHaveCount(0);
    await expect(page.locator(".hc-preview-head .hc-title")).toHaveText(/content/);
  });

  test("Quick Look shows the file as large as the window and asks for more pixels than the pane", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1200, height: 720 });
    await page.goto("/harness.html?at=/");
    await page.waitForSelector(".hc-row");
    // A directory listing, empty search field: Space is Quick Look. bild.png is the fourth row.
    await page.locator(".hc-row", { hasText: "bild.png" }).click();
    const before = (await calls(page)).filter((c) => c.command === "preview");
    await page.keyboard.press(" ");

    const look = page.locator(".hc-quicklook");
    await expect(look).toHaveCount(1);
    await expect(look.locator(".hc-title")).toHaveText("bild.png");
    await expect(look.locator(".hc-preview-image")).toHaveCount(1);

    const after = (await calls(page)).filter((c) => c.command === "preview");
    expect(after.length).toBeGreaterThan(before.length);
    // The pane asks for 512 × DPR; Quick Look asks for the window's long edge, here 1200. Which
    // request lands *last* is a race — the pane behind the overlay is still mounted and may
    // re-request, and reading `after[after.length - 1]` once caught its 512 and called it Quick
    // Look's. The property is not "the last request", it is "the full-window request happened,
    // and it was this file": exactly one request is larger than the pane's.
    const fullWindow = after.filter((c) => Number(c.args.pixels) > 512);
    expect(fullWindow, "exactly one request asks for the window, not the pane").toHaveLength(1);
    expect(String(fullWindow[0]!.args.path)).toContain("bild.png");

    // Space did not type a space into the search field.
    await expect(page.getByLabel("Suche")).toHaveValue("");

    // 1 is pixel for pixel, 0 fits again, Escape closes.
    await page.keyboard.press("1");
    await expect(look).toHaveAttribute("data-zoom", "actual");
    await page.keyboard.press("0");
    await expect(look).toHaveAttribute("data-zoom", "fit");
    await page.keyboard.press("Escape");
    await expect(look).toHaveCount(0);
    // And Escape closed only Quick Look: the search field is still empty and the list still there.
    await expect(page.locator(".hc-row").first()).toBeVisible();
  });

  test("Ctrl+Space opens Quick Look on a search result too, where plain Space is a character", async ({
    page,
  }) => {
    await searchContent(page);
    await page.keyboard.press("Control+ ");
    await expect(page.locator(".hc-quicklook")).toHaveCount(1);
    await expect(page.getByLabel("Suche")).toHaveValue("content");
    await page.keyboard.press("Escape");
    await expect(page.locator(".hc-quicklook")).toHaveCount(0);
    await expect(page.getByLabel("Suche")).toHaveValue("content");
  });

  test("the listing has column headers that say what orders it", async ({ page }) => {
    await page.setViewportSize({ width: 1200, height: 720 });
    await page.goto("/harness.html?at=/");
    await page.waitForSelector(".hc-row");
    const header = page.locator(".hc-list-header");
    await expect(header).toHaveCount(1);
    await expect(header.locator("[aria-sort='ascending']")).toHaveText(/Name/);

    await header.getByRole("columnheader", { name: /Größe/ }).click();
    await expect(header.locator("[aria-sort='ascending']")).toHaveText(/Größe/);
    const asked = (await calls(page)).filter((c) => c.command === "view_open");
    expect(String(asked[asked.length - 1]!.args.order)).toBe("size");

    // The header is a claim about the order, so it is gone while a search — ordered by
    // relevance — is showing.
    await page.getByLabel("Suche").fill("query");
    await expect(header).toHaveCount(0);
  });
});
