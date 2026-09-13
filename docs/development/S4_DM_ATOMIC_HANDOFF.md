# Task Handoff — S4-DM-ATOMIC

- Task: P0 S4-DM-ATOMIC, atomic group/DM creation and concurrent pair serialization.
- Parent controller: `01a0802a-43a1-7271-957a-7d613dd63b8d`.
- Worker task: `01a09a35-206a-7a41-ba11-1cf2521ff51f`.
- Branch: `teriri/s4-dm-atomic`.
- Dedicated worktree: `C:\Users\TeRiRi\.codex\worktrees\bbe6\TeriChat(tentative)`.
- Fetched and verified base: `3d6232a62ed610227a5b2a1e8842533fd9fe0e02` (origin/main and initial HEAD matched before editing).
- Test-only baseline: `b8b5019f6ae4c21668546818f1715cafc8444146`.
- Implementation: `7db3db94b88223457b3511201c97043a1d37bca0`.
- Final frozen/reviewed source head: `694241413d0b1e8bfeda8fe1908babd29d238b05`; following handoff commit contains documentation/evidence only. Parent receives that exact final commit separately.
- Risk: **High**; no auth/access-policy, crypto protocol, schema, dependency, or deployment changes.
- State: ready for parent review and hosted execution; runtime acceptance **unverified**, not Done or merge-ready.

## Delivered

`apps/server/src/messaging.rs` now commits the conversation and all participants in one transaction. `find_or_create_dm` takes a transaction-scoped PostgreSQL advisory lock derived from a domain-separated hash of canonical unordered UUID bytes, then uses a separate READ COMMITTED lookup. All lookup, membership read, and creation statements use the same acquired connection, avoiding nested acquisition when the production pool has five connections. Hash collisions only serialize unrelated pairs; lookup still uses complete UUIDs.

`apps/server/tests/conversation_creation.rs` adds seven mandatory synthetic PostgreSQL tests. A separate observer holds a SHARE table lock that permits the old lookup but blocks INSERT, and observes five actual database lock waits before releasing five HTTP requests. This produces a deterministic baseline race without sleeps pretending to establish overlap. The fixed leader blocks on insertion while followers wait on its transaction lock. Further tests cover FK rollback, cancellation, exact participant sets, self-DM/group compatibility, and retained duplicate histories. Missing DATABASE_URL fails explicitly rather than reporting a skip as a pass.

Other changed paths are this handoff and `docs/development/evidence/s4-dm-atomic/*`. No manifests, lockfiles, routes, health files, migrations, or shared policy files changed.

## Acceptance criteria

| Criterion | Met? | Evidence |
|---|---|---|
| Same/reversed pair requests return one DM with exactly two intended members | Unverified at runtime | Two five-request HTTP races; independent frozen-source review |
| Failed later participant insertion leaves no conversation/participants | Unverified at runtime | FK regression for group/dm/channel and missing DM peer |
| Cancellation rolls back and allows pair waiter progress | Unverified at runtime | Cancelled group insertion and DM leader/waiter tests |
| Pool of five does not deadlock through nested acquisition | Source-reviewed; runtime unverified | One transaction connection, five-request barriers |
| Reopen, sends/idempotency/history/outbox visibility and outsider denial stay compatible | Unverified at runtime | Race/group compatibility assertions and existing gateway/HTTP suites compiled |
| Existing self-DM and group response policy preserved | Source-reviewed; runtime unverified | New singleton self-DM on each call, original duplicate-input group response vector |
| Existing records/messages are preserved; no consolidation/migration | Source-reviewed; runtime unverified | Historical duplicate fixture and retained per-conversation history assertions |
| Independent investigation then different fresh candidate review | Yes | [Review evidence](evidence/s4-dm-atomic/review.md) |

## Verification

Windows toolchain: cargo 1.97.1, rustc 1.97.1. Commands use the committed dependency lockfile where applicable.

| Actual command | Revision | Result | Evidence |
|---|---|---|---|
| `git fetch origin main`; `git rev-parse HEAD origin/main` | initial base | PASS: both `3d6232a...` | Task transcript |
| `cargo fmt --all -- --check` | `7db3db9...` | PASS | [fmt.log](evidence/s4-dm-atomic/fmt.log) |
| `cargo check --workspace --locked` | `7db3db9...` | PASS | [check.log](evidence/s4-dm-atomic/check.log) |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | `7db3db9...` | FAIL: missing documentation backticks around PostgreSQL | [clippy.log](evidence/s4-dm-atomic/clippy.log) |
| Same Clippy command after one-line correction | `6942414...` | PASS | [clippy-fixed.log](evidence/s4-dm-atomic/clippy-fixed.log) |
| `cargo fmt --all -- --check` | `6942414...` | PASS | [fmt-final.log](evidence/s4-dm-atomic/fmt-final.log) |
| `cargo test --workspace --no-run --locked` | `6942414...` | PASS: all workspace test binaries compiled | [test-compile.log](evidence/s4-dm-atomic/test-compile.log) |
| `cargo test --workspace --doc --locked` | `6942414...` | Command PASS; zero doctests defined, no behavior coverage claimed | [doctests.log](evidence/s4-dm-atomic/doctests.log) |
| `git diff --check 3d6232a62ed610227a5b2a1e8842533fd9fe0e02 HEAD` | `6942414...` | PASS | Task transcript |

An earlier `cargo test -p terichat-server --test conversation_creation --no-run --locked` also completed successfully while dependency compilation overlapped author edits; it is not used as exact-revision evidence. The later frozen-source workspace test compilation above supersedes it.

## Remaining findings and constraints

- S4-DM-01/S4-DM-02 were independently source-confirmed S2 acceptance blockers. Code corrections and regressions are reviewed; actual red/green execution remains required before closing them. The different fresh reviewer found no actionable candidate defects.
- Local PostgreSQL/Docker unavailable by task input. No repair, install, staging, production, or database runtime attempt was made. No skipped test is counted as a pass.
- Parent reported dispatching the exact test-only baseline on `teriri/s4-dm-regression-proof`. Its baseline lane can hit the preserved doc lint error; the independent `db-tests` lane runs `cargo test` directly and must supply actual regression failures. At author handoff those logs/results were not yet supplied.
- PostgreSQL integration execution, complete workspace runtime suite, gateway runtime compatibility, ARM64 verification, hosted candidate CI, and current-base integration evidence: **NOT RUN locally / parent-owned**.
- Dependency/secret scanner checks: **NOT RUN**; cargo-deny, cargo-nextest, and gitleaks were not found in local command discovery; no new dependencies or secrets introduced. Manual changed-source inspection does not replace a scanner.
- Frontend runtime checks: not run; no frontend changes. No policy gate, PR approval, merge, or deployment result claimed.

## Compatibility and operations

- No migration, response-shape, permission, configuration, key-state, or persisted-ID changes. Existing duplicate DMs/messages are retained. Self-DM retains the existing new-singleton behavior; deduplicating it would be a separate policy/behavior task.
- Advisory coordination requires all concurrent DM endpoint writers to use the repaired helper. Old binaries and direct raw `create_conversation(..., "dm", ...)` calls are not serialized through it. There are no such production raw-DM callsites in the inspected source. An owner-controlled rollout must avoid mixed old/new writers if it needs the invariant during transition.
- One transaction connection per operation. Pair waiters retain connections until lock acquisition/completion or cleanup; this does not promise that contention cannot temporarily consume pool capacity.
- SQLx queues rollback on transaction drop. Cancelling the Rust future does not synchronously cancel a blocked PostgreSQL statement; cleanup finishes when it can progress. Tests release external blockers before asserting eventual cleanup.
- Rolling back the source requires no schema rollback but restores the reported race/partial-write defects. Do not delete/consolidate existing records as rollback or cleanup.

## Handoff

Source is frozen. Parent owns push, PR creation, CI/comment triage, current-base validation, authorized integration, and any rollout. No push/PR/merge/cleanup performed by this worker. Shared-file ownership remains untouched.

Next unblocked task: parent retrieves real baseline PostgreSQL failures, tests the reviewed final candidate in hosted PostgreSQL CI, resolves any concrete findings, and records exact current-base acceptance before integration. New implementation work here requires an explicit resumed task and refreshed review/evidence.
