//! What the window shows before it shows a directory.
//!
//! **Why one call.** The start screen answers two questions at once — *what is in this index*
//! and *what have the agents been doing to it* — and every number behind them is a `count(*)`
//! over a table the daemon already owns. Assembled in the window, that would be eight round
//! trips against a results-per-request budget, and it would put the arithmetic (shares,
//! coverage, refusal rates) in the client, which is the thing D4 forbids: the frontend paints
//! numbers, it does not make them. So the daemon counts once and ships the answer with every
//! derived value already computed.
//!
//! **Why the sentences are here.** `view::basis_for_dir` sets the precedent: a view says in
//! words what it is based on, because a number without its rule invites the reader to invent
//! one. The headline below is that rule for the line the start screen shows about agents — it
//! is composed from the same facts the tiles show, so it cannot drift from them.
//!
//! **What this is not.** No estimate, no extrapolation, no sampling. Every count is exact as of
//! `generated_at`, and the window is a plain time cut, not a rolling average. A window is
//! therefore only as meaningful as the traffic in it: on an index no agent has touched, the
//! headline says zero rather than nothing.

use serde::{Deserialize, Serialize};

/// One named bucket of a breakdown, with its share of the whole already computed.
///
/// `name` is the machine's word for the bucket (`cold`, `fact`, `fs.list`); the window has its
/// own label map, the same way it does for memory kinds. `share` is `count / total` in `0..1`,
/// and is `0.0` when the total is zero — which is why the window never divides.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Slice {
    pub name: String,
    pub count: i64,
    pub share: f64,
}

impl Slice {
    pub fn new(name: impl Into<String>, count: i64, total: i64) -> Self {
        Self {
            name: name.into(),
            count,
            share: share(count, total),
        }
    }
}

/// `count` as a fraction of `total`, and zero when there is nothing to be a fraction of.
pub fn share(count: i64, total: i64) -> f64 {
    if total <= 0 {
        0.0
    } else {
        count as f64 / total as f64
    }
}

/// The index itself: what is in it, how it is layered, and how far work has got.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct IndexFacts {
    pub files: i64,
    pub dirs: i64,
    pub chunks: i64,
    pub embeddings: i64,
    /// Vectors per chunk, `0..1`. A number below one is work in progress, not an error.
    pub vector_coverage: f64,
    pub episodes: i64,
    pub episodes_window: i64,
    pub concepts: i64,
    pub annotations: i64,
    pub edges: i64,
    pub zones: i64,
    pub pinned: i64,
    /// Files whose content is byte-identical to another file's.
    pub duplicates: i64,
    /// Vectors of deleted chunks still sitting in the ANN index (D107), cleared on the next
    /// idle pass. Reported because it is the one number that used to mean "rebuild everything,
    /// every five minutes".
    pub vectors_to_drop: i64,
    /// `working`, `stm`, `ltm`, `cold` over files and directories.
    pub tiers: Vec<Slice>,
    /// `pending`, `done`, `skipped`, `failed` over files.
    pub extract: Vec<Slice>,
    /// The edge vocabulary the graph actually uses, largest first.
    pub edge_types: Vec<Slice>,
    /// Who acted in the window: a person, or `agent:<name>`.
    pub actors_window: Vec<Slice>,
}

/// What is known and is not in a file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MemoryFacts {
    pub total: i64,
    pub human_confirmed: i64,
    pub superseded: i64,
    pub pinned: i64,
    /// Session memories whose time is up and that the next pass will remove.
    pub expiring: i64,
    /// Summed, never netted (D76): a memory useful in one place and noise in another is a
    /// memory that needs splitting, and a net of zero hides exactly that.
    pub helped: i64,
    pub noise: i64,
    pub retrieved: i64,
    /// Retrieved never, in any pack.
    pub never_retrieved: i64,
    pub kinds: Vec<Slice>,
    pub scopes: Vec<Slice>,
}

/// One agent's run against the surface.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionFacts {
    pub id: String,
    pub name: String,
    pub task: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub calls: i64,
    pub refusals: i64,
    pub memories: i64,
    pub refusal_share: f64,
}

/// One tool of the Agent Surface, as it was used in the window.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolFacts {
    pub tool: String,
    pub calls: i64,
    pub refusals: i64,
    /// Of the calls in the window, not of all time.
    pub share: f64,
}

/// How the Agent Surface was used: sessions, calls, refusals, packs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentFacts {
    pub sessions_total: i64,
    pub sessions_open: i64,
    pub sessions_window: i64,
    /// Newest first, and bounded — a start screen is not a session archive.
    pub sessions: Vec<SessionFacts>,
    pub calls_window: i64,
    pub refusals_window: i64,
    /// Refusals as a fraction of the calls in the window. The number that says whether the
    /// policy is binding or merely decorative.
    pub refusal_share: f64,
    pub calls_total: i64,
    pub refusals_total: i64,
    pub tools: Vec<ToolFacts>,
    pub packs_window: i64,
    pub packs_total: i64,
    pub pack_items: i64,
    /// Outcomes the agents reported about their own packs: the only signal in the system that
    /// says whether retrieval helped (D76).
    pub pack_helped: i64,
    pub pack_noise: i64,
}

/// The last consolidation pass, summarised.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConsolidationFacts {
    pub ran_at: i64,
    /// Seconds between the pass and this overview. `None` seconds ago is a pass that ran in the
    /// future, which is a clock change and not a scheduled pass.
    pub ago_secs: i64,
    pub dry_run: bool,
    pub episodes_replayed: usize,
    pub bursts: usize,
    pub bursts_discarded: usize,
    pub edges_written: usize,
    pub movements: usize,
    pub pinned_held: usize,
    pub concepts_total: usize,
    pub concepts_pruned: usize,
    /// Nodes somebody has touched that no association reaches, in either direction.
    pub orphans: usize,
    pub duplicates: usize,
    pub memories_expired: usize,
    pub memories_total: usize,
    pub duration_ms: u64,
    pub tier_totals: Vec<Slice>,
}

/// The process the numbers came out of.
///
/// Filled by the daemon rather than by the store, because the store has no opinion about the
/// process it happens to be running inside — and every field here is something the window must
/// be able to say even when nothing is being indexed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DaemonFacts {
    pub version: String,
    pub schema_version: i64,
    pub uptime_secs: u64,
    pub database: String,
    pub indexing_paused: bool,
    pub agent_surface_enabled: bool,
    pub require_dry_run: bool,
    pub write_allowlist: Vec<String>,
    pub indexed_paths: usize,
    pub index_memory_bytes: usize,
    /// `None` when no watcher is configured at all.
    pub watcher_active: Option<bool>,
}

/// Everything the start screen shows, in one answer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Overview {
    pub generated_at: i64,
    pub window_secs: i64,
    /// Already a phrase: „letzte 24 Stunden".
    pub window_label: String,
    pub headline: String,
    pub basis: String,
    pub index: IndexFacts,
    pub memory: MemoryFacts,
    pub agents: AgentFacts,
    pub consolidation: Option<ConsolidationFacts>,
    pub daemon: DaemonFacts,
}

impl Overview {
    /// The phrase for a window length, in the words a person would use.
    ///
    /// Only lengths the daemon itself offers get their own phrase; anything else is stated in
    /// hours, so a configuration change cannot produce „letzte 0 Sekunden".
    pub fn window_label(secs: i64) -> String {
        match secs {
            3600 => "letzte Stunde".to_string(),
            21_600 => "letzte 6 Stunden".to_string(),
            86_400 => "letzte 24 Stunden".to_string(),
            604_800 => "letzte 7 Tage".to_string(),
            2_592_000 => "letzte 30 Tage".to_string(),
            s if s % 3600 == 0 => format!("letzte {} Stunden", s / 3600),
            s => format!("letzte {s} Sekunden"),
        }
    }

    /// The same window as a phrase a sentence can use: „in den letzten 24 Stunden".
    ///
    /// A label can stand alone above a number („letzte 24 Stunden"), but it cannot be dropped into
    /// a clause — „Kein Agent hat die Oberfläche letzte 24 Stunden benutzt" is not German, and the
    /// window the sentence is about has to be the same window as the label's, decided the same way.
    /// So both come from here, one from the other.
    pub fn window_phrase(secs: i64) -> String {
        // The arms are the label's arms, in the label's order, on purpose: 72 hours is three days,
        // and a phrase that rounded it to days while the label beside it said hours would be two
        // statements about two different windows. The test below pins that they agree.
        match secs {
            3600 => "in der letzten Stunde".to_string(),
            604_800 => "in den letzten 7 Tagen".to_string(),
            2_592_000 => "in den letzten 30 Tagen".to_string(),
            s if s > 0 && s % 3600 == 0 => format!("in den letzten {} Stunden", s / 3600),
            s => format!("in den letzten {s} Sekunden"),
        }
    }

    /// Compose the sentence and the basis line from the facts that are already in here.
    ///
    /// Called once by whoever filled the numbers, so the sentence can never disagree with the
    /// tiles beside it.
    pub fn compose(&mut self) {
        self.headline = agent_headline(&self.agents, &Overview::window_phrase(self.window_secs));
        self.basis = overview_basis(self);
    }
}

/// What the agents did, in one sentence.
///
/// Written as a summary rather than as a list of the tiles, because the point of it is the
/// reading: sessions and refusals first, then the volume, then the one tool that dominates and
/// the memory outcome. A surface nothing has called says so in one clause instead of spelling
/// out four zeroes.
pub fn agent_headline(agents: &AgentFacts, window_phrase: &str) -> String {
    if agents.calls_window == 0 && agents.sessions_window == 0 {
        return format!(
            "Kein Agent hat die Oberfläche {window_phrase} benutzt — die Zahlen darunter sind \
             Bestand, keine Aktivität"
        );
    }

    let sessions = format!(
        "{} Sitzung{}, davon {} noch offen",
        agents.sessions_window,
        if agents.sessions_window == 1 { "" } else { "en" },
        agents.sessions_open
    );
    let calls = format!(
        "{} Aufruf{}, davon {} abgelehnt ({})",
        agents.calls_window,
        if agents.calls_window == 1 { "" } else { "e" },
        agents.refusals_window,
        percent(agents.refusal_share)
    );
    let mut text = format!("{sessions}; {calls}");

    if let Some(top) = agents.tools.first() {
        text.push_str(&format!("; meistgenutzt: {}", top.tool));
    }
    if agents.pack_helped > 0 || agents.pack_noise > 0 {
        text.push_str(&format!(
            "; Rückmeldungen zu Kontextpaketen: {} nützlich, {} Rauschen",
            agents.pack_helped, agents.pack_noise
        ));
    }
    text
}

/// What the start screen is based on, and what it therefore cannot say.
///
/// The two things worth stating plainly: these are counts and not estimates, and a quiet window
/// is a statement about the window rather than about the index.
pub fn overview_basis(o: &Overview) -> String {
    // No window length in here on purpose: a label cannot be dropped into a clause without the
    // sentence breaking (see `window_phrase`), and the one number this paragraph is about is the
    // distinction itself — counts against activity — not the length of the window.
    let mut text = String::from(
        "Alle Zahlen sind Zählungen aus dem Index selbst, Stand jetzt — keine Schätzung und \
         keine Stichprobe. Die Aktivitätswerte darunter sagen nur etwas über das gewählte \
         Fenster: ein Index, den niemand angefasst hat, sieht aus wie ein ruhiger Agent.",
    );
    if o.consolidation.is_none() {
        text.push_str(" Für dieses Verzeichnis hat noch keine Konsolidierung stattgefunden.");
    }
    if !o.daemon.agent_surface_enabled {
        text.push_str(" Die Agent-Oberfläche ist gerade ausgeschaltet (R7); die Zahlen sind der Stand von vorher.");
    }
    text
}

/// A share as a percentage with one decimal, in German notation.
///
/// One decimal, because the interesting refusals are the ones that happen once in a few hundred
/// calls and „0 %" is the one thing a rate must never say about a policy that is working.
pub fn percent(fraction: f64) -> String {
    format!("{:.1} %", (fraction * 1000.0).round() / 10.0).replace('.', ",")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agents(calls: i64, refusals: i64, sessions: i64) -> AgentFacts {
        AgentFacts {
            sessions_window: sessions,
            sessions_open: 1,
            calls_window: calls,
            refusals_window: refusals,
            refusal_share: share(refusals, calls),
            tools: vec![ToolFacts {
                tool: "fs.list".to_string(),
                calls,
                refusals,
                share: 1.0,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn a_share_of_nothing_is_zero_rather_than_a_division() {
        assert_eq!(share(0, 0), 0.0);
        assert_eq!(share(5, 0), 0.0);
        assert!(share(1, 4) == 0.25);
    }

    #[test]
    fn a_refusal_rate_never_reads_as_zero_when_there_was_one() {
        // 1 in 400: the rate that a policy which works produces, and the one „0 %" would hide.
        assert_eq!(percent(share(1, 400)), "0,3 %");
        assert_eq!(percent(0.0), "0,0 %");
        assert_eq!(percent(1.0), "100,0 %");
    }

    #[test]
    fn the_headline_counts_sessions_and_refusals_and_names_the_tool() {
        let text = agent_headline(&agents(42, 3, 2), "letzte 24 Stunden");
        assert!(text.contains("2 Sitzungen"), "{text}");
        assert!(text.contains("42 Aufrufe"), "{text}");
        assert!(text.contains("3 abgelehnt (7,1 %)"), "{text}");
        assert!(text.contains("meistgenutzt: fs.list"), "{text}");
    }

    #[test]
    fn a_single_call_is_not_pluralised() {
        let text = agent_headline(&agents(1, 1, 1), "in der letzten Stunde");
        assert!(text.contains("1 Sitzung,"), "{text}");
        assert!(text.contains("1 Aufruf,"), "{text}");
        assert!(text.contains("1 abgelehnt (100,0 %)"), "{text}");
    }

    #[test]
    fn a_silent_window_says_so_instead_of_printing_zeroes() {
        let text = agent_headline(&AgentFacts::default(), "in den letzten 24 Stunden");
        assert!(text.contains("Kein Agent"), "{text}");
        assert!(text.contains("Bestand, keine Aktivität"), "{text}");
        // The window is named in a clause, so it has to be the phrase and not the label.
        assert!(text.contains("in den letzten 24 Stunden"), "{text}");
    }

    #[test]
    fn the_basis_says_the_numbers_are_counts_and_owns_up_to_the_rest() {
        let mut o = Overview {
            window_secs: 86_400,
            window_label: Overview::window_label(86_400),
            ..Default::default()
        };
        o.daemon.agent_surface_enabled = true;
        o.compose();
        assert!(o.basis.contains("keine Schätzung"), "{}", o.basis);
        assert!(o.basis.contains("gewählte Fenster"), "{}", o.basis);
        // And it owns up to what is missing rather than staying silent about it.
        assert!(o.basis.contains("noch keine Konsolidierung"), "{}", o.basis);
    }

    #[test]
    fn a_window_length_the_daemon_does_not_offer_is_still_a_phrase() {
        assert_eq!(Overview::window_label(3600), "letzte Stunde");
        assert_eq!(Overview::window_label(259_200), "letzte 72 Stunden");
        assert_eq!(Overview::window_label(90), "letzte 90 Sekunden");
    }

    #[test]
    fn the_phrase_a_sentence_uses_matches_the_label_it_stands_beside() {
        assert_eq!(Overview::window_phrase(3600), "in der letzten Stunde");
        assert_eq!(Overview::window_phrase(86_400), "in den letzten 24 Stunden");
        assert_eq!(Overview::window_phrase(604_800), "in den letzten 7 Tagen");
        assert_eq!(Overview::window_phrase(2_592_000), "in den letzten 30 Tagen");
        assert_eq!(Overview::window_phrase(90), "in den letzten 90 Sekunden");
    }

    #[test]
    fn label_and_phrase_never_disagree_about_how_long_the_window_is() {
        // 72 hours is three days, and the first version of the phrase rounded it to „in den letzten
        // 3 Tagen" while the label beside it said „letzte 72 Stunden" — two statements about two
        // different windows. The digits are the part that has to match; the unit follows the label.
        for secs in [3600, 21_600, 86_400, 604_800, 2_592_000, 259_200, 90] {
            let label = Overview::window_label(secs);
            let phrase = Overview::window_phrase(secs);
            assert!(
                label.split_whitespace().all(|word| !word.chars().all(|c| c.is_ascii_digit())
                    || phrase.split_whitespace().any(|w| w == word)),
                "phrase {phrase:?} does not name what {label:?} names"
            );
        }
    }
}
