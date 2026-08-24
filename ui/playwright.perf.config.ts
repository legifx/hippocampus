/**
 * The perf suite, alone.
 *
 * Same setup as `playwright.config.ts`, with two differences that are the whole reason this file
 * exists: only `perf.spec.ts` runs, and it runs on one worker. Everything in it is a budget, and
 * a budget measured against a busy machine measures the machine — this project has a recorded
 * case of three sessions spent optimising something that was never slow.
 */

import { defineConfig } from "@playwright/test";
import base from "./playwright.config";

export default defineConfig({
  ...base,
  testIgnore: undefined,
  testMatch: /perf\.spec\.ts/,
  fullyParallel: false,
  workers: 1,
});
