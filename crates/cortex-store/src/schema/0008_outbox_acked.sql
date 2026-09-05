-- M7. An acknowledged event stays in the outbox, marked, instead of being deleted (D18: "the
-- outbox marks a row acknowledged only after the peer state is confirmed"). Two reasons it
-- has to stay: a concept event is harvested by identity rather than by cursor, and would be
-- queued again the moment its row was gone; and a human asking "did this reach the peer" needs
-- a row to ask about. Acknowledged episode events are pruned after a while — the cursor
-- already guarantees they cannot come back — and concept events are kept.
ALTER TABLE sync_outbox ADD COLUMN acked_at INTEGER;
ALTER TABLE sync_outbox ADD COLUMN kind TEXT NOT NULL DEFAULT 'episode';

DROP INDEX IF EXISTS sync_outbox_pending;
CREATE INDEX sync_outbox_pending ON sync_outbox(lamport)
    WHERE acked_at IS NULL AND attempts < 100;
CREATE INDEX sync_outbox_acked ON sync_outbox(kind, acked_at) WHERE acked_at IS NOT NULL;
