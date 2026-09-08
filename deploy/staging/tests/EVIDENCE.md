# Staging fix handoff and evidence

- Task: issue #9 / Alpha 0 staging repository gaps; parent-directed scoped fix.
- Branch/worktree: `fix/alpha0-staging`, `C:/Users/TeRiRi/Documents/GitHub/TeriChat-fix-staging`.
- Base and unchanged HEAD: `c325b4c90df5087a6f10ff6b5a2bc24787a88ec1`.
- Candidate: uncommitted modifications under `deploy/staging` only. No branch,
  checkout, commit, push, merge, deployment, DNS, SSH connection or real-data operation.
- Risk: Critical path floor (deployment/backup scripts); independent review and
  authorized human integration required. Local verification is not release approval.
- State: ready for review; actual Caddy runtime and environment acceptance blocked.

## Delivered / acceptance

| Criterion | Result | Evidence |
|---|---|---|
| Explicit `/ready` API route, not SPA | Implemented; contract regression red -> green | `test_ready.py`, Caddyfile diff |
| Actual Caddy HTTP status/body regression | Implemented, **BLOCKED** execution | `ready_runtime.py`, missing binary exit 2 |
| Existing SPICE 80/443 TLS -> staging 8080 topology | Preserved; docs corrected | README and unchanged compose/Dockerfiles |
| Independent-machine encrypted backup path | Implemented receiver-side SSH pull + GnuPG stream | `backup.sh`; synthetic local test below |
| No plaintext final archive; source/encryption failures cleaned | PASS in synthetic test | `backup_test.sh` |
| Real PostgreSQL custom dump restoration, exact rows | PASS with fresh disposable local PostgreSQL | `backup_test.sh` |
| Custodian/recipient/retention/legacy/restore documentation | Documented; actual provisioning pending owner | README |
| Off-machine/native ARM64/live staging drill | **NOT RUN** | No authorization/environment acceptance inferred |

Changed files: `Caddyfile`, `backup.sh`, `README.md`; added
`tests/test_ready.py`, `tests/ready_runtime.py`, `tests/backup_test.sh`,
`tests/evidence-local.json`, and this file. No schemas, manifests, images,
Compose topology, SPICE files, or application code changed.

## RED evidence (observed before implementation)

Executed `python deploy/staging/tests/test_ready.py` against the original
Caddyfile. Exit 1:

```text
FAIL: test_ready_has_explicit_api_handler_before_spa
AssertionError: unexpectedly None : /ready falls through to the SPA instead of API readiness
Ran 1 test
FAILED (failures=1)
```

Executed `bash deploy/staging/tests/backup_test.sh` against original
`backup.sh`, with a local SSH stub, a fresh local PostgreSQL container and
synthetic rows/key only. Exit 1:

```text
-rw------- ... 1608 ... /tmp/terichat-backup-test.HV9DSTFR/out/terichat-staging-20260908-084350.dump
FAIL: backup must publish only one encrypted .dump.gpg file
```

Both failures were observed before their respective production fixes.
The very first backup harness attempt failed at `mkdir -m 700` on Git Bash
with Permission denied; changed the fixture to `umask 077; mkdir` and then
observed the meaningful plaintext-output RED above. This was a test-host
portability issue, not counted as a product regression. The README explicitly
requires Windows ACL provisioning rather than claiming POSIX permissions
certify NTFS confidentiality.

## Final executed commands

Raw returned stdout, stderr-combined diagnostics and **individual exit codes**
are saved in [evidence-local.json](evidence-local.json). Commands ran from the
worktree root:

| Actual command | Result |
|---|---|
| `python deploy/staging/tests/test_ready.py` | PASS, 1 test, exit 0 |
| `python deploy/staging/tests/ready_runtime.py` | BLOCKED, no Caddy binary, exit 2 |
| `bash -n deploy/staging/backup.sh deploy/staging/tests/backup_test.sh` | PASS, exit 0 |
| `bash deploy/staging/tests/backup_test.sh` | PASS, exit 0 |
| `git diff --check` | PASS, exit 0 |
| `docker image inspect postgres:16-alpine --format "{{.Architecture}} {{.Id}}"` | Local image is **amd64**, not ARM64 |
| `docker ps -a --filter name=terichat-backup-test --format "{{.Names}}"` | Empty, fixture containers cleaned |
| `git status --short` | Only staging paths changed |

Available GnuPG reported 2.4.9, Docker server 29.6.2. Existing PostgreSQL
image used without pulling:
`sha256:cf78e76683b9ca8c5733cbbdce6c9262b45b6767934dd0a95e671f9a0fc20685`.
No dependencies installed. GnuPG uses an isolated throwaway keyring; its
synthetic private key and fixture dumps are deleted, never saved in evidence.
The fixture's Docker container has no network and no published ports, and
the SSH shim never invokes real SSH or staging Compose.

The backup test reported:

```text
PASS: encrypted custom-format backup restores exact synthetic rows
PASS: fail source fails without final/temporary artifacts
PASS: empty source fails without final/temporary artifacts
PASS: wrong source fails without final/temporary artifacts
PASS: missing recipient fails before SSH
PASS: invalid recipient/encryption failure cleans artifacts
PASS: truncated ciphertext rejected before restore
PASS: distinct generations survive a later failed backup
```

The source-failure case emits a valid `PGDMP` header and partial bytes before
exit 42, exercising `pipefail` rather than only header rejection. The
restore creates a separate database and compares both synthetic rows,
not just an archive listing. Expected GPG/missing-recipient diagnostics in
negative cases are recorded; the test does not retry failed assertions.

## Remaining findings / owner blockers

- An approved local **Caddy 2 binary** (or separately authorized provision of
  one) is needed to execute `ready_runtime.py`. No Caddy binary or cached
  Caddy image was present. Request coordinated dependency provisioning;
  do not classify the config contract test as a real Caddy runtime pass.
- Owner/environment work: independent backup receiver, verified SSH host key
  and restricted account, recipient fingerprint and private-key custodian,
  retention/alert schedule, actual off-machine restore rehearsal.
- Owner/environment work: native ARM64 artifact execution, public TLS and
  two-layer Host routing, WebSocket smoke, and host/cloud firewall exposure
  of all-interface `8080:80`. Current mapping is preserved, not certified.
- Actual service-data disaster recovery additionally needs an inventory for
  external objects/config/roles and tombstone/migration reconciliation. The
  implemented path covers PostgreSQL only.
- NOT RUN: live SSH/VPS/Compose operations, real dumps, `.env`/secret reads,
  external network drills, ARM64 execution, full Rust/frontend suite,
  dependency/secret scanner and independent Critical review. No Rust or
  frontend source was modified; this evidence is scoped, not a full merge gate.

## Compatibility / rollback / handoff

No data or migration changes. SSH host/out-dir positional interface, default
`./backups`, remote Compose path and PostgreSQL custom format are preserved.
Bash, `BACKUP_RECIPIENT_FILE`, pre-provisioned known_hosts and `.dump.gpg`
consumers are intentional fail-closed operational prerequisites; plaintext
output is not retained. Restore old `.dump` with ordinary isolated
`pg_restore` as documented. Preserve old private keys for old generations.

Parent owns integration and independent review. Revert only these staging
files if needed; no live rollback is required. Do not resume insecure
plaintext backup jobs as an automatic rollback. A deployed application/data
rollback needs separate owner approval and verified migration compatibility;
restoring a snapshot discards later writes. Next unblocked action is review
of this diff and coordinated provision of Caddy for the runtime probe.
