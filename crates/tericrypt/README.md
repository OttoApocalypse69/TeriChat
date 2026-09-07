# `tericrypt` — TeriCrypt-4096 device crypto (Alpha PoC)

Established primitives only. No invented algorithms, no homemade protocol:
the 1:1 envelope is the standard ephemeral-`ECDH` + `KDF` + `AEAD` + signature
composition (same shape as `NaCl crypto_box` plus explicit sender
authentication), and the production direction is `OpenMLS` groups.

## Libraries (reviewed, maintained)

| Role | Crate | Version used |
|---|---|---|
| Identity signing | `ed25519-dalek` | 2.x |
| Key agreement | `x25519-dalek` (`static_secrets`) | 2.x |
| Symmetric seal | `chacha20poly1305` (`XChaCha20-Poly1305`) | 0.10 |
| Key derivation | `hkdf` (`SHA-256`) | 0.12 |
| Hashing | `sha2` | 0.10 |
| Randomness | `rand_core` (`getrandom`, explicit dep) | 0.6 |
| Secret wiping | `zeroize` | 1 |

## Construction

Seal: fresh ephemeral `X25519` → `ECDH` with recipient agreement key (zero
peer key refused up front) →
`HKDF-SHA256(salt = ephemeral_pub, info = "terichat-dm-v2" || sender_verify
|| recipient_agreement)` → 32-byte key → `XChaCha20-Poly1305` with fresh
24-byte nonce → sign `sender_verify || recipient_agreement || ephemeral_pub
|| nonce || ciphertext` with sender `Ed25519`. Long-term secrets are drawn
independently per group and wiped with `zeroize` when dropped.

Wire: `ephemeral_pub[32] || nonce[24] || signature[64] || ciphertext`.

## Guarantees and gaps

- Confidentiality + authenticity against network and honest-but-curious
  server attackers; wrong-recipient, tampering, and forgery all fail closed
  (see unit tests).
- Gaps (all recorded in `docs/THREAT_MODEL.md`): no forward secrecy, no
  deniability, no groups, no key transparency, no replay protection beyond
  server sequencing, classical-only suites.
