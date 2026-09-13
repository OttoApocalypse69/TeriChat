# S4-DM-ATOMIC independent review evidence

## Preimplementation investigation

- Reviewer: `/root/adversarial_investigation`, read-only independent agent.
- Base/head: `3d6232a62ed610227a5b2a1e8842533fd9fe0e02`.
- Finding S4-DM-01: S2, high confidence, acceptance-blocking. `apps/server/src/messaging.rs:318-345`: concurrent exact-pair lookups can both miss, independently insert conversations, and split message histories. Participant primary keys only deduplicate within one conversation.
- Finding S4-DM-02: S2, high confidence, acceptance-blocking. `apps/server/src/messaging.rs:281-300`: valid first participant followed by missing participant causes FK failure after autocommitted conversation and earlier participant writes, leaving partial state.
- Preconditions: synthetic distinct valid users, initially no matching DM for race; valid creator and initial member followed by missing user for rollback.
- Reproduction status: source-confirmed; local PostgreSQL unavailable. Mandatory real-PostgreSQL regressions added, not represented as executed locally.
- Owner/disposition: S4-DM-ATOMIC author; corrected in candidate, runtime confirmation belongs to parent hosted CI. No waiver or test weakening.
- Production callsites: `apps/server/src/routes/messaging.rs` DM and group handlers. Other helper callsites are tests; workspace channel creation has its own transaction.
- Constraints confirmed: preserve accepted self-DM behavior (new singleton per call), duplicate group response inputs, exact participant predicates, existing records/messages, and single acquired connection per operation.

## Fresh frozen candidate review

- Reviewer: `/root/frozen_candidate_review`, a different agent, read-only.
- Initial candidate: `7db3db94b88223457b3511201c97043a1d37bca0`.
- Final reviewed source: `694241413d0b1e8bfeda8fe1908babd29d238b05`.
- Trusted base: `3d6232a62ed610227a5b2a1e8842533fd9fe0e02`.
- Result: no actionable S0-S3 findings. The reviewer verified the sole follow-up difference was module-documentation backticks required by Clippy.
- Scope: creation code and tests, HTTP callsites, FK/PK constraints, workspace participant mutation scope, pinned SQLx 0.8.6 transaction/pool cleanup source.
- Evidence: canonical unordered-pair transaction lock precedes separate READ COMMITTED lookup; same connection performs all writes and reopen reads; exact-pair predicates exclude larger DMs; no nested pool acquisition; error/cancellation drop queues rollback.
- Risk: **High**. Reviewer initially suggested Normal but acknowledged correction to the explicit assignment floor; the author did not lower risk.
- Limitations: reviewer ran no PostgreSQL or compile/lint execution. SQLx cancellation queues rollback and does not synchronously interrupt an already blocked statement. Tests release their external blocker before asserting eventual rollback/progress. Advisory coordination applies only to callers using `find_or_create_dm`; old binaries and raw creation remain outside it. Historical duplicate DMs remain intact.
- These agents provide review evidence, not independent human approval identities or merge authority.
