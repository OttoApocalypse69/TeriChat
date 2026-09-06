# Engineering Sources and Verification Notes

Checked on **2026-09-06**. These are primary-source references for external tool behavior in Rounds 1 and 2. Product choices, concurrency defaults, severity levels, and release gates are our proposed/accepted policies, not vendor guarantees. Verify actual repository plan and pinned tool versions during implementation.

## R1 — Semantic Versioning

https://semver.org/

Version precedence, prerelease/build metadata, and not modifying the contents of a published version. Used to separate software version, artifact identity, and release-channel promotion.

## R2 — Tauri updater

https://v2.tauri.app/plugin/updater/

Updater version, URL, signature metadata, and update behavior. Do not confuse updater signing with OS code signing/notarization or assume channel switching makes schema downgrades safe.

## R3 — GitHub Actions secure use

https://docs.github.com/en/actions/reference/security/secure-use

Covers least privilege, untrusted code, action SHA pinning, self-hosted runner risks, JIT cleanup limits, secret handling, and protecting workflow changes. Our stricter project policy keeps arbitrary PR execution away from Production and release credentials.

## R4 — GitHub deployment environments

https://docs.github.com/en/actions/reference/workflows-and-actions/deployments-and-environments

Feature access depends on plan and repository visibility. The checked documentation limits private-repository environment features and required-reviewer availability; listing several reviewers is not an all-reviewers quorum. Record actual enforcement rather than promising settings that are unavailable.

## R5 — GitHub merge queue

https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges/managing-a-merge-queue

Documents public-organization/private-Enterprise availability, candidate checks against the latest base plus earlier queued PRs, required `merge_group` workflow support, and the distinction between merge limits and build grouping. Our third-party queue selection remains open.

## R6 — GitHub rulesets

https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/about-rulesets

Branch/tag rulesets, bypass actors, and visibility/plan availability. An inactive rule or a written Markdown rule is not an enforced restriction.

## R7 — GitHub CODEOWNERS

https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/about-code-owners

Owner permissions, base-branch ownership, ordering, and required-review behavior. One approval from a listed owner can satisfy the built-in owner requirement; custom multi-role policy needs its own enforcement.

## R8 — Workflow events and conditional execution

https://docs.github.com/actions/using-workflows/events-that-trigger-workflows

https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/control-jobs-with-conditions

Use these to implement event coverage, understand skip behavior, and avoid confusing successful/skipped job states with completed required testing. Validate app-generated PR and merge-candidate events in the actual setup.

## R9 — cargo-nextest

https://nexte.st/

https://nexte.st/docs/running/

Test execution, reports, filtering, profiles, and retry behavior. Pin a supported version; keep a separate doctest command unless the selected version explicitly covers it. The policy gate must not hide an initial failure behind a successful retry.

## R10 — Rust Fuzz Book

https://rust-fuzz.github.io/book/cargo-fuzz.html

cargo-fuzz/libFuzzer tooling. Toolchain/host requirements and target configuration must be checked before scheduling; a tool running on nightly Rust is separate from an application's Nightly release channel.

## R11 — cargo-mutants

https://mutants.rs/

Mutation testing complements ordinary tests/coverage. Surviving mutations require interpretation, including equivalent mutations and incomplete runs.

## R12 — cargo-deny

https://embarkstudios.github.io/cargo-deny/

Dependency policy checks cover advisories, licenses, banned/duplicate packages, and sources. A scanner is not legal advice or proof every vulnerability is known.

## R13 — OpenAI agent instructions

https://developers.openai.com/codex/guides/agents-md

Official guidance on root/nested `AGENTS.md` discovery and instruction-size limits. This pack provides a compact root file plus linked detailed documents rather than duplicating the whole specification in automatically loaded instructions. Other agents may load such files differently; verify the chosen tool's behavior.

## Source scope

This update builds on the latest conversation-provided spec ZIP and decision register, plus the two accepted engineering rounds. No unrelated project's agent policies were imported. The new root `AGENTS.md` is assembled from this project's agreed rules and existing bootstrap brief.

The source check does not verify Oracle quotas, SPICE's deployed availability, project licenses, actual GitHub settings, or cryptographic implementation correctness. Those are separate recorded engineering/owner tasks.
