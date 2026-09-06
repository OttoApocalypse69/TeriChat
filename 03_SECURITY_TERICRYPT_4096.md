# TeriCrypt-4096™ Security Architecture

> Current decisions in [12_OPEN_DECISIONS.md](12_OPEN_DECISIONS.md) supersede older alternatives in this overview. For agent/release behavior, follow [Round 1](15_REPOSITORY_CHANNELS_AND_RELEASES.md), [Round 2](16_TESTING_REVIEW_AND_QUALITY_GATES.md), and [AGENTS.md](AGENTS.md).

## 1. Branding

**TeriCrypt-4096™** is the permanent working name for the platform security/cryptography suite.

The "4096" is branding.

It does not authorize inventing homemade cryptographic primitives.

## 2. Rule zero

Use established, reviewed, maintained cryptographic primitives and protocol designs.

Do not manually implement:

- block ciphers;
- stream ciphers;
- hashes;
- KDFs;
- elliptic-curve arithmetic;
- post-quantum algorithms;
- signature schemes.

Prefer audited libraries and existing protocol specifications.

## 3. Intended properties

Long-term goals:

- E2EE private messages;
- per-device identities;
- multi-device account security;
- authenticated device onboarding;
- forward secrecy;
- post-compromise security;
- group encryption;
- attachment encryption;
- key verification;
- QR/safety-number verification;
- key transparency;
- post-quantum-capable session establishment;
- encrypted voice/video;
- membership-triggered key rotation;
- protocol versioning.

## 4. Information classes

### Private content

Examples:

- message plaintext;
- attachment plaintext;
- call audio/video;
- screen share content;
- confidential document content where configured.

Goal: E2EE where practical and policy-compatible.

### Activity metadata

Examples:

- a message occurred;
- timestamp;
- sender identity;
- voice session start/end;
- session duration;
- channel/workspace identifiers;
- reaction occurrence.

Some metadata may be visible to the server because routing, abuse prevention, Stats, and synchronization need it.

### Public/application state

Examples:

- usernames;
- avatars;
- roles;
- workspace membership;
- public profile;
- achievements;
- economy balances;
- boost level.

## 5. Device identity

Every device has its own cryptographic identity.

Conceptually:

```text
Teri
├── Desktop → device key
├── Phone   → device key
└── Laptop  → device key
```

New-device onboarding should require strong account authentication and preferably cryptographic approval from an existing trusted device.

The server must not silently substitute a new device key without producing visible identity-change state.

## 6. TeriCrypt protocol surface

Potential protocol identifier:

`TERICRYPT/4096-v1`

Potential error codes:

```text
TC4096_IDENTITY_MISMATCH
TC4096_DEVICE_UNVERIFIED
TC4096_SESSION_DESYNC
TC4096_GROUP_EPOCH_STALE
TC4096_KEY_TRANSPARENCY_FAILURE
TC4096_PQ_HANDSHAKE_FAILED
```

## 7. Do not fake Signal-equivalent security

If Alpha uses a simpler secure envelope proof-of-concept, document exactly which properties exist.

Do not claim:

- forward secrecy;
- post-compromise security;
- post-quantum security;
- anonymous metadata;
- group security;

unless they are actually implemented and reviewed.

## 8. AI and E2EE

AI must not silently bypass E2EE.

Three valid modes:

### Local AI

Client decrypts content and runs a local model.

Server never receives plaintext.

### Explicit AI participant

AI is intentionally added as a cryptographic participant.

UI must clearly state that the AI can read the conversation.

Removing AI should trigger the same membership/key-rotation rules as removing a human participant.

### Managed professional AI

Workspace admins may configure specific workspace resources for managed AI processing.

This must be explicit, policy-controlled, auditable, and visible.

## 9. Bots and E2EE

A public bot receiving `message.created` does not automatically receive plaintext.

If a bot needs E2EE content, it must become an explicit cryptographic participant/capability holder.

## 10. First-party services and privacy

### Stats

Needs activity events, not message plaintext.

### Economy

Needs ledger events, not message plaintext.

### Music

Needs media output capability; it should not require receiving microphone audio.

Music may be deliberately **cryptographically deaf** in voice rooms: capable of sending audio without permission to receive/decrypt user speech.

## 11. Voice/video

Long-term voice/video security should be endpoint-based.

An SFU should route media without obtaining unnecessary plaintext content.

Spatial audio should be performed client-side after the client receives/decrypts independent participant streams.

## 12. Key transparency

Design for an append-only/verifiable mapping from account/device identities to public keys so clients can detect inconsistent key views.

Do not make the central service an unquestionable key oracle.

## 13. Threat model document

Before production use, create a formal threat model covering:

- malicious server;
- compromised database;
- stolen access token;
- stolen device;
- malicious bot;
- malicious plugin;
- malicious workspace admin;
- compromised media worker;
- network attacker;
- supply-chain dependency compromise;
- metadata privacy;
- backup compromise.

## 14. Security invariants

- Never log plaintext private messages.
- Never log passwords.
- Never log session secrets.
- Never log bot tokens.
- Never log cryptographic private keys.
- Never expose one workspace's private state to another.
- Do not let plugins gain ambient filesystem/network access.
- Do not let AI silently expand its own permissions.
