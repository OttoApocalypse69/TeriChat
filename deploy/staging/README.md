# TeriChat staging (issue #9 / Alpha 0)

## Existing topology (not changed by this runbook)

The SPICE host edge Caddy owns **host ports 80/443 and public TLS**. It
forwards the TeriChat public site to the staging host's **8080**. Staging
Compose publishes `8080:80` for its separate web Caddy, which serves plain
HTTP, the React bundle, and same-origin `/v1/*` (including WebSocket),
`/health`, and `/ready` proxies to `api:3001`. Its SPA fallback must never
answer a readiness probe. PostgreSQL and API publish **no host ports**.
Do not probe host `127.0.0.1:3001` or add another 80/443 listener.

The staging Caddy accepts `STAGING_HOST` and `chat.unknownchat.xyz` as Host
names. The SPICE edge must preserve the public Host header when forwarding
(the normal Caddy reverse-proxy behavior). Host-port publication does not
change the in-container listener. No SPICE config is managed here.

The current `8080:80` mapping binds all host interfaces, not only Tailscale.
Host/cloud firewall policy must restrict unintended public bypass of TLS;
this repository change does not inspect or change those rules. Tailnet HTTP
is protected by WireGuard only when traffic actually uses that path.
Administrative access uses the authorized Tailscale SSH path; no public SSH
or DNS/edge changes are required by these fixes.

## Owner-operated bring-up / upgrade (not performed by this change)

On the authorized staging box (ARM64 Linux; confirm `uname -m` reports
`aarch64`), with Docker Compose v2, Tailscale SSH, and Git already
provisioned by the owner, prepare `.env` from the tracked template. Never
commit or put that file in test evidence:

```sh
cd ~/terichat/deploy/staging
cp .env.example .env
openssl rand -base64 32   # paste into POSTGRES_PASSWORD below
```

Set `STAGING_HOST` to the box Tailscale IP or MagicDNS name, set a distinct
strong `POSTGRES_PASSWORD`, and substitute that same password into
`DATABASE_URL` in place of its `***` placeholder so db and api agree.
Select an approved revision/artifact and record it before building:

```sh
git rev-parse HEAD
docker compose up --build -d
docker compose ps
docker compose logs api
```

Upgrade/restart runbook (owner executes on the box, in order):

1. Record the approved revision and take a verified pre-upgrade encrypted
   backup before migration-changing upgrades; approve rollback/roll-forward
   first. Migrations run on API boot.
2. Rebuild and recreate with `docker compose up --build -d` (config/code
   changes are baked into images; `restart` alone does not pick them up).
3. Check `docker compose ps` and `docker compose logs api`, then probe each
   layer explicitly as below and confirm status **and body**.
4. For a restart persistence check, use `docker compose restart api` and
   re-probe; the PostgreSQL named `pgdata` volume persists across restarts.
   Never use `down -v` as an upgrade/rollback step.

Probe each layer explicitly (replace placeholders; these are operator
commands, not evidence of a performed deployment):

```sh
# Inside API, not an unpublished host port:
docker compose exec -T api curl -fsS http://127.0.0.1:3001/health
docker compose exec -T api curl -fsS http://127.0.0.1:3001/ready
# On-box staging Caddy; Host must match the configured site:
curl -fsS -H 'Host: chat.unknownchat.xyz' http://127.0.0.1:8080/health
curl -fsS -H 'Host: chat.unknownchat.xyz' http://127.0.0.1:8080/ready
# Public TLS through SPICE, with SNI-correct pre-DNS override if needed:
curl --resolve 'chat.unknownchat.xyz:443:<VPS-IP>' https://chat.unknownchat.xyz/ready
```

Check status **and body**, not only a 200. Readiness failure must propagate
as the API's failure response, not HTML. Repeat a supported `/v1/*` and
WebSocket smoke plus a client-side SPA route. The API Compose healthcheck
currently uses `/health`; that is not a substitute for a database-backed
`/ready` deployment gate. This change does not change startup dependencies.

Caddyfile and server binary are baked into images: config/code changes need
`docker compose up --build -d web` or `api`, not merely `restart`/recreate.
Verify actual responses after deployment. Local rebuild evidence is not
immutable artifact promotion evidence; release promotion must identify the
actual tested digest per the repository release policy.

## Encrypted off-machine backup

Run `backup.sh` **on an independent operator/backup machine**, pulling over
SSH from staging. Running it on the staging VPS is not an off-machine
backup. This is an implemented pull path, not evidence that an independent
receiver/schedule has been provisioned.

Receiver prerequisites: Bash, OpenSSH, GnuPG 2.2+ with `--recipient-file`,
standard POSIX tools, a private output directory on a filesystem supporting
hard links, enough disk space, and an owner-verified public encryption
certificate. On Windows use Git Bash; NTFS ACLs must protect the directory
because `umask` alone does not establish Windows ACL policy. On the source,
SSH user access to the existing `~/terichat/deploy/staging` Compose project
and its `db` service is required. Dumping inside the PostgreSQL 16 service
keeps client/server major versions aligned. The script neither reads `.env`
locally nor copies application secrets.

```sh
# Receiver only. Paths are examples, not provisioned destinations or keys.
export BACKUP_RECIPIENT_FILE=/secure-config/staging-backup-recipient.asc
bash deploy/staging/backup.sh ubuntu@staging-tailnet-alias /private/backups
```

Provision SSH host keys through a separately verified channel. The script
requires strict host-key checking, noninteractive SSH authentication,
connection timeout and dead-peer keepalives; it never accepts a new host
key automatically. Configure least-privilege access and alert on any
nonzero exit or overdue backup. A responsive but stalled remote operation
still needs an operator/job-runner deadline; no scheduler is installed here.

### Recipient and key ownership

- Teri/authorized operations owner chooses the independent receiver,
  retention, restore custodian, and backup public key. These are **pending
  owner decisions**, not invented identities or configured services.
- `BACKUP_RECIPIENT_FILE` is one exported OpenPGP public certificate with a
  usable encryption key. Verify its full fingerprint with the restore
  custodian out of band before placing it in owner-controlled configuration.
  Do not select recipients by ambiguous short key ID/email or fetch keys
  automatically. One certificate is supported; multi-custodian escrow or
  multi-recipient changes need explicit review.
- Backup creation needs **only the public key**. Keep the passphrase-protected
  private key and recovery copy with the authorized restore custodian,
  separate from staging and preferably separate from the backup receiver.
  Never put it in this repository, CI, VPS, logs, or backup directory.
  The synthetic test's throwaway unprotected key is not an operational model.
- Loss of the private key loses these backups. Preserve old private keys for
  retained generations after rotation; a new recipient cannot decrypt old
  backups. Key compromise requires owner incident/retention decisions.
  Public-key encryption provides confidentiality and ciphertext integrity,
  **not proof of who made the backup**. Protect receiver storage/SSH access
  and the backup inventory against replacement/deletion; anyone with the
  public key can create a different valid encrypted archive.

The custom-format dump streams directly from SSH into GnuPG; no plaintext
backup file is created. A private temporary directory holds only encrypted
output and an isolated GPG keyring (no ambient GPG configuration/private
keyring). Empty/wrong-header streams, SSH/pg_dump failures, encryption or
write failures fail closed. Bash `pipefail` catches a failed dump even if
GPG successfully encrypts the partial stream. Only success atomically
publishes `terichat-staging-<UTC>-<random>.dump.gpg`, without overwriting an
existing file. EXIT/INT/TERM cleanup removes temporary artifacts. SIGKILL,
power loss, or filesystem failure can leave `.terichat-backup.*` directories
containing ciphertext, not plaintext; inspect/remove stale ones only when
no backup job owns them. Existing successful archives are never pruned by
the script. Set retention and independent monitoring explicitly.

### Compatibility

The positional interface `<ssh-host> [out-dir]` (default `./backups`), remote
Compose path, database/user `terichat`, and inner PostgreSQL `-Fc` format
are preserved. **Intentional fail-closed changes:** Bash is now required
(`sh backup.sh` is unsupported), recipient configuration is mandatory,
strict pre-provisioned SSH host keys are required, and output is `.dump.gpg`
instead of plaintext `.dump`. Update existing cron/globs/restore consumers
before enabling the replacement. No plaintext-output compatibility switch
is provided. Previously created `.dump` files remain readable by
`pg_restore`; protect/quarantine them under the owner's retention policy,
and encrypt them before off-machine storage. This script does not delete or
silently rewrite historical backups.

## Restore procedure (independent, disposable target first)

1. The authorized custodian obtains an encrypted archive from the independent
   receiver, verifies the recorded backup identity/source and intended
   snapshot time, and prepares an **isolated PostgreSQL 16** target with no
   app connections, public ports, real data, or Production credentials.
2. Copy the **ciphertext** into a private, owner-controlled working directory
   and do not allow it to change between verification and restore. Unlock the
   custodian's private key using GnuPG's secure pinentry/agent, not a command
   line passphrase. Fully verify/decrypt to the null sink first:

   ```sh
   gpg --decrypt /private/restore/snapshot.dump.gpg > /dev/null
   ```

   Stop on nonzero exit. GPG may emit plaintext before detecting a bad final
   integrity packet, so do not initially pipe unverified ciphertext straight
   into a database. Use the unchanged protected copy for the next step.
3. Create an empty disposable database named `restore_drill` using the
   isolated target's `createdb`. With Bash `set -o pipefail`, stream the
   verified backup into that target (example uses an independently created
   local container named `isolated-restore`, **not staging Compose**):

   ```sh
   set -o pipefail
   gpg --decrypt /private/restore/snapshot.dump.gpg |
     docker exec -i isolated-restore pg_restore --exit-on-error \
       --no-owner --no-privileges -U postgres -d restore_drill
   ```

   Any error invalidates the drill: discard the disposable target and
   investigate. There is no `--clean --create` against a live `postgres`
   database here. For historical plaintext `.dump`, supply it directly on
   stdin to this same isolated `pg_restore` command; no GPG step applies.
4. Compare expected per-table counts **and representative content**, schema,
   constraints and migration tracker with the snapshot's recorded inventory.
   Before exposing any restored history, apply required deletion tombstones
   and reconcile writes/deletions after the snapshot. A database dump does
   not include role globals, external object bytes, client MLS keys, or
   external configuration. Inventory/back up those separately where needed;
   this script is a database backup, not complete service disaster recovery.
5. Record archive identity, source/artifact revision, PostgreSQL/GPG versions,
   custodian/operator, result, restore duration and losses/reconciliation.
   Only then consider an owner-approved maintenance-window cutover, with app
   stopped, retained original database, and reviewed migration state. Never
   replay migrations blindly or rename/drop a live database for a drill.

## Executable regressions and evidence

```sh
python deploy/staging/tests/test_ready.py
python deploy/staging/tests/ready_runtime.py
bash deploy/staging/tests/backup_test.sh
```

`test_ready.py` is a dependency-free Caddy configuration contract regression.
`ready_runtime.py` needs an approved **already-installed** Caddy 2 binary
(`CADDY_BIN` may select it) and uses only loopback synthetic services. It
checks real proxy status/body for readiness 503/200, `/v1` and SPA routing.
Missing Caddy is BLOCKED with exit 2, not a pass or silent skip.

`backup_test.sh` uses the already-local `postgres:16-alpine` image, a fresh
network-isolated disposable container, synthetic rows, an isolated
throwaway GPG keyring, and a local SSH stub. No image is pulled and no real
SSH/Compose/dump is invoked. It verifies encrypted backup + real pg_restore,
exact rows, source/encryption failures and cleanup, and corrupt-ciphertext
rejection. It is **not** an off-machine transport or ARM64 drill.

See [tests/EVIDENCE.md](tests/EVIDENCE.md) for actual results and blockers.
Before declaring staging complete, the owner still must verify native
ARM64 deployment, actual two-layer Caddy/TLS/WS responses, firewall policy,
independent receiver/key custody, scheduled retention/alerts and an
owner-operated off-machine restore drill. Do not infer these from local tests.

## Rollback scope

No topology, database schema/data, deployment, DNS, SPICE files, or live
resources were changed here. Integrate/revert only the staging files. An
operator rolling back the Caddy image must use the last approved artifact
and verify readiness (old config reintroduces the SPA false-200 bug).
Do not silently roll back to plaintext backup behavior; pause jobs and alert
until an approved encrypted path works. Application/data rollback is a
separate owner-approved action: a backup restore loses post-snapshot writes
and requires the migration/tombstone reconciliation above.
