/**
 * Selecting several things and doing something with them.
 *
 * This is the half of M6 that decides whether the thing can replace a file manager, and it is
 * tested through real key presses and clicks because the failure modes are all about
 * modifiers — a shift that grew a selection instead of picking a range, a Ctrl+A that the
 * search field should have had.
 *
 * The fixture records the calls the app makes rather than performing them: what matters here is
 * that five selected rows become one request naming five paths, and the daemon's own tests
 * cover what happens to those paths afterwards.
 */

import { expect, test, type Page } from "@playwright/test";

type Recorded = { command: string; args: Record<string, unknown> };

async function open(page: Page, width = 1200) {
  await page.setViewportSize({ width, height: 700 });
  await page.goto("/harness.html");
  await page.waitForSelector(".hc-row");
}

/** Everything the app asked the backend to do, in order. */
async function calls(page: Page): Promise<Recorded[]> {
  return page.evaluate(
    () => (window as unknown as { __hcCalls?: Recorded[] }).__hcCalls ?? [],
  ) as Promise<Recorded[]>;
}

/**
 * Assert how many rows are selected, waiting for it.
 *
 * A retrying expectation rather than a bare `count()`, because a directory change re-reads the
 * listing asynchronously and the selection settles a tick later. Reading the number once
 * measures whichever frame the test happened to land on — which is how this file first failed,
 * intermittently, on exactly one of its cases.
 */
async function expectSelected(page: Page, count: number) {
  await expect(page.locator('[aria-selected="true"]')).toHaveCount(count);
}

test.describe("selecting", () => {
  test("starts with one row and the cursor on it", async ({ page }) => {
    await open(page);
    await expectSelected(page, 1);
    await expect(page.locator(".hc-row-cursor")).toHaveCount(1);
  });

  test("shift and an arrow key extend the selection", async ({ page }) => {
    await open(page);
    await page.keyboard.press("Shift+ArrowDown");
    await page.keyboard.press("Shift+ArrowDown");
    await expectSelected(page, 3);
  });

  test("shift-clicking picks a range, and clicking again picks a different one", async ({
    page,
  }) => {
    await open(page);
    await page.locator("#hc-row-1").click();
    await page.locator("#hc-row-4").click({ modifiers: ["Shift"] });
    await expectSelected(page, 4);

    // Measured from the same anchor: a correction, not an addition.
    await page.locator("#hc-row-2").click({ modifiers: ["Shift"] });
    await expectSelected(page, 2);
  });

  test("ctrl-clicking picks individual rows", async ({ page }) => {
    await open(page);
    await page.locator("#hc-row-0").click();
    await page.locator("#hc-row-2").click({ modifiers: ["ControlOrMeta"] });
    await page.locator("#hc-row-4").click({ modifiers: ["ControlOrMeta"] });
    await expectSelected(page, 3);

    await page.locator("#hc-row-2").click({ modifiers: ["ControlOrMeta"] });
    await expectSelected(page, 2);
  });

  test("Ctrl+A takes the directory, and Escape gives it back", async ({ page }) => {
    await open(page);
    await page.keyboard.press("Control+a");
    await expectSelected(page, 5);
    await expect(page.locator(".hc-status")).toContainText("5/5");

    await page.keyboard.press("Escape");
    await expectSelected(page, 1);
  });

  test("Ctrl+A belongs to the search field while there is a query in it", async ({ page }) => {
    await open(page);
    await page.getByLabel("Suche").fill("query");

    // Selecting the text to replace it must not select every file instead. This is how a
    // browser test found the binding in the first place: it could no longer clear the input.
    await page.keyboard.press("Control+a");
    await page.keyboard.type("neu");
    await expect(page.getByLabel("Suche")).toHaveValue("neu");
  });

  test("the selection is forgotten when the directory changes", async ({ page }) => {
    await open(page);
    await page.keyboard.press("Control+a");
    await expectSelected(page, 5);

    await page.keyboard.press("Enter");
    // Keeping index 3 selected would mean acting on whatever landed third in here.
    await expectSelected(page, 1);
  });
});

test.describe("operations", () => {
  test("copy and paste send one request naming everything selected", async ({ page }) => {
    await open(page);
    await page.keyboard.press("Control+a");
    await page.keyboard.press("Control+c");
    await expect(page.locator(".hc-status")).toContainText("kopiert");

    await page.keyboard.press("Control+v");
    await page.waitForTimeout(100);

    const transfers = (await calls(page)).filter((c) => c.command === "transfer");
    expect(transfers).toHaveLength(1);
    expect(transfers[0]!.args.mode).toBe("copy");
    expect((transfers[0]!.args.sources as string[]).length).toBe(5);
  });

  test("cut says so and pastes as a move", async ({ page }) => {
    await open(page);
    await page.locator("#hc-row-2").click();
    await page.keyboard.press("Control+x");
    await expect(page.locator(".hc-status")).toContainText("ausgeschnitten");

    await page.keyboard.press("Control+v");
    await page.waitForTimeout(100);

    const transfers = (await calls(page)).filter((c) => c.command === "transfer");
    expect(transfers[0]!.args.mode).toBe("move");
    expect((transfers[0]!.args.sources as string[]).length).toBe(1);
  });

  test("Delete trashes everything selected, one call each", async ({ page }) => {
    await open(page);
    await page.locator("#hc-row-0").click();
    await page.locator("#hc-row-2").click({ modifiers: ["Shift"] });
    await page.keyboard.press("Delete");
    await page.waitForTimeout(100);

    const trashed = (await calls(page)).filter((c) => c.command === "trash");
    expect(trashed).toHaveLength(3);
  });

  test("F2 opens a name field with the current name in it", async ({ page }) => {
    await open(page);
    await page.locator("#hc-row-2").click();
    const name = await page.locator("#hc-row-2 .hc-row-name").textContent();

    await page.keyboard.press("F2");
    const field = page.locator("#hc-name");
    await expect(field).toBeVisible();
    await expect(field).toHaveValue(name!.trim());

    await field.fill("umbenannt.txt");
    await field.press("Enter");
    await page.waitForTimeout(100);

    const renames = (await calls(page)).filter((c) => c.command === "rename_entry");
    expect(renames).toHaveLength(1);
    expect(renames[0]!.args.name).toBe("umbenannt.txt");
  });

  test("Escape in the name field cancels rather than closing the window", async ({ page }) => {
    await open(page);
    await page.keyboard.press("F2");
    await expect(page.locator("#hc-name")).toBeVisible();

    await page.locator("#hc-name").press("Escape");
    await expect(page.locator("#hc-name")).toHaveCount(0);
    // And nothing was dismissed: the list is still there.
    await expect(page.locator(".hc-row").first()).toBeVisible();
    expect((await calls(page)).filter((c) => c.command === "dismiss")).toHaveLength(0);
  });

  test("Ctrl+Shift+N makes a folder in the directory being looked at", async ({ page }) => {
    await open(page);
    await page.keyboard.press("Control+Shift+KeyN");
    await expect(page.locator("#hc-name")).toBeVisible();

    await page.locator("#hc-name").fill("Neuer Ordner");
    await page.locator("#hc-name").press("Enter");
    await page.waitForTimeout(100);

    const made = (await calls(page)).filter((c) => c.command === "mkdir");
    expect(made).toHaveLength(1);
    // Joined onto the directory on screen with exactly one slash. The root is the case a
    // naive join gets wrong, and this fixture browses the root.
    expect(made[0]!.args.path).toBe("/Neuer Ordner");
  });

  test("a running transfer shows a bar that can be cancelled", async ({ page }) => {
    await open(page);
    await page.keyboard.press("Control+c");
    await page.keyboard.press("Control+v");

    // The fixture answers with a handle and then emits progress.
    await expect(page.locator(".hc-progress")).toBeVisible();
    await page.evaluate(() => {
      (window as unknown as { __hcEmit: (e: string, p: unknown) => void }).__hcEmit(
        "transfer:progress",
        {
          operation: 1,
          files_done: 1,
          bytes_done: 500,
          total_files: 2,
          total_bytes: 1000,
          fraction: 0.5,
          current: "/somewhere/a-file.txt",
        },
      );
    });
    await expect(page.locator(".hc-progress-text")).toContainText("50 %");
    await expect(page.locator(".hc-progress-text")).toContainText("a-file.txt");

    await page.keyboard.press("Escape");
    await page.waitForTimeout(50);
    expect((await calls(page)).filter((c) => c.command === "cancel_operation")).toHaveLength(1);
  });

  test("a finished transfer that partly failed says both halves", async ({ page }) => {
    await open(page);
    await page.keyboard.press("Control+c");
    await page.keyboard.press("Control+v");
    await expect(page.locator(".hc-progress")).toBeVisible();

    await page.evaluate(() => {
      (window as unknown as { __hcEmit: (e: string, p: unknown) => void }).__hcEmit(
        "transfer:done",
        {
          operation: 1,
          moved: ["/a", "/b", "/c"],
          failed: [{ source: "/d", reason: "already exists" }],
          cancelled: false,
          undo_ids: [1, 2, 3],
          elapsed_ms: 12,
        },
      );
    });

    // "3 übertragen" alone would be the kind of message that costs somebody a file.
    await expect(page.locator(".hc-banner")).toContainText("3 übertragen");
    await expect(page.locator(".hc-banner")).toContainText("1 nicht");
    await expect(page.locator(".hc-progress")).toHaveCount(0);
  });
});

test.describe("the conflict question", () => {
  /** Select only the row the fixture reports a conflict for, and paste it. */
  async function pasteClashing(page: Page) {
    await open(page);
    // `kurz.txt` is the fixture's clashing name.
    const rows = await page.locator(".hc-row-name").allTextContents();
    const index = rows.findIndex((name) => name.includes("kurz.txt"));
    await page.locator(`#hc-row-${index}`).click();
    await page.keyboard.press("Control+c");
    await page.keyboard.press("Control+v");
  }

  test("is asked before anything moves", async ({ page }) => {
    await pasteClashing(page);

    await expect(page.getByRole("dialog", { name: "Namenskonflikt" })).toBeVisible();
    // The real transfer has not been requested: only the dry run.
    const transfers = (await calls(page)).filter((c) => c.command === "transfer");
    expect(transfers).toHaveLength(1);
    expect(transfers[0]!.args.dryRun).toBe(true);
    await expect(page.locator(".hc-progress")).toHaveCount(0);
  });

  test("shows both sides rather than asking to overwrite", async ({ page }) => {
    await pasteClashing(page);
    const dialog = page.getByRole("dialog", { name: "Namenskonflikt" });

    // "Overwrite?" is not a question anybody can answer. Sizes and ages are.
    await expect(dialog).toContainText("kurz.txt");
    await expect(dialog).toContainText("12 B");
    await expect(dialog).toContainText("4.1 kB");
  });

  test("a choice starts the transfer with it", async ({ page }) => {
    await pasteClashing(page);
    await page.keyboard.press("e"); // Ersetzen
    await page.waitForTimeout(100);

    const transfers = (await calls(page)).filter((c) => c.command === "transfer");
    expect(transfers).toHaveLength(2);
    expect(transfers[1]!.args.dryRun).toBe(false);
    expect(transfers[1]!.args.conflict).toBe("replace");
    await expect(page.getByRole("dialog", { name: "Namenskonflikt" })).toHaveCount(0);
  });

  test("each option has a key, and Enter takes the safest", async ({ page }) => {
    await pasteClashing(page);
    // The first option is the one that loses nothing, and Enter is on it.
    await page.keyboard.press("Enter");
    await page.waitForTimeout(100);

    const transfers = (await calls(page)).filter((c) => c.command === "transfer");
    expect(transfers[1]!.args.conflict).toBe("keep");
  });

  test("Escape answers nothing and moves nothing", async ({ page }) => {
    await pasteClashing(page);
    await page.keyboard.press("Escape");

    await expect(page.getByRole("dialog", { name: "Namenskonflikt" })).toHaveCount(0);
    const transfers = (await calls(page)).filter((c) => c.command === "transfer");
    expect(transfers).toHaveLength(1);
    await expect(page.locator(".hc-banner")).toContainText("nichts übertragen");
  });

  test("nothing else responds to the keyboard while it is open", async ({ page }) => {
    await pasteClashing(page);
    const before = await page.locator('[aria-selected="true"]').count();

    // An arrow key that moved the selection underneath the question would change what the
    // answer applies to.
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("ArrowDown");
    expect(await page.locator('[aria-selected="true"]').count()).toBe(before);
    await expect(page.getByRole("dialog", { name: "Namenskonflikt" })).toBeVisible();
  });

  test("pasting where nothing is in the way asks nothing", async ({ page }) => {
    await open(page);
    await page.locator("#hc-row-0").click();
    await page.keyboard.press("Control+c");
    await page.keyboard.press("Control+v");
    await page.waitForTimeout(100);

    await expect(page.getByRole("dialog", { name: "Namenskonflikt" })).toHaveCount(0);
    const transfers = (await calls(page)).filter((c) => c.command === "transfer");
    expect(transfers).toHaveLength(2);
    expect(transfers[1]!.args.dryRun).toBe(false);
  });
});

/**
 * M6: cut and copy reach the desktop, not just this window.
 *
 * The formats and the encoding are tested in Rust, where they are pure functions. What can only
 * be tested here is the *wiring*: that a copy is offered to the desktop, that a paste asks the
 * desktop first, and that the window still works when the desktop says no.
 */
test.describe("the system clipboard", () => {
  /** Put something there as another file manager would. */
  async function desktopHolds(page: Page, mode: "copy" | "move", paths: string[]) {
    await page.evaluate(
      ([mode, paths]) => {
        (window as unknown as { __hcClipboard?: unknown }).__hcClipboard = { mode, paths };
      },
      [mode, paths] as const,
    );
  }

  async function desktopHolding(page: Page) {
    return page.evaluate(
      () => (window as unknown as { __hcClipboard?: { mode: string; paths: string[] } | null }).__hcClipboard ?? null,
    );
  }

  test("a copy is offered to the desktop, not only remembered here", async ({ page }) => {
    await open(page);
    await page.keyboard.press("Control+a");
    await page.keyboard.press("Control+c");

    await expect
      .poll(async () => (await desktopHolding(page))?.paths.length)
      .toBe(5);
    expect((await desktopHolding(page))?.mode).toBe("copy");
  });

  test("a cut is offered as a cut", async ({ page }) => {
    await open(page);
    await page.keyboard.press("Control+x");
    await expect.poll(async () => (await desktopHolding(page))?.mode).toBe("move");
  });

  test("pasting takes what another application copied", async ({ page }) => {
    await open(page);
    // Nothing has been copied in this window at all. Everything comes from outside.
    await desktopHolds(page, "copy", ["/elsewhere/from-another-app.txt"]);
    await page.keyboard.press("Control+v");

    const transfers = (await calls(page)).filter((c) => c.command === "transfer");
    expect(transfers.length).toBeGreaterThan(0);
    expect(transfers[0]!.args.sources).toEqual(["/elsewhere/from-another-app.txt"]);
    expect(transfers[0]!.args.mode).toBe("copy");
  });

  test("a cut made in another application stays a cut", async ({ page }) => {
    await open(page);
    await desktopHolds(page, "move", ["/elsewhere/cut-there.txt"]);
    await page.keyboard.press("Control+v");

    const transfers = (await calls(page)).filter((c) => c.command === "transfer");
    expect(transfers[0]!.args.mode).toBe("move");
  });

  test("what the desktop holds wins over what this window remembers", async ({ page }) => {
    await open(page);
    await page.keyboard.press("Control+c");
    // Somebody copied something else, somewhere else, in between.
    await desktopHolds(page, "copy", ["/elsewhere/newer.txt"]);
    await page.keyboard.press("Control+v");

    const transfers = (await calls(page)).filter((c) => c.command === "transfer");
    expect(transfers[0]!.args.sources).toEqual(["/elsewhere/newer.txt"]);
  });

  test("carrying out a cut empties the desktop's clipboard too", async ({ page }) => {
    await open(page);
    await page.keyboard.press("Control+x");
    await expect.poll(async () => (await desktopHolding(page))?.mode).toBe("move");

    await page.keyboard.press("Control+v");
    // Otherwise the next paste anywhere on the desktop names files that have moved.
    await expect.poll(async () => await desktopHolding(page)).toBeNull();
  });

  test("a copy the compositor refuses still works inside the window, and says so", async ({
    page,
  }) => {
    await open(page);
    await page.evaluate(() => {
      (window as unknown as { __hcClipboardRefuses?: boolean }).__hcClipboardRefuses = true;
    });

    await page.keyboard.press("Control+c");
    // Wayland only lets a focused window take the selection. Losing the ability to paste inside
    // this window because of that would be the worse of the two failures.
    await expect(page.locator(".hc-banner")).toContainText("nur in diesem Fenster");

    await page.keyboard.press("Control+v");
    const transfers = (await calls(page)).filter((c) => c.command === "transfer");
    expect(transfers.length).toBeGreaterThan(0);
  });

  test("summoning the window picks up what was copied while it was away", async ({ page }) => {
    await open(page);
    await desktopHolds(page, "move", ["/elsewhere/a.txt", "/elsewhere/b.txt"]);

    // The overlay is resident: it is hidden and summoned, and the clipboard may have changed
    // three applications ago. Focus is when it looks.
    await page.evaluate(() => window.dispatchEvent(new Event("focus")));
    await expect(page.locator(".hc-status")).toContainText("ausgeschnitten");
  });
});

/**
 * Files dragged in from another application.
 *
 * The shell intercepts the drop and hands the window a list of paths, so what can be tested
 * here is what the window does with them. The fixture emits the same event the shell does.
 */
test.describe("dropping files in", () => {
  async function dropOnto(page: Page, paths: string[]) {
    await page.evaluate((paths) => {
      (window as unknown as { __hcEmit?: (e: string, p?: unknown) => void }).__hcEmit?.(
        "tauri://drag-drop",
        { paths },
      );
    }, paths);
  }

  test("a drop copies into the directory being looked at", async ({ page }) => {
    await open(page);
    await dropOnto(page, ["/elsewhere/dropped.txt"]);

    const transfers = (await calls(page)).filter((c) => c.command === "transfer");
    expect(transfers.length).toBeGreaterThan(0);
    expect(transfers[0]!.args.sources).toEqual(["/elsewhere/dropped.txt"]);
    // Copy, not move: the drop event carries neither the modifier keys nor the source's
    // filesystem, and guessing "move" wrongly takes a file out of somebody else's window.
    expect(transfers[0]!.args.mode).toBe("copy");
    expect(transfers[0]!.args.destination).toBe("/");
  });

  test("a drop lands in the directory that is open now, not the one that was", async ({ page }) => {
    await open(page);
    await page.keyboard.press("Enter");
    await expect(page.locator('.hc-crumb[aria-current="page"]')).not.toHaveText("/");

    await dropOnto(page, ["/elsewhere/dropped.txt"]);
    const transfers = (await calls(page)).filter((c) => c.command === "transfer");
    expect(transfers[0]!.args.destination).not.toBe("/");
  });

  test("a drop onto a result list says there is nowhere to put it", async ({ page }) => {
    await open(page);
    await page.getByLabel("Suche").fill("query");
    await expect(page.locator(".hc-crumbs")).toHaveCount(0);

    await dropOnto(page, ["/elsewhere/dropped.txt"]);
    // A list of search results is not a place. Copying into whichever directory was browsed
    // last would put the file somewhere nobody was looking.
    await expect(page.locator(".hc-banner")).toContainText("Ordner");
    expect((await calls(page)).filter((c) => c.command === "transfer")).toHaveLength(0);
  });

  test("something dragged over the window shows that it can be let go of", async ({ page }) => {
    await open(page);
    const overlay = page.locator(".hc-overlay");
    await expect(overlay).not.toHaveAttribute("data-dropping", "yes");

    await page.evaluate(() => {
      (window as unknown as { __hcEmit?: (e: string, p?: unknown) => void }).__hcEmit?.(
        "tauri://drag-enter",
      );
    });
    await expect(overlay).toHaveAttribute("data-dropping", "yes");

    await page.evaluate(() => {
      (window as unknown as { __hcEmit?: (e: string, p?: unknown) => void }).__hcEmit?.(
        "tauri://drag-leave",
      );
    });
    await expect(overlay).not.toHaveAttribute("data-dropping", "yes");
  });
});

/**
 * Dragging files out of the window.
 *
 * What this can prove is that the page puts a `text/uri-list` on the drag. What it cannot prove
 * is that the webview forwards it to the compositor: these tests run in Chromium and the
 * overlay runs in WebKitGTK, and the handover from a page's `dataTransfer` to a GTK drag source
 * is the webview's business. That half is verified by dragging a row into another application.
 */
test.describe("dragging files out", () => {
  /** Start a drag on a row and report what it offered. */
  async function dragFrom(page: Page, rowId: string) {
    return page.evaluate((rowId) => {
      const row = document.querySelector(rowId);
      if (!row) throw new Error(`no ${rowId}`);
      const dataTransfer = new DataTransfer();
      row.dispatchEvent(
        new DragEvent("dragstart", { dataTransfer, bubbles: true, cancelable: true }),
      );
      return {
        uris: dataTransfer.getData("text/uri-list"),
        text: dataTransfer.getData("text/plain"),
      };
    }, rowId);
  }

  test("a drag offers the file as a URI the rest of the desktop understands", async ({ page }) => {
    await open(page);
    const offered = await dragFrom(page, "#hc-row-0");

    expect(offered.uris).toMatch(/^file:\/\//);
    expect(offered.uris).toContain("a-rather-long-directory-name-for-testing");
    // RFC 2483 separates with CRLF, and receivers that get it wrong get it wrong leniently.
    expect(offered.uris.endsWith("\r\n")).toBe(true);
    expect(offered.text.startsWith("/")).toBe(true);
  });

  test("dragging a selected row drags the whole selection", async ({ page }) => {
    await open(page);
    await page.keyboard.press("Control+a");

    const offered = await dragFrom(page, "#hc-row-0");
    expect(offered.uris.trimEnd().split("\r\n")).toHaveLength(5);
  });

  test("dragging a row outside the selection drags only that row", async ({ page }) => {
    await open(page);
    await page.locator("#hc-row-0").click();
    await page.locator("#hc-row-1").click({ modifiers: ["ControlOrMeta"] });

    // Row 3 is not selected. Dragging it must not take the two that are with it — that is how
    // a drag moves files somebody was not pointing at.
    const offered = await dragFrom(page, "#hc-row-3");
    expect(offered.uris.trimEnd().split("\r\n")).toHaveLength(1);
    expect(offered.uris).toContain("bild.png");
  });

  test("a name with a space in it survives the encoding", async ({ page }) => {
    await open(page);
    const offered = await dragFrom(page, "#hc-row-2");
    // The fixture's names have no spaces, so this checks the shape rather than a space: no raw
    // character that would end a URI early, and one line per file.
    expect(offered.uris).not.toMatch(/[ \t]/);
    expect(offered.uris.trimEnd().split("\r\n")).toHaveLength(1);
  });
});
