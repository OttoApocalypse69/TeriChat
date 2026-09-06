# System Services, Bots, and Plugins

## 1. Principal tiers

Suggested capability tiers:

```text
Tier 0 — Core platform
Tier 1 — First-party system services
Tier 2 — Installed external applications/bots
Tier 3 — Sandboxed WebAssembly plugins
```

Tier is not a substitute for permissions.

All principals still need explicit capabilities.

## 2. Built-in system services

Primary first-party services:

- Stats;
- Economy;
- Music;
- AI runtime.

Suggested identities:

```text
system:stats
system:economy
system:music
system:ai
```

## 3. Stats service

Stats consumes activity events.

It does not need plaintext message content for basic metrics.

Potential metrics:

### Messaging

- messages sent;
- edits;
- deletes;
- replies;
- reactions;
- attachments;
- active days.

### Voice

- total voice time;
- session count;
- longest session;
- stream time;
- camera time;
- current session duration.

### Social

- activity streaks;
- active hour/day;
- favorite channel;
- interaction counts;
- rankings.

### Music/Cinema

- tracks played;
- music listening time;
- cinema attendance;
- reactions;
- watch time.

### Commands

```text
/stats
/stats user
/stats server
/stats voice
/stats messages
/stats music
/leaderboard
/compare
```

### Achievements

Build an extensible achievement engine.

Examples:

```text
Who Needs A Bed?
Remain in one voice session for 24 hours.

Persistent Connection
Remain in voice for 72 hours.

Archivist
Send 100,000 messages.

Touch Grass
Spend less than one hour online during a day.
```

## 4. Economy service

Use a real double-entry ledger.

Core entities:

```text
Currency
Account
Transaction
Posting
```

Invariant:

```text
sum(postings for transaction) = 0
```

Support:

- user wallets;
- workspace currencies;
- system treasury;
- peer transfers;
- shops/rewards later;
- transaction history;
- audits;
- compensating transactions.

Never mix fake workspace currency with real subscription billing.

## 5. Music service

See `06_VOICE_MEDIA_AND_CINEMA.md`.

Music should be first-party and use SPICE as its provider/source backend.

## 6. External bots

External bots use public API/gateway interfaces.

Application model:

```text
Application
BotIdentity
BotToken
OAuthClient
Commands
Webhooks
Permissions
GatewayIntents
```

Permissions = actions allowed.

Intents = event categories received.

## 7. Slash commands/interactions

Interaction framework should eventually support:

- slash commands;
- buttons;
- selects;
- modals;
- autocomplete;
- context actions.

Use a shared conceptual interaction model for built-in services and third-party apps where reasonable.

## 8. Event resume

Bot gateway sessions should support reliable reconnect/resume semantics so bot authors do not silently lose event streams.

## 9. Plugins

Plugins are not bots.

```text
Bot
external process → network API

Plugin
WASM module → Wasmtime host → capability-limited host API
```

Use:

- Wasmtime;
- WIT/Component Model when practical;
- execution fuel/time limits;
- memory limits;
- namespaced storage;
- no ambient filesystem;
- no ambient network;
- explicit capability grants;
- versioned APIs.

A broken/infinite-loop plugin must not take down the platform.

## 10. Plugin package concept

```text
plugin.toml
plugin.wasm
signature
assets/
```

Marketplace is later.

## 11. BLOAT

Background subsystem name:

**BLOAT — Background Logistics, Optimization, Archival & Telemetry**

Legitimate tasks may include:

- database maintenance;
- Stats rollups;
- search indexing;
- checksums;
- deduplication;
- thumbnail generation;
- metadata extraction;
- expired-session cleanup;
- stale-upload cleanup;
- log compression;
- metrics aggregation;
- backups;
- integrity checks;
- plugin cache maintenance;
- later media transcoding.

Do not generate meaningless resource load merely to avoid provider-idle policies.

If BLOAT has no useful work, it sleeps.
