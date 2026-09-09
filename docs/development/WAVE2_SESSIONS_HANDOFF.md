# W2-SESSIONS handoff

- Task: W2-SESSIONS; parent backend wave 2.
- Worker: sessions_resume, resuming the interrupted session worker's partial source.
- Branch/worktree: `teriri/backend-sessions`, `C:/Users/TeRiRi/Documents/GitHub/TeriChat-wave2-sessions`.
- Base: `19a95155ac4941f597aab698f0d76f894c99ac77`.
- Immutable source head: `742680719ebfa38065c198cf6eb47f9fe2030498` (production implementation `23ba4f588181f46b52ec4e37395827b28950066f`, followed by isolated test fixtures).
- Risk: Critical, authentication/session enforcement.
- State: ready for independent review; database execution remains blocked locally.

## Delivered

`apps/server/src/session_management.rs`: GET `/v1/auth/sessions` returns only the caller's unexpired, unrevoked sessions, UUID-keyset pagination, default/capped limit 100 and positive-limit validation. Only id, device_id, created_at, expires_at and is_current are serialized. DELETE `/v1/auth/sessions/{id}` revokes an owned session; already-revoked owned targets succeed while the caller is live; foreign/unknown targets return the same 404. Self-revocation succeeds once and subsequent authenticated requests fail.

Inventory protects the caller row through its read. Revocation locks caller and owned target in UUID order, then checks caller validity using fresh statement time after waiting. Reciprocal revocations therefore have one winner; expired/revoked callers cannot mutate a second session after waiting.

`apps/server/src/gateway.rs`: session validity is checked after upgrade during identification, before Ready/replayed/live events and inbound application responses, plus one-second validity ticks. Database errors/timeouts fail closed. Pending replay queries and their five-second timeout are pinned across heartbeats and ticks rather than restarted. Idle invalid sessions close without client traffic.

## Acceptance criteria

| Criterion | Met? | Evidence |
|---|---|---|
| Own live sessions, bounded paging, safe fields | Implemented; DB execution unverified | `session_routes_ownership_pagination_and_revocation`, `session_inventory_caps_and_validates_pages` |
| Ownership, idempotency, self-revocation | Implemented; DB execution unverified | Route test covers foreign/unknown 404, duplicate 204 and post-self-revoke 401 |
| Lock-order safety and caller validity after wait | Implemented; DB execution unverified | `reciprocal_session_revocation_has_one_winner`, expiry-during-revoke and expiry-during-inventory tests |
| Gateway rejects revoked/expired sessions beyond upgrade | Implemented; DB execution unverified | Three real-WebSocket tests cover pre-identify, idle, heartbeat, live delivery and blocked replay |
| Replay progress during ticks/heartbeats | Implemented; DB execution unverified | Delayed replay checks unchanged query PID/start time after heartbeat and 1.2 seconds; valid release delivers an event; revoked session closes before lock release |
| Parallel-test isolation | Implemented; DB execution unverified | Per-test PostgreSQL schema/search_path/application_name; wait observations additionally match actual blocker PID |

## Verification

Native Windows Rust 1.97.1. Actual command results are preserved in this task's tool transcript.

| Actual command | Revision | Result | Evidence |
|---|---|---|---|
| `cargo fmt --all -- --check` | 7426807 | PASS | Tool chunk `617e7b`, no format diagnostics |
| `cargo check --workspace --locked` | 7426807 | PASS | Tool chunk `617e7b` |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | source tree committed as 7426807 | PASS | Tool chunk `da5885` |
| `cargo test --workspace --locked --no-run` | 7426807 | PASS; compiled only | Tool chunk `617e7b`, both test executables produced |
| `cargo test --workspace --doc --locked` | source tree committed as 23ba4f5 | PASS, zero doctests | Tool chunk `45d4dd`; later change only test-fixture isolation |
| `git diff --check` | source tree committed as 7426807 | PASS | Tool chunk `da5885` |
| Database/route/WebSocket tests | 7426807 | NOT RUN locally | Docker unavailable; parent owns repair and CI PostgreSQL run |
| Failing-baseline runtime regression | base 19a95155 | NOT RUN | Same DB blocker; do not claim red/green proof |
| Dependency/secret scanning, full supported platform matrix | 7426807 | NOT RUN by worker | Parent integration owns shared checks |

Eight new database tests are compiled, not claimed as passed. A test without DATABASE_URL explicitly reports skipped; no such skip is treated as database evidence. Tests create disposable schemas and require CREATE SCHEMA privileges in the synthetic test database.

## Remaining findings and constraints

Adversarial review identified periodic ticks canceling replay reads in the initial uncommitted implementation. The pinned pending read fixes both that defect and pre-existing heartbeat cancellation. The final delayed-read test covers this behavior, pending runtime verification.

Review also identified global-table locks and broad pg_stat_activity matching in the first committed test implementation. Commit 7426807 isolates each fixture and correlates wait evidence to its actual blocker, preserving parallel CI execution. No deterministic deadlock reproduction was claimed.

Preserved authoring failures: first test compile failed because OutboxEntry does not implement sqlx::FromRow (repaired with tuple decoding); subsequent Clippy flagged an oversized test (split into separate behavior tests). A PowerShell editing attempt failed to locate a newline-sensitive substring; its partial insertion was inspected and corrected. Final compile/lint results are recorded separately above.

Fresh independent review and real PostgreSQL results remain required. The parent integrates reviewer dispositions on the current combined revision. Agent reports do not provide independent human approvals or merge authority.

## Compatibility and operations

No migrations, dependencies, frontend, account policy or cryptographic protocol changes. Existing routes are additive. Gateway authorization performs periodic/per-frame database reads; each validity query has a five-second timeout. One-second ticks are not a strict one-second disconnect SLA when other bounded database/socket work is in flight. Already in-flight frames cannot be retracted. Session revocation does not revoke devices, MLS membership, copied keys or already-received content.

Rollback reverts these source commits; database schema is unchanged. Rollback restores the prior upgrade-only gateway session enforcement gap. Test schemas remain in the disposable database until its normal teardown, preserving failed fixtures for diagnosis.

## Handoff

Parent owns route integration, combined testing, PR and current-review evidence. Source ownership is released for integration after this handoff. Next unblocked task: review exact combined source and execute the existing CI PostgreSQL lane; do not mark W2-SESSIONS Done solely from successful compilation.
