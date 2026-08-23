/**
 * Turn the single-file build into a body fragment for a hosted page.
 *
 * A published Artifact supplies its own `<!doctype>`, `<html>`, `<head>` and `<body>`, so the
 * file handed to it must be the *contents* of a body — title, styles, snapshot and script, in
 * that order, with nothing wrapping them.
 *
 *   node scripts/artifact.mjs <graph.json> <out.html> "<title>"
 *
 * Refuses a snapshot that is not the synthetic one. A hosted page is a page somebody may share,
 * and a snapshot from a real index carries every path in it.
 */

import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync, statSync } from "node:fs";
import { dirname, resolve } from "node:path";
import process from "node:process";

const [, , source, target, title = "Hippocampus Agent-Ansicht"] = process.argv;
if (!source || !target) {
  console.error('usage: node scripts/artifact.mjs <graph.json> <out.html> "<title>"');
  process.exit(2);
}

const snapshot = JSON.parse(readFileSync(source, "utf8"));
if (!String(snapshot.basis ?? "").startsWith("synthetic")) {
  console.error(
    `refusing to build a shareable page from ${source}: its basis does not say "synthetic", so ` +
      "it may contain the paths of a real index. Generate one with brain-sim.",
  );
  process.exit(1);
}

const buildEnv = Object.assign({}, process.env, { HC_SINGLE: "1" });
execFileSync("npx", ["vite", "build"], { stdio: "inherit", env: buildEnv });

const built = readFileSync("dist-demo/index.html", "utf8");
const styles = [...built.matchAll(/<style[^>]*>[\s\S]*?<\/style>/g)].map((m) => m[0]).join("\n");
// The bundler puts the module script in the head and leaves only the mount point in the body,
// so the scripts are collected from the whole document rather than from the body. The first
// version of this took the body alone and produced a page with a stylesheet, a snapshot and no
// application — which rendered as an empty dark rectangle and reported no error at all.
const scripts = [...built.matchAll(/<script[^>]*>[\s\S]*?<\/script>/g)].map((m) => m[0]);
const body = built.match(/<body[^>]*>([\s\S]*)<\/body>/);
if (!styles || !body || scripts.length === 0) {
  console.error("the built page did not have the shape this script expects");
  process.exit(1);
}
const mount = body[1].replace(/<script[^>]*>[\s\S]*?<\/script>/g, "").trim();
const payload = JSON.stringify(snapshot).replaceAll("</", "<\\/");

const fragment = [
  `<title>${title}</title>`,
  styles,
  `<script>window.__HC_SNAPSHOT__=${payload}</script>`,
  mount,
  ...scripts,
  "",
].join("\n");

mkdirSync(dirname(resolve(target)), { recursive: true });
writeFileSync(target, fragment);
console.log(
  `${target}  ${(statSync(target).size / 1e6).toFixed(1)} MB  ` +
    `${snapshot.nodes.length} nodes, ${snapshot.edges.length} edges (synthetic)`,
);
