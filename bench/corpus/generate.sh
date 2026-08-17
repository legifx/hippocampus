#!/usr/bin/env bash
#
# Build the reproducible benchmark corpus.
#
# The corpus is generated, never committed. From a fixed seed it produces an identical tree on
# any machine at any time, which is the only reason a latency number measured today can be
# compared with one measured next month.
#
#   ./generate.sh                 100 000 files, the standard benchmark corpus
#   ./generate.sh small           1 000 files, for quick local checks
#   ./generate.sh --files 500000  a deliberately oversized corpus
#
# Everything lands in bench/corpus/data/<preset>, which is git-ignored.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$here/../.." && pwd)"

preset="standard"
files=100000
extra=()

if [[ ${1:-} == "small" ]]; then
  preset="small"; files=1000; shift
elif [[ ${1:-} == "standard" ]]; then
  shift
fi
extra=("$@")

out="$here/data/$preset"

echo "building corpus-gen…"
cargo build --release -p corpus-gen --manifest-path "$repo_root/Cargo.toml" >/dev/null

"$repo_root/target/release/corpus-gen" \
  --out "$out" \
  --files "$files" \
  --force \
  "${extra[@]}"
