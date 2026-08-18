-- Migration 0003 — key embeddings by the text they describe, not by the row that held it.
--
-- An embedding is a pure function of (text, model). Keying it by `chunk_id` tied it to a
-- particular row instead, which had three consequences, all bad:
--
-- 1. **A rebuild threw every vector away.** `cortex --rebuild-index` re-derives chunks from the
--    filesystem, and the new rows get new ids, so the cascade deleted every embedding. On the
--    reference corpus that is hours of inference to recover something that had not changed. Rule
--    R10 calls rebuilding "a tested command, not a hope" — a command nobody can afford to run is
--    a hope with a test.
--
-- 2. **Identical text was embedded once per copy.** Boilerplate — a licence header, a shared
--    footer, an exported template — is the same chunk in dozens of files, and each copy paid for
--    the model separately.
--
-- 3. **Moving a file re-embedded it.** The path changes, the node is renamed rather than
--    recreated, but any operation that recreated chunks lost the vectors with them.
--
-- Keyed by hash, none of that happens: the vector for a piece of text is computed once, ever,
-- and survives everything except the text changing.
--
-- The usearch index still keys by chunk id, because a search has to come back to a *place* in a
-- file. It is derived from this table by joining on the hash, and it is rebuilt rather than
-- migrated — it is a cache and always was.

-- Recreated rather than altered: SQLite cannot change a primary key in place, and the old rows
-- cannot be carried across without the chunk text, which is exactly what this database does not
-- store (D16). Losing them costs one re-embedding pass; keeping a table that cannot be joined
-- correctly would cost trust.
DROP TABLE IF EXISTS embeddings;

CREATE TABLE embeddings (
    -- BLAKE3 of the chunk text, hex. The same value as `chunks.hash`, and the reason this table
    -- outlives the rows it describes.
    chunk_hash    TEXT    NOT NULL,
    model_id      TEXT    NOT NULL,
    model_version TEXT    NOT NULL,
    dim           INTEGER NOT NULL,
    vector        BLOB    NOT NULL,
    created_at    INTEGER NOT NULL,
    PRIMARY KEY (chunk_hash, model_id, model_version)
) STRICT, WITHOUT ROWID;

CREATE INDEX embeddings_model ON embeddings(model_id, model_version);
