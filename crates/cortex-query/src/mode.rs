//! Deciding what kind of question was asked.
//!
//! The five query modes exist because they need genuinely different machinery: a path lookup
//! must not wait for content retrieval, and "what was I working on on Tuesday" needs no
//! retrieval at all. Getting the mode wrong is not a ranking problem, it is a latency problem.
//!
//! Detection is a heuristic and is therefore always overridable by a prefix. The heuristic
//! errs towards including content: a query that looks like a filename is answered from paths
//! alone, and everything else also asks the content index, because a word the user remembers
//! from inside a document is the case where a file manager's search normally gives up.

use cortex_core::model::QueryMode;

/// A query with its mode resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved<'a> {
    pub mode: QueryMode,
    /// The query with any mode prefix removed. Never search with the prefix still attached.
    pub query: &'a str,
    /// True when the user named the mode rather than the heuristic guessing it.
    pub forced: bool,
}

/// Prefixes that force a mode. Checked before the heuristic, longest match irrelevant because
/// they are distinct words.
const PREFIXES: &[(&str, QueryMode)] = &[
    ("locate:", QueryMode::Locate),
    ("path:", QueryMode::Locate),
    ("recall:", QueryMode::Recall),
    ("content:", QueryMode::Recall),
    ("when:", QueryMode::Temporal),
    ("temporal:", QueryMode::Temporal),
    ("graph:", QueryMode::Structural),
    ("structural:", QueryMode::Structural),
    ("deep:", QueryMode::Deep),
];

/// Resolve a requested mode and a raw query into the mode that will actually run.
///
/// `requested` comes from the client. [`QueryMode::Auto`] means "you decide", which is what
/// every client sends unless the user forced a mode in the interface.
pub fn resolve(requested: QueryMode, query: &str) -> Resolved<'_> {
    let trimmed = query.trim();

    for (prefix, mode) in PREFIXES {
        if let Some(rest) = strip_prefix_ignore_case(trimmed, prefix) {
            return Resolved {
                mode: *mode,
                query: rest.trim_start(),
                forced: true,
            };
        }
    }

    match requested {
        QueryMode::Auto => Resolved {
            mode: detect(trimmed),
            query: trimmed,
            forced: false,
        },
        explicit => Resolved {
            mode: explicit,
            query: trimmed,
            forced: true,
        },
    }
}

/// The heuristic. Only ever consulted when nobody said what they wanted.
pub fn detect(query: &str) -> QueryMode {
    if looks_like_a_path(query) {
        QueryMode::Locate
    } else {
        QueryMode::Recall
    }
}

/// Whether a query is asking for a location rather than for a memory.
///
/// Three signals, all of them things a person only types when they mean a file: a path
/// separator, a leading dot, or a bare token carrying a file extension. Everything else — a
/// word, a phrase, a sentence — could as easily be something they remember reading.
fn looks_like_a_path(query: &str) -> bool {
    if query.contains('/') || query.starts_with('.') || query.starts_with('~') {
        return true;
    }

    let mut words = query.split_whitespace();
    let (Some(single), None) = (words.next(), words.next()) else {
        return false;
    };

    match single.rsplit_once('.') {
        // A trailing dot is not an extension, and neither is a number: "1.5" is a version, not
        // a file. The extension also has to be short — "readme.markdown" yes, "e.mailadresse" no.
        Some((stem, extension)) => {
            !stem.is_empty()
                && (1..=8).contains(&extension.len())
                && extension.chars().all(|c| c.is_ascii_alphanumeric())
                && extension.chars().any(|c| c.is_ascii_alphabetic())
        }
        None => false,
    }
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &text[prefix.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prefix_forces_the_mode_and_is_stripped() {
        let resolved = resolve(QueryMode::Auto, "recall: was war der kamerawinkel");
        assert_eq!(resolved.mode, QueryMode::Recall);
        assert_eq!(resolved.query, "was war der kamerawinkel");
        assert!(resolved.forced);
    }

    #[test]
    fn a_prefix_beats_the_mode_the_client_asked_for() {
        // The user typed it; the client only guessed. Anything else makes the prefix useless in
        // an interface that remembers the last mode.
        let resolved = resolve(QueryMode::Recall, "locate: notizen.md");
        assert_eq!(resolved.mode, QueryMode::Locate);
    }

    #[test]
    fn an_explicit_client_mode_is_not_second_guessed() {
        let resolved = resolve(QueryMode::Locate, "eine ganze frage in worten");
        assert_eq!(resolved.mode, QueryMode::Locate);
        assert!(resolved.forced);
    }

    #[test]
    fn filenames_and_paths_are_located_not_recalled() {
        for query in [
            "notizen.md",
            "Projects/aurora",
            "./config.toml",
            "~/Documents",
            "IMG_2024.jpg",
        ] {
            assert_eq!(detect(query), QueryMode::Locate, "{query:?} should locate");
        }
    }

    #[test]
    fn words_and_sentences_reach_the_content_index() {
        // The failure this prevents: a single remembered word from inside a document finding
        // nothing, because the query looked short enough to be a filename.
        for query in [
            "kamerawinkel",
            "aurora report",
            "was habe ich zum budget geschrieben",
            "version 1.5",
        ] {
            assert_eq!(detect(query), QueryMode::Recall, "{query:?} should recall");
        }
    }

    #[test]
    fn a_word_with_a_dot_in_it_is_not_a_filename() {
        assert_eq!(detect("e.mailadresse"), QueryMode::Recall);
        assert_eq!(detect("trailing."), QueryMode::Recall);
    }

    #[test]
    fn prefix_matching_ignores_case_and_surrounding_space() {
        let resolved = resolve(QueryMode::Auto, "  When: gestern nachmittag");
        assert_eq!(resolved.mode, QueryMode::Temporal);
        assert_eq!(resolved.query, "gestern nachmittag");
    }

    #[test]
    fn a_prefix_that_is_only_part_of_a_word_is_not_a_prefix() {
        let resolved = resolve(QueryMode::Auto, "deeply nested");
        assert_eq!(resolved.mode, QueryMode::Recall);
        assert_eq!(resolved.query, "deeply nested");
    }
}
