#!/usr/bin/env bash
# Run the API server locally (no DB required for baseline).
set -euo pipefail
cd "$(dirname "$0")/.."
exec cargo run -p terichat-server
