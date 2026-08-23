-- A concept's name gets a column of its own.
--
-- It used to live in `mime`, which is where `upsert_concept` put it, and the lookup that found
-- it read
--
--     WHERE kind = 'concept' AND json_extract(mime, '$') IS NULL AND mime = ?1
--
-- That clause was meant to say "concepts have no path". What it actually does is ask SQLite to
-- parse a concept's name as JSON, which fails with `malformed JSON` for every name that is not
-- a JSON document — that is, for every name. The abstraction step was the first code to call it
-- with a real label, and it failed on the first call.
--
-- A MIME type is a MIME type. Keeping a name in it would have gone on being wrong quietly.
ALTER TABLE nodes ADD COLUMN label TEXT;

-- Concepts are looked up by name on every consolidation pass, and the name is their identity.
CREATE UNIQUE INDEX nodes_concept_label ON nodes(label) WHERE kind = 'concept';

-- Anything already carrying a name in `mime` keeps it, in the right place now.
UPDATE nodes SET label = mime, mime = NULL WHERE kind = 'concept' AND mime IS NOT NULL;
