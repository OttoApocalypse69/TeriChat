# S3-WALLET review evidence

## Initial adversarial pass

Reviewer `/root/wallet_adversarial` inspected base
`05df96fa45221ae99308edcc2d33925ec0230cc6` read-only before implementation.
No runtime reproduction or edits. Design hazards: timestamp-only boundaries
lose ties; global anchor lookup can leak foreign transaction existence;
timestamps are not commit order; bounds and authorization must hold before
querying another wallet. Recommended 205 tied entries, limits 1/37/100,
foreign/unknown/shared anchors, and live additions on both sides of a cursor.

Disposition: implemented caller-scoped UUID anchor resolution, exclusive tuple
comparison, existing bounds, unchanged envelope, and explicit live semantics.
Tests compile; PostgreSQL execution remains pending.

## Fresh final review

Different reviewer `/root/wallet_final_review` inspected immutable source
`3f18f15c59488030bf10f82a7cc4ae680349ee31` against the same base. Perspectives:
correctness, security/privacy, data integrity, concurrency and adversarial
test gaps. No source edits or runtime tests; `git diff --check` passed.

Conclusion: no concrete blocking defect found. Both lookup and retrieval use
the caller's posting; unknown and foreign anchors share an error; tuple order
matches the boundary; timestamp precision is retained; live semantics are
accurate; transfer/write arithmetic remains unchanged.

### Preserved nonblocking observation

- Finding ID: S3-WALLET-REVIEW-01
- Task: S3-WALLET
- Reviewer: `/root/wallet_final_review`, concurrency/test-gap
- Head/base: `3f18f15c59488030bf10f82a7cc4ae680349ee31` /
  `05df96fa45221ae99308edcc2d33925ec0230cc6`
- Severity: S3 (coverage observation, no demonstrated defect)
- Confidence: high about test coverage; no correctness failure inferred
- Path/symbol: `apps/server/src/ledger.rs`,
  `history_query_enforces_owner_and_exclusive_live_boundary`
- Claim: the test awaits the writer before page two. It tests additions between
  requests, but not a commit overlapping the anchor and page SQL statements.
- Preconditions/impact: an overlapping writer could change visible row sets;
  the contract explicitly permits live committed additions and existing
  application transactions have no update/delete route.
- Evidence: static candidate inspection only; no runtime result claimed.
- Proposed follow-up: barrier-controlled overlap test if snapshot semantics or
  mutable transactions are later introduced.
- Owner: parent/coordinator
- Disposition: retained nonblocking observation; no required behavior failure
  demonstrated, no implementation change. DB acceptance remains unverified.

Risk remains Critical. Automated review is evidence, not independent human
GitHub approval, permission to merge, or a substitute for hosted PostgreSQL
execution and current integration-candidate validation.
