# Security Policy

Security is a primary design requirement of this project. The security suite is branded **TeriCrypt-4096™**, but branding is not a substitute for reviewed cryptography, correct implementation, or honest threat modeling.

## Project status

The repository is currently **pre-alpha / private-development** unless a later release document explicitly states otherwise.

There is no promise that an unreleased build is suitable for protecting sensitive real-world communications. Security claims must be tied to implemented, tested, and reviewed behavior.

## Reporting a vulnerability

**Do not open a public issue for a vulnerability that could expose user data, credentials, cryptographic material, authorization boundaries, billing state, or production infrastructure.**

Preferred reporting path, in order:

1. Use the repository's private vulnerability-reporting / Security Advisory mechanism if it is enabled.
2. Otherwise contact the repository owner/maintainers through an existing private channel already established with them.
3. If neither is available, disclose only enough in a normal issue to request a private contact path; do not include exploit details, secrets, private data, or working attack material publicly.

A dedicated security email/contact may be added later. Do not invent one in documentation before it actually exists.

## What to include

A useful report includes:

- affected revision/version;
- affected component/path;
- prerequisites;
- concrete impact;
- reproduction steps or a minimal proof of concept when safe;
- whether exploitation crosses a tenant, account, device, cryptographic, plugin, AI, billing, or infrastructure boundary;
- suggested mitigation if known.

Avoid including real user messages, passwords, tokens, recovery words, private keys, production dumps, or other unnecessary sensitive data.

## High-priority security areas

Particularly sensitive areas include:

- TeriCrypt/MLS protocol state and key lifecycle;
- device identity, linking, revocation, and key transparency;
- Recovery Vault and recovery proofs;
- passkeys/WebAuthn, TOTP, recovery codes, sessions, step-up authentication;
- workspace/channel authorization and tenant isolation;
- E2EE attachment handling;
- plugin/WASM capability isolation;
- AI authorization and E2EE consent boundaries;
- Economy ledger invariants and real billing separation;
- database migrations and backup/restore;
- CI/release credentials and Production isolation.

## Security boundaries already accepted

Current architecture requires, among other things:

- no homemade low-level cryptographic primitives;
- MLS/OpenMLS as the TeriCrypt foundation;
- independent cryptographic membership per device;
- account recovery and encrypted-identity recovery remain separate;
- recovery words are not sent to the server;
- private E2EE content is not silently sent to remote AI;
- Stats does not automatically receive message plaintext;
- plugins receive explicit capabilities rather than ambient filesystem/network access;
- arbitrary CI jobs do not share Production credentials/security boundaries;
- deleted-message semantics do not falsely promise erasure from malicious endpoints that already retained plaintext.

See `12_OPEN_DECISIONS.md` for the current authoritative decision register.

## Testing and research rules

When testing security:

- use accounts, workspaces, devices, and data you own or have explicit permission to test;
- use synthetic fixtures whenever possible;
- do not intentionally access other users' data;
- do not perform denial-of-service or destructive testing against shared/Production infrastructure without explicit authorization;
- do not socially engineer users or maintainers;
- do not exfiltrate secrets as proof that they were reachable;
- stop once the security impact is demonstrated safely.

Long-running fuzzing, mutation testing, chaos testing, and adversarial review belong in controlled test environments.

## Dependency and supply-chain issues

Report compromised or malicious dependencies, suspicious build scripts, poisoned artifacts, or CI credential exposure as security issues.

Dependency policy/tooling will be pinned during repository bootstrap. Until then, do not add arbitrary Git dependencies or unreviewed binary downloads to trusted build paths.

## Supported versions

Until public releases exist, only the current development baseline is actively considered. Once Beta/Stable channels exist, this section should become a concrete support matrix.

Example future structure:

| Channel/version | Security fixes |
|---|---|
| Stable current | Yes |
| Previous Stable | Time-limited / TBD |
| Beta | Best effort |
| Canary/Nightly | No compatibility guarantee |

Do not publish this example as a promise until a release policy is accepted.

## Disclosure and fixes

For credible reports, maintainers should:

1. acknowledge privately;
2. classify impact and affected versions;
3. reproduce safely;
4. create a regression test where appropriate;
5. prepare and review the fix;
6. rotate/revoke affected secrets or keys when required;
7. publish an advisory after users have a safe remediation path.

Critical cryptographic/authentication findings should not be closed merely because several automated reviewers disagree. Resolve the technical claim with evidence.

## Bug bounty

There is **no implied paid bug bounty** unless the project explicitly announces one in the future.
