import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const debugBuild = process.env.TAURI_ENV_DEBUG === "true";
// The layout harness is a second entry point used by the Playwright tests. A release build
// leaves it out; there is no reason for a fixture to ship.
const withHarness = process.env.HC_HARNESS !== "0";

// Tauri loads the built assets from disk. A tiling user opens and closes windows constantly,
// so everything here is in service of a cold start that does not wait on anything: one bundle,
// no code splitting, sourcemaps only in a debug build because they double what has to be read.
export default defineConfig({
  plugins: [react()],
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
