-- A chunk that was deleted — its file changed or went away — leaves a vector behind in the ANN
-- index, and nothing removed it: the index keys by chunk id, the extraction stage that deletes
-- the chunk has no handle on the index, and the only thing that ever caught up was the
-- embedding stage noticing the count was off by the number of files edited since the last check
-- and rebuilding the whole index. On the reference machine that was a twelve-second rebuild
-- every five minutes, for ever (2026-09-06). This table is the handle: the deleting side writes
-- the id here, the embedding stage removes the vector and clears the row. Derived bookkeeping,
-- not a data store — a rebuild owes it nothing (R2).
CREATE TABLE dead_chunks (
    chunk_id INTEGER PRIMARY KEY
) WITHOUT ROWID;
