import { expect, test } from "@playwright/test";

/**
 * M11 in the main window: selecting a file shows what agents have learned about it.
 *
 * The Inspector is where somebody goes to audit the memory. This is where they meet it without
 * going anywhere — which is the difference between a feature and a feature people use.
 */
test.describe("the context panel carries what agents learned", () => {
  test("a memory about the selection is shown, attributed, at 1400 px", async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 700 });
    await page.goto("/harness.html?at=/&hits=20");
    await page.getByLabel("Suche").fill("query");
    await page.waitForSelector(".hc-row");
    await page.waitForSelector(".hc-memory");

    const memory = page.locator(".hc-memory").first();
    await expect(memory).toContainText("generated on every build");
    // Attributed, or it is a claim from nowhere.
    await expect(memory).toContainText("agent:claude");
    await expect(memory).toContainText("Beobachtung");
  });

  test("the memory section does not overflow at 400 px", async ({ page }) => {
    await page.setViewportSize({ width: 1400, height: 700 });
    await page.goto("/harness.html?at=/&hits=20");
    await page.getByLabel("Suche").fill("query");
    await page.waitForSelector(".hc-memory");
    // Narrow afterwards, so the panel has to survive being squeezed rather than being built
    // narrow — which is the case §4 is actually about.
    await page.setViewportSize({ width: 400, height: 700 });
    await page.waitForTimeout(150);

    const overflow = await page.evaluate(() => ({
      body: document.body.scrollWidth - document.body.clientWidth,
      worst: Math.max(
        0,
        ...Array.from(document.querySelectorAll(".hc-memory")).map(
          (el) => (el as HTMLElement).scrollWidth - (el as HTMLElement).clientWidth,
        ),
      ),
    }));
    expect(overflow.body).toBeLessThanOrEqual(0);
    expect(overflow.worst).toBeLessThanOrEqual(0);
  });
});

/**
 * M4 §7: every function is reachable without a mouse. The Inspector is a new surface and was
 * not — every one of its buttons used `onMouseDown`, which a keyboard never fires.
 *
 * Checked in a browser rather than in happy-dom, because "pressing Enter on a focused button
 * activates it" is a statement about a real event loop.
 */
test.describe("the Agent Inspector without a mouse", () => {
  test("opens, switches tabs with the arrows, and closes, from the keyboard alone", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 900, height: 700 });
    await page.goto("/harness.html?at=/&hits=20");
    // The application before the keystroke: a press that lands before React has attached its
    // keydown listener is simply lost, and the failure reads as a broken panel thirty seconds
    // later. See the note in `openMemoryTab`.
    await page.waitForSelector(".hc-row");

    await page.keyboard.press("Control+Shift+A");
    await page.waitForSelector(".hc-inspector");

    // The selected tab takes the focus, so the arrows have somewhere to move from. Polled
    // rather than read once: focus is moved by an effect that runs after the panel paints, and
    // asserting immediately is a test that passes when the machine is idle and fails under
    // load — which is the worst kind.
    await expect
      .poll(() => page.evaluate(() => document.activeElement?.textContent ?? ""))
      .toBe("Live");

    await page.keyboard.press("ArrowRight");
    await expect(page.locator('[role="tab"][aria-selected="true"]')).toHaveText("Gedächtnis");
    await page.waitForSelector(".hc-memory, .hc-faint");

    // And wraps, rather than stopping at the end where somebody would think it had broken.
    for (let i = 0; i < 4; i += 1) await page.keyboard.press("ArrowRight");
    await expect(page.locator('[role="tab"][aria-selected="true"]')).toHaveText("Live");

    await page.keyboard.press("Escape");
    expect(await page.locator(".hc-inspector").count()).toBe(0);
  });

  test("a button does what it says when it is pressed rather than clicked", async ({ page }) => {
    await page.setViewportSize({ width: 900, height: 700 });
    await page.goto("/harness.html?at=/&hits=20");
    await page.waitForSelector(".hc-row");
    await page.keyboard.press("Control+Shift+A");
    await page.waitForSelector(".hc-inspector");

    // The kill switch, reached by focusing it and pressing Enter. It reads "Schnittstelle an"
    // from the fixture; pressing it asks the daemon to turn the surface off.
    const toggle = page.getByRole("button", { name: /Schnittstelle/ });
    await toggle.focus();
    await page.keyboard.press("Enter");

    const calls = await page.evaluate(
      () =>
        ((window as unknown as { __hcCalls?: { command: string }[] }).__hcCalls ?? []).map(
          (c) => c.command,
        ),
    );
    expect(calls).toContain("agent_set_enabled");
  });
});


/**
 * D78's third verb. The panel could supersede with something that already existed and it could
 * delete; writing the corrected version — which is what "correct it" means — had nowhere to
 * happen.
 */
test.describe("the human can write a memory and correct one", () => {
  async function openMemoryTab(page: import("@playwright/test").Page) {
    await page.setViewportSize({ width: 1000, height: 700 });
    await page.goto("/harness.html?at=/&hits=20");
    // Wait for the application before pressing a key at it. Without this the press can land
    // before React has attached its keydown listener, the panel never opens, and the failure
    // shows up thirty seconds later as "waiting for tab Gedächtnis" — which reads like a broken
    // tab strip and is a lost keystroke. It failed three times in one session on a machine that
    // was also compiling, and never on an idle one.
    await page.waitForSelector(".hc-row");
    await page.keyboard.press("Control+Shift+A");
    await page.waitForSelector(".hc-inspector");
    await page.getByRole("tab", { name: "Gedächtnis" }).click();
    await page.waitForSelector(".hc-compose");
  }

  test("states a preference outright", async ({ page }) => {
    await openMemoryTab(page);
    await page.locator("#hc-compose-text").fill("auf Deutsch antworten");
    await page.getByRole("button", { name: "Merken" }).click();

    const call = await page.evaluate(() =>
      ((window as unknown as { __hcCalls?: { command: string; args: Record<string, unknown> }[] })
        .__hcCalls ?? []
      ).find((c) => c.command === "remember"),
    );
    expect(call?.args.text).toBe("auf Deutsch antworten");
    expect(call?.args.kind).toBe("preference");
    // Nothing is superseded by a memory stated on its own.
    expect(call?.args.supersedes).toEqual([]);
  });

  test("correcting one carries the memory it replaces", async ({ page }) => {
    await openMemoryTab(page);
    await page.getByRole("button", { name: "Korrigieren" }).first().click();
    // The label changes, so it is visible that this is a replacement and not a second opinion.
    await expect(page.locator(".hc-compose label")).toHaveText("Korrigierte Fassung");

    await page.locator("#hc-compose-text").fill("die bessere Fassung");
    await page.getByRole("button", { name: "Merken" }).click();

    const call = await page.evaluate(() =>
      ((window as unknown as { __hcCalls?: { command: string; args: Record<string, unknown> }[] })
        .__hcCalls ?? []
      ).find((c) => c.command === "remember"),
    );
    expect(call?.args.text).toBe("die bessere Fassung");
    expect((call?.args.supersedes as number[]).length).toBe(1);
  });
});
