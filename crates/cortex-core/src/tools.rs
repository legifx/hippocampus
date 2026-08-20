//! The Agent Surface's vocabulary: what a tool is asked for, and what it answers.
//!
//! These types live here rather than in the RPC crate because three surfaces share them — the
//! socket, the CLI and the MCP server — and one of those (the CLI) has no business linking the
//! storage layer. A tool whose arguments are spelled out once cannot be spelled differently by
//! its second caller, which is what M3 §5 asks to be proven.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsListParams {
    pub path: PathBuf,
    /// A shell-style pattern matched against each entry's file name. `*` and `?` only — a full
    /// glob dialect invites patterns whose cost is not obvious from reading them.
    #[serde(default)]
    pub glob: Option<String>,
    /// How far to descend. 1 is the directory itself.
    #[serde(default)]
    pub depth: Option<u32>,
    #[serde(default)]
    pub limit: Option<usize>,
    /// How to order the answer.
    ///
    /// Here rather than in the client, and that is D4's second condition rather than a
    /// preference: a file manager sorting its own listing is a file manager doing data
    /// processing, and two clients would eventually disagree about what "by name" means. The
    /// daemon decides once, and every surface shows the same order.
    #[serde(default)]
    pub sort: SortBy,
    #[serde(default)]
    pub reverse: bool,
}

/// The orders a listing can be asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
    /// Case-insensitive by file name, with directories first. What a file manager opens with.
    Name,
    Size,
    /// Most recently modified first — the one order where the default is descending, because
    /// "sorted by date" has only ever meant "newest at the top".
    Modified,
    /// The order the filesystem gave them. Cheap, and honest about being arbitrary.
    None,
}

impl Default for SortBy {
    fn default() -> Self {
        SortBy::Name
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FsEntry {
    pub path: PathBuf,
    /// What the entry *is*, not what it points at. A symlink is a symlink here, and where it
    /// leads is `target`. Reporting a link as its target is the mistake M6 means by "symbolic
    /// links displayed correctly": it makes a link indistinguishable from a copy, and deleting
    /// one look like deleting the other.
    pub kind: FsKind,
    pub size: u64,
    pub mtime: i64,
    /// Whether the brain knows this path. An agent that can see the difference can tell
    /// "not found" from "not indexed yet", which is the most common confusion at this surface.
    pub indexed: bool,
    /// The permission bits, as `stat` gives them. `None` where the platform has no such idea.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    /// Who owns it. The name where it could be resolved, the number otherwise — a file manager
    /// that shows `1000` has told you less than it knows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<Owner>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<Owner>,
    /// Where a symlink points, verbatim — relative if it was written relative, because that is
    /// what the link says and rewriting it would be answering a different question.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<PathBuf>,
    /// What the target turns out to be, so a link to a directory can be opened like one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_kind: Option<FsKind>,
    /// A link whose target is not there. Worth showing rather than hiding: a broken link looks
    /// exactly like a working one until somebody tries it.
    #[serde(default)]
    pub broken: bool,
}

/// A user or a group: the number, and the name if the system could give one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owner {
    pub id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Owner {
    pub fn label(&self) -> String {
        match &self.name {
            Some(name) => name.clone(),
            None => self.id.to_string(),
        }
    }
}

/// Render permission bits the way every Unix tool does.
///
/// Written out because the format is a contract with the reader's eyes rather than a formatting
/// preference: `drwxr-xr-x` is read at a glance by anybody who has used a terminal, and a
/// prettier invention would have to be learned.
pub fn format_mode(kind: FsKind, mode: u32) -> String {
    let mut out = String::with_capacity(10);
    out.push(match kind {
        FsKind::Dir => 'd',
        FsKind::Symlink => 'l',
        FsKind::File => '-',
        FsKind::Other => '?',
    });

    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 0o7;
        out.push(if bits & 0o4 != 0 { 'r' } else { '-' });
        out.push(if bits & 0o2 != 0 { 'w' } else { '-' });

        // The execute column carries setuid, setgid and the sticky bit, which is where a
        // surprise hides: a file that is `rwsr-xr-x` is not the same as one that is `rwxr-xr-x`.
        let executable = bits & 0o1 != 0;
        let special = match shift {
            6 => mode & 0o4000 != 0,
            3 => mode & 0o2000 != 0,
            _ => mode & 0o1000 != 0,
        };
        out.push(match (special, executable, shift) {
            (true, true, 0) => 't',
            (true, false, 0) => 'T',
            (true, true, _) => 's',
            (true, false, _) => 'S',
            (false, true, _) => 'x',
            (false, false, _) => '-',
        });
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsKind {
    File,
    Dir,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsListResult {
    pub path: PathBuf,
    pub entries: Vec<FsEntry>,
    /// True when `limit` cut the answer short.
    pub truncated: bool,
    /// Entries that exist but were not listed because the deny-list covers them. Counted
    /// rather than named: the count keeps the answer honest, the names would leak what R1
    /// exists to hide.
    pub hidden: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsReadParams {
    pub path: PathBuf,
    /// Half-open byte range `[start, end)`. Absent means "from the beginning, up to the cap".
    #[serde(default)]
    pub byte_range: Option<(u64, u64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsReadResult {
    pub path: PathBuf,
    pub content: String,
    /// Byte offset the content starts at, so a ranged read can be reassembled.
    pub offset: u64,
    /// Bytes returned, which is not `content.len()` once invalid UTF-8 has been replaced.
    pub bytes: u64,
    pub total_bytes: u64,
    pub truncated: bool,
    /// Set when the file was not valid UTF-8 and lossy replacement happened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsStatParams {
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsStatResult {
    pub path: PathBuf,
    pub exists: bool,
    pub kind: Option<FsKind>,
    pub size: u64,
    pub mtime: i64,
    pub readable: bool,
    /// Whether an agent could mutate this path if it asked.
    pub writable: bool,
    pub indexed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}

// ---------------------------------------------------------------------------
// Mutation. Everything below is gated: allow-list, dry-run, episode, undo entry.
// ---------------------------------------------------------------------------

/// How a write treats a file that is already there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteMode {
    /// Fail if the path exists. The safe default, and the default in the wire format.
    Create,
    /// Replace the contents. The previous version goes to the trash so the write is undoable.
    Overwrite,
    Append,
}

impl Default for WriteMode {
    fn default() -> Self {
        WriteMode::Create
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsWriteParams {
    pub path: PathBuf,
    pub content: String,
    #[serde(default)]
    pub mode: WriteMode,
    /// The id of a plan returned by a previous dry run. Absent means "plan it, do not do it".
    #[serde(default)]
    pub plan: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsMoveParams {
    pub from: PathBuf,
    pub to: PathBuf,
    #[serde(default)]
    pub plan: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsTrashParams {
    pub path: PathBuf,
    #[serde(default)]
    pub plan: Option<String>,
}

/// What a mutation *would* do. Returned by every gated tool called without a plan id.
///
/// A plan is not advice. It is the only way to reach the mutation, and it expires — both
/// because a filesystem an agent looked at ten minutes ago is not the one it is about to
/// write to, and because an approval that never runs out is not an approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub tool: String,
    pub steps: Vec<PlanStep>,
    /// Seconds until this plan stops being accepted.
    pub expires_in_secs: i64,
    /// Plain sentences, in the order the steps run. What a confirmation dialog shows.
    pub summary: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanStep {
    pub action: PlanAction,
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<PathBuf>,
    /// What is currently at `path`, if anything — the thing that would be lost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces: Option<FsEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanAction {
    Create,
    Overwrite,
    Append,
    Move,
    Trash,
}

impl PlanAction {
    pub fn as_str(self) -> &'static str {
        match self {
            PlanAction::Create => "create",
            PlanAction::Overwrite => "overwrite",
            PlanAction::Append => "append",
            PlanAction::Move => "move",
            PlanAction::Trash => "trash",
        }
    }
}

/// The answer to a gated call: either the plan it needs first, or what it did.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Mutation {
    /// Nothing happened. Call again with `plan` set to this plan's id.
    Planned { plan: Plan },
    Applied {
        /// The journal entry that can undo this.
        undo_id: i64,
        steps: Vec<PlanStep>,
        /// Where the displaced content went, if anywhere.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trashed: Option<PathBuf>,
    },
}

// ---------------------------------------------------------------------------
// Undo
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoParams {
    /// How many journal entries to reverse, newest first.
    #[serde(default)]
    pub last: Option<usize>,
    /// Reverse everything one plan did, whatever its size.
    #[serde(default)]
    pub plan: Option<String>,
    /// Report what would be reversed without reversing it.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoResult {
    pub undone: Vec<UndoneEntry>,
    /// Entries that could not be reversed, and why. An undo that half worked must say so.
    pub failed: Vec<UndoFailure>,
    pub dry_run: bool,
    pub remaining: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoneEntry {
    pub id: i64,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoFailure {
    pub id: i64,
    pub description: String,
    pub reason: String,
}

/// Plain sentences describing what a set of steps does, in the order they run.
///
/// Here rather than in the policy crate because all three surfaces print it: the CLI before
/// asking for confirmation, the MCP server as the tool's answer, and the GUI in its dialog. A
/// plan that reads differently depending on who shows it is a plan somebody approves twice.
pub fn describe_steps(steps: &[PlanStep]) -> Vec<String> {
    steps
        .iter()
        .map(|step| match (step.action, &step.from, &step.replaces) {
            (PlanAction::Move, Some(from), None) => {
                format!("move {} to {}", from.display(), step.path.display())
            }
            (PlanAction::Move, Some(from), Some(replaced)) => format!(
                "move {} to {}, replacing the {} bytes already there",
                from.display(),
                step.path.display(),
                replaced.size
            ),
            (PlanAction::Move, None, _) => format!("move something to {}", step.path.display()),
            (PlanAction::Trash, _, _) => format!("move {} to the trash", step.path.display()),
            (PlanAction::Create, _, _) => format!("create {}", step.path.display()),
            (PlanAction::Overwrite, _, Some(replaced)) => format!(
                "overwrite {}, discarding the {} bytes there (recoverable with `cortex undo`)",
                step.path.display(),
                replaced.size
            ),
            (PlanAction::Overwrite, _, None) => format!("write {}", step.path.display()),
            (PlanAction::Append, _, _) => format!("append to {}", step.path.display()),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The brain tools. These write to memory and never to a file, which is why they are gated by
// the kill switch and audited but not by the write allow-list: an agent's notes about a
// document cannot damage the document (rule R6).
// ---------------------------------------------------------------------------

/// A node, named the way the caller happens to know it.
///
/// Agents work in paths and the index works in ids, and forcing either side to translate makes
/// one of them keep a mapping it will get wrong. Untagged, so `"/srv/data/a.txt"` and `42` are
/// both valid without a wrapper object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Target {
    Node(i64),
    Path(PathBuf),
}

impl Target {
    pub fn describe(&self) -> String {
        match self {
            Target::Node(id) => format!("node {id}"),
            Target::Path(p) => p.display().to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotateParams {
    pub target: Target,
    #[serde(default)]
    pub tags: Vec<String>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotateResult {
    /// The annotation's own node id — it is a node, so it can be linked and found like one.
    pub annotation_id: i64,
    pub target_id: i64,
    pub target_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinParams {
    pub target: Target,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinResult {
    pub target_id: i64,
    pub pinned: bool,
    /// What the salience engine will do with this. Honest about M5 not being built yet.
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkParams {
    pub a: Target,
    pub b: Target,
    /// One of the edge types the schema allows. `similar_to` when the caller says nothing.
    #[serde(default)]
    pub edge: Option<String>,
    /// Remove the edge instead of asserting it.
    #[serde(default)]
    pub remove: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkResult {
    pub a_id: i64,
    pub b_id: i64,
    pub edge: String,
    pub removed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeighborsParams {
    pub target: Target,
    /// Which edge types to follow. Empty means all of them.
    #[serde(default)]
    pub edge_types: Vec<String>,
    #[serde(default)]
    pub depth: Option<u32>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeighborsResult {
    pub target_id: i64,
    pub neighbors: Vec<crate::model::NeighborView>,
    /// Annotations on the node itself, spelled out rather than left as ids to chase.
    pub annotations: Vec<crate::model::AnnotationView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineParams {
    /// Unix seconds, inclusive. Absent means "from the beginning".
    #[serde(default)]
    pub from: Option<i64>,
    #[serde(default)]
    pub to: Option<i64>,
    /// Only this actor's episodes, e.g. `agent:claude`.
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineResult {
    pub episodes: Vec<crate::model::Episode>,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingSetResult {
    /// What has been touched recently, most recent first.
    pub nodes: Vec<WorkingSetEntry>,
    /// Seconds since a client last reported human activity, if one ever has.
    pub idle_secs: Option<u64>,
    /// How this was derived. Working memory proper arrives with salience in M5; until then
    /// this is recent episodes, and it says so rather than implying more.
    pub basis: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingSetEntry {
    pub node_id: i64,
    pub path: Option<PathBuf>,
    pub last_touched: i64,
    pub actions: Vec<String>,
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffParams {
    /// Everything after this episode id. An agent keeps the `cursor` from the last answer and
    /// passes it back, which is what makes this a substitute for polling.
    #[serde(default)]
    pub since: Option<i64>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffResult {
    pub changes: Vec<crate::model::Episode>,
    /// Pass this back as `since` next time.
    pub cursor: i64,
    /// True when `limit` cut the answer short and another call would return more.
    pub more: bool,
}

// ---------------------------------------------------------------------------
// Transfers. Copying and moving are the two operations that can take a minute, so they are the
// two that answer with a handle instead of a result.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferMode {
    Copy,
    Move,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsTransferParams {
    /// What to move or copy. Several, because selecting five files and dragging them is one
    /// operation to the person doing it and should be one operation here.
    pub sources: Vec<PathBuf>,
    /// The directory they end up in.
    pub destination: PathBuf,
    pub mode: TransferMode,
    #[serde(default)]
    pub conflict: ConflictChoice,
    /// Answer with what would happen instead of doing it.
    ///
    /// This is how a client learns *which* names already exist, so it can ask before anything
    /// moves rather than reporting a failure afterwards. It reuses the same checks the real
    /// call makes, so a dry run that says "no conflicts" cannot be contradicted by a real run
    /// that finds one — barring somebody else writing the file in between, which the transfer
    /// then refuses on its own.
    #[serde(default)]
    pub dry_run: bool,
    /// A confirmed plan, for an agent. A human's client has already asked.
    #[serde(default)]
    pub plan: Option<String>,
}

/// One thing already at the destination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflict {
    pub source: PathBuf,
    pub destination: PathBuf,
    /// What is there now — so the question can be "replace these 2 kB from March with these
    /// 40 kB from today?" rather than "overwrite?".
    pub existing: FsEntry,
    /// And what would arrive.
    pub incoming: FsEntry,
}

/// What `fs.transfer` answers: either what it would do, or the handle of what it is doing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum TransferAnswer {
    /// A dry run. Nothing has moved.
    Planned {
        total_files: u64,
        total_bytes: u64,
        conflicts: Vec<Conflict>,
    },
    Started {
        operation: u64,
        total_files: u64,
        total_bytes: u64,
    },
}

/// What to do when something is already there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictChoice {
    /// Stop and say so. The default, because the other two each lose something.
    Fail,
    Replace,
    /// Keep both: the arrival becomes `name (2).ext`.
    Keep,
    /// Leave the existing one alone and count it as done.
    Skip,
}

impl Default for ConflictChoice {
    fn default() -> Self {
        ConflictChoice::Fail
    }
}

/// The answer to starting a transfer: a handle, immediately.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferStarted {
    pub operation: u64,
    /// What it is going to move, measured before the first byte.
    pub total_files: u64,
    pub total_bytes: u64,
}

/// Sent as the transfer runs. Throttled by the daemon — a notification per chunk would be a
/// thousand messages a second saying nothing new.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferProgress {
    pub operation: u64,
    pub files_done: u64,
    pub bytes_done: u64,
    pub total_files: u64,
    pub total_bytes: u64,
    /// Zero to one.
    pub fraction: f64,
    pub current: Option<PathBuf>,
}

/// Sent once, however it ended.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferDone {
    pub operation: u64,
    pub moved: Vec<PathBuf>,
    /// Sources that could not be transferred, each with the reason. A batch that stopped at the
    /// first problem would leave a person guessing which of five files arrived.
    pub failed: Vec<TransferFailure>,
    pub cancelled: bool,
    /// The journal entries that can undo what did happen.
    pub undo_ids: Vec<i64>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferFailure {
    pub source: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationCancelParams {
    pub operation: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsMkdirParams {
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsRenameParams {
    pub path: PathBuf,
    /// The new name, not a path: renaming is within a directory, and accepting a path here
    /// would make "rename" and "move" the same call with different expectations about what
    /// happens when the target is somewhere else entirely.
    pub name: String,
}

#[cfg(test)]
mod mode_tests {
    use super::*;

    #[test]
    fn the_common_cases_read_the_way_ls_prints_them() {
        assert_eq!(format_mode(FsKind::File, 0o644), "-rw-r--r--");
        assert_eq!(format_mode(FsKind::Dir, 0o755), "drwxr-xr-x");
        assert_eq!(format_mode(FsKind::Symlink, 0o777), "lrwxrwxrwx");
        assert_eq!(format_mode(FsKind::File, 0o600), "-rw-------");
        assert_eq!(format_mode(FsKind::File, 0), "----------");
    }

    #[test]
    fn a_full_stat_mode_ignores_the_type_bits_it_carries() {
        // `st_mode` is the permission bits *and* the file type. Formatting it without masking
        // would give a nonsense first triplet, and the type is already known from `kind`.
        assert_eq!(format_mode(FsKind::File, 0o100644), "-rw-r--r--");
        assert_eq!(format_mode(FsKind::Dir, 0o040755), "drwxr-xr-x");
    }

    #[test]
    fn setuid_setgid_and_sticky_replace_the_execute_column() {
        // The distinction that matters: `-rwsr-xr-x` runs as its owner and `-rwxr-xr-x` does
        // not, and they differ by one character a reader would otherwise never see.
        assert_eq!(format_mode(FsKind::File, 0o4755), "-rwsr-xr-x");
        assert_eq!(format_mode(FsKind::File, 0o2755), "-rwxr-sr-x");
        assert_eq!(format_mode(FsKind::Dir, 0o1777), "drwxrwxrwt", "like /tmp");

        // Capitals where the bit is set but the execute bit is not — an oddity worth showing
        // rather than smoothing over, because it is usually a mistake.
        assert_eq!(format_mode(FsKind::File, 0o4644), "-rwSr--r--");
        assert_eq!(format_mode(FsKind::File, 0o2644), "-rw-r-Sr--");
        assert_eq!(format_mode(FsKind::Dir, 0o1666), "drw-rw-rwT");
    }

    #[test]
    fn every_mode_gives_exactly_ten_characters() {
        // The column is aligned in the UI, so a mode that formats to nine or eleven characters
        // would shift every row after it.
        for mode in 0..0o10000u32 {
            assert_eq!(format_mode(FsKind::File, mode).chars().count(), 10, "{mode:o}");
        }
    }
}
