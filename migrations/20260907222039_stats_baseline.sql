-- Issue #7 Stats baseline: per-user message counters folded from outbox
-- routing metadata — never plaintext.
-- Forward-only: never edit after it has run anywhere; supersede with a new file.
--
-- Privacy shape: counters derive from `message.created` outbox payload fields
-- (`actor_id`, `conversation_id`) only. No ciphertext, plaintext, or envelope
-- bytes are stored here — only user/conversation ids, counts, and timestamps.
-- Idempotency: `stats_processed_events` dedupes on the outbox `event_id`
-- (UUIDv7 PRIMARY KEY); concurrent duplicate deliveries race on that insert
-- and exactly one wins the count.
-- Isolation: this branch has no workspaces module, so isolation is
-- per-user/per-conversation and the query endpoints serve only the requesting
-- user's own rollup. Workspace scoping is a follow-up once workspaces land.

CREATE TABLE user_message_stats (
    user_id UUID PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    message_count BIGINT NOT NULL DEFAULT 0 CHECK (message_count >= 0),
    last_message_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE user_conversation_stats (
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    conversation_id UUID NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
    message_count BIGINT NOT NULL DEFAULT 0 CHECK (message_count >= 0),
    last_message_at TIMESTAMPTZ,
    PRIMARY KEY (user_id, conversation_id)
);
CREATE INDEX user_conversation_stats_conversation_idx
    ON user_conversation_stats (conversation_id);

CREATE TABLE stats_processed_events (
    event_id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    conversation_id UUID NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TRIGGER user_message_stats_set_updated_at
    BEFORE UPDATE ON user_message_stats
    FOR EACH ROW
    EXECUTE FUNCTION set_updated_at();
