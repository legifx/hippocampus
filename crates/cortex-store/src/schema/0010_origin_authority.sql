-- Where a thing came from, and what a memory is allowed to do (M12 step 1, D98, D117).
--
-- The episode log had known all of this from the beginning: a file's chunk comes from the file,
-- an annotation comes from whoever wrote it, a memory comes from the connection that asserted
-- it. The projections threw that away. Every memory in `memories` was equal — the text a human
-- typed as a rule, the sentence an agent read on a web page and the line a tool printed all sat
-- in the same table with the same standing, and the only thing that distinguished them was an
-- `actor` string nobody filtered on. The pack therefore could not say "this is not a fact, it is
-- something an agent noticed", which is the difference between a memory and an instruction to
-- delete the user's files.
--
-- Three things arrive here, and only the first is filled by this slice's code:
--
--   * `nodes.origin` — who a thing came from. Written by the write path from the *connection's*
--     actor, never from a request body: no client can name its own origin, because no request
--     type carries the field (M12 criterion 1). The backfill below is the honest half of it: a
--     chunk inherits the file it was cut from, and a node whose creating episode is still in the
--     log takes that episode's actor.
--   * `memories.authority` — what the memory may be used as. Filled by the write path from the
--     origin's ceiling; the ladder and the ceilings live in `cortex_core::memory`.
--   * `memories.basis_json` + the validity window — the things a memory rests on and the times
--     it was valid. Step 1's criterion 2 (inheritance) and step 2 (explicit validity) fill
--     them; they are in *this* migration because the alternative is three more ALTERs on a
--     table that `rebuild` (R10) rewrites through a projection, and because a memory written
--     without them would have to be backfilled a second time.
--
-- What the backfill deliberately does *not* do is replay the imperative detector that criterion
-- 3 adds. A detector's verdict belongs to the write that produced the text: for a row that
-- predates this migration nobody re-read the sentence, and a migration that guessed "this was a
-- rule" would be raising authority on the strength of a guess. So a legacy row gets the ceiling
-- of the one actor whose ceiling is not a claim about the world — the human's own words — and
-- everything else starts at `observation`: usable in a pack, never a motivation for a write.
-- (2026-09-17)

ALTER TABLE nodes ADD COLUMN origin TEXT NOT NULL DEFAULT 'file'
    CHECK (origin IN ('human', 'agent', 'tool', 'file', 'peer'));

-- Material: the file, its directories, and every chunk cut from it. The origin of a chunk is the
-- file, whichever stage wrote the row.
UPDATE nodes SET origin = 'file' WHERE kind IN ('file', 'dir', 'chunk');

-- The rest was asserted by somebody, and the log still remembers who. The mapping is
-- `Origin::from_actor`'s, spelled out here because a migration cannot call into Rust: `system`
-- is the daemon's own bookkeeping and reads as `file`, an anonymous connection reads as `peer`.
UPDATE nodes
   SET origin = (SELECT CASE
                            WHEN e.actor = 'human'       THEN 'human'
                            WHEN e.actor = 'system'      THEN 'file'
                            WHEN e.actor LIKE 'agent:%'  THEN 'agent'
                            ELSE 'peer'
                        END
                   FROM episodes e
                  WHERE e.node_id = nodes.id
                    AND e.action IN ('annotate', 'remember')
                  ORDER BY e.ts ASC, e.id ASC
                  LIMIT 1)
 WHERE kind IN ('concept', 'annotation', 'memory')
   AND EXISTS (SELECT 1 FROM episodes e
                WHERE e.node_id = nodes.id AND e.action IN ('annotate', 'remember'));

ALTER TABLE memories ADD COLUMN authority TEXT NOT NULL DEFAULT 'observation'
    CHECK (authority IN ('untrusted', 'observation', 'fact', 'instruction'));

-- The human's own words are the one thing that was already an instruction when it was written;
-- an agent's assertion was never re-read, so it is an observation like any other unclaimed row.
UPDATE memories SET authority = CASE WHEN actor = 'human' THEN 'instruction' ELSE 'observation' END;

-- The list of what a memory rests on: `[{ "id": 12, "path": "/x.rs", "authority": "fact" }]`.
-- JSON rather than a join table, for the same reason `episodes.metadata_json` is JSON — it is
-- written once, read with the row it belongs to, and the pack's `min` over it is arithmetic in
-- Rust. A table that indexes the other direction (which memories rest on this file?) can be
-- added when a query actually goes that way.
ALTER TABLE memories ADD COLUMN basis_json TEXT NOT NULL DEFAULT '[]';

-- Validity. Deliberately empty, and deliberately nullable: the scope of step 2 says the daemon
-- never fills `valid_from` with `created_at` silently — "recorded 2026-09-05, validity unknown" is
-- the honest answer for a memory that was recorded today about something that may have been true
-- in 2019, and `ts` is the recording instant, not the validity one. So nothing writes these two
-- until step 2 gives `brain.remember` a field for them; they are here because the alternative is
-- two more ALTERs on a table that `rebuild` rewrites through a projection.
ALTER TABLE memories ADD COLUMN valid_from INTEGER;
ALTER TABLE memories ADD COLUMN valid_to INTEGER;

-- The pack filters by authority inside a scope, so the scope's own index still leads; this one
-- exists for the Inspector's and criterion 3's questions ("what is in the index that the pack
-- will never show?"), which have no scope and no path to start from.
CREATE INDEX memories_authority ON memories(authority);
