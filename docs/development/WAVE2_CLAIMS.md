# Backend wave 2 task claims

Parent: `01a0802a-43a1-7271-957a-7d613dd63b8d`; base main
`f74d166649adcd9fe0765a8920bceccbb4ea6c6f`. Claims last for this active task and
expire on handoff. Workers report progress to the parent; parent integrates.

| Task | Writer ownership | Acceptance |
|---|---|---|
| W2-SESSIONS | Session worker: `apps/server/src/session_management.rs`, `apps/server/src/gateway.rs`, session handoff | Bounded own-session inventory; individual idempotent revocation; cross-account denial; persistent gateway invalidation; HTTP/DB and real-WebSocket tests |
| W2-STATS | Stats worker: `apps/server/src/workspace_stats.rs`, stats handoff | Caller-private workspace aggregate and paginated own channel counts; current membership/ban checks; isolation/idempotency tests |
| W2-MODERATION | Parent: `apps/server/src/moderation.rs` | Ban directory gated by existing BanMembers permission; bounded paging, public profiles, unban consistency and adversarial tests |
| W2-MOD-RACES | Follow-up worker in `TeriChat-wave2-moderation-races`: `apps/server/src/workspaces.rs`, race handoff | Revalidate existing BanMembers/ManageRoles authority after lock waits; unban checks authority in the mutation transaction; isolated deterministic regressions |
| W2-INTEGRATION | Parent: `main.rs`, `routes.rs`, wave 2 docs/evidence | Merge routes, validate real combined backend; independent attack review and preserved dispositions |

Worker checkouts branch from this route-wiring checkpoint. The empty routers
are temporary coordination scaffolding, not delivered endpoint implementations;
each must be replaced and tested before the wave is ready for review.
Gateway ownership was expanded to enforce existing session revocation on persistent
connections, including replay and idle sockets. No frontend, auth-policy, crypto, manifests, migrations, deploy or CI changes
are assigned. Stats exposes only the requesting user's metadata, not a public
leaderboard while its privacy policy remains open. Session revocation does not
claim device/MLS epoch revocation. Existing moderation permissions are reused.

Two backend writers and the parent work in separate mutable checkouts. Shared
wiring belongs solely to the parent. One adversarial reviewer starts with the
attack plan while writers work; a second independent adversarial reviewer joins
when a worker finishes, within the four-slot concurrency limit. Reviews target
immutable heads, may reproduce on isolated snapshots, and cannot mutate author
branches. No agent review supplies human merge approval for Critical changes.

Each worker has an exclusive synthetic DB; the integration tests use another
DB with no standalone backend competing for outbox work. Preserve all failures.
