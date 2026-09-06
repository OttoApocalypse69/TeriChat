-- Milestone C messaging: conversations, encrypted envelopes, transactional outbox.
-- Forward-only: never edit after it has run anywhere; supersede with a new file.
--
-- Privacy shape: the server stores OPAQUE ciphertext bytes plus routing
-- metadata (conversation, sender, sequence). It never sees plaintext and
-- must never log envelope bytes. `client_msg_id` makes sends idempotent:
-- a retried send returns the original row instead of duplicating it.
-- `outbox.id` is UUIDv7 (time-ordered) so the gateway can resume by event id.

CREATE TABLE conversations (
    id UUID PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('dm', 'group')) DEFAULT 'dm',
    created_by UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Per-conversation message counter, bumped under row lock inside the
    -- send transaction. Drives history pagination and gateway resume.
    next_seq BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE conversation_participants (
    conversation_id UUID NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    joined_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_read_at TIMESTAMPTZ,
    PRIMARY KEY (conversation_id, user_id)
);
CREATE INDEX conversation_participants_user_idx ON conversation_participants (user_id);

CREATE TABLE messages (
    id UUID PRIMARY KEY,
    conversation_id UUID NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
    sender_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    sender_device_id UUID REFERENCES devices (id) ON DELETE SET NULL,
    -- Per-conversation sequence, assigned at send time. Unique per conversation.
    seq BIGINT NOT NULL,
    -- Opaque encrypted payload. The server cannot read it; clients own the suite.
    ciphertext BYTEA NOT NULL CHECK (octet_length(ciphertext) > 0 AND octet_length(ciphertext) <= 1048576),
    -- Opaque nonce/IV supplied by the sending client. NULL when the suite
    -- carries its own framing.
    nonce BYTEA,
    -- Client-chosen idempotency key. Retried sends return the original row.
    client_msg_id UUID NOT NULL,
    sent_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (conversation_id, seq),
    UNIQUE (conversation_id, client_msg_id)
);
CREATE INDEX messages_conversation_seq_idx ON messages (conversation_id, seq);

-- Transactional outbox: written in the SAME transaction as the message.
-- A worker claims unpublished rows, broadcasts them, then stamps them.
CREATE TABLE outbox (
    -- UUIDv7: time-ordered, doubles as the global event position for resume.
    id UUID PRIMARY KEY,
    topic TEXT NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    published_at TIMESTAMPTZ,
    attempts INT NOT NULL DEFAULT 0
);
CREATE INDEX outbox_unpublished_idx ON outbox (created_at) WHERE published_at IS NULL;
