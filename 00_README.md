# Platform Spec Pack — v3

**Temporary codename:** TeriChat. **Public name:** still open.  
**Security suite:** TeriCrypt-4096™. **Updated:** 2026-09-06. **Pack revision:** v4.

## Choose your entry point

**Teri:** read [TERI_TODO.md](TERI_TODO.md), starting with its five “Do now” items.  
**Coding agents:** read [AGENTS.md](AGENTS.md), then the assigned task and relevant files.  
**First product build:** use [the full bootstrap prompt](13_AGENT_BOOTSTRAP_PROMPT.md).

This is the existing product pack expanded with both engineering rounds. It is a specification and handoff kit, not a built application or already-configured CI/release system.

## Product direction

One secure platform for DMs/group chats, independently owned communities, professional workspaces, voice/video, SPICE Music, Stats, virtual Economy, Cinema/Broadcast rooms, spatial audio, collaborative whiteboards/docs/tasks, bots/WASM plugins, and explicitly authorized AI. Subscriptions and boosts fund capacity; basic privacy is not a premium-only benefit.

The initial deployment target remains a modest ARM64 host, potentially Oracle. Verify current quotas and availability before provisioning. Preserve off-machine backups and keep arbitrary CI code away from Production.

## Document map

| File | Purpose |
|---|---|
| [01_PRODUCT_VISION.md](01_PRODUCT_VISION.md) | Product scope, social/professional templates, independent workspaces |
| [02_SYSTEM_ARCHITECTURE.md](02_SYSTEM_ARCHITECTURE.md) | Rust-first modular architecture, data and event model |
| [03_SECURITY_TERICRYPT_4096.md](03_SECURITY_TERICRYPT_4096.md) | Security goals and trust boundaries; specific choices live in 12 |
| [04_CLIENTS_AND_UX.md](04_CLIENTS_AND_UX.md) | Desktop/web/mobile direction and interface concepts |
| [05_WORKSPACES_AND_COLLABORATION.md](05_WORKSPACES_AND_COLLABORATION.md) | Roles, invites, boards, docs, tasks, meetings |
| [06_VOICE_MEDIA_AND_CINEMA.md](06_VOICE_MEDIA_AND_CINEMA.md) | Voice, SPICE, spatial rooms, Cinema/Broadcast |
| [07_SERVICES_BOTS_AND_PLUGINS.md](07_SERVICES_BOTS_AND_PLUGINS.md) | Stats, ledger, Music, bots, WASM, BLOAT |
| [08_AI_PLATFORM.md](08_AI_PLATFORM.md) | Model/provider abstraction, permissions, local/remote processing |
| [09_MONETIZATION_AND_ENTITLEMENTS.md](09_MONETIZATION_AND_ENTITLEMENTS.md) | Draft plans, boosts, effective entitlements |
| [10_INFRASTRUCTURE_AND_OPERATIONS.md](10_INFRASTRUCTURE_AND_OPERATIONS.md) | Deployment, storage, backups, observability |
| [11_ROADMAP_AND_TASKS.md](11_ROADMAP_AND_TASKS.md) | Product milestones plus prioritized SETUP tasks and dependencies |
| [12_OPEN_DECISIONS.md](12_OPEN_DECISIONS.md) | Latest accepted decisions, unresolved details, decision queue |
| [13_AGENT_BOOTSTRAP_PROMPT.md](13_AGENT_BOOTSTRAP_PROMPT.md) | Full starter prompt, synchronized with current operating rules |
| [14_GLOSSARY.md](14_GLOSSARY.md) | Product and engineering vocabulary |
| [15_REPOSITORY_CHANNELS_AND_RELEASES.md](15_REPOSITORY_CHANNELS_AND_RELEASES.md) | **Round 1:** branches, channels, environments, releases, agents |
| [16_TESTING_REVIEW_AND_QUALITY_GATES.md](16_TESTING_REVIEW_AND_QUALITY_GATES.md) | **Round 2:** risk, reviewers, CI tiers, merge/release gates |
| [17_ENGINEERING_SOURCES.md](17_ENGINEERING_SOURCES.md) | Checked primary references for tool behavior and plan limitations |

Supporting files: [START_HERE.md](START_HERE.md), [AGENTS.md](AGENTS.md), [TERI_TODO.md](TERI_TODO.md), [CHANGELOG.md](CHANGELOG.md), [VALIDATION_REPORT.md](VALIDATION_REPORT.md), `manifest.json`, issue/PR templates under `.github/`, and finding/handoff/promotion templates under `templates/`.

## Authority and reading order

Owner-approved explicit updates and superseding ADRs take precedence. `12_OPEN_DECISIONS.md` records current product decisions; 15 and 16 define the accepted organizational/quality direction. `AGENTS.md` is the compact operating policy. The roadmap determines the next tasks; subsystem files supply detail. Older alternatives in general descriptions do not reopen a later locked choice.

For a first coordinator pass, read: AGENTS → CONTRIBUTING → 12 → 11 → 15 → 16 → docs/development/BOOTSTRAP_SESSION → relevant subsystem documents. Each specialist needs the applicable slice, not every future feature chapter. GitHub settings and executed tests—not documentation labels—establish what is actually enforced or implemented.

## Current fixed direction at a glance

Rust/Axum/Tokio + PostgreSQL, modular monolith with outbox and justified NATS distribution; React/TypeScript/Tailwind + Tauri/Vite, native SQLite; MLS/OpenMLS + RustCrypto with independent device membership; separate account and cryptographic recovery; passkeys/MFA; E2EE attachment originals; capability-based apps/AI; double-entry virtual ledger separate from billing.

One private monorepo, short-lived branches into protected `main`, one merge authority, explicit artifact provenance, Nightly/Canary/Beta/Stable channels, Local/CI/Staging/Production environments, and risk-based verification. Start small and activate more automation only when it works.

## What is not configured by this ZIP

No GitHub repository was created or modified. No merge bot was installed. No Actions workflow, branch protection, CODEOWNERS identity, Production secret, cloud resource, or paid service was enabled. The issue/PR templates and root agent instructions are ready to copy; agents implement the project-specific checks and Teri approves real access/budget/release decisions.

## Integration with SPICE

The existing project is `Spice-Production/SPICE`. Use its media-source capabilities through an explicitly designed adapter. Do not clone its provider implementation into this platform or assume directory names prove a deployed integration works.

## Scope discipline

The first product milestone remains **Alpha 0 — It Sends Bro**. Do not make it depend on multi-region infrastructure, Kubernetes, enterprise compliance, a plugin marketplace, or 4K HDR Cinema. The small engineering setup backlog protects that milestone; it must not become a replacement product.
