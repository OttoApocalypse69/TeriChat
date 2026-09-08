# UnknownChat — Alpha 0 development baseline

Repository, executable, package, and bundle identifiers still use TeriChat. The visible application branding changed in PR #22; this is not an identifier or data migration.

**Not ready for sensitive communications:** the current web/desktop client uses demo-plaintext envelopes. The Rust sealed-DM prototype is not integrated client MLS/E2EE.

Current evidence and open gates: [`IMPLEMENTATION_STATUS.md`](docs/development/IMPLEMENTATION_STATUS.md).

Spec map: [`00_README.md`](00_README.md). Operating rules: [`AGENTS.md`](AGENTS.md).
Human checklist: [`TERI_TODO.md`](TERI_TODO.md). Next tasks: [`11_ROADMAP_AND_TASKS.md`](11_ROADMAP_AND_TASKS.md).

## What runs today

- `terichat-server` (Axum): `GET /health` (liveness), `GET /ready` (readiness).
  Without `DATABASE_URL`, `/ready` reports `{"database":"not_configured"}`.
- Auth/session/device APIs, DM/group messaging, history, transactional outbox,
  WebSocket gateway, workspace/channel moderation, and Stats baseline.
- React/TypeScript web client and Tauri shell under `apps/desktop`, including
  server-loaded conversation lists, peer names, timestamps, and previews.
- `compose.yaml`: local PostgreSQL 16. NATS is intentionally deferred until
  outbox distribution needs it.
- Staging configuration: [`deploy/staging/README.md`](deploy/staging/README.md).
  Public application address: https://chat.unknownchat.xyz. Backup/restore and
  operational acceptance remain separate from URL reachability.

## Toolchain

- Rust stable **1.97.1** (locally verified) + rustfmt + clippy. CI pins `1.97`.
- No `rust-toolchain.toml` committed: on this dev machine rustup's channel
  self-update for a pinned file is broken (partial-install rename errors),
  so the pin lives in CI + this README until that is fixed.
- Docker 29 + Compose v5 for `compose.yaml`.

## Commands

```sh
cp .env.example .env        # optional for baseline
./scripts/dev.sh            # run server on 127.0.0.1:3001
./scripts/check.sh          # fmt --check + clippy + tests
./scripts/ci.sh             # check + build (mirrors CI)
docker compose up -d db     # start local PostgreSQL
docker compose config       # validate compose file
```

## Next work — finish Alpha 0 acceptance

1. Reconcile desktop #5, Stats #7, staging #9, and modularization #10 against
   their full acceptance criteria; merged code alone does not close an issue.
2. Prioritize MLS/OpenMLS device/group state (#6) and recovery mnemonic
   specification/implementation (#11), with explicit design and review gates.
3. Complete backup/restore evidence for staging #9; do not infer it from TLS.
4. Continue the virtual ledger #8 after higher-priority gaps and dependencies.

See the [evidence snapshot](docs/development/IMPLEMENTATION_STATUS.md) for
exact revisions, executed checks, and verification still outstanding.
