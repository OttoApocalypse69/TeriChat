# Monetization, Boosts, and Entitlements

## 1. Goal

Monetization should fund infrastructure expansion.

The commercial model should align expensive features with recurring revenue rather than arbitrarily paywalling basic communication or security.

**TeriCrypt-4096™ baseline security must not be a premium-only feature.**

## 2. Personal plan concept

Working prices only; not final.

### Free — €0

Core communication platform.

Possible baseline:

- DMs/group chats;
- workspaces;
- TeriCrypt baseline;
- text/voice/video;
- normal bots;
- Stats basics;
- Music basics;
- normal file limits.

### Plus — approximately €5/month

Primarily convenience/cosmetics.

Potential features:

- larger uploads;
- animated avatars;
- richer profile customization;
- themes;
- enhanced Stats;
- more emoji/sticker/profile options;
- possibly 1 boost.

### Pro — approximately €10/month

Main subscription.

Potential features:

- everything in Plus;
- **2 included boosts**;
- substantially higher personal limits;
- better streaming entitlement;
- advanced Stats;
- more developer/API quota;
- experimental features.

### Supporter — approximately €20–50/month

Explicit project-support tier.

Potential benefits:

- Pro features;
- more included boosts;
- supporter badge/cosmetics;
- early access;
- higher dev quotas;
- optional credits acknowledgement.

Supporter should not grant moderation superiority or "admin++".

## 3. Boosts

Users own boosts and allocate them to workspaces.

Boosts contribute toward workspace levels.

Boost benefits should roughly correspond to real infrastructure cost.

Example allocation:

```text
User owns 4 boosts

Workspace A: 2
Workspace B: 1
Workspace C: 1
```

Boost reassignment may have a cooldown.

If a workspace drops below a level threshold, use a grace period before expensive benefits disappear.

## 4. Example boost ladder

Numbers are placeholders.

### Level 0

- standard upload size;
- standard voice;
- 1080p target stream;
- 1 active Music instance;
- basic Stats;
- normal storage quota.

### Level 1 — e.g. 2 boosts

Potential unlocks:

- larger emoji/sticker allocation;
- animated workspace icon/banner;
- larger uploads;
- better voice bitrate;
- longer audit/Stats history;
- persistent Music queue.

### Level 2 — e.g. 7 boosts

Potential unlocks:

- 100MB-class uploads;
- 1440p target streaming;
- larger storage;
- always-on Music;
- 2 concurrent Music instances;
- advanced analytics;
- higher bot/plugin quotas.

### Level 3 — e.g. 14 boosts

Potential unlocks:

- 4K streaming where feasible;
- higher screen-share bitrate;
- 250MB+-class uploads;
- more storage;
- multiple Music workers;
- advanced backups;
- longer Stats retention;
- increased API/plugin capacity.

Exact quality/quota values must be capacity-tested before promising them.

## 5. Why boosts are useful

The intended flywheel:

```text
workspace grows
 ↓
more paid users / boosts
 ↓
more recurring revenue
 ↓
higher workspace entitlement
 ↓
more storage/compute/bandwidth can be afforded
 ↓
better workspace experience
```

This turns boosting into gamified infrastructure financing.

## 6. Professional plans

Possible future workspace plans:

```text
Workspace Free
Team
Business
Enterprise / Self-hosted
```

Potential paid professional features:

- larger storage;
- advanced whiteboards;
- professional admin controls;
- longer audit retention;
- SSO later;
- meeting transcription;
- AI credits;
- backup/export;
- retention policies;
- advanced automation;
- higher bot/API limits;
- premium media.

## 7. Entitlement engine

Do not write:

```rust
if plan == "pro"
```

throughout the codebase.

Use a centralized entitlement system.

Example attributes:

```text
max_upload_bytes
workspace_storage_quota
voice_bitrate_limit
max_stream_resolution
max_stream_fps
max_bots
max_plugins
max_music_sessions
music_always_on
stats_retention_days
audit_retention_days
ai_credit_pool
custom_branding
```

Feature logic asks for effective entitlements, not plan names.

## 8. Keep real and virtual economies separate

Real billing:

```text
EUR/USD/etc.
subscriptions
invoices
payment providers
boost ownership
```

Workspace economy:

```text
virtual currency
ledger accounts
shops
rewards
GDP/Gini jokes
```

Never automatically convert virtual currency into real subscription value without a deliberate future legal/accounting design.

## 9. Cosmetics as high-margin perks

Good low-cost premium features:

- animated avatar;
- animated banner;
- profile themes;
- custom backgrounds;
- nameplates;
- custom notification sounds;
- profile presets;
- enhanced Stats widgets.

These can subsidize expensive infrastructure features.
