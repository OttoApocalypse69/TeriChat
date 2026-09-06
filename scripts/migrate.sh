#!/usr/bin/env bash
# Apply pending migrations to DATABASE_URL (see .env.example).
set -euo pipefail
cd "$(dirname "$0")/.."
: "${DATABASE_URL:?DATABASE_URL must be set (copy .env.example to .env)}"
exec sqlx migrate run
