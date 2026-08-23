# Build Prompt — A Memory That Forgets

**A complete, self-contained specification for building a filesystem-native second brain for
humans and agents.** Hand this to a competent implementer — human or agent — who has not read
anything else. The companion paper `FORGETTING-IS-THE-FEATURE.md` is the argument for why the
system is shaped this way; this document is what you build. Where the two disagree, this one wins,
because this one has acceptance criteria.

---

## 0. How to use this document

Read §§1–3 before writing a line. They are short and everything else depends on them.

Then work milestone by milestone in the order given. **Each milestone has a Definition of Done
whose every item is executable** — a command that passes or fails, not a judgement. A milestone is
not finished because it feels finished. Do not begin the next one with an unmet criterion behind
you; write the exception down instead, in the progress file, with the reason.

Three files are the project's memory and you must maintain them in the same commit as the code:

| file | what goes in it |
|---|---|
| `docs/PROGRESS.md` | **the living state.** What is done, what is next, what the last session left rough and why. Read first, written last, every session. |
| `docs/DECISIONS.md` | append-only. One entry per decision: the decision, the reason, and **what would make you revisit it**. A decision without a revisit trigger is a dogma and will hurt later. |
| `docs/GLOSSARY.md` | §3 of this document, extended. Add a term here before using it in code. Inconsistent vocabulary is the most common cause of architectural drift in systems built over many sessions. |

**Write down your dead ends.** A dead end you do not record is one the next session pays to
rediscover, and that is the most expensive category of waste in a long-running project. §12 gives
the format.

---

## 1. What you are building

A **headless memory daemon** that indexes a filesystem, watches it change, and answers three kinds
of question in under a hundred milliseconds:

- *Where is the thing I am thinking of?* — hybrid search over paths and content.
- *What is this related to?* — a graph built from co-occurrence in time, not only from similarity.
- *What am I working on right now?* — a working set derived from recent events.

And one kind of question that takes longer and is the reason the system exists:

- *What can be forgotten?*

Every interface — a command line, a file manager, an agent surface, a visualisation — is a client
of that daemon over a local socket. None of them holds state the others cannot see. None of them
computes an opinion the daemon does not have.

**The property that distinguishes this from every other memory system: it forgets, and forgetting
is safe because nothing authoritative lives inside it.** The filesystem is the truth. The index is
a cache with an opinion.

### What it is not

- Not a note-taking application. It never writes a file next to the user's files, and it never
  imposes a folder structure.
- Not a chat memory. The unit is a file and an event, not a conversational turn.
- Not a cloud service. Zero outbound network connections on a fresh install.
- Not an LLM wrapper. No model call is on the path of a search.

---

## 2. Hard rules

Non-negotiable. A change that violates one of these is rejected regardless of how well it works.
Each must be enforced by a test, a schema constraint or a CI gate — not by intention.

**R1 — The deny-list applies before any content is read.** The classifier checks it *before* the
extractor is allowed to open a file. Compiled in, extendable by configuration, never shrinkable
below the built-in minimum:

```
paths:  .ssh/  .gnupg/  .aws/  .kube/  .docker/config.json  .mozilla/
        .local/share/keyrings/  .password-store/  browser profile directories
names:  .env  .env.*  *.pem  *.key  *.p12  *.pfx  *.kdbx  *.jks
        id_rsa*  id_ed25519*  id_ecdsa*  *credentials*  *secret*  *token*
        .netrc  .pgpass  .my.cnf  known_hosts  authorized_keys
```

A second, separate list is indexed as metadata only and never extracted, because reading it is
pointless rather than dangerous: `node_modules/ .git/objects/ target/ build/ dist/ .next/ .venv/
__pycache__/ .cache/ *.o *.a *.so *.pyc *.class`.

**R2 — Ignore files are honoured.** `.gitignore` semantics inside git repositories, plus a
`.brainignore` with the same syntax at any level. A `.brainignore` containing `*` means the
directory is metadata-only, not invisible.

**R3 — Content extraction is opt-in outside configured roots.** Metadata indexing may cover a whole
home directory. Content extraction happens only inside explicitly configured roots. Everything else
is a path and a stat.

**R4 — The index lives in the state directory, mode 0700, files 0600.** Never in the user's
documents, never in `/tmp`.

**R5 — No network by default.** Zero outbound connections on a fresh install: no telemetry, no
update check, no model download without an explicit user action. **Test this properly** — run the
daemon in a network namespace with no route and assert full local function.

**R6 — Agent writes are gated.** Agents get read access to files and write access to the brain.
That asymmetry is the safety model. Filesystem mutations go through a policy engine: allow-list
scoping, a mandatory dry run that returns a plan, an episode per mutation attributed to its actor,
and an undo journal that stores the inverse **before** applying the forward operation.

**R7 — A kill switch that is always reachable.** It pauses indexing and disables the agent surface,
from the command line and from any GUI, and its state survives a restart.

**R8 — Every agent access is audited**, allowed or refused, with timestamp, identity, tool,
arguments and result size. Append-only, separate from the episode log. The refusals are the
interesting half.

**R9 — Repository hygiene, if the repository is public.** Forbidden in code, tests, fixtures,
example configuration, documentation *and commit history*: hostnames, IP addresses, usernames, real
names, email addresses, absolute paths from a private machine, and any secret — including
fake-looking ones in tests. Enforce with a pre-commit hook **and** the same check as a CI gate.
Test fixtures use temporary directories created at runtime, never checked-in absolute paths.

**R10 — Reconstructability.** The episode log is the only table never mutated. Everything else must
be derivable from the filesystem plus that log. Any feature that stores non-reconstructible state
in the index is rejected. `rebuild-index` is a tested command, not a hope.

---

## 3. Vocabulary

Binding. Use these words and no synonyms.

| term | meaning |
|---|---|
| **Cortex** | the daemon. The core of the system. |
| **Node** | a memory entry: a file, a directory, a chunk, a concept, or an annotation. |
| **Episode** | a timestamped event — created, modified, opened, moved, deleted, searched, annotated. Immutable, append-only. |
| **Working Set** | what the human is doing right now. Derived from recent episodes. |
| **STM** | short-term memory: hot, fully indexed. |
| **LTM** | long-term memory: consolidated, still locally searchable. |
| **Cold** | metadata only. No embeddings. Read on explicit deep search. |
| **Salience** | how much a node is worth remembering. Drives ranking, tiering and forgetting. |
| **Consolidation** | the batch pass that replays episodes, forms associations, rescores and retiers. |
| **Zone** | a region of the filesystem treated as a place — usually a project. Carries a stable colour. |
| **Tier** | Working, STM, LTM, Cold. |
| **Agent Surface** | the machine interface: local JSON-RPC plus a protocol bridge. |
| **Agent View** | the visualisation of the brain state. Not the same thing as the Agent Surface. |

**Deliberately avoided:** "database" for the index — it is a *cache*, and calling it a database
invites treating it as authoritative. "AI search" — this is hybrid retrieval; models improve
ranking, they do not produce the result set.

---

## 4. Architecture and layout

```
crates/ (or packages/, or modules/ — one unit per row)
  core        domain types, ids, errors, configuration schema
  store       schema, migrations, full-text index, vector index, episode log
  ingest      watcher, classifier, extractor, chunker
  query       search modes, rank fusion, reranking, explainability
  memory      salience, decay, tiering, association — pure, no I/O
  agent       policy engine, kill switch, audit log, filesystem tools, undo journal
  rpc         the local socket protocol
  daemon      the binary
  cli         the command-line client
  mcp         the agent-protocol bridge — a bridge, no policy in it
ui/           optional graphical client
ui-agent/     the Agent View
bench/        the benchmark suite and the usage simulator
docs/         PROGRESS, DECISIONS, GLOSSARY, and this document
```

**`memory` must not depend on `store`.** All of the scoring, decay, tiering and association
mathematics are pure functions over values passed in. This is not tidiness: a milestone below
requires simulating sixty days of use, and a simulation that wrote a real index per simulated day
would be too slow to run, so nobody would run it.

**Language and stack.** The reference implementation is Rust with SQLite, JSON-RPC over a Unix
socket, and a TypeScript visualisation. Substitute freely, but preserve four properties: a single
binary with no runtime to install, an embedded database with full-text search, a local socket
rather than an HTTP port, and a pure, dependency-free core for the mathematics.

---

## 5. The data model

Write this schema first. Every column below exists for a reason and the reasons are given.

```sql
CREATE TABLE zones (
    id            INTEGER PRIMARY KEY,
    root_path     TEXT    NOT NULL UNIQUE,
    name          TEXT    NOT NULL,
    -- Derived from the zone's identity so it is stable across sessions. Saturation and
    -- lightness belong to the theme; only the hue is stored.
    color_hue     INTEGER NOT NULL CHECK (color_hue BETWEEN 0 AND 359),
    icon          TEXT,
    user_override INTEGER NOT NULL DEFAULT 0 CHECK (user_override IN (0, 1))
);

CREATE TABLE nodes (
    id           INTEGER PRIMARY KEY,
    kind         TEXT    NOT NULL CHECK (kind IN ('file','dir','chunk','concept','annotation')),
    path         TEXT,                       -- NULL for concepts and annotations
    parent_id    INTEGER REFERENCES nodes(id) ON DELETE CASCADE,
    mime         TEXT,
    size         INTEGER NOT NULL DEFAULT 0,
    mtime        INTEGER NOT NULL DEFAULT 0,
    ctime        INTEGER NOT NULL DEFAULT 0,
    -- Identity as the kernel sees it. A move is detected by (dev, inode) surviving a path
    -- change. Without this, `mv` of a large directory looks like a mass delete plus a mass
    -- create, which destroys history and forces a pointless re-index.
    inode        INTEGER NOT NULL DEFAULT 0,
    dev          INTEGER NOT NULL DEFAULT 0,
    content_hash TEXT,
    tier         TEXT    NOT NULL DEFAULT 'stm'
                 CHECK (tier IN ('working','stm','ltm','cold')),
    salience     REAL    NOT NULL DEFAULT 0.0,
    pinned       INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0,1)),
    zone_id      INTEGER REFERENCES zones(id) ON DELETE SET NULL,
    first_seen   INTEGER NOT NULL,
    last_seen    INTEGER NOT NULL,
    extract_state TEXT   NOT NULL DEFAULT 'pending'
                 CHECK (extract_state IN ('pending','done','skipped','failed'))
);

CREATE TABLE edges (
    src        INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    dst        INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    type       TEXT    NOT NULL CHECK (type IN
                 ('contains','co_access','imports','similar_to','derived_from','annotates')),
    weight     REAL    NOT NULL DEFAULT 1.0,
    created_at INTEGER NOT NULL,
    -- How we came to believe this edge. `explicit` edges are human or agent assertions and
    -- must never be pruned by a process that only trusts its own inferences.
    source     TEXT    NOT NULL CHECK (source IN ('observed','inferred','explicit')),
    PRIMARY KEY (src, dst, type)
);

CREATE TABLE episodes (
    id            INTEGER PRIMARY KEY,
    ts            INTEGER NOT NULL,
    actor         TEXT    NOT NULL,          -- 'human' | 'system' | 'agent:<name>'
    action        TEXT    NOT NULL CHECK (action IN
                    ('create','modify','open','move','delete','search','annotate')),
    -- DELIBERATELY NOT A FOREIGN KEY. An episode about a deleted file must outlive the node:
    -- this log is the ground truth from which nodes are rebuilt. See §7.3, trap 1 — this
    -- decision is correct and it will break your first consolidation if you forget it.
    node_id       INTEGER,
    session_id    TEXT,
    metadata_json TEXT    NOT NULL DEFAULT '{}'
);

CREATE TABLE salience_log (
    node_id         INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    computed_at     INTEGER NOT NULL,
    score           REAL    NOT NULL,
    components_json TEXT    NOT NULL,
    PRIMARY KEY (node_id, computed_at)
);

-- Plus: chunks, a contentless full-text index over them, embeddings keyed by content hash
-- (not by node, so a duplicated file is embedded once), an undo journal, an audit log,
-- and a small key/value table for daemon-wide state.
```

Indexes at minimum on: `nodes(parent_id)`, `nodes(dev, inode)`, `nodes(kind, tier)`,
`nodes(zone_id)`, `nodes(salience)`, `episodes(ts)`, `episodes(node_id)`, `episodes(session_id)`,
`edges(dst)`, `edges(type)`.

---

## 6. The mathematics

Implement this in the pure `memory` module. Every function here is total, deterministic, and takes
its clock as an argument. **Never call a wall clock inside these functions** — a test that cannot
choose "now" cannot test decay.

### 6.1 Salience

```
salience = w₁·access + w₂·recency + w₃·edits + w₄·centrality
         + w₅·pin + w₆·project − w₇·noise
```

**Every component is squashed to `[0,1]` before its weight is applied.** A weight is therefore the
maximum contribution of its signal, and the weight vector is a budget a reader can interpret.

| component | unit form | default weight |
|---|---|---|
| access | `min(1, ln(1+c_access) / ln(1+50))` | 1.0 |
| recency | `exp(−λ_r·(now − last_access))`, half-life 14 days; `0` if never | 1.5 |
| edits | `1 − exp(−c_edit / 3)` | 1.0 |
| centrality | `min(1, degree / degree_p95)` | 0.8 |
| pin | `1` if pinned | **10.0** |
| project | `1` if inside a zone touched in the last 14 days | 0.5 |
| noise | strongest matching pattern in `[0,1]`, **subtracted** | 2.0 |

Notes that are not optional:

- **`degree_p95`, not `max`.** A maximum lets one hub flatten every other node to nearly zero. A
  mean is dragged around by the long tail of degree-one nodes. Never zero, so the caller can divide.
- **Noise takes the strongest match rather than accumulating**, so a vendored directory inside a
  cache scores 1.0 and not 2.0.
- **The pin weight exceeds the sum of every other weight** (10.0 against a ceiling of 4.8). See
  §6.4.
- Store the component breakdown, not just the total.

### 6.2 Decay — read this twice

```
salience(t) = base · e^(−λ·Δt) + Σ(reinforcements)
```

This is not a second mechanism alongside the sum. **It is how the counts in the sum are computed.**
`c_access` and `c_edit` are *effective* counts: each episode is a reinforcement of size 1, and the
running total decays between episodes. Folding the ascending timestamps `t₁ … tₙ` of a node:

```
c₀ = 0
cᵢ = cᵢ₋₁ · e^(−λ(tᵢ − tᵢ₋₁)) + 1        for i = 1 … n
c  = cₙ · e^(−λ(now − tₙ))
```

which equals `Σᵢ e^(−λ(now − tᵢ))`. Default half-life: 30 days. Ignore timestamps in the future —
a clock skew must not be able to invent salience.

**If you implement `c_access` as a plain count, nothing in your system will ever be forgotten.**
`ln(1+n)` never decreases, so any file ever touched holds a permanent floor of salience above the
demotion threshold. The failure is silent: rankings still look plausible. It is only visible in a
long-horizon simulation that asks whether anything ever leaves the hot tier. Write that simulation
before you write the scorer.

### 6.3 Tiering

Thresholds on the bounded scale, against the default weights above:

```
working: salience ≥ 1.60  AND  last_access within 4 hours
stm:     salience ≥ 0.55
ltm:     salience ≥ 0.15
cold:    below that
```

Two asymmetries, both deliberate:

- **Promotion is immediate. Demotion is one tier per pass.** A memory that took a week to notice
  what you are doing is useless; a memory that drops a document from Working to Cold after one
  quiet fortnight teaches its owner not to trust it. The errors are not symmetric, so the policy
  is not either.
- **A node younger than a grace window is never judged.** One week. Demoting a file indexed an hour
  ago is forgetting something before ever having had it.

Thresholds, not quantiles: a quantile-based policy always demotes something, even on an index where
everything is genuinely in use.

**Consequence you must handle in the interface:** a fresh index has every node in one tier and a
tier legend reading `working 0 / ltm 0 / cold 0`, which looks like a broken tiering. Say so in
words wherever the distribution is shown.

### 6.4 Pins are exemptions

Implement a pin **twice**: as the dominant weight above, *and* as a branch that returns before the
demotion logic is reached. The weight makes a pinned node rank first; the branch makes it immune to
a future change in the weights.

Required test: a node pinned and untouched for ten years outranks a node with every other signal at
its maximum.

### 6.5 Association from co-occurrence

```
group episodes into bursts:
    same session_id, or within GAP = 300 s of the previous touch in that burst
    each open session accumulates its own burst, so interleaved sessions do not merge
discard any burst touching more than MAX_BURST = 12 distinct nodes
for each surviving burst: every pair of distinct nodes co-occurs once
emit an edge for a pair seen in ≥ MIN_BURSTS = 2 distinct bursts
weight = 1 − exp(−bursts / 3)
sort deterministically before truncating to any cap
```

**The discard rule is the important one.** A human editing a feature touches five or ten files in a
quarter of an hour; a build touches four thousand in twenty seconds. If builds formed edges the
graph would be a single hairball in which everything relates to everything. Discard the burst whole
— truncating keeps an arbitrary slice of a meaningless event and makes it look meaningful.

Expect to discard a lot: on real developer machines roughly two thirds of bursts are machine
activity.

### 6.6 The consolidation pass

Five steps, in this order, because each depends on the last:

1. **Replay** the episode log.
2. **Associate** — derive and upsert `co_access` edges.
3. **Score** — because centrality is a component, and a pass that scored before writing edges
   would rank against the previous pass's graph, permanently one step behind itself.
4. **Tier.**
5. **Report.**

**Replay the whole log every pass**, capped at a fixed number of most recent episodes. Deriving
only from new episodes makes the graph depend on how often consolidation happened to run — an
index consolidated hourly and one consolidated weekly would disagree, from identical histories.
Replaying everything and upserting makes the edge set a pure function of the log.

**The report is a deliverable, not a log line.** It must state: episodes replayed, bursts found,
**bursts discarded**, edges written, nodes scored, what moved between which tiers, how many pinned
nodes were held, the most salient nodes with a one-phrase reason each, and — always — a section
naming what this pass deliberately did not do. That last section is three lines of output and it
is the difference between a project that knows where it is and one that believes its own summary.

---

## 7. Three traps

These are defects found in a real implementation of this specification, after the code had passed
its unit tests. Write the regression test **before** the code that would contain the bug.

### 7.1 An immutable log outlives its foreign keys

`episodes.node_id` is deliberately not a foreign key. `edges` has one. Deriving an association
between two nodes that have since been deleted therefore fails the **entire transaction** with a
foreign-key violation.

This ran cleanly through hundreds of tests and a thirty-one-thousand-episode simulation before
failing on the first real index — because nothing in a synthetic history is ever deleted.

**Fix:** join the episode query to `nodes`. **Test:** create two files, give them a shared history,
delete one, consolidate, assert the pass succeeds and no edge references the dead node.
**General lesson:** a generated history that only ever adds cannot exercise the paths that make an
append-only log difficult. Put deletions in your simulator.

### 7.2 A monotone counter is not a memory

§6.2. Any evidence counter that never decreases will eventually dominate any decaying signal it is
summed with. **Test:** simulate ninety days with one touch on day zero and assert the node reaches
Cold, and that a pinned node beside it does not.

### 7.3 A connection pool deadlocks against itself

If your store hands out pooled connections, a function that holds one while calling a helper that
asks for another will wait for itself. On a pool of four this is latent and load-dependent. On an
in-memory test store, which has exactly one connection, it is an immediate and total hang — no
error, no CPU load, no output; it looks exactly like a slow compile.

**Rule:** everything that needs its own connection happens *before* the long-lived one is taken.
**Design your test store with fewer connections than production**, so this class of bug fails
immediately instead of intermittently.

---

## 8. Milestones

Each Definition-of-Done item is a command that passes or fails.

### M0 — Skeleton
Workspace, schema, migrations, local socket, a `ping`, a `status`, a config file.

**Done when:** ① the daemon starts, creates its index at the correct path with mode 0700, and
answers `ping`. ② migrations run forward from empty and are idempotent. ③ a second daemon instance
refuses to start rather than corrupting the index. ④ `status` reports schema version and index
location.

### M1 — Perception
Watcher, classifier with the R1 deny-list, metadata indexing, the episode log, path search.

**Done when:** ① indexing 100 000 files completes and every path is present, verified against
`find`. ② a file created, modified, moved and deleted while the daemon runs produces exactly four
episodes with the right actions, and a move is recognised by `(dev, inode)` rather than as a
delete-plus-create. ③ no file matching the deny-list has ever been opened — assert against a
syscall trace, not against the absence of extracted content. ④ path search returns in **under
50 ms** on the 100 000-file index. ⑤ deleting the index and rebuilding reproduces a byte-identical
canonical dump.

### M2 — Meaning
Extraction, chunking, embeddings, hybrid retrieval with rank fusion, and an `explain` that shows
how a result was produced.

**Done when:** ① content search over the corpus returns in **under 80 ms** at the 95th percentile.
② `explain` prints, for any result, its lexical rank, its vector rank, the fused score and any
boost, and the parts account for the score used. ③ a query for an exact identifier that appears in
one file returns that file first — the case dense retrieval alone gets wrong. ④ extraction never
runs outside configured roots (R3). ⑤ removing the embedding model degrades to lexical and path
search with no error.

### M3 — Agent Surface
The protocol bridge, the policy engine, the undo journal, the audit log, the kill switch.

**Done when:** ① every mutating tool refuses without a prior dry run. ② every mutation is
reversible through the undo journal, proven by a property test: apply a random sequence, undo it
all, assert the filesystem matches its initial state. ③ a write outside the allow-list is refused
**and audited as refused**. ④ the kill switch disables the surface within one request and survives
a restart. ⑤ a red-team test suite of at least twenty attempts to escape the allow-list — symlinks,
`..`, absolute paths, race conditions between the check and the write — all refused.

### M4 — Search Overlay
The first graphical client: a launcher-style overlay with a preview pane.

**Done when:** ① the window appears in **under 300 ms** from the hotkey on an idle machine,
measured ten times. ② typing to first results is under 100 ms. ③ the overlay holds no state the
daemon does not have; killing and restarting it mid-session loses nothing but focus.

### M5 — Memory
**The milestone this whole document exists for.** Salience, decay, tiering, association,
consolidation, and the report.

**Done when:** ① sixty simulated days against a synthetic corpus leave search latency within 10 %
of day one, and index growth sub-linear in episode count. ② a node touched once on day zero is in
Cold by day ninety; a pinned node beside it is still in STM. **Assert both.** ③ consolidation is
idempotent: two passes over the same log produce identical edges and, once settled, identical
tiers. ④ the report is generated and a human can tell from it what the system did without reading
logs — including what it discarded and what it did not do. ⑤ the stored component breakdown sums to
the score actually used, for every node, asserted. ⑥ a full rebuild after aggressive demotion
restores every node from episodes plus the filesystem (R10). ⑦ the three traps in §7 each have a
named regression test.

### M6 — Human View
The full file manager on top of the daemon: browsing, selection, copy, move, rename, trash, undo.

**Done when:** ① every listing is the daemon's — the frontend does not sort, filter or search. ②
scrolling holds 60 fps on a directory of 10 000 entries. ③ every destructive operation is undoable
and produces an attributed episode. ④ entries covered by the deny-list are counted in the status
line and never named (R1).

### M7 — Federation
An outbox protocol so that a second machine can hold a shared memory. Keep the concrete peer out of
the public repository.

**Done when:** ① a peer that is unreachable for 72 hours receives everything on reconnection, in
order, without exceeding a configured rate. ② replaying the entire outbox twice changes nothing on
the peer. ③ a grep of the public repository for endpoint patterns, private paths and credentials
returns nothing, in CI. ④ with the peer absent, every test still passes and the daemon starts
normally.

### M8 — Agent View
The visualisation of the brain state. Not decoration: this is the instrument through which the
scoring function becomes falsifiable by looking.

**Requirements:**
- One channel per variable: hue is the zone, brightness the tier, size the salience (square root —
  area is what the eye compares), a halo marks the Working Set.
- **Two deterministic layouts**: the containment tree (what the index *is*) and concentric shells
  by tier (what the index *believes*). Forgetting is visible only in the second, as a node moving
  outward, and it only reads as movement because the node was somewhere stable before.
- **No force-directed layout.** It settles differently on every reload, so two screenshots of the
  same brain cannot be compared and a node cannot be found twice in the same place. For an
  instrument, reproducibility beats organic appearance.
- One instanced mesh for all nodes, one line buffer for all edges. Per-node meshes are thousands of
  draw calls and that is what makes such views unusable.
- A consolidation playback that replays the last pass step by step **using the daemon's own report
  numbers**. Nothing in the animation may be computed in the frontend: if the animation and the
  report could disagree, the animation is a fiction.
- The view states its own basis in words, including the awkward cases — a young index, a
  never-consolidated index, a sampled snapshot.

**Done when:** ① a person watching the consolidation playback can describe what the system did.
② 5 000 nodes stay interactive on real hardware — **and the frame rate is recorded, not assumed.**
③ the view is read-only with respect to the filesystem. ④ the interface never compares two
different quantities in one statistic; count derived containment edges apart from stored edges.

### M9 — Release
Packaging, onboarding, and a second user.

**Done when:** ① a second person installs it on a clean machine, indexes their home directory and
performs a successful search, with no help and no access to any private repository, in under ten
minutes — timed. ② running with no peer is a documented, tested, first-class configuration.
③ the entire public history contains no secret and no private identifier. ④ every benchmark gate
is green on the release commit.

---

## 9. Latency budgets

These are the specification, not aspirations. Write the benchmark before the implementation, and
run it in CI on a fixed corpus generated from a seed — **commit the generator, never the corpus.**

| operation | budget | measured against |
|---|---|---|
| path search | 50 ms | 100 000 files |
| content search, p95 | 80 ms | same |
| overlay appears | 300 ms | idle machine, ten runs |
| keystroke to results | 100 ms | same |
| consolidation, dry run | 500 ms | 100 000 nodes |
| consolidation, full pass | 3 s | same |
| snapshot for the view | 2 s | 6 000 nodes |

For reference, the implementation these numbers come from measured 109 ms and 1 516 ms for the two
consolidation rows on 108 139 nodes with 9 534 episodes.

**Measure on an idle machine.** A latency number taken while the machine was compiling is not a
measurement, and a project that records one spends the next three sessions optimising something
that was never slow.

---

## 10. The agent surface

Local socket, JSON-RPC. Reads may be answered concurrently; writes keep their order, because an
undo undoes the previous thing.

```
ping · status                                     liveness and shape
search · search.explain                           retrieval and why
fs.list · fs.stat · fs.read · fs.places           navigation
fs.write · fs.move · fs.trash · fs.mkdir ·        gated by policy (R6)
  fs.rename · fs.transfer · undo
brain.search · brain.recall · brain.neighbors     memory, reading
brain.timeline · brain.working_set · brain.diff
brain.explain
brain.annotate · brain.pin · brain.forget ·       memory, writing (not gated by the
  brain.link                                        write allow-list — see R6)
brain.consolidate · brain.report                  the memory pass and its result
brain.graph                                       the whole brain, for a visualisation
index.path · index.set_paused                     control
agent.status · agent.set_enabled                  the kill switch (R7)
audit.recent                                      (R8)
```

Three interface rules worth stating explicitly:

- **`brain.diff` exists so that an agent does not poll.** It returns everything that changed since a
  cursor, plus a new cursor.
- **`brain.graph` is the one call that returns a large object.** Document it as needing a result
  gate. In an agent loop, tool output is the dominant token cost, and a memory tool that dumps is
  the most expensive component in the loop.
- **`brain.working_set` must say what it is based on.** If it is derived from recent episodes rather
  than from a salience-weighted working memory, the answer says so in a field. A tool that claims a
  faculty the daemon does not have is worse than one that admits the gap.

---

## 11. Tests to write before the code

Not a complete suite — the ones that catch the failures this design is prone to.

**Pure mathematics, no database:**
- the components sum to the total, for random inputs
- no component can exceed its weight, at absurd inputs
- recency halves over exactly one half-life
- decay halves over exactly one half-life; a reinforcement is added *after* decay, not before
- fifty accesses two years ago score below three accesses this week
- an episode dated in the future is ignored
- a pin outranks every other signal at maximum
- demotion takes exactly three passes to go Working → Cold, never one
- a settled tier assignment is stable under repetition
- a burst of 500 nodes produces no edges at all
- one co-occurrence produces no edge; two produce one
- interleaved sessions do not contaminate each other
- the same episodes always produce the same edge list in the same order

**Against a real index:**
- consolidation twice over the same log writes no duplicate edge and moves nothing once settled
- a dry run writes nothing at all — assert on the database, not on the return value
- ninety days of disuse demote an untouched node to Cold and hold a pinned one in STM
- an episode about a deleted file does not break the pass (§7.1)
- the stored breakdown sums to the stored score
- the salience log does not grow without bound
- build output ranks below the source it was generated from, given identical histories

**Against the interface:**
- the snapshot never contains an edge to a node it did not send
- the snapshot carries the ancestors of everything it sends, so nothing floats
- a malformed snapshot is refused with the name of the offending field
- the layout is identical across runs
- a 20 000-deep directory chain does not overflow the stack — make the layout iterative

---

## 12. What to write while you work

**In `docs/PROGRESS.md`, every session.** Current milestone, status, blockers, and:

- **`## Dead ends`** — what you tried that did not work, and why. This is the highest-value section
  in the file.
- **`## Measured`** — a table. Every performance claim in the file has a number and a date beside
  it, or it does not belong in the file.
- **Open questions for the owner**, phrased so they can be answered without reading the code.

**In `docs/DECISIONS.md`, when a decision is made.** Append only; supersede rather than rewrite.
State the decision, the reason, and the revisit trigger.

**A rule that pays for itself:** when a measurement contradicts an explanation you were confident
in, write down the explanation you had and why it was wrong. The wrong model is what the next
session will otherwise re-derive.

---

## 13. When this document is wrong

It will be, somewhere.

- **A small obstacle** — a library does not work, an API differs: solve it, note it in
  `PROGRESS.md`, move on. Do not amend this document.
- **A decision-level change** — a budget is unreachable, a component must be replaced: append to
  `DECISIONS.md`, edit the affected section here, and say so in the commit message.
- **A scope-level change** — a milestone must be split, reordered or dropped: do not do it
  silently. Write the proposal in `PROGRESS.md` under open questions and get an answer.

---

## 14. First session checklist

1. Create the repository, the three documentation files, and the pre-commit secret scanner **before
   the first commit of code** — R9 covers commit history, and a scanner added later cannot fix what
   is already in it.
2. Write the schema from §5 and its migration runner.
3. Write the pure `memory` module from §6 and its tests from §11, with no database anywhere near
   it. This is a few hundred lines and it is the heart of the system.
4. Write the usage simulator: a synthetic workspace, a plausible history over it — **including
   deletions** (§7.1) — and one real consolidation pass per simulated day. Everything after this
   is measurable.
5. Only then start M0.

The order matters. The mathematics is where the system's identity lives, it is the cheapest part to
get right, and it is the part that every later component will assume is correct.
