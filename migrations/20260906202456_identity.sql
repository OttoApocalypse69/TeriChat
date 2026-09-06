-- Alpha 0 identity foundation: accounts, devices, sessions.
-- Forward-only: never edit after it has run anywhere; supersede with a new file.
-- Account identity and TeriCrypt identity stay separate (see 12_OPEN_DECISIONS.md):
-- this migration covers the account side plus the device-identity key pointer.
-- Secrets are never stored here in recoverable form: password/token material
-- lives only as Argon2id PHC strings / SHA-256 hashes, filled by Milestone B.

CREATE TABLE users (
    id UUID PRIMARY KEY,
    -- Human-facing, globally unique, changeable. Never a foreign-key target:
    -- other tables reference users(id). Application lowercases on write.
    handle TEXT NOT NULL UNIQUE CHECK (char_length(handle) BETWEEN 2 AND 32),
    email TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL CHECK (char_length(display_name) BETWEEN 1 AND 64),
    -- Argon2id PHC string (e.g. "$argon2id$v=19$..."). Empty until Milestone B wires registration.
    password_hash TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE devices (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    label TEXT NOT NULL DEFAULT '',
    -- Initial per-device identity public key (Ed25519, 32 bytes). Each
    -- installation is a distinct member; rotation appends a row, never edits.
    identity_pubkey BYTEA NOT NULL CHECK (octet_length(identity_pubkey) = 32),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ
);
CREATE INDEX devices_user_id_idx ON devices (user_id);

CREATE TABLE sessions (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    device_id UUID REFERENCES devices (id) ON DELETE SET NULL,
    -- SHA-256 of the opaque bearer token. The raw token is shown once at
    -- login and never stored, logged, or returned again.
    token_hash BYTEA NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ
);
CREATE INDEX sessions_user_id_idx ON sessions (user_id);

-- Keep users.updated_at honest without application discipline.
CREATE OR REPLACE FUNCTION set_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER users_set_updated_at
    BEFORE UPDATE ON users
    FOR EACH ROW
    EXECUTE FUNCTION set_updated_at();
