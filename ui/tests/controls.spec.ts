/**
 * No control in this window is painted by the platform.
 *
 * A `<button>` with no `background` keeps the user agent's `buttonface`, which is *light*, while
 * the classes on it set only a text colour — so every control that did not explicitly paint
 * itself came out light-grey-on-light-grey. It hit the Agent Inspector's tabs, its kill switch,
 * the actions on a memory and the one `select` in the window, and it read as several separate
 * bugs rather than one missing rule.
 *
 * This is the rule, asserted: in a dark window, nothing is drawn light.
 */

import { expect, test, type Page } from "@playwright/test";

/** Perceived lightness of a computed colour, 0 (black) to 1 (white). Transparent counts as dark
 *  because what shows through is the window, which is dark. */
const LUMINANCE = `(value) => {
  const match = String(value).match(/rgba?\\(([^)]+)\\)/);
  if (!match) return 0;
  const parts = match[1].split(",").map((n) => Number(n.trim()));
  const [r, g, b, a = 1] = parts;
  if (a === 0) return 0;
  /* A translucent fill is not a light one: over the near-black window, the colour the screen
     actually shows is the fill pre-multiplied by its alpha - 5.5% white (the kit's glass
     fill) is the darkest paint in this window, not the brightest. Multiply first. */
  /* NB: this function travels as a template string to the page, so no backticks inside. */
  return ((0.2126 * r + 0.7152 * g + 0.0722 * b) * a) / 255;
}`;

async function lightControls(page: Page): Promise<string[]> {
  return page.evaluate(`(() => {
    const luminance = ${LUMINANCE};
    const offenders = [];
    for (const el of document.querySelectorAll("button, select, input")) {
      const style = getComputedStyle(el);
      if (luminance(style.backgroundColor) > 0.5) {
        offenders.push((el.className || el.tagName) + " — " + style.backgroundColor);
      }
    }
    return offenders;
  })()`) as Promise<string[]>;
}

test.describe("controls are painted by this window and not by the platform", () => {
  test("nothing in the ordinary window is drawn light", async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 700 });
    await page.goto("/harness.html?at=/&hits=20");
    await page.waitForSelector(".hc-row");
    expect(await lightControls(page)).toEqual([]);

    // The palette, which is nothing but buttons.
    await page.keyboard.press("Control+k");
    await page.waitForSelector(".hc-palette");
    expect(await lightControls(page)).toEqual([]);
  });

  test("nothing in the Agent Inspector is drawn light, on any tab", async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 700 });
    await page.goto("/harness.html?at=/&hits=20");
    await page.waitForSelector(".hc-row");
    await page.keyboard.press("Control+Shift+A");
    await page.waitForSelector(".hc-inspector");

    for (const tab of ["Live", "Gedächtnis", "Regeln", "Rückgängig"]) {
      await page.getByRole("tab", { name: tab }).click();
      await page.waitForTimeout(120);
      expect(await lightControls(page), `on the ${tab} tab`).toEqual([]);
    }
  });

  test("a tab that is not selected still looks pressable", async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 700 });
    await page.goto("/harness.html?at=/&hits=20");
    await page.waitForSelector(".hc-row");
    await page.keyboard.press("Control+Shift+A");
    await page.waitForSelector(".hc-inspector");

    const idle = page.getByRole("tab", { name: "Regeln" });
    const before = await idle.evaluate((el) => getComputedStyle(el).color);
    await idle.hover();
    await page.waitForTimeout(80);
    const after = await idle.evaluate((el) => getComputedStyle(el).color);
    expect(after).not.toBe(before);
  });
});
