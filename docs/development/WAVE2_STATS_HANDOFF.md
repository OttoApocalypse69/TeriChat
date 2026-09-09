# W2-STATS task handoff

- Task: W2-STATS, caller-private workspace activity reads.
- Parent: `01a0802a-43a1-7271-957a-7d613dd63b8d` backend wave 2 integration.
- Worker: stats_resume.
- Branch: `teriri/backend-workspace-stats`; worktree: `C:/Users/TeRiRi/Documents/GitHub/TeriChat-wave2-stats`.
- Base: `19a95155ac4941f597aab698f0d76f894c99ac77` (parent route-wiring checkpoint).
- Source head: `c63a63be6b17a5e98409239d3958ecde48c3e3b1`.
- Risk: High (caller authorization and private activity metadata).
- State: implemented, compiled; database validation blocked by local Docker startup; independent review pending. Not Done or merge-ready.

## Delivered

Only `apps/server/src/workspace_stats.rs` and this handoff are worker-owned.

- `GET /v1/workspaces/{id}/stats/me`: authenticated caller's sum and latest message timestamp across current workspace channels.
- `GET /v1/workspaces/{id}/stats/me/channels?after=<uuid>&limit=<n>`: current channel IDs, conversation IDs, names, and caller-only message counts/timestamps. Ascending UUID exclusive keyset; default and maximum 100; nonpositive limits rejected; zero-count channels included.
- No message bodies, another user's counters, DMs, foreign workspace counters, or moderation reasons are returned. Deleted channels cease contributing. Existing counters are eventual outbox projections, so counts need not immediately reflect a just-sent message. Rejoining exposes the caller's retained counts on current channels.
- Each read locks the exact authenticated session first, then current workspace membership with `FOR SHARE`, checks bans, and checks expiry using `clock_timestamp()` after lock waits. Existing revocation mutations either finish first (read denies) or wait for the authorized query to finish.

## Acceptance criteria

| Criterion | Met? | Evidence |
|---|---|---|
| Own workspace aggregate/current channel page | Implemented; runtime unverified | `current_channels_scope_counters_and_rejoin`, `routes_auth_scope_and_private_shape` |
| Other users, DMs, other workspaces excluded; duplicate event has one effect | Implemented; runtime unverified | Real message/outbox fixtures and duplicated `stats::process_event` calls |
| Zero-inclusive bounded UUID pages and invalid input | Implemented; runtime unverified | `pagination_is_bounded_and_exclusive`, HTTP invalid query cases |
| Membership removal/ban cannot bypass waits | Implemented; runtime unverified | `pending_revocation_is_rechecked_after_lock_wait`, four real-lock cases |
| Session revocation/expiry cannot bypass waits | Implemented; runtime unverified | `session_revocation_and_expiry_after_wait_deny_both_reads`, four real-lock cases |

## Verification

Native Windows x86_64, Rust 1.97.1; committed Cargo.lock unchanged. Evidence paths are local, ignored `target/evidence/` files in this worktree. Commands ran on the source now committed as the source head above.

| Actual command | Result | Evidence |
|---|---|---|
| `cargo fmt --all -- --check` | PASS | `stats-fmt.log` |
| `cargo check --workspace --all-targets --locked` | PASS | `stats-check.log` |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS | `stats-clippy.log` |
| `cargo test --workspace --no-run --locked` | PASS; binaries compiled, no tests executed | `stats-test-compile.log` |
| `cargo test --workspace --doc --locked` | PASS; zero doctests exist | `stats-doc.log` |
| `git diff --check` | PASS | Tool transcript |
| `cargo audit --json` | FAIL: existing `rsa 0.9.10`, `RUSTSEC-2023-0071` in unchanged lockfile | `stats-audit.json` |
| `cargo deny --version` | Tool unavailable; dependency policy check NOT RUN | Tool transcript |
| Focused and full DB tests | BLOCKED, not executed: parent recovering synthetic Docker PostgreSQL | No runtime success claimed |

A development Clippy run rejected identical test assertion branches after error-type conversion; this was fixed by asserting the actual banned-versus-nonmember reason and the next Clippy run passed. Initial failure is preserved in the tool transcript. No test was weakened or skipped to obtain a pass.

## Remaining findings and constraints

- Parent/reviewers own independent adversarial assessment and final integration against current main. Reviews must use the immutable source head.
- Existing RSA advisory remains unresolved, not waived by these changes.
- Dedicated secret scanner, ARM64 execution, frontend checks (unchanged), and GitHub CI not run by this worker.
- Required next action: parent confirms exclusive synthetic `wave2_stats` DB ready, then execute focused tests and full workspace suite without a competing backend/outbox process. Retain every failure.

## Compatibility and operations

Additive HTTP endpoints, no migrations or dependency/configuration changes. Revert the source commit to remove these endpoints; no persisted data format was changed. Parent owns route wiring. No deployment, push, PR creation, merge, account-access policy change, cryptographic claim, or public leaderboard is included.

## Handoff

Source claim can be released after parent integration/review. Next unblocked work is independent static/adversarial review while the parent repairs the local synthetic database runtime. Final acceptance remains pending actual DB execution and combined-source validation.
