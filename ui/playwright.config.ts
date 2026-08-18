import { existsSync, readdirSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { defineConfig } from "@playwright/test";

/**
 * The layout tests run against a real engine.
 *
 * M4 §4 — "resizing the window from full width to 400 px never produces horizontal overflow or
 * clipped controls, tested at 400, 600, 900, 1400 px" — is a claim about a box model, and a DOM
 * emulation has none to be wrong about. The webview Tauri uses is WebKit rather than Chromium,
 * so this is an approximation; it is a far closer one than jsdom, and the layout uses nothing
 * where the two engines disagree.
 */

/**
 * Reuse a Chrome for Testing that is already on the machine.
 *
 * Playwright pins a browser build per release and downloads it on demand. A developer who
 * already has one should not fetch a second copy of the same thing to run seventeen layout
 * assertions, so an existing build is preferred and Playwright's own is the fallback.
 */
function existingChrome(): string | undefined {
  const root = join(homedir(), ".cache", "ms-playwright");
  if (!existsSync(root)) return undefined;

  const builds = readdirSync(root)
    .filter((entry) => entry.startsWith("chromium-"))
    .sort((a, b) => Number(b.split("-")[1]) - Number(a.split("-")[1]));

  for (const build of builds) {
    const candidate = join(root, build, "chrome-linux64", "chrome");
    if (existsSync(candidate)) return candidate;
  }
  return undefined;
}

const chrome = existingChrome();

export default defineConfig({
  testDir: "./tests",
  fullyParallel: true,
  reporter: process.env.CI ? "line" : "list",
  use: {
    baseURL: "http://127.0.0.1:4173",
    launchOptions: chrome ? { executablePath: chrome } : {},
  },
  webServer: {
    command: "npx vite preview --port 4173 --strictPort",
    url: "http://127.0.0.1:4173/harness.html",
    reuseExistingServer: true,
    timeout: 30_000,
  },
});
