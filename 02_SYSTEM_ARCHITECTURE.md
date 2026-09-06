# System Architecture

> Current decisions in [12_OPEN_DECISIONS.md](12_OPEN_DECISIONS.md) supersede older alternatives in this overview. For agent/release behavior, follow [Round 1](15_REPOSITORY_CHANNELS_AND_RELEASES.md), [Round 2](16_TESTING_REVIEW_AND_QUALITY_GATES.md), and [AGENTS.md](AGENTS.md).

## 1. Architecture style

Start as a **modular monolith with distributed-capable boundaries**.

Do not begin with dozens of microservices.

Initially it should be possible to run the platform on one modest ARM64 VM.

Logical boundaries may later become independent processes.

## 2. Rust-first stack

Preferred backend ecosystem:

- Rust stable;
- Tokio;
- Axum;
- Tower;
- Serde;
- SQLx;
- PostgreSQL;
- NATS JetStream where durable asynchronous event distribution is useful;
- tracing;
- OpenTelemetry where useful;
- Wasmtime for plugins;
- WIT / Component Model where practical;
- UUIDv7 or equivalent sortable globally unique IDs.

Possible frontend:

- React/Next.js or a similarly modern frontend stack;
- Tauri for desktop;
- shared Rust core where practical.

## 3. High-level topology

```text
                         Clients
              ┌───────────┼───────────┐
              ▼           ▼           ▼
          Desktop        Web        Mobile
              │           │           │
              └──── HTTPS/WSS/WebRTC ─┘
                          │
                     Edge / Caddy
                          │
              ┌───────────┴───────────┐
              ▼                       ▼
          API/Core                Realtime Gateway
              │                       │
              ├──────────┬────────────┤
              ▼          ▼            ▼
          PostgreSQL   NATS        Media/Voice
              │          │
              ▼          ▼
          Outbox      Consumers
                         │
             ┌───────────┼──────────────┐
             ▼           ▼              ▼
           Stats       Economy         Music
                                       │
                                       ▼
                                      SPICE
```

## 4. Suggested repository layout

```text
terichat/
├── Cargo.toml
├── README.md
├── apps/
│   ├── server/
│   ├── gateway/
│   ├── worker/
│   ├── plugin-host/
│   └── media-worker/
├── crates/
│   ├── core/
│   ├── protocol/
│   ├── auth/
│   ├── database/
│   ├── events/
│   ├── permissions/
│   ├── tericrypt/
│   ├── client-core/
│   ├── sync-engine/
│   ├── bot-api/
│   ├── bot-sdk/
│   ├── plugin-api/
│   ├── plugin-sdk/
│   ├── entitlements/
│   └── telemetry/
├── services/
│   ├── stats/
│   ├── economy/
│   ├── music/
│   └── ai/
├── clients/
│   ├── desktop/
│   ├── web/
│   └── mobile/
├── migrations/
├── deploy/
├── docs/
└── scripts/
```

Only create crates when actual boundaries justify them.

## 5. Core entities

At minimum, design around:

### Identity

- User
- Device
- Session
- Credential
- DeviceIdentity
- Application
- BotIdentity
- ServicePrincipal

### Workspaces

- Workspace
- WorkspaceMembership
- Role
- Permission
- Channel
- ChannelPermissionOverride
- Invite

### Messaging

- Conversation
- ConversationParticipant
- MessageEnvelope
- MessageMetadata
- Reaction
- AttachmentMetadata
- ReadState

### Events

- DomainEvent
- EventOutbox
- ConsumerOffset / delivery state

### Voice/media

- VoiceRoom
- VoiceSession
- MediaSession
- CinemaSession
- PlaybackState
- SpatialPosition

### Collaboration

- Whiteboard
- CollaborativeDocument
- Task
- Project
- Meeting
- MeetingArtifact

### Platform

- BotInstallation
- Webhook
- SlashCommand
- PluginInstallation
- PluginCapabilityGrant

### Monetization

- Subscription
- Plan
- Entitlement
- Boost
- BoostAllocation
- WorkspaceBoostState

### Economy

- Currency
- LedgerAccount
- LedgerTransaction
- LedgerPosting

## 6. Event architecture

Significant actions should emit versioned domain events.

Examples:

```text
message.created
message.updated
message.deleted
reaction.created
reaction.deleted

member.joined
member.left

presence.updated

voice.session.started
voice.session.ended
voice.position.updated

cinema.session.started
cinema.playback.changed
cinema.reaction.created

music.track.queued
music.track.started
music.track.finished

economy.transaction.created

task.created
task.completed

whiteboard.updated

ai.action.requested
ai.action.completed

subscription.changed
boost.allocated
boost.removed
```

Suggested envelope:

```json
{
  "event_id": "...",
  "event_type": "message.created",
  "version": 1,
  "timestamp": "...",
  "actor_id": "...",
  "workspace_id": "...",
  "data": {}
}
```

## 7. Transactional outbox

For durable state-changing actions:

1. validate request;
2. authorize;
3. mutate PostgreSQL;
4. write outbox entry in same transaction;
5. commit;
6. worker publishes event;
7. mark/retry delivery.

Consumers must be idempotent because duplicate delivery is possible.

## 8. Realtime gateway

Versioned WebSocket protocol.

Required concepts:

- identify/authenticate;
- heartbeat;
- heartbeat ACK;
- event sequencing;
- reconnect;
- resume;
- intents;
- structured errors;
- server-requested reconnect;
- backpressure.

Potential path:

`/v1/gateway`

## 9. Permissions

Centralize authorization.

Do not scatter `if user.is_admin`.

Permission model should support:

- workspace roles;
- channel overrides;
- bot permissions;
- system-service capabilities;
- AI capabilities;
- plugin capabilities;
- professional/admin policy.

Permissions answer "what may this principal do?"

Gateway intents answer "what event categories may it receive?"

## 10. Multi-tenancy

Every tenant-owned entity must have clear workspace/conversation scope.

Cross-workspace leakage is a critical security failure.

Indexes, cache keys, object-storage paths, metrics, and background jobs should preserve tenant boundaries.

## 11. API versioning

Use explicit API versions.

Example:

```text
/api/v1/...
```

Use structured machine-readable errors.

## 12. Shared protocol types

Where practical, define canonical protocol/event structures in Rust and generate/bind client representations.

Avoid subtly divergent data models between desktop, web, mobile, bots, and server.
