# Backend wave 2 handoff

- Tasks: W2-SESSIONS, W2-STATS, W2-MODERATION, W2-MOD-RACES, W2-INTEGRATION.
- Parent: `01a0802a-43a1-7271-957a-7d613dd63b8d`.
- Branch/worktree: `teriri/backend-wave2`, `C:/Users/TeRiRi/Documents/GitHub/TeriChat-backend-wave2`.
- Base: `c1a6c90baf4c9eec44a352c92594b234c601cfc2`.
- Reviewed production source head: `a58b21701b95918f4f5cf581acc170153d9ae9c0`.
- Risk: Critical; session and workspace authorization. No policy files changed.
- State at this handoff: implemented and independently source-reviewed;
  expanded-candidate CI pending. Final exact-head evidence is maintained in
  [draft PR #30](https://github.com/OttoApocalypse69/TeriChat/pull/30).

## Delivered and acceptance

| Criterion | Implementation/evidence |
|---|---|
| Own-session inventory/revocation | `session_management.rs`: bounded private pages, idempotent owned revoke, lock-order and caller-expiry regressions |
| Persistent gateway enforcement | `gateway.rs`: session checks during idle/live/replay; pending query survives heartbeat/ticks; real WebSocket tests |
| Private workspace stats | `workspace_stats.rs`: current channels, caller-only counters, zero counts, keyset pages, revocation/expiry locks |
| Permission-gated ban list | Parent-authored `moderation.rs`: role matrix, bounded fields/pages, POST/DELETE compatibility, concurrent demotion denial |
| Existing moderation race fixes | `workspaces.rs`: locked-role permission checks for ban/kick/role change/unban; six deterministic regression/control tests |
| Integration | `main.rs`, `routes.rs`; contracts in `WAVE2_API.md`; worker handoffs and findings alongside this file |

The initial endpoint/gateway candidate passed real PostgreSQL CI (95 server +
13 crypto tests in each of two jobs). The six added mutation tests require the
expanded-candidate CI result; no local DB-skipped execution is claimed as proof.

## Commands and evidence

Native Windows x86_64 Rust 1.97.1, committed Cargo.lock unchanged. Local evidence
is under this worktree's ignored `target/wave2-evidence/`; worker-specific logs
are recorded in their handoffs. Preserve failures rather than rewriting them.

| Command | Source/result at handoff | Evidence |
|---|---|---|
| `cargo fmt --all -- --check` | PASS, including `a58b217` | Tool transcript |
| `cargo check --workspace --locked` | PASS earlier integrated source; all-target Clippy also compiles final source | `parent-check.log` |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS `a58b217` | `mod-races-clippy.log` |
| `cargo test --workspace --no-run --locked` | PASS endpoint/gateway source; worker compiles added tests | `final-test-compile.log`, worker evidence |
| `cargo build --workspace --locked` | PASS endpoint/gateway source | `integrated-build.log` |
| `cargo test --workspace --doc --locked` | PASS, zero doctests | `integrated-doc.log` |
| Existing GitHub CI run `34369990150` | PASS `2cfca6c`: baseline + db-tests, 95 server +13 crypto each | `ci-2cfca6c.log`, Actions run |
| Test-only baseline branch CI | Pending on `f3c6797`; five expected failing assertions, one successful control | PR #30 final evidence |
| Final expanded candidate CI | Pending; use exact final head on PR | PR #30 final evidence |
| `cargo audit --json` | FAIL existing rsa 0.9.10 / RUSTSEC-2023-0071; no waiver | `audit.json` |
| `cargo tree -i rsa --locked` | No active reverse tree printed for current target | Tool transcript; not an audit pass |

NOT RUN locally: PostgreSQL execution (Docker startup error), cargo-deny and
dedicated secret scanner (tools absent), ARM64 (trusted-main CI only), frontend
checks (no frontend edits). Existing CI uses its configured Rust toolchain and
PostgreSQL services; no workflow or failing test was weakened.

## Findings and operations

`WAVE2_REVIEW.md` preserves original findings, exact source, reproduction limits,
and fixes. Two independent subagents verified production source `a58b217`; their
reviews are not human approvals. RSA audit finding remains unresolved.

No migration, dependency, key format, gateway wire format, visibility, account
access policy, billing or production change. Added APIs are documented in
`WAVE2_API.md`. Stats are eventual metadata projections. Session checks add
database reads; one-second ticks are not a strict disconnect deadline while
other bounded work is in flight. No MLS/device/key revocation claim.

Rollback reverts these changes without data migration. That also restores the
old gateway and moderation authorization gaps; coordinate client fallback for
the added endpoints. No merge or deployment has been performed.

## Handoff

Parent owns final CI triage and PR evidence. Source ownership is released after
integration. Next unblocked step: finish runtime regression proof and current
candidate verification, then maintainer review/disposition of the dependency
audit and required merge gates. Do not mark Done based only on compilation.
