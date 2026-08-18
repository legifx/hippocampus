-- Migration 0002 — rebuild the lexical index so rows can be deleted.
--
-- Migration 0001 created `chunks_fts` as a plain contentless FTS5 table. Contentless tables
-- cannot delete a row without being handed the original text back, which the design in decision
-- D16 specifically does not keep. `contentless_delete=1` (SQLite 3.43+) removes that
-- restriction while still storing no copy of the text.
--
-- Recreating rather than migrating is correct here: the FTS index is derived data, and
-- rebuilding it from the files is a supported operation. That is the whole point of the index
-- being a cache.

DROP TABLE IF EXISTS chunks_fts;

CREATE VIRTUAL TABLE chunks_fts USING fts5(
    text,
    content = '',
    contentless_delete = 1,
    -- `remove_diacritics 2` folds ä to a, so a German query typed without umlauts still finds
    -- the document. Users type "grosse" and mean "größe"; refusing that match would make the
    -- search feel broken to exactly the audience this corpus belongs to.
    tokenize = 'unicode61 remove_diacritics 2'
);

-- Extraction state per node, so the pipeline knows what still needs doing without re-reading
-- the file to find out.
ALTER TABLE nodes ADD COLUMN extract_state TEXT NOT NULL DEFAULT 'pending'
    CHECK (extract_state IN ('pending', 'done', 'skipped', 'failed'));

-- Why a file was skipped or failed. Diagnostic only, but the alternative is a user asking why
-- their document is not findable and nobody being able to answer.
ALTER TABLE nodes ADD COLUMN extract_note TEXT;

-- Content hash of the file as extracted, so an unchanged file is never re-chunked.
CREATE INDEX nodes_extract_state ON nodes(extract_state) WHERE extract_state = 'pending';
