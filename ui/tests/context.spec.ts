import { expect, test } from "@playwright/test";

/**
 * M11 in the main window: selecting a file shows what agents have learned about it.
 *
 * The Inspector is where somebody goes to audit the memory. This is where they meet it without
 * going anywhere — which is the difference between a feature and a feature people use.
 */
test.describe("the context panel carries what agents learned", () => {
  test("a memory about the selection is shown, attributed, at 1400 px", async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 700 });
    await page.goto("/harness.html?at=/&hits=20");
    await page.getByLabel("Suche").fill("query");
    await page.waitForSelector(".hc-row");
    await page.waitForSelector(".hc-memory");

    const memory = page.locator(".hc-memory").first();
    await expect(memory).toContainText("generated on every build");
    // Attributed, or it is a claim from nowhere.
    await expect(memory).toContainText("agent:claude");
    await expect(memory).toContainText("Beobachtung");
  });

  test("the memory section does not overflow at 400 px", async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 700 });
    await page.goto("/harness.html?at=/&hits=20");
    await page.getByLabel("Suche").fill("query");
    await page.waitForSelector(".hc-memory");
    // Narrow afterwards, so the panel has to survive being squeezed rather than being built
    // narrow — which is the case §4 is actually about.
    await page.setViewportSize({ width: 400, height: 700 });
    await page.waitForTimeout(150);

    const overflow = await page.evaluate(() => ({
      body: document.body.scrollWidth - document.body.clientWidth,
      worst: Math.max(
        0,
        ...Array.from(document.querySelectorAll(".hc-memory")).map(
          (el) => (el as HTMLElement).scrollWidth - (el as HTMLElement).clientWidth,
        ),
      ),
    }));
    expect(overflow.body).toBeLessThanOrEqual(0);
    expect(overflow.worst).toBeLessThanOrEqual(0);
  });
});
