-- Annotations, as a table rather than as a shape reconstructed from JSON on every read.
--
-- This is derived state, and it obeys rule R10 the same way chunks do: every row here comes
-- from an `annotate` assertion in `episodes`, and `replay_assertions` rebuilds the table from
-- that log. Dropping it loses nothing that the episode log cannot put back.
--
-- The reason it exists at all is that node ids do not survive a rebuild. Reading an
-- annotation's text back out of the episode that created it works exactly until the rebuild
-- gives the annotation node a new id, at which point the episode points at a node that is gone.
-- A derived table sidesteps that: the replay writes both the node and this row together.
CREATE TABLE annotations (
    node_id    INTEGER PRIMARY KEY REFERENCES nodes(id) ON DELETE CASCADE,
    target_id  INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    -- The episode this was derived from. Not a foreign key: episodes outlive everything.
    episode_id INTEGER NOT NULL,
    tags_json  TEXT    NOT NULL DEFAULT '[]',
    note       TEXT    NOT NULL,
    actor      TEXT    NOT NULL,
    ts         INTEGER NOT NULL
) STRICT;

CREATE INDEX annotations_target ON annotations(target_id, ts DESC);
