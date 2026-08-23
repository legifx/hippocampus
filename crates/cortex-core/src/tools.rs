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

/// Parameters for `brain.graph` — one snapshot of the brain, for a visualisation.
///
/// Everything is optional and every default is a number the daemon chose, not the caller: a
/// view that picked its own limit would produce a different picture per client, and the whole
/// point of the Agent View is that it shows what the daemon believes rather than what a
/// frontend decided to draw.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphParams {
    /// How many nodes, chosen by salience. Ancestors are added on top so nothing floats.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Only nodes at or above this salience.
    #[serde(default)]
    pub min_salience: Option<f64>,
    /// Restrict to one zone.
    #[serde(default)]
    pub zone: Option<i64>,
    /// How many recent episodes to send along, for the animation.
    #[serde(default)]
    pub events: Option<usize>,
    /// How far back "recently touched" reaches, in seconds.
    #[serde(default)]
    pub window_secs: Option<i64>,
}

/// Parameters for `brain.consolidate`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConsolidateRequest {
    /// Compute the whole pass and report it, but write nothing.
    #[serde(default)]
    pub dry_run: bool,
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

// ---------------------------------------------------------------------------
// Places: where a person starts, as opposed to where the filesystem starts.
// ---------------------------------------------------------------------------

/// What a place is, so a client can draw it without knowing what it means.
///
/// The kind is not decoration. "Downloads" and "an external disk" are different things to a
/// person — one is always there and one may be unplugged mid-copy — and a list that drew them
/// identically would be hiding the difference that matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaceKind {
    Home,
    Desktop,
    Documents,
    Downloads,
    Pictures,
    Music,
    Videos,
    Trash,
    /// A mounted volume: an internal disk, a stick, a card.
    Drive,
    /// The root of the filesystem. One entry, so that everything technical is behind one row
    /// rather than spread across the first screen a person sees.
    Filesystem,
    /// Somewhere the person marked. Shared with the rest of the desktop.
    Bookmark,
}

/// How full a volume is. `None` for anything that is not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capacity {
    pub total_bytes: u64,
    pub free_bytes: u64,
}

impl Capacity {
    /// How much is in use, from 0 to 1. Zero when the total is zero, which happens for
    /// pseudo-filesystems and is not a division anybody wants to perform.
    pub fn used_fraction(&self) -> f32 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        let used = self.total_bytes.saturating_sub(self.free_bytes);
        (used as f64 / self.total_bytes as f64) as f32
    }
}

/// What a drive is, beyond being somewhere to go.
///
/// Only drives carry this, and the field a client must look at before anything else is
/// [`VolumeInfo::mounted`]: **a volume that is not mounted has no directory yet**, so its
/// [`Place::path`] is the device node — `/dev/sdb1` — and listing it would fail. It is a place
/// in the sense that a person points at it and says "open that"; what happens then is
/// `fs.mount` first and the mount point second.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeInfo {
    pub mounted: bool,
    pub fstype: String,
    pub size_bytes: u64,
    /// Whether the disk it lives on can be unplugged, which is the only case where ejecting
    /// means anything.
    pub removable: bool,
}

/// Somewhere worth starting from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    pub path: PathBuf,
    /// What to call it. The filesystem label for a volume, the folder's own name otherwise —
    /// never the path, because a column of paths is what this screen exists to replace.
    pub name: String,
    pub kind: PlaceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity: Option<Capacity>,
    /// The device behind a mount, for telling two disks of the same size apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// Present for drives, absent for everything else. See [`VolumeInfo`] — in particular that
    /// an unmounted volume's `path` is a device node and not a directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<VolumeInfo>,
}

/// One heading and what is under it.
///
/// The grouping is the daemon's, like every other ordering decision here (D4): a frontend that
/// sorted places into sections would be a second, quietly different opinion about which of them
/// is personal and which is plumbing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaceSection {
    pub title: String,
    pub places: Vec<Place>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsMountParams {
    /// The device node, as a `Place` gives it: `/dev/sdb1`.
    pub device: PathBuf,
    /// Let polkit ask for a password if the device needs one.
    ///
    /// **Only ever set by a client acting on something a person just did**, and only after an
    /// ordinary attempt came back with [`crate::error::NEEDS_AUTHORISATION`]. An internal disk
    /// requires an administrator, and the ordinary path refuses rather than wait — see D61 for
    /// why the second attempt is a separate request instead of a retry inside the first.
    #[serde(default)]
    pub interactive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsMountResult {
    pub device: PathBuf,
    /// Where udisks2 put it. Chosen by udisks2 and not asked for, so that this window and every
    /// other program agree about where the stick is.
    pub mount_point: PathBuf,
    /// False when it was already mounted and nothing had to be done.
    pub mounted_now: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsUnmountParams {
    pub device: PathBuf,
    /// Cut the power to the drive afterwards, so it can be unplugged. Refused for a disk that
    /// cannot be unplugged.
    #[serde(default)]
    pub eject: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsUnmountResult {
    pub device: PathBuf,
    /// Whether the drive was powered off as well, which is what makes it safe to pull out.
    pub ejected: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsPlacesParams {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsPlacesResult {
    pub sections: Vec<PlaceSection>,
}

// ---------------------------------------------------------------------------
// Applications: what opens this file, and what running it means.
// ---------------------------------------------------------------------------

/// An installed application that says it can open a type of file.
///
/// This is a desktop entry, reduced to what a chooser needs to draw a row and what a launcher
/// needs to start it. The entry is *read* by the daemon (`cortex-agent/src/apps.rs`) and *run*
/// by whichever surface has a session to run it in — the daemon is headless and has no business
/// spawning a window (D49). [`Application::command_line`] is the seam between those two halves,
/// and it lives here so that both use the same one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Application {
    /// The desktop file id — `org.gnome.Nautilus.desktop`. What `mimeapps.list` records, and
    /// therefore what a caller names when it sets a default.
    pub id: String,
    /// What to call it, in the session's language when the entry offers one.
    pub name: String,
    /// The entry's own one-line description. Two applications called "Bildbetrachter" are told
    /// apart by this and not by their names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    /// The `Exec` line, verbatim, field codes and all. Expanded by [`Application::command_line`]
    /// rather than here, because expanding it needs the files.
    pub exec: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Whether this wants a terminal around it. Which terminal is a property of the session, so
    /// the flag travels and the decision does not.
    pub terminal: bool,
    /// Whether this is what the desktop would open the file with — what `xdg-open` does.
    pub is_default: bool,
    /// The desktop entry this was read from. `%k` expands to it, and it is how a chooser can
    /// tell two entries with the same id in different data directories apart.
    pub path: PathBuf,
}

impl Application {
    /// The argv to run, with the field codes of the Desktop Entry Specification expanded.
    ///
    /// Three rules in here are the specification's rather than a preference, and each of them is
    /// a way of opening the wrong thing if it is guessed at instead:
    ///
    /// - **`%f` takes one file and `%F` takes all of them.** An application declaring `%f` can
    ///   open one document per invocation; handing it five would open the first and silently
    ///   drop four. The caller opens the rest by calling again.
    /// - **An `Exec` line with no field code at all gets no files.** That is not an oversight to
    ///   be corrected by appending them: an entry like `Exec=gimp-2.10` means the application
    ///   takes its documents some other way, and a path appended to it is an argument it never
    ///   agreed to parse.
    /// - **The deprecated codes are dropped, not passed through.** `%d`, `%D`, `%n`, `%N`, `%v`
    ///   and `%m` are removed from the standard; leaving them in argv passes the literal text
    ///   `%d` to the program.
    ///
    /// The terminal flag is *not* applied here. Wrapping a command in a terminal emulator means
    /// choosing one, and that is the session's business rather than this type's.
    pub fn command_line(&self, paths: &[PathBuf]) -> Vec<String> {
        let mut argv = Vec::new();
        for token in split_exec(&self.exec) {
            match token.as_str() {
                // The list forms, which are the only codes that may become more than one
                // argument — and may become none, for a launch with no file at all.
                "%F" => argv.extend(paths.iter().map(|p| p.display().to_string())),
                "%U" => argv.extend(paths.iter().map(|p| crate::uri::file_uri(p))),
                // `%i` is two arguments or none: the specification spells out that it expands to
                // `--icon` *and* its value, and to nothing when the entry has no icon.
                "%i" => {
                    if let Some(icon) = &self.icon {
                        argv.push("--icon".to_string());
                        argv.push(icon.clone());
                    }
                }
                "%d" | "%D" | "%n" | "%N" | "%v" | "%m" => {}
                _ => {
                    if let Some(expanded) = expand_in_token(&token, self, paths) {
                        argv.push(expanded);
                    }
                }
            }
        }
        argv
    }

    /// Every command line it takes to open these files with this application.
    ///
    /// Usually one. An entry declaring `%f` rather than `%F` can open a single document per
    /// invocation, and the specification's answer to a selection of five is to launch it five
    /// times — the alternative is opening one of them and silently dropping four, which is the
    /// sort of thing somebody discovers a week later.
    ///
    /// Launching is not here for the reason [`Application::command_line`] gives: the daemon has
    /// no session to launch anything into. This decides *what* would be run, which is data and
    /// can be tested; the surface that has a session does the spawning.
    pub fn command_lines(&self, paths: &[PathBuf]) -> Vec<Vec<String>> {
        let tokens = split_exec(&self.exec);
        let takes_all = tokens.iter().any(|t| t.contains("%F") || t.contains("%U"));
        let takes_one = tokens.iter().any(|t| t.contains("%f") || t.contains("%u"));

        if paths.len() > 1 && takes_one && !takes_all {
            return paths
                .iter()
                .map(|p| self.command_line(std::slice::from_ref(p)))
                .collect();
        }
        vec![self.command_line(paths)]
    }

    /// Whether running this opens the files, or merely starts the application.
    ///
    /// A chooser needs to know: offering "Öffnen mit GIMP" for an entry that cannot be handed a
    /// file promises something that will not happen.
    pub fn takes_files(&self) -> bool {
        split_exec(&self.exec)
            .iter()
            .any(|t| ["%f", "%F", "%u", "%U"].iter().any(|c| t.contains(c)))
    }
}

/// Expand the single-file codes inside one token, or drop the token.
///
/// Returns `None` when the token was a single-file code and there is no file to put in it —
/// `Exec=xdg-open %f` launched with nothing is `xdg-open`, not `xdg-open %f`.
fn expand_in_token(token: &str, app: &Application, paths: &[PathBuf]) -> Option<String> {
    if !token.contains('%') {
        return Some(token.to_string());
    }
    let first = paths.first();
    let mut out = String::with_capacity(token.len());
    let mut chars = token.chars().peekable();
    let mut dropped = false;
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some('f') => match first {
                Some(p) => out.push_str(&p.display().to_string()),
                None => dropped = true,
            },
            Some('u') => match first {
                Some(p) => out.push_str(&crate::uri::file_uri(p)),
                None => dropped = true,
            },
            Some('c') => out.push_str(&app.name),
            Some('k') => out.push_str(&app.path.display().to_string()),
            // An unknown code is not text to be passed on. A program receiving `%z` would treat
            // it as a file name.
            Some(_) => {}
            None => out.push('%'),
        }
    }
    (!dropped).then_some(out)
}

/// Split an `Exec` line into arguments, obeying the specification's quoting.
///
/// Not `shlex` and not `split_whitespace`: the rule is narrower than a shell's and wider than
/// whitespace. Double quotes group, a backslash inside them escapes the four characters the
/// specification names, and single quotes are ordinary characters — a file manager that treated
/// them as quotes would mangle every entry belonging to an application with an apostrophe in
/// its path.
fn split_exec(exec: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut started = false;
    let mut chars = exec.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                started = true;
            }
            '\\' if in_quotes => match chars.next() {
                Some(escaped @ ('"' | '\\' | '$' | '`')) => current.push(escaped),
                Some(other) => {
                    current.push('\\');
                    current.push(other);
                }
                None => current.push('\\'),
            },
            c if c.is_whitespace() && !in_quotes => {
                if started || !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if started || !current.is_empty() {
        out.push(current);
    }
    out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppsForPathParams {
    pub path: PathBuf,
    /// Every installed application instead of the ones that claim the type.
    ///
    /// For "open with something else". The type's own list is the right answer almost always,
    /// and the times it is not are exactly the times somebody needs to reach past it — a file
    /// with no extension, a type nothing has registered for.
    #[serde(default)]
    pub all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppsForPathResult {
    pub path: PathBuf,
    /// What the file is, as the desktop's own database names it. `inode/directory` for a
    /// directory, and `application/octet-stream` when nothing recognises it.
    pub mime: String,
    /// What can open it. The default first, then everything else in the order the association
    /// files give — which is the order the rest of the desktop shows them in.
    pub applications: Vec<Application>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppsSetDefaultParams {
    pub mime: String,
    /// The desktop file id, as [`Application::id`] gives it.
    pub application: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppsSetDefaultResult {
    pub mime: String,
    pub application: String,
    /// The file that was written. Every other application on the desktop reads it too, which is
    /// the point of writing there rather than into a preference of our own.
    pub file: PathBuf,
}

#[cfg(test)]
mod exec_tests {
    use super::*;

    fn app(exec: &str) -> Application {
        Application {
            id: "test.desktop".into(),
            name: "Ein Programm".into(),
            comment: None,
            exec: exec.into(),
            icon: None,
            terminal: false,
            is_default: false,
            path: PathBuf::from("/usr/share/applications/test.desktop"),
        }
    }

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn one_file_for_the_singular_code_and_all_of_them_for_the_plural() {
        let one = app("viewer %f").command_line(&paths(&["/a/x.png", "/a/y.png"]));
        assert_eq!(one, vec!["viewer", "/a/x.png"], "%f opens one document per invocation");

        let all = app("viewer %F").command_line(&paths(&["/a/x.png", "/a/y.png"]));
        assert_eq!(all, vec!["viewer", "/a/x.png", "/a/y.png"]);
    }

    #[test]
    fn the_uri_codes_hand_over_a_uri_and_not_a_path() {
        assert_eq!(
            app("browser %u").command_line(&paths(&["/a/two words.html"])),
            vec!["browser", "file:///a/two%20words.html"]
        );
        assert_eq!(
            app("browser %U").command_line(&paths(&["/a/x", "/a/y"])),
            vec!["browser", "file:///a/x", "file:///a/y"]
        );
    }

    #[test]
    fn an_entry_with_no_field_code_is_launched_with_no_files() {
        // Appending them would be inventing an interface the entry never claimed to have.
        assert_eq!(app("gimp-2.10").command_line(&paths(&["/a/x.png"])), vec!["gimp-2.10"]);
        assert!(!app("gimp-2.10").takes_files());
        assert!(app("viewer %U").takes_files());
    }

    #[test]
    fn a_code_with_nothing_to_put_in_it_leaves_no_argument_behind() {
        // Launching an application from a chooser with no selection is ordinary. The literal
        // text `%f` reaching the program would be a file it cannot find.
        assert_eq!(app("viewer %f").command_line(&[]), vec!["viewer"]);
        assert_eq!(app("viewer %F").command_line(&[]), vec!["viewer"]);
        assert_eq!(app("viewer --file=%f").command_line(&[]), vec!["viewer"]);
    }

    #[test]
    fn a_code_inside_a_token_is_substituted_in_place() {
        assert_eq!(
            app("viewer --file=%f --title=%c").command_line(&paths(&["/a/x"])),
            vec!["viewer", "--file=/a/x", "--title=Ein Programm"]
        );
    }

    #[test]
    fn double_quotes_group_and_single_quotes_do_not() {
        assert_eq!(
            app(r#""/opt/An Application/run" --flag %f"#).command_line(&paths(&["/a/x"])),
            vec!["/opt/An Application/run", "--flag", "/a/x"]
        );
        assert_eq!(
            app("/opt/l'application/run %f").command_line(&paths(&["/a/x"])),
            vec!["/opt/l'application/run", "/a/x"],
            "an apostrophe in a path is a character, not a quote"
        );
    }

    #[test]
    fn a_backslash_escapes_only_inside_quotes_and_only_what_the_standard_names() {
        assert_eq!(
            app(r#"run "a \"quoted\" thing""#).command_line(&[]),
            vec!["run", r#"a "quoted" thing"#]
        );
        assert_eq!(
            app(r#"run "a \$dollar""#).command_line(&[]),
            vec!["run", "a $dollar"]
        );
    }

    #[test]
    fn a_doubled_percent_is_a_literal_one() {
        assert_eq!(app("run 100%%").command_line(&[]), vec!["run", "100%"]);
    }

    #[test]
    fn the_icon_code_is_two_arguments_or_none() {
        let mut with_icon = app("run %i %f");
        with_icon.icon = Some("bild".into());
        assert_eq!(
            with_icon.command_line(&paths(&["/a/x"])),
            vec!["run", "--icon", "bild", "/a/x"]
        );
        assert_eq!(app("run %i %f").command_line(&paths(&["/a/x"])), vec!["run", "/a/x"]);
    }

    #[test]
    fn the_deprecated_codes_are_removed_rather_than_passed_on() {
        // `%d` reaching a program is a file called `%d`.
        assert_eq!(
            app("run %d %D %n %N %v %m %f").command_line(&paths(&["/a/x"])),
            vec!["run", "/a/x"]
        );
    }

    #[test]
    fn an_application_that_takes_one_file_is_started_once_per_file() {
        let files = paths(&["/a/one.txt", "/a/two.txt", "/a/three.txt"]);
        assert_eq!(
            app("editor %f").command_lines(&files),
            vec![
                vec!["editor", "/a/one.txt"],
                vec!["editor", "/a/two.txt"],
                vec!["editor", "/a/three.txt"],
            ]
        );
    }

    #[test]
    fn an_application_that_takes_all_of_them_is_started_once() {
        let files = paths(&["/a/one.txt", "/a/two.txt"]);
        assert_eq!(
            app("editor %F").command_lines(&files),
            vec![vec!["editor", "/a/one.txt", "/a/two.txt"]]
        );
        // And an entry declaring both is one invocation, not four.
        assert_eq!(app("editor %f %U").command_lines(&files).len(), 1);
    }

    #[test]
    fn one_file_and_no_file_are_both_a_single_invocation() {
        assert_eq!(app("editor %f").command_lines(&paths(&["/a/one"])).len(), 1);
        assert_eq!(app("editor %f").command_lines(&[]), vec![vec!["editor"]]);
        assert_eq!(
            app("editor").command_lines(&paths(&["/a/one", "/a/two"])),
            vec![vec!["editor"]],
            "an entry that takes no files is not launched twice for two of them"
        );
    }

    #[test]
    fn an_empty_quoted_argument_survives() {
        // `run "" x` is three arguments, and dropping the empty one shifts every later one.
        assert_eq!(app(r#"run "" x"#).command_line(&[]), vec!["run", "", "x"]);
    }
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
