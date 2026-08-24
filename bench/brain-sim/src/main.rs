//! Simulated usage, run through the real memory.
//!
//! Two jobs, one program, because they are the same program:
//!
//! 1. **The M5 test.** Definition-of-Done item 1 asks what an index looks like after thirty days
//!    of use. This builds a synthetic workspace, invents a plausible history over it, and runs
//!    the real consolidation once per simulated day.
//! 2. **The demo the Agent View ships with.** The public repository cannot contain a snapshot of
//!    anybody's real index — rule R9 — but a view with no data is not a demo. So the demo is a
//!    *generator*: deterministic from a seed, and every number in it was produced by the same
//!    `Store::consolidate` the daemon runs. Nothing here reimplements the arithmetic, which is
//!    the only way the demo can be trusted to show what the system actually does.
//!
//! The history is invented; the memory's response to it is not.

use anyhow::Result;
use clap::Parser;
use cortex_core::ids::{NodeId, SessionId};
use cortex_core::model::{Action, Actor, NodeKind};
use cortex_store::memory::{ConsolidateParams, GraphParams};
use cortex_store::queries::NodeUpsert;
use cortex_store::Store;
use std::path::PathBuf;

const DAY: i64 = 86_400;
/// Midnight UTC on 1 March 2027, as a fixed origin. A wall clock would make the output differ
/// between runs, and the whole value of this program is that it does not.
const EPOCH: i64 = 1_803_000_000;

#[derive(Parser, Debug)]
#[command(name = "brain-sim", about = "Simulate usage and consolidate it with the real engine")]
struct Args {
    /// Where to write the snapshot. `-` writes to standard output.
    #[arg(long, default_value = "graph.json")]
    out: String,
    /// Days of simulated use.
    #[arg(long, default_value_t = 60)]
    days: i64,
    /// Roughly how many files the synthetic workspace holds.
    #[arg(long, default_value_t = 1400)]
    files: usize,
    /// Change this and you get a different, equally reproducible history.
    #[arg(long, default_value_t = 20_270_301)]
    seed: u64,
    /// How many nodes the snapshot may contain.
    #[arg(long, default_value_t = 3000)]
    limit: usize,
    /// Print what each day's consolidation did.
    #[arg(long)]
    verbose: bool,
}

/// xorshift64*. Small, deterministic, and adequate for choosing which file a fictional person
/// opened next — this is a scene generator, not a source of randomness anything depends on.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

/// One project in the synthetic workspace.
struct Project {
    name: &'static str,
    /// How much of the fictional person's attention it gets, relative to the others.
    weight: u32,
    /// The day it stops being worked on. `None` means it stays live to the end.
    abandoned_after: Option<i64>,
}

const PROJECTS: &[Project] = &[
    Project { name: "atlas", weight: 40, abandoned_after: None },
    Project { name: "kettle", weight: 24, abandoned_after: None },
    Project { name: "harbour", weight: 16, abandoned_after: Some(22) },
    Project { name: "notes", weight: 12, abandoned_after: None },
    Project { name: "vellum", weight: 8, abandoned_after: Some(9) },
];

const SOURCE_DIRS: &[&str] = &["src", "tests", "docs", "assets"];
const GENERATED_DIRS: &[&str] = &["target", "node_modules", ".cache"];
const EXTENSIONS: &[&str] = &["rs", "ts", "md", "toml", "css", "py", "json"];
/// How many distinct working sets each project has. Small clusters, revisited.
/// One per subject vocabulary, so each has something of its own to be named after.
const CLUSTERS: usize = SUBJECTS.len();

/// One vocabulary per working set, so a cluster has something to be named after.
///
/// The first version drew every file name from one global list, which meant the only word a
/// cluster's files reliably shared was the project's own name — and the abstraction step
/// correctly refused to name nine clusters `kettle`. Real work is not like that: the files
/// somebody keeps open together are about the same thing and are named accordingly.
const SUBJECTS: &[&[&str]] = &[
    &["parser", "token", "grammar", "lexer"],
    &["render", "canvas", "shader", "sprite"],
    &["invoice", "ledger", "tax", "payment"],
    &["session", "login", "token", "cookie"],
    &["queue", "worker", "retry", "backoff"],
    &["schema", "migration", "column", "index"],
    &["router", "route", "redirect", "path"],
    &["cache", "evict", "warm", "hit"],
    &["report", "chart", "export", "summary"],
];

/// A file in the synthetic workspace, and what it is for.
struct File {
    id: NodeId,
    project: usize,
    /// Generated output. Touched by builds, never by a person, and penalised as noise.
    generated: bool,
    /// Files that tend to be opened together with this one.
    cluster: usize,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let store = Store::open_in_memory()?;
    let mut rng = Rng(args.seed | 1);

    let files = build_workspace(&store, &mut rng, args.files)?;
    let history_end = simulate(&store, &mut rng, &files, args.days)?;
    // An agent has been working here too, and it has learned things. Without this the published
    // snapshot draws the files and leaves out the half of the brain M11 is about — which was
    // exactly the state of `brain.graph` until somebody looked.
    let memories = simulate_agent(&store, &mut rng, &files, args.days)?;

    // One pass per simulated day, so decay and demotion happen the way they would in use — a
    // single pass at the end would measure a different system.
    // Just after the last episode. A snapshot taken at the following midnight would show an
    // empty Working Set, which is correct and useless — the tier that shows what somebody is
    // doing right now is only interesting while they are doing it.
    let now = history_end + 20 * 60;

    let mut last = None;
    for day in 0..args.days {
        let at = EPOCH + day * DAY + 23 * 3_600;
        if at >= now {
            break;
        }
        let report = store.consolidate(&ConsolidateParams::new(at))?;
        if args.verbose {
            eprintln!("day {day:>3}  {}", report.render().lines().next().unwrap_or(""));
            for movement in &report.movements {
                eprintln!("          {} → {}  {}", movement.from, movement.to, movement.count);
            }
        }
        last = Some(report);
    }
    // The pass the snapshot ships with is a real one, a day after the one before it. A final
    // pass at the same instant as its predecessor would report no movement at all, and the
    // animation would truthfully show a consolidation in which nothing happened.
    last = Some(store.consolidate(&ConsolidateParams::new(now))?);

    let mut snapshot = store.graph_snapshot(&GraphParams {
        limit: args.limit,
        events: 900,
        now,
        ..GraphParams::default()
    })?;
    snapshot.report = store.last_consolidation()?;
    snapshot.basis = format!(
        "synthetic workspace, {} simulated days, {memories} memories an invented agent wrote \
         while working in it. Every path here is invented; every salience, tier and edge was \
         computed by the same engine the daemon runs. {}",
        args.days, snapshot.basis
    );

    let text = serde_json::to_string(&snapshot)?;
    if args.out == "-" {
        println!("{text}");
    } else {
        let path = PathBuf::from(&args.out);
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(&path, text.as_bytes())?;
        eprintln!(
            "{}: {} nodes, {} edges, {} events",
            args.out,
            snapshot.nodes.len(),
            snapshot.edges.len(),
            snapshot.events.len()
        );
    }

    if let Some(report) = last {
        eprintln!("\n{}", report.render());
    }
    Ok(())
}

/// Build the tree: projects, their directories, and files inside them.
fn build_workspace(store: &Store, rng: &mut Rng, target: usize) -> Result<Vec<File>> {
    let created = EPOCH - 200 * DAY;
    let mut files = Vec::new();

    let root = upsert(store, NodeKind::Dir, "/w", None, created, 0)?;
    for (index, project) in PROJECTS.iter().enumerate() {
        let project_dir = upsert(
            store,
            NodeKind::Dir,
            &format!("/w/{}", project.name),
            Some(root),
            created,
            0,
        )?;
        store.upsert_zone(&cortex_core::zones::Zone {
            id: None,
            root_path: PathBuf::from(format!("/w/{}", project.name)),
            name: project.name.to_string(),
            // Spread over the wheel rather than derived from the name: five projects that
            // happened to hash close together would be five indistinguishable colours.
            color_hue: ((index * 67) % 360) as u16,
            icon: None,
            user_override: false,
        })?;

        let share = target * project.weight as usize / 100;
        for directory in SOURCE_DIRS.iter().chain(GENERATED_DIRS.iter()) {
            let generated = GENERATED_DIRS.contains(directory);
            let dir_node = upsert(
                store,
                NodeKind::Dir,
                &format!("/w/{}/{directory}", project.name),
                Some(project_dir),
                created,
                0,
            )?;
            // Generated trees are large and nobody reads them, which is exactly the shape that
            // makes forgetting worth having.
            let count = if generated { share / 2 } else { share / 6 };
            for i in 0..count.max(1) {
                let subject = SUBJECTS[(i / 5) % SUBJECTS.len()];
                let stem = subject[rng.below(subject.len())];
                let ext = EXTENSIONS[rng.below(EXTENSIONS.len())];
                let path = format!("/w/{}/{directory}/{stem}_{i}.{ext}", project.name);
                let id = upsert(
                    store,
                    NodeKind::File,
                    &path,
                    Some(dir_node),
                    created,
                    (400 + rng.below(60_000)) as i64,
                )?;
                files.push(File {
                    id,
                    project: index,
                    generated,
                    // Small clusters on purpose. With thirty files to choose from, a pair
                    // almost never comes up twice and no association is ever confirmed —
                    // which is true of the algorithm *and* of real work.
                    cluster: i / 5,
                });
            }
        }
    }

    store.assign_zones()?;
    Ok(files)
}

fn upsert(
    store: &Store,
    kind: NodeKind,
    path: &str,
    parent: Option<NodeId>,
    now: i64,
    size: i64,
) -> Result<NodeId> {
    Ok(store.upsert_node(
        &NodeUpsert {
            kind,
            path: PathBuf::from(path),
            parent_id: parent,
            mime: None,
            size,
            mtime: now,
            ctime: now,
            inode: 0,
            dev: 0,
            zone_id: None,
        },
        now,
    )?)
}

/// Invent a working history: sessions, mostly inside one project, plus builds that touch
/// everything and must not become associations.
/// An agent working alongside the human, and what it learns.
///
/// Deliberately sparse: a memory every few days, not one per session. An agent that remembered
/// something every hour would produce a graph in which memories outnumber files, which is not
/// what this system is for and would make the picture a lie in the flattering direction.
///
/// The outcome reports are the part worth simulating rather than assuming. Roughly half of what
/// is written is later reported useful, a fifth is reported as noise, and the rest is never
/// mentioned at all — which is what makes the decay of the unmentioned visible in the snapshot
/// instead of being a claim in a decision log.
fn simulate_agent(store: &Store, rng: &mut Rng, files: &[File], days: i64) -> Result<usize> {
    use cortex_core::memory::{MemoryKind, MemoryScope, PackArm, Reporter};
    use cortex_store::memories::{NewMemory, PackItemRecord};

    let agent = Actor::Agent("claude".into());
    let subjects: Vec<&File> = files.iter().filter(|f| !f.generated).collect();
    if subjects.is_empty() {
        return Ok(0);
    }

    // Sentence shapes, so the text reads like something rather than like a fixture. Nothing
    // here is claimed to be true of anything; it is a plausible sentence with a plausible shape.
    let claims: &[(MemoryKind, &str)] = &[
        (MemoryKind::Procedure, "the check has to run before the commit, not after it"),
        (MemoryKind::Failure, "the shortcut here was tried and it loses the ordering"),
        (MemoryKind::Decision, "kept the simpler structure; the faster one could not explain itself"),
        (MemoryKind::Fact, "this directory is output rather than input, whatever its name says"),
        (MemoryKind::Observation, "the numbers here were measured on a busy machine and are not comparable"),
        (MemoryKind::Preference, "answer in German unless asked otherwise"),
        (MemoryKind::Fact, "the generated files under this project are rewritten on every build"),
        (MemoryKind::Procedure, "regenerate the corpus before measuring anything against it"),
    ];

    let mut written: Vec<cortex_core::ids::NodeId> = Vec::new();
    for day in 0..days {
        if !rng.chance(28) {
            continue;
        }
        let at = EPOCH + day * DAY + 11 * 3_600 + rng.below(3_600) as i64;
        let (kind, claim) = claims[rng.below(claims.len())];
        let subject = subjects[rng.below(subjects.len())];
        let scope = match rng.below(10) {
            0 => MemoryScope::Global,
            1 | 2 => MemoryScope::Project(1),
            _ => MemoryScope::Agent("claude".into()),
        };
        let node = store.node_by_id(subject.id)?;
        let memory = NewMemory {
            subject: node.map(|n| (n.id, n.path.display().to_string())),
            confidence: 0.4 + (rng.below(50) as f64) / 100.0,
            session_id: Some(format!("agent-d{day}")),
            ..NewMemory::new(scope, kind, format!("{claim} (day {day})"))
        };
        let remembered = store.remember(&memory, &agent, at)?;
        if !remembered.deduplicated {
            written.push(remembered.node_id);
        }

        // A pack, and sometimes a verdict on it. The pack is what makes `retrieved_count` mean
        // anything, and the silence on most of them is the point.
        if written.len() >= 2 {
            let shown: Vec<cortex_core::ids::NodeId> =
                written.iter().rev().take(3).copied().collect();
            let pack = format!("sim-d{day}");
            let items: Vec<PackItemRecord> = shown
                .iter()
                .enumerate()
                .map(|(rank, node)| PackItemRecord {
                    node_id: *node,
                    rank,
                    arm: PackArm::Lexical,
                    score: 1.0 / (rank as f64 + 1.0),
                })
                .collect();
            store.record_pack(&pack, None, "a task", 2048, 400, &items, at)?;
            store.note_retrieved(&shown)?;

            let verdict = rng.below(10);
            let subject_of_report = shown[rng.below(shown.len())];
            if verdict < 5 {
                let reporter = if verdict == 0 { Reporter::Human } else { Reporter::Agent };
                let actor = if verdict == 0 { Actor::Human } else { agent.clone() };
                store.report_outcome(&pack, &[subject_of_report], &[], reporter, &actor, at + 60)?;
            } else if verdict < 7 {
                store.report_outcome(&pack, &[], &[subject_of_report], Reporter::Agent, &agent, at + 60)?;
            }
            // The remaining three tenths are packs nobody says anything about, which is what
            // the unreported decay is for.
        }
    }
    Ok(written.len())
}

fn simulate(store: &Store, rng: &mut Rng, files: &[File], days: i64) -> Result<i64> {
    let human: Vec<usize> = files
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.generated)
        .map(|(i, _)| i)
        .collect();
    let mut last = EPOCH;

    for day in 0..days {
        let day_start = EPOCH + day * DAY + 8 * 3_600;
        // Weekends are quieter. A history with no rhythm produces a graph with no rhythm.
        let weekend = (day % 7) >= 5;
        let sessions = if weekend { rng.below(2) } else { 2 + rng.below(3) };

        for session in 0..sessions {
            let project = pick_project(rng, day);
            let session_id = SessionId(format!("d{day}-s{session}"));
            let start = day_start + (session as i64) * 3 * 3_600 + rng.below(1_800) as i64;

            // A person works in a cluster: a handful of files that belong together, opened in
            // the same quarter of an hour. This is the signal co-access is supposed to find.
            let cluster = rng.below(CLUSTERS);
            let candidates: Vec<usize> = human
                .iter()
                .copied()
                .filter(|&i| files[i].project == project && files[i].cluster == cluster)
                .collect();
            if candidates.is_empty() {
                continue;
            }
            let touched = (candidates.len().min(6)).max(2) + rng.below(2);
            for step in 0..touched {
                let file = candidates[(rng.below(candidates.len()) + step) % candidates.len()];
                let ts = start + (step as i64) * (60 + rng.below(240) as i64);
                let action = if rng.chance(70) { Action::Modify } else { Action::Open };
                store.append_episode(
                    ts,
                    &Actor::Human,
                    action,
                    Some(files[file].id),
                    Some(&session_id),
                    &serde_json::json!({}),
                )?;
                last = last.max(ts);
            }

            // The build afterwards: hundreds of generated files in seconds, one session id.
            if rng.chance(60) {
                let build = SessionId(format!("d{day}-s{session}-build"));
                let generated: Vec<usize> = files
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| f.generated && f.project == project)
                    .map(|(i, _)| i)
                    .collect();
                let build_start = start + 40 * 60;
                for (k, &file) in generated.iter().enumerate().take(400) {
                    let ts = build_start + (k as i64) / 20;
                    store.append_episode(
                        ts,
                        &Actor::System,
                        Action::Modify,
                        Some(files[file].id),
                        Some(&build),
                        &serde_json::json!({}),
                    )?;
                    last = last.max(ts);
                }
            }
        }
    }

    // One pin, so the demo shows the one rule that overrides every inference: a document in a
    // project abandoned three weeks in, which every other signal would have forgotten.
    if let Some(file) = files.iter().find(|f| {
        !f.generated && PROJECTS[f.project].abandoned_after.is_some()
    }) {
        store.set_pinned(file.id, true)?;
    }
    Ok(last)
}

/// Which project gets this session. Attention follows the weights, and an abandoned project
/// stops receiving any.
fn pick_project(rng: &mut Rng, day: i64) -> usize {
    let live: Vec<(usize, u32)> = PROJECTS
        .iter()
        .enumerate()
        .filter(|(_, p)| p.abandoned_after.is_none_or(|end| day < end))
        .map(|(i, p)| (i, p.weight))
        .collect();
    let total: u32 = live.iter().map(|(_, w)| w).sum();
    if total == 0 {
        return 0;
    }
    let mut pick = (rng.next() % total as u64) as u32;
    for (index, weight) in live {
        if pick < weight {
            return index;
        }
        pick -= weight;
    }
    0
}
