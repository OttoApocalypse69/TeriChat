# Repository Bootstrap Session

This is the short agenda for Teri + the initial main/coordinator agent before large-scale multi-agent development begins.

The purpose is to turn the specification pack into a **real, reproducible repository baseline** without prematurely guessing environment details.

## Ground rule

Do not initialize every possible service and tool because it appears somewhere in the long-term roadmap.

Bootstrap only what Alpha 0 requires, while leaving clean extension points.

## 1. Inspect before deciding

The main agent should first report:

- current repository contents/status;
- host OS and architecture(s) expected for development;
- installed Rust/Node/Docker tooling;
- GitHub repository/organization capabilities actually available;
- whether any existing code/config must be preserved;
- constraints on local ports/storage/resources.

Do not change repository settings or install paid integrations merely to complete this checklist.

## 2. Decide the initial repository skeleton

Confirm the minimum real tree for Alpha 0.

Questions:

- Which crates/apps are necessary immediately?
- Which long-term directories should wait until they contain code?
- Where do migrations live?
- Where do integration tests live?
- Where does the minimal dev/CLI client live?
- What is generated vs hand-authored?

Expected direction: one private monorepo, modular monolith first.

## 3. Pin actual toolchains

Only after checking current maintained versions, commit:

- `rust-toolchain.toml`;
- root `Cargo.toml` / Cargo workspace;
- Node package manager and version policy if frontend exists yet;
- package lockfile;
- formatter/linter versions where needed.

Record why unusual pins exist.

## 4. Define local dependencies

For Alpha 0, likely candidates include:

- PostgreSQL;
- NATS if the initial outbox/event slice actually uses it;
- optional local object-storage fixture only when attachments require it.

Decide:

- Docker Compose service names;
- ports;
- healthchecks;
- persistent vs disposable volumes;
- test database isolation;
- ARM64 compatibility.

Do not add Redis/Kubernetes/production orchestration by reflex.

## 5. Define configuration and secrets

Create the real `.env.example` only after configuration keys exist.

For every variable, document:

- purpose;
- safe local default if one exists;
- whether required;
- whether secret;
- allowed environment(s).

Never put real credentials in examples or tests.

## 6. Choose the developer command surface

Decide whether to use `just`, `make`, scripts, or another small command runner.

Aim for obvious commands such as:

```text
dev
check
test
test-full
ci
fmt
lint
migrate
```

The committed commands must run real repository tooling; do not create decorative aliases to nonexistent tasks.

## 7. Establish the first CI baseline

Before adding elaborate quality gates, create a minimal honest workflow for the code that actually exists.

Initial candidates:

- format;
- compile/check;
- Clippy;
- unit tests;
- frontend type/lint/test when frontend exists;
- dependency/security checks once policies are configured;
- ARM64/container build check when practical.

Prove that one intentionally bad change fails and the corrected change passes.

Do not configure Production deployment in the bootstrap CI.

## 8. Decide GitHub controls from actual plan capabilities

Report rather than assume:

- branch/ruleset features available on this private repo;
- merge queue availability;
- Actions minutes/runners;
- private vulnerability reporting;
- environments/secrets availability;
- CODEOWNERS behavior available to the current plan.

Then Teri chooses which controls to enable.

Do not publish the repository merely to unlock a feature without explicit owner approval.

## 9. CODEOWNERS comes after real identities/paths exist

Do not guess usernames or teams.

Once the repository owner confirms actual GitHub identities and critical paths exist, create `.github/CODEOWNERS`.

Expected critical ownership areas eventually include:

- TeriCrypt;
- auth/recovery;
- permissions;
- billing/entitlements;
- Economy ledger core;
- destructive migrations;
- deployment/security policy.

## 10. Produce a bootstrap handoff

The main agent should finish with:

- exact files created;
- toolchain versions chosen and why;
- commands to run locally;
- actual CI checks configured;
- GitHub settings requiring manual owner action;
- unimplemented setup tasks;
- known risks/assumptions;
- the next 3–5 unblocked engineering tasks.

Do not mark Alpha 0 complete merely because the environment boots.

## Owner decision checklist

Teri should only need to answer concrete questions such as:

- "Use Rust X.Y or X.Z?"
- "Enable this branch protection?"
- "Give Ryan this GitHub role?"
- "Use NATS in the first slice or defer until outbox distribution?"
- "Allow this free/paid service?"

The main agent should resolve ordinary reversible engineering details without turning every command into a meeting.
