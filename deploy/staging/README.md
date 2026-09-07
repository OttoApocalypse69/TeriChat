# TeriChat staging (issue #9)

Single-box staging on ARM64: PostgreSQL 16 + Axum API + Caddy (TLS, static
web UI, same-origin `/v1/*` proxy so browsers never hit CORS). Only ports
80/443 are published; Postgres and the API stay on the internal network.
Admin path is Tailscale SSH (`ubuntu@<tailnet-ip>`); no public SSH needed.

## First bring-up (on the box)

```sh
cd ~/terichat/deploy/staging
cp .env.example .env
# Fill STAGING_HOST (public hostname) and POSTGRES_PASSWORD (openssl rand -base64 32).
docker compose up --build -d
docker compose logs -f api   # wait for "migrations applied" + "listening"
```

Migrations run automatically on API boot (embedded migrator). Verify:

```sh
curl -fsS http://127.0.0.1:3001/health          # direct, on-box
curl -fsS http://$STAGING_HOST/health           # through Caddy (tailnet)
```

(Public-TLS mode, when a hostname lands: replace with
`https://$STAGING_HOST/health` plus the `--resolve` pre-DNS check below.)

Pre-DNS check from anywhere (SNI-correct, bypasses DNS):

```sh
curl --resolve $STAGING_HOST:443:<VPS-IP> https://$STAGING_HOST/health
```

## Upgrade / restart

```sh
cd ~/terichat/deploy/staging
git pull
docker compose up --build -d
docker compose ps   # api + web healthy; db untouched, pgdata persists
```

Restart preserves the database (named `pgdata` volume). To restart only the
API: `docker compose restart api`.

> Gotcha: the Caddyfile is `COPY`d into the web image (and the server binary
> into the api image), so config/code changes need
> `docker compose up --build -d <svc>` — a plain `up -d` only restarts.
> Tarball deploys make it worse: extraction replaces the file (new inode),
> orphaning any bind mount. Always `--build` after deploying, then verify the
> live config (`exec web wget -q -O- http://127.0.0.1:2019/config/...`).

## Backup (off-machine) + restore drill

Nightly dump to the operator's machine (run from anywhere with SSH):

```sh
./backup.sh <tailnet-or-public-ip> ./backups
```

`backup.sh` writes `terichat-staging-<date>.dump` (Postgres custom format).
Restore drill (staging DB only, app stopped so renames succeed):

```sh
docker compose stop api
cat backup.dump | docker compose exec -T db pg_restore -U terichat -d postgres \
  --clean --create
docker compose start api
# Compare row counts per table before/after; see issue #9 acceptance.
```

Drill log (fill after performing): _pending first staging deploy_.

## Rollback

Redeploy = `git checkout <last-good> && docker compose up --build -d`.
Data rollback = restore the pre-upgrade dump (taken by `backup.sh` before
every migration-changing deploy).
