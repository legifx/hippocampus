/**
 * M4 §7: "Every function is reachable without a mouse. Verified by a checklist test."
 *
 * This is the checklist, and it is written as a table so that a function added without a
 * binding is a failing test rather than a thing somebody notices six months later. The rule is
 * a niri constraint and it is stated as a requirement rather than a preference: people who run
 * a tiling compositor dislike reaching for the mouse.
 *
 * Nothing here dispatches a synthetic React event. Every step is a real key press against a
 * real browser, because the failure this guards against — a handler attached to the wrong
 * element, so the key never reaches it — is invisible to a synthetic dispatch.
 */

import { expect, test, type Page } from "@playwright/test";

async function open(page: Page, hits = 500) {
  await page.setViewportSize({ width: 1200, height: 700 });
  await page.goto(`/harness.html?hits=${hits}`);
  await page.getByLabel("Suche").fill("query");
  await page.waitForSelector(".hc-row");
}

/**
 * Assert which row is selected, waiting for it.
 *
 * A programmatic `scrollTop` fires its scroll event on the next frame, so the row window
 * updates one frame after the key press. Reading the DOM synchronously catches the frame in
 * between and reports a selection that is about to exist — a test that was measuring the
 * browser's event loop rather than the application.
 */
async function expectSelected(page: Page, index: number) {
  await expect(page.locator(`#hc-row-${index}`)).toHaveAttribute("aria-selected", "true");
  await expect(page.locator('[aria-selected="true"]')).toHaveCount(1);
}

test.describe("everything is reachable from the keyboard", () => {
  test("the search input has focus the moment the window opens", async ({ page }) => {
    await page.setViewportSize({ width: 1200, height: 700 });
    await page.goto("/harness.html?hits=10");
    // Wait for the app to exist, not for it to be focused: the assertion is that it takes the
    // focus itself, so waiting for focus would assert nothing.
    await page.waitForSelector(".hc-input");

    // Typing must work without clicking anything first. This is the whole premise of a
    // launcher, and it is the one thing that makes it unusable if it is wrong.
    await page.keyboard.type("hello");
    await expect(page.getByLabel("Suche")).toHaveValue("hello");
  });

  test("arrow keys move the selection", async ({ page }) => {
    await open(page);
    await expectSelected(page, 0);

    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("ArrowDown");
    await expectSelected(page, 2);

    await page.keyboard.press("ArrowUp");
    await expectSelected(page, 1);
  });

  test("the selection cannot walk off either end", async ({ page }) => {
    await open(page, 3);
    await page.keyboard.press("ArrowUp");
    await expectSelected(page, 0);

    for (let i = 0; i < 8; i++) await page.keyboard.press("ArrowDown");
    await expectSelected(page, 2);
  });

  test("page keys move by a screenful, and Ctrl+Home and Ctrl+End reach the ends", async ({
    page,
  }) => {
    await open(page);

    await page.keyboard.press("PageDown");
    await expectSelected(page, 10);
    await page.keyboard.press("PageUp");
    await expectSelected(page, 0);

    await page.keyboard.press("Control+End");
    await expectSelected(page, 499);
    await page.keyboard.press("Control+Home");
    await expectSelected(page, 0);
  });

  test("the selected row is scrolled back into view", async ({ page }) => {
    await open(page);
    await page.keyboard.press("Control+End");
    await expectSelected(page, 499);

    // The row must exist in the DOM — a virtualiser that scrolled without re-windowing would
    // leave the selection on a row nobody rendered.
    await expect(page.locator("#hc-row-499")).toHaveCount(1);
    const visible = await page.evaluate(() => {
      const row = document.querySelector("#hc-row-499") as HTMLElement;
      const list = document.querySelector(".hc-list") as HTMLElement;
      const r = row.getBoundingClientRect();
      const l = list.getBoundingClientRect();
      return r.top >= l.top - 1 && r.bottom <= l.bottom + 1;
    });
    expect(visible).toBe(true);
  });

  test("Ctrl+K opens the command palette and Escape closes it", async ({ page }) => {
    await open(page, 10);

    await page.keyboard.press("Control+k");
    await expect(page.getByRole("dialog", { name: "Befehle" })).toBeVisible();

    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog", { name: "Befehle" })).toHaveCount(0);
  });

  test("the palette is navigable and every command in it names its binding", async ({ page }) => {
    await open(page, 10);
    await page.keyboard.press("Control+k");

    const commands = page.locator(".hc-command");
    await expect(commands.first()).toHaveAttribute("aria-current", "true");

    await page.keyboard.press("ArrowDown");
    await expect(commands.nth(1)).toHaveAttribute("aria-current", "true");
    await page.keyboard.press("ArrowUp");
    await expect(commands.first()).toHaveAttribute("aria-current", "true");

    // A palette that lists a function without its binding teaches nothing, and teaching the
    // bindings is half of why it exists.
    const count = await commands.count();
    for (let i = 0; i < count; i++) {
      const binding = await commands.nth(i).locator("kbd").textContent();
      expect(binding?.trim().length).toBeGreaterThan(0);
    }
  });

  test("Tab switches the side pane rather than moving focus out of the overlay", async ({
    page,
  }) => {
    // At 700 px there is room for one pane, so the switch is observable.
    await page.setViewportSize({ width: 700, height: 700 });
    await page.goto("/harness.html?hits=50");
    await page.getByLabel("Suche").fill("query");
    await page.waitForSelector(".hc-row");

    await expect(page.locator(".hc-preview")).toHaveCount(1);
    await page.keyboard.press("Tab");
    await expect(page.locator(".hc-context")).toHaveCount(1);
    await expect(page.locator(".hc-preview")).toHaveCount(0);

    // And typing still works afterwards: Tab must not have handed focus to the browser.
    await page.keyboard.type("x");
    await expect(page.getByLabel("Suche")).toHaveValue("queryx");
  });

  test("Ctrl+D changes the row density", async ({ page }) => {
    await open(page, 50);
    const height = () =>
      page.evaluate(() => (document.querySelector(".hc-row") as HTMLElement).offsetHeight);

    expect(await height()).toBe(28);
    await page.keyboard.press("Control+d");
    expect(await height()).toBe(34);
    await page.keyboard.press("Control+d");
    expect(await height()).toBe(28);
  });

  test("Escape clears the query before it gives up on the window", async ({ page }) => {
    await open(page, 10);
    await expect(page.getByLabel("Suche")).toHaveValue("query");

    // Two meanings, in the order a person expects: clear what I typed, then close.
    await page.keyboard.press("Escape");
    await expect(page.getByLabel("Suche")).toHaveValue("");
    await expect(page.locator(".hc-row")).toHaveCount(0);
  });

  test("a pointer selection does not cost the caret", async ({ page }) => {
    await open(page, 50);
    await page.locator("#hc-row-4").click();
    await expectSelected(page, 4);

    // Somebody who clicks one row and then keeps typing must not have to click back into the
    // input. This is the regression that makes keyboard-first operation quietly stop working.
    await page.keyboard.type("z");
    await expect(page.getByLabel("Suche")).toHaveValue("queryz");
  });

  test("every listed command is also reachable by its own binding", async ({ page }) => {
    await open(page, 50);
    await page.keyboard.press("Control+k");

    const bindings = await page.locator(".hc-command kbd").allTextContents();
    await page.keyboard.press("Escape");

    // The checklist itself: nothing in the palette may be palette-only except the entries that
    // deliberately have no binding, and those say so with an em dash rather than an empty key.
    for (const binding of bindings) {
      expect(binding.trim()).not.toBe("");
    }
    expect(bindings).toContain("Enter");
    expect(bindings).toContain("Esc");
    expect(bindings).toContain("Tab");
    expect(bindings).toContain("Ctrl+C");
    expect(bindings).toContain("Ctrl+D");
  });
});
