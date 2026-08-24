//! Turning what a person typed into something FTS5 will accept.
//!
//! FTS5's `MATCH` takes an expression language, not a string of words: `AND`, `OR`, `NOT` and
//! `NEAR` are operators, `^` anchors, `:` selects a column, `-` and `*` have meaning, and an
//! unbalanced quote is a syntax error. Passing user input to it unchanged means a search that
//! fails on an apostrophe and silently changes meaning on the word "or". Neither is acceptable
//! in a box people type into without thinking.
//!
//! So every token is emitted as a quoted string, which FTS5 treats as literal text, and the
//! operators are ours to add rather than the user's to trip over. A quoted section in the input
//! survives as a phrase, because that is the one piece of query syntax people do expect.

/// One unit of the query as the user meant it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Term {
    pub text: String,
    /// True when the user quoted it, so it must match as a phrase rather than as words.
    pub phrase: bool,
}

/// Split a raw query into terms.
///
/// Splitting is on anything that is not alphanumeric, which keeps umlauts and digits and drops
/// punctuation. It deliberately does not stem or fold: FTS5's tokenizer does that, and doing it
/// twice in two different ways is how a query stops matching its own index.
pub fn terms(query: &str) -> Vec<Term> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    let flush = |current: &mut String, phrase: bool, out: &mut Vec<Term>| {
        let text = current.trim().to_string();
        current.clear();
        if !text.is_empty() {
            out.push(Term { text, phrase });
        }
    };

    for c in query.chars() {
        match c {
            '"' => {
                flush(&mut current, in_quotes, &mut out);
                in_quotes = !in_quotes;
            }
            // Inside quotes everything but the quote character itself is part of the phrase.
            _ if in_quotes => current.push(c),
            _ if c.is_alphanumeric() => current.push(c),
            _ => flush(&mut current, false, &mut out),
        }
    }
    // An unterminated quote is treated as a phrase to the end of the input rather than as an
    // error. People open a quote and forget to close it; refusing to search is the wrong answer.
    flush(&mut current, in_quotes, &mut out);

    out
}

/// Build an FTS5 match expression, or `None` when there is nothing to match on.
///
/// `prefix_last` turns the final token into a prefix query, which is what makes results appear
/// while a word is still being typed. It is switched off for a completed query, where matching
/// "budge" against "budgetieren" would be a false positive rather than a prediction.
pub fn match_expression(query: &str, prefix_last: bool) -> Option<String> {
    let terms = terms(query);
    if terms.is_empty() {
        return None;
    }

    let last = terms.len() - 1;
    let mut parts = Vec::with_capacity(terms.len());
    for (i, term) in terms.iter().enumerate() {
        let quoted = format!("\"{}\"", term.text.replace('"', "\"\""));
        // A prefix on a phrase would apply to its last word only, which is rarely what quoting
        // meant, so quoted phrases are matched exactly.
        if prefix_last && i == last && !term.phrase {
            parts.push(format!("{quoted}*"));
        } else {
            parts.push(quoted);
        }
    }

    // Implicit AND. Every term must appear somewhere in the chunk — an OR search over a large
    // corpus returns everything and ranks noise above the answer.
    Some(parts.join(" AND "))
}

/// The expression for a *task* rather than for a typed query.
///
/// `brain.context` is handed a sentence — "work out why the watcher misses renames" — and not
/// a search box's worth of words. Running that through [`match_expression`] produces a
/// conjunction of every word in it, which matches nothing: no memory contains "work" and "out"
/// and "why" and "the" and "watcher" and "misses" and "renames" all at once.
///
/// So this is a disjunction over the content words, and BM25 does the discriminating. The
/// reason that is safe here and not for the file search is corpus size. An OR over a hundred
/// thousand files returns everything and ranks noise above the answer, which is why
/// [`match_expression`] is a conjunction and says so. A memory store holds thousands, the
/// caller takes the top of the ranking rather than the whole match set, and a rare word carries
/// exactly the weight BM25 gives it.
///
/// Stopwords are dropped so that "the" does not tie a task to every memory in the index. The
/// list is short and bilingual on purpose: this project's interface is German and its content
/// is mostly English, and a stemmer for either is a much larger thing than the problem needs.
/// If every word in the task is a stopword, the whole task is used — an empty expression would
/// silently return nothing at all, which is the one outcome worse than a noisy answer.
pub fn task_expression(task: &str) -> Option<String> {
    let terms = terms(task);
    if terms.is_empty() {
        return None;
    }
    let content: Vec<&Term> = terms
        .iter()
        // Lowercased before the comparison: a task that begins with "Der" or "The" would
        // otherwise keep its stopword and tie itself to most of the index.
        .filter(|term| term.phrase || !is_stopword(&term.text.to_lowercase()))
        .collect();
    let chosen: Vec<&Term> = if content.is_empty() {
        terms.iter().collect()
    } else {
        content
    };
    let parts: Vec<String> = chosen
        .iter()
        .map(|term| {
            let quoted = format!("\"{}\"", term.text.replace('"', "\"\""));
            // A quoted phrase means what it says, and truncating it would silently widen it.
            match stem_prefix(&term.text) {
                Some(prefix) if !term.phrase => {
                    format!("({quoted} OR \"{prefix}\"*)")
                }
                _ => quoted,
            }
        })
        .collect();
    Some(parts.join(" OR "))
}

/// A truncated form of `word` to match as a prefix, or `None` when it is too short to truncate.
///
/// This is the cheapest useful substitute for a stemmer, and it exists because of a hole a live
/// run found rather than because of a theory. A memory saying "the hygiene gate runs before
/// every **commit**" was not returned for the task "**committing** a change", because the
/// tokeniser folds case and diacritics and does nothing else: `commit` and `committing` are two
/// unrelated tokens to it. With an embedding model the semantic arm covers that; rule R5 means
/// a fresh install has no model, and a context pack that only works after a download is one
/// that does not work on the day somebody tries it.
///
/// Sixty per cent of the word, floor five characters, and only for words of seven or more. That
/// lands on `commit`* for `committing`, `reposi`* for `repository`, `Umbene`* for
/// `Umbenennung` — and leaves `corpus`, `index` and `zone` alone, where truncation would turn a
/// term into a wildcard matching most of the store.
///
/// It over-matches, and deliberately: `commit`* also finds `committee`. The cost of that is one
/// slot in a ranked list that BM25 orders and diversification thins; the cost of missing is the
/// memory that answered the question never being seen at all. The two are not symmetric.
fn stem_prefix(word: &str) -> Option<String> {
    let chars: Vec<char> = word.chars().collect();
    if chars.len() < 7 {
        return None;
    }
    let keep = ((chars.len() * 3) / 5).max(5);
    if keep >= chars.len() {
        return None;
    }
    Some(chars[..keep].iter().collect())
}

/// Words too common to say anything about which memory is meant.
///
/// English and German together, and deliberately short — every entry is a word that would
/// otherwise match most of the index, and nothing else. A long list starts removing words that
/// carry meaning in a technical corpus ("state", "process", "make").
fn is_stopword(word: &str) -> bool {
    const STOPWORDS: &[&str] = &[
        // English
        "a", "an", "and", "are", "as", "at", "be", "but", "by", "can", "do", "does", "for",
        "from", "has", "have", "how", "i", "if", "in", "is", "it", "its", "me", "my", "no",
        "not", "of", "on", "or", "our", "out", "so", "that", "the", "their", "them", "then",
        "there", "these", "they", "this", "to", "up", "was", "we", "were", "what", "when",
        "where", "which", "who", "why", "will", "with", "you", "your",
        // German
        "aber", "als", "am", "auch", "auf", "aus", "bei", "bin", "bist", "das", "dass", "dem",
        "den", "der", "des", "die", "dies", "diese", "du", "ein", "eine", "einem", "einen",
        "einer", "es", "für", "hat", "ich", "ihr", "im", "ist", "kann", "mein", "mit", "nicht",
        "noch", "nur", "oder", "sein", "sich", "sie", "sind", "über", "um", "und", "uns", "von",
        "vom", "war", "was", "wenn", "wer", "wie", "wir", "wird", "zu", "zum", "zur",
    ];
    STOPWORDS.contains(&word)
}

/// The plain words of a query, for highlighting a snippet.
pub fn plain_terms(query: &str) -> Vec<String> {
    terms(query).into_iter().map(|t| t.text).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_query_becomes_a_conjunction_of_quoted_terms() {
        assert_eq!(
            match_expression("kamerawinkel blickachse", false).as_deref(),
            Some("\"kamerawinkel\" AND \"blickachse\"")
        );
    }

    #[test]
    fn a_task_becomes_a_disjunction_of_its_content_words() {
        // The failure this exists for: a whole sentence run through the conjunction matched
        // nothing at all, and a context pack that returns nothing looks exactly like an empty
        // memory.
        assert_eq!(
            task_expression("where does the reference corpus come from").as_deref(),
            Some("(\"reference\" OR \"refer\"*) OR \"corpus\" OR \"come\"")
        );
    }

    #[test]
    fn a_task_made_only_of_stopwords_keeps_them_rather_than_matching_nothing() {
        assert_eq!(
            task_expression("what is it").as_deref(),
            Some("\"what\" OR \"is\" OR \"it\"")
        );
    }

    #[test]
    fn a_german_task_loses_its_stopwords_too() {
        // Case is kept in the expression — FTS5 folds it — but the stopword check does not
        // depend on it, which a capitalised "Der" at the start of a sentence would otherwise
        // slip past.
        assert_eq!(
            task_expression("warum findet der Watcher die Umbenennung nicht").as_deref(),
            Some("\"warum\" OR \"findet\" OR (\"Watcher\" OR \"Watch\"*) OR (\"Umbenennung\" OR \"Umbene\"*)")
        );
    }

    /// The hole a live run found: `commit` and `committing` are unrelated tokens to a tokeniser
    /// that folds case and diacritics and does nothing else.
    #[test]
    fn a_long_word_is_matched_by_a_truncated_prefix_too() {
        let expression = task_expression("committing a change to the repository").unwrap();
        assert!(
            expression.contains("\"commit\"*"),
            "committing does not reach commit: {expression}"
        );
        assert!(expression.contains("\"reposi\"*"));
    }

    #[test]
    fn a_short_word_is_left_whole_because_truncating_it_would_match_everything() {
        // `gate` truncated to five characters is longer than the word; `image` to five is the
        // word itself. Neither may become a wildcard.
        for short in ["gate", "image", "index", "zone", "log"] {
            let expression = task_expression(short).unwrap();
            assert!(
                !expression.contains('*'),
                "{short} was truncated into a prefix: {expression}"
            );
        }
    }

    #[test]
    fn a_quoted_phrase_survives_being_a_stopword() {
        // Somebody who quoted "the" meant it.
        assert_eq!(
            task_expression("\"the\" watcher").as_deref(),
            // The phrase keeps its exact form; the bare word gets its prefix.
            Some("\"the\" OR (\"watcher\" OR \"watch\"*)")
        );
    }

    #[test]
    fn the_last_term_can_be_a_prefix_so_typing_shows_results_early() {
        assert_eq!(
            match_expression("budget plan", true).as_deref(),
            Some("\"budget\" AND \"plan\"*")
        );
    }

    #[test]
    fn fts_operators_typed_by_a_user_are_matched_literally() {
        // "and", "or" and "near" are words people search for. Left unquoted they would be
        // parsed as operators and change the query into something else entirely.
        assert_eq!(
            match_expression("rock and roll", false).as_deref(),
            Some("\"rock\" AND \"and\" AND \"roll\"")
        );
        assert_eq!(
            match_expression("NEAR", false).as_deref(),
            Some("\"NEAR\"")
        );
    }

    #[test]
    fn punctuation_cannot_produce_a_syntax_error() {
        // Every one of these is a parse error to FTS5 when passed through unquoted.
        for query in ["c++", "foo(bar)", "was ist das?", "-minus", "a:b", "^caret", "50%"] {
            let expression = match_expression(query, false);
            assert!(
                expression.is_some(),
                "{query:?} produced no expression at all"
            );
            let expression = expression.unwrap();
            assert!(
                !expression.contains(":") || expression.contains("\":"),
                "{query:?} leaked a column filter: {expression}"
            );
        }
    }

    #[test]
    fn an_unbalanced_quote_is_a_phrase_rather_than_an_error() {
        assert_eq!(
            match_expression("\"unbalanced phrase", false).as_deref(),
            Some("\"unbalanced phrase\"")
        );
    }

    #[test]
    fn a_quoted_phrase_stays_one_term() {
        assert_eq!(
            terms("\"grosse datei\" klein"),
            vec![
                Term { text: "grosse datei".into(), phrase: true },
                Term { text: "klein".into(), phrase: false },
            ]
        );
        assert_eq!(
            match_expression("\"grosse datei\" klein", true).as_deref(),
            Some("\"grosse datei\" AND \"klein\"*")
        );
    }

    #[test]
    fn a_quoted_phrase_is_never_turned_into_a_prefix() {
        assert_eq!(
            match_expression("\"exakt so\"", true).as_deref(),
            Some("\"exakt so\"")
        );
    }

    #[test]
    fn umlauts_and_digits_survive_tokenisation() {
        assert_eq!(
            plain_terms("Übersicht 2026 größe"),
            vec!["Übersicht", "2026", "größe"]
        );
    }

    #[test]
    fn a_query_of_pure_punctuation_yields_nothing_to_search_for() {
        // The caller must be able to tell "no lexical arm" from "an arm that found nothing",
        // or an empty query would look like a corpus with no matches.
        assert_eq!(match_expression("!!! ???", false), None);
        assert_eq!(match_expression("   ", false), None);
    }
}
