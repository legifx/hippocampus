/**
 * The brain view, in a real layout engine.
 *
 * What is claimed: the view opens from the rail and from `?section=brain`; the daemon's numbers
 * arrive in the status line, the heatmap, the agents and the live log exactly as sent (D4); the
 * curator can be asked from the window; and — the claim this view is built around — the scene
 * does not draw while nothing happens, and stops drawing and polling when the view is left.
 */

import { expect, test, type Page } from "@playwright/test";

async function openBrain(page: Page) {
  await page.goto(`/harness.html?section=brain`);
  await expect(page.getByRole("region", { name: "Gehirn", exact: true })).toBeVisible();
}

async function calls(page: Page, command: string): Promise<number> {
  return page.evaluate(
    (c) => ((window as unknown as { __hcCalls?: { command: string }[] }).__hcCalls ?? []).filter((x) => x.command === c).length,
    command,
  );
}

test.describe("the brain view", () => {
  test("shows the daemon's numbers as they arrive", async ({ page }) => {
    await openBrain(page);
    const status = page.locator(".hc-brain-status");
    await expect(status).toContainText("Kurator: schläft (Qwen3-4B-Q4_K_M)");
    await expect(status).toContainText("Main Brain: synchron vor 4 min");
    await expect(status).toContainText("Sicherung: vor 1 h");
    await expect(page.locator(".hc-heat li").first()).toContainText("Projects");
    await expect(page.locator(".hc-agents li").first()).toContainText("claude");
    await expect(page.locator(".hc-log li").first()).toBeVisible();
  });

  test("the live log grows while the view is showing", async ({ page }) => {
    await openBrain(page);
    const first = await page.locator(".hc-log li").count();
    await expect.poll(async () => page.locator(".hc-log li").count(), { timeout: 8000 }).toBeGreaterThan(first);
  });

  test("the curator can be asked from the window", async ({ page }) => {
    await openBrain(page);
    await page.getByLabel("Frage an den Kurator").fill("Wo ist der Parser?");
    await page.getByRole("button", { name: "fragen" }).click();
    await expect(page.locator(".hc-answer-text")).toHaveText("Der Parser steht in [1].");
    await expect(page.locator(".hc-answer-path").first()).toHaveText("~/Projects/parser.rs");
  });

  test("leaving the view stops the polling", async ({ page }) => {
    await openBrain(page);
    await expect.poll(() => calls(page, "brain_activity")).toBeGreaterThan(0);
    await page.getByRole("button", { name: /Dateien/ }).first().click();
    const after = await calls(page, "brain_activity");
    await page.waitForTimeout(3500);
    expect(await calls(page, "brain_activity")).toBeLessThanOrEqual(after + 1);
  });

  test("opens from the rail and survives 400 px without horizontal overflow", async ({ page }) => {
    await page.setViewportSize({ width: 400, height: 800 });
    await page.goto(`/harness.html`);
    await page.getByRole("button", { name: "Gehirn" }).click();
    await expect(page.getByRole("region", { name: "Gehirn", exact: true })).toBeVisible();
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(overflow).toBeLessThanOrEqual(0);
  });
});

test.describe("the launcher's section flags", () => {
  async function fire(page: import("@playwright/test").Page, event: string, payload: unknown) {
    await page.evaluate(
      ([e, p]) => {
        const l = (window as unknown as { __hcListeners?: Map<string, Set<(x: unknown) => void>> }).__hcListeners;
        l?.get(e as string)?.forEach((h) => h(p));
      },
      [event, payload],
    );
  }

  test("`--brain` while hidden opens the window on the brain", async ({ page }) => {
    await page.goto(`/harness.html?section=overview`);
    await page.waitForSelector(".hc-ov-headline");
    await fire(page, "window:section", "brain");
    await fire(page, "window:shown", null);
    await expect(page.getByRole("region", { name: "Gehirn", exact: true })).toBeVisible();
  });

  test("`--files` while showing the brain switches to the files", async ({ page }) => {
    await page.goto(`/harness.html?section=brain`);
    await fire(page, "window:section-toggle", "files");
    await expect(page.getByRole("region", { name: "Gehirn", exact: true })).toBeHidden();
  });
});
