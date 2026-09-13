# Task Handoff

- Task and issue: S4-READINESS; P1 foundation reliability, no separate issue.
- Human/parent thread: 01a0802a-43a1-7271-957a-7d613dd63b8d.
- Worker ID: parent coordinator.
- Branch/worktree: teriri/s4-readiness; C:/Users/TeRiRi/Documents/GitHub/TeriChat-swarm-controller.
- Base SHA: 3d6232a62ed610227a5b2a1e8842533fd9fe0e02.
- Test-only baseline: 0ab78b35d9ba64450f879c9438d8e0ce9ecc59dd.
- Implementation SHA: ac7afacc70b0877c1b52c2d248366e397251c616.
- Risk: Normal; isolated readiness handler and tests, no auth policy change.
- State: in progress; hosted PostgreSQL acceptance and external review pending.

## Delivered

`apps/server/src/health.rs` bounds the complete database ping, including pool acquisition, to two seconds. Query failures and deadline expiry return the same HTTP 503 JSON error: code `unavailable`, message `database unavailable`. Public responses no longer include SQLx/network details.

`apps/server/tests/health.rs` adds actual-router regressions for a closed pool, a loopback TCP peer that never answers the database handshake, and a real PostgreSQL pool whose only lease is held and then released. The existing healthy, dependency-free liveness and no-database `not_configured` contracts are preserved.

## Acceptance criteria

| Criterion | Met? | Evidence |
|---|---|---|
| Driver details omitted from public failure | Yes locally | Exact safe-envelope regression failed baseline and passed implementation |
| Overall probe timeout includes acquisition | Yes locally | Real stalled TCP handshake: baseline watchdog failed at 4.01 s; fixed HTTP 503 in 2.13 s |
| Liveness and no-database readiness unchanged | Yes locally | Existing router tests passed |
| Exhausted real PostgreSQL pool returns bounded 503, then recovers | Unverified | Mandatory SQLx integration regression included for hosted CI |
| Already-established query execution stall | Unverified as a separate scenario | Same timeout surrounds the complete execution future; handshake/pool tests do not independently simulate a running query stall |
| Fresh independent final review | Yes | Different reviewer found no blocking findings on ac7afacc; source/router/driver cancellation and diff inspected |

## Verification

| Actual command | Revision | Result | Evidence |
|---|---|---|---|
| cargo test -p terichat-server --test health ready_closed_pool_returns_safe_error --locked | Test-only baseline | FAIL as intended; public driver details | Parent target/swarm-control/readiness-baseline-closed.log |
| cargo test -p terichat-server --test health ready_stalled_database_has_overall_deadline --locked | Test-only baseline | FAIL as intended; 4 s watchdog | Parent target/swarm-control/readiness-baseline-stalled.log |
| Same two commands | Implementation | PASS | Parent target/swarm-control/readiness-fixed-closed.log and readiness-fixed-stalled.log |
| cargo test -p terichat-server --test health ready_without_database_reports_not_configured --locked | Implementation | PASS | Parent command output |
| cargo test -p terichat-server --test health health_returns_ok --locked | Implementation | PASS | Parent command output |
| cargo clippy --workspace --all-targets --locked -- -D warnings | Implementation | PASS | Parent command output, completed in 1m16s |

cargo fmt --all -- --check and cargo check --workspace --locked passed. cargo test --workspace --doc --locked passed separately with zero doctests; output retained in parent target/swarm-control/readiness-doctests.log. Local PostgreSQL is unavailable. The mandatory SQLx test is not run locally and is not counted as a pass. Full PostgreSQL tests, exact merge candidate, and ARM64 verification require hosted CI. Existing RSA advisory remains unwaived; cargo-deny and dedicated secret scanner unavailable. No frontend source or dependency changes.

## Remaining findings and constraints

Pre-implementation independent review identified S2 missing total query deadline and S2 public driver detail. The implementation addresses both; a different fresh final reviewer found no blocking source defects. Real PostgreSQL saturation/recovery evidence remains a separate gate. Timeout cancellation drops the original request future; no background query task is spawned. Tests use only synthetic loopback/service fixtures. Reviewer limitation: SQLx 0.8.6 may retain an already-acquired connection in its background return-to-pool ping after query cancellation. This patch bounds HTTP latency, not every driver cleanup operation. Recovery from a silent established connection is not demonstrated; the included recovery test covers releasing an exhausted pool lease.

## Compatibility and operations

HTTP success shapes and no-database behavior are unchanged. Public failure text becomes a stable generic value. An unhealthy or saturated configured database returns 503 after two seconds instead of waiting for the pool's longer acquisition deadline or a potentially stalled query. This is a readiness signal, not a restart or deployment action. No schema, config, dependency, role or topology changes. Revert source to restore prior behavior, including prior exposure/latency defects; no data rollback required.

## Handoff

Parent owns PR, current-base CI, comment triage and eligible serial merge. Do not claim Done from local tests alone. Preserve the active controller worktree and ignored swarm-control state. The separate S4-DM worker owns messaging.rs; client PR48 owns its acceptance files/workflow.

