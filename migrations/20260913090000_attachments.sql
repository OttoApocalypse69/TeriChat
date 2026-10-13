-- Attachments v1: async file/image uploads referenced from message JSON.
-- Forward-only: never edit after it has run anywhere; supersede with a new file.
--
-- Privacy shape: the server stores ORIGINAL bytes on disk (see ATTACHMENTS_DIR)
-- plus routing metadata (conversation, uploader, filename, mime, size, sha256).
-- Message envelopes are UNCHANGED: attachment refs travel inside client JSON
-- ({text, attachments:[...]}), never as new envelope fields. Per-attachment
-- keys and client-side previews are a later slice; this table holds bytes only.

CREATE TABLE attachments (
    id UUID PRIMARY KEY,
    conversation_id UUID NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
    uploader_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    filename TEXT NOT NULL CHECK (char_length(filename) BETWEEN 1 AND 255),
    mime TEXT NOT NULL CHECK (char_length(mime) BETWEEN 1 AND 127),
    size_bytes BIGINT NOT NULL CHECK (size_bytes > 0 AND size_bytes <= 10485760),
    sha256 BYTEA NOT NULL CHECK (octet_length(sha256) = 32),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX attachments_conversation_idx ON attachments (conversation_id);
