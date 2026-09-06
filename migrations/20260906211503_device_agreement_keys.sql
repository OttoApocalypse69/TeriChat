-- TeriCrypt device keys: persist the X25519 agreement key alongside the
-- Ed25519 identity key. Forward-only.
--
-- Nullable so pre-existing device rows stay valid; new registrations always
-- provide both keys (the CHECK passes NULLs and constrains real values).

ALTER TABLE devices
    ADD COLUMN agreement_pubkey BYTEA CHECK (octet_length(agreement_pubkey) = 32);
