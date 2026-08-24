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

import type { FsKind, Owner, ScoreBreakdown, ViewRow } from "./ipc";
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
  /** The node behind the row, when the index knows it. Null for a file it has never seen. */
  nodeId: number | null;
  /** Why this row is where it is, when something ranked it. */
  score: ScoreBreakdown | null;
};

/**
 * The one mapper, for the one row shape (M10 step 1).
 *
 * There used to be two — one per source — and they disagreed about what a row is: a browsed row
 * had permissions and no tier, a search result had a tier and no permissions, and the renderer
 * had to know which it was looking at. The daemon now answers in one shape and this turns it
 * into the one the renderer draws. Nothing here decides anything; it formats.
 *
 * `commonDir` is the directory every row would otherwise repeat. A listing passes the directory
 * it is showing and gets bare names; a query passes null and every row says where it lives.
 * That is a rule about repetition rather than about sources, which is why it survives the
 * arrival of concepts and tiers as sources in step 3.
 */
export function rowFromView(row: ViewRow, commonDir: string | null): Row {
  const { dir, name } = splitPath(row.path);
  const target = row.target ?? null;
  return {
    // The node id where there is one, so a row keeps its identity across a reordering; the path
    // otherwise, which is what a directory listing has.
    key: row.node_id === null || row.node_id === undefined ? `p${row.path}` : `n${row.node_id}`,
    path: row.path,
    name,
    dir: commonDir !== null && dir === commonDir ? "" : dir,
    // A link to a directory opens like a directory. It is still drawn as a link — the arrow and
    // the target are right there — but Enter on it has to descend rather than try to preview a
    // directory, which is the whole practical difference the target kind makes.
    isDir: row.kind === "dir" || row.target_kind === "dir",
    size: row.size,
    mtime: row.mtime,
    snippet: row.snippet ?? null,
    tier: row.tier ?? null,
    zoneId: row.zone_id ?? null,
    indexed: row.indexed,
    mode:
      row.mode === null || row.mode === undefined ? null : formatMode(row.kind, row.mode),
    owner: formatOwner({ owner: row.owner ?? null, group: row.group ?? null }),
    linkTarget: row.kind === "symlink" ? target ?? "?" : null,
    broken: row.broken === true,
    nodeId: row.node_id ?? null,
    score: row.score ?? null,
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
/**
 * Whether a row is something the daemon might unpack.
 *
 * By name, and only to decide whether to *offer* the command — the daemon reads the file's
 * magic bytes and has the final say, so a `.zip` that is really a text file is refused by the
 * call rather than by this. Getting it wrong here costs a menu entry that does nothing, which
 * is the right way round: the opposite mistake hides a working command.
 */
export function isArchive(name: string): boolean {
  const lower = name.toLowerCase();
  return [".zip", ".tar", ".tar.gz", ".tgz"].some((suffix) => lower.endsWith(suffix));
}

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
export function formatOwner(entry: { owner?: Owner | null; group?: Owner | null }): string | null {
  const label = (who: Owner | null | undefined) =>
    who === null || who === undefined ? null : who.name ?? String(who.id);
  const user = label(entry.owner);
  const group = label(entry.group);
  if (user === null && group === null) return null;
  return `${user ?? "?"}:${group ?? "?"}`;
}
