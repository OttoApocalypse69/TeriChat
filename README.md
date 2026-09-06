# TeriChat — backend baseline (Alpha 0)

Spec map: [`00_README.md`](00_README.md). Operating rules: [`AGENTS.md`](AGENTS.md).
Human checklist: [`TERI_TODO.md`](TERI_TODO.md). Next tasks: [`11_ROADMAP_AND_TASKS.md`](11_ROADMAP_AND_TASKS.md).

## What runs today

- `terichat-server` (Axum): `GET /health` (liveness), `GET /ready` (readiness).
  Without `DATABASE_URL`, `/ready` reports `{"database":"not_configured"}`.
- `compose.yaml`: local PostgreSQL 16 (for the upcoming migrations/auth slice).
  NATS is intentionally deferred until outbox distribution needs it.

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

## Next unblocked work (Milestone A → B)

1. ~~First SQLx migration~~ — done: `migrations/20260906202456_identity.sql`
   (`users`/`devices`/`sessions` + `updated_at` trigger, applied on boot).
2. Config loader that reads `.env` keys documented in `.env.example`.
3. Auth slice: register/login/logout + Argon2id + revocation (Milestone B).
