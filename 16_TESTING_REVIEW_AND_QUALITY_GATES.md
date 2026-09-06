# Round 2 — Testing, Review, CI, and Release Quality Gates

**Status:** accepted direction; not evidence that any tests, bots, or gates exist yet.  
**Date:** 2026-09-06.  
**Companion:** [Round 1](15_REPOSITORY_CHANNELS_AND_RELEASES.md).  
**Agent operating rules:** [AGENTS.md](AGENTS.md).

> Cheap changes move quickly. Dangerous changes receive proportionate, evidence-based scrutiny.

## 1. What this system must accomplish

Every integrated change has a known author/task, classified impact, independently evaluated evidence, and a tested relationship to current `main`. Every promoted artifact can be traced to those decisions.

A hundred agreeing agents do not prove correctness. A passing test run does not establish a cryptographic security proof. Human approval is accountability, not a substitute for expertise. Security-critical protocol designs require competent review before meaningful production security claims.

The full intended flow is:

```text
scoped issue → author preflight → independent review → PR checks
       → finding triage → merge-candidate checks → main smoke checks
       → Nightly → selected Canary → Beta → approved Stable artifact
                              ↘ scheduled stress/fuzz/mutation campaigns
```

Activate this incrementally. Alpha 0 does not need a marketplace, six reviewer bots, or a custom CI scheduler before it can exchange `bro`.

## 2. Keep priority, risk, and severity separate

**Priority P0–P4:** when the roadmap needs work.  
**Risk Low/Normal/High/Critical:** how much scrutiny a proposed change requires.  
**Severity S0–S3:** the impact of an observed finding.

| Severity | Meaning | Disposition |
|---|---|---|
| S0 | Catastrophic security boundary failure, major data loss, or similarly severe incident | Block affected merge/release; incident response if live |
| S1 | Concrete serious correctness, privacy, security, or integrity defect | Block until fixed or shown inapplicable with evidence |
| S2 | Meaningful but bounded defect | Track; block if it violates this PR's acceptance criteria |
| S3 | Nonessential cleanup/style suggestion | Nonblocking unless an existing mechanical rule fails |

Automated labels are suggestions until trusted policy evaluates them. An author cannot lower risk or close a blocking finding by editing the PR description.

## 3. Risk policy

| Risk | Typical scope | Independent review | Minimum evidence |
|---|---|---|---|
| Low | Ordinary prose, cosmetic UI with no sensitive behavior | One general pass | Relevant preflight and candidate policy gate |
| Normal | Ordinary API/UI/Stats behavior | Correctness plus test-gap role | Preflight, affected integration, candidate checks |
| High | Additive migrations, gateway concurrency, media/native parsing, plugin integration | Correctness, domain/data/concurrency, security where relevant | Full affected stack plus specialized tests; owner routing |
| Critical | Crypto, recovery, auth, permissions, ledger core, billing, secrets, destructive migrations, sandbox boundaries, CI/release policy | Correctness, security, relevant domain and adversarial/test-gap roles; human owner approval | Full relevant suite, targeted adversarial evidence, isolated candidate, owner acceptance |

Initial path floors include:

```text
crates/tericrypt/**              critical
crates/auth/**                   critical
crates/permissions/**            critical
services/economy/**              high; ledger core critical
billing/payment paths           critical
plugin sandbox/host capabilities critical
migrations/**                   high; destructive changes critical
.github/workflows/**            critical
.github/CODEOWNERS               critical
AGENTS.md and nested agent rules critical
gate classifiers/merge scripts   critical
release/signing/deploy scripts  critical
ordinary docs/**                 low unless they alter authoritative policy
```

Paths are only a floor. A change to a frontend login screen can leak a secret; a dependency upgrade can alter crypto; a build script executes code. Shared manifests, lockfiles, code generation, and transitive dependency changes must expand affected scope. Unknown affected scope fails closed to broader evaluation, not a docs-only pass.

The classifier and gate policy must run from a trusted revision. Changes to the classifier itself cannot lower the requirements for their own PR.

## 4. Independent review roles

Roles: Correctness, Test Gap, Security, Architecture, Concurrency, Data Integrity, Performance, Adversarial, and Review Arbiter.

For Normal PRs, two focused passes are usually enough. For Critical PRs, select the relevant four-to-six perspectives rather than assigning six identical prompts. Multiple roles may run on the same approved model/provider, but that does not make their errors statistically independent. Separate context, evidence, and viewpoints matter more than headcount.

Authors do not self-approve. A review subagent should not mutate the author's branch. It may produce a reproduction patch on its own branch. When all automated agents share a GitHub principal, reports are evidence, not distinct human/GitHub approval identities.

Every finding records:

```text
finding_id, severity, affected_path/symbol, head_sha
claim, impact, preconditions, concrete failure path
reproduction/test or reason reproduction is unsafe/unavailable
confidence, owner, disposition, supporting evidence
```

Use [the finding template](templates/REVIEW_FINDING.md). Do not upload secrets or real private messages as proof.

The arbiter deduplicates and checks plausibility. It does not erase an unresolved security concern merely because another agent disagrees or no test can reproduce it yet. Disputed serious findings go to the appropriate human/domain owner. Preserve the original claim and disposition. A fix or rebase that changes relevant code invalidates stale review evidence.

## 5. Author preflight and completion rules

A bug fix should normally include a regression test that fails on the buggy baseline and passes on the fix. Where this is genuinely infeasible or unsafe, record the reason and equivalent evidence; do not invent a test run.

Features need behavior/negative-case tests tied to acceptance criteria. Prefer externally observable outcomes and domain invariants over assertions on private implementation details.

Candidate commands, **only once the workspace exists and these configurations are implemented**:

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo nextest run --workspace --locked
cargo test --workspace --doc --locked
cargo deny check
```

Use the repository's chosen frontend package manager and committed lockfile; discover actual scripts rather than assuming `npm test` exists. Pin tool versions in reviewed configuration. Do not blindly add `--all-features` if feature combinations are mutually exclusive. Enumerate supported targets/features explicitly.

Use `forbid(unsafe_code)` for first-party crates that do not need unsafe operations. Any exception needs local justification and review. This does not certify transitive dependencies.

A PR report states commands, exact source revision, result, evidence location, and unrun checks. Missing prerequisites mean `BLOCKED`/`NOT RUN`, never `PASS`.

## 6. CI lanes

| Lane | Trigger | Work | Gating role |
|---|---|---|---|
| Preflight | PR updates/local | Format, compile, lint, targeted unit/component tests, secret scan, config validation | All applicable PRs |
| Full PR | Ready PR/risk routing | DB/NATS integration, API/permission contracts, migrations, supported platform builds | Affected Normal+ and sensitive scope |
| Merge candidate | Queue-selected combined revision | Fresh synthetic stack and cross-component acceptance against current base | Required for integration |
| Main smoke | Actual merged revision | Startup, core E2E, policy consistency | Stops promotion if red |
| Nightmare | Scheduled or explicitly selected | Bounded fuzzing, mutation, long properties, load/soak/chaos, restore campaigns | Findings can block affected merges/releases |
| Release | Candidate artifact | Install/update/signatures, backward compatibility, staging, restore/rollback | Required before the relevant channel promotion |

### Scheduling and cost controls

Start with conservative configurable concurrency: two ordinary Linux jobs and one ARM64 job; increase after measuring memory, queue latency, failure rate, and spend. These are initial resource limits, not hardware capacity promises. Set per-job timeouts, maximum artifacts, cache budgets, and monthly spend alerts.

Cancel superseded PR runs using a PR-scoped concurrency key. Do not cancel unrelated PRs or the active Production deployment. Preserve the logs of failed attempts. Group deployment concurrency by environment and serialize schema-changing jobs.

Share safe compiler/dependency caches where worthwhile, but separate untrusted PR caches from release/signing environments. Never cache credentials, decrypted user data, or production configuration. A warm cache is an optimization; a clean rebuild must remain possible.

## 7. Required checks: make missing coverage impossible to disguise

Use one always-reporting policy result, for example `policy/merge-gate`, backed by structured job evidence. The controller calculates the required set from trusted policy and the complete diff.

| Raw result | Gate interpretation |
|---|---|
| Pass on required current revision | Eligible evidence |
| Fail | Block |
| Missing, canceled, timed out, unknown, or stale | Block |
| Not applicable, justified by trusted classifier | Accept the scoped omission |
| Unexpected skip / zero tests when tests were expected | Block |
| Passed only after retry | Flaky; apply the flake policy, not a silent clean pass |

Do not use workflow-level path filters that leave required checks permanently pending. Run the dispatcher/summary on every applicable event; select jobs inside it. The summary must inspect dependency results and cannot return success just because it ran with `always()`.

GitHub-native queues require the `merge_group` event and evidence for its candidate SHA. Third-party controllers require their documented candidate mechanism. Never report green for the PR head and treat that as proof of a different candidate. See **R5** and **R8** in [sources](17_ENGINEERING_SOURCES.md).

Store the base SHA, head SHA, candidate SHA/tree, policy revision, reviewer evidence, toolchain/config digests, and test results. Bind required checks to trusted reporting apps where the host supports it. If the plan cannot enforce that boundary, record the limitation and keep integration owner-controlled.

## 8. Merge controller behavior

One writer advances `main`. Authors/agents request entry; they cannot self-issue merge authorization.

1. Confirm current scope, approvals, finding disposition, and queue eligibility.
2. Freeze base and PR head; construct the candidate including preceding queued work.
3. Run candidate checks on that exact tree.
4. If anything relevant changes, invalidate evidence and rebuild/retest.
5. Merge using the queue provider's protected semantics; a custom path needs compare-and-swap/base checks and exclusive writer control.
6. Confirm the resulting tree corresponds to the tested candidate and run main smoke checks.

Do not implement a homemade `if checks green then gh pr merge` loop and call it a race-free queue. Serial manual integration is an acceptable bootstrap fallback with recorded limitations.

Hotfix, Critical, Normal, and Low are scheduling lanes in **one coherent queue**, not independent bots racing to update `main`. Start with batch size one. Later trial batches of two-to-four compatible Low/Normal PRs if the chosen tool supports genuine combined testing. Critical changes remain isolated. Split a failed batch and retain evidence; do not repeatedly retry until random failures disappear.

A broken `main` pauses feature merging and channel promotion. Permit only reviewed repair/revert/incident changes. Determine whether the cause is code, infrastructure, or flaky evidence; re-open after relevant verification. A billing outage in CI must not lead to disabling auth tests to restore a green badge.

## 9. Test portfolio

### Unit, integration, contracts, and E2E

Unit tests cover local behavior. Integration tests use actual isolated PostgreSQL/NATS when correctness depends on them. Contract tests verify serialization and supported API/protocol versions. E2E tests exercise supported clients through real service paths.

Keep the permanent Alpha 0 scenario:

```text
create Teri and Ryan accounts/devices
→ authenticated encrypted DM carrying 'bro'
→ persist ciphertext; server/log fixtures contain no plaintext 'bro'
→ disconnect and reconnect; recover the permitted history
→ Stats records one intended message effect despite duplicate event delivery
→ provision wallets with a valid treasury grant
→ transfer 50 credits atomically; verify balanced postings and idempotency
```

“No plaintext in a log fixture” is a tested observation, not proof that no leak exists anywhere. Distinguish native execution on ARM64 from cross-compilation; compiling for an OS does not prove its installer works.

### Property tests

Generate sequences, not just individual values. Use reproducible seeds, bounded sizes, and retained minimal failures.

Economy: zero-sum postings **per transaction and currency**, authorized mint/burn semantics, idempotent transaction IDs, no unintended overdrafts under concurrent transfers, and independent workspace ledgers.

Messaging/outbox: duplicate deliveries do not duplicate durable effects; acknowledged writes survive modeled restart boundaries; replay/retention gaps trigger an explicit resync rather than pretending complete history exists.

Crypto: use official/library test vectors and protocol-focused tests beyond `decrypt(encrypt(x)) == x`. Require corruption/authentication failures, replay handling, add/remove epochs, recovery binding, and crash-safe state transitions. Revocation tests must name the epoch/commit boundary: a server revocation flag alone does not erase already-held keys. Do not restore cloned device keys/current ratchet state from a generic account backup.

### Fuzzing

Use cargo-fuzz/libFuzzer or an appropriate maintained alternative; the Rust Fuzz Book documents cargo-fuzz. See **R10**. Pin its supported toolchain/host; nightly Rust for fuzzing is separate from the product's Nightly channel.

Targets include framing/parser inputs, gateway events, attachment metadata/chunks, bot interactions, plugin host inputs, and protocol state machines. Apply memory/time limits, use synthetic inputs, keep a minimized regression corpus, and triage crashes rather than treating “ran without a crash” as a proof.

### Mutation testing and coverage

Use cargo-mutants for targeted changed-domain campaigns and scheduled broader runs; its purpose is to identify behavior changes the tests do not detect. See **R11**. A surviving mutant may be equivalent or irrelevant, not automatically a defect. An interrupted campaign is incomplete, not a successful score.

Coverage measures execution, not correctness. Track meaningful changes with a tolerance/ratchet after a baseline exists. Review uncovered critical branches and surviving non-equivalent mutants. Do not require arbitrary 100% coverage or increase raw test count by cloning tests. Redundant/flaky tests may be replaced with stronger coverage and documented reasoning.

### Rust runner details

Use cargo-nextest for its CI execution/reporting features. Run Rust doctests separately with the pinned toolchain unless the verified runner version explicitly supports them. Preserve reports about retries/flaky tests. See **R9**.

### Dependency and supply-chain checks

Use cargo-deny for advisories, licenses, sources, and dependency policy. See **R12**. Add frontend dependency checks and a secret scanner appropriate to the implemented stack. Lock toolchains/dependencies, review upgrade PRs, and generate release dependency inventories. A vulnerability exception needs owner, scope, rationale, compensating controls, and expiry. Dependency license policy must respect the still-provisional publication license.

## 10. Subsystem-specific acceptance campaigns

| Area | Required negative cases to design |
|---|---|
| TeriCrypt | Corrupt/truncated inputs, replay, missing epochs, device removal, stale state, interrupted persistence, unauthorized credential substitution |
| Recovery | Wrong phrase locally, account/vault binding, replayed/expired challenge, stale vault, compromised-recovery rotation, no secrets in telemetry |
| Authentication | Session rotation races, logout/revocation, WebAuthn challenge/origin/RP binding, TOTP replay/window policy, atomic one-use recovery codes, rate-limit abuse |
| Permissions | Tenant × role × channel × action matrix; negative authorization, role hierarchy, confused-deputy paths, feature flag bypass |
| Ledger/billing | Concurrent transfers, replayed webhooks, duplicated grants, rounding/currency separation, refund/revoke effects, no virtual-to-real-money coupling |
| Gateway/outbox | Reorder/duplicate/gap, bounded backpressure, slow receivers, worker crash between publish/ack, reconnect under load |
| Migrations | Clean install, supported old snapshots, interrupted migration, concurrent runner, old-app/new-schema compatibility when promised |
| Plugins | Infinite loop, memory growth, malformed components, capability denial, host-call abuse, per-tenant quotas, cancellation |
| Attachments/deletion | Corrupt/chunk-order/truncation errors, size limits, interrupted upload, deletion tombstones applied after restore, notification cleanup limitations |
| SPICE/Music | Provider unavailable, expired URL, queue contention, worker loss, authentication boundary, bounded temporary cache |
| AI | Unauthorized retrieval/tool calls, prompt injection in retrieved text, cross-tenant context, cost caps, visible consent, no silent E2EE upload |
| Cinema/spatial/voice | Playback drift, seek races, listener loss, host loss, jitter/loss, private-side-channel recipient enforcement |
| Release/updater | Invalid signatures, stale/rollback feed, incompatible cache migration, wrong environment config, revoked signing path |

Historical-cipher roleplay plugins are not security substitutes. Genuine participant privacy requires a modern endpoint confidentiality boundary; a classical cipher inside a group does not make its plaintext secure against other authorized group members who can attack that cipher.

## 11. Flaky-test policy

Allow at most one diagnostic retry by default. Keep the initial failure, seed, runner identity, and logs. Some runners otherwise report a final green result after a retry, so our gate must explicitly consume the flake information rather than infer cleanliness from exit code alone.

An unexpected flake blocks until triaged. A temporary approved quarantine needs owner, linked issue, scope, justification, replacement coverage where possible, and expiry. It remains visible in a separate job/report; it is not deleted.

Critical security/integrity tests cannot be silently removed from required coverage. Any temporary exception requires human authorization and recorded risk, not an agent's edit to an ignore list. Expired quarantines fail the policy gate.

## 12. Runner and CI trust boundaries

**Project policy, supported by GitHub's security guidance (R3/R8):**

- Treat code from PRs, dependency builds, npm scripts, and Rust build scripts as executable/untrusted even when an agent or friend authored it.
- PR jobs get read-only permissions and synthetic credentials. No production deploy keys, signing keys, or broad owner tokens.
- Never check out/execute an untrusted PR inside a privileged `pull_request_target` or `workflow_run` context. Treat prior-job artifacts as untrusted inputs too.
- Pin third-party actions to verified full commit SHAs; protect workflow changes and gate-policy files.
- Separate the trusted controller from executors. The controller may publish checks/merge; the worker running arbitrary code may not forge those permissions.
- Self-hosted workers run in dedicated disposable compute, outside Production and outside personal credentials/network services. A Docker socket can expose the host; do not treat a container wrapper alone as isolation.
- Never run attack/fuzz/load jobs against real users, third-party services, or Production by default.
- Use short-lived scoped credentials when the actual provider supports them; do not assume every OCI deployment supports a particular OIDC integration without verification.

PR text, repository comments, test fixtures, tool output, and dependency README files are data to review—not instructions to expose secrets, disable checks, or approve their author. A PR attempting to rewrite `AGENTS.md` must be assessed under the trusted existing rules.

## 13. Promotion gates

| Destination | Additional evidence |
|---|---|
| Nightly | Accepted `main`, baseline/current smoke checks, identifiable artifacts, no known release-blocking critical issue |
| Canary | Full relevant checks, clean synthetic Staging deployment, bounded soak, reviewed known issues |
| Beta | Supported-platform install/update, protocol compatibility, schema upgrades, restore exercise, release notes |
| Stable | Approved immutable artifact identity, valid signing, no unresolved relevant S0/S1, verified recovery/rollback or roll-forward plan, owner release approval |

These are our gates, not promises that tests eliminate bugs. Known S0/S1 blocks the affected merge/release, even if a scheduled campaign found it after PR CI passed. Reassess already-published artifacts when an advisory or critical finding affects them.

Release checks use the actual artifact to be shipped. Rebuilding/repackaging invalidates artifact-dependent evidence. Production rollout may be an ordinary controlled single-node deployment at first; do not claim a fleet canary when none exists.

## 14. Emergency and exception handling

Only an authorized human incident owner can approve an exceptional path. Record incident, exact bypassed requirement, limited scope, reason, expiry, compensating controls, responsible operator, and follow-up. Never waive missing artifact signatures, unknown source provenance, or unresolved crypto boundaries by silently turning off verification.

Keep a reversible repair path. Destructive database recovery is an explicit incident action with understood data-loss implications, not an automatic response to a red test.

## 15. What to implement first

**Bootstrap gate:** formatting, compilation, targeted unit tests, basic integration, secret handling, PR template, truthful evidence, serial candidate checks.

**Before private alpha:** auth/permission/crypto/ledger negative tests, device/session flows, encrypted attachment limits, supported-client E2E, restore drill, protected release process.

**As PR volume grows:** trusted risk dispatcher, independent review reports, actual merge queue, bounded caching/sharding, policy-gate tests, flake registry.

**As each feature arrives:** specialized property/fuzz/mutation/chaos tests and release gates. Do not add an empty “security test” workflow merely to increase the green-check count.

Use the setup backlog added to `11_ROADMAP_AND_TASKS.md`. The acceptance criterion for this organizational system is a deliberately bad change that it blocks, followed by a legitimate repair that it admits—with evidence for the exact revision.
