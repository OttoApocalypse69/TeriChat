# Alpha client swarm — 2026-09-08

Base: `6f11a057b9ae60b2925578c729d020c8244b7387` (remote main inspected this session).
Owner: parent task `01a0802a-43a1-7271-957a-7d613dd63b8d`.
Claims remain active while this task is running and expire at its completion;
the parent receives worker progress and serializes integration.

| Task | Branch / owned paths | Acceptance / evidence |
|---|---|---|
| SWARM-BE-01 | `teriri/alpha0-members-api`; server `workspaces.rs`, `routes.rs`, backend handoff | Authoritative bounded member directory, tenant/revocation denial, pagination and minimal fields; real PostgreSQL tests |
| SWARM-FE-01 | `teriri/alpha0-workspace-client`; `apps/desktop/src/**`, frontend handoff | Create/select workspace; server-derived roster, paging, refresh after mutations, stale-response isolation; mounted tests/build |
| SWARM-INT-01 | `teriri/alpha0-swarm`; new workspace acceptance harness and swarm evidence/handoff | Combined candidate checks and two-browser flow against a disposable backend |
| SWARM-REVIEW-01 | Read-only candidate review | Correctness, security, concurrency and test-gap findings; no author edits or human approval claims |

This advances client issue #5 and Milestone E. The existing client already sends
demo plaintext and the pending MLS worktree has uncommitted work. Those files
are preserved, and neither encrypted Alpha acceptance nor issue completion is
claimed. Recovery format, payments, deployment, visibility and policy changes
are outside this wave. No shared manifests, lockfiles or migrations are claimed.

The backend contract is `GET /v1/workspaces/{id}/members?after=<UUID>&limit=100`,
returning `{members:[{user_id,handle,display_name,role,joined_at}],next_cursor}`.
Rows are ordered by immutable user UUID; the cursor is null when exhausted.
Current workspace members may read the roster; banned/removed/other tenants
may not. The existing member POST and moderation permissions remain in force.

Each writer has a separate checkout. Parent assembles commits only on the
integration branch; main, existing worktrees and external deployments are not
integration targets. Independent reports are review evidence, not authorization
to merge a sensitive change. No migration is needed; rollback is a code revert.
