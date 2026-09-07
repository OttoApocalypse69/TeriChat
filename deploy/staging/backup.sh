#!/bin/sh
# Off-machine staging backup: pg_dump (custom format) over SSH.
# Usage: ./backup.sh <ssh-host> [out-dir]
set -eu
HOST="${1:?usage: backup.sh <ssh-host> [out-dir]}"
OUT="${2:-./backups}"
mkdir -p "$OUT"
FILE="$OUT/terichat-staging-$(date +%Y%m%d-%H%M%S).dump"
ssh "$HOST" "cd ~/terichat/deploy/staging && docker compose exec -T db pg_dump -U terichat -d terichat -Fc" > "$FILE"
ls -la "$FILE"
