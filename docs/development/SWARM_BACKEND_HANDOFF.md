# Task Handoff — SWARM-BE-01

- Task and issue: SWARM-BE-01; Alpha client #5 / Milestone E authoritative workspace roster.
- Human/parent thread: Teri / coordinating swarm task.
- Worker ID: backend.
- Branch/worktree: `teriri/alpha0-members-api`, `C:/Users/TeRiRi/Documents/GitHub/TeriChat-swarm-backend`.
- Base SHA: `6f11a057b9ae60b2925578c729d020c8244b7387`.
- Implementation head SHA: `a54b5d365fea143af6ce2ac15c8fc56d0f96497f`; this handoff is a subsequent documentation-only commit.
- Risk: High (membership-gated profile read; existing account-access policy is unchanged).
- State: ready for independent review and parent integration; no merge, push, deployment or issue-closure authority exercised.

## Delivered

`GET /v1/workspaces/{id}/members?after=<user UUID>&limit=100` now returns:

```json
{
  "members": [
    {
      "user_id": "00000000-0000-0000-0000-000000000001",
      "handle": "synthetic_member",
      "display_name": "Synthetic member",
      "role": "member",
      "joined_at": "2026-09-08T00:00:00Z"
    }
  ],
  "next_cursor": null
}
```

The default and maximum page size is 100. Positive larger limits are capped; nonpositive, malformed and overflowing numeric values are rejected with HTTP 400. `after` is an exclusive UUID cursor; invalid UUID syntax returns 400. Rows sort by immutable `user_id`, with one lookahead row determining whether another page exists. A full final page has a null cursor. A cursor does not have to identify an extant member, so departure of the last member on a prior page does not break pagination.

All current workspace members, including guests, may list. Missing workspaces, outsiders and removed callers receive the existing 403 non-member response; banned callers receive the existing 403 ban response. Each call obtains the caller's authoritative membership row `FOR SHARE` inside the transaction before checking bans and fetching the page. Existing kick, leave and ban writes conflict with this row lock, making a pending committed revocation deny the read and preventing revocation from committing between authorization and retrieval. The lock ends when the read transaction commits; it does not claim to revoke responses already obtained.

Only the five documented profile fields are selected and serialized. A separate HTTP DTO avoids reusing account responses that contain email. Banned rows are also excluded from results if an inconsistent stale membership exists. The existing POST member-add endpoint remains available.

Changed paths:

- `apps/server/src/workspaces.rs`: bounded domain query, member/page types, three DB-backed tests.
- `apps/server/src/routes.rs`: GET handler, DTOs, query parsing, preserved POST binding, two HTTP tests.
- `docs/development/SWARM_BACKEND_HANDOFF.md`: this evidence and handoff.

## Acceptance criteria

| Criterion | Met? | Evidence |
|---|---|---|
| Authoritative member, role and public profile list | Yes | `member_list_authoritative_roles_and_revocation` tests guest access and live role changes |
| Stable UUID keyset pagination; maximum 100; correct exhaustion | Yes | `member_list_keyset_cap_and_exhaustion` covers 102 members, oversized limit, exact full final page and empty continuation |
| Tenant isolation and removed/banned denial | Yes | Domain and HTTP negative tests; outsider and missing-workspace HTTP responses match |
| Revocation race handling | Yes | `member_list_waits_for_revocation_and_rechecks` observes a real PostgreSQL blocking relationship before committing removal/ban; reader denies after the wait |
| No private account fields | Yes | Explicit DB selection and HTTP profile key allowlist assertion |
| Existing POST contract preserved | Yes | HTTP test adds a guest via POST before listing via GET |
| Independent review and accepted integration | Pending | Parent owns review findings and integration; implementation commit supplied to reviewer |

## Verification

Host: Windows x86_64; `rustc 1.97.1 (8bab26f4f 2026-07-14)`, `cargo 1.97.1 (c980f4866 2026-06-30)`. Isolated default worktree target directory. Database tests used the fresh synthetic `swarm_backend` database provisioned by the parent, never existing application data; credentials are not retained here.

| Actual command | Revision | Result | Evidence |
|---|---|---|---|
| `cargo fmt --all` then `cargo fmt --all -- --check` | Implementation source matching `a54b5d3` | PASS | Tool transcript, exit 0 |
| `cargo check --locked --workspace --all-targets` | Implementation source matching `a54b5d3` | PASS | Tool transcript, exit 0 |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | Implementation source matching `a54b5d3` | PASS | `target/evidence/clippy.log` |
| `cargo test --locked --workspace -- --nocapture` with synthetic `DATABASE_URL` | `a54b5d3` source | PASS: 79 server + 13 tericrypt; 0 failed, ignored or filtered; no SKIPPED markers | `target/evidence/workspace-tests.log` |
| Doctest phase of `cargo test --locked --workspace -- --nocapture` | `a54b5d3` source | PASS: 0 doctests present | Same test log |
| `cargo build --locked --workspace` | `a54b5d3` | PASS | `target/evidence/build.log` |
| `git diff --check` | Implementation source matching `a54b5d3` | PASS | Tool transcript |
| `cargo deny --version` | Host tool discovery | Unavailable: no cargo-deny command | Tool transcript; dependency checks NOT RUN |
| `gitleaks version` | Host tool discovery | Unavailable: command not found | Tool transcript; automated secret scan NOT RUN |

Evidence logs are local, ignored build output, not committed. Source blob identities: `routes.rs` = `c7807d4511f3aedcf88872402f855e4a3019ca5b`; `workspaces.rs` = `b43eb31ffbe1449a2b83a2a73a066e276b0f1d4c`.

## Remaining findings and constraints

No failing test or known acceptance failure from the author checks. Independent review may produce findings; this is not self-approval. Dependency and automated secret scanners were unavailable. ARM64, hosted CI and frontend checks were not run by this backend worker; parent handles integration/client verification. No manifest, dependency, lockfile, schema, migration or workflow changes were needed.

## Compatibility and operations

Additive GET endpoint; existing POST and mutation responses are unchanged. No migrations, configuration, key-state changes or new services. Pagination is a live per-page read, not a multi-page historical snapshot: membership changes between calls can change later pages, while immutable UUID cursors prevent shifts caused by offset pagination. Each call reads at most 101 member profiles. Rollback is reverting the additive API commit; a client using the new endpoint must be rolled back or tolerate its absence.

## Handoff

Backend source and handoff paths are released to the parent after the handoff commit. The next unblocked task is integrating and verifying the frontend member roster against this API, followed by independent review disposition and the authorized contribution workflow. This task does not implement MLS, account MFA/recovery, attachments or future roadmap features.
