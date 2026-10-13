-- Attachment blob GC: every deleted attachment row leaves a tombstone.
-- Forward-only: never edit after it has run anywhere; supersede with a new file.
--
-- Rows leave `attachments` through cascades (conversation/channel deletion,
-- account deletion) that never reach `AttachmentStorage::delete`, which
-- orphaned the stored bytes. The trigger records each deleted id in the same
-- transaction as the delete, so a tombstone exists exactly when the row is
-- gone, whichever path deleted it. A background sweeper deletes the bytes and
-- then the tombstone; a failed delete keeps the tombstone for the next pass.

CREATE TABLE attachment_blob_gc (
    attachment_id UUID PRIMARY KEY,
    enqueued_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0)
);
CREATE INDEX attachment_blob_gc_enqueued_idx ON attachment_blob_gc (enqueued_at);

CREATE FUNCTION attachments_enqueue_blob_gc() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO attachment_blob_gc (attachment_id) VALUES (OLD.id)
    ON CONFLICT (attachment_id) DO NOTHING;
    RETURN OLD;
END;
$$;

CREATE TRIGGER attachments_blob_gc
AFTER DELETE ON attachments
FOR EACH ROW EXECUTE FUNCTION attachments_enqueue_blob_gc();
