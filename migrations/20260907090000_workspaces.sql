-- Milestone E workspaces: governed spaces with roles, text channels, invites,
-- channel permission overrides, bans, and a minimal audit log.
-- Forward-only: never edit after it has run anywhere; supersede with a new file.
--
-- Design notes:
-- - Each text channel is backed by one `conversations` row of kind 'channel'
--   (`channels.conversation_id`, UNIQUE). Channel messages reuse the Milestone C
--   path (sequenced sends, opaque envelopes, outbox fan-out) unchanged; the
--   HTTP layer adds the workspace permission check on top. Channel membership
--   is denormalized into `conversation_participants` (synced by the
--   application on join/kick/ban/channel-create) so gateway resume keeps
--   working through the existing membership filter.
-- - Banned users stay in `workspace_bans` after removal and cannot rejoin via
--   invite or direct add until unbanned. Kicked users may rejoin.
-- - Audit rows never carry message plaintext — actions and ids only.

-- Channels need a third conversation kind. The inline CHECK from the
-- messaging migration auto-names itself `conversations_kind_check`.
ALTER TABLE conversations DROP CONSTRAINT IF EXISTS conversations_kind_check;
ALTER TABLE conversations
    ADD CONSTRAINT conversations_kind_check
    CHECK (kind IN ('dm', 'group', 'channel'));

CREATE TABLE workspaces (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 100),
    owner_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE workspace_members (
    workspace_id UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- 'owner' | 'admin' | 'moderator' | 'member' | 'guest'. Validated by the
    -- application (see workspaces.rs); CHECK keeps rogue writers honest.
    role TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'moderator', 'member', 'guest')),
    joined_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, user_id)
);
CREATE INDEX workspace_members_user_idx ON workspace_members (user_id);

CREATE TABLE workspace_bans (
    workspace_id UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    banned_by UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    reason TEXT NOT NULL DEFAULT '' CHECK (char_length(reason) <= 500),
    banned_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, user_id)
);

CREATE TABLE channels (
    id UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    -- Backing conversation for the Milestone C message path. One-to-one.
    conversation_id UUID NOT NULL UNIQUE REFERENCES conversations (id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 100),
    kind TEXT NOT NULL DEFAULT 'text' CHECK (kind = 'text'),
    created_by UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, name)
);
CREATE INDEX channels_workspace_idx ON channels (workspace_id);

-- Per-channel permission overrides. Only 'SEND_MESSAGES' and 'MANAGE_MESSAGES'
-- are overridable in Alpha; the application rejects the rest. `target` holds a
-- role name when `target_kind = 'role'`, or a user id (text) when 'member'.
-- A member-specific row beats a role row; an applicable row beats the base
-- role grant. Absence of a row means the base role grant applies.
CREATE TABLE channel_overrides (
    channel_id UUID NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    target_kind TEXT NOT NULL CHECK (target_kind IN ('role', 'member')),
    target TEXT NOT NULL CHECK (char_length(target) BETWEEN 1 AND 64),
    permission TEXT NOT NULL CHECK (permission IN ('SEND_MESSAGES', 'MANAGE_MESSAGES')),
    allowed BOOLEAN NOT NULL,
    PRIMARY KEY (channel_id, target_kind, target, permission)
);

-- Single-use-or-bounded invite codes. `code` is a random unguessable string
-- (application-generated); `uses` counts redemptions against `max_uses`.
CREATE TABLE invites (
    id UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    code TEXT NOT NULL UNIQUE CHECK (char_length(code) BETWEEN 16 AND 128),
    created_by UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    initial_role TEXT NOT NULL DEFAULT 'member'
        CHECK (initial_role IN ('admin', 'moderator', 'member', 'guest')),
    expires_at TIMESTAMPTZ,
    max_uses INT CHECK (max_uses IS NULL OR max_uses > 0),
    uses INT NOT NULL DEFAULT 0 CHECK (uses >= 0),
    revoked BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX invites_workspace_idx ON invites (workspace_id);

-- Minimal audit framework (P2 starter): who did what to whom, when. Never
-- message plaintext — `detail` carries only ids, names, and roles.
CREATE TABLE workspace_audit (
    id UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    actor_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    action TEXT NOT NULL CHECK (char_length(action) BETWEEN 1 AND 64),
    target_id UUID REFERENCES users (id) ON DELETE SET NULL,
    detail JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX workspace_audit_workspace_idx ON workspace_audit (workspace_id, created_at);

CREATE TRIGGER workspaces_set_updated_at
    BEFORE UPDATE ON workspaces
    FOR EACH ROW
    EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER channels_set_updated_at
    BEFORE UPDATE ON channels
    FOR EACH ROW
    EXECUTE FUNCTION set_updated_at();
