# Clients and UX

> Current decisions in [12_OPEN_DECISIONS.md](12_OPEN_DECISIONS.md) supersede older alternatives in this overview. For agent/release behavior, follow [Round 1](15_REPOSITORY_CHANNELS_AND_RELEASES.md), [Round 2](16_TESTING_REVIEW_AND_QUALITY_GATES.md), and [AGENTS.md](AGENTS.md).

## 1. Client strategy

Build three primary client families:

### Desktop

Flagship client.

Preferred stack:

- Tauri;
- Rust shared core;
- modern web UI.

Expected native capabilities:

- notifications;
- tray;
- local secure key storage;
- filesystem integration;
- voice/video;
- screen share;
- media keys;
- deep links;
- auto-update;
- local cache/database.

### Web

Universal fallback and public app.

Possible stack:

- React/Next.js;
- WASM bindings to shared Rust components where practical.

### Mobile

Later milestone.

Possible UI stacks:

- React Native;
- Flutter;
- native platform UI if required.

Security/protocol logic should still reuse the Rust core through FFI where practical.

## 2. Shared client core

Design reusable Rust crates around:

```text
tericrypt
protocol
client-core
sync-engine
media-core
```

Avoid three independent implementations of security/session synchronization.

## 3. Main social UI

A community workspace can resemble:

```text
┌──────┬───────────────┬─────────────────────────────┬─────────────┐
│ SERV │ CHANNELS      │ #general                    │ MEMBERS     │
│      │               │                             │             │
│  A   │ # general     │ Teri                        │ Teri        │
│  B   │ # memes       │ bro                         │ Ryan        │
│      │ # music       │                             │ Stats       │
│      │ 🔊 General VC │ Ryan                        │ Economy     │
│      │ 🎬 Cinema     │ brotato                     │ Music       │
└──────┴───────────────┴─────────────────────────────┴─────────────┘
```

## 4. Professional UI

Same platform, different emphasis:

```text
SPICE Development
├── #general
├── #backend
├── #mobile
├── #releases
├── 🔊 Dev VC
├── 🎬 Demo / Presentation
├── ▦ Architecture Whiteboard
├── ✓ Project Board
├── 📝 Documents
└── AI Assistant
```

## 5. Built-in services deserve native UI

Stats, Economy, Music, and AI should not exist only as slash commands.

### Stats panel

- personal activity;
- workspace leaderboard;
- voice time;
- achievements;
- historical graphs;
- comparisons.

### Economy panel

- balance;
- transfers;
- transaction history;
- server shop;
- economy metrics.

### Music panel

- now playing;
- queue;
- playback controls;
- listeners;
- SPICE metadata.

### AI panel

- workspace assistant;
- action history;
- model/provider;
- permissions/context;
- agent tasks;
- cost/usage quotas.

## 6. Server/workspace creation

Offer templates:

```text
Community
Team
Company
Study
Custom
```

Templates should only choose defaults.

## 7. Security UX

Security state must be understandable.

Display:

- device verification;
- encrypted/managed channel state;
- identity changes;
- explicit AI/bot membership;
- TeriCrypt status;
- suspicious key changes.

Avoid implying stronger security than is actually provided.

## 8. Subscription UX

Clearly separate:

- personal plan;
- boosts owned;
- boost allocation;
- workspace boost level;
- workspace professional plan;
- actual unlocked entitlements.

Do not rely on vague "premium" checks.

## 9. Cinema UX

Cinema should have purpose-built controls and audience UI.

See `06_VOICE_MEDIA_AND_CINEMA.md`.

## 10. Accessibility

Plan for:

- keyboard navigation;
- screen readers;
- captions;
- configurable font sizes;
- reduced motion;
- high-contrast modes;
- color-blind-safe state indicators;
- voice transcription when explicitly enabled;
- AI-assisted accessibility later.
