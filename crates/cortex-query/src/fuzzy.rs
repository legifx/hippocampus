//! Fuzzy matching and scoring for paths.
//!
//! The model to beat is `fzf`, so the rules are the ones people already have in their fingers:
//!
//! - Matching is case-insensitive unless the query contains an upper-case letter, in which case
//!   it becomes case-sensitive. This "smart case" convention is universal in this class of tool.
//! - A contiguous match beats a scattered one.
//! - A match in the file name beats a match in a parent directory. People remember what a thing
//!   is called far more reliably than where they put it.
//! - A match at a word boundary beats a match inside a word.
//! - Among equals, the shorter path wins — it is more likely to be the thing itself rather than
//!   something buried beneath it.
//!
//! Scores are relative and only meaningful within one result set. Nothing outside this module
//! should interpret their absolute magnitude.

/// A contiguous run of matched characters scores far above a scattered subsequence.
const SCORE_CONTIGUOUS: i32 = 1000;
/// Each additional character in a contiguous run.
const BONUS_RUN: i32 = 30;
/// The match lies entirely within the file name rather than a parent directory.
const BONUS_IN_BASENAME: i32 = 400;
/// The match starts at the very beginning of the file name.
const BONUS_BASENAME_START: i32 = 300;
/// The match starts at a word boundary (after `-`, `_`, `.`, `/`, or a case change).
const BONUS_WORD_BOUNDARY: i32 = 120;
/// Subtracted per character of path length, so that shallow results edge out deep ones.
const PENALTY_PER_CHAR: i32 = 1;
/// Subtracted per character skipped between matched characters in a subsequence match.
const PENALTY_PER_GAP: i32 = 8;

/// How a query matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    /// The query appears as a contiguous run.
    Contiguous,
    /// The query characters appear in order, with gaps.
    Subsequence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub score: i32,
    pub kind: MatchKind,
    /// Byte offset in the haystack where the match begins.
    pub start: usize,
}

/// Whether the query should be matched case-sensitively.
///
/// Smart case: an all-lower-case query matches anything; the moment the user types a capital,
/// they mean it.
pub fn is_case_sensitive(query: &str) -> bool {
    query.bytes().any(|b| b.is_ascii_uppercase())
}

#[inline]
fn fold(byte: u8, case_sensitive: bool) -> u8 {
    if case_sensitive {
        byte
    } else {
        byte.to_ascii_lowercase()
    }
}

#[inline]
fn is_boundary_before(haystack: &[u8], index: usize) -> bool {
    if index == 0 {
        return true;
    }
    let previous = haystack[index - 1];
    matches!(previous, b'/' | b'-' | b'_' | b'.' | b' ' | b'+')
        // A lower-to-upper transition is a word boundary in camelCase.
        || (previous.is_ascii_lowercase() && haystack[index].is_ascii_uppercase())
        // A letter following a digit, or the reverse, also reads as a new word.
        || (previous.is_ascii_digit() != haystack[index].is_ascii_digit())
}

/// Byte offset at which the final path component starts.
pub fn basename_start(path: &[u8]) -> usize {
    match path.iter().rposition(|&b| b == b'/') {
        Some(slash) => slash + 1,
        None => 0,
    }
}

/// Find the best contiguous occurrence of `needle` in `haystack`.
///
/// "Best" means the one that scores highest, not the first one: a query matching both a
/// directory name and the file name should be credited for the file name.
fn best_contiguous(haystack: &[u8], needle: &[u8], case_sensitive: bool) -> Option<Match> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }

    let base = basename_start(haystack);
    let first = fold(needle[0], case_sensitive);
    let mut best: Option<Match> = None;

    for start in 0..=(haystack.len() - needle.len()) {
        if fold(haystack[start], case_sensitive) != first {
            continue;
        }
        if !needle
            .iter()
            .enumerate()
            .all(|(i, &n)| fold(haystack[start + i], case_sensitive) == fold(n, case_sensitive))
        {
            continue;
        }

        let mut score = SCORE_CONTIGUOUS + needle.len() as i32 * BONUS_RUN;
        if start >= base {
            score += BONUS_IN_BASENAME;
            if start == base {
                score += BONUS_BASENAME_START;
            }
        }
        if is_boundary_before(haystack, start) {
            score += BONUS_WORD_BOUNDARY;
        }
        score -= haystack.len() as i32 * PENALTY_PER_CHAR;

        if best.is_none_or(|b| score > b.score) {
            best = Some(Match {
                score,
                kind: MatchKind::Contiguous,
                start,
            });
        }
    }

    best
}

/// Greedy subsequence match, scanned from the end of the path backwards.
///
/// Scanning backwards matters: it anchors the match as far towards the file name as possible,
/// which is where users expect it. A forward greedy scan would happily consume the query's
/// letters out of a long parent directory and then rank an unrelated file highly.
fn subsequence(haystack: &[u8], needle: &[u8], case_sensitive: bool) -> Option<Match> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }

    let mut positions = Vec::with_capacity(needle.len());
    let mut hay = haystack.len();

    for &n in needle.iter().rev() {
        let target = fold(n, case_sensitive);
        let mut found = None;
        while hay > 0 {
            hay -= 1;
            if fold(haystack[hay], case_sensitive) == target {
                found = Some(hay);
                break;
            }
        }
        positions.push(found?);
    }
    positions.reverse();

    let base = basename_start(haystack);
    let start = positions[0];
    let end = *positions.last().unwrap();

    let mut score = 0;
    let mut run = 1;
    for window in positions.windows(2) {
        if window[1] == window[0] + 1 {
            run += 1;
            score += BONUS_RUN * run;
        } else {
            run = 1;
            score -= (window[1] - window[0] - 1) as i32 * PENALTY_PER_GAP;
        }
    }

    if start >= base {
        score += BONUS_IN_BASENAME;
        if start == base {
            score += BONUS_BASENAME_START;
        }
    }
    if is_boundary_before(haystack, start) {
        score += BONUS_WORD_BOUNDARY;
    }
    score -= haystack.len() as i32 * PENALTY_PER_CHAR;
    let _ = end;

    Some(Match {
        score,
        kind: MatchKind::Subsequence,
        start,
    })
}

/// Match one term against one path.
///
/// A contiguous match is always preferred, and is tried first because it is also the cheaper
/// of the two in the common case.
pub fn match_term(haystack: &[u8], needle: &[u8], case_sensitive: bool) -> Option<Match> {
    if let Some(m) = best_contiguous(haystack, needle, case_sensitive) {
        return Some(m);
    }
    subsequence(haystack, needle, case_sensitive)
}

/// Match a whole query, which may contain several space-separated terms.
///
/// Every term must match — the semantics people expect from this kind of search box, where
/// adding a word narrows the result set rather than widening it.
pub fn match_query(haystack: &[u8], terms: &[&str], case_sensitive: bool) -> Option<Match> {
    if terms.is_empty() {
        return None;
    }

    let mut total = 0i32;
    let mut kind = MatchKind::Contiguous;
    let mut earliest = usize::MAX;

    for term in terms {
        let m = match_term(haystack, term.as_bytes(), case_sensitive)?;
        total += m.score;
        if m.kind == MatchKind::Subsequence {
            kind = MatchKind::Subsequence;
        }
        earliest = earliest.min(m.start);
    }

    // The length penalty is applied once per term above; with several terms that would punish
    // long paths disproportionately, so the surplus is refunded here.
    total += (terms.len() as i32 - 1) * haystack.len() as i32 * PENALTY_PER_CHAR;

    Some(Match {
        score: total,
        kind,
        start: if earliest == usize::MAX { 0 } else { earliest },
    })
}

/// Split a query into terms. Empty terms are dropped.
pub fn terms(query: &str) -> Vec<&str> {
    query.split_whitespace().filter(|t| !t.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(path: &str, query: &str) -> Option<i32> {
        let cs = is_case_sensitive(query);
        match_query(path.as_bytes(), &terms(query), cs).map(|m| m.score)
    }

    fn better(query: &str, winner: &str, loser: &str) {
        let w = score(winner, query).unwrap_or_else(|| panic!("{winner:?} did not match {query:?}"));
        let l = score(loser, query).unwrap_or_else(|| panic!("{loser:?} did not match {query:?}"));
        assert!(
            w > l,
            "for query {query:?}: expected {winner:?} ({w}) to outrank {loser:?} ({l})"
        );
    }

    #[test]
    fn a_contiguous_match_is_found() {
        let m = match_term(b"/home/user/report.md", b"report", false).unwrap();
        assert_eq!(m.kind, MatchKind::Contiguous);
    }

    #[test]
    fn a_scattered_match_is_found_as_a_subsequence() {
        let m = match_term(b"/home/user/report.md", b"rpt", false).unwrap();
        assert_eq!(m.kind, MatchKind::Subsequence);
    }

    #[test]
    fn a_query_that_is_not_present_does_not_match() {
        assert!(match_term(b"/home/user/report.md", b"zzz", false).is_none());
        // Order matters for a subsequence: the letters are all present, but not in this order.
        assert!(match_term(b"/home/user/ab.md", b"ba", false).is_none());
    }

    #[test]
    fn contiguous_beats_scattered() {
        better("report", "/x/report.md", "/x/rehearsal-porting.md");
    }

    #[test]
    fn a_match_in_the_file_name_beats_one_in_a_parent_directory() {
        better("report", "/archive/2026/report.md", "/report/2026/summary.md");
    }

    #[test]
    fn a_match_at_the_start_of_the_file_name_wins() {
        better("report", "/x/report-final.md", "/x/final-report.md");
    }

    #[test]
    fn a_word_boundary_beats_the_middle_of_a_word() {
        better("port", "/x/some-port.md", "/x/passport.md");
    }

    #[test]
    fn the_shorter_path_wins_when_all_else_is_equal() {
        better("report", "/a/report.md", "/a/very/deeply/nested/tree/report.md");
    }

    #[test]
    fn smart_case_is_insensitive_until_the_user_types_a_capital() {
        assert!(!is_case_sensitive("report"));
        assert!(is_case_sensitive("Report"));

        assert!(score("/x/REPORT.md", "report").is_some(), "lower-case query matches any case");
        assert!(score("/x/report.md", "Report").is_none(), "a capital makes it exact");
        assert!(score("/x/Report.md", "Report").is_some());
    }

    #[test]
    fn every_term_must_match() {
        assert!(score("/projects/aurora/notes.md", "aurora notes").is_some());
        assert!(
            score("/projects/aurora/notes.md", "aurora missing").is_none(),
            "a term that does not appear must exclude the path"
        );
    }

    #[test]
    fn term_order_does_not_matter() {
        let a = score("/projects/aurora/notes.md", "aurora notes").unwrap();
        let b = score("/projects/aurora/notes.md", "notes aurora").unwrap();
        assert_eq!(a, b, "terms are a set, not a sequence");
    }

    #[test]
    fn adding_a_term_narrows_rather_than_widens() {
        let one = score("/projects/aurora/notes.md", "notes");
        let two = score("/projects/aurora/notes.md", "notes aurora");
        let three = score("/projects/aurora/notes.md", "notes aurora absent");
        assert!(one.is_some() && two.is_some());
        assert!(three.is_none());
    }

    #[test]
    fn multi_term_scores_are_comparable_to_single_term_scores() {
        // Without the length-penalty refund, a two-term query would rank a long path far below
        // where a one-term query puts it, and result ordering would change for no good reason.
        let short = score("/a/aurora-notes.md", "aurora notes").unwrap();
        let long = score(
            "/a/very/deeply/nested/directory/tree/aurora-notes.md",
            "aurora notes",
        )
        .unwrap();
        assert!(short > long);
        assert!(
            (short - long).abs() < 200,
            "the length penalty should be applied once, not once per term (got {short} vs {long})"
        );
    }

    #[test]
    fn an_empty_query_matches_nothing() {
        assert!(match_query(b"/x/a.md", &[], false).is_none());
        assert!(terms("   ").is_empty());
    }

    #[test]
    fn a_query_longer_than_the_path_cannot_match() {
        assert!(match_term(b"/a", b"aaaaaaaaaaaa", false).is_none());
    }

    #[test]
    fn basename_start_finds_the_last_component() {
        assert_eq!(basename_start(b"/a/b/c.md"), 5);
        assert_eq!(basename_start(b"c.md"), 0);
        assert_eq!(basename_start(b"/"), 1);
    }

    #[test]
    fn the_subsequence_scan_anchors_towards_the_file_name() {
        // Scanning forwards would consume "a", "u", "r" out of the parent directories and then
        // score this as if the match were far from the file name.
        let m = match_term(b"/aurora/utils/random/aur.md", b"aur", false).unwrap();
        assert!(
            m.start >= basename_start(b"/aurora/utils/random/aur.md"),
            "match anchored at {} but the file name starts at {}",
            m.start,
            basename_start(b"/aurora/utils/random/aur.md")
        );
    }

    #[test]
    fn camel_case_transitions_count_as_word_boundaries() {
        // Lower-case query, so smart case stays off and both paths are eligible; the only
        // difference between them is that one starts the match at a camelCase boundary.
        better("widget", "/x/MyWidgetFactory.ts", "/x/somethingwidgetish.ts");
    }

    #[test]
    fn digits_adjacent_to_letters_count_as_a_boundary() {
        assert!(is_boundary_before(b"report2026", 6));
        assert!(is_boundary_before(b"2026report", 4));
    }

    #[test]
    fn realistic_ranking_holds_for_a_camera_style_corpus() {
        // The case the preview pane exists for: many near-identical names.
        let query = "IMG_2024";
        let mut paths = vec![
            "/Pictures/holiday/IMG_20240612_004411.jpg",
            "/Pictures/holiday/IMG_20190612_004411.jpg",
            "/Archive/old/backup-of-IMG_2024-notes.txt",
        ];
        paths.sort_by_key(|p| -score(p, query).unwrap_or(i32::MIN));
        assert!(
            paths[0].contains("IMG_20240612"),
            "unexpected ranking: {paths:?}"
        );
    }
}
