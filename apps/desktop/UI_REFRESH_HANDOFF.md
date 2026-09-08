# Task Handoff — bounded chat UI refresh

- Task: existing chat layout/navigation/readability/mobile responsiveness; no landing page or backend features.
- Parent: coordinating agent; independent review/publication remains with parent.
- Branch/worktree: `feat/chat-ui-refresh`, `C:/Users/TeRiRi/Documents/GitHub/UnknownChat-ui-refresh`.
- Base/head: `322c9c5977ccf1e27bb48d3a86477c945dfacebe`; changes deliberately uncommitted.
- Risk: Normal (presentation navigation). State: ready for review, not published.

## Delivered

- Restrained zinc/emerald shell, clearer section hierarchy, larger active conversation rows, readable previews and metadata, explicit current conversation/workspace semantics, visible keyboard focus.
- Three columns on wide screens; workspace details become a separate pane below 1200px; one pane below 640px, with back/return/details controls. Focus follows explicit pane navigation. Navigation can scroll rather than compressing DMs offscreen.
- Panes stay mounted when hidden/resized. Same-conversation drafts survive back/reopen/details; existing conversation and session keys still clear drafts on switching. A details pane cannot remain stranded after leaving its workspace.
- Readable, bounded message bubbles with unbroken-text wrapping; min-width-zero composer and onscreen Send; dynamic viewport/safe-area sizing. Existing single-line/Enter-to-send behavior retained.
- Plaintext warning preserved verbatim and made more visible. No unread badges added: current data has no read/unread state. No fake online/member/security features.
- Channel title/current-row presentation no longer incorrectly carries into a DM after selecting a channel.
- No API/gateway/store/native transport/auth lifecycle modifications, new dependencies, migrations, crypto or policy changes.

Changed implementation: `src/App.tsx`, `src/index.css`, `src/components/{ConversationList,ConversationView,ChannelList,WorkspaceList}.tsx`.
Tests/evidence tooling: `src/App.test.tsx`, `acceptance/run.mjs`, new `acceptance/chat-layout.mjs`, `acceptance/README.md`, this handoff.

## Acceptance and verification

Commands run from `apps/desktop`:

| Command | Result |
|---|---|
| `npm ci` | PASS; locked install, audit reported zero vulnerabilities |
| Baseline `npm test` | PASS, original 69 tests |
| `npm test` after implementation | PASS, 72 tests; all original tests retained |
| `npm run test:acceptance:unit` | PASS, 16 report-gate tests |
| `npm run build` | PASS, TypeScript + Vite production build |
| `git diff --check` | PASS |
| `npm run test:acceptance` | PASS, final browser full campaign below |
| `npm run test:acceptance:native` | PASS, final native full campaign below |
| `npm run validate:acceptance -- <each-final-result.json>` | Both FULL ACCEPTANCE PASS |

Initial pre-review evidence under `evidence/client-acceptance/` (superseded by S2 final runs below; local, ignored; do not stage raw evidence):

- Browser: `2026-09-08T08-30-00-376Z/result.json`.
- Native WebView2/plugin-http: `2026-09-08T08-30-41-609Z/result.json`.
- Both folders contain `chat-desktop.png` (1440×900), `chat-phone.png` (390×844), `chat-phone-navigation.png`, `chat-phone-details.png`, `chat-narrow.png` (320×568), and `layout-measurements.json`.
- Screenshots were loaded and visually inspected, not merely generated. Tests also exercised 768×700 and 390×400 viewports. Browser measurements: document widths equal viewport widths throughout; 320px composer width 224.328125px, Send right 308px / bottom 556px within 320×568. Hidden phone navigation/details have `display:none` and zero rectangles.
- Both full campaigns preserve live two-client messaging, 125-message offline backlog pagination/sequence/dedup, non-null resume, quiet-history recovery, account isolation and renderer reload. Additional synthetic workspace/channel and long unbroken message exercise the responsive UI.
- Source file hashes, main harness hash and layout harness hash match between final browser/native runs; source hashes were checked against the final worktree. Both reports verify container/volumes removed and empty cleanup errors.
- Native artifact exercised: `.acceptance/native-target/debug/terichat-desktop.exe`; SHA256 `25b83bf330d42f3410d4873cf66848c76457128d8f47e57d0025a366c5625d3a`. Two real native window handles recorded.

## RED evidence / encountered issues

- New mounted navigation test first failed because no Back control existed, then passed with presentation navigation.
- New leave-details regression exposed that `WorkspaceStore.setWorkspaces` already clears the selected workspace before the old conditional. The UI fallback now checks the resulting selection; test passes.
- Deliberate pre-responsive-CSS campaign `2026-09-08T08-22-21-268Z` failed `phone shows one pane` after all original messaging scenarios passed. At that point both panes had no phone hide rule. The failed report and existing screenshots remain unchanged; that first RED run did not capture a phone screenshot/computed measurements before failing. Later harness now saves measurements and phone screenshots.
- Intermediate `2026-09-08T08-27-52-818Z` passed but included source HMR while finishing the leave-details fix. It is NOT final source-bound acceptance. Final runs above were rerun serially on frozen source and compared by hashes.
- Existing Vite module/config-loader and PostCSS module-type warnings remain; no suppression or unrelated manifest churn.

## Constraints / next action

- Review found S2 hidden-incoming tail-follow; reproduced and fixed as recorded below. Parent must independently review the fix and final evidence before integration.
- Physical phone OS/soft keyboard, Safari, assistive-technology audit, signed/default-release binary, installer/update and full executable restart were NOT tested. Native acceptance uses an isolated debug build with exact loopback capability override; viewport checks are CDP emulation. Browser acceptance uses the existing test-only HTTP adapter/CORS bridge. Neither proves default release transport configuration or E2EE.
- Broader Rust unit/Clippy/doctest/ARM64 matrix and comprehensive secret scanner were NOT run for this frontend-only change; real backend build and integration campaigns were run.
- No branch/commit/push/deploy. No migration/config rollout needed. Rollback is reverting the scoped frontend change and rebuilding; no persisted data format changed.

## Review S2 — hidden incoming tail-follow (resolved for pane navigation)

- Finding source: full independent review `subagent-summary-0-20260908_113812_593746.txt`, S2 at `src/components/ConversationView.tsx:54–57` interacting with mounted `App.tsx` navigation and CSS `display:none`.
- Reproduction was added **before production changes** to `acceptance/chat-layout.mjs`: real client A sends four messages while client B's mounted phone history has `clientHeight === 0`; wait for actual incoming DOM attachment and renderer frames; return via list or workspace-details Back without any further send. Assertions require scrollable history, zero tail gap, complete final bubble inside the history viewport, identical composer DOM node, unchanged draft, main navigation focus, and successful draft keyboard editing.
- First actual browser RED: `2026-09-08T08-41-11-772Z/result.json`, failed `navigation: incoming tail visible after return without another message (gap 317)`. Screenshot shows the old wrapping message, with all four received messages below the viewport. This confirms the review's mechanism, not merely a source hypothesis.
- Final-harness controlled RED: `2026-09-08T08-49-18-015Z/result.json`, with only the original scrolling effect restored temporarily, failed the same assertion; tail top/bottom `1011.25/1078.5` vs history `239.5/773`, gap `317`. Cleanup succeeded. Fixed effect was restored before final campaigns. RED stops at the list path; both list and details paths are explicitly exercised by each final GREEN campaign.
- Fix: `App.tsx` supplies `isActivePane`; `ConversationView.tsx` records a pending tail-follow when the selected conversation/message count changes, and a layout effect reconciles it only when a real scroll box exists. Pane-return reruns the reconciliation without a new message. Pending state is component-local under the unchanged selected-conversation/session reset keys; no deferred cross-account callback, transport change, remount, polling, or composer focus/draft mutation. Already-consumed pending state prevents unrelated pane toggles from forcing another scroll.
- Regression tooling also drains registered browser routes with `unrouteAll({ behavior: 'wait' })` before browser disposal. Controlled RED `2026-09-08T08-47-06-393Z` hit an existing teardown race (`route.fetch: Request context disposed`) after correctly failing S2, aborting before `result.json` was written. Its partial artifacts remain; the exact ownership-labeled container and volume were removed manually and absence verified, and its backend PID was already absent. No report was fabricated. Subsequent controlled RED and final campaigns verify normal cleanup. Raw failure output contains synthetic-only authorization diagnostics and must not be published wholesale.
- Intermediate GREEN runs `2026-09-08T08-42-57-871Z` / `2026-09-08T08-44-34-038Z` are superseded: their zero-gap check was valid, but an auxiliary descendant selector measured the first bubble rather than the tail. The final harness selects the last actual bubble and checks both top and bottom bounds. No existing assertion was weakened.

### Final S2 verification and source identity

Commands rerun after the final harness correction: `npm test` (**72/72**), `npm run test:acceptance:unit` (**16/16**), `npm run build` (TypeScript/Vite PASS), `npm run test:acceptance`, `npm run test:acceptance:native`, both `npm run validate:acceptance -- <result.json>` (**FULL ACCEPTANCE PASS**), and `git diff --check` (PASS). Existing Vite/PostCSS warnings remain unsuppressed.

Final evidence under `apps/desktop/evidence/client-acceptance/`:

- Browser: `2026-09-08T08-51-00-924Z/result.json`.
- Native WebView2/plugin-http: `2026-09-08T08-51-50-104Z/result.json`.
- Each includes `hidden-incoming.json`, `hidden-incoming-navigation.png`, `hidden-incoming-details.png`, plus the existing responsive screenshots/measurements. Both paths in both modes: gap **0**, last bubble top/bottom **694.25/761.5**, within history **239.5/773**. Draft identity, value, navigation focus and keyboard editing assertions pass.
- Both reports' **24 source hashes** match each other and all current `src` files; both harness hashes match each other and disk. Final native executable hash matches its report. Source/harness remained frozen during these serial campaigns.
- `src/App.tsx` SHA256: `3849afd9697000c9e0e6b463e7d1f0ce64d41a0afb741ce6aafb13856a750e7a`.
- `src/components/ConversationView.tsx` SHA256: `4a134de1faae2c5143836f4a46ff54720ba0a472924014e5649a3c4749365eee`.
- `acceptance/run.mjs` SHA256: `35c4534bd9f347dcd0fc2bd9bb4cffff9d6f7e27beac675cae18c3b72cbd81f6`.
- `acceptance/chat-layout.mjs` SHA256: `9064207a9c3697e76612c7dadd07d96cc9419953c677ab42b4fa78acbe4e5c87`.
- Native `.acceptance/native-target/debug/terichat-desktop.exe` SHA256: `661cd6dc440703d66e178bb7c92e2a75652dd6be672908eb124a7d0e499a5c77`; actual native window handles `3673304` and `15207518`.
- Both final reports: container removed, volumes removed, empty cleanup errors; original full messaging/reconnect/account/reload checks retained.

## Final lifecycle follow-up — CSS-only breakpoint reveal (resolved)

The former resize-only limitation is closed by the bounded follow-up below; this evidence supersedes the pane-navigation-only final runs above.

- Task scope: exclusively `apps/desktop/**`; same branch/base/head as above, no branch/commit/push/deploy. Earlier worker changes preserved.
- Added the actual mounted-app regression first: hide phone history through Back, receive four real synthetic incoming messages, wait for their DOM attachment and renderer effects, then resize to 1440×900 **without navigation or another send**. Require the entire tail bubble visible, unchanged message count, identical history/composer nodes, preserved draft and keyboard editing. Then scroll to old messages and resize to 1300×850: scrollTop must remain zero. Original list/details-return assertions remain intact.
- RED `npm run test:acceptance`: `2026-09-08T08-57-24-123Z/result.json` failed exactly `breakpoint: incoming tail visible after return without another message (gap 633)`. Both navigation paths had passed. Tail top/bottom **1362.25/1433.5** were below history **167/821**. Failed evidence is retained unchanged; cleanup passed.
- Fix in `src/components/ConversationView.tsx`: a `ResizeObserver` on the actual history box retries only a pending follow when the box has positive clientHeight. It disconnects on effect cleanup/unmount and replacement; no polling, remount, focus mutation or unconditional resize scroll. Immediate navigation reconciliation and existing conversation/session reset boundaries remain unchanged. Environments without ResizeObserver keep the existing explicit-navigation behavior (jsdom has no layout observer); browser/WebView2 use the real observer.
- Changed by this follow-up: `src/components/ConversationView.tsx`, `acceptance/chat-layout.mjs`, `acceptance/README.md`, `UI_REFRESH_HANDOFF.md`. No new dependencies or persisted formats.

### Final verified candidate

Commands actually rerun on frozen source/harness: `npm test` **72/72 PASS**; `npm run test:acceptance:unit` **16/16 PASS**; `npm run build` **TypeScript/Vite PASS**; `npm run test:acceptance` **PASS**; `npm run test:acceptance:native` **PASS**; `npm run validate:acceptance -- <each result.json>` **both FULL ACCEPTANCE PASS**; `git diff --check` **PASS**. Existing Vite config-loader/PostCSS module warnings remain unsuppressed.

Evidence under `apps/desktop/evidence/client-acceptance/`:

- Browser GREEN: `2026-09-08T08-58-49-432Z/result.json`.
- Native GREEN: `2026-09-08T09-00-07-445Z/result.json`.
- Both contain `hidden-incoming.json` and `hidden-incoming-breakpoint.png`, alongside navigation/details and responsive evidence. Breakpoint tail gap **0**, tail **729.25/800.5** within history **167/821** in both modes. Navigation/details each retain gap **0**. New no-extra-message, DOM/draft retention and reader-resize assertions all passed. Both breakpoint screenshots were loaded and visually inspected: final `Synthetic hidden breakpoint 3` bubble and unsent draft are visible.
- Programmatic comparison verified all **24 source hashes** against each other and every current `src` file, both harness hashes against each other and disk, and native executable hash against disk. No source/harness edits during or between GREEN campaigns; later edits only document results.
- `src/components/ConversationView.tsx` SHA256: `dfa25703878e4f9800ddd4da7c8b0b6f848e4cad9c866fdb11268ba741364372`.
- `acceptance/chat-layout.mjs` SHA256: `0ffcecef672160f8fd48af8ed599c5d08e630df7d70385f7b91429cd42610b10`.
- `acceptance/run.mjs` SHA256 unchanged: `35c4534bd9f347dcd0fc2bd9bb4cffff9d6f7e27beac675cae18c3b72cbd81f6`.
- Native `.acceptance/native-target/debug/terichat-desktop.exe` SHA256: `312807e92ce7306ad8a4e0ac9956af9168c67327b911cf39b91bcccce9cc53ee`; live window handles **723516** and **2035150**.
- RED and both GREEN reports: container removed, volumes removed, empty cleanup errors. Original full messaging/reconnect/account/reload scenarios retained and passing in both GREEN modes.

No remaining blocker for this bounded lifecycle fix. Parent still owns independent review/publication. Physical mobile keyboards, Safari/accessibility, default release/native shipping configuration and broader Rust matrix remain unverified as above; native acceptance proves its isolated debug/plugin-http configuration and CDP viewport, not physical mobile or a release installer. Rollback remains reverting the scoped UI fix and rebuilding; no migration required.

Parent verification: 72 frontend tests, 16 report tests, TypeScript/Vite build and both full acceptance campaigns passed. Fresh browser evidence: `2026-09-08T09-03-57-024Z`; native: `2026-09-08T09-04-47-758Z`. Raw local campaign artifacts remain ignored, not included in the publication set. Final focused review pending at publication preparation.
