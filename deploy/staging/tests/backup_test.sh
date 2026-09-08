#!/usr/bin/env bash
# Entirely synthetic local PostgreSQL + GPG. SSH is a local fixture, never network.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
for tool in docker gpg gpgconf bash; do command -v "$tool" >/dev/null; done
docker image inspect postgres:16-alpine >/dev/null # never pull implicitly
WORK=$(mktemp -d "${TMPDIR:-/tmp}/terichat-backup-test.XXXXXXXX")
CONTAINER="terichat-backup-test-${RANDOM}-$$"
VOLUME=""
cleanup() {
    docker rm -fv "$CONTAINER" >/dev/null 2>&1 || true
    gpgconf --homedir "$WORK/keys" --kill gpg-agent >/dev/null 2>&1 || true
    rm -rf -- "$WORK"
    if [[ -n "$VOLUME" ]] && docker volume inspect "$VOLUME" >/dev/null 2>&1; then
        printf 'FAIL: fixture volume remains after cleanup: %s\n' "$VOLUME" >&2
        exit 1
    fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
umask 077
mkdir "$WORK/bin" "$WORK/keys" "$WORK/out"
export GNUPGHOME="$WORK/keys"
gpg --no-options --batch --pinentry-mode loopback --passphrase '' \
    --quick-generate-key 'Synthetic backup fixture (NOT FOR USE)' rsa2048 encr 1d >/dev/null 2>&1
gpg --no-options --batch --armor --export > "$WORK/recipient.asc"
export BACKUP_RECIPIENT_FILE="$WORK/recipient.asc" FIXTURE_CONTAINER="$CONTAINER"
export FIXTURE_MODE=ok FIXTURE_CALLED="$WORK/ssh-called"
cat > "$WORK/bin/ssh" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf called > "$FIXTURE_CALLED"
case "$FIXTURE_MODE" in
  fail) printf 'PGDMPpartial synthetic dump'; exit 42 ;;
  wrong) printf 'not a pg_dump archive'; exit 0 ;;
  empty) exit 0 ;;
  ok) exec docker exec "$FIXTURE_CONTAINER" pg_dump -U postgres -d synthetic -Fc ;;
esac
STUB
chmod +x "$WORK/bin/ssh"
export PATH="$WORK/bin:$PATH"
docker run --pull=never --detach --name "$CONTAINER" --network none \
    -e POSTGRES_HOST_AUTH_METHOD=trust postgres:16-alpine >/dev/null
VOLUME=$(docker inspect --format '{{range .Mounts}}{{if eq .Destination "/var/lib/postgresql/data"}}{{.Name}}{{end}}{{end}}' "$CONTAINER")
[[ -n "$VOLUME" ]] || { printf '%s\n' 'FAIL: fixture volume was not identified' >&2; exit 1; }
for ((i=0; i<60; i++)); do
    if docker exec "$CONTAINER" pg_isready -U postgres >/dev/null 2>&1; then break; fi
    sleep 1
done
docker exec "$CONTAINER" pg_isready -U postgres >/dev/null
docker exec "$CONTAINER" createdb -U postgres synthetic
printf '%s\n' "CREATE TABLE fixture (id integer PRIMARY KEY, body text NOT NULL);" \
    "INSERT INTO fixture VALUES (1, 'entirely synthetic one'), (2, 'entirely synthetic two');" \
    | docker exec -i "$CONTAINER" psql -X -v ON_ERROR_STOP=1 -U postgres -d synthetic >/dev/null
bash "$ROOT/backup.sh" synthetic-host "$WORK/out"
shopt -s nullglob
files=("$WORK/out/"*)
[[ ${#files[@]} == 1 && ${files[0]} == *.dump.gpg ]] || {
    printf '%s\n' 'FAIL: backup must publish only one encrypted .dump.gpg file' >&2; exit 1;
}
# Verify GPG integrity completely BEFORE restore; second decrypt streams into disposable DB.
gpg --no-options --batch --decrypt "${files[0]}" >/dev/null 2>&1
docker exec "$CONTAINER" createdb -U postgres restored
gpg --no-options --batch --decrypt "${files[0]}" 2>/dev/null \
    | docker exec -i "$CONTAINER" pg_restore --exit-on-error --no-owner --no-privileges -U postgres -d restored
actual=$(docker exec "$CONTAINER" psql -X -At -U postgres -d restored -c 'SELECT id, body FROM fixture ORDER BY id')
[[ "$actual" == $'1|entirely synthetic one\n2|entirely synthetic two' ]]
printf '%s\n' 'PASS: encrypted custom-format backup restores exact synthetic rows'
for mode in fail empty wrong; do
    rm -f "$WORK/out/"*; export FIXTURE_MODE="$mode"
    if bash "$ROOT/backup.sh" synthetic-host "$WORK/out"; then
        printf 'FAIL: %s source accepted\n' "$mode" >&2; exit 1
    fi
    [[ -z $(find "$WORK/out" -mindepth 1 -print -quit) ]]
    printf 'PASS: %s source fails without final/temporary artifacts\n' "$mode"
done
export FIXTURE_MODE=ok
rm -f "$FIXTURE_CALLED"
if BACKUP_RECIPIENT_FILE= bash "$ROOT/backup.sh" synthetic-host "$WORK/out"; then exit 1; fi
[[ ! -e "$FIXTURE_CALLED" ]]
printf '%s\n' 'PASS: missing recipient fails before SSH'
printf 'invalid synthetic public key\n' > "$WORK/invalid.asc"
if BACKUP_RECIPIENT_FILE="$WORK/invalid.asc" bash "$ROOT/backup.sh" synthetic-host "$WORK/out"; then exit 1; fi
[[ -z $(find "$WORK/out" -mindepth 1 -print -quit) ]]
printf '%s\n' 'PASS: invalid recipient/encryption failure cleans artifacts'
# A corrupted archive cannot pass the integrity check required by the restore runbook.
bash "$ROOT/backup.sh" synthetic-host "$WORK/out" >/dev/null
files=("$WORK/out/"*.dump.gpg)
size=$(wc -c < "${files[0]}")
dd if="${files[0]}" of="$WORK/truncated.gpg" bs=1 count="$((size - 16))" status=none
if gpg --no-options --batch --decrypt "$WORK/truncated.gpg" >/dev/null 2>&1; then exit 1; fi
printf '%s\n' 'PASS: truncated ciphertext rejected before restore'
# A second backup must not overwrite the first; failure preserves both.
bash "$ROOT/backup.sh" synthetic-host "$WORK/out" >/dev/null
files=("$WORK/out/"*.dump.gpg)
[[ ${#files[@]} == 2 ]]
export FIXTURE_MODE=fail
if bash "$ROOT/backup.sh" synthetic-host "$WORK/out"; then exit 1; fi
files=("$WORK/out/"*.dump.gpg)
[[ ${#files[@]} == 2 ]]
for file in "${files[@]}"; do gpg --no-options --batch --decrypt "$file" >/dev/null 2>&1; done
leftovers=("$WORK/out/".terichat-backup.*)
[[ ${#leftovers[@]} == 0 ]]
printf '%s\n' 'PASS: distinct generations survive a later failed backup'
