# Current implementation and evidence snapshot

Reconciled on 2026-09-13 against main
`f503c361ddaf3f8b7e9c4bfb35c978c7981a1f9f` (PRs #45–50 integrated).
This is a source and executed-evidence inventory, not deployment or release
certification. Application: **UnknownChat**; repository/internal identifiers
remain TeriChat. The client still uses **demo-plaintext envelopes**.
Secure Alpha 0 is not complete.

## Current implementation

| Area | Shipped source/evidence | Remaining limits |
|---|---|---|
| Auth and gateway | User/device/session baseline, opaque server envelopes, outbox/replay, owned-session inventory/revocation and persistent gateway session checks; #30. Production-sized pool replay starvation repaired in #47. | Full passkey/MFA/recovery lifecycle remains broader. The #47 fix addresses the confirmed replay/validity-check deadlock, not every possible pool or overload failure. |
| Conversations | #50 makes conversation/participant insertion atomic and serializes DM reopening for an unordered participant pair. Seven real-PostgreSQL regressions cover rollback, cancellation, concurrency and compatibility. | No schema uniqueness constraint, historical-duplicate consolidation or mixed-old/new-writer uniqueness guarantee. |
| Readiness | #49 bounds the complete database probe to two seconds and returns a uniform safe unavailable response. Closed pool, silent TCP handshake and exhausted-pool/recovery tests pass. | HTTP timeout is not proof of immediate SQLx background connection cleanup or recovery from every stalled established query. |
| Workspaces and Stats | Workspace/channel/member UI and API; permissions, moderation, ban directory; metadata-only, caller-private Stats. #48 adds session controls and private workspace activity with real API/browser acceptance. | API pagination is proven directly; this is not UI load-more acceptance. Account replacement covers delayed activity, not a delayed session-inventory response. No public leaderboard or complete workspace feature claim. |
| Economy #8 | #32 implements balanced atomic virtual-credit transfers, idempotency and caller-isolated history. #46 adds complete keyset history paging and transaction-bound cursors. | Virtual CREDITS only; no real payments or complete Economy roadmap claim. |
| Recovery mnemonic #11 | #33 implements terirecovery-v1: BIP-39 English, 256-bit entropy as 24 words, checksum/normalization, secret-free errors and nine tests; #42 reconciles the decision register. | Encoding is not a working recovery vault/device recovery lifecycle. Older unchecked mnemonic entries remain stale. |
| TeriCrypt / MLS #6 | Current crate contains sealed-envelope proof-of-concept code and mnemonic support. | Foundation #34 was reverted by #35; restoration #36 closed unmerged. MLS is absent. Persistence, credential representation, audit disposition and specialist review remain in the decision queue. |
| Desktop/web #5 | Executable-restart harness/evidence in #40; browser/native transport selection fixed in #41. #48 adds account controls and the real browser/API acceptance workflow. | Prior native evidence belongs to its recorded executable. Current browser evidence does not certify a native installer, public deployment, HTTP cache policy or E2EE. |
| Notifications #43 | #44 implements best-effort native/browser notification delivery for new unfocused DMs, suppression and click focus. | Still lacks headed native OS-toast proof, per-conversation mute, tray and quiet hours. OS history can retain plaintext. |
| Server organization / Staging | Per-domain routes/tests in #37 and staging runbooks/network fixes in #38–39. | No fresh operational verification, production change or restore campaign in this inventory. |

## Verified evidence and its bounds

[Main CI 34753297870](https://github.com/OttoApocalypse69/TeriChat/actions/runs/34753297870)
passed at exact main `f503c361ddaf3f8b7e9c4bfb35c978c7981a1f9f`.
Both PostgreSQL jobs report 97 server unit, 34 server integration, 13 crypto unit
and nine mnemonic tests, with zero failures or ignored tests. The 34 integration
tests include seven new conversation-creation tests and eight health tests.
Format, Clippy and build passed. Doctest commands report zero tests. ARM64 checks
and compiles test targets; it does not execute those tests. Earlier main runs for
#49 and #50 were cancelled when newer main commits superseded them; the combined
main run supplies the integration evidence.

[Real API acceptance 34753297894](https://github.com/OttoApocalypse69/TeriChat/actions/runs/34753297894)
also passed on that exact main. Its sanitized result records tree
`f4c30fa39469e1ece238765519915d69c8602f04`, matching Git, Chromium
153.0.8010.12, all seven scenarios PASS and cleanup PASS. It exercises synthetic
accounts, real messages/outbox/private projection, cross-account denial, direct
API paging and safe session fields, stale workspace responses, another-session
revocation, self-revocation while closing controls, and account replacement.
The workflow also runs locked frontend/Rust preflight and real transport
regressions. Browser HTTP cache is disabled for controlled response overlap;
cache policy and native execution are explicitly not tested.

The committed #40 restart evidence at
`apps/desktop/evidence/client-acceptance/2026-09-11T15-24-40-350Z/result.json`
records source `2e8e47b5edfdf815e128e0fea0db875160445984` and executable SHA256
`b2a9d2c3fa852ca202faa2fcb92a59e5a965437e21d1804da1dbb96087b95cef`.
Its two-client exchange, restart/backlog and resumed-send results remain historical
native evidence; they do not establish current-main native or MLS acceptance.

The owner account merged #48–50. This inventory does not infer standing Critical
merge authority from those actions. Their committed handoffs retain pre-hosted-CI
snapshots; the exact-main evidence above supersedes their pending-CI statements.
This documentation pass inspected source, merge records, CI logs and the downloaded
acceptance result. It did not rerun application tests or native/operational checks.

## Remaining findings and next work

The later #30 replay/pool-exhaustion finding is addressed by #47 and its preserved
baseline failure/candidate regressions; current-main tests also pass. Wallet
paging and client activity/session controls are now integrated. Do not assign
those completed slices again from older checkboxes.

The next bounded backend slice is owned, bounded server/worker shutdown: current
bootstrap listens only for Ctrl-C and discards outbox/Stats worker handles.
SIGTERM handling, drain order, cancellation and replay need implementation and
real evidence. A separate writer owns SQLx feature hygiene; it must preserve
PostgreSQL, migration, macro and JSON behavior without unrelated upgrades.

Existing dependency evidence remains limited: the 2026-09-13 audit at
`1ef7821b65a8462e53ba90a89538d38e27b7fe3f` (unchanged root manifest/lock
relative to this main) reports
`rsa 0.9.10 / RUSTSEC-2023-0071` through optional SQLx/MySQL lock nodes, while the
inspected supported root-server build graph contains neither RSA nor MySQL.
That is scoped build-graph evidence, not removal of the advisory or a native
security assessment. cargo-deny and gitleaks were NOT RUN (tools/configuration
absent). No audit ignore, waiver or scanner pass is implied. A separate open
Dependabot alert #6 concerns glib in the native Cargo.lock; applicability and
remediation are not established by the root-server graph inspection.

MLS remains decision-blocked and its preserved branch is not approved for
restoration. Notification #43 still needs headed OS evidence. No original
acceptance requirements, dependency warnings or review gates are waived.

## Historical audit

The following snapshot is preserved verbatim as historical evidence. Its
unimplemented/unmerged statements describe its old baseline, not current main.

<details>
<summary>Earlier Alpha 0 snapshot (baseline c325b4c)</summary>

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

</details>
