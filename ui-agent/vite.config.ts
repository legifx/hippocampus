import { defineConfig } from "vite";
import { viteSingleFile } from "vite-plugin-singlefile";

/**
 * Two builds out of one source.
 *
 * `vite build` produces the ordinary app, which fetches `graph.json` next to itself — that is
 * what the Tauri shell and the dev server serve. `HC_SINGLE=1 vite build` inlines everything
 * into one HTML file, which is the demo: no server, no daemon, opens from a file manager.
 */
export default defineConfig(() => ({
  base: "./",
  plugins: process.env.HC_SINGLE === "1" ? [viteSingleFile()] : [],
  build: {
    target: "es2022",
    outDir: process.env.HC_SINGLE === "1" ? "dist-demo" : "dist",
    emptyOutDir: true,
    chunkSizeWarningLimit: 1500,
  },
  server: { port: 5174, strictPort: false },
}));
