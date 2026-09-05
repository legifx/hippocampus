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

### Added by M11

| term | meaning |
|---|---|
| **Memory** | a node with `kind = 'memory'`: what is known and is not in a file. Text, a scope, a kind, no path. |
| **Scope** | `session:<id>` · `agent:<name>` · `project:<zone_id>` · `global`. Who a memory belongs to. |
| **Context Pack** | what `brain.context` returns: budgeted, deduplicated, attributed, and addressable by `pack_id`. |
| **Usefulness** | the learned salience component `brain.outcome` moves. Not the same as **confidence**, which is the writer's own claim and is never evidence. |

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
    kind         TEXT    NOT NULL CHECK (kind IN
                 ('file','dir','chunk','concept','annotation','memory')),
    path         TEXT,                       -- NULL for concepts, annotations and memories
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
    -- A concept's name, and its identity. Not `mime`: a MIME type is a MIME type, and a name
    -- kept in one goes on being wrong quietly. Unique among concepts.
    label        TEXT,
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
                 ('contains','co_access','imports','similar_to','derived_from','annotates',
                  -- M11: which memory replaced which, and which memories were pulled together
                  'supersedes','recalled_with')),
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
                    ('create','modify','open','move','delete','search','annotate',
                     -- M11. `remember` and `forget_memory` are assertions: the memory row is a
                     -- projection of them, so a rebuild replays both. A deletion that is not an
                     -- episode comes back on the next rebuild.
                     'remember','outcome','supersede','forget_memory')),
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

-- M11 — what is known and is not in a file. Derived from `remember` episodes, exactly the way
-- annotations are, which is what lets a rebuild restore it.
CREATE TABLE memories (
    node_id        INTEGER PRIMARY KEY REFERENCES nodes(id) ON DELETE CASCADE,
    -- 'session:<id>' | 'agent:<name>' | 'project:<zone_id>' | 'global'
    scope          TEXT    NOT NULL,
    kind           TEXT    NOT NULL CHECK (kind IN
                     ('fact','preference','procedure','decision','failure','observation')),
    -- Held here because there is no file to re-read it from. The one place this system stores
    -- text rather than a pointer to it, and the reason is that there is nothing to point at.
    text           TEXT    NOT NULL,
    subject_id     INTEGER REFERENCES nodes(id) ON DELETE SET NULL,
    -- The writer's own claim. Never treated as evidence; see `usefulness`, which is earned.
    confidence     REAL    NOT NULL DEFAULT 0.5,
    usefulness     REAL    NOT NULL DEFAULT 0.0,
    helped_count   INTEGER NOT NULL DEFAULT 0,
    noise_count    INTEGER NOT NULL DEFAULT 0,
    -- Counted, not netted: a memory that is useful in one context and noise in another needs
    -- splitting, and a net of zero hides that.
    retrieved_count INTEGER NOT NULL DEFAULT 0,
    human_confirmed INTEGER NOT NULL DEFAULT 0 CHECK (human_confirmed IN (0,1)),
    expires_at     INTEGER,                  -- only 'session:' scope has one
    superseded_by  INTEGER REFERENCES nodes(id) ON DELETE SET NULL,
    session_id     TEXT,
    episode_id     INTEGER NOT NULL,         -- the assertion this row is a projection of
    actor          TEXT    NOT NULL,
    ts             INTEGER NOT NULL
);

CREATE TABLE agent_sessions (
    id         TEXT PRIMARY KEY,
    name       TEXT    NOT NULL,
    task       TEXT,
    started_at INTEGER NOT NULL,
    ended_at   INTEGER,
    calls      INTEGER NOT NULL DEFAULT 0,
    refusals   INTEGER NOT NULL DEFAULT 0
);

-- What a context pack contained, so that an outcome reported minutes later can be attributed
-- to the items that were actually in it rather than to whatever ranks highly now.
CREATE TABLE context_packs (
    id            TEXT PRIMARY KEY,
    session_id    TEXT,
    task          TEXT    NOT NULL,
    budget_tokens INTEGER NOT NULL,
    used_tokens   INTEGER NOT NULL,
    created_at    INTEGER NOT NULL
);
CREATE TABLE context_pack_items (
    pack_id  TEXT    NOT NULL REFERENCES context_packs(id) ON DELETE CASCADE,
    node_id  INTEGER NOT NULL,
    rank     INTEGER NOT NULL,
    arm      TEXT    NOT NULL,
    score    REAL    NOT NULL,
    reported TEXT,                           -- NULL | 'helped' | 'noise'
    PRIMARY KEY (pack_id, node_id)
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

**Before the numbers: check them against each other.** This is the single cheapest bug-class
prevention in the whole specification, and it is here rather than in §11 because it belongs to
the thresholds themselves.

Compute the maximum an *ordinary* node can reach with one fresh touch and nothing else — no
edit, no edges, no active project:

```
one_touch_ceiling = w_access · ln(2)/ln(1 + ACCESS_REFERENCE)  +  w_recency
```

Every tier threshold must clear it with room. Assert it:

```
working_threshold <= one_touch_ceiling * 0.75
```

**Why this and not another unit test.** A running second-brain daemon set its demotion threshold
to exactly its recency weight — which is precisely the score an untouched note reaches — so every
unused note sat on the demotion edge by construction and its short memory ran at one note out of
eighty. Nothing miscalculated. No test of the formula could fail, because the formula was right.
A system whose numbers were checked against *cases* and never against *each other* looked
calibrated and was dead. The same coupling was latent in the reference implementation of this
document, four per cent from the same failure.



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

### 6.6 Abstraction: naming a cluster

Association answers questions about pairs. The step that makes memory *semantic* turns a
recurring cluster into one thing with a name.

```
communities  = label propagation over the co_access edges
               deterministic node order; ties to the smaller label; run to a fixed point
keep         = communities with MIN_MEMBERS (3) <= size <= MAX_MEMBERS (60)
name         = the terms the members' paths share and the rest of the index does not:
                 inside  = fraction of the cluster's files whose path contains the term
                 outside = fraction of every *other* clustered file that contains it
                 keep terms with inside >= 0.5, outside <= 0.4, outside < inside
                 score   = inside * ln((inside + eps) / (outside + eps)); take the top 3
concept      = a node of kind `concept`, identified by that name, with `derived_from`
               edges to its members and a salience equal to the mean of its top three
```

Five rules, each of which a first implementation got wrong:

- **Compare inside against outside, not against the global frequency.** With two clusters, a term
  covering one of them entirely is 50 % of all documents — so a global-share filter throws away
  the perfect label first.
- **The label is the identity.** That is what lets a cluster which gains a file update its concept
  rather than grow a rival beside it. Key the upsert on the name.
- **A cluster with no term the others lack is not named at all.** Do not append a disambiguator.
  A member count gives you `kettle (5)` three times; the smallest member id gives you
  `kettle #1230`, which is unique, stable and meaningless. A node that looks like knowledge and is
  not is worse than an absent node.
- **Exclude the tokens of the index's own root.** The longest common prefix of every indexed path
  is, on a personal machine, the user's home directory, and it will otherwise appear in every
  concept's name. Find it from `MIN(path)` and `MAX(path)` — two rows, not a scan.
- **Rewrite membership each pass, never merge it.** A file that left the cluster must leave the
  concept, or abstraction becomes another way of accumulating.

**Do not use a language model for the name**, however tempting. It makes this step
non-deterministic, unrunnable offline, untestable and unreproducible from a seed. Keep the ranked
terms on the cluster so a summariser can be added later as a pure function from cluster to string.

### 6.7 The consolidation pass

Six steps, in this order, because each depends on the last:

1. **Replay** the episode log.
2. **Associate** — derive and upsert `co_access` edges.
3. **Abstract** — §6.6. Before scoring, because a file belonging to a named concept is more
   connected than one that does not.
4. **Score** — because centrality is a component, and a pass that scored before writing edges
   would rank against the previous pass's graph, permanently one step behind itself.
5. **Tier.**
6. **Report.**

**A dry run must cluster from the edges the replay just derived, not from the edge table.** The
two agree after a real pass — co-access is a pure function of the log — but a dry run has written
nothing, so reading the table makes it propose concepts from the previous pass, or from none at
all on a fresh index. A dry run that cannot answer "what would this do" is not worth having.

**Replay the whole log every pass**, capped at a fixed number of most recent episodes. Deriving
only from new episodes makes the graph depend on how often consolidation happened to run — an
index consolidated hourly and one consolidated weekly would disagree, from identical histories.
Replaying everything and upserting makes the edge set a pure function of the log.

**The report is a deliverable, not a log line.** It must state: episodes replayed, bursts found,
**bursts discarded**, edges written, concepts formed and pruned, nodes scored, what moved between which tiers, how many pinned
nodes were held, the most salient nodes with a one-phrase reason each, and — always — a section
naming what this pass deliberately did not do. That last section is three lines of output and it
is the difference between a project that knows where it is and one that believes its own summary.

---

## 7. Six traps

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

### 7.3 An answer that will not fit is a hang, not an error

Whatever framing your transport uses will have a size limit. A response above it cannot be sent —
and if the code that sends it discards the write error, the caller waits for ever. Both processes
sit at zero per cent CPU, which produces no evidence at all and is the most expensive failure to
diagnose.

**Rule:** never `let _ =` a write on a request/response protocol. **Fix, at two layers:** a reply
that does not fit comes back as an error naming its size and telling the caller to ask for less;
and any method that can produce a large answer caps its own request at a size you have measured.
**Test:** hand the responder an oversized result and assert an error comes back on the wire, with
the request's id, small enough to send.

### 7.4 Two SQL shapes that are wrong without an error

**`json_extract` on a column that is not JSON** fails the statement with `malformed JSON` — for
every value that is not a JSON document, which is most values. A `WHERE` clause written to mean
"this column is empty" that reaches for `json_extract` will work in tests full of empty columns
and fail on the first real one.

**`NOT IN` over an empty list**, rendered as `id NOT IN (SELECT NULL)`, is NULL for every row in
SQL's three-valued logic and therefore true for none. A delete written that way removes nothing
**exactly when there is most to remove**, and a delete that removed nothing is indistinguishable
from one that had nothing to remove. Write the empty case as its own statement.

### 7.5 A rebuild that produced nothing replaces everything with nothing

Anything derived — an edge cache, a concept layer, an embedding table — is rebuilt from something
else, and a rebuild can fail by producing an empty result instead of an error. If the writer then
commits it, the whole derived layer is gone, silently, and the next read looks like data loss.

Observed in a running system twice in one day: 939 of ~4 900 semantic edges vanished because a
vector store that was not ready yet returned "no vector" for every note without raising, and the
empty edge list was committed. The second time had a worse cause: `OperationalError` inherits
from `DatabaseError`, so a handler that treated `DatabaseError` as corruption **deleted the
database file** whenever a write lock timed out.

**Three rules, all of them cheap:**

- **Stale is better than empty.** A replacement is committed only if it is credible — refuse when
  the new set is below a configured fraction of the old one, and log the refusal.
- **A lock is not damage.** Only what the store explicitly reports as corrupt counts as corrupt.
  Everything else — busy, locked, I/O, permissions — is propagated, not "recovered" from.
- **Set aside, never delete.** From a file moved to `*.corrupt-<timestamp>` something can still
  be salvaged. From a deleted one, nothing.

**Test:** hand the writer an empty result while a populated one exists, and assert the old one
survives and the refusal is reported.

### 7.6 A connection pool deadlocks against itself

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
logs — including what it discarded and what it did not do. ⑤ files worked on together become a
named concept; two passes do not produce two concepts; a cluster with nothing distinctive to say
is not named at all. ⑥ the stored component breakdown sums to
the score actually used, for every node, asserted. ⑦ a full rebuild after aggressive demotion
restores every node from episodes plus the filesystem (R10). ⑧ the six traps in §7 each have a
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

### M10 — The explorer as a memory browser

**Goal:** the window stops being a file manager that consults a memory and becomes a memory
browser that can show a directory. **Everything the memory computes should be navigable with what
the user already knows: they are in a place, they can go back, they can open things.**

Full argument, current state, the four steps and the acceptance criteria for each:
**[`A-DIRECTORY-IS-JUST-ONE-QUERY.md`](A-DIRECTORY-IS-JUST-ONE-QUERY.md)**.
A session prompt for an agent picking it up: **[`EXPLORER-KICKOFF-PROMPT.md`](EXPLORER-KICKOFF-PROMPT.md)**.

### Scope

- **One row stream, many origins.** `view.open(source)` replaces the several row-producing calls
  at the view layer. A source is a directory, a query, a concept, a zone, the working set, the
  neighbours of a node, a slice of time, or a tier. The row shape does not depend on which.
- **Ordering is the daemon's for every source**, salience beside name, size and modification time.
- **Every view states its own basis in words** — what this list is, and what it is not.
- **The memory columns on every row**: salience, tier, degree, last episode, zone, pinned, and
  which concepts it belongs to. Tier renders as brightness, the same encoding the graph view uses.
- **Concepts, the working set and the cold tier become places**, in the same navigation and the
  same history stack as directories.
- The context panel keeps what belongs to one selection — why this ranked here, what happened to
  it, annotate it — and gives up "related", which is a place now.

### Definition of Done

1. The frontend contains no branch on "am I browsing or searching", and no code path orders,
   filters or merges rows. Asserted by the architectural test that already guards this.
2. Every M4 performance gate still holds **with the memory columns present**, measured on an idle
   machine: 300 ms cold start, 80 ms P95 keystroke to first row, first row of a 10 000-entry
   directory in 100 ms, 60 fps scrolling, bounded DOM nodes.
3. A concept can be opened, its members operated on like any other rows, and its basis line says
   how it was named — including that the naming is lexical.
4. The cold tier is one place that explains itself, and a row can be pinned back out of it.
5. `brain.explain` for the selected row is one keystroke away and shows the component breakdown.
6. Navigating back out of every new kind of place works with the same key as a directory.
7. **The measurement that decides it:** ordering a directory by salience finds a file the owner
   knows they have faster than ordering by modification time — tried by hand, with a stopwatch,
   and the result written into `docs/03-PROGRESS.md` whichever way it goes.

**Do not start this while the file-manager basics are missing.** Archives, drag-and-drop out of
the window, network locations and correct permission display come first: a file manager that
cannot do the basics will not be used, however good the brain is.

### M11 — The Agent Brain

**Goal:** an agent working through the surface remembers what it learned, gets it back next
session inside a context budget it can afford, and is measurably better for it. The human can
read, correct and delete every part of that.

The whole milestone is one sentence: **a memory is a node.** What an agent knows that is not in a
file becomes a `kind = 'memory'` node with no path, its own text and a **scope** — and every
faculty the file memory already has applies to it unchanged: chunking, embedding, salience,
decay, tiering, co-access, consolidation, `explain`, the graph, the audit log. Building a second,
agent-flavoured store instead is the mistake this design exists to avoid; the reasoning is in
the decision log as D74.

### Scope

- **Scope, so a shared brain stays usable.** `session:<id>` · `agent:<name>` ·
  `project:<zone_id>` · `global`. Writes default to the narrowest, `global` needs an explicit
  argument, reads never cross into another agent's private scope. A shared pool without this
  fails in one specific way: one agent's wrong guess becomes every agent's premise, silently.
- **Memory kinds, because retrieval differs.** `fact · preference · procedure · decision ·
  failure · observation`. The kind is a retrieval prior, not a separate index — a `failure`
  should surface when the agent is about to repeat it, a `preference` should surface always.
- **`brain.context(task, budget_tokens)` — the primitive the milestone is for.** Not a result
  list: a token-budgeted, deduplicated, attributed block, containing pinned preferences
  unconditionally, the working set, memories fused and then diversified by MMR, and the files
  those memories point at as path plus byte range. Every item carries its basis. The pack has an
  id.
- **`brain.outcome(pack_id, helped[], noise[])` — the learning signal, and the only one.** It
  moves a `usefulness` component of salience. Its point is the property no recency-and-frequency
  model has: **a memory retrieved often and never reported useful decays faster than one never
  retrieved at all.** Retrieval without use is evidence against a memory, because the retrieval
  was chosen by the ranking being evaluated and not by anybody.
- **Supersession, so contradictions resolve instead of accumulating.** A new memory may supersede
  an old one; retrieval excludes superseded memories unless asked for history. The daemon
  *detects* likely supersessions by embedding similarity within scope and kind and reports the
  candidate ids — it does not decide, because deciding is a language model's job and there is no
  language model in this daemon.
- **A second vector collection.** Memories are thousands, file chunks are millions; one shared
  ANN index means a memory recall competes for twenty slots and loses. Same index type, second
  file. Not a different engine — see D75 for why the answer to "should this be Qdrant" is no.
- **Sessions.** `agent.session.begin/end`, so that "what did this task do" is answerable and
  `session:` scope has something to expire against.
- **The Agent Inspector**, one button and one binding in the explorer: live sessions and their
  calls, the memory list with provenance, the policy and kill-switch state, the undo journal, and
  the M8 view embedded and live. It renders; it does not decide. It writes in exactly three
  places, all deliberate: the kill switch, undo, and the human's right to delete a memory.

### Definition of Done

1. A memory written in one session is returned by `brain.context` in the next, identically
   through all three surfaces.
2. `rebuild` over an index of 1 000 memories across every scope and kind reproduces a
   byte-identical canonical dump — including supersession chains, usefulness counts and
   deletions. **The dump is the assertion.** A rebuild that exits zero having lost a column is
   the failure this criterion exists to catch.
3. The token budget is honoured for budgets of 512, 2 048 and 8 192 against 5 000 memories, and
   no item is truncated mid-structure. The count is a structural estimate that is biased upwards
   and reported as an estimate — the daemon cannot know which tokeniser the caller uses, and the
   only one this repository ships belongs to a retrieval model. It is bounded below by the word
   count, which a byte-pair encoding cannot go under, and above by the character count.
4. `brain.context` P95 **under 120 ms** at a 2 048-token budget over 5 000 memories and 100 000
   file chunks, warm — written as a benchmark before the implementation.
5. MMR at λ = 0.7 returns strictly fewer duplicate facts than λ = 1.0 at the same budget on a
   seeded near-duplicate store, without losing recall of the distinct ones. Both numbers recorded.
6. The learning signal moves the ranking in all three directions, including the counter-intuitive
   one: a memory retrieved fifty times with no outcome reported ranks below one never retrieved.
7. A memory in one agent's scope is never returned to another's, asserted per surface; an
   unrequested promotion to `global` is refused and audited.
8. With the surface off, every memory tool refuses, nothing is written, and the Inspector says so
   rather than showing an empty list.
9. Deleting a memory in the Inspector writes a `forget_memory` episode as the human, removes it
   from retrieval at once, and **survives a rebuild as a deletion**.
10. The Inspector renders at 400 px, costs under 1 % of one core while streaming, and closing it
    returns the window to its measured idle cost.
11. **The measurement that decides it:** a real task, twice. Session one discovers something
    non-obvious and remembers it; session two, with no transcript from session one, is faster or
    more correct because of the context pack — timed, and written into the progress file
    whichever way it goes.


---

---

### M12 — The Governed Brain

**Goal:** a memory that knows where each thing it holds came from, what it is allowed to be used
as, when it was true, and whether it is current — and a measurement, on this machine, of whether
the ranking machinery earns its place. The argument, with every source, is
[`WHAT-2026-CHANGED.md`](WHAT-2026-CHANGED.md). Read it first; this section is the part with
acceptance criteria.

The milestone is one sentence: **a memory carries its provenance, and provenance is
non-malleable.** Everything an agent, a file, a tool or a peer writes into the brain arrives with
an origin it cannot shed and an authority it cannot raise. The four falsifiers of the design paper
are run, not listed. Nothing here is a second store, a second engine or a model in the write path;
D74, D75 and R10 hold unchanged.

**Order inside the milestone is by evidence, not by size:** provenance and authority (§3 of the
research update) before validity and state (§4) before the evaluation (§2) before kind-conditioned
decay (§1) before adaptive fusion (§5). The evaluation gates the last two: neither ships if the
gold set says it does not pay.

### Scope

- **Origin, on every node.** `origin ∈ {human, agent, tool, file, peer}` — who caused the bytes to
  exist. A file's chunks are `file`; an `fs.read` result an agent remembers is `tool`; a note the
  agent worked out itself is `agent`; a pin, a note or an Inspector edit is `human`; anything from
  the sync inbox is `peer`. Recorded in the episode, projected to the node, returned in every
  `explain`, `why` and pack item.
- **Authority, on every memory.** `authority ∈ {instruction, fact, observation, untrusted}` — what
  the memory may be used *as*. Only `human` origin may create an `instruction`. `agent` origin
  creates at most `fact`. `tool`, `file` and `peer` create `observation` at most, and `untrusted`
  by default when the content contains anything that parses as an imperative aimed at an agent.
  **Non-malleable downward:** a memory whose `basis` cites other memories inherits the lowest
  authority among them; the agent paraphrasing a file does not launder the file. **Gated upward:**
  authority rises only by an action-bound human confirmation in the Inspector, or when the
  consolidation pass finds two memories with *independent* trusted origins that agree (two chunks
  of the same file are one origin). Every elevation is an episode with `actor`.
- **The policy engine reads authority.** A gated write (`fs.write`, `fs.move`, `fs.trash`,
  `fs.transfer`) that the agent attributes to a memory — `motivated_by: memory_id` in the request,
  and the MCP tool descriptions say to send it — is refused if that memory's authority is below
  `instruction`, audited either way. An agent that omits the field gets the policy as before; the
  field exists so that a poisoned memory cannot be the *stated* reason for a write.
- **Validity beside recording time.** `valid_from`, `valid_to` (nullable) on every memory beside
  `created_at`. The daemon never fills `valid_from` with `created_at` silently; if the agent does
  not say, the field is null and the pack says "recorded 2026-09-05, validity unknown."
- **Supersession takes a reason.** `brain.supersede(old, new, reason ∈ {changed, wrong})`. `changed`
  makes the old memory `historical` (answers "what was true before", retrievable with
  `include_history`); `wrong` makes it `retracted` (never returned without `include_retracted`, and
  then labelled). `brain.remember(..., supersedes)` without a reason is refused with the two options
  named. The daemon still only *detects* candidates; it never chooses.
- **State, on every memory, derived not stored.** `state ∈ {pending, current, historical,
  retracted, deleted}` — a pure function of the episode log (R10). `pending` is the admission
  state for `tool`, `file` and `peer` origins: retrievable when a call asks for it, absent from a
  pack by default, promoted to `current` by a human, by an `outcome` that names it as helped, or
  by the consolidation pass under the two-independent-origins rule. `deleted` never revives: a
  rebuild reproduces the deletion, and a later `remember` of identical text is a *new* memory with
  no inherited usefulness.
- **The pack labels and accounts.** Every item in `brain.context` carries `origin`, `authority`,
  `state`, and its validity when known. The pack gains an `omitted` block: how many items were left
  out for budget, how many were `pending`, how many `retracted`, so an agent can ask rather than
  guess. Pinned preferences remain unconditional — and are `human`/`instruction` by construction.
- **Half-life per kind.** `decay_half_life_days` becomes a table keyed by memory kind, with the
  present value as the default and as the value for files. Defaults, to be corrected by the
  falsifier and nothing else: `preference` 365, `procedure` 180, `decision` 120, `fact` 60,
  `failure` 90, `observation` 14. Written into the effective configuration, printed by
  `cortexd --check`, and shown per memory in `why`.
- **The audit log is hash-chained.** Each audit row carries the hash of the previous one; `cortex
  audit --verify` walks the chain. Tampering with the log becomes detectable, which is what the
  poisoning threat model needs and the rest of the design already assumes.
- **The evaluation, in MemDelta's shape.** A gold set built from the owner's own re-finding episodes
  (a query, the node they opened within a minute, the modes they used), frozen with a hash, and a
  harness `bench-quality` that runs *one variable at a time* with the embedding model fixed: salience
  boost on/off; kind half-lives on/off; adaptive fusion on/off; RRF versus each arm alone. Recall@10
  and MRR per mode, P95 latency per run, written to a JSON the review can diff. Retrieval and
  utilisation are reported separately: what was in the pack, and what `brain.outcome` said helped.
- **The forgetting suite.** Command-driven cases, scored by substring absence across every surface:
  delete, retract, "forget everything about ‹subject›", supersede-as-wrong, a rotated secret-shaped
  string, and non-revival after each of them through a rebuild and through a re-`remember` of the
  same text.
- **Adaptive fusion, behind a flag.** Per-query weighting of the RRF arms from the query's IDF
  profile, and a cascade that skips the vector arm when the lexical arms agree with a wide margin.
  Off by default until `bench-quality` and `bench-latency` both say it pays on this index.
- **Weights fitted from the log, offline.** A `cortex consolidate --fit-weights` step that fits the
  salience weight vector against observed re-access in the episode log (logistic, no model), writes
  the fitted vector *into the report*, and applies nothing until a human copies it into the config.
  Learned weights are advice here, not a mechanism — a weight vector nobody read is D76's problem
  again.

### Out of scope

- Any language model in admission, elevation or supersession. The daemon detects and labels; a
  human or an outcome decides.
- A trust *score*. Authority is an enum with a rule; a continuous trust number is exactly the
  malleable signal the research update says is unsound.
- Memory gravity audits (suspend the top concepts, replay logged queries) — possible once the
  gold set exists; noted as an option in the consolidation report, not built.
- Encrypted memories, multi-user authority, organisational invariants. One person, one machine,
  and peers that are the same person elsewhere.

### Definition of Done

1. **Origin is unforgeable from the socket.** A `brain.remember` over MCP or the socket cannot
   set `origin = human`; the Inspector and the CLI as the human can. Asserted per surface.
2. **Authority does not launder.** A memory whose `basis` cites a `file`-origin chunk or an
   `untrusted` memory is stored with authority ≤ `observation` whatever the request asked for,
   asserted for a direct write, for a paraphrase (the agent's text differs from the source) and
   for a chain of three.
3. **A poisoned file cannot write.** A seeded file containing an imperative aimed at an agent is
   indexed; an agent remembers it; the memory is `pending`/`untrusted`; it never appears in a pack
   without both labels; an `fs.write` with `motivated_by` pointing at it is refused and audited.
   The same three assertions for a seeded tool output and for a seeded peer batch.
4. **Elevation is an event.** Raising authority writes an episode with `actor = human` or
   `actor = consolidation` and the two independent origins it relied on; the Inspector shows it;
   a rebuild reproduces it.
5. **Reasons differ at read time.** After `supersede(a, b, changed)`, `a` is returned with
   `include_history` labelled `historical`; after `supersede(a, b, wrong)`, `a` is returned only
   with `include_retracted` and labelled `retracted`; in neither case does `a` enter a default
   pack. A supersede without a reason is refused.
6. **State is a function of the log.** `cortex rebuild` over a store containing every state, every
   origin and every authority reproduces a byte-identical canonical dump; the dump includes the
   derived state column so a rebuild that mis-derives one row fails.
7. **Nothing revives.** For each case in the forgetting suite, the forgotten text is absent from
   `search`, `brain.recall`, `brain.context`, `brain.graph` and the Inspector, after a rebuild, and
   after a fresh `remember` of the same text — which is a new memory with zero usefulness.
8. **The pack accounts for itself.** For a budget that forces omission, `omitted` reports counts
   that sum to (candidates − returned), and a test that sets one item `pending` sees it move from
   the pack to the `omitted.pending` count.
9. **Half-lives are per kind and visible.** With the default table, an `observation` and a
   `preference` touched once on the same day sit in different tiers after 60 simulated days; `why`
   on each names its half-life; the tier legend on a fresh store still reads honestly.
10. **The audit chain verifies, and fails.** `cortex audit --verify` passes on a store the daemon
    wrote and fails, naming the row, after one byte of one row is edited underneath it.
11. **The gold set exists and is frozen.** At least 200 query→node pairs from real episodes, hash
    recorded in `03-PROGRESS.md`, and `bench-quality` runs it in under a minute.
12. **The four falsifiers are run, whichever way they go.** Salience boost on/off; cold-tier
    retrievals per kind; co-access edges traversed in the gold set; explanations opened in the
    Inspector over a week of use. Each written into `03-PROGRESS.md` with its number. **If the boost
    does not win, it leaves the ranking in the same commit** and stays in the explanation.
13. **Adaptive fusion is measured before it is on.** `bench-quality` and `bench-latency` with the
    flag on and off, both recorded; the default follows the numbers.
14. **Fitted weights are advice.** `--fit-weights` writes a vector into the report and changes no
    ranking; a test asserts the config is untouched.
15. **Every mechanism has a bracket.** For each of 1–10 there is a test that *fails when the
    mechanism is disabled* — a check that passes with the check turned off is the failure this
    criterion exists to catch.


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
| context pack, 2 048-token budget | 120 ms | 5 000 memories + 100 000 file chunks |
| memory ANN, top 100 | 5 ms | the memory collection alone |

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
brain.remember · brain.recall_memories ·          the agent's own memory (M11)
  brain.supersede · brain.forget_memory
brain.context                                     a context pack, not a result list (M11);
                                                    every item labelled origin · authority ·
                                                    state, and an `omitted` account (M12)
brain.outcome                                     the learning signal (M11)
brain.supersede(old, new, reason)                 reason ∈ {changed, wrong} — required (M12)
fs.write … with motivated_by: memory_id           refused below `instruction` authority (M12)
audit.verify                                      walks the hash chain (M12)
agent.session.begin · agent.session.end ·         attribution (M11)
  agent.sessions
index.path · index.set_paused                     control
agent.status · agent.set_enabled                  the kill switch (R7)
audit.recent                                      (R8)
```

Three interface rules worth stating explicitly:

- **`brain.diff` exists so that an agent does not poll.** It returns everything that changed since a
  cursor, plus a new cursor.
- **`brain.graph` is the one call that returns a large object.** Document it as needing a result
  gate, and cap its `limit` at a size you have measured against your transport's framing limit
  (§7.3). In an agent loop, tool output is the dominant token cost, and a memory tool that dumps is
  the most expensive component in the loop.
- **`brain.context` is the one an agent should reach for first, and the one to get right.** Every
  other read tool returns rows; an agent has a context window, not rows. Assembling twenty results
  into two thousand useful tokens is work the daemon can do once with the scores in front of it,
  or every agent can do badly without them.
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
