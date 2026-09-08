# SWARM-INT-01 — Workspace onboarding and authoritative roster

- Task: client issue #5 / Milestone E, SWARM-BE-01 + SWARM-FE-01.
- Parent: `01a0802a-43a1-7271-957a-7d613dd63b8d`; workers: backend, frontend, independent review.
- Branch/worktree: `teriri/alpha0-swarm`, `C:/Users/TeRiRi/Documents/GitHub/TeriChat-swarm`.
- Base: `6f11a057b9ae60b2925578c729d020c8244b7387` (remote main rechecked after integration).
- Reviewed/tested source head: `38a6a082b0c3924a1daa6df13125e53367efa40c`. Subsequent commits add only handoff/evidence documentation; the PR records its final head.
- Risk: **Critical** (permission-sensitive member API). State: **ready for owner review**, not merged/deployed; dependency audit remains non-green.

## Delivered

Users can create and select a workspace directly in the shared React client,
then create channels using the existing channel API. The member panel now reads
current server membership instead of reconstructing seats from audit events.
It supports paginated loading, retry, refresh, current public profiles/roles,
and fresh server reads after add, role change, kick, ban or unban operations.
Late responses are scoped to the workspace/account and stale directory reads
cannot overwrite newer creation/join results.

Backend `GET /v1/workspaces/{id}/members` returns at most 100 rows with UUID
keyset pagination. Each request checks active membership and bans; a caller
membership lock serializes retrieval against removal. Explicit projections
exclude emails, keys and authentication data. Existing member POST and
moderation permissions remain in force.

Changed paths: server `routes.rs`/`workspaces.rs`; client `App.tsx`,
`CreateWorkspace`, `MemberPanel`, `WorkspaceList`, API/member/workspace helpers
and associated tests; new `acceptance/workspaces.mjs`/`WORKSPACES.md`; swarm
handoffs and evidence under `docs/development`. No manifests, lockfiles,
migrations, workflow/policy files, or cryptographic code changed.

## Acceptance criteria

| Criterion | Met? | Evidence |
|---|---|---|
| UI creates and selects a persisted owner workspace | Yes | Real browser campaign and mounted tests |
| Ordinary member sees roster without audit access | Yes | Browser campaign checks roster and audit 403 |
| Pagination is bounded, stable and excludes private fields | Yes | DB/route tests and two API pages in browser campaign |
| Outsiders, removed and banned users cannot list | Yes | DB/route tests, lock-wait race tests, browser outsider/kick checks |
| Mutations refresh current roster, stale replies are isolated | Yes | Mounted regressions and real role/kick flow |
| Channel message survives reload | Yes | Shared React client/real HTTP backend, synthetic `bro` |
| Secure Alpha 0 / MLS / recovery / native two-client acceptance | No | Outside this slice; demo plaintext warning remains visible |

## Verification

Commands ran in the integration worktree unless noted. Backend Rust source is
identical to reviewed worker `a54b5d3`; frontend source is identical to reviewed
worker `92bbd5f`. [Worker backend](SWARM_BACKEND_HANDOFF.md) and
[worker frontend](SWARM_FRONTEND_HANDOFF.md) handoffs retain their own evidence.

| Actual command | Revision/scope | Result |
|---|---|---|
| `npm ci --ignore-scripts --no-audit` | Unchanged lockfile | PASS |
| `cargo fmt --all -- --check` | `198ef22`, final Rust source | PASS |
| `cargo check --workspace --locked` | Same | PASS |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Same | PASS |
| `cargo build --workspace --locked` | Same | PASS |
| `cargo test --workspace --locked -- --nocapture` | `1f3107e`, database shared with running browser backend | **FAIL**, 78/79 server tests; WS event timeout |
| Same command, fresh verification-only DB | `1f3107e`, final Rust source | PASS, 79 server + 13 crypto tests; zero skipped/ignored |
| `cargo test --workspace --doc --locked` | `38a6a08` | PASS command; zero doctests, not additional coverage |
| `npm test` | `38a6a08` | PASS, 89 tests / 8 files |
| `npm run build` (includes `tsc --noEmit`) | `38a6a08` | PASS |
| `npm run test:acceptance:unit` | `cc8d85b`, unchanged validator | PASS, 16 tests |
| `node acceptance/workspaces.mjs http://127.0.0.1:<owned-port>` | `38a6a08` | PASS, all 5 scenarios and cleanup |
| `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml --locked --target-dir target/native-check` | Unchanged native source/lockfile | PASS, Windows compile |
| `npm audit --json` | Unchanged package lockfile | PASS, zero reported vulnerabilities |
| `cargo audit --json` | Unchanged Cargo.lock | **FAIL**, existing RUSTSEC-2023-0071 / rsa 0.9.10 |
| `cargo tree --locked -i rsa`, also `--target all` | Unchanged selected dependency graph | No active reverse dependency printed; does not waive audit failure |
| `git diff --check 6f11a05` | Combined candidate | PASS |

The first parent Rust run used the same DB as its running browser backend.
Both processes claim the same outbox rows for separate in-process hubs, which
can steal the test's event. The existing `WS_SERIAL` comment in `main.rs`
documents this constraint. The failed log is preserved; the one cause-corrected
rerun used a separate fresh database with no standalone backend. Tests and
timeouts were not changed. The worker's independent fresh-DB run also passed.

Local raw logs remain in `target/swarm-runtime/`: `rust-tests.log`,
`rust-tests-isolated.log`, `frontend-final-tests.log`, `frontend-final-build.log`,
`native-check.log`, and dependency audit JSON. Browser evidence remains in
`apps/desktop/.acceptance/workspace-2026-09-08T08-55-25-674Z/`; the earlier red
baseline is `workspace-2026-09-08T08-46-09-936Z/` (missing Workspace name input).
Only sanitized result JSON is committed under `evidence/swarm/`.

## Review and remaining constraints

Independent reviewer inspected correctness, tenant/privacy boundaries,
concurrency, frontend state isolation and test gaps at exact source head
`38a6a08`. Scoped source comparisons to both worker heads and the harness
returned no differences. See [review report](SWARM_REVIEW.md).

One S2 finding was fixed: creating/joining B before the initial directory `[A]`
resolved discarded A without a replacement fetch. Fresh post-write reconciliation
now preserves the selected workspace; baseline-failing mounted tests cover both
actions and overlapping creation/join. No unresolved blocking source finding.

Remaining limits: the existing Rust dependency advisory is unresolved. Full
`cargo deny` and dedicated secret scanning were NOT RUN because tools/policy are
not installed; a scoped pattern scan is not equivalent certification. ARM64
verification, signed installers, native IPC/two-window acceptance, full existing
messaging acceptance campaign, deployment and production security review were
NOT RUN in this wave. Existing Vite/PostCSS module-format warnings remain.
The browser test uses the existing test-only HTTP adapter/CORS bridge and does
not certify default native or deployment transport. Caller-role text can remain
cached until its member page is loaded; every action is still server-authorized.

## Compatibility, operations and next work

The API is additive. Deploy matching backend before the new frontend; an older
backend produces a visible roster error. No migration or key-state change is
required. Revert frontend first or both code changes for rollback. Resource use
is bounded to one roster page/transaction; no new services or paid resources.

All newly created synthetic backend processes and the owned PostgreSQL container
were stopped/removed after verification; existing containers/worktrees were
preserved. The original checkout remains untouched. Claims are released.
Owner review and existing audit disposition remain required before integration.
Do not close #5 or mark Alpha 0 complete. Next P0 work is to coordinate the
existing uncommitted MLS foundation (#6) with its owner, then implement/review
client encrypted transport and persistence; recovery encoding (#11) retains its
separate decision boundary.
