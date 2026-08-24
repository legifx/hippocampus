/**
 * The perf suite, alone.
 *
 * Same setup as `playwright.config.ts`, with two differences that are the whole reason this file
 * exists: only the tests that measure something run, and they run on one worker. That is the
 * perf budgets and the one rendering test — both of them answer questions about *time*, and both
 * give the wrong answer when four other browsers are using the machine. Everything in it is a budget, and
 * a budget measured against a busy machine measures the machine — this project has a recorded
 * case of three sessions spent optimising something that was never slow.
 */

import { defineConfig } from "@playwright/test";
import base from "./playwright.config";

export default defineConfig({
  ...base,
  testIgnore: undefined,
  testMatch: /(perf|architecture)\.spec\.ts/,
  fullyParallel: false,
  workers: 1,
});
