#!/usr/bin/env bash
#
# Public functions that nothing outside their own file calls.
#
# This exists because one session found the same defect five times: code written from the
# specification, tested at the moment it was written — so the test called it directly — and
# called by production never. The worst of them had a user interface built on top: a panel
# displayed a number, the number was always zero, every test passed.
#
#   scripts/uncalled.sh                    # the whole workspace
#   scripts/uncalled.sh crates/foo/src/*.rs
#
# **It is a prompt, not a verdict.** Three shapes come out of it that are not bugs:
#
#   * a query helper whose only user is a test assertion — that is test infrastructure;
#   * something reached through a trait, a macro or `serde`, which no call site names;
#   * a genuinely public API of a library crate somebody outside this tree calls.
#
# The line that matters is: *code whose purpose is to make something happen in production, and
# which production never calls, is a bug.* Everything else is a judgement, and the judgement is
# the point — sorting the list is more useful than producing it.
#
# Exits 0 always. This is a thinking aid, not a gate: a gate here would be argued down to
# nothing within a month by the three false-positive shapes above.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

files=("$@")
if [ ${#files[@]} -eq 0 ]; then
  mapfile -t files < <(find crates bench -name '*.rs' -not -path '*/target/*' | sort)
fi

found=0
for file in "${files[@]}"; do
  [ -f "$file" ] || continue
  while read -r name; do
    [ -z "$name" ] && continue
    # A bare call as well as a method or path call: a function imported with `use` is called by
    # its bare name, and the first version of this script missed every one of those.
    callers=$(grep -rn --include='*.rs' -e "\b${name}(" crates bench ui/src-tauri/src 2>/dev/null \
      | grep -v "^${file}:" \
      | grep -vc "pub fn ${name}(" || true)
    if [ "${callers:-0}" = "0" ]; then
      echo "no caller outside its file: ${name}  (${file})"
      found=$((found + 1))
    fi
  done < <(grep -o 'pub fn [a-z_][a-z_0-9]*' "$file" | sed 's/pub fn //' | sort -u)
done

echo
echo "${found} to look at. Sorting them is the work; see the header for the three shapes that"
echo "are not bugs and the one line that decides."
