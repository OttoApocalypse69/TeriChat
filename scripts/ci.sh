#!/usr/bin/env bash
# Format check + clippy + tests + build. Mirrors CI.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
