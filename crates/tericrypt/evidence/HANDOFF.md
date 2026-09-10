# Issue #6 — bounded OpenMLS foundation handoff

- Parent: coordinating agent; independent review is explicitly still pending.
- Branch/worktree: `feat/6-mls-foundation`, `C:/Users/TeRiRi/Documents/GitHub/UnknownChat-mls-foundation`.
- Base/HEAD: `5cf0a590c0298be198eee7095053f0f736d150fb`; depends on unmerged PR23.
- Risk: Critical (cryptographic state). State: implementation ready for parent verification / independent review, **not merge-ready or issue-complete**.
- No branch operations, commits, pushes, deployments, real secrets or environment-file reads.

## Delivered

Non-default `mls-foundation` feature; independent memory-only installation and
single-scope group-session wrappers; exact locked suite; upstream KeyPackage /
Welcome / application / membership wire artifacts; typed safe errors and no PoC
fallback. Existing public PoC APIs, server code and root Cargo.toml are unchanged.
All source/evidence changes are in `crates/tericrypt/**` plus root `Cargo.lock`.
`manifest.json` enumerates every changed path and binds source files to SHA-256.

## Acceptance criteria

| Criterion | Status | Evidence |
|---|---|---|
| Two synthetic users, independent desktop/phone members, exchange in all directions | Met locally | `mls_foundation.rs`, final-tests.log |
| Add advances existing peers and newly joined installation to new epoch | Met locally | same |
| Remove advances surviving members; evicted member fails future decryption even without notice | Met locally | same |
| Malformed/foreign-group/tamper/replay/alternate-suite/legacy rejection | Met locally | `mls_rejection.rs`, final-tests.log |
| Server never needs plaintext | Local boundary only; no server migration/real E2E claim | Only wire artifacts passed between synthetic client states |
| Durable persistence/restart/reload | Not implemented, intentionally out of slice | README and module docs |
| Real scoped credential trust/device bootstrap | Not implemented; production slice stopped | README and module docs |
| Implemented vs pending security properties documented | Met | README + src/mls.rs |

## Verification and actual results

Commands are stored exactly in matching `.json` files. Final source-binding hashes
appear in `final-*.json`; earlier iterations retain their original logs/statuses.

- `cargo test --locked -p tericrypt --all-features`: PASS, 13 PoC + 3 membership + 7 rejection/state tests; none ignored. `final-tests.log`.
- `cargo test --locked -p tericrypt --no-default-features`: PASS, 13 compatibility tests. MLS tests deliberately absent with feature disabled. `final-default-tests.log`.
- `cargo test --locked -p tericrypt --all-features --doc`: PASS runner, **zero doctests present**, not positive behavioral coverage. `final-doctests.log`.
- `cargo check --locked --workspace --all-targets --all-features`: PASS. `final-check.log`.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: PASS. `final-clippy.log`.
- `cargo fmt --all -- --check`: PASS (fmt has no Cargo --locked option). `final-fmt.log`.
- `git diff --check`: PASS. `final-diff-check.log`.
- `cargo audit --file Cargo.lock`: **FAIL**, exit 1. `audit.log`; no exceptions/ignores.
- Cross-platform builds/native execution, real DB/UI E2E, automated secret scanner,
  fuzz/mutation campaigns, crash/restart tests, specialist review: **NOT RUN**.

### RED/GREEN history (preserved, not rewritten)

1. `red-1`: missing `tericrypt::mls` import (compile-red for the new interface;
   not a behavioral-red claim). `green-1`: four-device exchange passes.
2. `red-2`: runtime failure merging add commit (`InvalidInput`); `green-2`: valid
   staged commit merge implemented and two scenarios pass.
3. `red-3`: runtime failure in minimal remove stub; `green-3` records the first
   borrow-check failure; `green-3-fixed` passes after iterator lifetime correction.
4. `red-4`: runtime failures for oversize sends and corrupted-message ratchet
   consumption; three upstream validation characterizations already passed.
   `green-4-ratchet` proves the targeted ratchet fix; `green-4` passes full suite.
   Alternate-suite and additional send/replay preservation are regression
   characterizations of existing/upstream behavior, not claimed RED cycles.
5. `clippy-initial`: documentation/let-else warnings. Fixed without lint suppressions.
   Final logs reflect the additional send/replay regression and all required checks.

## Findings and dependencies requiring parent disposition

- **Audit not green**: unchanged baseline `rsa 0.9.10`, RUSTSEC-2023-0071 (no fixed
  version reported). Added locked `proc-macro-error2 2.0.1` unmaintained warning,
  RUSTSEC-2026-0173, through the upstream all-target hax/libcrux/HPKE graph. See
  `all-target-warning-tree.log`, `provider-tree.log`, `lock-changes.json`.
  No baseline package/version was removed by resolution. No speculative dependency
  downgrade or provider substitution was made to hide the warning.
- Current OpenMLS/provider/credential/traits releases: 0.9.0/0.6.0/0.6.0/0.6.0,
  all MIT, MSRV 1.91.0. TLS codec 0.5.0 is Apache-2.0 OR MIT; HPKE 0.7.0 is MPL-2.0.
  Full resolved licenses/features are in dependencies.json. This is not legal approval.
- Upstream provider enables HPKE experimental/hazmat and includes PQ/libcrux
  transitive code. The actual selected provider is RustCrypto (`HpkeRustCrypto`),
  and this API only accepts the locked classical suite. No MLS drafts enabled.
- A tampered ciphertext can consume the upstream live receive ratchet on AEAD
  failure before its storage write. Fixed locally by reloading only the latest
  provider MemoryStorage state on processing error. Tests prove authentic retry,
  continued sends and persistent replay rejection. This is NOT a durable rollback
  mechanism or general transaction guarantee. Independent reviewer should examine
  `Session::receive` carefully against OpenMLS 0.9.0 processing/storage semantics.
- No authenticated account-to-credential policy, application-level membership
  authorization, durable key store, rollback protection, bootstrap approval or
  production ingress limits. Do not wire this API to a production directory/UI.

## Compatibility and operations

No migrations/config changes/server/UI protocol changes. Feature disabled by default.
Root Cargo.toml unchanged. Existing lock versions retained; duplicate-version edges
are disambiguated and new MLS dependencies added. Rollback is removal of this isolated
feature and dependency additions, not a key-state migration. All MLS state here is
synthetic and non-durable. Tool output shows the host's preconfigured shared Cargo
build cache at `TeriChat-fix-backend/target`; no cache configuration was changed.

## Handoff / next task

Parent verifies `manifest.json`, hashes and commands, then commissions separate
independent Critical review. Review dependency advisories/license obligations and
ratchet failure semantics. Persistence/bootstrap/credential trust needs its own scoped
design and review before any production integration. Issue #6 remains open.

## Reusable verification lesson (kept within authorized paths)

For MLS integrations, test corrupted ciphertext followed by the original authentic
message, then repeated replays and a new send; merely asserting the corrupt message
fails can miss live-ratchet mutation and stale-state restoration. Inspect the exact
upstream version's order of decryption and storage writes. Never assume an in-memory
reload proves durable crash safety or invent a generic rollback scheme for ratchets.
