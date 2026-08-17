#!/usr/bin/env bash
# Set up a development checkout: install the git hooks and verify the toolchain.
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

install -m 755 scripts/pre-commit .git/hooks/pre-commit
echo "installed .git/hooks/pre-commit"

if ! command -v cargo >/dev/null; then
  echo "cargo not found — install a Rust toolchain (1.85 or newer)" >&2
  exit 1
fi

cargo --version
echo
echo "Run 'cargo test --workspace' to verify the checkout."
