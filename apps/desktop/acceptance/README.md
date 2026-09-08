# Windows client acceptance

Run from `apps/desktop` using the existing Windows toolchain, installed Edge,
Docker and Rust. All backend accounts/data are synthetic and disposable.
The harness refuses to reuse its reserved container, does not read dotenv or
credential files, and only removes its owned processes/profiles/container/volumes.

```sh
npm run test:acceptance:unit
npm run test:acceptance
npm run test:acceptance:native
npm run validate:acceptance -- evidence/client-acceptance/<run>/result.json
```

`acceptance/report.mjs` is the full-acceptance machine gate. **Do not consume
`mode` or `result` alone.** The gate requires explicit arguments, `scope` equal
to `full-campaign`, `fullCampaignComplete: true`, all scenario markers, B's
successful reconnect pagination, and successful cleanup. Native reports also
require executable identity and both live window handles. Old reports without
these fields intentionally fail closed; they remain historical evidence.

`--probe-only` is backend readiness only, even with `--native`: no native build
or window is exercised. `--ui-probe` exercises login DOM/screenshots only.
Both successful probes report `PROBE_PASS`, their exact arguments/scope, and
`fullCampaignComplete: false`. They cannot satisfy full acceptance.

## Pagination evidence

Login is sequential and awaited until connected. Each session token is bound
in memory to the active client instance (`a` or `b`) on its first authenticated
request/upgrade; reconnect uses the same binding. Tokens are never persisted.
History observations snapshot the phase at request start, then record the actual
response status and array length without persisting response payloads.
The B backlog reconnect must successfully fetch `since=1, limit=100, size=100`
then `since=101, limit=100, size=25`. A's earlier requests and B's other phases
cannot satisfy this check. Existing exact message-body and sequence checks remain.

To verify real campaign reports and reject in-memory mutations without editing
any evidence:

```sh
node acceptance/verify-evidence.mjs <browser-result.json> <native-result.json> <backend-probe-result.json> <ui-probe-result.json>
```

The regression RED logs preserve the original history assertion accepting
wrong-client/phase/page evidence and the legacy result-only report consumer
accepting probes. GREEN tests use the exported full validator. The separate
real-evidence verification exercises new reports, including wrong-client/phase
mutations and probes relabeled with legacy `PASS`, in memory only.

## Scope and publication

Browser mode uses a test-only plugin-http/fetch alias and CORS response bridge
forwarding real backend data. Native mode builds a separate debug executable
with an exact synthetic loopback capability override and test-only CDP/profile.
Neither mode certifies default native capability configuration, signed release,
MSI/NSIS install/update, another OS, human usability, full executable restart,
or MLS/E2EE. Renderer reload is not a native process restart.

`evidence/client-acceptance/publication-manifest.json` is the explicit publication
set with hashes. `.gitignore` excludes all other acceptance evidence by default.
Never stage the whole raw evidence tree with `git add -f`. Raw backend/build/DB
logs, commands (including synthetic DB credentials), screenshots, diagnostic
HTML/JSON, dumps, and profiles stay local. The curated text set is pattern-scanned;
this is not comprehensive secret-scanner certification. Preserve old evidence
unchanged; add new reports and handoff references rather than repairing old JSON.
