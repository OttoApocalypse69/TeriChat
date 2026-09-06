# Roadmap and Prioritized Task List

This file is the engineering priority source of truth.

## Engineering setup backlog — Rounds 1 and 2

**All items below are pending implementation/verification.** Documentation acceptance is not completion. Use this queue alongside the product milestones; retain the P0 recovery/security decision work. `P*` means roadmap urgency, never review severity.

| ID | Priority | Accountable party | Task and acceptance evidence | Dependencies |
|---|---|---|---|---|
| SETUP-001 | P0 | Teri + coordinator | Select existing/new private repo; inspect current code, plan capabilities, access, tools, and budget. Produce an actual capability/gap report. No paid change without approval. | None |
| SETUP-002 | P0 | Coordinator | Integrate spec pack, root AGENTS and templates; preserve existing code; record task/branch naming and ownership. | SETUP-001 |
| SETUP-003 | P0 | Build agent | Minimal Rust/workspace/toolchain, synthetic Compose dependencies, formatting/lint configuration, and health check. Use existing foundation if already present. | SETUP-002 |
| SETUP-004 | P0 | CI agent | Implement real baseline checks and structured output. Observe one known failure and repaired success; no empty passing test placeholders. | SETUP-003 |
| SETUP-005 | P0 | Coordinator/reviewer | Implement scoped task claims, separate worktrees, risk floors, review/finding/handoff reports, and shared-file coordination. | SETUP-002 |
| SETUP-006 | P0 | Teri | Approve real owners and enable available PR/check/branch/tag protections after checks exist. Verify enforcement; record unavailable controls. | SETUP-004, SETUP-005 |
| SETUP-007 | P0 | Teri/Ryan + integrator | Two-task pilot with distinct branches and independent review; serialized current-base integration. Demonstrate stale/failing work is not admitted. | SETUP-006 |
| SETUP-008 | P1 | Coordinator + Teri | Choose actual merge provider based on private-repo capabilities/cost; install only with owner approval. Keep serial fallback documented. | SETUP-007 |
| SETUP-009 | P1 | CI/integration agent | Trusted risk dispatcher and always-reporting policy gate; test missing/skipped/canceled/stale/fake-result denial and candidate event coverage. | SETUP-008 |
| SETUP-010 | P1 | CI/security agent | Dedicated runner isolation, cache boundaries, resource/time limits, secret-free PR execution; no Production cohost. | SETUP-009 |
| SETUP-011 | P1 | Release agent | Immutable artifact manifest, internal Nightly feed, correct version/channel separation, controlled signing/provenance. | SETUP-004, SETUP-010 |
| SETUP-012 | P1 | Operations + Teri | Synthetic Staging and independent encrypted backup destination; restore drill with recorded results. | SETUP-010 |
| SETUP-013 | P1 | Domain/test agents | Expand auth/crypto/permission/ledger acceptance with real implementation; keep the encrypted bro scenario permanent. | SETUP-004; corresponding product components |
| SETUP-014 | P2 | Test/CI agents | Seeded property/fuzz campaigns, targeted mutations, flake registry and expiring exception policy; measure cost and coverage quality. | SETUP-009, SETUP-013 |
| SETUP-015 | P2 | Release + Teri | Canary/Beta/Stable gates, supported-client/installer matrix, migration compatibility, rollout and rollback/roll-forward rehearsal. | SETUP-011, SETUP-012, SETUP-013 |
| SETUP-016 | P2 | Integration lead | Increase writer/runner capacity only after queue metrics justify it; trial small noncritical batches. | SETUP-009, SETUP-014 |
| SETUP-017 | P2 | Teri + relevant reviewer | Pre-public-beta security/operations review, unresolved S0/S1 disposition, privacy/reporting and licensing decisions. No unsupported security claim. | SETUP-013, SETUP-015; relevant product gates |

**First delegation:** SETUP-001 through SETUP-005 where authorized/unblocked. Teri handles actual repo access/protection decisions. Then prove the pilot and continue Alpha 0; do not finish by only writing more plans.

[Human checklist](TERI_TODO.md) · [Round 1](15_REPOSITORY_CHANNELS_AND_RELEASES.md) · [Round 2](16_TESTING_REVIEW_AND_QUALITY_GATES.md)

## Priority legend

- **P0** — required to prove the platform works at all.
- **P1** — required for a credible private alpha.
- **P2** — major platform differentiator / early beta.
- **P3** — expansion and professionalization.
- **P4** — ambitious/expensive long-term work.

---

# Milestone A — Repository and foundations

## P0

- [ ] Choose temporary repository name and initialize Git.
- [ ] Create Cargo workspace.
- [ ] Add Rust toolchain config.
- [ ] Add formatting/lint policy.
- [ ] Create configuration system and `.env.example`.
- [ ] Add structured error model.
- [ ] Add tracing/logging.
- [ ] Add health/readiness endpoints.
- [ ] Create Docker Compose for PostgreSQL + NATS.
- [ ] Add SQLx migrations.
- [ ] Define UUIDv7/domain ID strategy.
- [ ] Add CI for fmt/clippy/test.
- [ ] Ensure Linux ARM64 build compatibility.
- [ ] Write initial ADRs.

Exit condition: repository builds cleanly and local dependencies boot reproducibly.

---

# Milestone B — Identity and authentication

## P0

- [ ] User model.
- [ ] Device model.
- [ ] Session model.
- [ ] Register/login/logout.
- [ ] Argon2id password hashing or current equivalent best practice.
- [ ] Session revocation.
- [ ] Authenticated API extractor/middleware.
- [ ] Device registration.
- [ ] Initial device identity key model.
- [ ] Tests for auth/session lifecycle.

## P1

- [ ] Device management UI/API.
- [ ] Existing-device approval flow.
- [ ] Refresh/session rotation.
- [ ] Implement the accepted first-class passkey/MFA architecture; do not treat it as an optional security afterthought.

---

# Milestone C — Core messaging: "It Sends Bro"

## P0

- [ ] Conversation model.
- [ ] Conversation participants.
- [ ] MessageEnvelope schema.
- [ ] Store encrypted payload, not plaintext.
- [ ] Send-message API.
- [ ] Message history API.
- [ ] Transactional outbox.
- [ ] Outbox worker.
- [ ] Idempotent event publish.
- [ ] Realtime WebSocket gateway.
- [ ] Identify + heartbeat.
- [ ] Event sequence/resume model.
- [ ] Two-client integration test.
- [ ] Reconnect and history retrieval test.
- [ ] First encrypted message should be `bro`.

Exit condition: two users can securely exchange an encrypted DM and reconnect without losing history.

---

# Milestone D — TeriCrypt-4096™ Alpha

## P0

- [ ] Dedicated `tericrypt` crate/interface.
- [ ] Threat-model skeleton.
- [ ] Established library selection.
- [ ] Per-device identity abstraction.
- [ ] Secure encrypted-DM proof of concept.
- [ ] Explicit documentation of missing security properties.
- [ ] No plaintext logs.

## P1

- [ ] Device verification.
- [ ] QR/safety-number-style verification.
- [ ] Identity change warnings.
- [ ] Multi-device encrypted message fanout.

## P2

- [ ] Forward secrecy.
- [ ] Post-compromise recovery.
- [ ] Group E2EE.
- [ ] Attachment E2EE.
- [ ] Key transparency.
- [ ] Post-quantum-capable session establishment.

## P3

- [ ] Formal protocol versioning.
- [ ] External security review preparation.
- [ ] Fuzz/property testing of protocol state machines.

---

# Milestone E — Workspaces and channels

## P1

- [ ] Workspace model.
- [ ] Membership model.
- [ ] Role model.
- [ ] Central permission evaluator.
- [ ] Text channels.
- [ ] Invites.
- [ ] Channel permission overrides.
- [ ] Basic moderation actions.
- [ ] Group DM.

## P2

- [ ] Workspace templates.
- [ ] Announcement/forum types.
- [ ] Server-specific profiles.
- [ ] Audit log framework.

---

# Milestone F — Built-in Stats

## P1

- [ ] `system:stats` service principal.
- [ ] Consume message activity events.
- [ ] User message count.
- [ ] Voice-session data model.
- [ ] `/stats`.
- [ ] Basic leaderboard.

## P2

- [ ] Voice time.
- [ ] Session streaks.
- [ ] Music/cinema stats.
- [ ] Achievement engine.
- [ ] `/compare`.
- [ ] Historical graphs.

## P3

- [ ] Advanced workspace analytics.
- [ ] Yearly "Wrapped" report.
- [ ] Premium/retention entitlements.

---

# Milestone G — Built-in Economy

## P1

- [ ] `system:economy`.
- [ ] Currency model.
- [ ] Ledger accounts.
- [ ] Ledger transactions.
- [ ] Ledger postings.
- [ ] Balanced transaction invariant.
- [ ] User wallet creation.
- [ ] `/wallet`.
- [ ] `/pay`.
- [ ] Atomic peer transfer.
- [ ] Property tests for ledger invariants.

## P2

- [ ] Workspace currencies.
- [ ] Treasury/system accounts.
- [ ] Shops/rewards.
- [ ] Economy Stats.

---

# Milestone H — Desktop and web clients

## P1

- [ ] Minimal CLI/dev client for integration testing.
- [ ] Tauri desktop shell.
- [ ] Login.
- [ ] DM list.
- [ ] Message view.
- [ ] Realtime updates.
- [ ] connection state.
- [ ] TeriCrypt verification state.

## P2

- [ ] Workspace/channel UI.
- [ ] Native notifications.
- [ ] tray.
- [ ] local encrypted cache.
- [ ] web client.
- [ ] settings.
- [ ] profiles.
- [ ] service panels.

## P3

- [ ] Mobile client foundation.
- [ ] shared Rust mobile FFI.

---

# Milestone I — Bots and interactions

## P2

- [ ] Application model.
- [ ] Bot identity/token model.
- [ ] bot permissions.
- [ ] gateway intents.
- [ ] slash commands.
- [ ] interaction responses.
- [ ] Webhooks.
- [ ] bot reconnect/resume.
- [ ] bot SDK skeleton.

## P3

- [ ] OAuth-style installs.
- [ ] buttons/selects/modals.
- [ ] developer portal.
- [ ] TypeScript/Python SDKs.

---

# Milestone J — WebAssembly plugins

## P2

- [ ] Wasmtime plugin host.
- [ ] capability manifest.
- [ ] fuel/time limits.
- [ ] memory limits.
- [ ] no ambient filesystem.
- [ ] no ambient network.
- [ ] namespaced plugin storage.
- [ ] WIT API prototype.

## P3

- [ ] package signatures.
- [ ] publisher identities.
- [ ] plugin update lifecycle.
- [ ] plugin management UI.

## P4

- [ ] marketplace.

---

# Milestone K — Voice

## P2

- [ ] WebRTC architecture spike.
- [ ] coturn/STUN/TURN.
- [ ] VoiceRoom and VoiceSession models.
- [ ] basic group VC.
- [ ] mute/deafen.
- [ ] Stats integration.

## P3

- [ ] SFU deployment.
- [ ] encrypted voice architecture.
- [ ] screen sharing.
- [ ] video.
- [ ] voice quality entitlements.

---

# Milestone L — Music + SPICE

## P2

- [ ] `system:music`.
- [ ] `MusicProvider` abstraction.
- [ ] SPICE provider adapter.
- [ ] search/resolve integration.
- [ ] persistent queue model.
- [ ] command plumbing.
- [ ] mock playback state machine.
- [ ] Stats events.

## P3

- [ ] media worker.
- [ ] voice injection.
- [ ] SPICE playback source integration.
- [ ] Music native UI.
- [ ] multiple room queues.
- [ ] always-on Music entitlement.
- [ ] cryptographically deaf output-only mode.

---

# Milestone M — Spatial voice / virtual rooms

## P3

- [ ] spatial position protocol.
- [ ] client-side stereo positioning.
- [ ] distance attenuation.
- [ ] optional HRTF.
- [ ] room layout editor.
- [ ] movable participant positions.
- [ ] Music source position.

## P4

- [ ] room acoustic models.
- [ ] proximity zones.
- [ ] professional virtual-office layouts.

---

# Milestone N — Cinema / Broadcast Rooms

## P3

- [ ] Cinema channel type.
- [ ] CinemaSession.
- [ ] authoritative playback clock.
- [ ] synchronized play/pause/seek.
- [ ] join-in-progress sync.
- [ ] dedicated Cinema UI.
- [ ] Silent/Commentary/Host Commentary/Intermission modes.
- [ ] ephemeral reactions.
- [ ] private text whisper.

## P4

- [ ] private voice whisper.
- [ ] temporary side voice groups.
- [ ] spatial seating.
- [ ] high-bitrate cinema profiles.
- [ ] surround audio.
- [ ] generalized BroadcastRoom.
- [ ] 4K/HDR tiers after capacity testing.
- [ ] spoiler-aware AI assistant.

---

# Milestone O — Collaboration

## P3

- [ ] Whiteboard object.
- [ ] CRDT selection/prototype.
- [ ] real-time whiteboard.
- [ ] task system.
- [ ] project board.
- [ ] collaborative docs.
- [ ] meeting object.
- [ ] meeting notes.

## P4

- [ ] timelines.
- [ ] advanced document revision.
- [ ] professional export/import.
- [ ] enterprise retention controls.

---

# Milestone P — AI platform

## P3

- [ ] `system:ai`.
- [ ] provider abstraction.
- [ ] capability model.
- [ ] context builder.
- [ ] tool execution framework.
- [ ] action audit events.
- [ ] per-user/workspace quotas.
- [ ] one remote provider.
- [ ] one local OpenAI-compatible provider.
- [ ] "summarize what I missed" prototype.

## P4

- [ ] meeting transcription.
- [ ] meeting summaries.
- [ ] task creation from meetings.
- [ ] model routing.
- [ ] BYOK.
- [ ] local-client AI.
- [ ] spoiler-aware cinema assistant.
- [ ] deeper automation/agents.

---

# Milestone Q — Monetization and boosts

## P2

- [ ] Entitlement engine.
- [ ] plan definition schema.
- [ ] personal subscription state.
- [ ] boost ownership.
- [ ] boost allocation.
- [ ] workspace boost aggregation.
- [ ] grace-period behavior.
- [ ] feature gating via entitlements.

## P3

- [ ] Plus/Pro/Supporter plans.
- [ ] workspace boost levels.
- [ ] professional plans.
- [ ] billing provider integration.
- [ ] invoices/payment webhooks.
- [ ] cosmetics.
- [ ] enhanced Stats entitlements.
- [ ] Music/stream quality entitlements.

Important: payment integration should not be allowed to block the Alpha core messaging milestone.

---

# Milestone R — Operations and scale

## P1

- [ ] off-machine PostgreSQL backups.
- [ ] restore procedure.
- [ ] ARM64 deployment.
- [ ] Caddy/TLS.
- [ ] structured metrics.
- [ ] BLOAT maintenance jobs.

## P2

- [ ] Infrastructure-as-code.
- [ ] object-storage abstraction.
- [ ] external attachment storage.
- [ ] rate limiting.
- [ ] bot/plugin quotas.

## P3

- [ ] multi-instance gateway.
- [ ] multi-worker deployment.
- [ ] advanced observability.
- [ ] production SFU.

## P4

- [ ] multi-region strategy if growth actually requires it.

---

# Immediate execution order

1. Inspect the actual repository and complete the authorized SETUP-001–SETUP-005 foundation work.
2. Teri enables verified protections; run the small separate-branch pilot under serial integration.
3. Establish the selected OpenMLS/device interfaces and immediate cryptographic persistence/bootstrap design before claiming a secure DM implementation.
4. Implement user/device/session authentication using the accepted account-security design.
5. Implement conversations, encrypted message envelopes, transactional outbox, and WebSocket delivery.
6. Prove two-client encrypted `bro`, reconnect/history behavior, and negative authorization cases.
7. Add metadata-only Stats and the balanced/idempotent 50-credit Economy transfer.
8. Add the minimal Tauri desktop client and independently owned workspaces/channels/permissions.
9. Add central entitlements and app/bot interfaces as their milestones become unblocked.
10. Add voice and SPICE integration, then the other feature milestones with their relevant tests.
11. Grow queue automation, release feeds, and the larger test portfolio when real code/volume justifies them.

Use `AGENTS.md`, Round 1, and Round 2 throughout. Do not build the full CI organization instead of the application, and do not begin with 4K Cinema or enterprise SSO.
