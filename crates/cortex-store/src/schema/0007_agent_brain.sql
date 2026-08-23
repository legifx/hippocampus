-- Migration 0007 — the agent brain (M11).
--
-- One node kind, one derived table, and the bookkeeping that makes an outcome report
-- attributable. Nothing here is a second memory system: a memory is a `nodes` row like any
-- other, so salience, tier, decay, pinning, the graph and `explain` reach it without being
-- written twice. D74 has the reasoning and what it costs.
--
-- Rule R10 is the hard part and it is why `memories` looks derived even though nothing on the
-- filesystem produces it: the authority is the `remember` episode, and this table is a
-- projection of it, exactly as `annotations` is (D35). A deletion is an assertion too — a
-- `forget_memory` episode — or the memory returns the first time the index is rebuilt and the
-- delete button was a lie (D78).

-- SQLite cannot alter a CHECK constraint, so widening one means rebuilding the table. The
-- documented procedure is followed exactly, and the two parts of it that are easy to get wrong
-- are handled by the migration runner rather than by this file:
--
--   * `PRAGMA foreign_keys` is a **no-op inside a transaction**. Written here it would do
--     nothing, `DROP TABLE nodes` would run with enforcement on, and the cascades on `edges`,
--     `chunks`, `annotations` and `salience_log` would delete the entire derived index on the
--     way out. The runner turns enforcement off outside the transaction; this migration is
--     marked `rebuilds_tables` so that it does.
--   * `PRAGMA foreign_key_check` runs before the commit, so a rebuild that left a dangling
--     reference fails the migration instead of producing a database that only breaks later.
--
-- Every index is recreated by name, including the ones added by migrations 5 and 6. Dropping
-- a table drops its indexes, and an index quietly missing after an upgrade is a performance
-- regression nobody can attribute months later.

CREATE TABLE nodes_new (
    id           INTEGER PRIMARY KEY,
    kind         TEXT    NOT NULL CHECK (kind IN
                     ('file', 'dir', 'chunk', 'concept', 'annotation', 'memory')),
    -- Absolute path. NULL for nodes that have no location: concepts, annotations, memories.
    path         TEXT,
    parent_id    INTEGER REFERENCES nodes(id) ON DELETE CASCADE,
    mime         TEXT,
    size         INTEGER NOT NULL DEFAULT 0,
    mtime        INTEGER NOT NULL DEFAULT 0,
    ctime        INTEGER NOT NULL DEFAULT 0,
    inode        INTEGER NOT NULL DEFAULT 0,
    dev          INTEGER NOT NULL DEFAULT 0,
    content_hash TEXT,
    tier         TEXT    NOT NULL DEFAULT 'stm'
                 CHECK (tier IN ('working', 'stm', 'ltm', 'cold')),
    salience     REAL    NOT NULL DEFAULT 0.0,
    pinned       INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
    zone_id      INTEGER REFERENCES zones(id) ON DELETE SET NULL,
    first_seen   INTEGER NOT NULL,
    last_seen    INTEGER NOT NULL,
    extract_state TEXT   NOT NULL DEFAULT 'pending'
                 CHECK (extract_state IN ('pending', 'done', 'skipped', 'failed')),
    extract_note TEXT,
    label        TEXT
) STRICT;

INSERT INTO nodes_new (id, kind, path, parent_id, mime, size, mtime, ctime, inode, dev,
                       content_hash, tier, salience, pinned, zone_id, first_seen, last_seen,
                       extract_state, extract_note, label)
    SELECT id, kind, path, parent_id, mime, size, mtime, ctime, inode, dev,
           content_hash, tier, salience, pinned, zone_id, first_seen, last_seen,
           extract_state, extract_note, label
    FROM nodes;

DROP TABLE nodes;
ALTER TABLE nodes_new RENAME TO nodes;

CREATE UNIQUE INDEX nodes_path_unique     ON nodes(path) WHERE path IS NOT NULL;
CREATE INDEX        nodes_parent          ON nodes(parent_id);
CREATE INDEX        nodes_identity        ON nodes(dev, inode) WHERE inode != 0;
CREATE INDEX        nodes_kind_tier       ON nodes(kind, tier);
CREATE INDEX        nodes_zone            ON nodes(zone_id);
CREATE INDEX        nodes_salience        ON nodes(salience DESC) WHERE tier IN ('working', 'stm');
CREATE INDEX        nodes_mtime           ON nodes(mtime DESC);
CREATE INDEX        nodes_extract_state   ON nodes(extract_state) WHERE extract_state = 'pending';
CREATE INDEX        nodes_progress        ON nodes(kind, extract_state);
CREATE UNIQUE INDEX nodes_concept_label   ON nodes(label) WHERE kind = 'concept';

-- The edge vocabulary gains two types, by the same procedure.
CREATE TABLE edges_new (
    src        INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    dst        INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    type       TEXT    NOT NULL CHECK (type IN
                   ('contains', 'co_access', 'imports', 'similar_to', 'derived_from',
                    'annotates',
                    -- M11: which memory replaced which, and what was recalled together.
                    'supersedes', 'recalled_with')),
    weight     REAL    NOT NULL DEFAULT 1.0,
    created_at INTEGER NOT NULL,
    source     TEXT    NOT NULL CHECK (source IN ('observed', 'inferred', 'explicit')),
    PRIMARY KEY (src, dst, type)
) STRICT;

INSERT INTO edges_new (src, dst, type, weight, created_at, source)
    SELECT src, dst, type, weight, created_at, source FROM edges;
DROP TABLE edges;
ALTER TABLE edges_new RENAME TO edges;
CREATE INDEX edges_dst  ON edges(dst, type);
CREATE INDEX edges_type ON edges(type, weight DESC);

-- Episodes gain four actions. This is the one table the whole system treats as append-only,
-- and it is rewritten here for exactly one reason: a CHECK constraint cannot be altered, and
-- leaving it narrow would reject the assertion that is the authority for the memory layer.
-- Every row is copied with its id, so nothing that points at an episode by id moves.
CREATE TABLE episodes_new (
    id            INTEGER PRIMARY KEY,
    ts            INTEGER NOT NULL,
    actor         TEXT    NOT NULL,
    action        TEXT    NOT NULL CHECK (action IN
                    ('create', 'modify', 'open', 'move', 'delete', 'search', 'annotate',
                     'remember', 'supersede', 'forget_memory', 'outcome')),
    -- Deliberately not a foreign key; an episode about a deleted node must outlive it.
    node_id       INTEGER,
    session_id    TEXT,
    metadata_json TEXT    NOT NULL DEFAULT '{}'
) STRICT;

INSERT INTO episodes_new (id, ts, actor, action, node_id, session_id, metadata_json)
    SELECT id, ts, actor, action, node_id, session_id, metadata_json FROM episodes;
DROP TABLE episodes;
ALTER TABLE episodes_new RENAME TO episodes;
CREATE INDEX episodes_ts      ON episodes(ts DESC);
CREATE INDEX episodes_node    ON episodes(node_id, ts DESC);
CREATE INDEX episodes_actor   ON episodes(actor, ts DESC);
CREATE INDEX episodes_session ON episodes(session_id, ts);

-- What is known and is not in a file.
CREATE TABLE memories (
    node_id         INTEGER PRIMARY KEY REFERENCES nodes(id) ON DELETE CASCADE,
    -- 'session:<id>' | 'agent:<name>' | 'project:<zone_id>' | 'global'. Stored as text rather
    -- than split into a tag and an id column because every query wants it whole, and because
    -- an `IN (...)` over a list the reader computed is the one place scope isolation lives.
    scope           TEXT    NOT NULL,
    kind            TEXT    NOT NULL CHECK (kind IN
                        ('fact', 'preference', 'procedure', 'decision', 'failure',
                         'observation')),
    -- The one place this system stores text instead of a pointer to it. Everywhere else the
    -- filesystem is the neocortex; here there is nothing to point at.
    text            TEXT    NOT NULL,
    subject_id      INTEGER REFERENCES nodes(id) ON DELETE SET NULL,
    -- The writer's own claim. Never evidence: see `usefulness`, which is earned.
    confidence      REAL    NOT NULL DEFAULT 0.5 CHECK (confidence BETWEEN 0.0 AND 1.0),
    usefulness      REAL    NOT NULL DEFAULT 0.0,
    -- Counted, never netted. A memory that is useful in one context and noise in another needs
    -- splitting, and a net of zero is exactly how that stays invisible (D76).
    helped_count    INTEGER NOT NULL DEFAULT 0,
    noise_count     INTEGER NOT NULL DEFAULT 0,
    -- How often it has been put into a pack. The ratio against the two above is the signal.
    retrieved_count INTEGER NOT NULL DEFAULT 0,
    human_confirmed INTEGER NOT NULL DEFAULT 0 CHECK (human_confirmed IN (0, 1)),
    -- Only 'session:' scope has one. NULL means it does not expire on its own; forgetting is
    -- still the decay engine's decision.
    expires_at      INTEGER,
    superseded_by   INTEGER REFERENCES nodes(id) ON DELETE SET NULL,
    session_id      TEXT,
    -- The assertion this row is a projection of. Not a foreign key: episodes outlive
    -- everything, which is the whole point of them.
    episode_id      INTEGER NOT NULL,
    actor           TEXT    NOT NULL,
    ts              INTEGER NOT NULL
) STRICT;

-- Retrieval always filters by scope first, and almost always excludes the superseded.
CREATE INDEX memories_scope    ON memories(scope, kind) WHERE superseded_by IS NULL;
CREATE INDEX memories_subject  ON memories(subject_id) WHERE subject_id IS NOT NULL;
CREATE INDEX memories_expiry   ON memories(expires_at) WHERE expires_at IS NOT NULL;
CREATE INDEX memories_session  ON memories(session_id) WHERE session_id IS NOT NULL;

-- One agent's run against the surface. Without this, `actor` says `agent:claude` for a year of
-- unrelated work and no question about *this task* can be answered.
CREATE TABLE agent_sessions (
    id         TEXT PRIMARY KEY,
    name       TEXT    NOT NULL,
    task       TEXT,
    started_at INTEGER NOT NULL,
    ended_at   INTEGER,
    calls      INTEGER NOT NULL DEFAULT 0,
    refusals   INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX agent_sessions_started ON agent_sessions(started_at DESC);
CREATE INDEX agent_sessions_open    ON agent_sessions(ended_at) WHERE ended_at IS NULL;

-- What a context pack contained. An outcome reported ten minutes later must be attributed to
-- the items that were in the pack, not to whatever the same query ranks highly now — and by
-- then the ranking has moved, because the pack itself moved it.
CREATE TABLE context_packs (
    id            TEXT    PRIMARY KEY,
    session_id    TEXT,
    task          TEXT    NOT NULL,
    budget_tokens INTEGER NOT NULL,
    used_tokens   INTEGER NOT NULL,
    created_at    INTEGER NOT NULL
) STRICT;

CREATE TABLE context_pack_items (
    pack_id  TEXT    NOT NULL REFERENCES context_packs(id) ON DELETE CASCADE,
    node_id  INTEGER NOT NULL,
    rank     INTEGER NOT NULL,
    arm      TEXT    NOT NULL,
    score    REAL    NOT NULL,
    -- NULL until reported. 'helped' | 'noise'.
    reported TEXT,
    PRIMARY KEY (pack_id, node_id)
) STRICT;

CREATE INDEX context_packs_created ON context_packs(created_at DESC);
