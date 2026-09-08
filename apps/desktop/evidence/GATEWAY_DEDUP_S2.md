# S2: transport redelivery must reach unfinished-history recovery

- Worktree: `C:/Users/TeRiRi/Documents/GitHub/TeriChat-docs-status`
- Branch: `docs/alpha0-status`
- HEAD/base: `c325b4c90df5087a6f10ff6b5a2bc24787a88ec1`; verification is of the uncommitted combined frontend candidate, not that bare commit.
- Follow-up source changes: `src/App.tsx`, `src/lib/gateway.ts`, `src/lib/__tests__/gateway.test.ts`, new `src/App.gateway.test.tsx` (paths relative to apps/desktop).

## Result and responsibility

GatewayClient keeps transport dedup for existing onEvent consumers, with a 1024-ID FIFO retention bound. An additive optional onDuplicateEvent callback lets App retry effects that did not finish. App routes this callback through its existing history-cursor check and serialized/coalescing history jobs. Application state still owns effect dedup; transport receipt does not acknowledge history completion. Cache eviction can redeliver an older ID to onEvent, consistent with at-least-once delivery.

The mounted-App regression uses the real GatewayClient and injects only the WebSocket boundary; HTTP ApiClient methods return synthetic/deferred fixtures. Both settled-failure and duplicate-during-pending-failure cases recover in exactly one follow-up request, from the unchanged cursor. Further repeats cause no extra history request or duplicate message row. Existing mocked App tests and transport resume/redelivery assertions remain unchanged.

Backend gateway inspected read-only: its live cache compares exact IDs and is bounded. No backend edits or polling were added.

## Executed evidence

Commands run from apps/desktop; logs in this directory:

| Command | Result | Log |
| --- | --- | --- |
| `npm test -- src/App.gateway.test.tsx` before production fix | RED: 2 failed; expected 2 history calls, received 1 | `gateway-dedup-red.log` |
| `npm test -- src/App.gateway.test.tsx` after callback integration | GREEN: 2 passed | `gateway-dedup-green.log` |
| `npm test -- src/lib/__tests__/gateway.test.ts` before cache bound | RED: 1 failed, 6 passed; expected cache size 1024, received 1025 | `gateway-bound-red.log` |
| `npm test` on final frontend candidate | PASS: 69 tests, 6 files | `gateway-dedup-full-tests.log` |
| `npm run typecheck` | PASS, exit 0 | `gateway-dedup-typecheck.log` |
| `npm run build` | PASS, exit 0; Vite transformed 36 modules | `gateway-dedup-build.log` |
| `git diff --check -- apps/desktop` | PASS, exit 0 | terminal tool output |

Vite native-config/ESM and PostCSS package-module-type warnings appeared on RED and GREEN runs; not suppressed or changed by this follow-up. No runtime test failures remain. No dependency/manifests were changed by this follow-up. No commits, branch changes, staging, push, deployment, migrations, backend tests, or native Tauri build were performed. Parent retains integration/review authority. Rollback is the scoped follow-up source edits; no data migration applies.
