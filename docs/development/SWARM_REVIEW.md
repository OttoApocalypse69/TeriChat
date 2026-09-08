# SWARM-REVIEW-01

- Reviewer: independent read-only agent `/root/review`.
- Base: `6f11a057b9ae60b2925578c729d020c8244b7387`.
- Final reviewed source: `38a6a082b0c3924a1daa6df13125e53367efa40c`.
- Risk: Critical, from the trusted permission-sensitive path rules.
- Roles: correctness, security/privacy, concurrency, adversarial/test gap.
- Result: no unresolved blocking source finding; this is not human approval.

The reviewer compared the final server paths to `a54b5d365fea143af6ce2ac15c8fc56d0f96497f`,
frontend paths to `92bbd5f3a3a92340bbe9070ffb283fe185220caf`, and harness to
`b4c863e`. All scoped `git diff --exit-code` comparisons returned zero.
They inspected runtime evidence but did not run author code or edit files.

## Finding SWARM-REVIEW-01-F1

- Task/owner: SWARM-FE-01, frontend agent.
- Original head: `cc8d85b5f292ea729097955d897ec5b5d8c0fe7f`.
- Path: `apps/desktop/src/App.tsx:334` and `:356` at that revision.
- Severity: S2, high confidence, onboarding acceptance blocker.
- Claim: invalidating a pending initial workspace list without replacing it
  hides existing workspaces until reconnect/relogin.
- Preconditions: account already belongs to A; initial `[A]` read remains
  pending; create/join B finishes first.
- Failure: handler stores only B and increments the list revision; the initial
  A result is ignored; no further list request restores A.
- Evidence: reviewer traced the concrete async path; author added regressions
  that failed on `559f854` and passed on `92bbd5f`.
- Disposition: **FIXED**. Creation/join now reconcile through a fresh directory
  read, preserve selected B, and reject reads superseded by subsequent writes.
  Tests cover delayed `[A]`, fresh `[A,B]`, and overlapping creation/join.

## Evidence and limits

Reviewer inspected the final browser report (five scenarios, cleanup PASS),
79 server + 13 crypto test results without skipped markers, final 89 frontend
test results, and successful Windows native compile. The original failed Rust
attempt and its corrected DB isolation are retained in the parent handoff.

Existing Rust audit failure remains visible. Browser HTTP-adapter acceptance
does not establish native IPC, installation, or E2EE readiness. Cached caller
role before its page is loaded is a nonblocking UI freshness limitation;
server authorization still controls every mutation. No new migration,
dependency, crypto, policy or accepted authority change was introduced.
