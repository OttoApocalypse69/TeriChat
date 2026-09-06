# Contributing

Thank you for contributing to the platform. The temporary project codename is **TeriChat**; the security suite is **TeriCrypt-4096™**.

This repository is initially private and pre-alpha. The goal of these rules is to keep early work understandable while leaving room for the repository and development environment to be bootstrapped deliberately.

## Read first

Before making a meaningful change:

1. Read `AGENTS.md`.
2. Read the assigned issue/task and its acceptance criteria.
3. Read `12_OPEN_DECISIONS.md` for current locked and open decisions.
4. For repository/release work, read `15_REPOSITORY_CHANNELS_AND_RELEASES.md`.
5. For tests/review/CI work, read `16_TESTING_REVIEW_AND_QUALITY_GATES.md`.
6. Read only the subsystem documents relevant to the change.

Accepted owner decisions and superseding ADRs override older proposals. Existing code and actual test output describe implementation reality; planning documents describe intent until implemented.

## Early bootstrap mode

The repository will **not** begin with a 200-agent swarm.

During bootstrap:

- one coordinator/main agent may establish the initial workspace and development environment;
- keep active writers small and non-overlapping;
- prefer reversible setup decisions;
- document environment/tooling choices as they become real;
- do not install paid services, merge bots, production infrastructure, or broad repository permissions without explicit owner approval;
- do not invent environment variables, ports, secrets, or deployment topology merely to fill in documentation.

See `docs/development/BOOTSTRAP_SESSION.md` for the initial setup discussion.

## Work item workflow

For non-trivial work, prefer:

```text
issue/task
   ↓
scoped branch/worktree
   ↓
implementation + tests
   ↓
PR
   ↓
review / CI
   ↓
authorized integration
```

During the very first bootstrap commits, the owner may choose a lighter flow. Once concurrent writers begin, use short-lived branches and PRs consistently.

### Branch names

Use concise task-linked names where possible:

```text
feat/123-workspace-invites
fix/188-session-revocation
refactor/247-gateway-state
infra/301-ci-bootstrap
agent/t03/412-economy-transfer
```

Do not create permanent `dev`, `beta`, `nightly`, `stable`, or `production` branches merely to represent release channels. See Round 1.

## Scope and ownership

Before editing:

- inspect repository status;
- identify paths you will touch;
- check for another writer modifying shared manifests, migrations, lockfiles, generated files, protocol schemas, or policy documents;
- keep changes scoped to the task;
- do not overwrite unrelated work.

If a task requires an accepted trust boundary, license, provider, production topology, payment policy, account-recovery policy, or cryptographic decision to change, stop that part and request owner input.

## Commits

Prefer small, coherent commits with meaningful messages.

Examples:

```text
feat(auth): add device session revocation
fix(gateway): preserve resume sequence after reconnect
test(economy): cover concurrent transfer invariant
docs(security): record recovery vault decision
```

Do not add AI co-author trailers unless explicitly requested. Preserve actual human authorship and repository policy.

## Pull requests

A PR should state:

- the task/issue it addresses;
- what changed;
- acceptance criteria;
- risk classification where known;
- tests/checks actually run;
- checks not run and why;
- migration/compatibility/rollback implications;
- security/privacy impact where relevant.

Use `.github/pull_request_template.md` and `templates/TASK_HANDOFF.md` where applicable.

## Tests and quality

The long-form quality policy is in `16_TESTING_REVIEW_AND_QUALITY_GATES.md`.

Core rules:

- every bug fix should normally add a regression test;
- do not weaken or delete a failing test just to obtain green CI;
- do not claim a check passed if it was not run;
- security-critical code requires stronger evidence than ordinary UI code;
- test behavior and invariants, not only line coverage.

## Security-sensitive changes

Changes touching the following are high-risk or critical by default:

- `TeriCrypt-4096™` and MLS state;
- account recovery and the Recovery Vault;
- authentication, MFA, passkeys, sessions;
- permissions/authorization;
- billing and entitlements;
- Economy ledger invariants;
- production secrets/deployment boundaries;
- migrations that can lose or reinterpret data.

Do not invent cryptographic primitives. Do not claim security properties that have not been implemented and tested.

See `SECURITY.md` for vulnerability reporting.

## Dependencies

Before adding a dependency, consider:

- maintenance status;
- license compatibility;
- security history/advisories;
- ARM64 support;
- WASM/mobile compatibility if relevant;
- whether the standard library or an existing dependency already solves the problem.

The exact automated dependency policy will be committed during repository bootstrap rather than guessed in advance.

## Documentation and ADRs

Use an ADR when a decision is architecturally significant, difficult to reverse, or changes an accepted boundary.

ADR rules and template:

- `docs/adr/README.md`
- `docs/adr/0000-template.md`

Do not create an ADR for every variable name or ordinary implementation detail.

## Release authority

Contributing code does not grant authority to:

- merge Critical changes;
- publish releases;
- deploy Production;
- purchase services;
- modify billing;
- rotate real secrets;
- change repository visibility/access;
- weaken branch/security rules.

Those actions require the appropriate owner/maintainer authorization.

## Current milestone

The immediate product milestone remains:

**Alpha 0 — It Sends Bro**

A sophisticated future feature is not more important than getting the secure core working, testable, and understandable.
