-- Migration 0001 — the complete data model.
--
-- Design note that governs this whole file: `episodes` is the only table that is never
-- mutated. Everything else is derivable from it plus the filesystem. That is what makes the
-- index a cache rather than a data store, and it is what makes aggressive forgetting safe.
--
-- Chunk text is never stored. The FTS index is contentless: it tokenises text for BM25
-- without keeping a copy, and snippets are re-read from the file using the stored byte
-- offsets. The filesystem is the neocortex; this database holds pointers into it.

-- Spatial zones: the major roots of the tree, used for colour coding and for scoping.
-- Declared before `nodes` because `nodes` references it.
CREATE TABLE zones (
    id            INTEGER PRIMARY KEY,
    root_path     TEXT    NOT NULL UNIQUE,
    name          TEXT    NOT NULL,
    -- Derived deterministically from the zone identity so it is stable across sessions.
    -- Saturation and lightness are fixed by the theme, so only the hue is stored.
    color_hue     INTEGER NOT NULL CHECK (color_hue BETWEEN 0 AND 359),
    icon          TEXT,
    -- 1 when the user picked the colour by hand, so regeneration must not overwrite it.
    user_override INTEGER NOT NULL DEFAULT 0 CHECK (user_override IN (0, 1))
) STRICT;

-- What exists, and where. A node is not necessarily a file.
CREATE TABLE nodes (
    id           INTEGER PRIMARY KEY,
    kind         TEXT    NOT NULL CHECK (kind IN ('file', 'dir', 'chunk', 'concept', 'annotation')),
    -- Absolute path. NULL for nodes that have no location: concepts and annotations.
    path         TEXT,
    parent_id    INTEGER REFERENCES nodes(id) ON DELETE CASCADE,
    mime         TEXT,
    size         INTEGER NOT NULL DEFAULT 0,
    mtime        INTEGER NOT NULL DEFAULT 0,
    ctime        INTEGER NOT NULL DEFAULT 0,
    -- Identity as the kernel sees it. A move is detected by (dev, inode) surviving a path
    -- change; without this a `mv` of a large directory looks like a mass delete plus a mass
    -- create, which destroys history and forces a pointless re-index.
    inode        INTEGER NOT NULL DEFAULT 0,
    dev          INTEGER NOT NULL DEFAULT 0,
    content_hash TEXT,
    tier         TEXT    NOT NULL DEFAULT 'stm' CHECK (tier IN ('working', 'stm', 'ltm', 'cold')),
    salience     REAL    NOT NULL DEFAULT 0.0,
    pinned       INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
    zone_id      INTEGER REFERENCES zones(id) ON DELETE SET NULL,
    first_seen   INTEGER NOT NULL,
    last_seen    INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX nodes_path_unique ON nodes(path) WHERE path IS NOT NULL;
CREATE INDEX nodes_parent      ON nodes(parent_id);
CREATE INDEX nodes_identity    ON nodes(dev, inode) WHERE inode != 0;
CREATE INDEX nodes_kind_tier   ON nodes(kind, tier);
CREATE INDEX nodes_zone        ON nodes(zone_id);
-- Ranking reads salience constantly and only ever cares about nodes still in a hot tier.
CREATE INDEX nodes_salience    ON nodes(salience DESC) WHERE tier IN ('working', 'stm');
CREATE INDEX nodes_mtime       ON nodes(mtime DESC);

-- Associative connections. This is the hippocampal index proper.
CREATE TABLE edges (
    src        INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    dst        INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    type       TEXT    NOT NULL CHECK (type IN (
                   'contains', 'co_access', 'imports', 'similar_to', 'derived_from', 'annotates')),
    weight     REAL    NOT NULL DEFAULT 1.0,
    created_at INTEGER NOT NULL,
    -- How we came to believe this edge. `explicit` edges are user or agent assertions and
    -- must never be pruned by a process that only trusts its own inferences.
    source     TEXT    NOT NULL CHECK (source IN ('observed', 'inferred', 'explicit')),
    PRIMARY KEY (src, dst, type)
) STRICT, WITHOUT ROWID;

CREATE INDEX edges_dst  ON edges(dst, type);
CREATE INDEX edges_type ON edges(type, weight DESC);

-- Episodic memory. Append-only. Never updated, never deleted by any normal operation.
CREATE TABLE episodes (
    id            INTEGER PRIMARY KEY,
    ts            INTEGER NOT NULL,
    -- 'human' | 'system' | 'agent:<name>'
    actor         TEXT    NOT NULL,
    action        TEXT    NOT NULL CHECK (action IN (
                      'create', 'modify', 'open', 'move', 'delete', 'search', 'annotate')),
    -- Deliberately not a foreign key: an episode about a deleted file must outlive the node.
    -- The episode log is the ground truth; nodes are derived from it.
    node_id       INTEGER,
    session_id    TEXT,
    metadata_json TEXT    NOT NULL DEFAULT '{}'
) STRICT;

CREATE INDEX episodes_ts      ON episodes(ts DESC);
CREATE INDEX episodes_node    ON episodes(node_id, ts DESC);
CREATE INDEX episodes_session ON episodes(session_id, ts);
CREATE INDEX episodes_actor   ON episodes(actor, ts DESC);

-- Pattern separation units. Offsets into the file, not copies of it.
CREATE TABLE chunks (
    id           INTEGER PRIMARY KEY,
    node_id      INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    ordinal      INTEGER NOT NULL,
    byte_start   INTEGER NOT NULL,
    byte_end     INTEGER NOT NULL,
    -- Content hash of this chunk. The single largest performance lever in ingest: an unchanged
    -- chunk is never re-embedded, so editing one paragraph of a large document costs one
    -- embedding call rather than hundreds.
    hash         TEXT    NOT NULL,
    -- A short excerpt for result display, so the common case needs no file I/O. Bounded in
    -- code; this is an excerpt, never the chunk.
    text_preview TEXT,
    UNIQUE (node_id, ordinal)
) STRICT;

CREATE INDEX chunks_hash ON chunks(hash);

-- Lexical index. Contentless: tokens only, no stored text.
-- rowid is the chunk id, so a hit maps straight back to `chunks`.
CREATE VIRTUAL TABLE chunks_fts USING fts5(
    text,
    content = '',
    tokenize = 'unicode61 remove_diacritics 2'
);

-- Vectors, versioned by model so that changing the model is a background migration rather
-- than a full rebuild.
CREATE TABLE embeddings (
    chunk_id      INTEGER NOT NULL REFERENCES chunks(id) ON DELETE CASCADE,
    model_id      TEXT    NOT NULL,
    model_version TEXT    NOT NULL,
    dim           INTEGER NOT NULL,
    vector        BLOB    NOT NULL,
    created_at    INTEGER NOT NULL,
    PRIMARY KEY (chunk_id, model_id, model_version)
) STRICT, WITHOUT ROWID;

CREATE INDEX embeddings_model ON embeddings(model_id, model_version);

-- Federation. Events queue here and drain when a peer is reachable; offline is the normal case.
CREATE TABLE sync_outbox (
    id           INTEGER PRIMARY KEY,
    -- Stable across retries and restarts, so duplicate delivery is a no-op on the peer.
    event_id     TEXT    NOT NULL UNIQUE,
    lamport      INTEGER NOT NULL,
    payload_json TEXT    NOT NULL,
    created_at   INTEGER NOT NULL,
    attempts     INTEGER NOT NULL DEFAULT 0,
    last_error   TEXT
) STRICT;

CREATE INDEX sync_outbox_pending ON sync_outbox(lamport) WHERE attempts < 100;

CREATE TABLE sync_state (
    peer               TEXT    PRIMARY KEY,
    last_acked_lamport INTEGER NOT NULL DEFAULT 0,
    last_sync_at       INTEGER
) STRICT, WITHOUT ROWID;

-- Explainability. Every ranking must be able to say why.
CREATE TABLE salience_log (
    node_id         INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    computed_at     INTEGER NOT NULL,
    score           REAL    NOT NULL,
    components_json TEXT    NOT NULL,
    PRIMARY KEY (node_id, computed_at)
) STRICT, WITHOUT ROWID;

-- Agent safety. Every agentic mutation stores its own inverse before it is applied.
CREATE TABLE undo_journal (
    id            INTEGER PRIMARY KEY,
    ts            INTEGER NOT NULL,
    actor         TEXT    NOT NULL,
    operation_json TEXT   NOT NULL,
    inverse_json  TEXT    NOT NULL,
    applied       INTEGER NOT NULL DEFAULT 0 CHECK (applied IN (0, 1)),
    undone_at     INTEGER
) STRICT;

CREATE INDEX undo_journal_ts ON undo_journal(ts DESC);

-- Every agent access, allowed or refused. Refusals matter most.
CREATE TABLE audit_log (
    id             INTEGER PRIMARY KEY,
    ts             INTEGER NOT NULL,
    actor          TEXT    NOT NULL,
    tool           TEXT    NOT NULL,
    args_json      TEXT    NOT NULL,
    result_summary TEXT,
    allowed        INTEGER NOT NULL CHECK (allowed IN (0, 1))
) STRICT;

CREATE INDEX audit_log_ts    ON audit_log(ts DESC);
CREATE INDEX audit_log_actor ON audit_log(actor, ts DESC);

-- Lamport clock and other small daemon-wide values.
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT, WITHOUT ROWID;

INSERT INTO meta (key, value) VALUES ('lamport', '0');
