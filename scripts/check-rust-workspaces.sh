#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "$0")/.." && pwd)"

cargo +1.97.1 test \
  --manifest-path "$root_dir/Cargo.toml" \
  -p verge-domain \
  -p verge-helper-protocol \
  -p verge-helper \
  -p verge-config \
  -p verge-core \
  -p verge-platform \
  -p verge-application \
  -p verge-runtime \
  -p verge-ipc \
  -p verge-ui
cargo +1.97.1 clippy \
  --manifest-path "$root_dir/Cargo.toml" \
  -p verge-domain \
  -p verge-helper-protocol \
  -p verge-helper \
  -p verge-config \
  -p verge-core \
  -p verge-platform \
  -p verge-application \
  -p verge-runtime \
  -p verge-ipc \
  -p verge-ui \
  --all-targets -- -D warnings
cargo +1.97.1 test --manifest-path "$root_dir/apps/verge-gpui/Cargo.toml"
cargo +1.97.1 clippy \
  --manifest-path "$root_dir/apps/verge-gpui/Cargo.toml" \
  --all-targets -- -D warnings
