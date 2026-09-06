# Teri's Setup Checklist

**Start here. You do not need to read both engineering manuals first.**

Nothing below is checked off automatically: writing a policy does not turn on a GitHub setting.

## Do now — five things

1. [ ] **Create or choose one private GitHub repository.** Any temporary name is fine. Decide its organization/account; do not publish it just to unlock a GitHub feature.
2. [ ] **Put this pack in its root.** Keep `AGENTS.md`, `CONTRIBUTING.md`, `SECURITY.md`, the numbered documents, `docs/`, and `.github/` in the correct places. If the repo already has files with those names, ask the agent to merge them, not overwrite them blindly.
3. [ ] **Add Ryan with scoped access.** Collaborator first; do not give agents or shared accounts organization-owner credentials. Keep your GitHub account protected with passkeys/MFA.
4. [ ] **Set a spending ceiling.** Default to no new paid services until you approve them. Check the repo plan's actual protections and runner allowance before installing a merge service.
5. [ ] **Give one coordinator the prompt below.** Have it use `docs/development/BOOTSTRAP_SESSION.md` to discuss and establish the real environment with you. Start with a small pilot; expand the swarm after it can safely integrate two PRs.

```text
Read AGENTS.md, 00_README.md, the latest decisions, and Rounds 1 and 2.
Inspect this repository and use docs/development/BOOTSTRAP_SESSION.md with me
to resolve the real local toolchain, workspace layout, dependencies, commands,
and GitHub capabilities. Then implement the unblocked SETUP-001 through
SETUP-005 tasks from 11_ROADMAP_AND_TASKS.md on a scoped branch.
Set up honest baseline CI and a small testable application foundation.
Do not publish, deploy, buy anything, install apps, alter repository access,
or give agents production credentials. Report settings I must enable myself.
Prove one intentional bad change fails the gate and the repaired change passes.
Stop before merging unless I explicitly authorize integration.
```

## Intentionally decide with the main agent, not in this pack

- [ ] Exact Rust/Node/tool versions.
- [ ] Real local ports and environment-variable names.
- [ ] `justfile`/script commands after the repository has commands worth wrapping.
- [ ] CI workflow YAML after the first compilable/testable baseline exists.
- [ ] `.github/CODEOWNERS` after actual GitHub usernames/teams and critical paths are confirmed.
- [ ] Exact staging/production secrets and cloud configuration.

This is deliberate: these files should describe reality, not guesses made before the repository exists.

## Before unleashing both swarms

- [ ] Get the coordinator's **actual GitHub capability report**: what works on this private repo, what is unavailable, and what remains manual.
- [ ] Approve real code-owner usernames/teams. These are not guessed from display names.
- [ ] Enable available branch protections after the named CI checks have run once. Confirm a failing PR is blocked. Keep integration owner-controlled if the plan cannot enforce it.
- [ ] Choose one merge path: native queue if available, approved third-party queue, or one-at-a-time maintainer integration for the pilot. Do not enable two competing merge bots.
- [ ] Run two small non-overlapping pilot tasks—one from each of you—with separate branches/worktrees and independent review.
- [ ] Increase active writers only when the queue is keeping up. More agents can review/test without all opening new feature PRs.

## Before friends use real accounts

- [ ] See the **It Sends Bro** demo and the actual test report, not just screenshots of a UI.
- [ ] Review remaining P0 security decisions, beginning with recovery-word encoding. Do not treat the chosen crypto library as a complete security review.
- [ ] Confirm production and arbitrary CI jobs are on separate compute/security boundaries.
- [ ] Confirm an encrypted off-machine backup was restored successfully with synthetic data.
- [ ] Approve the Staging → Production release plan, update signing, and rollback/roll-forward procedure.

## After setup, your recurring job is small

Review owner-blocked decisions, Critical PR evidence, and release approvals. Ask for a dashboard showing only: **blocked decisions, serious findings, queue health, costs, and candidate releases**.

Do not personally click through every harmless UI PR. Also do not approve crypto merely because five agents said it looked fine: ask for the threat model, regression evidence, and appropriate specialist review before real security claims.

## Ignore for now

Final logo, exact boost prices, Kubernetes, marketplace economics, enterprise certifications, 4K HDR, and the .NET Colossus arms race. They are allowed to remain glorious future problems.

**Tiny vocabulary reminder:** `main` = accepted code; Nightly/Beta/Stable = who gets a build; Staging/Production = where it runs; feature flags = which features are enabled. TeriCrypt is still TeriCrypt-4096™.

---

## Implementation progress (agent-maintained — Teri reads this, not the five boxes above)

Local repo only: no GitHub repo, no Ryan access, no spending decisions yet (boxes 1, 3, 4 untouched).
Pack IS in the local repo root with git history (`main` + `setup/baseline`).

- [x] SETUP-001/002 — local repo initialized, spec pack committed (`77a1dd3`)
- [x] SETUP-003 — Rust workspace + health/ready baseline, Compose PG, `.env.example` (`e18bce1`)
- [x] Deps on machine — Rust 1.97.1, sqlx-cli 0.8.6 (= runtime), Docker + PG16 running
- [x] First migration — `users`/`devices`/`sessions`, applied on boot (`f555be6`)
- [x] Parallel wave 1 — `config.rs`, `password.rs` (Argon2id), CI `db-tests` job (merged `a345137`)
- [x] Config boot wiring — `Config::from_env` drives port/log-filter/pool, bad `PORT` exits 1
- [x] Milestone B auth — `auth.rs` + `/v1/auth/*`: register (201), login (token once),
  logout (idempotent), device registry, bearer extractor, revocation; 23/23 tests
  offline + DB-backed, live curl lifecycle verified (register→login→device→logout→401)
- [ ] NEXT — Milestone C messaging: conversations, `MessageEnvelope`, outbox, WS gateway
