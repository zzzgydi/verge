#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)

mihomo_bin=$("$script_dir/ensure-mihomo.sh")
export VERGE_MIHOMO_BIN=$mihomo_bin

cd "$repo_dir"
exec cargo +1.97.1 run --manifest-path apps/verge-gpui/Cargo.toml "$@"
