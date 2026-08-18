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
