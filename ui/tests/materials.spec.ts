/**
 * The design system's materials, checked against the layout engine rather than by eye.
 *
 * Every material in `src/glass.css` is a single `box-shadow` list, and one wrong token inside it does
 * not weaken the shadow — it makes the *whole* declaration invalid at computed-value time. The
 * browser then drops it and reports `none`, silently: the fill and the gradient are separate
 * properties and still render, so the surface does not look broken, it looks flat. „Flat glass" is
 * exactly what a missing rim looks like, which is why this went unseen for as long as it did: the
 * tokens carry their own geometry (`--hc-glass-rim` *is* `inset 0 1px 0 rgba(...)`), and writing the
 * kit's `inset 0 1px 0 0` in front of one produced `inset 0 1px 0 0 inset 0 1px 0 rgba(...)` — every
 * material in the file, `.glass` through `.hc-pill` (D112).
 *
 * A picture of the window cannot say this. The computed value can, so that is what is asserted:
 * whatever carries a material class comes back with a shadow, on the screens that use them.
 */

import { expect, test, type Page } from "@playwright/test";

/** The kit's material names (§2c). `glass-weak` / `glass-strong` are modifiers — they only set a
 *  fill, so on their own they have nothing to say about a shadow and are checked with their base. */
const MATERIALS = [
  "glass",
  "glass-flat",
  "raised",
  "raised-flat",
  "raised-bezel",
  "well",
  "solid",
  "hc-pill",
  "hc-tile",
];

/**
 * Every element that wears one of these classes, with the shadow the engine computed for it.
 * Returns the ones that came back bare, plus how many were checked — a screen that stops rendering
 * materials must fail this test rather than pass it by finding nothing.
 */
async function bareMaterials(page: Page, url: string, minimum: number) {
  await page.goto(url);
  await page.waitForSelector(".hc-row, .hc-card, .hc-rail");
  await page.waitForTimeout(400);

  const scan = (materials: string[]) => {
    const bare: string[] = [];
    let checked = 0;
    document.querySelectorAll("*").forEach((el) => {
      const names = Array.from(el.classList).filter((name) => materials.includes(name));
      if (!names.length) return;
      checked += 1;
      if (getComputedStyle(el).boxShadow === "none") {
        const where = `${el.tagName.toLowerCase()}.${names.join(".")}`;
        bare.push(`${where} (${(el.textContent ?? "").trim().slice(0, 30)})`);
      }
    });
    return { bare, checked };
  };

  const found = await page.evaluate(scan, MATERIALS);
  expect(found.checked, `kein Material auf ${url} gefunden — der Test prüft nichts`).toBeGreaterThanOrEqual(
    minimum,
  );
  expect(found.bare, `Material ohne Schatten auf ${url}`).toEqual([]);
}

test.describe("every material reaches the DOM", () => {
  test("the start screen's surfaces carry their recipe", async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 900 });
    await bareMaterials(page, "/harness.html", 4);
  });

  test("the file list's surfaces carry theirs, at both widths", async ({ page }) => {
    // The list screen is where the kit is most visible — the rail, the listing's strip, the context
    // panel and the pills on a row — and 400 px is where a pane that fails to paint is easiest to
    // mistake for a pane that is not there.
    await page.setViewportSize({ width: 1400, height: 900 });
    await bareMaterials(page, "/harness.html?at=/", 5);

    await page.setViewportSize({ width: 400, height: 800 });
    await page.waitForTimeout(300);
    const found = await page.evaluate((materials: string[]) => {
      let checked = 0;
      const bare: string[] = [];
      document.querySelectorAll("*").forEach((el) => {
        const names = Array.from(el.classList).filter((name) => materials.includes(name));
        if (!names.length) return;
        checked += 1;
        if (getComputedStyle(el).boxShadow === "none") bare.push(`${el.tagName}.${names.join(".")}`);
      });
      return { bare, checked };
    }, MATERIALS);
    expect(found.checked).toBeGreaterThanOrEqual(3);
    expect(found.bare).toEqual([]);
  });
});

/**
 * The canvas the materials sit on (§6.4).
 *
 * D113's glass is only glass because there is light behind it: this window's panel fill is six
 * percent white, and six percent white over a black rectangle is a black rectangle. So the contract
 * has two halves — the three glows are there, in the kit's own colours, behind everything and
 * untouchable by the pointer, and they do not move, because moving them was measured at four more
 * composited layers and 48 fps instead of 60 (D114). A later change that re-enables the drift fails
 * here, and has to bring the numbers with it.
 */
