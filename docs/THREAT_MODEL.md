# TeriChat Threat Model (skeleton — Alpha)

This is a working skeleton, not a completed security review. It records what
the current implementation does and does not defend against. Every claim here
must be re-verified before any production or real-user milestone.

## Trust domains

- **Account identity** (handle, email, billing, memberships): visible to the
  server by design. Separate from cryptographic identity.
- **TeriCrypt identity** (device keys, envelopes, recovery material): the
  server routes opaque bytes and must never need plaintext to operate.
- **Recovery words**: client-only, never sent to or verified by the server
  (envelope-vault direction accepted; encoding/persistence TBD).

## What the Alpha implementation provides

- `Argon2id` password hashes (`PHC` strings); no plaintext passwords stored.
- Bearer tokens stored as `SHA-256` hashes, shown once, revocable, expiring.
- 1:1 sealed envelopes (`crates/tericrypt`): ephemeral `X25519` →
  `HKDF-SHA256` (`terichat-dm-v2`, binding both peer identities) →
  `XChaCha20-Poly1305`, `Ed25519`-signed over the full context. The two
  device secrets are drawn independently; all-zero peer keys are refused at
  seal time and at device registration — a planted zero directory key cannot
  yield a predictable envelope key.
  See `crates/tericrypt/README.md` for the construction and libraries.
- Membership-checked sends/history (non-members learn nothing about
  existence), idempotent sends, revocation that actually kills sessions.
- No credential or plaintext logging anywhere in request paths.

## Explicitly NOT provided (do not claim otherwise)

- **Forward secrecy**: recipient agreement keys are long-term. Compromise of
  a device key exposes past envelopes to that device. Ratcheting/`MLS` is
  the planned fix (Milestone D follow-up).
- **Deniability**: envelopes are signed; a recipient can prove authorship.
- **Group E2EE**: groups currently store one sender-posted blob with no
  per-recipient sealing and no `MLS` group state. Real group privacy needs
  the `MLS` follow-up, not scoped credential labels.
- **Key transparency / verification**: no QR/safety-number ceremony yet; a
  malicious server could substitute device keys undetected. (Zero-key
  substitution specifically is now refused, but arbitrary-key substitution
  is not — transparency is still the real fix.)
- **Low-order peer keys**: all-zero `X25519` keys are refused at seal time
  and at registration, but non-zero low-order points are not yet blacklisted
  (torsion check rides with the verification ceremonies). `Ed25519`
  small-order identity keys parse at registration but fail closed at open
  via `verify_strict`.
- **Metadata privacy**: the server sees who talks to whom, when, and how
  much (conversation graph, timing, envelope sizes). No padding yet.
- **Replay protection at the crypto layer**: ordering comes from the
  server's per-conversation sequence, which a malicious server can forge.
- **Post-quantum security**: all suites are classical elliptic-curve.
- **Device compromise recovery**: no remote wipe attestation, no key
  rotation protocol yet (re-registration only).
- **Malicious endpoints**: a compromised peer device exposes its plaintext;
  no deletion claim defeats a recipient that kept copies.

## Out of scope for this document (tracked elsewhere)

- Billing/ledger correctness (`09`, Milestone G), plugin/AI capability
  enforcement (Milestones I–J), infrastructure hardening (`10`).
- The 24-word recovery encoding and vault serialization (open decisions).

## Review bar

Crypto production claims need appropriate specialist review — never just an
owner click or an agent quorum. This file must grow a dated review log
before any real-user rollout.
