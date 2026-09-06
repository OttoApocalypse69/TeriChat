# Infrastructure and Operations

> Current CI/release policy is in [Round 1](15_REPOSITORY_CHANNELS_AND_RELEASES.md) and [Round 2](16_TESTING_REVIEW_AND_QUALITY_GATES.md). The single-node application example does not authorize running arbitrary CI jobs on the Production host. Separate secrets/data and verified off-machine restoration are required before real-user rollout.

## 1. Initial deployment philosophy

The first production-like deployment may run on a small ARM64 VPS, including Oracle Cloud Always Free if available.

Do not couple the architecture to Oracle.

Before actual deployment, verify current Oracle quotas, reclaim policies, storage, egress, and ARM availability because cloud free-tier terms can change.

## 2. Initial single-node deployment

Conceptually:

```text
ARM64 VPS
├── Caddy
├── TeriChat server
├── realtime gateway
├── worker
├── PostgreSQL
├── NATS JetStream
├── plugin host
├── Stats
├── Economy
├── Music controller
└── optional media worker
```

Not every logical service needs a separate process initially.

## 3. Storage

Separate:

- relational application state;
- event/outbox state;
- object/attachment storage;
- temporary media cache;
- backups.

Do not store binary attachments directly in PostgreSQL.

Use object-storage abstraction.

Development may use:

- filesystem;
- MinIO.

Production may use:

- OCI Object Storage;
- S3-compatible storage;
- another provider.

## 4. Media cost control

Media is expected to dominate:

- egress;
- storage;
- CPU/transcoding.

Therefore:

- impose sensible upload caps;
- use quality entitlements;
- prefer direct/efficient media paths where security permits;
- bound caches;
- expire temporary transcodes;
- avoid permanent duplicate storage.

## 5. ARM64 compatibility

Everything must be tested on Linux ARM64.

Avoid x86-only dependencies unless an ARM alternative exists.

Prefer multi-arch Docker builds.

## 6. Reproducible deployment

Long-term desired path:

```text
Git
 ↓
CI
 ↓
container images
 ↓
Terraform/OpenTofu
 ↓
new VM
 ↓
restore data
 ↓
service operational
```

Treat compute as replaceable.

## 7. Backups

At minimum:

- PostgreSQL backups;
- object-storage metadata;
- encryption;
- off-machine copy;
- retention;
- documented restore process;
- periodic restore test.

A backup is not trusted until restoration has been tested.

## 8. BLOAT and maintenance

See `07_SERVICES_BOTS_AND_PLUGINS.md`.

BLOAT performs legitimate maintenance only.

Do not create fake CPU/network load merely to evade provider idle/reclaim policies.

## 9. Observability

Track:

- HTTP request rate/errors;
- WebSocket connections;
- gateway reconnects;
- message throughput;
- outbox backlog;
- NATS consumer lag;
- database pool utilization;
- auth failures;
- media sessions;
- media egress;
- plugin executions;
- AI usage/cost;
- storage utilization;
- boost/subscription entitlement usage.

## 10. Rate limiting and abuse controls

From the beginning plan for:

- auth rate limits;
- message rate limits;
- WebSocket abuse protection;
- bot API quotas;
- plugin resource limits;
- upload size limits;
- object scanning policies later;
- AI quota enforcement.

## 11. Future scaling

Do not optimize for millions of users yet, but preserve the ability to scale:

```text
multiple API instances
multiple gateway instances
multiple workers
multiple Stats consumers
multiple Music/media workers
multiple AI workers
SFU clusters
```

Public protocol semantics should not depend on single-process assumptions.

## 12. CI

GitHub Actions should eventually run:

- `cargo fmt --check`
- `cargo clippy`
- `cargo test`
- frontend lint/test
- security/dependency audit
- Docker build verification
- ARM64 build verification where practical.
