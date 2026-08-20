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

import type { FsEntry, SearchHit } from "./ipc";
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
    isDir: entry.kind === "dir",
    size: entry.size,
    mtime: entry.mtime,
    snippet: null,
    tier: null,
    zoneId: null,
    indexed: entry.indexed,
  };
}
