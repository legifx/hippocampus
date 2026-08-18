//! Generating file contents that behave like text.
//!
//! The first version of this generator filled every file with sentences drawn from a list of
//! sixteen. That is fine for a benchmark that only indexes paths, and actively misleading for
//! one that measures retrieval: with sixteen sentences, every term occurs in almost every
//! document. BM25 then has nothing to discriminate with, and ranking a query means scoring a
//! third of the corpus — which measured as 44 ms against a 30 ms budget, not because retrieval
//! was slow but because the input was not text.
//!
//! Real text obeys Zipf's law: the frequency of a word is roughly inversely proportional to its
//! rank, so a handful of words are everywhere and the great majority are rare. Rare is what
//! makes a query selective. This module therefore builds a large vocabulary and samples it
//! Zipfian, which reproduces the property that actually matters for retrieval: **a query term
//! matches a small fraction of the corpus.**
//!
//! Two other properties are deliberate:
//!
//! - **Topics.** Each file draws most of its content from a small topic vocabulary, so terms
//!   cluster in documents instead of being sprinkled uniformly. Without this, every document
//!   looks alike to a retrieval system and the goldset cannot have right answers.
//! **No landmark sentences.** An earlier version sprinkled the fixed German/English sentence list
//! through the corpus so a goldset would have memorable phrases to search for. The goldset now
//! brings its own documents, and the sentences turned out to be actively harmful: they are real
//! prose on real topics, several of which the goldset also covers, so the generated "distractors"
//! contained genuine answers that no judgement had marked relevant. A reranker that found one was
//! then scored as having made a mistake — which is how an unjudged relevant document turns a
//! measurement into its own opposite.
//!
//! Distractors must be *plausible and irrelevant*. Making them irrelevant is the part that has to
//! be true by construction.

use crate::rng::Rng;
use crate::words;

/// Distinct words in the generated vocabulary.
///
/// A real personal corpus of this size has tens of thousands of distinct word forms — German
/// compounding pushes it higher still. Twenty thousand is the low end of realistic, chosen
/// because the tail beyond it contributes nothing a benchmark can observe.
const VOCABULARY: usize = 20_000;

/// Zipf exponent. 1.0 is the classical law; natural language sits slightly above it.
const ZIPF_S: f64 = 1.07;

/// Where the searchable vocabulary starts in the ranking.
///
/// At this rank a word occurs in a few per cent of documents, which is what a content word does
/// in a real corpus: common enough that a query for it returns a list, rare enough that the list
/// is not the whole index. Everything above it is function words and filler.
const DOMAIN_RANK: usize = 1_000;

/// A vocabulary with a sampling distribution over it.
///
/// Built once and shared by every file, which is what makes term frequencies corpus-wide rather
/// than per file.
pub struct Vocabulary {
    words: Vec<String>,
    /// Cumulative Zipf weights, for inverse-transform sampling by binary search.
    cumulative: Vec<f64>,
    total: f64,
}

impl Vocabulary {
    pub fn build() -> Self {
        let mut words: Vec<String> = Vec::with_capacity(VOCABULARY);
        // The word lists overlap — "draft" is both a noun and an adjective — and a duplicate
        // would be one word occupying two ranks, which is not a distribution.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut synthetic = 0usize;

        let push = |word: &str, words: &mut Vec<String>, seen: &mut std::collections::HashSet<String>| {
            if seen.insert(word.to_string()) {
                words.push(word.to_string());
            }
        };

        // Rank order *is* frequency order, so where a word sits decides how often it occurs.
        // Function words at the head, because that is where they are in every real corpus.
        for word in words::FUNCTION_WORDS {
            push(word, &mut words, &mut seen);
        }

        // Then filler, so that the words a person would actually search for land at a middling
        // rank. A domain noun at rank one would appear in nearly every document, and a query
        // for it would select a third of the corpus — the exact defect this corpus had before.
        while words.len() < DOMAIN_RANK {
            let word = synthesise(synthetic);
            synthetic += 1;
            push(&word, &mut words, &mut seen);
        }

        for list in [words::EN_NOUNS, words::DE_NOUNS, words::ADJECTIVES, words::PROJECT_NAMES] {
            for word in list {
                push(word, &mut words, &mut seen);
            }
        }

        // Everything after that is synthesised, deterministically and without an RNG, so the
        // vocabulary itself is identical on every machine regardless of the seed.
        while words.len() < VOCABULARY {
            let word = synthesise(synthetic);
            synthetic += 1;
            push(&word, &mut words, &mut seen);
        }

        let mut cumulative = Vec::with_capacity(words.len());
        let mut total = 0.0;
        for rank in 1..=words.len() {
            total += 1.0 / (rank as f64).powf(ZIPF_S);
            cumulative.push(total);
        }

        Self {
            words,
            cumulative,
            total,
        }
    }

    /// Draw one word, Zipf-distributed.
    fn sample(&self, rng: &mut Rng) -> &str {
        // Inverse transform: a uniform draw against the cumulative weights. Binary search keeps
        // this logarithmic, which matters when it runs tens of millions of times.
        let target = (rng.next_u64() as f64 / u64::MAX as f64) * self.total;
        let index = self
            .cumulative
            .partition_point(|c| *c < target)
            .min(self.words.len() - 1);
        &self.words[index]
    }

    /// A topic: a handful of words this document keeps coming back to.
    fn topic(&self, rng: &mut Rng) -> Vec<&str> {
        (0..rng.range(3, 8)).map(|_| self.sample(rng)).collect()
    }

    pub fn len(&self) -> usize {
        self.words.len()
    }
}

/// Build word number `index` of the synthetic tail.
///
/// Syllable combination rather than random letters: the result has to survive a tokenizer and
/// look like a word to anyone who opens a file, and letter soup would also defeat the umlaut
/// handling this corpus exists to exercise.
fn synthesise(index: usize) -> String {
    const ONSETS: &[&str] = &[
        "b", "d", "f", "g", "h", "k", "l", "m", "n", "p", "r", "s", "t", "v", "w", "z", "st",
        "str", "sch", "br", "kl", "gr", "tr", "fl", "pf",
    ];
    const NUCLEI: &[&str] = &["a", "e", "i", "o", "u", "ä", "ö", "ü", "au", "ei", "ie", "ou"];
    const CODAS: &[&str] = &["", "n", "r", "t", "s", "ng", "ch", "ll", "nd", "rk", "st"];
    const SUFFIXES: &[&str] = &[
        "", "ung", "heit", "keit", "lich", "ion", "ment", "ance", "ings", "erei", "schaft",
    ];

    let mut n = index;
    let mut word = String::new();
    // Two syllables plus a suffix gives roughly 20 000 × the suffix count of distinct forms,
    // which is more than the vocabulary needs.
    for _ in 0..2 {
        word.push_str(ONSETS[n % ONSETS.len()]);
        n /= ONSETS.len();
        word.push_str(NUCLEI[n % NUCLEI.len()]);
        n /= NUCLEI.len();
        word.push_str(CODAS[n % CODAS.len()]);
        n /= CODAS.len();
    }
    word.push_str(SUFFIXES[n % SUFFIXES.len()]);
    word
}

/// How long a document is, in sentences.
///
/// Heavily skewed, because document lengths are: most files are a note, a few are a manuscript.
/// A uniform length distribution would make every chunk boundary look the same and would never
/// exercise the hard-split path.
fn sentence_count(rng: &mut Rng) -> usize {
    match rng.below(100) {
        0..=59 => rng.range(3, 20),
        60..=97 => rng.range(20, 120),
        _ => rng.range(200, 800),
    }
}

/// Write one document.
pub fn write_document(
    out: &mut impl std::io::Write,
    vocabulary: &Vocabulary,
    rng: &mut Rng,
) -> std::io::Result<()> {
    let topic = vocabulary.topic(rng);
    let sentences = sentence_count(rng);

    for i in 0..sentences {
        // A heading every so often, so the Markdown chunker has structure to find.
        if i % 40 == 0 {
            writeln!(out, "\n## {}\n", capitalise(topic[i % topic.len()]))?;
        }

        let mut sentence = String::new();
        for word in 0..rng.range(6, 18) {
            if word > 0 {
                sentence.push(' ');
            }
            // Topic words are what make a document *about* something. Two in five is enough to
            // cluster a document without turning it into a list of the same word.
            let next = if rng.chance(40) {
                topic[rng.below(topic.len())]
            } else {
                vocabulary.sample(rng)
            };
            if word == 0 {
                sentence.push_str(&capitalise(next));
            } else {
                sentence.push_str(next);
            }
        }
        sentence.push('.');
        writeln!(out, "{sentence}")?;
    }

    Ok(())
}

fn capitalise(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn the_vocabulary_is_the_size_it_claims_and_has_no_duplicates() {
        let vocabulary = Vocabulary::build();
        assert_eq!(vocabulary.len(), VOCABULARY);
        let distinct: std::collections::HashSet<&String> = vocabulary.words.iter().collect();
        assert_eq!(
            distinct.len(),
            vocabulary.len(),
            "a duplicate word would be two ranks with one frequency, which is not Zipf"
        );
    }

    #[test]
    fn a_word_worth_searching_for_is_neither_ubiquitous_nor_absent() {
        // The number that decides whether the BM25 benchmark measures retrieval or measures a
        // full scan. A domain word must appear in a small percentage of documents.
        let vocabulary = Vocabulary::build();
        let rank = vocabulary
            .words
            .iter()
            .position(|w| w == "rechnung")
            .expect("the domain vocabulary must be in there");
        assert!(
            rank >= DOMAIN_RANK,
            "a searchable word sits at rank {rank}, which would make it a stopword"
        );

        let mut documents_containing = 0;
        let documents = 200;
        for seed in 1..=documents {
            let mut out = Vec::new();
            write_document(&mut out, &vocabulary, &mut Rng::new(seed)).unwrap();
            if String::from_utf8(out).unwrap().to_lowercase().contains("rechnung") {
                documents_containing += 1;
            }
        }
        let share = documents_containing as f64 / documents as f64;
        assert!(
            (0.005..0.25).contains(&share),
            "a benchmark query word appears in {:.1} % of documents",
            share * 100.0
        );
    }

    #[test]
    fn the_vocabulary_does_not_depend_on_the_seed() {
        // Two corpora from different seeds must still share a vocabulary, or their term
        // frequencies are not comparable and neither are their benchmark numbers.
        assert_eq!(Vocabulary::build().words, Vocabulary::build().words);
    }

    #[test]
    fn word_frequencies_follow_zipf_rather_than_being_uniform() {
        // The property the whole module exists for, stated as the two things Zipf actually
        // says: the head carries most of the mass, and the tail carries almost none. Uniform
        // sampling would put every term in every document, and a query would then select a
        // third of the corpus — which is what made the first version of this corpus useless
        // for measuring retrieval.
        let vocabulary = Vocabulary::build();
        let mut rng = Rng::new(1);
        let mut counts: HashMap<&str, usize> = HashMap::new();
        let draws = 200_000;
        for _ in 0..draws {
            *counts.entry(vocabulary.sample(&mut rng)).or_default() += 1;
        }

        let mut ranked: Vec<usize> = counts.values().copied().collect();
        ranked.sort_unstable_by(|a, b| b.cmp(a));

        let share = |n: usize| ranked.iter().take(n).sum::<usize>() as f64 / draws as f64;
        assert!(
            share(1) > 0.02,
            "the most frequent word took only {:.2} % of draws",
            share(1) * 100.0
        );
        assert!(
            share(100) > 0.40,
            "the hundred most frequent words took only {:.1} % of draws",
            share(100) * 100.0
        );

        // And the tail: a word from the back half of the vocabulary is genuinely rare, which is
        // what makes a query built from one selective.
        let tail: usize = ranked.iter().skip(ranked.len() / 2).sum();
        let tail_share = tail as f64 / draws as f64;
        assert!(
            tail_share < 0.20,
            "the rare half of the vocabulary took {:.1} % of draws",
            tail_share * 100.0
        );
    }

    #[test]
    fn a_document_is_reproducible_from_its_seed() {
        let vocabulary = Vocabulary::build();
        let render = || {
            let mut out = Vec::new();
            write_document(&mut out, &vocabulary, &mut Rng::new(42)).unwrap();
            out
        };
        assert_eq!(render(), render());
    }

    #[test]
    fn documents_from_different_seeds_are_about_different_things() {
        let vocabulary = Vocabulary::build();
        let render = |seed| {
            let mut out = Vec::new();
            write_document(&mut out, &vocabulary, &mut Rng::new(seed)).unwrap();
            String::from_utf8(out).unwrap()
        };
        assert_ne!(render(1), render(2));
    }

    #[test]
    fn no_generated_document_states_anything_true() {
        // Distractors have to be irrelevant *by construction*. When they were not — an earlier
        // version planted real sentences among them — the goldset scored a reranker as wrong for
        // finding a passage that genuinely answered the question, and the resulting number said
        // the reranker made retrieval worse. It did not; the corpus did.
        let vocabulary = Vocabulary::build();
        for seed in 1..40u64 {
            let mut out = Vec::new();
            write_document(&mut out, &vocabulary, &mut Rng::new(seed)).unwrap();
            let text = String::from_utf8(out).unwrap();
            for sentence in words::SENTENCES {
                assert!(
                    !text.contains(sentence),
                    "a generated document contains a real sentence: {sentence:?}"
                );
            }
        }
    }
}
