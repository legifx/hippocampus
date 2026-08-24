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

  test("frames are actually drawn, not just a canvas created", async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 800 });
    await page.goto("/harness.html?at=/&hits=20");
    await page.waitForSelector(".hc-row");
    await page.keyboard.press("Control+Shift+A");
    await page.waitForSelector(".hc-inspector");
    await page.getByRole("tab", { name: "Architektur" }).click();
    await expect(page.locator(".hc-scene canvas")).toBeVisible({ timeout: 15_000 });

    // The scene does not drive itself: `render()` draws one frame and returns, and the panel has
    // to call it from a loop. It did not, and the tab was black — built, sized, never drawn. The
    // panel now measures its own frame rate and prints it, which is both the diagnostic that was
    // missing and the thing a test can read.
    const rate = page.locator(".hc-scene-rate");
    await expect(rate).toBeVisible();
    // Generous, and deliberately so: this is the one test in the suite that compiles shaders,
    // and five of them at once on a software renderer take a while to produce a first frame.
    // What is being asserted is "something draws", not "it draws quickly" — the frame budgets
    // live in the perf suite, which runs alone.
    await expect(rate).not.toContainText("es wird nichts gezeichnet", { timeout: 30_000 });

    // And the canvas has a drawing buffer of its own, sized to the box rather than left at zero.
    const buffer = await page.locator(".hc-scene canvas").evaluate((el) => ({
      w: (el as HTMLCanvasElement).width,
      h: (el as HTMLCanvasElement).height,
    }));
    expect(buffer.w).toBeGreaterThan(200);
    expect(buffer.h).toBeGreaterThan(120);
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

  test("it stops drawing when nobody can see the window", async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 800 });
    await page.goto("/harness.html?at=/&hits=20");
    await page.waitForSelector(".hc-row");
    await page.keyboard.press("Control+Shift+A");
    await page.waitForSelector(".hc-inspector");
    await page.getByRole("tab", { name: "Architektur" }).click();
    await expect(page.locator(".hc-scene canvas")).toBeVisible({ timeout: 15_000 });
    await expect(page.locator(".hc-scene-rate")).not.toContainText("es wird nichts gezeichnet", {
      timeout: 30_000,
    });

    // The overlay is resident: it spends almost all of its life hidden, and a three-dimensional
    // scene rendering for nobody is D59's lesson with a GPU attached. The shell says when the
    // window goes away; the loop reads it and stops.
    await page.evaluate(() => {
      (window as unknown as { __hcEmit?: (e: string, p?: unknown) => void }).__hcEmit?.(
        "window:visible",
        false,
      );
    });
    await expect(page.locator(".hc-scene-rate")).toContainText("es wird nichts gezeichnet", {
      timeout: 15_000,
    });
  });
});
