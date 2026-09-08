# SWARM-FE-01 task handoff

- Task and issue: SWARM-FE-01; Alpha client #5 / Milestone E onboarding and authoritative member directory.
- Human/parent thread: Teri / root coordinator.
- Worker ID: frontend.
- Branch/worktree: `teriri/alpha0-workspace-client`; `C:/Users/TeRiRi/Documents/GitHub/TeriChat-swarm-frontend`.
- Base SHA: `6f11a057b9ae60b2925578c729d020c8244b7387`.
- Tested implementation head SHA: `559f854046afcc4bb3fb885c6c101696c0fcd2a6`. The subsequent handoff commit changes only this report; obtain the final branch tip with `git rev-parse teriri/alpha0-workspace-client`.
- Risk: High; client role gating and asynchronous tenant/account scope. Backend remains authorization authority; this changes no account policy or cryptographic boundary.
- State: ready for independent review and parent integration acceptance; no merge/release/deployment authority claimed.

## Delivered

Workspace creation posts a trimmed name to the existing endpoint, selects the returned owner workspace and exposes the existing channel creation flow. The form preserves its draft on failure, reports errors and prevents repeated submission while pending. A workspace list begun before create/join/leave cannot erase the resulting state. Old account creation callbacks cannot publish into a replacement login.

The member panel obtains current UUID, handle, display name, role and joined-at records from the new paginated server endpoint. Loading, permission/network errors, page retry, refresh, loaded count and explicit Load more are visible. It never reads audit events or manufactures guessed seats. Successful add/role/kick/ban/unban invalidates loaded pages and retrieves the first authoritative page; more pages can be loaded again. The server-provided self role updates the parent workspace role when that self record appears in a fetched page. Existing rank gates, last-owner leave error handling and unban-by-ID remain. Ban pre-fills that ID in the unban form.

Page requests and mutations serialize within the panel. Workspace/account identity changes remount directory state; generation guards discard late completions and prevent old mutation callbacks or follow-up fetches. A related existing race in channel storage was fixed: a late list for a different workspace no longer clears the current channel selection; late channel creation cannot select a channel in the wrong workspace.

Changed paths:

- `apps/desktop/src/App.tsx`, `App.test.tsx`.
- `apps/desktop/src/components/CreateWorkspace.tsx`, `MemberPanel.tsx`, `MemberPanel.test.tsx`, `WorkspaceList.tsx`.
- `apps/desktop/src/lib/api.ts`, `members.ts`, `workspaces.ts`.
- `apps/desktop/src/lib/__tests__/workspace-api.test.ts`, `members.test.ts`, `workspaces.test.ts`.
- This handoff document only outside the source claim. No package, lockfile, configuration or policy files changed.

## Acceptance criteria

| Criterion | Met? | Evidence |
|---|---|---|
| Create workspace, select returned owner workspace, create first channel | Yes at mounted client layer | App tests exercise API calls and visible channel/member context. Parent owns real browser/backend acceptance. |
| Creation failure/retry and duplicate submission prevention | Yes | Mounted App tests retain draft, expose failure and issue one request while pending. |
| Authoritative current roles and ordinary-member directory access | Yes at client contract layer | Mounted panel renders server records without audit access, hides manager forms on authoritative self demotion and disables rank-forbidden actions. |
| Keyset paging, failed page retry, refresh and page invalidation | Yes | Mounted panel verifies exact returned cursor, preserves earlier page on page error, replaces stale rows on refresh and stops when cursor is null. |
| Add/role/kick/ban refresh and no overlapping mutations | Yes | Parameterized mounted tests exercise each endpoint and verify only returned post-mutation seats/roles. |
| Account/workspace stale responses and stale mutation callbacks | Yes | Mounted success/failure regressions plus existing authenticated-tree isolation regressions. |
| Leave/unban preserved | Yes | Mounted panel verifies endpoints, callback and server error display. |
| Real combined backend/browser, native Tauri and complete issue #5 acceptance | Unverified here | Assigned to parent integration; native and encrypted messaging remain distinct gates. |

## Verification

All npm commands below run in this worktree's `apps/desktop` unless stated otherwise. Local evidence directory: `C:/Users/TeRiRi/AppData/Local/Temp/terichat-swarm-fe-20260908/`.

| Actual command | Revision | Result | Evidence |
|---|---|---|---|
| `npm ci --ignore-scripts` | source worktree before implementation commit, committed lockfile unchanged | PASS; 200 packages installed; npm reported 0 vulnerabilities | Agent command output. Dependency lifecycle scripts intentionally not run. |
| `npm test` | exact source tree committed as `559f854` | PASS; 87 tests in 8 files | `tests.log` in evidence directory. |
| `npm run typecheck` | exact source tree committed as `559f854` | PASS | `typecheck.log`. |
| `npm run build` | exact source tree committed as `559f854` | PASS; tsc and Vite build, 37 modules | `build.log`. |
| `git diff --check` | source tree committed as `559f854` | PASS | Agent output, before commit. |
| `npm test -- src/lib/__tests__/workspaces.test.ts -t 'late channel list'` with only the new store guard temporarily removed | baseline behavior plus new regression | EXPECTED FAIL; current conversation became null instead of `conv-selected` | Retained agent command output; source restored in `finally`. Final full suite verifies repair. |

One initial `npm run typecheck` invocation mistakenly ran from repository root and failed with ENOENT because that directory has no package.json. The correctly scoped commands above subsequently passed. An initial diff check found extra blank EOF lines introduced during editing; they were corrected before the passing check. Neither was a flaky application test retry.

Existing Vite and PostCSS module-format warnings remain visible. No warning suppression or configuration edits. Two old `MemberDirectory` tests that explicitly asserted guessed audit-derived state were removed with the obsolete class, replaced by the mounted authoritative directory tests; rank and permission tests were retained.

Not run here: native Tauri build/UI acceptance, Rust format/compile/Clippy/database integration (no Rust changes; backend worker and parent own these), dedicated secret scanner (no configured scanner or `gitleaks` executable discovered), standalone `npm audit` beyond the clean-install audit, hosted CI and real combined backend/browser acceptance. No complete milestone/security/E2EE claim.

## Remaining findings and constraints

- Independent review and current combined candidate acceptance belong to the parent. No author self-approval.
- This directory lists current members only. It is not a bans listing, audit viewer or real-time membership subscription. Unban uses a supplied user UUID and the last successful ban can pre-fill it.
- Each mutation refresh starts at page one, keeping requests bounded; users explicitly load further pages again. Keyset traversal is a sequence of server reads rather than a transactional membership snapshot. Refresh begins a new traversal.
- The self-role fallback comes from the workspace API until the self UUID appears in a fetched member page; all permissions are still enforced by the server.
- Existing demo plaintext messaging notices remain unchanged. MLS/client encryption is outside this task.

## Compatibility and operations

Requires the paired backend `GET /v1/workspaces/{id}/members?limit=100&after=<uuid>` route returning `{members: [{user_id, handle, display_name, role, joined_at}], next_cursor}`. Workspace POST already exists. No database migration, credential, local-storage, key-state or production configuration change. Roll back the client commit to restore the previous UI if required; the additive backend route can remain. That rollback also restores the old guessed directory limitations, so coordinate with the integrator.

## Handoff

Source commit `559f854046afcc4bb3fb885c6c101696c0fcd2a6` supplied to parent and read-only reviewer. Parent owns integration and `apps/desktop/acceptance/workspaces.mjs`. Frontend source claim is released at completion; no writer remains active after this handoff. Next unblocked task is independent source review and the combined two-account browser/backend scenario, followed by native acceptance as a separate gate. No push, PR, merge or deployment performed by this worker.

## Review correction: SWARM-REVIEW-01-F1

Independent review found an S2 acceptance defect at implementation `559f854`: invalidating a pending initial workspace list after creating B preserved B but could omit existing membership A that had never entered the local store. The same omission affected joining while the first list remained pending.

Fix source head: `92bbd5f3a3a92340bbe9070ffb283fe185220caf`, parented by the documentation-only handoff `3fd10056456656d0c4e3a4e83cf929587ea93322`. Only `apps/desktop/src/App.tsx` and `App.test.tsx` changed in the fix. After create/join succeeds, the client now obtains a fresh authoritative list while preserving the selected returned workspace. Request revision guards also discard an earlier reconciliation if another mutation supersedes it.

The mounted regressions delay initial `[A]`, return fresh `[A,B]`, and assert existing A remains visible while B stays selected, for both create and join. Both fail against source `559f854`. Another mounted regression delays creation's fresh list until a subsequent join completes, confirming the earlier response cannot erase the newly joined workspace or its selection.

| Command | Source | Result | Evidence |
|---|---|---|---|
| `npm test -- src/App.test.tsx -t 'refreshes after'` with prior `559f854` App.tsx and new tests; source restored in finally | prior implementation behavior | EXPECTED FAIL; 2 failing regressions | `review-f1-baseline.log` in evidence directory above. |
| `npm test -- src/App.test.tsx` | fix source tree | PASS; 27 mounted App tests | Agent command output. |
| `npm test` | source tree committed as `92bbd5f` | PASS; 89 tests in 8 files | `review-f1-tests.log`. |
| `npm run build` | source tree committed as `92bbd5f` | PASS; includes `tsc --noEmit` | `review-f1-build.log`. |
| `git diff --check` | source tree committed as `92bbd5f` | PASS | Agent command output. |

The original 87-test evidence remains valid only for the earlier source. Fresh independent review and combined parent verification are required for the corrected head. The reviewer and parent received the exact fix SHA; remaining unrun checks and authority constraints above are unchanged. Source claim is again released after this correction.
