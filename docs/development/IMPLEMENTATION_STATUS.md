# Alpha 0 implementation and evidence snapshot

## Local repair candidate

The findings below describe the merged baseline. Subsequent fixes are integrated locally on `docs/alpha0-status`; see [the repair handoff](ALPHA0_FIX_HANDOFF.md) for current review dispositions and parent verification: 69 frontend tests, 73 server tests and 13 crypto tests passed, plus real local Caddy routing and synthetic encrypted backup/restore. They are not merged or deployed. Native/live operational checks and GitHub candidate CI remain pending.

## Scope and revision

Task: reconcile documentation with the merged Alpha 0 baseline, not declare the roadmap complete.

- Source baseline: `c325b4c90df5087a6f10ff6b5a2bc24787a88ec1` (merged PR #22).
- Locally tested source: `7d6112cecd26531e0686d3c9caee3e6e443961f5`; `git diff --exit-code` against the baseline returned zero (identical tracked trees).
- Documentation branch: `docs/alpha0-status`, based on the baseline above.
- Application branding: **UnknownChat**. Repository, crate, executable, bundle identifier, and TeriCrypt branding remain unchanged.
- Status: three read-only audits completed. Findings below distinguish inspected failure paths from runtime reproductions; fixes and regression verification remain outstanding. This snapshot does not grant merge, release, or production approval.

## Implemented versus accepted

| Area / issue | Implemented evidence | Outstanding acceptance / limits |
|---|---|---|
| Auth and messaging | `apps/server/src/auth.rs`, `messaging.rs`, `gateway.rs`, migrations | Full roadmap security goals are broader than the implemented baseline. Green server tests do not establish complete passkey/MFA/recovery behavior. |
| Desktop #5; polish #22 | `apps/desktop/src`, `src-tauri`; server `GET /v1/conversations`; PR #22 merged | Fresh two-native-client live/reconnect acceptance and native rebuild are not part of this verification pass. Client envelopes are demo plaintext, not E2EE. |
| TeriCrypt #6 | `crates/tericrypt/src/lib.rs` sealed-DM prototype and tests | MLS/OpenMLS integration, independent device/group state, membership epochs, persistence/reload, and client integration remain required. Prototype tests are not MLS evidence. |
| Recovery #11 | Locked envelope-vault direction in `12_OPEN_DECISIONS.md` | Mnemonic encoding/checksum/normalization/versioning and implementation remain open. No recovery-format decision is made by this snapshot. |
| Workspaces | `apps/server/src/workspaces.rs`; desktop workspace/channel/member panels | Implemented baseline does not imply every Milestone E feature or authoritative member-list UI exists. |
| Stats #7 | `apps/server/src/stats.rs` and stats migration/tests | Late-commit cursor loss and missing workspace-specific isolation coverage block acceptance; see STATS-01 below. |
| Staging #9 | `deploy/staging/`; public application address `https://chat.unknownchat.xyz` | Fresh-host reproducibility, off-machine encrypted backup, restore drill, and administration/exposure evidence must be verified separately. No operational completion claim. |
| Server modularization #10 | `state.rs`, `bootstrap.rs`, `routes.rs`, `errors.rs`, `health.rs` | Reviewer found criteria met with refactor and exact-main CI evidence; candidate for maintainer closure, not closed by this audit. |
| Economy #8 | Open implementation issue | Double-entry ledger baseline and invariant/concurrency evidence remain outstanding. |

## Executed verification

Local commands were run in the clean polish checkout whose tracked tree matches the merged baseline. Test and build artifacts do not change that source comparison.

| Command / evidence | Result |
|---|---|
| `git diff --exit-code origin/main 7d6112cecd26531e0686d3c9caee3e6e443961f5` with origin/main at the baseline above | PASS; identical source trees |
| `cargo fmt --all -- --check` | PASS |
| `cargo check --workspace --locked` | PASS |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| `cargo test -p tericrypt --locked` | PASS; 13 unit tests |
| `cargo test --workspace --doc --locked` | PASS command; zero doctests exist, not additional coverage |
| `npm run build` in `apps/desktop` | PASS; TypeScript check and Vite build |
| `npm test` in `apps/desktop` | PASS; 46 tests across four files |
| Merged-main CI [run 34168372582](https://github.com/OttoApocalypse69/TeriChat/actions/runs/34168372582), `db-tests` log | PASS reported; 66 server + 13 crypto tests, zero failed/ignored |

Vite/Node emitted module-format warnings for configuration files; the web build and tests still exited successfully. These warnings were not suppressed or fixed during this documentation-only task.

Not run in this pass: local full database suite, native Tauri rebuild/two-client UI smoke, dependency audit, secret scan, fresh-host deployment, backup/restore drill, production security assessment. CI evidence is not a replacement for these distinct checks. Current CI does not establish frontend/native build coverage merely by being green.

## Audit findings and disposition

All paths/lines below refer to the source baseline above, not this documentation diff. These are retained findings, not implemented repairs.

| ID / severity | Preconditions and failure path | Evidence and disposition |
|---|---|---|
| CLIENT-01 / S1 | Account A has a pending list/history response, logs out, then B logs in. A's callback writes into the replacement current store, exposing A's peer metadata or messages in B's session. | Parent inspected `App.tsx:75–106,169–185,220–225`; no session-generation guard. Static confirmation, no mounted-client reproduction yet. Blocks affected client acceptance; add delayed-response/account-switch regressions across sibling async paths. |
| CLIENT-02 / S2 | More than 100 history rows exist; only one page is fetched. A later send/live message raises the largest sequence beyond the missing range, so future requests skip it. | Parent inspected `App.tsx:75–86,176–181`. Reviewer executed store-level evidence, not full UI E2E. Track contiguous recovery separately and test paginated history with concurrent arrivals. Blocks #5. |
| CLIENT-03 / S2 | Event is marked seen/resumable before its history request succeeds. Failed history then reconnects with no newer event, leaving the message absent. | Reviewer static/store-level evidence at `store.ts:278–285`, `App.tsx:169–185`; parent confirmed failed fetch is swallowed. Full reconnect regression pending. Blocks #5. |
| GATEWAY-01 / S2 | More than 100 missed events exist. Server replays one page then enters live mode; already-published remainder is not replayed. | Parent inspected `gateway.rs:164–174`. Runtime regression pending; verify replay/live handoff and a quiet conversation beyond page one. Blocks #5. |
| CLIENT-04 / S2 | Background state update changes the enclosing main element's version-based key, remounting the conversation view and discarding an unsent draft/focus. | Parent confirmed `App.tsx:380`; reviewer inspected composer-local state. Mounted regression pending. |
| STATS-01 / S2 | Transaction A allocates a lower outbox UUID but commits after B. Worker processes B and advances beyond A before A becomes visible; A is then missed during normal operation. | Parent confirmed high-water query/update at `stats.rs:291–319`; reviewer traced pre-commit UUID allocation in `messaging.rs:461–475`. Deterministic two-transaction regression pending. Blocks #7 exactly-once acceptance. |
| STAGING-01 / acceptance gap | Request `/ready` reaches static SPA fallback rather than backend readiness. | Parent inspected `deploy/staging/Caddyfile:11–22`: only `/health` and `/v1/*` proxy. No live request in this audit. Blocks #9 readiness criterion. |

Additional reviewer-reported gaps: Stats lacks workspace-specific isolation tests; staging instructions do not reproduce the external shared SPICE edge; backup script does not establish encrypted-at-rest backup and performed restore evidence; new inbound DM metadata can remain generic. These need targeted follow-up, not blanket completion claims.

The roadmap reviewer found #10's startup/state/router separation meets its issue criteria with existing regression and exact-main CI evidence. It is a candidate for maintainer closure, not a new implementation track; no issue was closed. #5, #7, and #9 remain incomplete. #6 and #11 remain unimplemented in their requested MLS/recovery scope. No Accepted ADR exists at this baseline; the locked decision register remains authoritative. Automated crypto review is not specialist approval.

## Next work and ownership

1. Preserve the completed audits' concrete findings and acceptance gaps rather than closing issues from merge status.
2. Fix any confirmed blocking baseline defects with regression tests before adding affected features.
3. Prioritize #6 and #11 under the accepted MLS/recovery boundaries; unresolved material decisions need the prescribed owner/domain review. Separate writers by explicit paths and coordinate manifests, lockfiles, and protocol contracts.
4. Complete #9 operational evidence without silently provisioning backup services or changing production topology.
5. Continue #8 after higher-priority gaps/dependencies; no Cinema, AI, landing-page expansion, or unrelated feature work in this slice.

## Compatibility and rollback

This reconciliation changes documentation only. No schema, API, key format, bundle identity, infrastructure, or security policy is changed. Reverting the documentation commit restores prior prose but does not change implementation state. Original roadmap requirements remain in force; unchecked items have not been waived. Historical checklist entries are retained and explicitly labeled historical.
