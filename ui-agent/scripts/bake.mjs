/**
 * Bake a snapshot into a single HTML file.
 *
 * The result needs no daemon, no server and no network: it is opened by double-clicking it.
 * That is what makes it a demo rather than a screenshot — the graph can be turned, a node can
 * be clicked, the consolidation can be replayed.
 *
 * A baked file contains every path in the snapshot. One made from a real index is therefore as
 * sensitive as the index, and this script says so every time it runs.
 *
 *   node scripts/bake.mjs <graph.json> [out.html]
 */

import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync, statSync } from "node:fs";
import { dirname, resolve } from "node:path";
import process from "node:process";

const [, , source, target = "dist-demo/agent-view.html"] = process.argv;
if (!source) {
  console.error("usage: node scripts/bake.mjs <graph.json> [out.html]");
  process.exit(2);
}

const snapshot = JSON.parse(readFileSync(source, "utf8"));
if (!Array.isArray(snapshot.nodes)) {
  console.error(`${source} is not a snapshot: it has no nodes array`);
  process.exit(2);
}

// The single-file build inlines the script and the stylesheet; the snapshot is prepended
// separately so that one bake step can produce demos of different indexes from one build.
const buildEnv = Object.assign({}, process.env, { HC_SINGLE: "1" });
execFileSync("npx", ["vite", "build"], { stdio: "inherit", env: buildEnv });

const built = readFileSync("dist-demo/index.html", "utf8");
// `</script>` inside a JSON string would end the tag it is sitting in. Escaping the slash is
// the standard, boring fix and keeps the value valid JSON.
const payload = JSON.stringify(snapshot).replaceAll("</", "<\\/");
const injected = built.replace(
  "<body>",
  `<body>\n<script>window.__HC_SNAPSHOT__=${payload}</script>`,
);
if (injected === built) {
  console.error("could not find <body> in the built page");
  process.exit(1);
}

mkdirSync(dirname(resolve(target)), { recursive: true });
writeFileSync(target, injected);

const bytes = statSync(target).size;
const synthetic = String(snapshot.basis ?? "").startsWith("synthetic");
console.log(
  `${target}  ${(bytes / 1e6).toFixed(1)} MB  ${snapshot.nodes.length} nodes, ` +
    `${snapshot.edges.length} edges`,
);
if (!synthetic) {
  console.log(
    "This file contains the paths of a real index. Do not commit it and do not publish it.",
  );
}
