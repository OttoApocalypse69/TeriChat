# `tericrypt` — TeriCrypt-4096 device crypto (pre-alpha)

## Opt-in MLS foundation — issue #6, first slice only

`--features mls-foundation` exposes `tericrypt::mls`, an isolated **synthetic,
client-local experiment**. The feature is off by default. Server and desktop
messaging still use the existing PoC path; no wire migration, fallback, or
production E2EE claim is introduced. This slice does **not** close issue #6.

### Implemented and tested locally

- OpenMLS MLS 1.0 group creation, signed KeyPackages, Welcome joins, opaque
  serialized application messages, add/remove commits and epoch transitions.
- Only `MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519`; valid alternate-suite
  KeyPackages/Welcome messages are refused. OpenMLS performs all protocol crypto.
- Independent provider, signing key, random 32-byte scoped credential label,
  and volatile store per installation. A harness maps four independent members
  to two synthetic users; no global account UUID is included in a credential.
- Removed-device inability to decrypt the next epoch, including when its removal
  notice is withheld; surviving devices continue decrypting.
- Malformed, trailing, tampered, replayed, foreign-group, foreign-Welcome,
  unsupported-suite and legacy-PoC input rejection. Application sends are capped
  locally at 64 KiB; this is not a server framing/size contract.

### Explicit non-guarantees / stopped production slices

- `OpenMlsRustCrypto` uses upstream **MemoryStorage**, not a database or secure
  vault. There is no durable save/restart/reload, encrypted storage, transaction,
  crash atomicity, anti-rollback, key backup, or secure-erasure claim. Public MLS
  artifacts may leave the client; provider/signer/group secrets are not exported.
- On processing error, the session reconciles its live ratchet with the provider's
  *latest volatile* storage state using upstream `MlsGroup::load`. OpenMLS 0.9.0
  can mutate the live ratchet before an AEAD failure without storing that mutation
  (`group/mls_group/processing.rs`, decrypt before `write_message_secrets`). This
  is not restoration of an old durable backup. Tests verify valid retransmission,
  continued sending, and non-resurrection of successfully consumed messages.
  This is **not** a general all-errors-are-transactional guarantee.
- Random BasicCredential labels are **not trusted identities**. No account/device
  authentication-service mapping, ownership proof, credential verification policy,
  bootstrap approval, QR ceremony, key transparency, or recovery exists. Production
  credential trust and storage/bootstrap remain blocked pending their proper design
  and review; no new trust decision is silently selected here.
- A valid group peer's commit is merged without application-level role/authorization
  policy. Only directly supplied, synthetic trusted fixtures may use this API.
  A random label alone does not provide unlinkability or metadata privacy.
- One installation is consumed into one group scope; join consumes it even on
  error. Package cleanup/retry/replenishment, multi-group installation lifecycle,
  transport ordering/forks, external proposals/commits, ingress resource limits,
  and atomic send-state/outbound-delivery coordination are not production-ready.
  Add/remove merges locally before the harness delivers the returned commit.
- Loss of the in-memory state loses membership. Revocation is an epoch boundary,
  not retroactive erasure of retained keys/plaintext. No PQ, forward-secrecy,
  post-compromise recovery, or audited production security claim is made by these
  tests. Specialist review is still required.

### Dependencies and provenance

The published compatible family was checked with `cargo info`, then against the
registry source manifests and the upstream `tests/book_code.rs` examples. Not Git
snapshots or guessed API versions. All four direct versions are exact-pinned:

| Component | Version | SPDX license |
|---|---|---|
| `openmls` | 0.9.0 | MIT |
| `openmls_rust_crypto` | 0.6.0 | MIT |
| `openmls_basic_credential` | 0.6.0 | MIT |
| `openmls_traits` | 0.6.0 | MIT |
| `openmls_memory_storage` (transitive) | 0.6.0 | MIT |
| `openmls_serialization_helpers` (transitive) | 0.1.0 | MIT |
| `tls_codec` / `tls_codec_derive` (transitive) | 0.5.0 | Apache-2.0 OR MIT |
| `hpke-rs`, `hpke-rs-crypto`, `hpke-rs-rust-crypto` (transitive) | 0.7.0 | MPL-2.0 |

Sources: [OpenMLS 0.9.0](https://crates.io/crates/openmls/0.9.0),
[RustCrypto provider 0.6.0](https://crates.io/crates/openmls_rust_crypto/0.6.0),
[basic credential 0.6.0](https://crates.io/crates/openmls_basic_credential/0.6.0),
[traits 0.6.0](https://crates.io/crates/openmls_traits/0.6.0),
[versioned API docs](https://docs.rs/openmls/0.9.0/openmls/).
OpenMLS-family MSRV is 1.91.0; tested with rustc/cargo 1.97.1 on Windows x86_64.
ARM64, Linux, WASM and mobile are not validated by this slice.

`evidence/dependencies.json` records the Cargo-resolved dependency graph and
licenses (including optional/target graph entries); `provider-tree.log` is the
active normal/build tree on the tested host. The upstream provider enables HPKE
`experimental`/`hazmat` features and brings PQ implementations/libcrux SHA3
transitively. This does **not** select a PQ MLS suite or replace `HpkeRustCrypto`
for the locked X25519 suite; OpenMLS draft/test/debug features are not enabled.
Transitive MPL-2.0 and other license obligations need release/legal review;
project `UNLICENSED`/private status is unchanged.

`cargo audit` is **not green**: the root lockfile retains the baseline
`rsa 0.9.10` advisory RUSTSEC-2023-0071 and adds the unmaintained
`proc-macro-error2 2.0.1` warning RUSTSEC-2026-0173 via the upstream all-target
`hpke-rs -> libcrux-sha3 -> hax-lib -> hax-lib-macros` graph. The latter does not
appear in the active Windows build tree, but must be reviewed for other targets.
No advisory was ignored or dependency policy waived.

### Verification

```sh
cargo test --locked -p tericrypt --all-features
cargo test --locked -p tericrypt --no-default-features
cargo test --locked -p tericrypt --all-features --doc
cargo fmt -p tericrypt -- --check
cargo check --locked --workspace --all-targets --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo audit --file Cargo.lock
```

See `evidence/HANDOFF.md`, real command logs and JSON exit statuses. The evidence
runner never prints key material or environment configuration. Formatting has
no Cargo `--locked` switch; compilation/tests/lints use the committed resolution.

## Existing sealed-DM PoC (unchanged compatibility path)

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
