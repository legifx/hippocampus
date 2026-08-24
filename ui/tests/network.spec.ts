/**
 * M6's network locations, in the window.
 *
 * The daemon decides which mounts are shares and what to call them, and its own tests cover
 * that. What is only visible here is the half a person meets: that a share is a row you can open
 * like any other, that it does not pretend to be a drive, and — the one that decides whether any
 * of this is usable — that a share whose server has gone produces a sentence rather than a
 * frozen window.
 */

import { expect, test, type Page } from "@playwright/test";

async function openAtPlaces(page: Page, query = "") {
  await page.setViewportSize({ width: 1200, height: 700 });
  await page.goto(`/harness.html${query}`);
  await page.waitForSelector(".hc-place");
}

test.describe("shares in the places screen", () => {
  test("they are their own section, with the address under the name", async ({ page }) => {
    await openAtPlaces(page);

    const network = page.locator(".hc-places-section", { hasText: "Netzwerk" });
    const rows = network.locator(".hc-place");
    await expect(rows).toHaveCount(2);
    await expect(rows.first()).toContainText("fotos auf nas");
    // The address is what tells two shares called `fotos` apart.
    await expect(rows.first()).toContainText("smb://nas/fotos");
  });

  test("a share is not a drive: no bar, no capacity, its own mark", async ({ page }) => {
    await openAtPlaces(page);

    const share = page.locator(".hc-place", { hasText: "fotos auf nas" });
    // Measuring how full a share is means a call that can block, so the daemon does not make it
    // and there is nothing here to draw.
    await expect(share.locator(".hc-place-bar")).toHaveCount(0);
    await expect(share.locator(".hc-place-mark")).toHaveAttribute("data-kind", "network");

    const drive = page.locator(".hc-place", { hasText: "Arbeit" });
    const bar = drive.locator(".hc-place-bar");
    await expect(bar).toBeVisible();
    // And the bar keeps its width. Making the column beside the name shrinkable — so a long
    // address gives way — squashed it to a stub the first time, which is a bar showing a wrong
    // number rather than a smaller one.
    expect((await bar.boundingBox())?.width).toBe(56);
  });

  test("the name survives a long address, and a narrow window", async ({ page }) => {
    await openAtPlaces(page);
    const share = page.locator(".hc-place", { hasText: "backup auf server" });

    // `server:/export/backup` is an ordinary address and it is longer than the name. The name
    // is the thing somebody reads; the address is what gives way.
    await expect(share.locator(".hc-place-name")).toHaveText("backup auf server");
    const wide = await share.locator(".hc-place-name").boundingBox();
    expect(wide?.width ?? 0).toBeGreaterThan(40);

    await page.setViewportSize({ width: 400, height: 700 });
    await page.waitForTimeout(150);
    const narrow = await share.locator(".hc-place-name").boundingBox();
    expect(narrow?.width ?? 0).toBeGreaterThan(20);

    const overflow = await page.evaluate(
      () => document.body.scrollWidth - document.body.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);
  });

  test("opening one browses it like any other directory", async ({ page }) => {
    await openAtPlaces(page);

    await page.locator(".hc-place", { hasText: "fotos auf nas" }).dblclick();
    await page.waitForSelector(".hc-row");

    const asked = (await page.evaluate(
      () => (window as unknown as { __hcCalls?: { command: string; args: Record<string, unknown> }[] }).__hcCalls ?? [],
    )) as { command: string; args: Record<string, unknown> }[];
    // The window opens a view over a directory source — one call for every kind of place (M10).
    const opened = asked.filter((c) => c.command === "view_open");
    const last = opened[opened.length - 1]!.args.source as { kind: string; path: string };
    expect(last.kind).toBe("dir");
    expect(last.path).toContain("smb-share:server=nas");
  });

  test("a share that has gone says so instead of hanging", async ({ page }) => {
    await openAtPlaces(page, "?deadshare=1");

    await page.locator(".hc-place", { hasText: "backup auf server" }).dblclick();

    // The daemon's own sentence, forwarded rather than rewritten: it names the share and the
    // deadline, which is what somebody needs to tell "gone" from "slow".
    const empty = page.locator(".hc-empty");
    await expect(empty).toContainText("did not answer");
    await expect(empty).toContainText("2000 ms");
    // And the window is still a window: the search field still takes a keystroke.
    await page.keyboard.type("x");
    await expect(page.getByLabel("Suche")).toHaveValue("x");
  });
});
