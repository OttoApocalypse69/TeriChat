# S3-CLIENT-VALIDATION handoff

**PR48 repair update:** the initial hosted campaign failed in the harness after several real scenarios passed. The repair and preserved failure are documented below; the initial verification table is historical. Current repaired full PostgreSQL acceptance is still pending.

- Task: S3-CLIENT-VALIDATION, real browser / API / PostgreSQL acceptance for S3 controls.
- Parent: 01a0802a-43a1-7271-957a-7d613dd63b8d; parent owns integration, push, PR and hosted execution.
- Worker branch/worktree: `teriri/s3-client-validation`, isolated `ce89` worktree.
- Trusted policy/origin base: `05df96fa45221ae99308edcc2d33925ec0230cc6`, fetched before branching.
- Frozen client dependency: `1223e60d1456f770754180f1834d9e3520103ad5`. Its implementation was reviewed at `f55fffdf60e9e2af3b90b2c53badef58e2124a23`; later commits contain documentation/evidence. Parent separately reviewed the final dependency.
- Validation implementation head: `ffc94f3e59f8e9e085ecce4e85bb8fb94d574a94`. The following handoff commit changes only this document; parent receives its exact SHA in the task message.
- Risk: **Critical**, because this adds executable CI. This document does not waive trusted-base review, current-candidate evidence or human owner approval.
- State: ready for parent review and hosted execution; **real acceptance remains UNVERIFIED**, not Done.

## Delivered and ownership

Only three new paths belong to this workstream:

- `apps/desktop/acceptance/s3-controls-real.mjs`
- `.github/workflows/client-controls-acceptance.yml`
- `docs/development/S3_CLIENT_VALIDATION_HANDOFF.md`

The harness takes exactly one synthetic loopback backend argument:

```text
node apps/desktop/acceptance/s3-controls-real.mjs http://127.0.0.1:3001
```

The backend must be disposable, have PostgreSQL configured, and run the candidate's compiled server. It applies embedded migrations and starts real outbox/Stats workers. No rollup table is seeded. Synthetic users, sessions, workspaces, channels and messages are created through actual HTTP endpoints. Data remains in the disposable database until the runner destroys it.

The shared client uses its existing plain-browser fetch path and Vite's loopback `/v1` HTTP/WebSocket proxy. No Tauri transport alias, Playwright route interception or fixture API responses are installed. One-shot proxy barriers hold **actual completed backend responses** (same status and bytes) until controls close or the destination account/workspace is visible. They then release the response and wait for browser completion and rendering before assertions. This deliberately controls response timing; it does not simulate the backend result.

The new workflow runs on GitHub-hosted Ubuntu 24.04 for PR, main push, merge-group and manual events, without path filters. It has read-only contents permissions, full action SHA pins, no persisted checkout credentials, no application secrets, no self-hosted/deployment environment, a synthetic PostgreSQL service, job/acceptance timeouts and cancellation of superseded runs. It runs locked frontend preflight/audit, Rust format/check/Clippy/tests/doctests/build, installs Chromium via the already committed `playwright-core` CLI and executes the harness against the compiled loopback server. It adds no dependency, manifest, lockfile or existing workflow changes.

## Acceptance coverage

These scenarios are implemented but their real hosted execution is still pending:

| Criterion | Harness assertion | Execution status |
|---|---|---|
| Browser sign-in and message persistence | Actual login, connected gateway, UI send returns 201; real projection reaches exact caller count | NOT RUN |
| Inventory current/other sessions | UI markers and nullable device label; API allowlist excludes bearer fields | NOT RUN |
| Revoke another session | UI removes row; revoked bearer gets 401; current bearer still succeeds | NOT RUN |
| Close controls during self revoke | Real DELETE 204 held while Close hides controls; delivery triggers logout and subsequent bearer 401 | NOT RUN |
| Private activity | Distinct caller counts in shared workspace, real zero channel, distinct second workspace; outsider 403 | NOT RUN |
| Paging | API `limit=1` session/channel pages, cursor progression, no duplicates and terminal null | NOT RUN |
| Workspace/account stale responses | Hold actual old summary; switch to distinct destination; release and assert correct destination count/absence | NOT RUN |
| Other-account session ownership | Other-account DELETE gets 404 | NOT RUN |

Pagination coverage here is the real API contract with small pages; the UI's 100-row Load more boundary remains covered by the client's component tests, not this harness. Account switching delays a real Stats response; session inventory is checked after switching, without a delayed inventory response. Only the Close-button self-revoke path is exercised here; client regressions additionally cover Escape and header toggle. Native transport/capabilities, OS notifications, installer behavior and encrypted-message/production crypto proof are outside this browser campaign.

## Verification actually run

All local execution used the frozen dependency plus the validation implementation bytes. No backend or Docker mutation was attempted. The final validation commit freezes those implementation bytes.

| Actual command/check | Result and evidence |
|---|---|
| `git status --short`, `git branch --show-current`, `git rev-parse HEAD` | Initial clean detached worktree at 05df96f |
| `git fetch origin`; `git switch -c teriri/s3-client-validation origin/main` | PASS; fetched main remained 05df96f |
| `git rebase 1223e60d1456f770754180f1834d9e3520103ad5` | PASS after parent supplied/authorized immutable dependency |
| `npm --prefix apps/desktop ci --ignore-scripts` | PASS: 201 packages installed; committed lock unchanged |
| `npm --prefix apps/desktop run typecheck` | PASS |
| `npm --prefix apps/desktop test` | PASS: 12 files, 137 tests |
| `npm --prefix apps/desktop run test:acceptance:unit` | PASS: 16 existing report tests; not tests of real harness execution |
| `npm --prefix apps/desktop run build` | PASS: 45 modules, JS 220.25 kB / gzip 67.23 kB |
| `npm --prefix apps/desktop audit --audit-level=low` | PASS: zero reported vulnerabilities |
| `cargo fmt --all -- --check` | PASS |
| `node --check apps/desktop/acceptance/s3-controls-real.mjs` | PASS |
| Python/PyYAML parse plus structural assertions | PASS: expected events, hosted runner, read-only permission, 40-character action pins; not actionlint |
| `git diff --check`; `git diff 1223e60 HEAD --check` | PASS |
| Python regex scan of the two implementation files for private-key/GitHub/OpenAI secret patterns | No matches; limited inspection, not full scanner certification |
| `node apps/desktop/acceptance/s3-controls-real.mjs http://127.0.0.1:1` | Expected exit 1, `result: FAIL`, empty steps, `failurePhase: startup`, cleanup PASS. This tests fail-closed reporting only, **not acceptance** |
| `git ls-remote` official checkout v4 / rust-toolchain stable / setup-node v4.4.0 / upload-artifact v4.6.2 | All four workflow pins matched the official refs observed in this task |

The fail-closed diagnostic is local ignored output at `apps/desktop/.acceptance/s3-controls-real/result.json`; it identifies the then-current dependency HEAD plus uncommitted harness, and must never be described as candidate acceptance. Local command outputs are retained in the task transcript. Hosted runs write an allowlisted JSON result at the same path with candidate HEAD/tree, npm lock digest, browser version, completed stages and failure phase. Only that file is uploaded for seven days. Raw bearer-bearing gateway URLs, headers, DOM, traces, backend logs and private data are not retained. Vite diagnostics are counted with fixed text because raw proxy errors may include query bearers.

Existing Vite config-loader/CommonJS and PostCSS module-type warnings appeared during tests/build and were not treated as failures or disabled. Some initial read-only searches used Windows globs that `rg` rejected; corrected directory searches were used. Discovery found no `rust-toolchain.toml`, actionlint, psql or gitleaks command; workflow Rust follows existing CI's 1.97 pin. No files outside ownership were changed.

**NOT RUN:** actual real-PostgreSQL browser acceptance, hosted workflow, local backend compile/check/Clippy/Rust tests/doctests, cargo-deny, full secret scanner, actionlint and native OS checks. Docker is unavailable per the parent's environment investigation; this worker did not repeat or bypass the blocked socket-removal repair, install local services, or use staging. Rust source is unchanged; relevant Rust checks are included in the hosted candidate job. Missing evidence is not a waived gate.

## Independent review

Read-only design investigator `/root/adversarial` examined the trusted base and existing contracts independently. Recommendations adopted: same-origin proxy without API fixtures, actual outbox counts, authoritative session fields, deterministic close/switch ordering, caller-private sentinels, pinned hosted isolation and credential-safe evidence. No baseline vulnerability was claimed.

Fresh reviewer `/root/final_review` reviewed immutable diff `1223e60..ffc94f3` under trusted policy `05df96f` for correctness, security, CI trust and test-gap review. **No blocking findings.** It confirmed hosted isolation, read-only permissions, pins, credential-free checkout, bounded execution, sanitized reporting, real-response barriers, selectors and API expectations. It independently passed diff whitespace, Node syntax and Vite transformation checks (the browser API base resolves to the forwarding origin). It explicitly retained the unrun real-PostgreSQL/hosted acceptance and browser Load more/native limitations. No edits were made by reviewers. Agent review is evidence, not distinct human/GitHub approval.

## Compatibility, rollback and next step

No migration, schema, API, product component, trust boundary, native capability, deployment or repository-settings changes. New CI consumes a bounded hosted job; it does not enable a required-check ruleset or a merge controller. Parent must review and execute the complete current integration candidate, check every required result, preserve failure evidence, and obtain Critical human merge authority. A PR-head pass is not proof of a different merged revision.

Parent should integrate the client dependency first (or construct the equivalent combined tree), then cherry-pick **only this worker's validation commit and handoff commit**. Do not duplicate the four inherited client commits. Rollback removes the two new executable files and this handoff; no persisted production data or migration is involved. The next unblocked task is parent fresh combined review and push/PR followed by actual hosted candidate acceptance. No push, PR, merge, deployment, release or local service installation was performed by this worker.

Official tool references used for requirements: [setup-node](https://github.com/actions/setup-node), [upload-artifact](https://github.com/actions/upload-artifact), and [Playwright browser installation](https://playwright.dev/docs/browsers). The pinned CLI installs its matching browser revision; no unpinned npx package is used.

## PR48 hosted timeout repair

- Repair base: `25694911e21236495b9e0d7ea389f4243c945a3c`.
- Repair implementation candidate: `a8a51b210a644a738abf74b9e411faa5bdd01e01`; subsequent handoff commit is documentation only.
- Branch remains `teriri/s3-client-validation`. Parent owns current-main integration, final combined revision, push/PR and hosted rerun. Owner merges advanced main during repair; this worker deliberately did not integrate them concurrently.
- Parent explicitly expanded ownership to new `apps/desktop/acceptance/s3-controls-runtime.mjs` and `apps/desktop/acceptance/s3-controls-real.test.mjs` for transport/deadline/report helpers and regression only. No manifest/lockfile, client or backend changes.
- Risk remains Critical. No timeout extension, blind retry or weakening of real acceptance assertions.

### Preserved failure and root cause

Hosted run `34749865370` failed the five-minute acceptance step (09:44:22–09:49:34 UTC). All preceding frontend/Rust preflight passed. Real synthetic setup, UI message/outbox counts, API paging, stale-workspace isolation and other-session revoke reached PASS; the last completed stage was logged at 09:44:30.798. There was no final JSON artifact because the harness had not exited its self-revoke stage.

The full original log remains at `C:/Users/TeRiRi/Documents/GitHub/TeriChat-swarm-controller/target/swarm-control/client-real-api-failure-34749865370.log`, SHA-256 `DFFCDBEE037FE7BFDFA20BA9F9071EF34C54216FD5CCA4830C85E9659326A75E`. It is parent-owned evidence, not overwritten or committed here.

Finding `S3-VALIDATION-HANG-01`, S2 (blocks this task's acceptance), owner S3-CLIENT-VALIDATION: baseline harness `release()` awaited `response.finished()` without a deadline. The self DELETE returns HTTP204; the browser client's `ApiClient.req` intentionally returns on 204 without reading a body. With installed Chromium and committed Playwright, that valid fetch resolves but Playwright's request-finished promise can remain unresolved. Thus self revoke reaches its real response but the harness never advances to logout/401 assertions or its final report. Reading a body on unrelated 200 responses explains why the earlier workspace barrier passed.

Fresh read-only investigator `/root/hang_investigator` reproduced the precise transport precondition using a real loopback HTTP server. Ignored `target/s3-hang-diagnostic/probe.mjs` and `result.txt` preserve the runnable probe and results. Command `node target/s3-hang-diagnostic/probe.mjs` exited 0: unread204 browser fetch resolved while `finished()` timed out; consumed204 and consumed200 finished; bounded rendering completed; absent response failed within its deadline. Local browser was existing Chromium `151.0.7922.34` (`chromium_headless_shell-1234`), not a newly installed browser. This is a transport diagnosis, not full hosted application acceptance or a claim that local/hosted browser binaries are identical.

### Correction and regression evidence

`completeForwardedResponse` checks the actual status and treats HTTP204 as bodyless. Other responses must yield complete body bytes within 15 seconds. The campaign still asserts the actual backend DELETE204, controls hidden before delivery, browser logout and revoked-bearer401. Real response forwarding, projected message counts, scope checks and all other acceptance assertions remain.

Response-body and rendering waits are bounded. Both resource closers are attempted independently with ten-second limits; failed cleanup is FAIL. An independent 180-second watchdog records FAIL and terminates a stalled harness before the existing five-minute CI deadline. Atomic startup/phase/operation checkpoints preserve the latest complete JSON while work is ongoing. Final PASS requires both scenarios and cleanup to pass. Reports contain fixed phase labels, never raw Playwright errors, headers, bearer URLs or DOM.

The workflow explicitly invokes the new loopback regression after installing the committed Playwright browser. Readiness curl calls have connect/total limits, and backend cleanup escalates to termination after five seconds. The main acceptance step remains five minutes; no gate is disabled or relabeled.

| Repair verification actually run | Result |
|---|---|
| New unread204 test with helper temporarily restored to baseline-equivalent `await response.finished()`; corrected file restored in `finally` | FAIL as expected: `204 completion stalled`, exit1. This replays the exact faulty wait; it is not a full baseline DB run |
| `node --test apps/desktop/acceptance/s3-controls-real.test.mjs` with `S3_BROWSER_EXECUTABLE` pointing to existing local Chromium1234 | PASS: 2 tests. Real unread204 completes, status mismatch rejects, real200 completes, stalled200 body fails within limit; stalled/rejected cleanup returns FAIL and progress JSON remains readable |
| Same two tests rerun on exact committed repair candidate a8a51b2 | PASS |
| `node --check` for harness, runtime helper and regression | PASS |
| Python/PyYAML parse/assertions | PASS; explicit regression enabled and main acceptance timeout remains5 minutes |
| `git diff 2569491 HEAD --check` | PASS |
| Limited private-key/GitHub/OpenAI secret-pattern scan of four changed executable files | No matches; not full secret-scanner certification |
| Harness against `http://127.0.0.1:1` | Expected startup FAIL, checkpoint and cleanup PASS written; not acceptance |

Command output is preserved in this task's transcript. No dependency change occurred. Client/Rust tests were not repeated locally for this harness-only repair; their original hosted preflight passed, while the new transport tests are the affected local regression. Full repaired browser/backend/PostgreSQL execution, hosted exact-candidate run, actionlint, cargo-deny/full secret scan and native checks remain NOT RUN by this worker. The old failed run remains failed and is not a waiver.

Fresh final reviewer `/root/repair_final_review`, different from the hang investigator, reviewed exact `2569491..a8a51b2` under trusted Critical rules: **no actionable findings**. It independently passed both transport tests using installed Chromium151, harness syntax and diff whitespace checks. It confirmed real status/logout/bearer-denial assertions remain, body waits are bounded, reporting is sanitized, cleanup failure cannot yield PASS, and hosted isolation/read-only credentials/five-minute timeout remain. It did not edit files or claim full PostgreSQL/hosted acceptance. Parent must perform fresh combined-candidate review and hosted execution after integration. Revert the repair commit to restore the prior harness/workflow (including its known hang); no schema/data migration or product rollback is involved.
