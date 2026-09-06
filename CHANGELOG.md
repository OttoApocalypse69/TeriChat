# Pack Change Log

## v3 — Engineering organization and quality system — 2026-09-06

### Added

- Round 1: repository, multi-agent coordination, branches, version/artifact identity,
  release channels, isolated environments, feature flags, promotion, and plan constraints.
- Round 2: risk/severity policy, independent review, test/CI lanes, trusted merge gate,
  runner isolation, flake handling, specialized campaigns, and release/incident gates.
- Root AGENTS.md, START_HERE.md, and a short human-oriented TERI_TODO.md.
- GitHub issue and PR templates; finding, handoff, and release-promotion templates.
- Primary-source references, SETUP backlog, packaging manifest, and validation report.

### Updated

- README, full bootstrap prompt, roadmap, decision register, operational pointers,
  and glossary now reference the new rules without discarding earlier product specs.
- Latest provided security/recovery/MFA decisions retained. Next product P0 remains
  recovery mnemonic encoding; new DevOps choices are tracked separately.
- Clarified deletion: retained ciphertext plus keys can also defeat retroactive erasure.
- Clarified artifact promotion versus version-changing repackaging, real GitHub plan
  limitations, CI/control-plane risk, and policies versus verified enforcement.

### Not performed

No repository, cloud, billing, deployment, or account configuration changes.
No application compilation, application test suite, security audit, or live CI run.
Packaging validation checks only the delivered files and archive.

### Source pack

Extended the latest mounted teriplatform_spec_pack_updated.zip. Its 15 original
Markdown documents are retained, with the intentional updates above. The new AGENTS.md
is compiled from this project's accepted rules and existing bootstrap brief; no
unrelated project's agent policy was substituted.

## v4 — Governance bootstrap additions

- Added root `CONTRIBUTING.md` for human/agent contribution workflow and early bootstrap mode.
- Added root `SECURITY.md` with private vulnerability-reporting rules and current security boundaries.
- Added `docs/adr/README.md` and `docs/adr/0000-template.md` for governed architectural decisions.
- Added `docs/development/BOOTSTRAP_SESSION.md` so Teri and the first main agent can establish the real toolchain/environment instead of pre-filling guessed configuration.
- Updated `AGENTS.md`, `00_README.md`, and `TERI_TODO.md` to reference the new governance/bootstrap files.
- Deliberately did not pre-create toolchain pins, env-variable files, CI workflows, or CODEOWNERS identities before the real repository/environment exists.
