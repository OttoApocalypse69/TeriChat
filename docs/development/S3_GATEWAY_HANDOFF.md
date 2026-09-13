# S3-GATEWAY task handoff

- Task: S3-GATEWAY, remaining PR30 replay/control pool-starvation finding.
- Parent: `01a0802a-43a1-7271-957a-7d613dd63b8d`; parent owns PR, hosted validation, integration and cleanup.
- Worker: dedicated worktree `4cee/TeriChat(tentative)`.
- Branch: `teriri/s3-gateway`.
- Base: `05df96fa45221ae99308edcc2d33925ec0230cc6` (fetched origin/main).
- Regression/extraction revision: `d06e564a15496b588a83e6fe767a74b5b2cc3066`.
- Immutable reviewed source revision: `204cdb2723145f9812ffe046ffbae4e2275be2f9`.
- Risk: **Critical**, session-enforcement boundary.
- State/outcome: **blocked verification**. Implementation and two independent automated passes complete; PostgreSQL and dependency/secret-tool evidence remain unavailable locally. No push, PR or merge performed.

## Delivered

Changed `apps/server/src/gateway.rs` and this task-specific handoff only. No shared manifest, lockfile, migration, policy, production pool or account-policy changes.

The pinned replay used to stop polling while tick/incoming branches awaited a second pooled connection for session validation. Five pending reads could hold all five production connections, causing otherwise valid sockets to fail closed after acquisition/check timeout even after their queries became ready.

Both control branches now call `with_delivery_progress`, which keeps polling the original replay future during session checks and frame writes. It retains at most one completed delivery until control work finishes. Failed validation or read failure/deadline closes the pump; completed futures are not repolled. Event visibility and final session checks remain in the caller before event transmission. No background task, extra production connection, retry, or restarted replay deadline was added.

## Acceptance criteria

| Criterion | Met? | Evidence |
|---|---|---|
| Confirm reported path on current main | Yes, source | Pre-investigator independently confirmed the same pool dependency at base; production max is five in bootstrap.rs |
| Preserve replay polling/deadline during control work | Yes, focused local test | Deadline test fails on regression revision and passes on fixed source |
| Buffer completion without overriding validation denial | Yes, focused local test | `gateway_completed_replay_waits_for_control_validation`, valid and false controls |
| Deterministic production-sized PostgreSQL saturation progress | Unverified runtime | Three tests use max5 application pool, separate observer pool/schema, five observed blocked reads, barrier and validation-start channel before unlock |
| Revoked/expired denial under saturation | Unverified runtime | Separate revoked and expired saturation cases require `(false, false)` validation/delivery result |
| Existing replay, idle, live, heartbeat and revocation socket behavior | Unverified runtime | Existing socket regressions preserved; local fixtures skip without DATABASE_URL |
| No pool inflation or weaker auth | Yes, source | Production pool and session query unchanged |

The saturation regression targets the shared helper directly with real SQLx replay/session operations. It does not run five real sockets under saturation. Existing socket regressions exercise pump routing using the existing larger fixture pool. The post-review preserved this evidence limitation; no runtime result is claimed for either set.

## Verification

Local logs are under `target/s3-evidence/` in this worktree (ignored, not published). Tool outputs also remain in the task transcript. All source checks refer to the reviewed source tree unless otherwise stated.

| Actual command | Revision | Result | Evidence |
|---|---|---|---|
| `git fetch origin`, status/branch inspection | Base | PASS; clean detached start, scoped branch created | Task transcript |
| `docker info --format '{{.ServerVersion}}'` | Environment | BLOCKED, Docker engine pipe unavailable | Task transcript; parent instructed no repair/install/production substitute |
| `cargo test -p terichat-server --locked gateway_replay_deadline_interrupts_control_operation -- --nocapture` | d06e564 | Expected FAIL: 0 passed, 1 failed, `read deadline must remain polled` | `baseline-deadline.log` |
| Same deadline test | Fixed source | PASS: 1 test | `fixed-deadline.log` |
| `cargo test -p terichat-server --locked gateway_completed_replay_waits_for_control_validation -- --nocapture` | Fixed source | PASS: 1 test, both valid/denied cases | `buffered-validation.log` |
| `cargo fmt --all -- --check` | Fixed source | PASS | `format-candidate.log` |
| `git diff --check 05df96f HEAD` | Fixed source | PASS | Task transcript |
| `cargo check --workspace --locked` | Fixed source | PASS | `check-candidate.log` |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Fixed source | PASS | `clippy-candidate.log` |
| `cargo build --workspace --locked` | Fixed source | PASS | `build-candidate.log` |
| `cargo test --workspace --doc --locked` | Fixed source | PASS command; zero doctests present | `doctests.log` |
| `cargo test --workspace --locked -- --nocapture` | Intermediate fixed tree before final test-helper split/buffer test | FAIL/BLOCKED: 29 mandatory DB tests fail because DATABASE_URL is unset; older DB fixtures skip. 64 reported passes include skips and are not suite verification | `workspace-tests.log` |
| `cargo deny --version` | Environment | Unavailable; dependency audit NOT RUN | Task transcript |
| `Get-Command gitleaks -ErrorAction SilentlyContinue` | Environment | Tool unavailable; automated secret scan NOT RUN | Task transcript |

Initial strict Clippy exposed an oversized test setup helper (136 then 112 lines); it was split, without lint allowances. The failures are preserved in the transcript and `clippy.log`. No required assertion/test was removed or waived. Manual diff inspection found only synthetic fixtures and no credential/private-content additions; it is not an automated secret-scan result.

## Review findings and dispositions

### S3-GW-001

- Severity: pre-investigator classified S2 bounded availability defect; blocking because it is this assignment's acceptance criterion. Assignment retained Critical risk and the original P1 priority.
- Affected path/symbol: `apps/server/src/gateway.rs`, `next_delivery`, at base `05df96fa45221ae99308edcc2d33925ec0230cc6`.
- Claim/preconditions: five authenticated replay reads hold the max5 pool; a tick or incoming branch then waits for session validation while its retained read is not polled.
- Impact/failure path: bounded pool starvation and false session closure; read deadline cannot run during branch await. No confidentiality bypass identified.
- Confidence: high from source; PostgreSQL reproduction unavailable locally.
- Owner/disposition: S3 worker implemented narrow boundary fix; parent must obtain hosted runtime evidence before closure.
- Pre-review: fresh read-only `pre_review`, independently traced source, callers, resource ownership and acceptance design before patching. No edits or runtime tests.
- Post-review: different fresh read-only `post_review`, reviewed immutable `204cdb2` against base without author rationale/test-success claims. No concrete surviving bypass or regression found. Confirmed both branches, bounded buffering, fail-closed behavior and final authorization checks. Preserved helper-versus-five-real-socket coverage limitation above. No edits or runtime tests. These are automated evidence, not independent human GitHub approvals.

## Compatibility and operations

No schema, wire format, key lifecycle, dependency or configuration changes. Per pump, one delivery may be buffered during an already-bounded control operation. Idle/live and heartbeat processing retain their checks. Revert the scoped gateway change to roll back, but that restores the starvation defect; no migration rollback is needed.

## Parent handoff and next unblocked work

Run the baseline and candidate on the existing hosted PostgreSQL CI. For an unambiguous original-trigger comparison, select `gateway_production_pool_saturation_preserves_valid_replay` on both revisions; the baseline deadline regression also intentionally fails. The baseline revision is evidence-only, not an integration candidate. It includes behavior-preserving extraction to invoke the shared boundary with unfixed `operation.await` behavior.

Run all three saturation cases, existing gateway/socket regressions, full applicable workspace checks and available dependency/secret checks against the immutable final candidate. Preserve failed baseline and successful candidate logs separately. If hosted evidence exposes a defect, return it for scoped correction. Parent retains fresh integration review, PR/CI comment handling and owner-controlled serial merge authority. No next feature is claimed by this bounded workstream.
