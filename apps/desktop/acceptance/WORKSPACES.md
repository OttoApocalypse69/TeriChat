# Workspace browser acceptance

Run from `apps/desktop` with installed lockfile dependencies, Microsoft Edge,
and an already-running **disposable local backend with its own synthetic DB**:

```sh
node acceptance/workspaces.mjs http://127.0.0.1:<backend-port>
```

The harness rejects non-loopback URLs, creates three unique synthetic accounts,
starts its own Vite instance, and drives two isolated browser contexts. It
checks UI workspace creation, member addition, ordinary-member roster access,
API cursor pagination and field minimization, outsider denial, channel/message
persistence after renderer reload, and role/kick updates with revocation.

Every response comes from the real server. As with the existing browser
acceptance campaign, the browser uses a test-only `plugin-http` fetch adapter
and CORS bridge. This verifies the shared React flow, not native IPC,
installation/update, default browser deployment routing, or MLS encryption.
No production or shared database should be supplied. The harness closes its
browser and Vite; the caller owns disposal of the backend/database fixtures.

Results and a synthetic screenshot are written below `.acceptance/workspace-*`
(gitignored). A failing scenario sets a nonzero process exit code and `FAIL`;
cleanup failure also fails the run. This scoped report is separate from the
existing full messaging acceptance gate. Tokens and passwords are never
included in the report. Inspect source differences alongside the reported head
when executing an uncommitted candidate.
