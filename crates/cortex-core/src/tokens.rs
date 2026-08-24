//! Estimating how much of a context window a piece of text will cost.
//!
//! ## Why this is an estimate, and why it is not `chars / 4`
//!
//! A context pack is budgeted in tokens because that is the unit the caller's context window is
//! measured in. The daemon cannot know which tokeniser that caller uses: Claude, GPT and Llama
//! all tokenise the same sentence differently, and the one tokeniser this repository does ship —
//! the embedding model's WordPiece vocabulary — belongs to a 384-dimension retrieval model and
//! has nothing to do with any of them. Budgeting a language model's context with it would be
//! precise about the wrong number, which is worse than being honestly approximate about the
//! right one. See D79.
//!
//! So this is a structural estimate rather than a divisor, and it is **deliberately biased
//! upwards**. The two errors are not symmetric: over-estimating wastes a little of the budget,
//! and under-estimating truncates a system prompt at a byte boundary.
//!
//! ## What it counts
//!
//! Byte-pair encodings do not merge across a word boundary, so **every word costs at least one
//! token** — that is a real lower bound rather than a guess, and it is asserted in the tests.
//! Beyond it:
//!
//! * a run of ASCII letters or digits costs one token per [`CHARS_PER_TOKEN`] characters,
//!   rounded up, never fewer than one;
//! * punctuation costs one token per character, which is what these vocabularies do to
//!   anything that is not part of a word;
//! * a newline costs a token; a run of ordinary spaces does not, because a leading space is
//!   merged into the word that follows it;
//! * anything outside ASCII costs one token per character. Accented Latin is roughly half
//!   that and CJK is roughly one, so this is the upward bias where it matters most — German
//!   text with umlauts is exactly the case a naive divisor gets worst.

/// Characters per token inside a word, for the built-in estimate.
///
/// 3.5 rather than 4: four is the number quoted for English prose, and the text this system
/// carries is disproportionately paths, code and identifiers, which tokenise harder.
pub const CHARS_PER_TOKEN: f64 = 3.5;

/// Estimate with a caller-supplied calibration.
///
/// The knob exists so that somebody who has measured their own model against their own corpus
/// can say so, rather than being told a number this crate invented.
pub fn estimate_with(text: &str, chars_per_token: f64) -> usize {
    let chars_per_token = if chars_per_token.is_finite() && chars_per_token >= 1.0 {
        chars_per_token
    } else {
        CHARS_PER_TOKEN
    };

    let mut total = 0usize;
    let mut word_len = 0usize;

    let flush = |word_len: &mut usize, total: &mut usize| {
        if *word_len > 0 {
            let cost = (*word_len as f64 / chars_per_token).ceil() as usize;
            *total += cost.max(1);
            *word_len = 0;
        }
    };

    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            word_len += 1;
            continue;
        }
        flush(&mut word_len, &mut total);

        if ch == '\n' || ch == '\r' {
            total += 1;
        } else if ch.is_whitespace() {
            // Absorbed into the following token, which is how a leading space is encoded.
        } else if ch.is_ascii() {
            total += 1;
        } else {
            // Non-ASCII: one per character, the upward-biased bound.
            total += 1;
        }
    }
    flush(&mut word_len, &mut total);
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default calibration, for the tests. Not a public entry point: every caller in this
    /// workspace has a calibration of its own to pass, and a convenience wrapper whose only
    /// users are tests is the shape of dead code this project has now found five times.
    fn estimate(text: &str) -> usize {
        estimate_with(text, CHARS_PER_TOKEN)
    }

    fn words(text: &str) -> usize {
        text.split_whitespace().count()
    }

    /// The property that makes this safe to budget with: a byte-pair encoding cannot merge two
    /// words into one token, so the true count is never below the word count — and neither is
    /// the estimate.
    #[test]
    fn the_estimate_is_never_below_the_word_count() {
        for sample in SAMPLES {
            assert!(
                estimate(sample) >= words(sample),
                "estimate {} is below the {} words of {sample:?}",
                estimate(sample),
                words(sample)
            );
        }
    }

    /// And never above one token per character, which no tokeniser exceeds on text.
    #[test]
    fn the_estimate_is_never_above_one_token_per_character() {
        for sample in SAMPLES {
            assert!(
                estimate(sample) <= sample.chars().count().max(1),
                "estimate {} exceeds the {} characters of {sample:?}",
                estimate(sample),
                sample.chars().count()
            );
        }
    }

    #[test]
    fn german_costs_more_than_the_naive_divisor_says() {
        // The case a `chars / 4` estimate gets worst, and the reason the bias is upwards.
        let german = "Die Änderungen an der Größe des Verzeichnisses überraschten niemanden.";
        assert!(estimate(german) > german.chars().count() / 4);
    }

    #[test]
    fn nothing_costs_nothing_and_something_costs_something() {
        assert_eq!(estimate(""), 0);
        assert_eq!(estimate("   "), 0, "a run of spaces is absorbed");
        assert!(estimate("a") >= 1);
    }

    #[test]
    fn newlines_are_counted_because_they_are_tokens() {
        assert!(estimate("a\nb") > estimate("a b"));
    }

    #[test]
    fn the_estimate_grows_with_the_text() {
        let mut previous = 0;
        let mut text = String::new();
        for word in "the quick brown fox jumps over the lazy dog".split(' ') {
            text.push_str(word);
            text.push(' ');
            let now = estimate(&text);
            assert!(now >= previous);
            previous = now;
        }
    }

    #[test]
    fn a_calibration_below_one_is_ignored_rather_than_dividing_by_nothing() {
        assert_eq!(estimate_with("hello world", 0.0), estimate("hello world"));
        assert_eq!(estimate_with("hello world", f64::NAN), estimate("hello world"));
    }

    #[test]
    fn a_generous_calibration_costs_fewer_tokens() {
        let text = "a reasonably long sentence with several ordinary english words in it";
        assert!(estimate_with(text, 6.0) <= estimate_with(text, 3.0));
    }

    const SAMPLES: &[&str] = &[
        "the reference corpus is generated from a seed, never checked in",
        "Die Änderungen an der Größe des Verzeichnisses überraschten niemanden.",
        "fn main() { let x: Vec<String> = vec![]; println!(\"{x:?}\"); }",
        "/usr/share/doc/example/README.md",
        "a",
        "one two three four five six seven eight nine ten",
        "SELECT count(*) FROM nodes WHERE kind = 'memory' AND salience > 0.5",
        "🙂 emoji and 中文 in the same line",
    ];
}
