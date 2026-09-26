/**
 * M10 step 2, in the window: what a row carries from the brain, and how it shows.
 *
 * The daemon's tests assert that the columns arrive. What only a browser can answer is the
 * encoding: **tier is brightness, not a badge** (§4.4) — the same encoding the three-dimensional
 * view uses, so somebody who has looked at one can read the other — and a row that somebody
 * pinned is the exception to it.
 */

import { expect, test, type Page } from "@playwright/test";

async function open(page: Page, at = "/") {
  await page.setViewportSize({ width: 1200, height: 700 });
  await page.goto(`/harness.html?at=${at}`);
  await page.waitForSelector(".hc-row");
}

/** The rendered opacity of a row's text, which is what "dimmer" means here. */
async function brightness(page: Page, name: string): Promise<number> {
  return page
    .locator(".hc-row", { hasText: name })
    .first()
    .locator(".hc-row-name")
    .evaluate((el) => Number(getComputedStyle(el).opacity));
}

test.describe("the memory on a row", () => {
  test("a cold row is dimmer, and nothing says the word cold", async ({ page }) => {
    await open(page);

    const cold = await brightness(page, "kurz.txt");
    const ordinary = await brightness(page, "notes-with-a-long");
    expect(cold).toBeLessThan(ordinary);

    // No badge, no icon, no column — the encoding is the brightness and only the brightness.
    await expect(page.locator(".hc-tier")).toHaveCount(0);
    await expect(page.locator(".hc-row", { hasText: "kurz.txt" }).first()).not.toContainText(
      "cold",
    );
  });

  test("a row somebody pinned does not fade", async ({ page }) => {
    await open(page);
    // `bild.png` is pinned in the fixture and in the working tier: held against forgetting, and
    // an interface that dimmed it would be disagreeing with the person who held it.
    expect(await brightness(page, "bild.png")).toBe(1);
  });

  test("the rows still carry what stat says as well as what the brain says", async ({ page }) => {
    await open(page);
    const row = page.locator(".hc-row", { hasText: "kurz.txt" }).first();
    // The point of one row shape: the memory arrived without costing the permissions.
    await expect(row).toContainText("-rw-r--r--");
    await expect(row).toContainText("12 B");
  });

  test("a column heading stands over its own values", async ({ page }) => {
    // The heading and the values it heads live in two different elements — one line of buttons, and
    // every row's own metadata — so „under" is arranged rather than inherited, and until now it was a
    // guess: the first cell under „Salienz" was the permission string and the salience value was not
    // rendered at all. A heading is a claim about what is beneath it, which makes it something to
    // measure rather than something to look at.
    //
    // Measured in a wide column, because that is where the widths are the widths: when the list has
    // no room for the table the numbers give way (they shrink rather than pushing the metadata out
    // over the panel beside them), and that containment is the second half of this test.
    await page.setViewportSize({ width: 1440, height: 800 });
    await page.goto("/harness.html?at=/");
    await page.waitForSelector(".hc-row");

    const row = page.locator(".hc-row").first();
    await expect(row.locator(".hc-row-salience")).toHaveText(/^\d+\.\d{2}$/);

    const pairs: [string, string][] = [
      [".hc-list-header-salience", ".hc-row-salience"],
      [".hc-list-header-size", ".hc-row-size"],
      [".hc-list-header-modified", ".hc-row-age"],
    ];
    for (const [heading, cell] of pairs) {
      const head = (await page.locator(heading).boundingBox())!;
      const value = (await row.locator(cell).boundingBox())!;
      expect(Math.abs(head.x + head.width - (value.x + value.width))).toBeLessThan(2);
    }

    // „Under" has a second half, and that is the half that was wrong: the headings have to stand on
    // the *same* line as each other. The strip declared no area map while `.hc-list-header-meta`
    // asked for `grid-area: meta`, and an undefined named line in `grid-area` makes the grid invent
    // tracks — here two 0 px columns and a second 22 px row, so „Name" stood alone with the three
    // headings under it, half a line below the values they head. A strip that is one line measures
    // as one line.
    const lines = await page.evaluate(() => {
      const strip = document.querySelector(".hc-list-header")!;
      const cells = Array.from(document.querySelectorAll(".hc-list-header-name, .hc-list-header-meta > *"));
      const centres = cells.map((el) => {
        const r = el.getBoundingClientRect();
        return r.top + r.height / 2;
      });
      return {
        spread: Math.max(...centres) - Math.min(...centres),
        height: strip.getBoundingClientRect().height,
        cells: cells.length,
      };
    });
    expect(lines.cells).toBeGreaterThan(2);
    expect(lines.spread, "die Überschriften stehen nicht auf einer Zeile").toBeLessThan(2);
    expect(lines.height, "der Streifen ist höher als seine eine Zeile").toBeLessThan(34);

    // Narrow the window until the list is cramped: the metadata has to stay inside its row. It used
    // to sit in the row's `auto` track, sized to its own max-content, and ran out over the panel.
    await page.setViewportSize({ width: 1200, height: 800 });
    await page.waitForTimeout(200);
    const crowded = await page.evaluate(() => {
      const row = document.querySelector(".hc-row")!;
      const meta = row.querySelector(".hc-row-meta")!;
      return meta.getBoundingClientRect().right - row.getBoundingClientRect().right;
    });
    expect(crowded).toBeLessThan(1);
  });

  test("salience is offered as an order, and asking for it asks the daemon for it", async ({
    page,
  }) => {
    await open(page);
    await page.keyboard.press("Control+k");
    const command = page.getByText("Sortieren: Salienz", { exact: true });
    await expect(command).toBeVisible();
    await command.click();

    await page.waitForTimeout(150);
    const asked = (await page.evaluate(
      () =>
        (window as unknown as { __hcCalls?: { command: string; args: Record<string, unknown> }[] })
          .__hcCalls ?? [],
    )) as { command: string; args: Record<string, unknown> }[];

    const opened = asked.filter((c) => c.command === "view_open");
    // The frontend never sorts (D4): it asks for an order and renders what comes back.
    expect(opened[opened.length - 1]!.args.order).toBe("salience");
  });
});
