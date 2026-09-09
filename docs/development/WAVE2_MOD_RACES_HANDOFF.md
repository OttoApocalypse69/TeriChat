# W2-MOD-RACES handoff

- Task: W2-MOD-RACES; parent backend wave 2, follow-up to W2-BASE-AUTH-01/02.
- Worker: sessions_resume.
- Branch/worktree: `teriri/backend-moderation-races`, `C:/Users/TeRiRi/Documents/GitHub/TeriChat-wave2-moderation-races`.
- Base: `1f8bcb2c764cdb5712c9ba24bdb6cf7e7d31b1ab`.
- Test-only baseline: `f3c6797b00b187020539becce1bb5759b1e2a06f`.
- Fixed source: `621c9013a32ab90d8b5dc5f3717f2d3c7ce80b0a`.
- Risk: Critical, workspace authorization.
- State: implementation ready for independent review and CI database proof.

## Delivered

Only production path changed: `apps/server/src/workspaces.rs`.

`set_role`, `ban_member` and `remove_member` now check the existing permission evaluator against the actor role read under the transaction's existing sorted membership locks. A rank advantage no longer substitutes for ManageRoles, BanMembers or KickMembers after the actor is demoted.

`unban` locks actor membership before the target ban row, checks membership and BanMembers under that lock, and retains the lock through deletion and audit insertion. This follows the existing membership-before-ban order. Demotion/removal ordered before that lock is respected; changes ordered after it wait for the authorized transaction. A successful unban still does not rejoin the target, and absent bans remain an audit-free idempotent no-op.

No session policy, role grants, account access policy, migrations, manifests or frontend changed.

## Acceptance criteria

| Criterion | Implementation and evidence |
|---|---|
| Fresh ban permission | `ban_rechecks_permission_after_actor_demotion`: Admin to Moderator while blocked; Member target remains; no ban/audit effect |
| Fresh role-management permission | `set_role_rechecks_permission_after_actor_demotion`: same demotion; Member to Guest request denied despite rank advantage |
| Fresh kick permission | `kick_rechecks_permission_after_actor_demotion`: Moderator to Member; Guest target remains despite rank advantage |
| Fresh unban permission | `unban_rechecks_permission_after_actor_demotion`: ban retained and no audit effect |
| Fresh unban membership | `unban_rechecks_membership_after_actor_removal`: ban retained and no audit effect |
| Preserve authorized behavior | `authorized_moderation_preserves_effects_and_unban_idempotency`: role change, ban, unban and duplicate unban |
| Deterministic parallel-safe races | Per-test PostgreSQL schema and application name; actual blocker PID observed through pg_blocking_pids before mutation of authority |

All six tests live under `workspaces::moderation_race_tests`. The five race tests are expected to fail on the test-only baseline and pass on the fixed source; runtime proof is owned by parent CI and is not claimed from compilation.

The unban regressions deliberately hold both actor and ban rows in one blocker transaction: baseline unban waits on the ban, fixed unban waits on the actor. Demotion/removal then commits before releasing both. This tests the same ordering on both versions without trying to demote a row already protected by fixed unban.

## Verification

| Actual command | Revision | Result | Evidence |
|---|---|---|---|
| `cargo fmt --all -- --check` | source tree committed as 621c901 | PASS | Tool chunk `d0d397` |
| `cargo check --workspace --locked` | source tree committed as 621c901 | PASS | Tool chunk `d0d397` |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | source tree committed as 621c901 | PASS | Tool chunk `d0d397` |
| `git diff --check` | source tree committed as 621c901 | PASS | Tool chunk `6ab871` |
| `cargo test --workspace --locked --no-run` | 621c901 | PASS, compilation only | Tool chunk `67e284`, both test executables produced |
| `cargo test --workspace --doc --locked` | 621c901 | PASS, zero doctests | Tool chunk `67e284` |
| PostgreSQL tests on baseline and fixed source | f3c6797 / 621c901 | NOT RUN locally | Docker unavailable; parent running existing CI PostgreSQL lane |

Native Windows toolchain. No required test was weakened or serialized globally. Synthetic schema setup requires CREATE SCHEMA in the disposable test database; schemas remain available for failure diagnosis until database teardown. No local runtime failure/pass claim is made while the database is unavailable.

## Findings, compatibility and handoff

The source repairs the two existing S1 candidates and the independently reported analogous kick race. Runtime proof and fresh independent review remain required for final disposition. Existing errors remain Forbidden for a demoted member and NotMember for a removed actor; mutations denied after locking roll back without audit or target effects.

No persistent schema/API changes. Rollback is a source revert and restores the reported stale-authority defects. Parent owns integration, existing CI evidence, reviewer dispositions and PR handling; no merge/deploy authority is claimed. Next unblocked task: compare actual test-only CI failures with fixed-source results, then review the exact combined candidate.
