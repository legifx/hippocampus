//! One row stream, whatever produced it.
//!
//! The window has had two row-producing calls with different shapes — `fs.list` and `search` —
//! and a renderer that maps each of them into a third shape of its own. That mapping is the
//! frontend deciding what a row *is*, which is the decision D4 keeps on this side of the socket,
//! and it is why the memory a file carries has been visible in one list and not the other.
//!
//! So: a **[`Source`]** says where rows come from, a **[`View`]** is what comes back, and a
//! **[`ViewRow`]** is the same shape from every source. A directory is one query among several
//! (M10, `docs/paper/A-DIRECTORY-IS-JUST-ONE-QUERY.md`); this module is the type that makes that
//! sentence true rather than aspirational.
//!
//! Step 1 ships two sources — a directory and a query — and changes no behaviour. The others
//! named in the paper (`concept`, `working_set`, `neighbours`, `timeline`, `tier`) are step 3,
//! and they arrive as variants here rather than as new methods.

use crate::ids::{NodeId, ZoneId};
use crate::model::{NodeKind, ScoreBreakdown, SearchHit, Tier};
use crate::tools::{FsEntry, FsKind, Owner};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Where a view's rows come from.
///
/// Tagged by `kind` on the wire, so a client reads `{"kind":"dir","path":"/…"}` — and so that a
/// source it does not understand is a name it can print rather than a parse error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// A directory, listed. The spatial case, and the only one where "up" means anything.
    Dir { path: PathBuf },
    /// Hybrid search over the index.
    Query { text: String },
}

impl Source {
    /// What a client shows as "where am I". Not a path: a query view is somewhere too.
    pub fn label(&self) -> String {
        match self {
            Source::Dir { path } => path.display().to_string(),
            Source::Query { text } => text.clone(),
        }
    }

    /// Whether "go up" is a question this source can answer.
    ///
    /// Only a directory has a parent. A query view's history is where it came *from*, which is
    /// the back stack rather than a hierarchy — and a client that tried to walk up out of a
    /// search would be inventing a place.
    pub fn is_spatial(&self) -> bool {
        matches!(self, Source::Dir { .. })
    }
}

/// What the daemon sorted by. The frontend never sorts (D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    #[default]
    Name,
    Size,
    Modified,
    /// What the brain thinks matters, most first. Arithmetic on stored numbers — nothing in a
    /// listing path may wait on a model (§7 of the paper).
    Salience,
    /// The order the source produced, kept. For a query that is relevance, and re-sorting it
    /// would throw away the ranking that is the answer.
    None,
}

/// One row, from any source.
///
/// A superset rather than an enum of shapes: every field that only one source can fill is an
/// `Option`, and a renderer that draws `None` as nothing needs no branch on where the row came
/// from. `mode` is absent from a query row because the index stores what a file *is* rather than
/// the permissions it had when it was indexed, and a stale `drwxr-xr-x` is worse than none.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewRow {
    pub path: PathBuf,
    /// What the entry is — a symlink is a symlink here, and `target` says where it leads.
    pub kind: FsKind,
    pub size: u64,
    pub mtime: i64,
    /// Whether the brain knows this path. False is a real answer: "on disk, never indexed".
    pub indexed: bool,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<NodeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_id: Option<ZoneId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<Tier>,
    /// A short excerpt, when the row came from content rather than from a name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<ScoreBreakdown>,

    // The memory columns (M10 step 2). On every row from every source, from one query per view
    // rather than one per row — §5 of the paper, and the measurement in
    // `cortex-store/tests/listing_cost.rs`.
    /// What the brain thinks this is worth keeping. Absent for a path the index has never seen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salience: Option<f64>,
    /// How many edges touch it, in either direction: what "well connected" means before salience
    /// turns it into a number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degree: Option<u32>,
    /// When something last happened to it. `None` means nothing ever has, which is a different
    /// thing from "long ago" and is shown as different.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_episode: Option<i64>,
    /// Held by a person against forgetting.
    #[serde(default)]
    pub pinned: bool,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<Owner>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<Owner>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_kind: Option<FsKind>,
    #[serde(default)]
    pub broken: bool,
}

/// The answer to `view.open`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct View {
    pub rows: Vec<ViewRow>,
    /// What produced this, echoed back. A client's history stack holds these, so it must not
    /// have to remember what it asked for.
    pub origin: Source,
    pub order: Order,
    /// **What this list is, and what it is not**, in words.
    ///
    /// The cheapest honesty mechanism in the system, borrowed from the graph snapshot: a list
    /// that cannot say what it is invites the reader to invent it. A directory says how many
    /// entries there are and that nothing is ranked; a query says which arms answered.
    pub basis: String,
    /// How many exist. `rows` may be a page of them.
    pub total: u64,
    /// True when `rows` is short of `total` because a limit cut it.
    pub truncated: bool,
    /// Entries the deny-list covers: counted, never named (R1).
    #[serde(default)]
    pub hidden: u64,
    /// True when a better order is on its way as a `view.update` notification. A client that
    /// ignores it still has a complete answer.
    #[serde(default)]
    pub refining: bool,
    pub elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// A reordering of a view that has already been answered.
///
/// Its own notification rather than `search.update`'s: that one carries hits, this one carries
/// rows, and a client of the view layer should never have to know the shape underneath it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewUpdate {
    /// The request this belongs to. Applying a reordering to a later view would shuffle a list
    /// somebody is already reading, which is worse than not reranking at all.
    pub request_id: i64,
    pub rows: Vec<ViewRow>,
    pub elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl ViewRow {
    /// A row from a directory listing.
    pub fn from_entry(entry: FsEntry) -> Self {
        ViewRow {
            path: entry.path,
            kind: entry.kind,
            size: entry.size,
            mtime: entry.mtime,
            indexed: entry.indexed,
            node_id: None,
            zone_id: None,
            tier: None,
            snippet: None,
            score: None,
            salience: None,
            degree: None,
            last_episode: None,
            pinned: false,
            mode: entry.mode,
            owner: entry.owner,
            group: entry.group,
            target: entry.target,
            target_kind: entry.target_kind,
            broken: entry.broken,
        }
    }

    /// A row from a search hit.
    ///
    /// `indexed` is true by construction: a hit came out of the index. What is absent is
    /// everything that comes from a fresh `stat` — mode, owner, link target — because the index
    /// does not keep them and inventing them would put a stale answer next to a live one.
    pub fn from_hit(hit: SearchHit) -> Self {
        ViewRow {
            path: hit.path,
            kind: match hit.kind {
                NodeKind::Dir => FsKind::Dir,
                _ => FsKind::File,
            },
            size: hit.size.max(0) as u64,
            mtime: hit.mtime,
            indexed: true,
            node_id: Some(hit.node_id),
            zone_id: hit.zone_id,
            tier: Some(hit.tier),
            snippet: hit.snippet,
            score: Some(hit.score),
            salience: None,
            degree: None,
            last_episode: None,
            pinned: false,
            mode: None,
            owner: None,
            group: None,
            target: None,
            target_kind: None,
            broken: false,
        }
    }
}

/// Attach what the index knows to a row, in place.
///
/// A free function rather than a method on the store's type, because `cortex-core` must not
/// depend on the store: the caller reads the columns and hands them over. The row keeps whatever
/// the source already filled — a search hit arrives with a tier and a zone — and gains what only
/// the index has.
pub fn attach_memory(
    row: &mut ViewRow,
    node_id: NodeId,
    tier: Tier,
    salience: f64,
    pinned: bool,
    zone_id: Option<ZoneId>,
    degree: u32,
    last_episode: Option<i64>,
) {
    row.node_id = Some(node_id);
    row.tier = Some(tier);
    row.salience = Some(salience);
    row.pinned = pinned;
    row.zone_id = row.zone_id.or(zone_id);
    row.degree = Some(degree);
    row.last_episode = last_episode;
    // A path the index knows is indexed, whatever the listing believed a moment ago.
    row.indexed = true;
}

/// The sentence a directory view says about itself.
pub fn basis_for_dir(
    shown: usize,
    total: u64,
    hidden: u64,
    truncated: bool,
    order: Order,
    known: u64,
) -> String {
    let ordering = match order {
        Order::Name => "nach Namen geordnet",
        Order::Size => "nach Größe geordnet",
        Order::Modified => "nach Änderungsdatum geordnet",
        Order::Salience => "nach Salienz geordnet — was das Gedächtnis für wichtig hält",
        Order::None => "in der Reihenfolge des Dateisystems",
    };
    let mut text = format!(
        "{total} {}, {ordering} — der Ordner selbst, nichts anderes",
        if total == 1 { "Eintrag" } else { "Einträge" }
    );
    // What the memory columns are worth here, stated rather than implied: a directory the index
    // has never seen shows no salience, and a view ordered by it would be ordering nothing.
    if known < total {
        text.push_str(&format!(
            "; {known} davon kennt der Index, der Rest ist nur auf der Platte"
        ));
    }
    if truncated {
        text.push_str(&format!("; {shown} davon geladen"));
    }
    if hidden > 0 {
        text.push_str(&format!(
            "; {hidden} durch die Sperrliste verborgen und nie geöffnet"
        ));
    }
    text
}

/// The sentence a query view says about itself.
///
/// It names the mode the daemon chose, because "why is this here" starts with which arm answered
/// — and a search that fell back to paths because the model was not loaded looks exactly like a
/// bad semantic search unless it says so.
pub fn basis_for_query(text: &str, mode: &str, total: u64, refining: bool) -> String {
    let mut basis = format!(
        "{total} {} für „{text}“, nach Relevanz ({mode})",
        if total == 1 { "Treffer" } else { "Treffer" }
    );
    if refining {
        basis.push_str("; eine bessere Reihenfolge wird noch berechnet");
    }
    basis
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_says_where_it_is_and_whether_up_means_anything() {
        let dir = Source::Dir {
            path: PathBuf::from("/tmp/notes"),
        };
        let query = Source::Query {
            text: "parser bug".into(),
        };
        assert_eq!(dir.label(), "/tmp/notes");
        assert_eq!(query.label(), "parser bug");
        assert!(dir.is_spatial());
        assert!(
            !query.is_spatial(),
            "walking up out of a search would be inventing a place"
        );
    }

    #[test]
    fn a_source_survives_the_wire_it_will_be_carried_over() {
        let source = Source::Dir {
            path: PathBuf::from("/tmp/notes"),
        };
        let text = serde_json::to_string(&source).unwrap();
        assert_eq!(text, r#"{"kind":"dir","path":"/tmp/notes"}"#);
        assert_eq!(serde_json::from_str::<Source>(&text).unwrap(), source);
    }

    #[test]
    fn a_directory_row_and_a_hit_row_have_the_same_shape() {
        let entry = FsEntry {
            path: PathBuf::from("/tmp/notes.md"),
            kind: FsKind::File,
            size: 12,
            mtime: 1_700_000_000,
            indexed: true,
            mode: Some(0o100644),
            owner: Some(Owner {
                id: 1000,
                name: Some("someone".into()),
            }),
            group: None,
            target: None,
            target_kind: None,
            broken: false,
        };
        let hit = SearchHit {
            node_id: NodeId(7),
            path: PathBuf::from("/tmp/notes.md"),
            kind: NodeKind::File,
            tier: Tier::Stm,
            size: 12,
            mtime: 1_700_000_000,
            zone_id: None,
            snippet: Some("…a line…".into()),
            score: ScoreBreakdown {
                path_rank: Some(1),
                lexical_rank: None,
                vector_rank: None,
                fused: 0.5,
                salience_boost: 1.0,
                rerank: None,
                final_score: 0.5,
            },
        };

        let from_dir = ViewRow::from_entry(entry);
        let from_query = ViewRow::from_hit(hit);

        // The same file, from both sources: same path, kind, size, mtime.
        assert_eq!(from_dir.path, from_query.path);
        assert_eq!(from_dir.kind, from_query.kind);
        assert_eq!(from_dir.size, from_query.size);

        // And the fields only one source can fill are absent rather than invented.
        assert!(from_dir.node_id.is_none() && from_dir.tier.is_none());
        assert!(
            from_query.mode.is_none(),
            "the index does not keep permissions; a stale mode is worse than none"
        );
        assert!(from_query.node_id.is_some() && from_query.tier.is_some());

        // Both serialise to the same set of keys minus what is `None`, which is what makes one
        // renderer possible.
        let a = serde_json::to_value(&from_dir).unwrap();
        let b = serde_json::to_value(&from_query).unwrap();
        for required in ["path", "kind", "size", "mtime", "indexed"] {
            assert!(a.get(required).is_some(), "{required} missing from a dir row");
            assert!(b.get(required).is_some(), "{required} missing from a hit row");
        }
    }

    #[test]
    fn a_view_says_what_it_is_and_what_it_is_not() {
        let plain = basis_for_dir(3, 3, 0, false, Order::Name, 3);
        assert!(plain.contains("3 Einträge"), "{plain}");
        assert!(plain.contains("nach Namen"), "{plain}");

        // The order is named, because "why is this first" starts with what it was sorted by —
        // and salience is the one a person has not seen a file manager do before.
        let ranked = basis_for_dir(3, 3, 0, false, Order::Salience, 3);
        assert!(ranked.contains("Salienz"), "{ranked}");

        // And what the index does not know is said rather than implied: a salience column over
        // files it has never seen would be a column of zeroes pretending to be a judgement.
        let partly = basis_for_dir(10, 10, 0, false, Order::Name, 4);
        assert!(partly.contains("4 davon kennt der Index"), "{partly}");

        // The two things a listing must admit to: what a limit cut off, and what the deny-list
        // covered. Counted, never named.
        let cut = basis_for_dir(100, 5000, 2, true, Order::Name, 5000);
        assert!(cut.contains("100 davon geladen"), "{cut}");
        assert!(cut.contains("2 durch die Sperrliste"), "{cut}");

        let query = basis_for_query("parser", "hybrid", 12, true);
        assert!(query.contains("12 Treffer"), "{query}");
        assert!(query.contains("hybrid"), "{query}");
        assert!(query.contains("bessere Reihenfolge"), "{query}");
    }
}
