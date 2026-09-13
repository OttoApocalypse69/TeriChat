# S3-CLIENT task handoff

- Task: S3-CLIENT, usable session controls and caller-private workspace activity.
- Parent: 01a0802a-43a1-7271-957a-7d613dd63b8d; parent owns PR, CI, integration and cleanup.
- Branch: teriri/s3-client-controls, dedicated de8f worktree.
- Base: 05df96fa45221ae99308edcc2d33925ec0230cc6 (fetched origin/main before branching).
- Implementation/review head: f55fffdf60e9e2af3b90b2c53badef58e2124a23. Subsequent handoff commit adds only this document and browser evidence.
- Risk: Critical (session controls).
- State: ready for parent integration review; real backend/native acceptance remains unverified. Not pushed, no PR opened.

## Delivered / owned paths

- apps/desktop/src/lib/api.ts: typed session and private Stats methods through unchanged native/browser transport.
- apps/desktop/src/components/SessionPanel.tsx: live inventory with authoritative current-session marker, nullable device link, created/expiry times, paging, refresh/retry, serialized individual revoke, truthful unconfirmed failures.
- apps/desktop/src/components/WorkspaceActivity.tsx: opt-in workspace details activity, independent server aggregate and paginated caller channel counts, zero/empty/loading/error states and eventual consistency wording.
- apps/desktop/src/App.tsx: discoverable Sessions header control, focus restoration, mobile header wrapping, account-lifetime self-revoke logout; hidden inventory stays mounted to complete pending operations. Composer and notification behavior preserved.
- apps/desktop/src/components/AccountControls.test.tsx, App.test.tsx, lib/__tests__/api-transport.test.ts: real component/App lifecycle tests and native/browser transport assertions.
- apps/desktop/acceptance/s3-controls.mjs: reproducible installed-Edge UI check using synthetic HTTP fixtures. Run `node apps/desktop/acceptance/s3-controls.mjs` from repository root.
- apps/desktop/evidence/s3-controls/: fixture-only browser screenshots and measurements. These are NOT full backend acceptance evidence.
- This task-specific handoff. No backend, migrations, manifests, lockfiles, native settings or policy changes.

## Acceptance

| Criterion | Result | Evidence |
|---|---|---|
| Current identification, expiry, paging, revoke outcomes | Met locally | AccountControls.test.tsx; transport tests |
| Successful self-revoke logs out safely, including when inventory closes | Met locally | App tests for Close/Escape/header toggle and account replacement |
| Caller aggregate and current-channel page counts, empty/errors/eventual wording | Met locally | AccountControls.test.tsx; browser activity screenshot |
| Ignore stale account/workspace responses and clear denied Stats | Met locally | Mounted component tests; existing token-keyed App isolation |
| Preserve draft/focus, responsive controls | Met locally | App test and Edge 1440/768/390/320 measurements/screenshots |
| Preserve notifications and transport | Met locally | Existing notification suite passes; new native/browser API tests |
| Real browser/backend and native acceptance | Unverified | Docker Linux engine unavailable; no live service or native runtime claimed |

## Verification actually run

Commands use npm's committed lockfile. No dependencies changed.

| Command | Result / scope |
|---|---|
| `npm ci --ignore-scripts` (apps/desktop) | PASS, 201 installed, 0 audit vulnerabilities |
| `npm --prefix apps/desktop test` | PASS, 12 files / 137 tests; final source equals f55fffd |
| `npm --prefix apps/desktop run typecheck` | PASS |
| `npm --prefix apps/desktop run build` | PASS, 45 modules; final JS 220.25 kB / gzip 67.23 kB |
| `npm audit --audit-level=low` (apps/desktop) | PASS, 0 vulnerabilities |
| `npm --prefix apps/desktop run test:acceptance:unit` | PASS, 16 tests |
| `node apps/desktop/acceptance/s3-controls.mjs` | PASS with final CSS/config, Edge fixture-only UI; result.json and five PNGs beside it |
| `git diff --check` | PASS; existing Windows LF/CRLF notices only |
| Added-diff secret-pattern inspection | No private-key/GitHub/OpenAI token-pattern matches. Limited inspection, not a full scanner certification |
| `docker info --format '{{.ServerVersion}}'` | BLOCKED: dockerDesktopLinuxEngine named pipe absent |
| gitleaks discovery | NOT RUN: command unavailable |

Rust format/compile/Clippy/tests/doctests: NOT RUN, no Rust changes. Full backend/native acceptance: NOT RUN, unavailable Docker prerequisite. Hosted CI and parent integration checks: NOT RUN by this worker.
Existing Vite CommonJS/native config and module-type warnings remain. The UI fixture harness also reports the existing envFile deprecation. No warnings were suppressed.
An initial browser invocation from repository root produced a Tailwind content warning; it was not counted as styled layout evidence. The harness now sets its working directory to apps/desktop, and the subsequent fully styled run passed. Two earlier shell file operations used doubled relative paths and failed before mutation; corrected paths were used.

## Independent review and dispositions

Preimplementation reviewer `/root/adversarial_design` inspected concrete baseline App/API/backend code. Constraints included authoritative is_current, nullable device_id, aggregate versus loaded-page sum, revoked-token handling, account/workspace epochs, serialized mutations, and draft/focus preservation. No baseline candidate defect was claimed.

Fresh candidate reviewer `/root/candidate_review` inspected 9db0a64808f69191acc1fab02d60d441a5d17efe against base, finding:

- Finding ID: S3-CLIENT-R2-01; severity S2, high confidence; owner S3-CLIENT.
- Affected: SessionPanel.tsx cleanup/epoch guard and App.tsx conditional inventory rendering.
- Failure: begin self DELETE, hide panel using Close/Escape/header toggle, receive success; unmount invalidates onSessionEnded and leaves authenticated UI with revoked bearer.
- Evidence: exact-code static finding, then mounted App reproduction: `npm --prefix apps/desktop test -- --run src/App.test.tsx -t 'completes self-revoke'` failed all three close paths on the original implementation, with 33 unrelated tests intentionally filtered out. Expected Log in; received authenticated UnknownChat DOM.
- Correction: keep opened inventory mounted while hidden; token-keyed authenticated unmount still invalidates old account completions.
- After fix: all three regressions pass in the full 137-test suite.
- Disposition: FIXED in f55fffdf60e9e2af3b90b2c53badef58e2124a23. Same independent reviewer inspected the corrected immutable candidate and browser harness, confirmed fix and reported no remaining blocking findings. Reviews were static; runtime results are author execution evidence, not independent human approval.

## Compatibility / rollback / next

No schema, configuration, crypto membership, key-state or API contract changes. Requires PR30 APIs already in base; missing endpoints show errors. Session revocation is distinct from device/MLS/key revocation. Stats includes only caller counters; no wallet UI or leaderboard.
Reverting the two implementation commits removes these controls without migration. Sessions already revoked on the server remain revoked.
Next unblocked step: parent fresh integration review and PR/CI handling; obtain real synthetic backend/native acceptance when a permitted Docker engine is available. Critical human merge authorization remains required. No unresolved local blocking finding; unavailable runtime evidence is not a waiver.
