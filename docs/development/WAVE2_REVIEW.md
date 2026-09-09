# Backend wave 2 adversarial review

Task/PR: W2-INTEGRATION / #30. Base:
`c1a6c90baf4c9eec44a352c92594b234c601cfc2`.
Both independent reviewers inspected source head
`2cfca6c8aa58519727c15f82485a4c0fdc569d66` read-only.
Reviewers: `adversary_resume` and `adversary_concurrency`.
No agent review constitutes independent human approval. Runtime evidence is
tracked on the PR; local PostgreSQL execution was blocked by Docker startup.

## R1-STAT-STALE

- Severity/confidence: S2, high; stats worker owns correction.
- Affected source: preliminary worker tree based on `19a9515`,
  `workspace_stats.rs` blob `98802acb4157883f09b471343873ef2eb48a64f0`.
- Claim: a stats request authenticated before a membership lock wait could
  return private metadata after the caller session expired or was revoked.
- Preconditions/path: valid bearer; competing membership transaction; session
  becomes invalid while the read waits; the original helper checked membership
  only and continued. Source-derived; runtime baseline reproduction not run.
- Correction: lock the requesting session before membership and check fresh
  expiry after waits. Tests cover both stats reads under revocation and expiry.
- Triage: fixed in source `c63a63b`, integrated into reviewed `2cfca6c`.
  Both reviewers verified the correction; actual DB execution is required.

## R1-GW-TICK

- Severity/confidence: S2, high; session worker owns correction.
- Affected source: preliminary worker tree based on `19a9515`, `gateway.rs`
  blob `fb7368572cc2027627e33a778f862855323739bf`.
- Claim: a new one-second validity tick cancelled/restarted pending replay
  reads and their five-second timeout; a slow read could never progress.
- Preconditions/path: replay DB read takes longer than one second; timer wins
  select repeatedly. Source-derived; runtime baseline reproduction not run.
- Correction: pin the pending delivery future and timeout across heartbeat and
  timer handling. Keep periodic checks active during invisible replay too.
  Regressions hold a replay query after Ready, verify its PID/start time remains
  unchanged across ticks/heartbeats, then test delivery and mid-replay revocation.
- Triage: fixed in source `23ba4f5`, retained in reviewed `2cfca6c`.
  Both reviewers verified the correction; actual DB execution is required.

## R1-GW-TEST-ISOLATION / W2-CONC-01

- Severity/confidence: S2, high; duplicate independent reports, session worker.
- Affected head: `c5a84ea8e40bc817be80dcfb0f5fe2b28514d3d6`;
  `session_management.rs:176`, `gateway.rs:593`, `gateway.rs:651`,
  `gateway.rs:659`, `gateway.rs:688` at that revision.
- Claim: shared-schema table locks interfere with parallel tests; database-wide
  query-text matching can observe another test's blocked query. This can fail
  a correct implementation or produce false regression evidence.
- Preconditions/path: normal parallel `cargo test --workspace`; multiple
  gateway fixtures query shared outbox/participants tables. No deterministic
  deadlock or executed failure is claimed.
- Correction: unique schema and application name per fixture, same-test peers
  share that pool, and wait observations match both application name and exact
  blocker PID through `pg_blocking_pids`.
- Triage: fixed in source `7426807`, integrated as `2cfca6c`. Both reviewers
  independently verified the incremental fix. Parallel CI remains required.

## Existing moderation mutation findings requiring follow-up

These are source-derived candidates in unchanged `workspaces.rs` at base and
reviewed head. They are not fixed or waived by adding the ban read endpoint.
Owner: backend integrator/maintainer; next action: deterministic DB reproduction
and transactional permission revalidation. Candidate severity S1 because the
failure path permits an account to perform management actions after losing the
required permission; confidence high from source, runtime reproduction pending.

- `W2-BASE-AUTH-01`, `ban_member:1156-1167` and `set_role:1003-1010`:
  initial permission check precedes the transaction. After a concurrent
  Admin-to-Moderator demotion, locked roles are fresh but only rank is rechecked.
  Moderator still outranks a Member/Guest yet lacks BanMembers/ManageRoles.
  Reproduce by blocking the actor row between initial check and lock acquisition,
  committing demotion, and asserting the mutation is denied.
- `W2-BASE-AUTH-02`, `unban:1218-1227`: permission check precedes the transaction
  and no actor membership lock/recheck follows. A demotion/removal between that
  check and deletion can allow a now-unauthorized unban. Reproduce with a held
  ban-row lock, commit actor demotion/removal, release the lock and assert denial.

No serious candidate is closed by agent agreement. These remain open for
maintainer disposition; this wave is not a claim of full authorization safety
or merge readiness.
