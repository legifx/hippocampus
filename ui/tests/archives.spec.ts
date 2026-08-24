/**
 * M6's archive item, in the window.
 *
 * The daemon's own tests cover what unpacking does to a filesystem. What can only be checked
 * here is the half a person touches: that the command is offered for an archive and for nothing
 * else, that the key asks the daemon before it asks it to start, and that the sentence at the
 * end says what happened rather than borrowing a transfer's words.
 */

import { expect, test, type Page } from "@playwright/test";

type Recorded = { command: string; args: Record<string, unknown> };

/** One level down, because that is where the fixture keeps its archive. */
async function openSubdirectory(page: Page, width = 1200) {
  await page.setViewportSize({ width, height: 700 });
  await page.goto("/harness.html?at=/zweiter-ordner");
  await page.waitForSelector(".hc-row");
}

async function calls(page: Page): Promise<Recorded[]> {
  return page.evaluate(
    () => (window as unknown as { __hcCalls?: Recorded[] }).__hcCalls ?? [],
  ) as Promise<Recorded[]>;
}

/** Put the cursor on a row by clicking it, which is what a person does. */
async function put(page: Page, name: string) {
  await page.locator(".hc-row", { hasText: name }).first().click();
}

test.describe("unpacking an archive", () => {
  test("Ctrl+E asks what is inside before it asks for it to be unpacked", async ({ page }) => {
    await openSubdirectory(page);
    await put(page, "bilder.zip");
    await page.keyboard.press("Control+e");

    await expect(page.locator(".hc-progress")).toBeVisible();

    const asked = (await calls(page)).filter((c) => c.command === "extract_archive");
    expect(asked).toHaveLength(2);
    // The dry run first. A file manager that starts unpacking and then reports that the file
    // was not an archive has already made the decision, badly.
    expect(asked[0]!.args.dryRun).toBe(true);
    expect(asked[1]!.args.dryRun).toBe(false);
    expect(asked[0]!.args.archive).toBe("/zweiter-ordner/bilder.zip");
    // Where it goes is the daemon's decision, so the window names nothing.
    expect(asked[1]!.args.destination).toBeNull();

    // And it says what it is doing, with the name it will land under.
    await expect(page.locator(".hc-banner")).toContainText("entpacke 3 nach bilder");
  });

  test("the finished extraction says where it landed, and a refused entry is named", async ({
    page,
  }) => {
    await openSubdirectory(page);
    await put(page, "bilder.zip");
    await page.keyboard.press("Control+e");
    await expect(page.locator(".hc-progress")).toBeVisible();

    await page.evaluate(() => {
      (window as unknown as { __hcEmit: (e: string, p: unknown) => void }).__hcEmit(
        "transfer:done",
        {
          operation: 7,
          moved: ["/zweiter-ordner/bilder"],
          failed: [{ source: "sneaky", reason: "is a symbolic link" }],
          cancelled: false,
          undo_ids: [4],
          elapsed_ms: 20,
        },
      );
    });

    // Not "1 übertragen": an extraction that borrowed a transfer's sentence would report the
    // one directory it made and say nothing about the entry it refused.
    const banner = page.locator(".hc-banner");
    await expect(banner).toContainText("entpackt nach bilder");
    await expect(banner).toContainText("symbolic link");
    await expect(page.locator(".hc-progress")).toHaveCount(0);
  });

  test("a cancelled extraction says so rather than counting files it did not keep", async ({
    page,
  }) => {
    await openSubdirectory(page);
    await put(page, "bilder.zip");
    await page.keyboard.press("Control+e");
    await expect(page.locator(".hc-progress")).toBeVisible();

    await page.evaluate(() => {
      (window as unknown as { __hcEmit: (e: string, p: unknown) => void }).__hcEmit(
        "transfer:done",
        {
          operation: 7,
          moved: [],
          failed: [],
          cancelled: true,
          undo_ids: [],
          elapsed_ms: 8,
        },
      );
    });

    await expect(page.locator(".hc-banner")).toContainText("entpacken abgebrochen");
  });

  test("the command is offered for an archive and for nothing else", async ({ page }) => {
    await openSubdirectory(page);
    await put(page, "bilder.zip");
    await page.keyboard.press("Control+k");
    await expect(page.getByText("Entpacken", { exact: true })).toBeVisible();
    await page.keyboard.press("Escape");

    await put(page, "kurz.txt");
    await page.keyboard.press("Control+k");
    await expect(page.getByText("Entpacken", { exact: true })).toHaveCount(0);
  });

  test("Ctrl+E on something that is not an archive asks the daemon nothing", async ({ page }) => {
    await openSubdirectory(page);
    await put(page, "kurz.txt");
    await page.keyboard.press("Control+e");
    await page.waitForTimeout(50);

    expect((await calls(page)).filter((c) => c.command === "extract_archive")).toHaveLength(0);
    await expect(page.locator(".hc-progress")).toHaveCount(0);
  });
});
