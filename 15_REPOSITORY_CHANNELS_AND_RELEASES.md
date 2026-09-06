# Round 1 — Repository, Build Channels, and Release Organization

**Status:** accepted direction; implementation and GitHub enforcement are not yet verified.  
**Owner:** Teri. **Date:** 2026-09-06.  
**Companion:** [Round 2: quality gates](16_TESTING_REVIEW_AND_QUALITY_GATES.md).  
**Human setup:** [Teri's checklist](TERI_TODO.md).

## 1. The five independent dimensions

| Dimension | Question it answers | Our vocabulary |
|---|---|---|
| Branch | Where are source changes being integrated? | `main`, short-lived work branches, occasional `release/x.y` |
| Artifact/build | Which exact software bytes are these? | Commit SHA, build ID, digest, target architecture |
| Release channel | Who should receive this build? | Nightly, Canary, Beta, Stable |
| Environment | Where is it running, and whose data is there? | Local, CI, Staging, Production |
| Feature maturity | How ready is a particular feature? | Hidden, Experimental, Preview, GA |

**Stable is not a server. Production is not a branch. Nightly is not a separate codebase.**

Example: an experimental Cinema feature can exist on `main`, be included in a Canary client running against Staging, and remain disabled in Stable. A Beta client may connect to Production only when the supported protocol and security-policy matrix permits it.

Our channel names are project policy, not universal industry standards.

## 2. Repository structure and ownership

Start with one **private platform monorepo**. Keep SPICE in its existing repository and integrate through a versioned adapter contract. Do not move or modify SPICE automatically merely because this pack mentions it.

Keep the numbered planning files together at the repository root initially, preserving their relative links. Keep `AGENTS.md` at the repository root. A later move to `docs/spec/` must update every link and instruction in one reviewed change; do not leave two competing copies.

```text
<platform-repository>/
├── AGENTS.md
├── START_HERE.md
├── TERI_TODO.md
├── 00_README.md ... 17_ENGINEERING_SOURCES.md
├── .github/
│   ├── ISSUE_TEMPLATE/
│   ├── pull_request_template.md
│   ├── CODEOWNERS             # create after actual usernames/teams are known
│   └── workflows/             # agents implement; not active merely from this pack
├── apps/  crates/  services/  clients/
├── migrations/  deploy/  scripts/
└── docs/architecture/adr/
```

The final public name and GitHub organization remain owner decisions. **Do not assume Spice-Production must own the new platform.** Do not create an infrastructure repository until independent access/lifecycle needs justify it.

Teri is the owner/release authority. Ryan is a collaborator and may receive scoped maintainer responsibilities. Neither an agent label nor a team name creates permissions. Agents get no organization-owner credential, billing authority, production access, or implicit release approval.

## 3. Branches and tags

Use trunk-based development:

- `main`: newest integrated, accepted code. Protected; not automatically deployed to Production.
- Short-lived task branches: `feat/123-workspace-invites`, `fix/188-session-revoke`, `agent/t03/392-economy-transfer`.
- `release/0.4`: optional maintenance line only when Stable needs a fix while `main` has incompatible newer work. Forward-port every fix and record linked PRs.
- Immutable release tags: `v0.4.0`, `v0.4.1`, optionally prerelease tags such as `v0.5.0-beta.2`.

Do not create eternal `dev`, `nightly`, `beta`, `stable`, and `production` branches. Channels are publication pointers, not merge destinations.

One deliberate initial import is acceptable for an empty repository. Once multiple writers start, all code changes use branches and PRs. No force-pushing/deleting `main` or rewriting published release tags. Prefer squash merges for ordinary PRs, with a meaningful task-linked message. Preserve appropriate attribution; do not invent authors or add AI co-author trailers unless the owner explicitly requests that convention.

Maintenance releases get the same risk and evidence gates as other releases. “Hotfix” changes queue priority, not the permission to bypass security checks.

## 4. Task flow and the multi-agent workforce

```text
Issue → Ready → claimed work branch → draft PR → review → merge queue → main
```

One task needs: purpose, acceptance criteria, exclusions, priority, risk, owner, permitted paths, dependencies, evidence plan, and rollback/compatibility concerns.

Use one initial project board: Backlog → Design → Ready → In Progress → Review → Merge Ready → Done; Blocked is a separate state with a reason. Board status is canonical. Optional `state:*` labels are mirrors maintained by automation, not a second manually maintained truth.

### Agent coordination

- Name parent threads `T01`–`T10` and `R01`–`R10`; subagent IDs such as `T03-A17` are traceability metadata, not GitHub identities.
- The parent thread owns the task and PR. One issue, one primary implementer, one accountable human/parent thread.
- Each writer uses a separate worktree or checkout and branch. Never share one mutable working directory between concurrent writers.
- Use a task claim with an expiry/heartbeat. Reclaim abandoned work deliberately; do not run two replacement writers blindly.
- Serialize shared hotspots: workspace manifests, dependency lockfiles, migrations, public protocol schemas, generated API files, release policy, and `AGENTS.md`.
- Claim new migration identifiers centrally; check uniqueness and dependency order.
- Review subagents start from the diff, specification, and evidence, not merely the author's confident summary.
- Dependent PRs record their parent. When squash-merging a parent, rebase/update the child and invalidate stale evidence.
- Stop increasing active writers when review or merge capacity is saturated. Two hundred available agents do not require two hundred simultaneous PRs.

**Initial pilot default:** one active writing task per human plus independently scoped reviewers. Expand in measured waves once task claims, evidence, and merging work. This is a scheduling limit, not a limit on how many read-only reviewers or research workers may exist.

## 5. Labels, board fields, and milestones

| Namespace | Examples | Meaning |
|---|---|---|
| `area:*` | auth, tericrypt, gateway, messaging, workspace, voice, cinema, music, spice, stats, economy, ai, desktop, web, infra | Component |
| `type:*` | feature, bug, refactor, docs, security, performance, dependency | Kind of work |
| `priority:*` | P0–P4 | Roadmap urgency |
| `risk:*` | low, normal, high, critical | Change impact and required gate |
| `severity:*` | S0–S3 | Finding severity, not task priority |
| `state:*` | blocked, ready, in-progress, review, merge-ready | Optional automated board mirror |

Milestone, owner, agent/thread, target version, and target channel are board fields. Do not require a `release:stable` label on every feature PR; release promotion is a separate operation. Labels requesting a lower risk or claiming “reviewed” are not trusted authorization.

Initial milestone examples: Alpha 0 — It Sends Bro; Alpha 1 — Friends Can Use It; Alpha 2 — Workspaces; Alpha 3 — Voice; Beta 1; v1.0. These are product checkpoints, not delivery-date commitments.

## 6. Build channels

| Channel | Admission | Intended audience | Publication |
|---|---|---|---|
| Nightly | Accepted `main` build with required baseline checks | Developers and explicit volunteers | Scheduled, only if an eligible new commit exists |
| Canary | Selected build passes staging/candidate gates | Teri, Ryan, selected testers | Controlled promotion |
| Beta | Wider compatibility/upgrade checks pass | Opt-in testers | Controlled promotion |
| Stable | Release gates and human release approval pass | Normal users | Controlled promotion |

Define all four channels now. **Initially activate only an internal Nightly feed; activate Canary/Beta/Stable as their gates become real.** Do not build four parallel release pipelines before the first application works.

“Experimental” describes a feature flag/maturity. “Test” describes a test run/environment. “Production” describes a live installation. None becomes a fifth update channel.

A user explicitly opts into riskier feeds. Switching back to Stable must check local database/protocol compatibility; if the Stable binary cannot read the current cache, wait for a compatible release or use a separately initialized profile. Do not silently delete local keys or try an unsafe downgrade.

## 7. Version numbers versus promotion: no disguised rebuilds

Use SemVer for software versions and a separate unique build ID. A published version must not acquire different bytes later; prerelease identifiers affect version precedence and build metadata does not. See source **R1** in [references](17_ENGINEERING_SOURCES.md).

Record for each artifact:

```text
component, software_version, build_id, git_commit, git_tree
platform/architecture, artifact_digest, build_config_digest
protocol capabilities, database/cache schema compatibility
build provenance, signing evidence, release-gate evidence
```

### Recommended promotion behavior

Server containers/web bundles: promote the exact immutable digest from Staging toward Production. The release feed or deployment record moves; bytes do not.

Desktop packages: renaming `0.4.0-beta.2` to `0.4.0` normally changes embedded metadata and signatures. That is a **new artifact**, not byte-for-byte promotion. Use either:

1. A final-version candidate (`0.4.0`, build ID recorded) distributed only through the Beta feed, then promote those exact bytes; or
2. A newly packaged final-version artifact from the approved source, with new digests/signatures and fresh artifact-dependent release checks.

If a final-version candidate fails, discard it; do not later reuse a published version/build identity for different bytes. Allocate the next release version as needed. Do not fabricate a Stable label over a prerelease binary.

Tauri's update response includes version, artifact URL, and signature; updater signing is separate from platform code signing/notarization. Validate all required signing paths and keep signing keys out of PR jobs. See **R2**.

### Example

```text
main commit a1… → dev build → Nightly
selected commit b2… → final-version candidate 0.4.0 → Beta-only feed
same approved digest, same embedded version → Stable feed → Production rollout
```

Release tags, build records, channel feed versions, and deployment records must all agree. A mutable container tag such as `stable` may be convenient for humans but is not the deployment source of truth.

## 8. Environments and isolation

| Environment | Data | Purpose |
|---|---|---|
| Local | Synthetic/developer fixtures | Fast iteration |
| CI | Fresh synthetic fixtures | Automated verification |
| Staging | Synthetic accounts/content | Pre-release deployment rehearsal |
| Production | Real users/data | Actual service |

Use separate credentials, volumes, database identities, NATS subjects/accounts where relevant, object namespaces, networks, signing/recovery roots, and configuration per environment. Never copy real recovery keys, payment data, or plaintext user content into CI fixtures.

Before real users, an Oracle test VM can host trusted staging services. **Once Production exists, arbitrary agent/PR code must not execute on its VM or access its network/secrets.** Separate Compose projects on one host are organizational separation, not a strong security boundary. Do not colocate development runners with Production. A JIT runner still needs clean compute isolation, not merely a new runner registration. See **R3**.

The GitHub environment approval, secrets, and branch/tag protection feature set depends on the repository plan and visibility. Check the actual settings rather than assuming a private repository has every protection. If independent deployment approval cannot be enforced, keep deployment credentials out of repository-wide secrets and use an owner-controlled release process. See **R4**.

## 9. Feature flags and compatibility

Each flag has an owner, description, safe default, permitted environments/channels, target removal milestone, dependencies, migration impact, and emergency disable behavior.

Feature flags are not authorization. Paid entitlements, role checks, tenant isolation, and cryptographic membership remain enforced on the server/protocol path. Hiding a button proves nothing.

Do not randomly roll out incompatible encryption suites or membership rules per user. A group needs an explicitly supported, authenticated protocol transition. Do not expose an experimental UI against a backend that cannot enforce its semantics.

Keep a capabilities matrix rather than “client version N probably works.” Test old supported clients against new servers and required client upgrade behavior. API, gateway protocol, crypto protocol, SQLite schema, server schema, and plugin ABI versions are independent.

## 10. Merge authority and GitHub plan constraints

Exactly one merge controller owns advancement of `main`. Authors may request queue entry; they do not grant their own final approval. Review evidence is bound to the current PR head and tested candidate/base.

GitHub's documented native merge queue is available for public organization repositories and private organization repositories on Enterprise Cloud. Its Actions integration requires `merge_group` checks. Native merge-limit settings are not the same as cost-saving batch-build settings. See **R5**.

**Private-repository fallback:** use a supported third-party queue, or an explicitly serialized maintainer process while selecting one. Do not assume Mergify is installed, free, or already configured. A bot-based process is not equivalent to branch enforcement if writers can bypass it; record that limitation. Do not publish the private repository merely to obtain a free feature.

Private-repository branch/ruleset and CODEOWNERS capabilities also depend on plan. CODEOWNERS is not itself an enforcement engine, and listing multiple owners on a path does not require every owner to approve. Verify actual protection and use a separate policy check for any additional quorum. See **R6**, **R7**.

PR authors, including agents, must not possess credentials that can forge required controller checks. Keep gate policy in a protected, trusted execution context. Implementation details are in Round 2.

## 11. Release operations and rollback

Promotion is an auditable record: artifact identities, evidence, operator, destination channel/environment, timestamp, rollout cohort, known limitations, and rollback/roll-forward plan. Production needs an explicit owner approval; permission to create a PR is not permission to deploy.

Use expand → migrate → contract for server schemas. Before promotion, test the supported rollback application against the migrated database. If data/schema changes are irreversible, say so and require a roll-forward/restore plan. Restoring a backup can discard newer writes and is not a casual substitute for binary rollback.

On a single VM, use a controlled restart or staged rollout the host can actually support. Do not claim multi-node canary deployment or zero downtime without infrastructure and evidence.

Protect release/update signing material separately from ordinary build jobs. Record emergency actions and issue a repaired release; never silently rewrite tags or disable signature verification to make an update succeed.

## 12. Adoption sequence

1. Create the private repo, grant scoped human access, commit this pack.
2. Add issue/PR conventions and an initial owner map.
3. Implement minimal honest CI; observe a known success and a known failure.
4. Enable protections that the actual GitHub plan supports; record gaps.
5. Trial two independent work branches through serial integration.
6. Add a queue/controller and verify candidate testing plus stale-result rejection.
7. Produce an internal Nightly artifact.
8. Add isolated Staging, then Beta/Stable promotion only when their evidence exists.

The owner-facing choices and delegation prompts are in `TERI_TODO.md`. Remaining vendor, budget, and identity choices stay open in `12_OPEN_DECISIONS.md`.
