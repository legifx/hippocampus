#!/usr/bin/env bash
#
# Build the reproducible benchmark corpus.
#
# The corpus is generated, never committed. From a fixed seed it produces an identical tree on
# any machine at any time, which is the only reason a latency number measured today can be
# compared with one measured next month.
#
#   ./generate.sh                 100 000 files with content, the standard benchmark corpus
#   ./generate.sh small           1 000 files, for quick local checks
#   ./generate.sh --files 500000  a deliberately oversized corpus
#   ./generate.sh --no-content    paths only, as M1 used it — faster and far smaller
#
# Content is written by default from M2 onwards. The paths are identical either way, because
# they come from the same seed, so path-search numbers stay comparable across the change; what
# changes is that the content index has something to index.
#
# Everything lands in bench/corpus/data/<preset>, which is git-ignored.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$here/../.." && pwd)"

preset="standard"
files=100000
content="--with-content"
extra=()

if [[ ${1:-} == "small" ]]; then
  preset="small"; files=1000; shift
elif [[ ${1:-} == "standard" ]]; then
  shift
fi

for arg in "$@"; do
  if [[ $arg == "--no-content" ]]; then
    content=""
  else
    extra+=("$arg")
  fi
done

out="$here/data/$preset"

# The corpus is data for a benchmark, not for the index. When the repository sits inside a
# content root — which it does on the reference machine — the daemon would otherwise index
# 293 000 generated files as if they were the owner's, and did, until 2026-09-05. The marker is
# the daemon's own exclusion mechanism (`denylist::IGNORE_MARKER`), placed once for every preset.
mkdir -p "$here/data"
: > "$here/data/.brainignore"

echo "building corpus-gen…"
cargo build --release -p corpus-gen --manifest-path "$repo_root/Cargo.toml" >/dev/null

"$repo_root/target/release/corpus-gen" \
  --out "$out" \
  --files "$files" \
  --force \
  ${content:+$content} \
  "${extra[@]}"

# The content database derived from a corpus describes the corpus that no longer exists.
rm -f "$out-content.db" "$out-content.db-wal" "$out-content.db-shm"
