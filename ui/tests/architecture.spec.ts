/**
 * The Agent Inspector's Architektur tab draws something.
 *
 * It drew nothing, on the owner's machine, with no error anywhere — and the cause was not the
 * renderer or the data: the scene measured its container once, at construction, while the panel
 * was still animating in. Zero by zero, and a `window` resize listener that nothing ever fired.
 *
 * So this asserts the property that was missing: the canvas ends up the size of the box it is
 * in. Chromium is not WebKitGTK, and a passing test here does not prove the picture appears
 * there — what it does prove is that the sizing bug cannot come back.
 */

import { expect, test } from "@playwright/test";

test.describe("the three-dimensional view inside the panel", () => {
  test("its canvas is the size of the box it sits in, not zero", async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 800 });
    await page.goto("/harness.html?at=/&hits=20");
    await page.waitForSelector(".hc-row");
    await page.keyboard.press("Control+Shift+A");
    await page.waitForSelector(".hc-inspector");
    await page.getByRole("tab", { name: "Architektur" }).click();

    // The scene mounts asynchronously: three.js is loaded on demand, deliberately, because this
    // window has a 300 ms cold-start gate and most sessions never open this tab.
    const canvas = page.locator(".hc-scene canvas");
    await expect(canvas).toBeVisible({ timeout: 15_000 });

    const box = await canvas.boundingBox();
    const host = await page.locator(".hc-scene").boundingBox();
    expect(box?.width ?? 0).toBeGreaterThan(200);
    expect(box?.height ?? 0).toBeGreaterThan(120);
    // And it fills its box rather than sitting in a corner of it.
    expect(Math.abs((box?.width ?? 0) - (host?.width ?? 0))).toBeLessThan(4);
  });

  test("it says what it is showing, and how to read it", async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 800 });
    await page.goto("/harness.html?at=/&hits=20");
    await page.waitForSelector(".hc-row");
    await page.keyboard.press("Control+Shift+A");
    await page.waitForSelector(".hc-inspector");
    await page.getByRole("tab", { name: "Architektur" }).click();
    await expect(page.locator(".hc-scene canvas")).toBeVisible({ timeout: 15_000 });

    const pane = page.locator(".hc-architecture");
    // The counts, so "is this all of it" has an answer on screen.
    await expect(pane).toContainText("Knoten");
    await expect(pane).toContainText("Kanten");
    // And the legend, because brightness-is-tier is not guessable.
    await expect(pane).toContainText("Helligkeit ist die Stufe");
  });
});
