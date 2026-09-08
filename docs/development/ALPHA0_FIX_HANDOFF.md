# Alpha 0 audit repair handoff

- Task and issues: CLIENT-01 through CLIENT-04, GATEWAY-01, STATS-01, STAGING-01; affected acceptance #5, #7, #9.
- Human/parent thread: owner requested fixes after the read-only audit; coordinator owns integration and verification.
- Base SHA: `c325b4c90df5087a6f10ff6b5a2bc24787a88ec1`.
- Risk: Critical account-boundary repair; High replay/concurrency and staging operations.
- State: fixes integrated locally on `docs/alpha0-status`; parent verification passed and recorded review findings are addressed. GitHub CI/native/live operational acceptance remain pending. Not deployed.

## Parent verification of integrated candidate

The documentation worktree now also contains the three implementation tracks; its branch name is historical. Source remains uncommitted over the base SHA above. The original writer trees remain preserved.

Actual parent executions on this combined source:

- `cargo fmt --all -- --check`, locked workspace Clippy with `-D warnings`: PASS.
- `cargo test --workspace --locked`: 73 server and 13 crypto tests PASS, both on the retained synthetic PostgreSQL 17 service and again on a newly provisioned PostgreSQL 16 service matching deployment/CI.
- `cargo test --workspace --doc --locked`: command PASS; zero doctests exist.
- `cargo build --workspace --locked`: PASS.
- `npm ci --ignore-scripts`, `npm test`, `npm run build` in desktop: PASS; 66 tests including 20 mounted regressions; no reported dependency vulnerabilities during install.
- `python deploy/staging/tests/test_ready.py`: PASS.
- `bash deploy/staging/tests/backup_test.sh`: PASS, including real synthetic PostgreSQL dump/restore and expected failure-path cleanup.
- Actual Caddy runtime via an isolated local container: `ready_runtime.py` PASS for backend 503/200 preservation, `/v1` proxy, and SPA fallback. Container had `--network none` and a read-only repository mount. This resolves the writer's missing-Caddy blocker, not live TLS/ARM64/off-machine acceptance.
- `git diff --check`: PASS.

A deliberate `env -u DATABASE_URL cargo test --locked -p terichat-server stats::tests::duplicate_delivery_is_idempotent -- --nocapture` failed with the required database prerequisite. The existing baseline CI job would therefore fail. The coordinator added a synthetic PostgreSQL 16 service and DATABASE_URL to that job, preserving its full test step rather than weakening the new regressions. Workflow execution on GitHub is still NOT RUN for this uncommitted candidate; workflow change needs Critical review.

Staging runtime test image: `terichat-ready-check:local`, assembled from official Caddy and Python Alpine images. Parent database containers `terichat-fix-backend` and `terichat-fix-pg16` are disposable local synthetic services, not staging/production.

No native Tauri rebuild, packaged two-client smoke, live deployment, or real off-machine backup/restore has been performed.

## Review dispositions

Focused independent follow-up review (`deleg_4acdbe09`) found no remaining findings in the CI prerequisite, exact-volume cleanup, or combined gateway lower-UUID delivery scope. It confirmed the additive workflow preserves the unfiltered suite, cleanup targets only the recorded fixture volume, and the reversed-commit regression asserts delivery. This was source review plus Bash syntax checking, not a new database run or GitHub CI result. Desktop duplicate-retry repair remains outside that verdict and pending.

- Client duplicate retry: confirmed S2, now fixed with optional `onDuplicateEvent` while retaining deduplicated `onEvent` semantics. Transport cache is bounded to 1,024 IDs. Parent inspected the actual gateway diff and mounted real-GatewayClient/socket tests (duplicates after failure and during deferred failure), then reran all 69 frontend tests and the TypeScript/web build successfully. The mutation-verifier warning referred to an earlier failed patch: the final file diff confirms the change is present. Evidence: `apps/desktop/evidence/GATEWAY_DEDUP_S2.md`.
- Reported lower-UUID live suppression: the desktop reviewer inspected the old backend in its desktop-only worktree. Combined `gateway.rs` uses `seen.contains(&entry.id)` instead of the old UUID high-water comparison; the service-enabled reversed-commit test passed. Preserve the separate documented UUID-only reconnect limitation, which requires the combined client's reconnect-wide history reconciliation.
- Baseline CI prerequisite: confirmed and corrected by providing a synthetic PostgreSQL 16 service; full tests preserved. Fresh local PG16 suite passed; GitHub workflow execution remains pending.
- Backup fixture volume leak: confirmed via a new exact-volume cleanup assertion. `backup_test.sh` exited 1 after otherwise passing with the original `docker rm -f`. Changed to `docker rm -fv`; repeated full backup/restore/failure suite and syntax/diff checks passed, including absence of the test-owned volume. The exact volume created for RED was explicitly removed; no broad volume pruning was performed. Historical unidentified volumes were not deleted.

## Scoped writers

| Writer / branch | Exclusive ownership | Required evidence |
|---|---|---|
| `fix/alpha0-client` | `apps/desktop/**`, including desktop test dependencies/lockfile | Mounted stale-account response regression; history pagination/concurrent arrival; failed-fetch reconnect recovery; draft/focus preservation; frontend tests/typecheck/build |
| `fix/alpha0-replay-stats` | `apps/server/src/**`; additive migration `20260908090000` only if necessary | Reversed-commit Stats regression; multi-page replay/live/visibility behavior; workspace Stats isolation; service-enabled full Rust suite, format/check/Clippy/doctests |
| `fix/alpha0-staging` | `deploy/staging/**` and tests under it | Readiness routing; accurate shared-edge instructions; encrypted backup failure/success handling and synthetic recovery verification where supported |
| `docs/alpha0-status` | Entry docs and evidence/status documents | Parent reconciliation, diff/link validation, exact commands and remaining blockers |

Each writer has a separate sibling worktree based on the exact SHA above. No writer may switch branches, commit, push, deploy, access production data, read credential files, or modify another writer's paths. Root manifests and protocol changes require coordinator agreement. Regression failures must precede fixes; failed required checks are not waived.

## Acceptance and current evidence

The table below preserves the original task-start criteria, not current pass counts. Parent verification and review dispositions above are the current evidence. The baseline tests in `IMPLEMENTATION_STATUS.md` describe the buggy baseline and must not be reused as proof of these repairs.

| Criterion | Met? | Evidence |
|---|---|---|
| Old-account async results cannot enter a new account's state | Unverified | Writer must supply failing-then-passing mounted regressions, including sibling paths |
| Paginated recovery preserves gaps despite live/send responses | Unverified | Writer must supply multi-page and concurrent-arrival regressions |
| Failed history is recovered without another new message | Unverified | Reconnect recovery regression required |
| Incoming/background updates preserve unsent draft and focus | Unverified | Mounted regression required |
| Replay delivers beyond the first page without visibility bypass | Unverified | Real service-enabled gateway regression required |
| Stats cannot permanently miss a lower-ID late commit | Unverified | Deterministic separate transactions required |
| Staging readiness returns backend readiness, not SPA content | Unverified | Executable proxy regression required |
| Off-machine backup is encrypted and recoverable | Unverified | Synthetic tooling tests are not a performed live/off-machine restore drill |

## Verification and review gates

1. Parent re-runs each writer's checks and inspects the actual diff, including all new files.
2. Review verified changes against trusted base policy, with security/account-boundary and concurrency/test-gap attention.
3. Integrate sequentially in an isolated checkout; run combined service-enabled tests and client build/tests.
4. Record exact candidate revision/tree and evidence. No auto-merge or live deployment is authorized by this task.

## Compatibility and operations

Preserve the current demo-plaintext label and existing API behavior unless a reviewed compatibility contract requires a change. No MLS/recovery implementation is included in this repair slice. Do not rewrite shipped migrations. Backup encryption must document changed output/restore commands and must not silently delete earlier backups. The existing SPICE shared TLS edge is not changed.

A performed ARM64 fresh-host drill, live off-machine backup/restore, native two-client smoke, and production readiness remain separate acceptance work unless actually exercised and recorded. Never describe synthetic fixtures or mocked processes as those operational results.

## Handoff

Local fixes and regression checks are complete for the recorded defects. GitHub CI on the final candidate, native packaged smoke, and real off-machine operational acceptance remain unrun. No issue closure, commit, push, merge, or deployment has occurred.
