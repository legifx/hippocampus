/**
 * M4 §3 and §4, in a real layout engine.
 *
 * §3 asks that the DOM node count stay bounded and be asserted rather than assumed; §4 asks
 * that the layout survive 400 px, which under niri is an ordinary column width and not an edge
 * case. Both are checked against ten thousand results, because the failure they describe only
 * appears at that size.
 */

import { expect, test, type Page } from "@playwright/test";

/** The widths §4 names, plus the minimum the window config allows. */
const WIDTHS = [400, 600, 900, 1400];

async function open(page: import("@playwright/test").Page, width: number, hits = 10_000) {
  await page.setViewportSize({ width, height: 700 });
  await page.goto(`/harness.html?at=/&hits=${hits}`);
  await page.getByLabel("Suche").fill("query");
  await page.waitForSelector(".hc-row");
}

test.describe("the layout holds from 400 px to full width", () => {
  for (const width of WIDTHS) {
    test(`no horizontal overflow at ${width} px`, async ({ page }) => {
      await open(page, width);

      // The page itself must never scroll sideways. Wide content scrolls inside its own pane
      // or elides; it does not push the window.
      const overflow = await page.evaluate(() => ({
        body: document.body.scrollWidth - document.body.clientWidth,
        overlay: (() => {
          const el = document.querySelector(".hc-overlay") as HTMLElement;
          return el.scrollWidth - el.clientWidth;
        })(),
      }));

      expect(overflow.body, `body overflows by ${overflow.body}px at ${width}px`).toBeLessThanOrEqual(0);
      expect(overflow.overlay).toBeLessThanOrEqual(0);
    });

    test(`no row is clipped at ${width} px`, async ({ page }) => {
      await open(page, width);

      // Every rendered row must sit inside the list's box. A row wider than its container is
      // the clipped control §4 forbids, and it is invisible until somebody looks for it.
      const widest = await page.evaluate(() => {
        const list = document.querySelector(".hc-list") as HTMLElement;
        const rows = Array.from(document.querySelectorAll(".hc-row")) as HTMLElement[];
        return Math.max(...rows.map((row) => row.scrollWidth - list.clientWidth));
      });
      expect(widest).toBeLessThanOrEqual(0);
    });

    test(`the search input is usable at ${width} px`, async ({ page }) => {
      await open(page, width);
      const input = page.getByLabel("Suche");
      await expect(input).toBeVisible();

      const box = await input.boundingBox();
      expect(box, "the input has no box at all").not.toBeNull();
      // An input squeezed to nothing by a status line that refused to shrink is the other way
      // §4 fails.
      expect(box!.width).toBeGreaterThan(80);
      expect(box!.x + box!.width).toBeLessThanOrEqual(width);
    });
  }

  test("the search field is measured in its own type, at every width", async ({ page }) => {
    // The field and its text used to be two separate numbers, which is why they only ever agreed
    // at one window width — the search field is where the owner saw it. The box is `em` against
    // its own font-size now, so what is asserted is not a number but a ratio that has to hold at
    // both ends of the range.
    const heights: number[] = [];
    for (const width of [400, 1400]) {
      await open(page, width);
      heights.push(
        await page.evaluate(() => {
          const field = document.querySelector(".hc-field") as HTMLElement;
          const size = parseFloat(getComputedStyle(field).fontSize);
          return field.getBoundingClientRect().height / size;
        }),
      );
    }

    // Roughly three times the type size — a pill with its padding — and the same proportion in a
    // 400 px column as in a full-width window.
    expect(Math.abs(heights[0] - heights[1])).toBeLessThan(0.25);
    for (const ratio of heights) expect(ratio).toBeGreaterThan(2.2);
  });

  test("panes appear as the window gets wider, and only then", async ({ page }) => {
    await open(page, 400);
    expect(await page.locator(".hc-preview").count()).toBe(0);
    expect(await page.locator(".hc-context").count()).toBe(0);

    await page.setViewportSize({ width: 700, height: 700 });
    await expect(page.locator(".hc-preview")).toHaveCount(1);
    expect(await page.locator(".hc-context").count()).toBe(0);

    await page.setViewportSize({ width: 1400, height: 700 });
    await expect(page.locator(".hc-preview")).toHaveCount(1);
    await expect(page.locator(".hc-context")).toHaveCount(1);
  });

  test("shrinking back to 400 px leaves nothing overflowing behind it", async ({ page }) => {
    await open(page, 1400);
    await expect(page.locator(".hc-context")).toHaveCount(1);

    // The resize §4 actually describes: full width down to a narrow column, in one go.
    await page.setViewportSize({ width: 400, height: 700 });
    await page.waitForFunction(() => document.querySelectorAll(".hc-context").length === 0);

    const overflow = await page.evaluate(
      () => document.body.scrollWidth - document.body.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);
  });
});

test.describe("the DOM stays bounded", () => {
  test("ten thousand results render a viewport's worth of rows", async ({ page }) => {
    await open(page, 1400);

    const counts = await page.evaluate(() => ({
      rows: document.querySelectorAll(".hc-row").length,
      total: document.querySelectorAll("*").length,
    }));

    // A 700 px viewport at 28 px per row is 25 rows, plus six of overscan at each edge.
    expect(counts.rows).toBeLessThan(40);
    // The whole document, not just the list: a bounded list inside an unbounded wrapper would
    // pass the first assertion and fail the milestone.
    expect(counts.total).toBeLessThan(400);
  });

  test("the count stays bounded after scrolling deep into the list", async ({ page }) => {
    await open(page, 1400);

    await page.evaluate(() => {
      const list = document.querySelector(".hc-list") as HTMLElement;
      list.scrollTop = 5000 * 28;
      list.dispatchEvent(new Event("scroll"));
    });
    await page.waitForTimeout(100);

    const rows = await page.locator(".hc-row").count();
    expect(rows).toBeLessThan(40);

    // And the rows shown are the ones that belong there, not the first ones re-rendered.
    const firstName = await page.locator(".hc-row-name").first().textContent();
    expect(firstName).not.toBe("module-0.ts");
  });

  test("the scrollable height is the full list, so the scrollbar does not lie", async ({ page }) => {
    await open(page, 1400);
    const height = await page.evaluate(
      () => (document.querySelector(".hc-list > div") as HTMLElement).offsetHeight,
    );
    expect(height).toBe(10_000 * 28);
  });
});

/**
 * A resident window spends most of its life hidden, and a hidden GTK window is not a suspended
 * page: WebKit keeps running the document. Whatever this interface does periodically, it does
 * forever, for nobody — which measured, on this machine, as 99% of a CPU core with nothing on
 * screen.
 */
test.describe("while nobody is looking", () => {
  async function conceal(page: Page) {
    await page.evaluate(() => {
      (window as unknown as { __hcEmit?: (e: string, p?: unknown) => void }).__hcEmit?.(
        "window:visible",
        false,
      );
    });
  }

  test("nothing animates once the window is put away", async ({ page }) => {
    // With work to report, so there is an animation for hiding the window to stop. The idle
    // window does not animate at all any more (D59), and asserting this rule from that state
    // would pass with the rule deleted.
    await page.setViewportSize({ width: 1200, height: 700 });
    await page.goto("/harness.html?at=/&pending=500");
    await page.waitForSelector(".hc-row");

    const breath = page.locator(".hc-breath");
    await expect
      .poll(() => breath.evaluate((el) => getComputedStyle(el).animationName))
      .toBe("hc-breathe");

    await conceal(page);
    await expect(page.locator(".hc-overlay")).toHaveAttribute("data-visible", "no");
    await expect
      .poll(() => breath.evaluate((el) => getComputedStyle(el).animationName))
      .toBe("none");
  });

  test("the daemon is left alone until the window comes back", async ({ page }) => {
    await page.setViewportSize({ width: 1200, height: 700 });
    await page.goto("/harness.html?at=/");
    await page.waitForSelector(".hc-row");
    await conceal(page);

    const asked = () =>
      page.evaluate(
        () =>
          ((window as unknown as { __hcCalls?: { command: string }[] }).__hcCalls ?? []).filter(
            (c) => c.command === "extract_status",
          ).length,
      );
    const before = await asked();

    // Longer than the poll's own interval would be if it were still running.
    await page.waitForTimeout(1200);
    expect(await asked()).toBe(before);

    // And it resumes when the window does — a heartbeat that never came back would be worse
    // than one that costs something.
    await page.evaluate(() => {
      (window as unknown as { __hcEmit?: (e: string, p?: unknown) => void }).__hcEmit?.(
        "window:visible",
        true,
      );
    });
    await expect.poll(asked).toBeGreaterThan(before);
  });
});

/**
 * M11 §10: the Agent Inspector at every width M4 names.
 *
 * The panel is the newest thing in this window and the one most likely to overflow: it holds
 * paths, memory text and a two-column split, and at 400 px there is no second column to split
 * into. Measured in a browser for the same reason the rest of this file is — happy-dom has no
 * box model to be wrong about.
 */
test.describe("the Agent Inspector holds its width", () => {
  async function openInspector(page: Page, width: number, tab: string) {
    await page.setViewportSize({ width, height: 700 });
    await page.goto(`/harness.html?at=/&hits=50`);
    await page.getByLabel("Agenten-Inspektor").click();
    await page.getByRole("tab", { name: tab }).click();
    await page.waitForSelector(".hc-inspector");
  }

  for (const width of WIDTHS) {
    for (const tab of ["Live", "Gedächtnis", "Regeln"]) {
      test(`${tab} does not overflow at ${width} px`, async ({ page }) => {
        await openInspector(page, width, tab);
        // Give the fixture's answers a frame to land, so the measurement is of a full panel.
        await page.waitForTimeout(150);

        const overflow = await page.evaluate(() => {
          const panel = document.querySelector(".hc-inspector") as HTMLElement;
          const rows = Array.from(
            document.querySelectorAll(".hc-row-item, .hc-inspector-facts dd"),
          ) as HTMLElement[];
          return {
            body: document.body.scrollWidth - document.body.clientWidth,
            panel: panel.scrollWidth - panel.clientWidth,
            widestRow: rows.length
              ? Math.max(...rows.map((row) => row.scrollWidth - row.clientWidth))
              : 0,
          };
        });

        expect(overflow.body, `body overflows by ${overflow.body}px`).toBeLessThanOrEqual(0);
        expect(overflow.panel, `the panel overflows by ${overflow.panel}px`).toBeLessThanOrEqual(0);
        expect(
          overflow.widestRow,
          `a row overflows its own box by ${overflow.widestRow}px`,
        ).toBeLessThanOrEqual(0);
      });
    }
  }

  test("Escape closes it and gives the window back", async ({ page }) => {
    await openInspector(page, 900, "Live");
    await page.keyboard.press("Escape");
    expect(await page.locator(".hc-inspector").count()).toBe(0);
    // And the search field has the caret again, which is what "gives the window back" means
    // for a launcher: the next keystroke is a query and not a lost one.
    const focused = await page.evaluate(() => document.activeElement?.getAttribute("aria-label"));
    expect(focused).toBe("Suche");
  });

  test("the button and Ctrl+Shift+A are the same control", async ({ page }) => {
    await page.setViewportSize({ width: 900, height: 700 });
    await page.goto("/harness.html?at=/&hits=50");
    // The shortcut is a React handler on the window's own element, so a keystroke only reaches
    // it when the focus is already inside — the app puts the caret in the search field on
    // mount. A key pressed into the gap before that effect has run goes to `<body>`, bubbles
    // past the handler and is gone; nothing presses again, so the wait below times out after
    // 30 s with no panel. That is the whole of what happened once under parallel load.
    await expect(page.getByLabel("Suche")).toBeFocused();
    await page.keyboard.press("Control+Shift+A");
    await page.waitForSelector(".hc-inspector");
    // Wait for the panel to have taken focus before pressing again. Opening moves focus in an
    // effect, and a second keystroke sent into the middle of that is a race — this failed once
    // under parallel load and passed every time it was run alone, which is the signature.
    await expect
      .poll(() => page.evaluate(() => document.activeElement?.getAttribute("role") ?? ""))
      .toBe("tab");
    await page.keyboard.press("Control+Shift+A");
    await expect(page.locator(".hc-inspector")).toHaveCount(0);
  });
});

/**
 * The line between two rows is the kit's, and it is the kind of detail that is easy to port by eye and
 * get wrong: the kit draws it as `::after` on the row — `left-[76px] right-3 h-px bg-hairline`, i.e.
 * starting past the icon column and stopping 12 px short of the right edge — and hides it under the
 * row that is selected. Both ends matter (a line touching the row's corner curve reads as a box, not
 * as a list) and the hidden one matters more: a rule through the selected fill is the seam §8.2 keeps
 * out of the selection.
 *
 * Geometry, not pixels: the pseudo-element's used values are what the engine resolved, which is the
 * only way to tell a rule that exists from a rule that a later declaration overwrote.
 */
test.describe("the rule between two rows", () => {
  test("runs from past the icon column to 12 px short of the edge, and stops at a selection", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1400, height: 800 });
    await page.goto("/harness.html?at=/");
    await page.waitForSelector(".hc-row");
    await page.locator(".hc-row").first().click();

    const rule = await page.evaluate(() => {
      const read = (selector: string, index: number) => {
        const row = document.querySelectorAll(selector)[index] as HTMLElement;
        const after = getComputedStyle(row, "::after");
        const box = row.getBoundingClientRect();
        return {
          opacity: after.opacity,
          height: after.height,
          left: after.left,
          right: after.right,
          background: after.backgroundColor,
          width: Math.round(box.width),
        };
      };
      return {
        rows: document.querySelectorAll(".hc-row").length,
        selected: read(".hc-row-selected", 0),
        plain: read(".hc-row:not(.hc-row-selected):not(.hc-row-cursor)", 0),
      };
    });

    expect(rule.rows).toBeGreaterThan(2);
    // The line itself: one pixel, inset at both ends.
    expect(rule.plain.height).toBe("1px");
    expect(rule.plain.opacity).toBe("1");
    expect(parseFloat(rule.plain.left), "die Linie beginnt an der Zeilenkante").toBeGreaterThan(4);
    expect(parseFloat(rule.plain.right), "die Linie läuft bis in die Ecke").toBeGreaterThan(8);
    expect(rule.plain.background).not.toBe("rgba(0, 0, 0, 0)");
    // And on the selected row it is gone, rather than drawn over the fill.
    expect(rule.selected.opacity).toBe("0");
    // Something was actually selected by that click — otherwise the line above is trivially true.
    expect(rule.selected.width).toBe(rule.plain.width);
  });
});