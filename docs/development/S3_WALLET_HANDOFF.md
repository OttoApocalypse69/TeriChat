# Task Handoff: S3-WALLET

- Task: caller-owned wallet history continuation; completed ledger #8 is not reopened.
- Parent: `01a0802a-43a1-7271-957a-7d613dd63b8d`.
- Worker: S3-WALLET, isolated worktree `6dab/TeriChat(tentative)`.
- Branch: `teriri/s3-wallet-history`.
- Base: `05df96fa45221ae99308edcc2d33925ec0230cc6` (fetched origin/main).
- Test-only baseline: `682d6e03e929e347e18418a846b74220287daa1e`.
- Immutable reviewed source: `3f18f15c59488030bf10f82a7cc4ae680349ee31`.
- Risk: Critical (ledger path floor).
- State: ready for parent review/hosted verification; not Done or merge-ready.

## Delivered

Only `apps/server/src/ledger.rs`, `docs/development/S3_WALLET_HISTORY_API.md`,
this handoff and `docs/development/evidence/s3-wallet/` change. Additive `before`
cursor on the existing authenticated history route. Existing entries envelope,
default 50, maximum 100, signed amounts and ledger writes remain unchanged.
Cursor anchor resolves only through caller-owned postings; exclusive
timestamp/UUID order supports ties with full database precision.

## Acceptance criteria

| Criterion | Met? | Evidence |
|---|---|---|
| Additive caller-owned continuation, stable exclusive ties, same API defaults | Implemented; runtime unverified | Source + API contract |
| Invalid/unknown/foreign cursor handling without metadata leak | Implemented; runtime unverified | HTTP and query tests |
| 205 entries, traversal at 1/37/100, bounded requests, empty termination | Tests compile; runtime unverified | `history_http_traverses_timestamp_ties_beyond_cap` |
| Shared transfers, signed perspective, replay and conserved balances | Tests compile; runtime unverified | New HTTP/query tests and unchanged ledger suite |
| Concurrent additions ahead/behind active cursor | Tests compile; runtime unverified | Query + HTTP live-boundary test |
| Isolated synthetic PostgreSQL schemas | Existing `isolated_pool` reused | No gateway schema changes |
| Initial adversarial + different fresh final reviewer | Yes | `evidence/s3-wallet/REVIEW.md` |

## Verification

Windows x86_64 MSVC, Rust/Cargo 1.97.1. All commands use the committed lockfile
where applicable. Logs are under `evidence/s3-wallet/`.

| Actual command | Revision | Result | Evidence |
|---|---|---|---|
| `cargo fmt --all -- --check` | reviewed source | PASS | `fmt-final.log` (empty success) |
| `cargo check --workspace --locked` | reviewed source | PASS | `check-final.log` |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | reviewed source | PASS | `clippy-fixed.log` |
| `cargo test --workspace --no-run --locked` | reviewed source | PASS, compilation only | `test-compile.log` |
| `cargo test --workspace --doc --locked` | reviewed source | PASS command, zero doctests in both crates | `doctests.log` |
| `git diff --check` | reviewed source | PASS | author and fresh reviewer |
| `cargo audit --json` | same unchanged Cargo.lock | FAIL: RUSTSEC-2023-0071, rsa 0.9.10 | `audit.json` |
| `cargo deny check` | reviewed source | BLOCKED: command not installed | `deny.log` |
| `docker info --format '{{.ServerVersion}}'` | local host | BLOCKED: Docker Linux-engine named pipe absent | observed tool output; no repair attempted |

Initial strict Clippy failed on a missing Markdown backtick in the newly added
doc comment; `clippy.log` preserves failure. Commit `3f18f15` corrected the
comment and strict Clippy passed. `check.log` and `fmt.log` retain initial
preflight evidence. No tests were weakened, skipped or silently retried.

## Remaining findings and constraints

No concrete blocking code defect found in automated final review. One
nonblocking overlap-test observation is preserved in REVIEW.md. The existing
dependency audit finding remains unwaived; no dependency changes are owned by
this task. Secret scanner is not installed/configured locally: NOT RUN.
Full PostgreSQL workspace/ledger HTTP execution is NOT RUN locally, not PASS.
Frontend checks and ARM64 runtime are NOT RUN (no frontend changes; hosted
platform validation belongs to the parent).

Parent reported pushing the test-only baseline to
`teriri/s3-wallet-regression-proof`, hosted run `34749285958`. Its result and
candidate runtime result were not observed by this worker. Parent must inspect
baseline failure and exact candidate green; this handoff supplies no runtime
success claim. Local tests fail hard if DATABASE_URL is absent.

## Compatibility and operations

No migrations, dependencies, configuration, crypto boundaries, grants, mint
routes or real-money semantics change. Each page is bounded to 100 entries;
cursor requests add a caller-scoped anchor lookup. This is live keyset history,
not snapshot history or commit-order completeness. Refresh for new entries
ahead of a cursor. No transaction update/delete API exists.

Rollback reverts this source change without database work; old servers ignore
`before`, so pagination clients must upgrade with the server or guard against
repeated pages. No shared-file claim needs release.

## Handoff

Parent owns separate PR creation, hosted database red/green proof, current-base
integration review, CI/comment triage and authorized serial merge/cleanup.
Next unblocked action: verify the baseline run and run the final source in the
existing hosted PostgreSQL lane. Author did not push/open PR/merge.
