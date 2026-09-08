-- Permanent malformed countable events must not monopolize bounded poll batches.
-- No payload/content is retained. Successful receipts remain in stats_processed_events.
CREATE TABLE stats_rejected_events (
    event_id UUID PRIMARY KEY REFERENCES outbox(id) ON DELETE CASCADE,
    rejected_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
