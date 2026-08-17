//! The in-memory path index.
//!
//! Serves the `locate` query mode, which is roughly ninety per cent of real usage and has the
//! tightest budget in the whole system: **10 ms P95 over a hundred thousand paths**.
//!
//! ## How it is fast
//!
//! A trigram posting list narrows a hundred thousand paths to a few hundred candidates before
//! any scoring happens. Scoring a few hundred candidates is free; scoring a hundred thousand is
//! not. The rarest trigrams in the query are intersected first, so the candidate set collapses
//! as quickly as possible.
//!
//! ## How it stays small
//!
//! Paths live in one concatenated byte buffer rather than a hundred thousand separately
//! allocated `String`s — the allocator overhead alone would exceed the strings. Entries hold
//! offsets into it. Matching folds case on the fly, so the original bytes are stored once and
//! serve both matching and display.
//!
//! ## Two tiers, and why
//!
//! A trigram index can only accelerate **contiguous** matches. No index accelerates arbitrary
//! subsequence matching — that is why `fzf` simply scans everything. So the search runs in two
//! tiers:
//!
//! 1. **Trigram-filtered.** Sub-millisecond. Finds every path containing the query literally.
//! 2. **Full scan**, entered *only when the first tier found nothing at all*. This is what makes
//!    abbreviations work: `prj3` shares no trigram with `project3`, so tier one returns nothing
//!    and tier two finds it.
//!
//! The condition is "nothing at all" rather than "fewer than the limit" on purpose. If thirty
//! files literally contain what the user typed, they do not want two hundred loosely-related
//! subsequence matches diluting them — and paying for a full scan to produce that dilution
//! would be the worst of both. Searching harder is what you do when the obvious answer is
//! empty, not what you do routinely.
//!
//! ## How it stays correct while the filesystem moves
//!
//! Removal is a tombstone, not a compaction: rewriting every posting list that mentions a path
//! would make deleting a directory quadratic. Dead entries are skipped during matching and
//! reclaimed by `compact()` once they exceed a threshold. This is the standard trade and it is
//! the reason the watcher can keep up with a `git checkout`.

use crate::fuzzy::{self, MatchKind};
use cortex_core::ids::NodeId;
use std::collections::HashMap;

/// Trigrams are the shortest useful unit. Shorter queries fall back to a full scan, which is
/// affordable precisely because they are rare — nobody searches a hundred thousand files by
/// typing one letter and then stopping.
const TRIGRAM: usize = 3;

/// Intersecting more than this many posting lists costs more than it saves: after three or four
/// rare trigrams the candidate set is already small enough that scoring dominates.
const MAX_PROBES: usize = 4;

/// Compact once this fraction of entries is dead.
const COMPACT_THRESHOLD: f32 = 0.25;

/// Below this many entries, spawning threads costs more than the scan itself.
const PARALLEL_SCAN_THRESHOLD: usize = 20_000;

/// Scanning is memory-bandwidth bound long before it is core bound, so more threads than this
/// buys nothing and starts competing with the rest of the daemon.
const MAX_SCAN_THREADS: usize = 8;

#[derive(Debug, Clone)]
struct Entry {
    node_id: NodeId,
    start: u32,
    len: u32,
    live: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PathHit {
    pub node_id: NodeId,
    pub path: String,
    pub score: i32,
    pub exact: bool,
}

#[derive(Debug, Default)]
pub struct PathIndex {
    haystack: Vec<u8>,
    entries: Vec<Entry>,
    postings: HashMap<u32, Vec<u32>>,
    by_node: HashMap<NodeId, u32>,
    dead: usize,
}

impl PathIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from an iterator of paths. Used at startup, straight out of the database.
    pub fn build<I, S>(paths: I) -> Self
    where
        I: IntoIterator<Item = (NodeId, S)>,
        S: AsRef<str>,
    {
        let mut index = Self::new();
        let mut scratch = Vec::new();
        for (node_id, path) in paths {
            index.insert_with_scratch(node_id, path.as_ref(), &mut scratch);
        }
        index
    }

    pub fn len(&self) -> usize {
        self.entries.len() - self.dead
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Approximate heap use, in bytes. Reported by `cortex status` so that the memory budget is
    /// observable rather than a matter of faith.
    pub fn memory_bytes(&self) -> usize {
        let postings: usize = self
            .postings
            .values()
            .map(|v| v.capacity() * std::mem::size_of::<u32>())
            .sum();
        self.haystack.capacity()
            + self.entries.capacity() * std::mem::size_of::<Entry>()
            + postings
            + self.postings.capacity() * (std::mem::size_of::<u32>() + std::mem::size_of::<Vec<u32>>())
            + self.by_node.capacity() * (std::mem::size_of::<NodeId>() + std::mem::size_of::<u32>())
    }

    pub fn insert(&mut self, node_id: NodeId, path: &str) {
        let mut scratch = Vec::new();
        self.insert_with_scratch(node_id, path, &mut scratch);
    }

    fn insert_with_scratch(&mut self, node_id: NodeId, path: &str, scratch: &mut Vec<u32>) {
        // Re-inserting a known node is a rename. Tombstone the old entry rather than trying to
        // patch it: the path length almost always differs, so the buffer slot cannot be reused.
        if self.by_node.contains_key(&node_id) {
            self.remove(node_id);
        }

        let start = self.haystack.len() as u32;
        self.haystack.extend_from_slice(path.as_bytes());
        let entry_index = self.entries.len() as u32;

        self.entries.push(Entry {
            node_id,
            start,
            len: path.len() as u32,
            live: true,
        });
        self.by_node.insert(node_id, entry_index);

        trigrams_of(path.as_bytes(), scratch);
        for &trigram in scratch.iter() {
            self.postings.entry(trigram).or_default().push(entry_index);
        }
    }

    /// Tombstone a path. Cheap; the space comes back at the next compaction.
    pub fn remove(&mut self, node_id: NodeId) -> bool {
        let Some(index) = self.by_node.remove(&node_id) else {
            return false;
        };
        if let Some(entry) = self.entries.get_mut(index as usize) {
            if entry.live {
                entry.live = false;
                self.dead += 1;
            }
        }
        true
    }

    pub fn needs_compaction(&self) -> bool {
        !self.entries.is_empty()
            && (self.dead as f32 / self.entries.len() as f32) >= COMPACT_THRESHOLD
    }

    /// Rebuild without the tombstones.
    pub fn compact(&mut self) {
        if self.dead == 0 {
            return;
        }
        let live: Vec<(NodeId, String)> = self
            .entries
            .iter()
            .filter(|e| e.live)
            .map(|e| {
                let bytes = &self.haystack[e.start as usize..(e.start + e.len) as usize];
                (e.node_id, String::from_utf8_lossy(bytes).into_owned())
            })
            .collect();
        *self = Self::build(live);
    }

    #[inline]
    fn path_bytes(&self, entry: &Entry) -> &[u8] {
        &self.haystack[entry.start as usize..(entry.start + entry.len) as usize]
    }

    /// Search. Returns at most `limit` hits, best first.
    pub fn search(&self, query: &str, limit: usize) -> Vec<PathHit> {
        let terms = fuzzy::terms(query);
        if terms.is_empty() || limit == 0 {
            return Vec::new();
        }
        let case_sensitive = fuzzy::is_case_sensitive(query);

        let mut scored: Vec<(i32, u32, MatchKind)> = Vec::new();

        if let Some(candidates) = self.candidates(&terms) {
            for index in candidates {
                let Some(entry) = self.entries.get(index as usize) else {
                    continue;
                };
                if !entry.live {
                    continue;
                }
                if let Some(m) = fuzzy::match_query(self.path_bytes(entry), &terms, case_sensitive)
                {
                    scored.push((m.score, index, m.kind));
                }
            }
        }

        // Fall back to a full scan only when the fast path found *nothing at all*. See the
        // module documentation: this is the "try harder" tier, and it is deliberately not
        // reached whenever the literal search already succeeded.
        if scored.is_empty() {
            scored = self.full_scan(&terms, case_sensitive);
        }

        // Rank by score, then by path length, then by node id. The tie-breakers are not
        // cosmetic: a result list that reshuffles between identical queries is unusable,
        // because the user's muscle memory for "second from the top" stops working.
        let order = |a: &(i32, u32, MatchKind), b: &(i32, u32, MatchKind)| {
            b.0.cmp(&a.0)
                .then_with(|| {
                    let ea = &self.entries[a.1 as usize];
                    let eb = &self.entries[b.1 as usize];
                    ea.len.cmp(&eb.len)
                })
                .then_with(|| {
                    let ea = &self.entries[a.1 as usize];
                    let eb = &self.entries[b.1 as usize];
                    ea.node_id.cmp(&eb.node_id)
                })
        };

        // Only the first `limit` results are ever shown, so only they are worth ordering.
        // A single-character query matches nearly every path, and fully sorting a hundred
        // thousand candidates to display fifty of them was the single largest cost on the
        // keystroke path — larger than the matching it was ordering.
        if scored.len() > limit {
            scored.select_nth_unstable_by(limit - 1, order);
            scored.truncate(limit);
        }
        scored.sort_unstable_by(order);

        scored
            .into_iter()
            .take(limit)
            .map(|(score, index, kind)| {
                let entry = &self.entries[index as usize];
                PathHit {
                    node_id: entry.node_id,
                    path: String::from_utf8_lossy(self.path_bytes(entry)).into_owned(),
                    score,
                    exact: kind == MatchKind::Contiguous,
                }
            })
            .collect()
    }

    /// Score every live entry.
    ///
    /// This is the tier-two path, and it is also the path every *first keystroke* takes: a one-
    /// or two-character query has no trigrams, so there is nothing to narrow with. That makes
    /// it latency-critical despite being the fallback, and it is why it is parallelised.
    ///
    /// Threads are used directly rather than through a work-stealing pool. The work is a flat,
    /// perfectly balanced scan over a contiguous slice — the one shape where a pool's scheduling
    /// overhead buys nothing.
    fn full_scan(&self, terms: &[&str], case_sensitive: bool) -> Vec<(i32, u32, MatchKind)> {
        let threads = if self.entries.len() < PARALLEL_SCAN_THRESHOLD {
            1
        } else {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
                .min(MAX_SCAN_THREADS)
        };

        if threads <= 1 {
            return self.scan_range(0, self.entries.len(), terms, case_sensitive);
        }

        let chunk = self.entries.len().div_ceil(threads);
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..threads)
                .map(|t| {
                    let start = t * chunk;
                    let end = ((t + 1) * chunk).min(self.entries.len());
                    scope.spawn(move || self.scan_range(start, end, terms, case_sensitive))
                })
                .collect();

            let mut out = Vec::new();
            for handle in handles {
                // A panic in a scoring thread would be a bug in the matcher, not a condition to
                // recover from; propagating it is correct.
                out.extend(handle.join().expect("scoring thread panicked"));
            }
            out
        })
    }

    fn scan_range(
        &self,
        start: usize,
        end: usize,
        terms: &[&str],
        case_sensitive: bool,
    ) -> Vec<(i32, u32, MatchKind)> {
        let mut out = Vec::new();
        for index in start..end {
            let entry = &self.entries[index];
            if !entry.live {
                continue;
            }
            if let Some(m) = fuzzy::match_query(self.path_bytes(entry), terms, case_sensitive) {
                out.push((m.score, index as u32, m.kind));
            }
        }
        out
    }

    /// Candidate entry indices from the trigram postings.
    ///
    /// `None` means the index cannot narrow this query and the caller must scan everything.
    /// An empty vector means the index is certain there is nothing to find, which is a very
    /// different statement and must not be confused with it.
    fn candidates(&self, terms: &[&str]) -> Option<Vec<u32>> {
        // Only the longest term is used to generate candidates. Every term still has to match
        // during scoring, so this loses no results — and one well-chosen term narrows about as
        // well as several while doing a fraction of the intersection work.
        let term = terms.iter().max_by_key(|t| t.len())?;

        let mut wanted = Vec::new();
        trigrams_of(term.as_bytes(), &mut wanted);
        if wanted.is_empty() {
            return None;
        }

        // Rarest first: the smallest posting list bounds the whole intersection.
        let mut lists: Vec<&Vec<u32>> = Vec::with_capacity(wanted.len());
        for trigram in &wanted {
            match self.postings.get(trigram) {
                // A trigram nobody has seen means nothing can match it.
                None => return Some(Vec::new()),
                Some(list) => lists.push(list),
            }
        }
        lists.sort_unstable_by_key(|l| l.len());

        let mut current = lists[0].clone();
        for list in lists.iter().skip(1).take(MAX_PROBES - 1) {
            current = intersect_sorted(&current, list);
            if current.is_empty() {
                break;
            }
        }
        Some(current)
    }
}

/// Distinct trigrams of `bytes`, lower-cased, written into `out`.
///
/// Case is folded here so that the index is case-insensitive by construction. A case-sensitive
/// query still works: it narrows with case-folded trigrams and then rejects the wrong-case
/// candidates during scoring.
fn trigrams_of(bytes: &[u8], out: &mut Vec<u32>) {
    out.clear();
    if bytes.len() < TRIGRAM {
        return;
    }
    for window in bytes.windows(TRIGRAM) {
        let packed = (window[0].to_ascii_lowercase() as u32) << 16
            | (window[1].to_ascii_lowercase() as u32) << 8
            | window[2].to_ascii_lowercase() as u32;
        out.push(packed);
    }
    out.sort_unstable();
    out.dedup();
}

/// Intersect two ascending lists.
///
/// Posting lists are ascending by construction because entry indices only ever grow, so this
/// never has to sort.
fn intersect_sorted(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len().min(b.len()));
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                out.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index_of(paths: &[&str]) -> PathIndex {
        PathIndex::build(
            paths
                .iter()
                .enumerate()
                .map(|(i, p)| (NodeId(i as i64 + 1), *p)),
        )
    }

    fn found(index: &PathIndex, query: &str) -> Vec<String> {
        index.search(query, 100).into_iter().map(|h| h.path).collect()
    }

    #[test]
    fn finds_a_path_by_a_fragment_of_its_name() {
        let index = index_of(&["/home/user/Documents/report.md", "/home/user/notes.txt"]);
        assert_eq!(found(&index, "report"), vec!["/home/user/Documents/report.md"]);
    }

    #[test]
    fn finds_nothing_when_there_is_nothing() {
        let index = index_of(&["/a/b.md"]);
        assert!(found(&index, "zzzzzz").is_empty());
    }

    #[test]
    fn an_empty_query_returns_nothing_rather_than_everything() {
        let index = index_of(&["/a/b.md", "/c/d.md"]);
        assert!(index.search("", 10).is_empty());
        assert!(index.search("   ", 10).is_empty());
    }

    #[test]
    fn a_zero_limit_returns_nothing() {
        let index = index_of(&["/a/report.md"]);
        assert!(index.search("report", 0).is_empty());
    }

    #[test]
    fn short_queries_still_work_through_the_full_scan_path() {
        // Under three characters there are no trigrams, so this exercises the fallback.
        let index = index_of(&["/a/xy.md", "/a/other.md"]);
        assert_eq!(found(&index, "xy"), vec!["/a/xy.md"]);
        assert_eq!(found(&index, "x"), vec!["/a/xy.md"]);
    }

    fn sample_paths() -> Vec<String> {
        (0..500)
            .map(|i| format!("/home/user/project{}/src/module_{}.rs", i % 17, i))
            .collect()
    }

    #[test]
    fn trigram_filtering_never_drops_a_literal_match() {
        // The guarantee tier one makes: if the query appears literally in a path, that path is
        // in the results. Anything less and the fast path would be silently lossy.
        let paths = sample_paths();
        let index = PathIndex::build(
            paths
                .iter()
                .enumerate()
                .map(|(i, p)| (NodeId(i as i64 + 1), p.as_str())),
        );

        for query in ["module", "project3", "src", "rs", "module_42", "roject1"] {
            let mut via_index: Vec<String> =
                index.search(query, 5000).into_iter().map(|h| h.path).collect();

            let mut literal: Vec<String> = paths
                .iter()
                .filter(|p| p.to_lowercase().contains(&query.to_lowercase()))
                .cloned()
                .collect();

            via_index.sort();
            literal.sort();
            assert!(
                literal.iter().all(|p| via_index.contains(p)),
                "trigram filtering lost a literal match for {query:?}"
            );
        }
    }

    #[test]
    fn an_abbreviation_is_found_through_the_full_scan_tier() {
        // "prj3" shares no trigram with "project3", so tier one returns nothing and tier two
        // has to find it. This is the behaviour people bring with them from fzf.
        let paths = sample_paths();
        let index = PathIndex::build(
            paths
                .iter()
                .enumerate()
                .map(|(i, p)| (NodeId(i as i64 + 1), p.as_str())),
        );

        let hits = index.search("prj3", 20);
        assert!(!hits.is_empty(), "the abbreviation tier found nothing");
        assert!(
            hits.iter().all(|h| !h.exact),
            "these should be reported as scattered matches, not literal ones"
        );
    }

    #[test]
    fn the_full_scan_tier_stays_out_of_the_way_when_the_literal_search_succeeds() {
        // If a literal match exists, the result set must be exactly the literal matches —
        // not diluted by every path that happens to contain the letters in order.
        let paths = sample_paths();
        let index = PathIndex::build(
            paths
                .iter()
                .enumerate()
                .map(|(i, p)| (NodeId(i as i64 + 1), p.as_str())),
        );

        let hits = index.search("project3", 5000);
        assert!(!hits.is_empty());
        assert!(
            hits.iter().all(|h| h.path.contains("project3")),
            "a successful literal search must not pull in subsequence matches"
        );
    }

    #[test]
    fn results_are_ranked_with_the_best_first() {
        let index = index_of(&[
            "/archive/2019/old/report-draft.md",
            "/report.md",
            "/x/quarterly-report.md",
        ]);
        let hits = index.search("report", 10);
        assert_eq!(hits[0].path, "/report.md");
    }

    #[test]
    fn the_limit_is_respected() {
        let paths: Vec<String> = (0..50).map(|i| format!("/x/report{i}.md")).collect();
        let index = PathIndex::build(
            paths
                .iter()
                .enumerate()
                .map(|(i, p)| (NodeId(i as i64 + 1), p.as_str())),
        );
        assert_eq!(index.search("report", 7).len(), 7);
    }

    #[test]
    fn ordering_is_stable_across_identical_queries() {
        // A list that reshuffles between identical queries destroys the user's ability to
        // reach for "the second one".
        let paths: Vec<String> = (0..200).map(|i| format!("/x/same-name-{i}.md")).collect();
        let index = PathIndex::build(
            paths
                .iter()
                .enumerate()
                .map(|(i, p)| (NodeId(i as i64 + 1), p.as_str())),
        );
        let first = index.search("same", 20);
        for _ in 0..5 {
            assert_eq!(index.search("same", 20), first);
        }
    }

    #[test]
    fn a_removed_path_stops_being_found() {
        let mut index = index_of(&["/a/report.md", "/b/report.md"]);
        assert_eq!(found(&index, "report").len(), 2);

        assert!(index.remove(NodeId(1)));
        assert_eq!(found(&index, "report"), vec!["/b/report.md"]);
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn removing_an_unknown_node_is_harmless() {
        let mut index = index_of(&["/a/report.md"]);
        assert!(!index.remove(NodeId(999)));
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn reinserting_a_node_replaces_its_old_path() {
        // This is what a rename looks like from the index's point of view.
        let mut index = index_of(&["/old/location/report.md"]);
        index.insert(NodeId(1), "/new/location/report.md");

        assert_eq!(found(&index, "report"), vec!["/new/location/report.md"]);
        assert!(found(&index, "old").is_empty(), "the old path must not linger");
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn compaction_reclaims_tombstones_without_changing_results() {
        let paths: Vec<String> = (0..100).map(|i| format!("/x/file-{i}.md")).collect();
        let mut index = PathIndex::build(
            paths
                .iter()
                .enumerate()
                .map(|(i, p)| (NodeId(i as i64 + 1), p.as_str())),
        );

        for i in 1..=40 {
            index.remove(NodeId(i));
        }
        assert!(index.needs_compaction());

        let before = index.search("file", 100);
        let memory_before = index.memory_bytes();
        index.compact();
        let after = index.search("file", 100);

        assert_eq!(before, after, "compaction must not change what is found");
        assert_eq!(index.len(), 60);
        assert!(!index.needs_compaction());
        assert!(
            index.memory_bytes() < memory_before,
            "compaction should actually free memory"
        );
    }

    #[test]
    fn compacting_a_clean_index_is_a_no_op() {
        let mut index = index_of(&["/a/b.md"]);
        let before = index.search("b", 10);
        index.compact();
        assert_eq!(index.search("b", 10), before);
    }

    #[test]
    fn a_query_that_matches_nothing_at_all_returns_empty() {
        // "qqq" is in no posting list, so tier one short-circuits; tier two then scans and
        // also finds nothing, because the letters do not appear in order either.
        let index = index_of(&["/a/report.md"]);
        assert!(index.search("qqq", 10).is_empty());
    }

    #[test]
    fn multi_term_queries_narrow_the_result_set() {
        let index = index_of(&[
            "/projects/aurora/src/main.rs",
            "/projects/basalt/src/main.rs",
            "/projects/aurora/docs/readme.md",
        ]);
        assert_eq!(found(&index, "aurora main"), vec!["/projects/aurora/src/main.rs"]);
    }

    #[test]
    fn non_ascii_paths_are_indexed_and_found() {
        let index = index_of(&["/dokumente/jahresübersicht.md", "/dokumente/größe.txt"]);
        assert_eq!(found(&index, "übersicht"), vec!["/dokumente/jahresübersicht.md"]);
        assert_eq!(found(&index, "größe"), vec!["/dokumente/größe.txt"]);
    }

    #[test]
    fn the_hit_reports_whether_the_match_was_contiguous() {
        let index = index_of(&["/x/report.md"]);
        assert!(index.search("report", 1)[0].exact);
        assert!(!index.search("rpt", 1)[0].exact);
    }

    #[test]
    fn intersect_sorted_behaves() {
        assert_eq!(intersect_sorted(&[1, 2, 3, 4], &[2, 4, 6]), vec![2, 4]);
        assert_eq!(intersect_sorted(&[], &[1, 2]), Vec::<u32>::new());
        assert_eq!(intersect_sorted(&[1, 2], &[3, 4]), Vec::<u32>::new());
    }

    #[test]
    fn trigrams_are_distinct_and_case_folded() {
        let mut out = Vec::new();
        trigrams_of(b"aaaa", &mut out);
        assert_eq!(out.len(), 1, "repeated trigrams should be deduplicated");

        let mut lower = Vec::new();
        let mut upper = Vec::new();
        trigrams_of(b"report", &mut lower);
        trigrams_of(b"REPORT", &mut upper);
        assert_eq!(lower, upper, "the index must be case-insensitive by construction");
    }

    #[test]
    fn a_string_shorter_than_a_trigram_yields_none() {
        let mut out = Vec::new();
        trigrams_of(b"ab", &mut out);
        assert!(out.is_empty());
    }
}
