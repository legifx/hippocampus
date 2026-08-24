import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";

const debugBuild = process.env.TAURI_ENV_DEBUG === "true";
// The layout harness is a second entry point used by the Playwright tests. A release build
// leaves it out; there is no reason for a fixture to ship.
const withHarness = process.env.HC_HARNESS !== "0";

// Tauri loads the built assets from disk. A tiling user opens and closes windows constantly,
// so everything here is in service of a cold start that does not wait on anything: one bundle,
// no code splitting, sourcemaps only in a debug build because they double what has to be read.
//
// **One exception, and it protects the rule rather than eroding it.** The Agent Inspector's
// three-dimensional tab imports three.js dynamically, so Rollup gives it a chunk of its own.
// That is around half a megabyte which most sessions never open, and putting it in the main
// bundle would spend the 300 ms cold-start gate on a tab nobody asked for. Splitting *out* what
// the first paint does not need is the same argument as not splitting what it does.
const here = (path: string) => fileURLToPath(new URL(path, import.meta.url));

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      // The Agent Inspector's three-dimensional tab mounts the M8 scene rather than
      // reimplementing it. Two implementations of "what the brain looks like" would diverge,
      // and the one in this window would be the one nobody noticed had. `three` resolves to
      // `ui-agent`'s copy for the same reason: one version, one renderer.
      "@agent-view": here("../ui-agent/src"),
      three: here("../ui-agent/node_modules/three"),
    },
  },
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: {
    target: "es2022",
    sourcemap: debugBuild,
    minify: debugBuild ? false : "esbuild",
    rollupOptions: {
      input: withHarness
        ? { index: "index.html", harness: "harness.html" }
        : { index: "index.html" },
    },
  },
  test: {
    environment: "happy-dom",
    globals: true,
    setupFiles: ["src/test/setup.ts"],
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
