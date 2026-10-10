#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "$0")/.." && pwd)"
export MACOSX_DEPLOYMENT_TARGET=15.0

cargo +1.97.1 test --manifest-path "$root_dir/Cargo.toml" --workspace --locked
cargo +1.97.1 clippy \
  --manifest-path "$root_dir/Cargo.toml" \
  --workspace --all-targets --locked -- -D warnings
