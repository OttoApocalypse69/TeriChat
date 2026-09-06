#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
exec cargo clippy --workspace --all-targets -- -D warnings
