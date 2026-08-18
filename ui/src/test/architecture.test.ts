/**
 * M4 §8: "Frontend performs no filtering, sorting or searching. Enforced by review and by an
 * architectural test asserting the frontend has no data-processing dependencies."
 *
 * This is that test. It reads the source rather than the behaviour, because the property is
 * about what the code *is allowed to contain*, and behaviour tests cannot see the difference
 * between "does not filter" and "does not filter yet".
 *
 * The rule it enforces comes from D4, which made the web stack conditional on two things: every
 * list is virtualised, and the frontend does no data processing. Both conditions exist because
 * the daemon is the application — a frontend that started ranking things would become a second,
 * quietly different implementation of the search.
 */

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const SRC = join(import.meta.dirname, "..");
const PACKAGE = join(SRC, "..", "package.json");

/**
 * What the frontend is allowed to depend on at runtime.
 *
 * React renders, Framer Motion animates, and the Tauri API is the door to the daemon. A search
 * library, a fuzzy matcher, a sorter or a data-frame library on this list would mean the
 * decision in D4 had been reversed without anybody writing it down.
 */
const ALLOWED_DEPENDENCIES = new Set(["react", "react-dom", "framer-motion"]);

/**
 * Operations that reshape a collection. `slice` is not among them: windowing a list to what
 * fits on screen is virtualisation, which D4 *requires*.
 */
const FORBIDDEN_OPERATIONS = [".filter(", ".sort(", ".reduce(", ".find(", ".findIndex("];

function sourceFiles(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) {
      out.push(...sourceFiles(path));
    } else if (/\.tsx?$/.test(entry) && !/\.test\.tsx?$/.test(entry)) {
      out.push(path);
    }
  }
  return out;
}

describe("the frontend does no data processing", () => {
  it("depends on nothing that could rank, match or reshape data", () => {
    const manifest = JSON.parse(readFileSync(PACKAGE, "utf8")) as {
      dependencies?: Record<string, string>;
    };
    const runtime = Object.keys(manifest.dependencies ?? {});

    for (const dependency of runtime) {
      expect(
        ALLOWED_DEPENDENCIES.has(dependency),
        `${dependency} is a runtime dependency of the frontend. D4 allows only ` +
          `${[...ALLOWED_DEPENDENCIES].join(", ")}; anything that could rank, match or reshape ` +
          `data belongs in the daemon.`,
      ).toBe(true);
    }
  });

  it("contains no collection operation outside the one place that windows a list", () => {
    const offenders: string[] = [];

    for (const file of sourceFiles(SRC)) {
      const source = readFileSync(file, "utf8");
      for (const operation of FORBIDDEN_OPERATIONS) {
        if (!source.includes(operation)) continue;
        // `useVirtual` is the exception D4 asks for by name: it exists to bound the DOM, and
        // the arithmetic in it is layout rather than data processing.
        if (file.endsWith("useVirtual.ts")) continue;
        offenders.push(`${file.slice(SRC.length + 1)} uses ${operation}`);
      }
    }

    expect(
      offenders,
      "the frontend renders what the daemon sends; these reshape it instead:\n" +
        offenders.join("\n"),
    ).toEqual([]);
  });

  it("reaches the daemon through one door", () => {
    // Every component imports from `ipc`, and nothing else talks to Tauri. A component that
    // reached for `invoke` directly could call a command the IPC layer does not expose, which
    // is how a frontend acquires capabilities nobody reviewed.
    const offenders = sourceFiles(SRC)
      .filter((file) => !file.endsWith("ipc.ts"))
      .filter((file) => readFileSync(file, "utf8").includes("@tauri-apps/api"))
      .map((file) => file.slice(SRC.length + 1));

    expect(offenders, "only ipc.ts may import the Tauri API").toEqual([]);
  });

  it("uses the design system's tokens rather than its own colours", () => {
    // A hex colour outside tokens.css means the design document stopped being the contract.
    const offenders: string[] = [];
    for (const file of sourceFiles(SRC)) {
      const source = readFileSync(file, "utf8");
      const matches = source.match(/#[0-9a-fA-F]{3,8}\b/g) ?? [];
      // Row ids like `#hc-row-3` are selectors, not colours.
      const colours = matches.filter((m) => /^#[0-9a-fA-F]{3,8}$/.test(m));
      if (colours.length > 0) {
        offenders.push(`${file.slice(SRC.length + 1)}: ${colours.join(", ")}`);
      }
    }
    expect(offenders, "colours belong in tokens.css, per docs/06-DESIGN-SYSTEM.md").toEqual([]);
  });
});
