/**
 * M10 step 4: the sidebar's new job.
 *
 * Its Definition of Done: "a person can ask *why is this here* about any row and get the
 * component breakdown, and can attach a note to a file without touching the file." Plus the
 * removal that makes room for both — "Verbunden" is a place now, so the panel sends you there
 * instead of showing the first few of it.
 */

import { expect, test, type Page } from "@playwright/test";

async function open(page: Page) {
  // Wide enough that the context pane is showing beside the list.
  await page.setViewportSize({ width: 1400, height: 700 });
  await page.goto("/harness.html?at=/&hits=20");
  await page.waitForSelector(".hc-row");
  await page.getByLabel("Suche").fill("module");
  await page.waitForSelector(".hc-context");
}

async function calls(page: Page) {
  return (await page.evaluate(
    () =>
      (window as unknown as { __hcCalls?: { command: string; args: Record<string, unknown> }[] })
        .__hcCalls ?? [],
  )) as { command: string; args: Record<string, unknown> }[];
}

test.describe("why is this here", () => {
  test("the parts of the score are named, not just the number", async ({ page }) => {
    await open(page);

    const why = page.locator(".hc-why");
    await expect(why).toBeVisible();
    // The parts, in words a person can argue with — "zuletzt benutzt" rather than `recency`.
    await expect(why).toContainText("zuletzt benutzt");
    await expect(why).toContainText("wie verbunden");
    // Including the one that subtracts, signed, so the parts visibly add up to the score.
    await expect(why).toContainText("-0.10");
    await expect(why).toContainText("Salienz");
  });

  test("what happened to it is there, in the verbs the log uses", async ({ page }) => {
    await open(page);
    const history = page.locator(".hc-episode");
    await expect(history.first()).toContainText("geöffnet");
    await expect(history.nth(1)).toContainText("notiert");
    // Who did it: an agent's episode and a person's read differently, and the panel says which.
    await expect(history.nth(1)).toContainText("agent:claude");
  });

  test("a note can be written without leaving the window or touching the file", async ({
    page,
  }) => {
    await open(page);
    await page.locator(".hc-annotate-input").fill("gehört zum Parser-Umbau");
    await page.keyboard.press("Enter");

    await expect(page.locator(".hc-annotate-said")).toContainText("notiert");
    const written = await page.evaluate(
      () => (window as unknown as { __hcAnnotated?: { path: string; note: string } }).__hcAnnotated,
    );
    expect(written?.note).toBe("gehört zum Parser-Umbau");
    expect(written?.path).toContain("module");

    // And the panel re-reads rather than pushing the note into its own list: what it shows has
    // to be what the daemon stored.
    const asked = (await calls(page)).filter((c) => c.command === "node_why");
    expect(asked.length).toBeGreaterThan(1);
  });

  test("Verbunden is a place now, and the panel sends you there", async ({ page }) => {
    await open(page);
    // No list of neighbours in the panel any more.
    await expect(page.locator(".hc-related")).toHaveCount(0);

    const go = page.locator(".hc-context-open");
    await expect(go).toContainText("Verbunden ansehen");
    await go.click();
    await page.waitForTimeout(150);

    const opened = (await calls(page)).filter((c) => c.command === "view_open");
    const source = opened[opened.length - 1]!.args.source as { kind: string; path?: string };
    expect(source.kind).toBe("neighbours");
    expect(String(source.path)).toContain("module");
  });
});
