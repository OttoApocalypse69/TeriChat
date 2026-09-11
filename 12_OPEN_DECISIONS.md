# Open Decisions and Resolved Decisions

This file tracks both:

- decisions that are now **LOCKED** or **PROVISIONALLY LOCKED**;
- decisions that remain genuinely open.

The goal is to prevent already-resolved architecture from repeatedly re-entering discussion while keeping room for changes when implementation/testing exposes a real problem.

---

# 1. Locked / Provisionally Locked Decisions

## A. Identity model — LOCKED

Internal identity:

```text
Account ID
└── immutable UUIDv7 (or equivalent sortable globally unique ID)

Handle
└── globally unique and changeable, e.g. @teri

Display name
└── non-unique and freely changeable
```

Rules:

- Account IDs are the canonical internal identity.
- Handles are human-facing identifiers and must not be used as immutable foreign keys.
- Display names are cosmetic and non-unique.
- Verified email is required for the initial account model.
- Phone number is not required.
- Passkeys/WebAuthn are first-class authentication methods and should be preferred where available.
- TOTP 2FA, hardware security keys, and one-time recovery codes are supported as additional account-security methods.

## B. Account identity vs cryptographic identity — LOCKED

Account state and TeriCrypt state are related operationally but are separate security domains.

Account identity contains things such as:

- handle;
- email;
- subscriptions;
- boosts;
- workspace memberships;
- billing/account metadata.

TeriCrypt identity contains things such as:

- device cryptographic identities;
- encrypted group/session state;
- recovery material;
- encrypted synchronization/bootstrap state.

Recovering the account must not automatically grant the server access to old E2EE plaintext.

## C. Recovery model and Recovery Vault — LOCKED

Account recovery and cryptographic recovery are deliberately separate.

Preferred cross-device path:

```text
new device
    ↓
normal account login / passkey
    ↓
existing trusted device approves via QR/device-link flow
    ↓
encrypted bootstrap material transferred
```

Disaster recovery uses a human-readable TeriCrypt recovery mnemonic.

Working design:

- **24 words** representing **256 bits of machine-generated recovery entropy**;
- user is instructed to store the mnemonic offline;
- the words are never sent to or verified by the server;
- vault decryption happens locally on the recovering client.

### Envelope-encrypted Account/TeriCrypt Vault — LOCKED

The 24-word recovery secret does not directly encrypt every vault object.

Instead:

```text
24-word Recovery Secret
        ↓
HKDF-SHA256 with explicit TeriCrypt domain separation
        ↓
Recovery Wrapping Key
        ↓
unwraps
        ↓
random 256-bit Vault Data Key
        ↓
decrypts
        ↓
Encrypted Account/TeriCrypt Vault
```

The Vault Data Key is independently random. This allows recovery methods to be rotated or added without re-encrypting the entire vault.

Working vault/wrapping AEAD:

```text
XChaCha20-Poly1305
```

The exact Rust crate/provider and serialized blob format must still be implementation-tested, but the envelope-encryption model is resolved.

The server may store only:

- encrypted vault ciphertext;
- wrapped Vault Data Key(s);
- nonces;
- public version/KDF metadata;
- account binding metadata needed to locate the blob.

The server must not possess:

- the 24 words;
- raw recovery secret;
- Recovery Wrapping Key;
- plaintext Vault Data Key;
- vault plaintext.

### Vault contents

The vault may contain recovery-oriented material such as:

- Recovery Identity material;
- secure sync/bootstrap secrets;
- key-transparency recovery state;
- optional encrypted-history backup root key;
- recovery metadata.

It should **not** simply back up every device private key. A recovered device generates a fresh device identity and is newly authorized.

### Recovery proof

After local vault decryption, the client proves recovery by signing a fresh server challenge with a recovered Recovery Identity. The server validates possession without learning the recovery words.

### Old-message history

Recovering MLS membership does not inherently grant historical plaintext. Historical recovery is a separate optional encrypted-backup feature using a dedicated history-backup root key stored inside the vault.

Users may eventually choose:

- encrypted history backup enabled; or
- maximum-paranoia mode where loss of every trusted device permanently loses old history.

### Recovery risk controls — LOCKED

Recovery endpoints use defense-in-depth risk controls:

- per-account rate limiting;
- per-IP/network rate limiting;
- device reputation / known-device signals;
- IP reputation;
- geolocation and impossible-travel signals;
- recent password/passkey/reset activity;
- progressive delays and temporary recovery freezes;
- security notifications to existing trusted sessions/devices.

IP/geolocation are **risk signals, not identity factors**. Users may travel, use VPNs, mobile networks, or inaccurate geolocation databases.

Do not permanently lock an account purely because an attacker intentionally generated failed attempts; that would create a denial-of-service primitive.

Support may help restore ordinary account access after strong verification, but support cannot decrypt TeriCrypt data or reveal recovery words because the service never has them.

Classic security questions are rejected as a recovery mechanism.

### Lost-all-secrets behavior

If all trusted devices and the recovery mnemonic are lost:

- the user may still recover the platform account, subscription, boosts, memberships, etc. through the account-recovery process;
- the old TeriCrypt cryptographic identity/history may be unrecoverable;
- a fresh TeriCrypt identity is generated;
- contacts/workspaces receive a clear identity-reset warning.

Exact mnemonic wordlist/checksum encoding remains open.

## D. TeriCrypt-4096™ protocol direction — LOCKED

Primary protocol:

- **MLS 1.0 / RFC 9420**
- **OpenMLS**
- **RustCrypto provider**

Initial mandatory TeriCrypt-4096™ v1 ciphersuite:

```text
MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519
```

Working protocol branding:

```text
TERICRYPT/4096-v1
```

Rationale:

- this is the MLS mandatory-to-implement interoperable ciphersuite;
- it uses established, widely reviewed primitives;
- OpenMLS is Rust-native and suitable for the project's architecture;
- the provider boundary keeps the platform from being permanently tied to one cryptographic implementation.

Optional future conventional suite:

```text
X25519
ChaCha20-Poly1305
SHA-256
Ed25519
```

Post-quantum direction:

- keep protocol/ciphersuite negotiation extensible;
- experimental hybrid PQ MLS may be implemented behind an explicit experimental identifier;
- do not make experimental PQ MLS the Alpha default;
- do not claim post-quantum security until a concrete stable suite is selected and validated.

### Device / MLS membership model — LOCKED

Every physical/logical client installation is an independent MLS member.

Example:

```text
Teri
├── Desktop → MLS member A
├── Phone   → MLS member B
└── Laptop  → MLS member C

Ryan
├── Desktop → MLS member D
└── Phone   → MLS member E
```

A logical Teri ↔ Ryan DM may therefore contain several MLS members while the UI still presents two human participants.

Benefits:

- new devices can be added independently;
- compromised/stolen devices can be removed independently;
- group epochs rotate when membership changes;
- no account-wide private key needs to be copied to every device.

### MLS credential mapping — LOCKED CONCEPTUALLY

Use OpenMLS BasicCredential-style application-bound identities initially.

Each device has:

- immutable platform device ID;
- device signing key material;
- one or more scoped MLS credentials.

Prefer scoped/random credential identifiers rather than exposing the global account UUID directly inside every MLS group.

The platform Authentication Service maps:

```text
scoped MLS credential
        ↓
account
device
valid/revoked status
signing-key binding
```

This leaves room for future key transparency without making MLS credentials themselves universal tracking identifiers.

Exact serialized credential format and database schema remain implementation details, but the account → many independent device members model is resolved.

## E. Authentication, MFA, and session architecture — LOCKED CONCEPTUALLY

Authentication methods:

- **Passkeys / WebAuthn are first-class and preferred** where available;
- password login remains supported initially, hashed using Argon2id or current equivalent best practice;
- **TOTP authenticator-app 2FA** is supported as a standard fallback/additional factor;
- **hardware security keys** are supported through WebAuthn;
- generate **one-time recovery codes** when MFA is configured;
- email confirmation is useful for recovery/risk assessment but is not treated as strong MFA;
- SMS, if ever supported, is not a preferred/default MFA method.

Account authentication and TeriCrypt recovery are separate:

```text
Passkey / password + MFA
        ↓
proves account access

TeriCrypt 24-word recovery / trusted-device bootstrap
        ↓
proves encrypted-identity recovery
```

A compromised account login must not automatically reveal old E2EE history.

### Step-up authentication — LOCKED

Sensitive actions require fresh/recent strong authentication even if the user already has a valid session. Examples:

- disable MFA;
- add/remove passkeys or recovery methods;
- reset TeriCrypt identity;
- transfer workspace ownership;
- change sensitive billing settings;
- delete account;
- perform other high-impact security operations.

### Session design

- opaque high-entropy session/refresh tokens preferred over giant long-lived self-contained JWTs;
- server stores only secure hashes/records necessary to validate/revoke tokens;
- sessions are individually revocable;
- devices and sessions are separate concepts.

Desktop/mobile:

- session material stored using OS-protected credential storage.

Web:

- use Secure + HttpOnly + SameSite cookies where appropriate.

Device revocation should revoke:

- active authenticated sessions for that device;
- TeriCrypt device participation/bootstrap state as appropriate.

Exact token lifetimes, rotation intervals, and default passkey/password onboarding policy remain implementation decisions.

## F. Workspace/channel encryption modes — LOCKED

Support three explicit channel security modes:

### `E2EE`

- default mode;
- server cannot read plaintext;
- ordinary server-side bots/AI do not receive plaintext.

### `E2EE_WITH_AUTHORIZED_PARTICIPANTS`

- still end-to-end encrypted;
- explicitly approved bots/AI/services may join as cryptographic participants;
- UI must clearly state when such a participant gains message access.

### `MANAGED`

- workspace-authorized server/service processing is permitted;
- intended for professional automation/search/compliance/managed-AI use cases;
- UI must make managed state obvious.

Stats remains metadata-oriented and must not receive message plaintext merely because it is first-party.

## G. E2EE message editing/deletion semantics — LOCKED

Edits:

- represented as new encrypted revisions/events;
- clients render the newest valid revision;
- do not mutate historical ciphertext in place as if the old revision never existed.

Deletion:

- represented as a signed/authenticated tombstone/delete event;
- official clients remove the plaintext from active UI, local message cache, local search index, decrypted attachment/preview cache, and other controlled local state where practical;
- official clients update/cancel OS notifications where the operating system permits;
- the server must not expose a convenient plaintext/archive API that allows a later client to retrieve a deleted message it did not preserve before deletion.

Security property:

> After deletion, official clients apply authenticated tombstones and the service suppresses deleted content from ordinary retrieval. This is not retroactive erasure: an endpoint retaining plaintext **or ciphertext plus sufficient decryption material** may still recover it. Backup restore must reapply tombstones before exposing restored history.

Implementation clarification (2026-09-06): “did not preserve plaintext” alone was too broad a condition. The earlier guarantee must not be used as a security claim against archival/modified clients. This clarification preserves the intended official-client cleanup behavior.

Limitations:

- a malicious/modified endpoint that legitimately decrypted the message before deletion can intentionally archive plaintext;
- screenshots, notification history, accessibility tools, clipboard/logging software, external backups, or another camera cannot be cryptographically erased after the fact;
- therefore the platform must never promise retroactive universal erasure from already-compromised or intentionally archival endpoints.

Notification privacy should support at least:

```text
Full message preview
Sender only
Generic "New message"
```

This minimizes plaintext persistence in OS notification history for privacy-conscious users.

## H. E2EE search strategy — LOCKED FOR INITIAL DESIGN

E2EE conversations use client-side search.

Desktop/mobile maintain local encrypted indexes/caches as needed.

Managed channels may later support server-side search according to workspace policy.

Do not require server plaintext access merely to implement search.

## I. Attachment storage format — LOCKED

Preserve the original file bytes.

Do **not** convert arbitrary attachments to JPEG or another universal format.

Examples that may be stored as encrypted opaque blobs:

- JPG/JPEG;
- PNG;
- WebP;
- GIF;
- PDF;
- ZIP;
- executable/binary files;
- model files;
- audio/video;
- arbitrary user files subject to policy and size limits.

E2EE attachment flow:

```text
original bytes
    ↓
random per-attachment encryption key
    ↓
authenticated encryption
    ↓
ciphertext object/blob storage
```

Attachment keys and sensitive metadata travel inside TeriCrypt-protected message content.

Optimized previews/thumbnails may be generated separately.

For E2EE content, client-side preview generation is preferred where privacy requires it.

Never lossy-convert the original unless the sender explicitly requests optimization.

## J. Frontend and local database — LOCKED

Application UI:

- React;
- TypeScript;
- Tailwind CSS.

Desktop:

- Tauri;
- Rust shared protocol/security/client core.

Web app:

- shared React application where practical;
- Vite is preferred for the actual messenger app unless implementation reveals a strong reason otherwise.

Marketing/docs may use Next.js separately.

Local database:

- SQLite.

SQLite may store:

- local message/cache state;
- sync state;
- drafts;
- encrypted search indexes;
- workspace metadata;
- media metadata.

Sensitive local data should be encrypted/protected using keys rooted in the operating system's secure credential/key storage where feasible.

## K. Workspace terminology — LOCKED INTERNALLY

The backend/domain term is:

```text
Workspace
```

Consumer/community UI may display:

```text
Server
```

Professional/team UI may display:

```text
Workspace
```

This is presentation terminology only; the underlying entity remains Workspace.

## L. Licensing/open-source strategy — PROVISIONALLY LOCKED

Development phase:

- repository remains private while architecture/product direction is still rapidly changing.

Preferred future publication direction:

- **source-available** rather than immediately committing to permissive open source;
- a license in the FSL/BSL family is a current candidate;
- current preference: an **FSL-style source-available license with eventual Apache-2.0 conversion**, subject to legal review before public release.

Reasoning:

- permit source inspection, learning, contribution, and many self-hosting/use cases;
- retain protection against immediate commercial clone competition;
- preserve eventual open-source conversion.

This is provisional until:

- legal review;
- business model is clearer;
- dependency license compatibility is audited.

Do not choose dependencies that unnecessarily force an incompatible licensing outcome.

## M. Workspace authority model — LOCKED CONCEPTUALLY

`Owner` and `Administrator` are not equivalent.

Owner-only examples:

- transfer workspace ownership;
- delete workspace;
- manage billing/subscription;
- perform critical security/encryption-policy changes;
- other catastrophic/irreversible actions.

Administrators may manage:

- channels;
- normal roles;
- moderation;
- invites;
- members;
- bots;
- most workspace settings.

All meaningful capabilities should be represented through configurable permissions rather than one giant admin boolean.

Default Member / `@everyone`-style role should be conservative:

- read allowed channels;
- send messages where allowed;
- react;
- join normal voice rooms;
- use ordinary interactions.

Exact default permission matrix remains open and should be refined through real testing.

## N. Development / CI / first deployment — LOCKED

Development:

- local machine;
- Docker Compose for dependencies/integration testing.

CI:

- GitHub Actions;
- fmt;
- clippy;
- Rust tests;
- frontend checks;
- security/dependency checks where practical;
- ARM64/container build validation.

First live deployment target:

- Oracle Cloud ARM64 Always Free / low-cost VPS, subject to verifying current availability and quotas at deployment time.

Initial single-node services may include:

```text
Caddy
Rust backend/API
Realtime gateway
Worker
PostgreSQL
NATS
Stats
Economy
```

Attachments/object storage must use an abstraction so storage can move off-node later.

Backups must be copied **off the Oracle VM/provider compute instance**.

The live VM must never be the only copy of important data.


## O. Repository/build/release organization — ACCEPTED DIRECTION

See [Round 1](15_REPOSITORY_CHANNELS_AND_RELEASES.md) for the complete policy.

- One private platform monorepo; SPICE remains a separate integration. The final GitHub org/name is not chosen.
- Trunk-based `main`, short-lived work branches, optional maintenance branches, immutable release identities.
- Nightly / Canary / Beta / Stable are update channels, not permanent branches.
- Local / CI / Staging / Production are distinct data/execution environments.
- Feature maturity and flags are separate from channel, environment, authorization, and protocol compatibility.
- One merge authority; scoped agent worktrees, task claims, and shared-file coordination.
- Prefer immutable artifact promotion; rebuilding/repackaging requires renewed artifact-specific evidence.
- Verify the real private-repo plan's enforcement capabilities before choosing a queue/protection setup.

These are accepted design decisions. Actual GitHub configuration, deployments, and tested enforcement remain pending SETUP tasks.

## P. Testing/review/quality system — ACCEPTED DIRECTION

See [Round 2](16_TESTING_REVIEW_AND_QUALITY_GATES.md).

- Roadmap priority P0–P4, PR risk Low–Critical, and finding severity S0–S3 are distinct.
- Use risk-based independent review and behavioral/negative-case tests, not raw reviewer/test counts.
- CI has preflight, full PR, merge-candidate, main-smoke, scheduled stress, and release lanes.
- Serious findings block; evidence is tied to exact revisions. Missing/skipped/stale evidence is not a pass.
- Critical includes the review/CI policy, agent instructions, workflows, signing, and release machinery itself.
- Human approval remains mandatory for Critical changes and Production/Stable release, with competent specialist review for material security claims.
- PR execution cannot receive Production credentials or run on the Production host.
- Start with baseline CI and a serial pilot; the complete automation is staged, not an Alpha prerequisite.

No test run, installed bot, or configured protection is implied by accepting this section.

---

# 2. Remaining Open Decisions

## A. Product identity

- [ ] Final platform name to replace `TeriChat`.
- [ ] Domain name.
- [ ] Visual identity/logo.
- [ ] Name for the built-in AI assistant/runtime.
- [ ] Name for Cinema vs Watch Room vs Theater.
- [ ] Name for the generalized BroadcastRoom primitive.

Resolved:

- [x] Internal domain term is `Workspace`.
- [x] Social UI may say `Server`; professional UI may say `Workspace`.

## B. Repository/organization

- [ ] Final monorepo boundaries once media/client scale justifies splitting anything.
- [ ] GitHub organization/repository naming.
- [ ] Final public license after legal/dependency review.
- [ ] Plugin SDK license.

Resolved/provisional:

- [x] Start private.
- [x] Source-available is preferred over immediately committing to permissive open source.
- [x] FSL-style eventual Apache conversion is the current preferred direction, pending review.

## C. Security

- [x] Exact OpenMLS ciphersuite/provider configuration — OpenMLS + RustCrypto + MLS mandatory X25519/AES-128-GCM/SHA-256/Ed25519 suite.
- [ ] Exact MLS credential representation for devices/accounts.
- [ ] Exact persistence model for MLS group state.
- [ ] Audit-warning disposition for the OpenMLS graph (proc-macro-error2 RUSTSEC-2026-0173 via hax/libcrux; baseline rsa RUSTSEC-2023-0071 with no fixed version).
- [ ] Specialist crypto review trigger and scope before any production E2EE claim.
- [ ] Exact post-quantum upgrade path and when it becomes stable/default.
- [ ] Key transparency implementation.
- [ ] Metadata minimization policy.
- [x] Exact Account/TeriCrypt Vault construction — envelope encryption with 256-bit recovery secret, HKDF-SHA256 wrapping key, random 256-bit Vault Data Key, local decryption, XChaCha20-Poly1305 working AEAD.
- [x] Recovery mnemonic derivation/encoding specification — BIP-39 English, 256-bit entropy plus 8-bit checksum as exactly 24 words with NFKD/lowercase/whitespace handling and versioned format policy (`terirecovery-v1`; issue #11, PR #33).
- [ ] Multi-device encrypted bootstrap format.
- [ ] Key backup rotation/revocation policy.
- [ ] Encrypted attachment chunking/streaming format for large files.
- [ ] Formal threat model and external security review timeline.

Resolved:

- [x] MLS/OpenMLS is the primary TeriCrypt direction.
- [x] No homemade low-level crypto.
- [x] PQ remains optional/experimental until sufficiently mature.
- [x] DMs/group DMs are E2EE.
- [x] Workspace channels support E2EE, E2EE with explicit cryptographic app participants, and Managed modes.
- [x] E2EE search is client-side initially.
- [x] E2EE edits are revisions; deletes are tombstone events.
- [x] Account recovery and cryptographic recovery are separate.

## D. Identity/authentication

- [ ] Exact password/session token lifetimes.
- [ ] Refresh/session rotation policy.
- [ ] Anonymous/guest accounts?
- [ ] Account recovery proof requirements when email access is lost.
- [ ] Age requirements/moderation implications if public growth occurs.

Resolved:

- [x] immutable UUIDv7-like Account ID;
- [x] unique changeable handle;
- [x] non-unique display name;
- [x] verified email initially required;
- [x] phone number not required;
- [x] opaque revocable sessions preferred;
- [x] device and session are separate concepts;
- [x] passkeys/WebAuthn are first-class and preferred;
- [x] TOTP is supported as 2FA;
- [x] hardware security keys are supported through WebAuthn;
- [x] one-time MFA recovery codes are supported;
- [x] sensitive operations use step-up authentication;
- [x] SMS is not a preferred/default MFA method.

## E. Workspaces

- [ ] Maximum free workspace size.
- [ ] Exact permission bitset/capability catalog.
- [ ] Exact default Member permission matrix.
- [ ] Default invite expiry/use limits.
- [ ] Public/discoverable servers eventually?
- [ ] Workspace ownership transfer workflow.
- [ ] Federation/self-hosted interoperability eventually?

Resolved:

- [x] Owner is distinct from Admin.
- [x] Catastrophic/billing/security operations are owner-only by default.
- [x] Workspace permissions remain configurable.

## F. Messaging

- [ ] Retention defaults.
- [ ] Thread implementation details.
- [ ] Maximum message size.
- [ ] Attachment size limits per entitlement/plan.
- [ ] Client cache retention.
- [ ] Offline-send conflict semantics.

Resolved:

- [x] E2EE edits are encrypted revisions/events.
- [x] E2EE deletes are authenticated tombstones; official clients purge controlled local plaintext state and the server does not offer post-delete plaintext retrieval.
- [x] E2EE search is local/client-side.
- [x] Original attachment bytes are preserved.

## G. Clients

- [ ] Secure OS key-storage implementation per platform.
- [ ] Exact desktop SQLite schema/cache policy.
- [ ] Whether the web client supports every TeriCrypt feature or some require native clients.
- [ ] Mobile UI stack: React Native vs Flutter vs native.
- [ ] Local encrypted search/index implementation.

Resolved:

- [x] React + TypeScript + Tailwind.
- [x] Tauri desktop.
- [x] SQLite local DB.
- [x] Vite/shared React app preferred for the messenger UI.
- [x] Rust shared client/security/protocol core.

## H. Voice/video

- [ ] SFU implementation/provider/library.
- [ ] Voice codec/bitrate defaults.
- [ ] Video codec priority.
- [ ] Maximum free call sizes.
- [ ] Screen-share quality targets.
- [ ] Exact encrypted-media/SFU trust model.
- [ ] Whether spatial audio is opt-in per room or workspace.
- [ ] HRTF library.
- [ ] Room coordinate model and limits.

## I. Cinema

- [ ] Final name: Cinema / Theater / Watch Room.
- [ ] Free quality target.
- [ ] Boost quality ladder.
- [ ] 4K requirements.
- [ ] HDR timeline.
- [ ] Surround formats.
- [ ] Exact synchronization tolerance.
- [ ] Host-controlled vs democratic play/pause/seek.
- [ ] Whisper voice semantics.
- [ ] Whether timestamp comments persist by default.
- [ ] Movie-source permissions/legal policy.
- [ ] Generalized BroadcastRoom API.

## J. Music/SPICE

- [ ] SPICE API contract specifically for this platform.
- [ ] Local SPICE runtime beside Music workers vs remote SPICE service.
- [ ] Authentication between platform and SPICE.
- [ ] Playback-source handoff format.
- [ ] Caching policy.
- [ ] Exact split of playback state between platform and SPICE.
- [ ] Always-on Music shutdown/idle behavior.
- [ ] Multiple Music identities vs one service principal with many sessions.

## K. Stats

- [ ] Which Stats are global-account vs workspace-specific.
- [ ] Privacy controls for Stats.
- [ ] Whether users can opt out of leaderboards.
- [ ] Whether sleep-in-VC counts fully, partially, or deliberately remains a competitive meta.
- [ ] AFK detection.
- [ ] Achievement catalog.
- [ ] Historical retention by plan.

## L. Economy

- [ ] Default virtual currency naming.
- [ ] Per-workspace currency customization.
- [ ] Daily rewards.
- [ ] Shops.
- [ ] Auctions/games.
- [ ] Inflation controls.
- [ ] Which Economy metrics are public.

Resolved:

- [x] Economy may be disabled per workspace.
- [x] Real-money billing and virtual Economy remain strictly separate.
- [x] Virtual Economy uses a double-entry ledger.

## M. Bots/plugins

- [ ] Public API versioning guarantees.
- [ ] Bot rate limits by plan.
- [ ] Exact OAuth model.
- [ ] Plugin WIT API.
- [ ] Plugin signing authority.
- [ ] Whether plugins can render custom UI components.
- [ ] Marketplace moderation.
- [ ] Paid plugins eventually?

Resolved:

- [x] Bots and plugins are distinct.
- [x] Plugins are Wasmtime/WASM capability-sandboxed.
- [x] No ambient filesystem/network by default.

## N. Collaboration

- [ ] CRDT implementation/library.
- [ ] Whiteboard file format.
- [ ] Document model.
- [ ] Task/project complexity.
- [ ] Calendar integration.
- [ ] Export/import formats.
- [ ] Which collaboration modules are free vs professional-only.

## O. AI

- [ ] Default hosted AI provider(s).
- [ ] Whether the platform sells its own AI credits.
- [ ] BYOK.
- [ ] Local model requirements.
- [ ] AI provider retention/privacy requirements.
- [ ] Exact AI confirmation/risk policy.
- [ ] Meeting transcription provider/local model.
- [ ] AI data retention.
- [ ] Cost/token budgets.
- [ ] Agent background-task model.
- [ ] Whether professional admins can mandate workspace AI policy.

Resolved:

- [x] AI is provider-agnostic.
- [x] AI operates through explicit capabilities.
- [x] E2EE plaintext is never silently sent to remote AI.
- [x] AI can participate in E2EE only through explicit local processing or explicit cryptographic membership/policy.

## P. Monetization

- [ ] Final Free/Plus/Pro/Supporter pricing.
- [ ] Whether Plus includes 0 or 1 boost.
- [ ] Final Pro boost count.
- [ ] Supporter pricing/boost count.
- [ ] Individual boost purchase price.
- [ ] Boost thresholds for workspace levels.
- [ ] Grace period duration.
- [ ] Exact feature matrix.
- [ ] Professional Team/Business pricing.
- [ ] AI credit pricing.
- [ ] Storage pricing.
- [ ] Taxes/VAT/payment-provider strategy.
- [ ] Refunds/chargebacks.
- [ ] Regional pricing/PPP eventually?

Resolved:

- [x] Security is not premium-only.
- [x] Monetization resolves to centralized effective entitlements.
- [x] Boosts are user-owned and workspace-allocated.
- [x] Boost levels should correspond broadly to real infrastructure cost.
- [x] Real billing and virtual Economy are separate.

## Q. Infrastructure

- [ ] Exact Oracle region/instance shape at deployment time.
- [ ] Verify current Always Free quotas/reclaim rules before deployment.
- [ ] PostgreSQL same-VM vs managed DB migration trigger.
- [ ] NATS durability configuration.
- [ ] Object-storage provider.
- [ ] Off-provider backup destination.
- [ ] CDN strategy.
- [ ] Media/SFU hosting once the first VPS is insufficient.
- [ ] Observability stack.
- [ ] Secrets-management system.
- [ ] IaC choice: Terraform vs OpenTofu.
- [ ] Container orchestration if/when scaling actually requires it.

Resolved:

- [x] local Docker Compose development;
- [x] GitHub Actions CI;
- [x] ARM64 compatibility;
- [x] Oracle/low-cost ARM VPS is the first deployment target;
- [x] off-machine backups are mandatory;
- [x] compute is replaceable.

## R. Governance and moderation

- [ ] Platform-wide Terms of Service.
- [ ] Abuse/reporting flow.
- [ ] Workspace moderation responsibilities.
- [ ] Data deletion/account export.
- [ ] Public-server content rules if discovery is added.
- [ ] Bot/plugin abuse review.
- [ ] AI-generated-content policy.

## S. Professional/enterprise

- [ ] SSO timeline.
- [ ] SCIM.
- [ ] Data residency.
- [ ] Legal hold.
- [ ] DLP.
- [ ] Audit export.
- [ ] Enterprise self-hosting.
- [ ] SLA/support model.

## T. Scope discipline

The following intentionally remain unresolved until real usage requires them:

- [ ] exact maximum scale target;
- [ ] multi-region architecture;
- [ ] Kubernetes;
- [ ] federation;
- [ ] marketplace economics;
- [ ] enterprise compliance certifications.

Do not solve hypothetical scale before users are actually using the platform.

---

# 3. Next Decision Queue, Ordered by Priority

## P0 — Resolve during Alpha 0 engineering

1. [x] OpenMLS ciphersuite/provider configuration — resolved.
2. [x] MLS credential/device mapping — resolved.
3. [x] Account/TeriCrypt Vault construction and online recovery-defense model — resolved.
4. [ ] **Recovery mnemonic wordlist/checksum encoding.**
5. [ ] Device-link/bootstrap protocol.
6. [ ] MLS persistence/group-state schema.
7. [ ] Session/refresh token lifetime and rotation.
8. [ ] Large encrypted attachment chunking/streaming format.
9. [ ] Formal Alpha threat-model draft.

## P1 — Resolve before wider private alpha

10. [ ] Exact default workspace permission matrix.
11. [ ] Invite defaults and ownership-transfer workflow.
12. [ ] Secure local-key storage on Windows/Linux/macOS.
13. [ ] Local encrypted search/index design.
14. [ ] Backup provider + restore procedure.
15. [ ] Initial object-storage provider.
16. [ ] Attachment/message quota defaults.
17. [ ] Stats privacy/leaderboard opt-out behavior.

## P2 — Resolve before public beta / monetization

18. [ ] Final public licensing decision.
19. [ ] Plan/boost pricing and feature matrix.
20. [ ] Payment provider + VAT/refund policy.
21. [ ] Public bot API/OAuth guarantees.
22. [ ] Public moderation/reporting/TOS.
23. [ ] AI provider/privacy/cost policy.
24. [ ] Voice/SFU implementation choice.

## P3 — Resolve when the corresponding feature enters implementation

25. [ ] SPICE integration contract.
26. [ ] Whiteboard CRDT.
27. [ ] Cinema/Broadcast quality and synchronization policies.
28. [ ] Spatial-audio/HRTF architecture details.
29. [ ] Professional collaboration packaging.
30. [ ] Meeting transcription/AI pipeline.

## P4 — Resolve only when growth proves the need

31. [ ] Federation.
32. [ ] Multi-region.
33. [ ] Kubernetes/container orchestration.
34. [ ] Marketplace economics.
35. [ ] Enterprise compliance programs.
36. [ ] 4K/HDR/surround guarantees at scale.

---

# 4. Change-control rule

A LOCKED decision may still change if implementation, testing, security review, legal review, or real user behavior reveals a strong reason.

When changing one:

1. record why the old decision failed;
2. update this file;
3. update the relevant architecture/security/roadmap docs;
4. add an ADR when the change is architecturally significant.

"Locked" means **do not repeatedly reconsider without evidence**, not "never change under any circumstances."


---

# 5. Engineering setup choices still open

These belong to the new organizational suite; they do not replace the next product/security P0 (recovery-word encoding).

| Priority | Choice | Decision owner | Default while unresolved |
|---|---|---|---|
| P0 | Actual GitHub organization/repo and real Teri/Ryan usernames | Teri | Keep existing chosen repo private; do not guess identities |
| P0 | Available private-repo protections and budget | Teri + coordinator | Capability report; no new paid service |
| P0 | Code owners and shared-file scheduling authority | Teri | One coordinator; owner review for Critical work |
| P1 | Native versus third-party merge queue | Teri | Explicitly serialized integration, recorded enforcement gaps |
| P1 | Enforced check names and supported toolchain/target matrix | Engineering | Implement real checks before requiring them |
| P1 | Dedicated ARM runner versus hosted capacity | Teri + operations | No arbitrary CI on Production |
| P1 | Artifact registry, signing custodians, and feed hosting | Teri + release | Internal builds only until approved |
| P1 | Concrete backup destination and staging isolation | Teri + operations | No real-user rollout without a tested restore |
| P2 | Quarantine expiry, coverage baseline, and per-lane budgets | Test lead + Teri | Conservative bounded defaults; no silent skips |
| P2 | Broader release cohorts and platform support promises | Teri + release | Explicitly tested platforms only |

Implementation status lives in the SETUP backlog, not in this accepted-decision register.
