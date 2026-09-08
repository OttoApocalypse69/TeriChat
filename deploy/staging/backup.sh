#!/usr/bin/env bash
# Pull a custom-format dump over SSH and encrypt on the independent receiver.
# Usage remains: bash backup.sh <ssh-host> [out-dir]
# Required: BACKUP_RECIPIENT_FILE=owner-verified-public-key.asc (GnuPG 2.2+).
set -euo pipefail
umask 077
[[ $# -ge 1 && $# -le 2 ]] || { printf 'usage: backup.sh <ssh-host> [out-dir]\n' >&2; exit 2; }
HOST=$1
OUT=${2:-./backups}
[[ "$HOST" =~ ^[a-zA-Z0-9_][a-zA-Z0-9_.@:-]*$ ]] || { printf 'Invalid SSH host/alias\n' >&2; exit 2; }
: "${BACKUP_RECIPIENT_FILE:?Set BACKUP_RECIPIENT_FILE to an owner-verified public encryption key}"
[[ -f "$BACKUP_RECIPIENT_FILE" && -r "$BACKUP_RECIPIENT_FILE" ]] || { printf 'Recipient file is not readable\n' >&2; exit 2; }
for tool in ssh gpg mktemp ln; do command -v "$tool" >/dev/null; done
mkdir -p -- "$OUT"
WORK=$(mktemp -d "$OUT/.terichat-backup.XXXXXXXXXX")
cleanup() { rm -rf -- "$WORK"; }
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir "$WORK/keyring"
# No ambient GPG config/keyring, private keys, trust prompts, or plaintext file.
# The owner verifies the public certificate before provisioning this file.
# PGDMP is pg_dump -Fc's five-byte magic; reject empty/wrong successful streams.
ssh -o BatchMode=yes -o StrictHostKeyChecking=yes -o ConnectTimeout=15 \
    -o ServerAliveInterval=15 -o ServerAliveCountMax=3 -- "$HOST" \
    'cd ~/terichat/deploy/staging && docker compose exec -T db pg_dump -U terichat -d terichat -Fc' \
    | { IFS= read -r -N 5 magic; [[ "$magic" == PGDMP ]]; printf '%s' "$magic"; cat; } \
    | gpg --no-options --homedir "$WORK/keyring" --batch --no-tty \
        --recipient-file "$BACKUP_RECIPIENT_FILE" --encrypt --output "$WORK/archive.dump.gpg"
# pipefail prevents publishing an encrypted but incomplete dump on SSH failure.
[[ -s "$WORK/archive.dump.gpg" ]]
FILE="$OUT/terichat-staging-$(date -u +%Y%m%dT%H%M%SZ)-${WORK##*.}.dump.gpg"
# Same-filesystem hard link publishes atomically, fails rather than overwriting.
ln -- "$WORK/archive.dump.gpg" "$FILE"
printf '%s\n' "$FILE"
