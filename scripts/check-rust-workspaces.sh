#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "$0")/.." && pwd)"

cargo +1.97.1 test --manifest-path "$root_dir/Cargo.toml" --workspace
cargo +1.97.1 clippy \
  --manifest-path "$root_dir/Cargo.toml" \
  --workspace --all-targets -- -D warnings
