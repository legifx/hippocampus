/**
 * M6's shape: one list, two modes.
 *
 * "Search as an **overlay**, not a separate page — results replace the list live." That is a
 * claim about what happens to the *same* list when somebody types, so it is tested by typing
 * and looking at the list rather than by checking that two components exist.
 */

import { expect, test, type Page } from "@playwright/test";

/** Everything the window asked the backend to do, in order. */
async function calls(page: Page): Promise<{ command: string; args: Record<string, unknown> }[]> {
  return page.evaluate(
    () =>
      (window as unknown as { __hcCalls?: { command: string; args: Record<string, unknown> }[] })
        .__hcCalls ?? [],
  );
}

async function open(page: Page, width = 1200) {
  await page.setViewportSize({ width, height: 700 });
  await page.goto("/harness.html?at=/");
  await page.waitForSelector(".hc-row");
}

test.describe("browsing", () => {
  test("an empty query shows a directory rather than an invitation to type", async ({ page }) => {
    await open(page);

    await expect(page.locator(".hc-crumbs")).toBeVisible();
    // The fixture's directory: two folders and three files.
    await expect(page.locator(".hc-row")).toHaveCount(5);
    await expect(page.locator(".hc-row-directory")).toHaveCount(2);
  });

  test("typing replaces the list and clearing brings it back", async ({ page }) => {
    await open(page);
    const crumbs = page.locator(".hc-crumbs");
    await expect(crumbs).toBeVisible();

    await page.getByLabel("Suche").fill("query");
    // The same list, different contents — and the breadcrumb goes, because a result list is
    // not in a directory.
    await expect(crumbs).toHaveCount(0);
    await expect(page.locator(".hc-row").first()).toBeVisible();

    // Escape rather than `fill("")`: Playwright empties a field by inserting an empty string,
    // which fires no input event, so a controlled React input never hears about it and reverts.
    // Escape is also what a person presses, and what the app documents.
    await page.keyboard.press("Escape");
    await expect(page.getByLabel("Suche")).toHaveValue("");
    await expect(crumbs).toBeVisible();
    await expect(page.locator(".hc-row")).toHaveCount(5);
  });

  test("Enter descends into a directory and Backspace comes back up", async ({ page }) => {
    await open(page);

    const first = page.locator(".hc-row").first();
    const name = await first.locator(".hc-row-name").textContent();
    await page.keyboard.press("Enter");

    // The breadcrumb is the evidence: the last segment is the directory just entered.
    const crumb = page.locator('.hc-crumb[aria-current="page"]');
    await expect(crumb).toHaveText(name!.trim());

    await page.keyboard.press("Backspace");
    await expect(page.locator('.hc-crumb[aria-current="page"]')).toHaveText("/");
  });

  test("Backspace does not navigate while there is something to delete", async ({ page }) => {
    await open(page);
    await page.getByLabel("Suche").fill("quer");

    await page.keyboard.press("Backspace");
    await expect(page.getByLabel("Suche")).toHaveValue("que");
    // Still searching, still not somewhere else.
    await expect(page.locator(".hc-crumbs")).toHaveCount(0);
  });

  test("a breadcrumb segment is a way back", async ({ page }) => {
    await open(page);
    await page.keyboard.press("Enter");
    await expect(page.locator('.hc-crumb[aria-current="page"]')).not.toHaveText("/");

    // The root crumb, not the first one — the first is "Orte", which leaves the filesystem
    // for the places screen and is tested separately.
    await page.locator(".hc-crumb", { hasText: /^\/$/ }).first().click();
    await expect(page.locator('.hc-crumb[aria-current="page"]')).toHaveText("/");
  });

  test("Alt+Left goes back through where you have been", async ({ page }) => {
    await open(page);
    await page.keyboard.press("Enter");
    const descended = await page.locator('.hc-crumb[aria-current="page"]').textContent();

    await page.keyboard.press("Backspace");
    await expect(page.locator('.hc-crumb[aria-current="page"]')).toHaveText("/");

    // Back is history, not the parent: it returns to the directory that was left.
    await page.keyboard.press("Alt+ArrowLeft");
    await expect(page.locator('.hc-crumb[aria-current="page"]')).toHaveText(descended!.trim());
  });

  test("the deny-list's hidden entries are counted and not named", async ({ page }) => {
    await open(page);
    // The fixture reports one hidden entry. It must appear as a number and nowhere else.
    await expect(page.locator(".hc-status")).toContainText("+1");
  });

  test("a directory reads as a place, with a mark rather than only a colour", async ({ page }) => {
    await open(page);
    const directories = page.locator(".hc-row-directory");
    await expect(directories).toHaveCount(2);
    // D15: colour is never the only signal.
    await expect(directories.first().locator(".hc-row-dirmark")).toHaveCount(1);
  });

  test("the layout still holds at 400 px while browsing", async ({ page }) => {
    await open(page, 400);
    const overflow = await page.evaluate(
      () => document.body.scrollWidth - document.body.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);
    // The breadcrumb is the new thing that could push the window sideways.
    const crumbs = await page.evaluate(() => {
      const el = document.querySelector(".hc-crumbs") as HTMLElement;
      return el.scrollWidth - el.clientWidth;
    });
    expect(crumbs).toBeLessThanOrEqual(0);
  });
});

/**
 * M6 §1: "permissions, ownership, symbolic links displayed correctly".
 *
 * Correctly is the load-bearing word. A link drawn as its target is indistinguishable from a
 * copy, and that is the difference somebody needs before they decide to delete one of them.
 */
test.describe("what a row says beyond its name", () => {
  /** The fixture puts links one level down, so the root listing stays as the layout tests measure it. */
  async function descend(page: Page) {
    await page.keyboard.press("Enter");
    await expect(page.locator('.hc-crumb[aria-current="page"]')).not.toHaveText("/");
  }

  test("a link shows where it points, and a broken one says it is broken", async ({ page }) => {
    await open(page);
    await descend(page);

    const link = page.locator(".hc-row", { hasText: "verweis-auf-ordner" });
    await expect(link.locator(".hc-row-link")).toContainText("→");
    await expect(link.locator(".hc-row-link")).toContainText("anderswo");

    const broken = page.locator(".hc-row", { hasText: "kaputter-verweis" });
    await expect(broken.locator(".hc-row-link-broken")).toBeVisible();
    await expect(broken.locator(".hc-row-broken")).toHaveText("fehlt");

    // Relative links are shown as written rather than resolved: that is what the link says.
    const relative = page.locator(".hc-row", { hasText: "relativer-verweis" });
    await expect(relative.locator(".hc-row-link")).toContainText("kurz.txt");
  });

  test("a link to a directory opens like a directory", async ({ page }) => {
    await open(page);
    await descend(page);

    // It is still drawn as a link — but the marker and Enter both treat it as the place it is.
    const link = page.locator(".hc-row", { hasText: "verweis-auf-ordner" });
    await expect(link).toHaveClass(/hc-row-directory/);
  });

  test("permissions are shown in a wide window and dropped in a narrow one", async ({ page }) => {
    await open(page, 1200);
    await descend(page);

    const first = page.locator(".hc-row").first();
    await expect(first.locator(".hc-row-mode")).toHaveText("drwxr-xr-x");

    // Ten characters of monospace is the first thing to go when the column narrows: the name
    // is what identifies a row, and permissions are read deliberately rather than at a glance.
    await page.setViewportSize({ width: 400, height: 700 });
    await expect(first.locator(".hc-row-mode")).toBeHidden();
  });

  test("the sidebar names the owner and the link target", async ({ page }) => {
    await open(page, 1200);
    await descend(page);

    const context = page.locator(".hc-context");
    await expect(context.getByText("someone:someone")).toBeVisible();

    // Down to the broken link — the sidebar settles 120 ms after the selection stops.
    for (let i = 0; i < 6; i++) await page.keyboard.press("ArrowDown");
    await expect(context.getByText("/home/user/weg.txt — fehlt")).toBeVisible();
  });
});

/**
 * The first screen.
 *
 * A file manager that opens at `/` opens at `bin boot dev etc lib opt proc run sbin srv sys tmp
 * usr var` — sixteen directories of which one is ever wanted, with everything a person actually
 * uses four levels below it. These tests are about the screen that replaces that.
 */
test.describe("the places screen", () => {
  /** The default: no `?at=`, so the overlay opens where it really opens. */
  async function openAtPlaces(page: Page, width = 1200) {
    await page.setViewportSize({ width, height: 700 });
    await page.goto("/harness.html");
    await page.waitForSelector(".hc-place");
  }

  test("the overlay opens at the places, not at the filesystem root", async ({ page }) => {
    await openAtPlaces(page);

    // No directory listing at all, and no breadcrumb: this is not a directory.
    await expect(page.locator(".hc-row")).toHaveCount(0);
    await expect(page.locator(".hc-crumbs")).toHaveCount(0);
    await expect(page.locator(".hc-places-title").first()).toHaveText("Persönlich");
  });

  test("everything technical is one row, at the end", async ({ page }) => {
    await openAtPlaces(page);

    const headings = page.locator(".hc-places-title");
    await expect(headings).toHaveText([
      "Persönlich",
      "Laufwerke",
      "Netzwerk",
      "Gedächtnis",
      "System",
    ]);

    // The whole point: one row called what it is, rather than sixteen in the way.
    const system = page.locator(".hc-places-section").last().locator(".hc-place");
    await expect(system).toHaveCount(1);
    await expect(system).toContainText("Dateisystem");
  });

  test("the folders a person uses come first", async ({ page }) => {
    await openAtPlaces(page);
    const personal = page.locator(".hc-places-section").first().locator(".hc-place-name");
    await expect(personal).toHaveText([
      "Persönlicher Ordner",
      "Downloads",
      "Dokumente",
      "Papierkorb",
    ]);
  });

  test("a drive shows how much room is left", async ({ page }) => {
    await openAtPlaces(page);
    // By the exact name rather than by substring: since the memory section arrived, "Arbeit"
    // also appears inside "Woran ich gerade arbeite", and a substring match resolved to two
    // rows. A locator that is nearly right is a test that fails for the wrong reason.
    const drive = page
      .locator(".hc-place")
      .filter({ has: page.getByText("Arbeit", { exact: true }) });
    await expect(drive.locator(".hc-place-bar")).toBeVisible();
    await expect(drive).toContainText("frei");

    // At 400 px the name is what identifies a drive; the bar is the first thing to go.
    await page.setViewportSize({ width: 400, height: 700 });
    await expect(drive.locator(".hc-place-bar")).toBeHidden();
  });

  test("arrow keys move through the places across the headings", async ({ page }) => {
    await openAtPlaces(page);
    await expect(page.locator('.hc-place[aria-selected="true"]')).toHaveCount(1);

    // Four personal places, then the first drive: the fifth press crosses a heading, because
    // the cursor counts places and not rows on screen.
    for (let i = 0; i < 4; i++) await page.keyboard.press("ArrowDown");
    await expect(page.locator('.hc-place[aria-selected="true"]')).toContainText("Arbeit");

    await page.keyboard.press("End");
    await expect(page.locator('.hc-place[aria-selected="true"]')).toContainText("Dateisystem");
    await page.keyboard.press("Home");
    await expect(page.locator('.hc-place[aria-selected="true"]')).toContainText(
      "Persönlicher Ordner",
    );
  });

  test("a drive that is not mounted says so, and shows its size instead of how full it is", async ({
    page,
  }) => {
    await openAtPlaces(page);
    const stick = page.locator(".hc-place", { hasText: "ESD-USB" });
    await expect(stick).toContainText("nicht eingehängt");
    // No bar: how full it is cannot be known before it is mounted, and a bar at zero would be
    // a measurement nobody made.
    await expect(stick.locator(".hc-place-bar")).toHaveCount(0);
  });

  test("opening one mounts it first and goes where it landed", async ({ page }) => {
    await openAtPlaces(page);
    for (let i = 0; i < 5; i++) await page.keyboard.press("ArrowDown");
    await expect(page.locator('.hc-place[aria-selected="true"]')).toContainText("ESD-USB");
    await page.keyboard.press("Enter");

    // The device was handed to the daemon, and the window went to the mount point the daemon
    // answered with — not to a path guessed from the label, which is where a stick with an
    // unusual name would not be.
    await expect
      .poll(() => page.evaluate(() => (window as unknown as { __hcMounted?: string }).__hcMounted))
      .toBe("/dev/sdb1");
    await expect(page.locator('.hc-crumb[aria-current="page"]')).toHaveText("ESD-USB");
    await expect(page.locator(".hc-row").first()).toBeVisible();
  });

  test("a refused mount says what is wrong, in words", async ({ page }) => {
    // What this is guarding: the daemon's errors arrive as `{ code, message }` objects, not as
    // strings, and the first version of the places screen ran `String(e)` over one. The window
    // then said `[object Object]` for every internal disk, which is how the bug was reported —
    // as "everything is broken and it hangs".
    await page.goto("/harness.html?mountfails=1");
    await page.waitForSelector(".hc-place");
    for (let i = 0; i < 5; i++) await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Enter");

    await expect(page.locator(".hc-banner")).toContainText("permission");
    await expect(page.locator(".hc-banner")).not.toContainText("[object Object]");

    // And the window still works afterwards: the cursor moves, and another place opens.
    await page.keyboard.press("ArrowUp");
    await expect(page.locator('.hc-place[aria-selected="true"]')).toContainText("Arbeit");
  });

  test("a disk that needs a password is asked for a second time", async ({ page }) => {
    // The daemon never waits on a person, so it refuses with a code that means "a password
    // would do it" — and the window asks again, which is the moment polkit puts a dialog in
    // front of somebody who just pressed Enter and is therefore still there to answer.
    await page.goto("/harness.html?needsauth=1");
    await page.waitForSelector(".hc-place");
    for (let i = 0; i < 5; i++) await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Enter");

    // It ends up in the directory, which only happens if the second call was made.
    await expect(page.locator('.hc-crumb[aria-current="page"]')).toHaveText("ESD-USB");

    const mounts = (await calls(page)).filter((c) => c.command === "mount_volume");
    expect(mounts.map((c) => c.args.interactive)).toEqual([false, true]);
  });

  test("Ctrl+E ejects the drive under the cursor", async ({ page }) => {
    await openAtPlaces(page);
    for (let i = 0; i < 4; i++) await page.keyboard.press("ArrowDown");
    await expect(page.locator('.hc-place[aria-selected="true"]')).toContainText("Arbeit");
    await page.keyboard.press("Control+e");

    await expect
      .poll(() =>
        page.evaluate(
          () => (window as unknown as { __hcEjected?: { device: string; eject: boolean } }).__hcEjected,
        ),
      )
      .toEqual({ device: "/dev/sdc1", eject: true });
    // And it says what happened, because "can be unplugged" is the whole reason somebody
    // pressed it.
    await expect(page.locator(".hc-banner")).toContainText("abgezogen");
  });

  test("Ctrl+E on something that is not a drive does nothing", async ({ page }) => {
    // The places screen holds folders too, and unmounting a home directory is not a thing.
    await openAtPlaces(page);
    await page.keyboard.press("Control+e");
    expect(
      await page.evaluate(() => (window as unknown as { __hcEjected?: unknown }).__hcEjected),
    ).toBeUndefined();
  });

  test("Enter opens a place and the breadcrumb takes you back", async ({ page }) => {
    await openAtPlaces(page);
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Enter");

    // A directory now: rows, a breadcrumb, and the path of the place that was opened.
    await expect(page.locator(".hc-row").first()).toBeVisible();
    await expect(page.locator('.hc-crumb[aria-current="page"]')).toHaveText("Downloads");

    await page.locator(".hc-crumb-places").click();
    await expect(page.locator(".hc-place").first()).toBeVisible();
    await expect(page.locator(".hc-row")).toHaveCount(0);
  });

  test("Backspace above the root comes back to the places", async ({ page }) => {
    await open(page); // starts at `/`
    await expect(page.locator(".hc-row").first()).toBeVisible();

    // Up from `/` is not "nowhere". It is where everything else is reachable from.
    await page.keyboard.press("Backspace");
    await expect(page.locator(".hc-place").first()).toBeVisible();
  });

  test("typing searches from the places screen too", async ({ page }) => {
    await openAtPlaces(page);
    await page.getByLabel("Suche").fill("query");

    // One window, three screens, and typing always wins.
    await expect(page.locator(".hc-place")).toHaveCount(0);
    await expect(page.locator(".hc-row").first()).toBeVisible();

    await page.keyboard.press("Escape");
    await expect(page.locator(".hc-place").first()).toBeVisible();
  });

  test("nothing on the places screen can be cut, copied or deleted", async ({ page }) => {
    await openAtPlaces(page);
    await page.keyboard.press("Control+c");
    await page.keyboard.press("Delete");

    // You do not cut a drive. Nothing was asked of the daemon, and nothing said otherwise.
    const asked = await page.evaluate(
      () =>
        ((window as unknown as { __hcCalls?: { command: string }[] }).__hcCalls ?? []).filter(
          (c) => c.command === "trash" || c.command === "clipboard_write",
        ).length,
    );
    expect(asked).toBe(0);
  });
});
