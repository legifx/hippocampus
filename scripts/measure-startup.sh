#!/usr/bin/env bash
#
# Measure the overlay's cold start, the same way every time.
#
# This script exists because the number was reported wrong twice, in the same way both times: a
# single run taken immediately after a build, on a machine that was still compiling, quoted
# against a budget. It read as 418 ms and then as 366 ms; measured properly it was 265 and 278.
#
# What "properly" means, and why each part is here:
#
#   - **Ten runs, not one.** The spread between the fastest and slowest run is around 20 ms, and
#     one sample cannot tell that from a regression.
#   - **A warm page cache.** The first run after a build reads the binary from disk and is
#     slower for a reason that has nothing to do with the code. The first run is discarded.
#   - **An idle machine.** The script refuses to report a number while the load average says
#     otherwise, because a latency measurement taken next to a compiler measures the compiler.
#   - **The service stopped.** A second invocation reaches the resident process and returns
#     immediately, which would measure nothing at all.
#
# Usage: scripts/measure-startup.sh [runs]

set -euo pipefail

runs="${1:-10}"
binary="${HOME}/.local/bin/hippocampus"
budget=300

if [[ ! -x "${binary}" ]]; then
    binary="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/target/release/hippocampus"
fi
if [[ ! -x "${binary}" ]]; then
    echo "no overlay binary; build it with: cd ui && npm run tauri build -- --no-bundle" >&2
    exit 1
fi

load="$(cut -d' ' -f1 /proc/loadavg)"
if (( $(echo "${load} > 3.0" | bc -l) )); then
    echo "the load average is ${load}." >&2
    echo "A startup measurement taken next to a compiler measures the compiler. Wait." >&2
    exit 1
fi

was_running=no
if systemctl --user is-active --quiet hippocampus-overlay.service 2>/dev/null; then
    was_running=yes
    systemctl --user stop hippocampus-overlay.service
    sleep 1
fi
restore() {
    if [[ "${was_running}" == yes ]]; then
        systemctl --user start hippocampus-overlay.service
    fi
}
trap restore EXIT

work="$(mktemp -d)"
trap 'rm -rf "${work}"; restore' EXIT

echo "measuring ${runs} starts of ${binary}…"
for ((i = 0; i <= runs; i++)); do
    # The window never appears: the overlay is resident and starts hidden. Run zero warms the
    # page cache and is thrown away.
    timeout 10 "${binary}" > "${work}/run${i}.log" 2>&1 || true
done

sed -i 's/\x1b\[[0-9;]*m//g' "${work}"/*.log

python3 - "${work}" "${budget}" <<'PY'
import pathlib, re, sys

work, budget = pathlib.Path(sys.argv[1]), int(sys.argv[2])
pattern = re.compile(r"(\w+_ms)=(\d+)")

rows = []
for log in sorted(work.glob("run*.log")):
    # Run zero is the cold-cache one and is not part of the answer.
    if log.stem == "run0":
        continue
    fields = dict((k, int(v)) for k, v in pattern.findall(log.read_text()))
    if "interactive_ms" in fields:
        rows.append(fields)

if not rows:
    print("no runs reported a cold start; is the daemon running?")
    raise SystemExit(1)

def column(key):
    return sorted(r[key] for r in rows if key in r)

def median(values):
    return values[len(values) // 2]

print(f"\n{len(rows)} runs, first discarded:\n")
for key in ("setup_ms", "dom_ready_ms", "page_ms", "interactive_ms"):
    values = column(key)
    if values:
        print(f"  {key:16} median {median(values):4}   min {values[0]:4}   max {values[-1]:4}")

interactive = column("interactive_ms")
inside = sum(1 for value in interactive if value <= budget)
print(f"\n  budget {budget} ms — {inside} of {len(interactive)} runs inside it")
print(f"  verdict: {'MET' if inside == len(interactive) else 'MISSED'}\n")
PY
