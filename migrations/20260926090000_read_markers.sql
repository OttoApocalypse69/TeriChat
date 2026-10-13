-- Private per-member read position, by per-conversation message sequence.
-- A member's marker is only ever returned to that member; no read receipts.
-- Sequences order messages exactly where `last_read_at` timestamps cannot.
ALTER TABLE conversation_participants
    ADD COLUMN last_read_seq BIGINT NOT NULL DEFAULT 0
        CONSTRAINT conversation_participants_last_read_seq_nonnegative CHECK (last_read_seq >= 0);

-- Existing memberships start "read up to now" so upgrading does not flood
-- every account with historical unread counts. New memberships start at 0.
UPDATE conversation_participants p
   SET last_read_seq = GREATEST(c.next_seq - 1, 0)
  FROM conversations c
 WHERE c.id = p.conversation_id;
