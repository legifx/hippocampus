/**
 * The mapping from what the daemon says to what a row shows.
 *
 * The cases here are the ones where a file manager quietly lies: a link rendered as its target,
 * a link to a directory that cannot be opened, a permission string that drops the setuid bit.
 * `formatMode` exists twice — here and in `cortex-core` — so these assertions and the Rust ones
 * are deliberately the same strings.
 */

import { describe, expect, it } from "vitest";
import type { ViewRow } from "./ipc";
import { fileUri } from "./format";
import { formatMode, formatOwner, rowFromView } from "./row";

function entry(overrides: Partial<ViewRow> = {}): ViewRow {
  return {
    path: "/home/user/notes.txt",
    kind: "file",
    size: 1024,
    mtime: 1_700_000_000,
    indexed: true,
    ...overrides,
  };
}

/** One mapper now, whatever the source. The directory of the fixture, so names come out bare. */
const row = (entry: ViewRow) => rowFromView(entry, "/home/user/");

describe("permissions", () => {
  it("prints the triplets the way every Unix tool does", () => {
    expect(formatMode("file", 0o644)).toBe("-rw-r--r--");
    expect(formatMode("dir", 0o755)).toBe("drwxr-xr-x");
    expect(formatMode("symlink", 0o777)).toBe("lrwxrwxrwx");
    expect(formatMode("other", 0o660)).toBe("?rw-rw----");
  });

  it("ignores the type bits a full stat mode carries", () => {
    expect(formatMode("file", 0o100644)).toBe("-rw-r--r--");
    expect(formatMode("dir", 0o040755)).toBe("drwxr-xr-x");
  });

  it("shows setuid, setgid and sticky in the execute column", () => {
    expect(formatMode("file", 0o4755)).toBe("-rwsr-xr-x");
    expect(formatMode("file", 0o2755)).toBe("-rwxr-sr-x");
    expect(formatMode("dir", 0o1777)).toBe("drwxrwxrwt");
    expect(formatMode("file", 0o4644)).toBe("-rwSr--r--");
    expect(formatMode("dir", 0o1666)).toBe("drw-rw-rwT");
  });

  it("is always ten characters wide, so the column stays aligned", () => {
    for (let mode = 0; mode < 0o10000; mode++) {
      expect(formatMode("file", mode)).toHaveLength(10);
    }
  });
});

describe("ownership", () => {
  it("prefers names and falls back to numbers", () => {
    expect(formatOwner(entry({ owner: { id: 1000, name: "someone" }, group: { id: 100, name: "users" } })))
      .toBe("someone:users");
    expect(formatOwner(entry({ owner: { id: 1000 }, group: { id: 100 } }))).toBe("1000:100");
    expect(formatOwner(entry({ owner: { id: 0, name: "root" }, group: { id: 7 } }))).toBe("root:7");
  });

  it("is absent rather than invented when the daemon said nothing", () => {
    expect(formatOwner(entry())).toBeNull();
  });
});

describe("symbolic links", () => {
  it("stays a link and shows where it points", () => {
    const mapped = row(
      entry({ path: "/home/user/link", kind: "symlink", target: "/home/user/notes.txt" }),
    );
    expect(mapped.linkTarget).toBe("/home/user/notes.txt");
    expect(mapped.broken).toBe(false);
    expect(mapped.isDir).toBe(false);
  });

  it("opens like a directory when it points at one", () => {
    // Without this the row renders as a file, and Enter tries to preview a directory instead
    // of descending into it.
    const mapped = row(
      entry({ path: "/home/user/shortcut", kind: "symlink", target: "/data", target_kind: "dir" }),
    );
    expect(mapped.isDir).toBe(true);
    expect(mapped.linkTarget).toBe("/data");
  });

  it("marks a link whose target is gone", () => {
    const mapped = row(
      entry({ path: "/home/user/dangling", kind: "symlink", target: "/gone", broken: true }),
    );
    expect(mapped.broken).toBe(true);
    expect(mapped.linkTarget).toBe("/gone");
  });

  it("leaves everything that is not a link without a target", () => {
    expect(row(entry()).linkTarget).toBeNull();
    expect(row(entry({ kind: "dir" })).linkTarget).toBeNull();
  });
});

describe("a browsed row", () => {
  it("carries the stat facts through", () => {
    const mapped = row(
      entry({ mode: 0o100640, owner: { id: 1000, name: "someone" }, group: { id: 1000, name: "someone" } }),
    );
    expect(mapped.mode).toBe("-rw-r-----");
    expect(mapped.owner).toBe("someone:someone");
  });

  it("has none of them when the daemon sent none", () => {
    const mapped = row(entry());
    expect(mapped.mode).toBeNull();
    expect(mapped.owner).toBeNull();
  });
});

/**
 * `fileUri` has to agree, character for character, with the shell's encoder in
 * `ui/src-tauri/src/clipboard.rs` — one is used for a drag and the other for the clipboard, and
 * the same file has to arrive at the other end either way. These are the strings that file
 * asserts, deliberately copied rather than derived.
 */
describe("file URIs", () => {
  it("encodes what the shell's encoder encodes", () => {
    expect(fileUri("/home/user/notes.txt")).toBe("file:///home/user/notes.txt");
    expect(fileUri("/a/two words")).toBe("file:///a/two%20words");
    expect(fileUri("/a/b#c")).toBe("file:///a/b%23c");
    expect(fileUri("/a/b?c")).toBe("file:///a/b%3Fc");
    expect(fileUri("/a/100%")).toBe("file:///a/100%25");
    expect(fileUri("/a/b\nc")).toBe("file:///a/b%0Ac");
    expect(fileUri("/a/Grüße")).toBe("file:///a/Gr%C3%BC%C3%9Fe");
  });

  it("encodes the sub-delimiters encodeURIComponent leaves alone", () => {
    // Legal unescaped, and escaped anyway: two encoders that disagree about the same file are
    // two encoders nothing can check against each other.
    expect(fileUri("/a/it's")).toBe("/a/it%27s".replace("/a", "file:///a"));
    expect(fileUri("/a/(one)")).toBe("file:///a/%28one%29");
    expect(fileUri("/a/star*")).toBe("file:///a/star%2A");
    expect(fileUri("/a/bang!")).toBe("file:///a/bang%21");
  });

  it("keeps the separators and handles the root", () => {
    expect(fileUri("/")).toBe("file:///");
    expect(fileUri("/a/b/c")).toBe("file:///a/b/c");
    // A doubled separator is not a segment, and inventing one would name a different file.
    expect(fileUri("/a//b")).toBe("file:///a/b");
  });
});
