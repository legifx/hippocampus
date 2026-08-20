/**
 * One shape for the list, whichever mode produced it.
 *
 * The overlay shows a directory when nothing is typed and results when something is, and M6
 * asks for that to be *one list* rather than two pages — "search as an overlay, results replace
 * the list live". That only works if the row renderer does not know which it is looking at.
 *
 * This is mapping, not processing: nothing here drops a row, reorders one, or decides which to
 * show. The daemon did that.
 */

import type { FsEntry, FsKind, SearchHit } from "./ipc";
import { splitPath } from "./format";

export type Row = {
  /** Stable across a re-render of the same list. */
  key: string;
  path: string;
  name: string;
  /** The directory holding it, with its trailing slash. Empty in a directory listing, where
   *  every row shares the same one and repeating it would be noise. */
  dir: string;
  isDir: boolean;
  size: number;
  mtime: number;
  /** Present only for a search result that matched on content. */
  snippet: string | null;
  /** The memory tier, when the row came from the index. A directory listing has none. */
  tier: string | null;
  zoneId: number | null;
  /** False for a file on disk the brain has never seen. */
  indexed: boolean;
  /** The permission triplets, ready to print. Null for a search result, which has no `stat`. */
  mode: string | null;
  /** `owner:group`, names where the system knew them. */
  owner: string | null;
  /** Where a link points, verbatim. Null for everything that is not a link. */
  linkTarget: string | null;
  /** A link whose target is not there. */
  broken: boolean;
};

export function rowFromHit(hit: SearchHit): Row {
  const { dir, name } = splitPath(hit.path);
  return {
    key: `n${hit.node_id}`,
    path: hit.path,
    name,
    dir,
    isDir: hit.kind === "dir",
    size: hit.size,
    mtime: hit.mtime,
    snippet: hit.snippet,
    tier: hit.tier,
    zoneId: hit.zone_id,
    indexed: true,
    // A hit comes from the index, which stores what a file *is* rather than the permissions it
    // had when it was indexed. Showing a stale mode would be worse than showing none.
    mode: null,
    owner: null,
    linkTarget: null,
    broken: false,
  };
}

export function rowFromEntry(entry: FsEntry): Row {
  const { name } = splitPath(entry.path);
  return {
    key: `p${entry.path}`,
    path: entry.path,
    name,
    // Deliberately empty: in a listing every row is in the same directory, and printing it on
    // each one is a column of identical text where the eye is looking for differences.
    dir: "",
    // A link to a directory opens like a directory. It is still drawn as a link — the arrow and
    // the target are right there — but Enter on it has to descend rather than try to preview a
    // directory, which is the whole practical difference the target kind makes.
    isDir: entry.kind === "dir" || entry.target_kind === "dir",
    size: entry.size,
    mtime: entry.mtime,
    snippet: null,
    tier: null,
    zoneId: null,
    indexed: entry.indexed,
    mode: entry.mode === null || entry.mode === undefined ? null : formatMode(entry.kind, entry.mode),
    owner: formatOwner(entry),
    linkTarget: entry.kind === "symlink" ? entry.target ?? "?" : null,
    broken: entry.broken === true,
  };
}

/**
 * `drwxr-xr-x`, the way every Unix tool prints it.
 *
 * Written out here as well as in the daemon because this is *formatting*, which D4 places on
 * the display side of its line — nothing is being decided, ranked or reshaped. The two
 * implementations are held together by `format_mode`'s tests on the Rust side and this file's
 * on the TypeScript side asserting the same strings.
 */
export function formatMode(kind: FsKind, mode: number): string {
  const type = kind === "dir" ? "d" : kind === "symlink" ? "l" : kind === "other" ? "?" : "-";
  let out = type;
  for (const shift of [6, 3, 0]) {
    const bits = (mode >> shift) & 0o7;
    out += bits & 0o4 ? "r" : "-";
    out += bits & 0o2 ? "w" : "-";

    // The execute column doubles as setuid, setgid and sticky: `-rwsr-xr-x` runs as its owner
    // and `-rwxr-xr-x` does not, and they differ by one character.
    const executable = (bits & 0o1) !== 0;
    const special =
      shift === 6 ? (mode & 0o4000) !== 0 : shift === 3 ? (mode & 0o2000) !== 0 : (mode & 0o1000) !== 0;
    if (special && shift === 0) out += executable ? "t" : "T";
    else if (special) out += executable ? "s" : "S";
    else out += executable ? "x" : "-";
  }
  return out;
}

/** `owner:group`, or as much of it as the daemon could name. */
export function formatOwner(entry: FsEntry): string | null {
  const label = (who: FsEntry["owner"]) =>
    who === null || who === undefined ? null : who.name ?? String(who.id);
  const user = label(entry.owner);
  const group = label(entry.group);
  if (user === null && group === null) return null;
  return `${user ?? "?"}:${group ?? "?"}`;
}
