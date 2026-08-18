#!/usr/bin/env bash
#
# Install Hippocampus for the current user: five binaries into ~/.local/bin, two systemd user
# units, and nothing outside the home directory.
#
# The overlay is installed as a *resident* service (M4, D41 option 1): it starts hidden at
# login and is summoned with Ctrl+Alt+Space. Its cold start is 418 ms, which is over the 300 ms
# the milestone asks for — paying that once at login is what makes it bearable, and the
# alternative is written up in D41.
#
# Everything here is reversible with `scripts/uninstall-user.sh`.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin_dir="${HOME}/.local/bin"
unit_dir="${XDG_CONFIG_HOME:-${HOME}/.config}/systemd/user"

# The overlay finds the thumbnailer beside itself rather than on PATH, so they have to be
# installed together — see `Thumbnailer::beside_this_binary`.
binaries=(cortexd cortex cortex-mcp cortex-preview hippocampus)

echo "Building…"
cargo build --release --bins --manifest-path "${repo_root}/Cargo.toml"
# The overlay must be built through the Tauri CLI: a plain cargo build embeds the dev server's
# URL instead of the frontend. See ui/README.md.
(cd "${repo_root}/ui" && npm run tauri build -- --no-bundle)

mkdir -p "${bin_dir}" "${unit_dir}"

for binary in "${binaries[@]}"; do
  source="${repo_root}/target/release/${binary}"
  if [[ ! -x "${source}" ]]; then
    echo "missing: ${source}" >&2
    exit 1
  fi
  install -m 0755 "${source}" "${bin_dir}/${binary}"
  echo "  ${bin_dir}/${binary}"
done

for unit in hippocampus-cortexd.service hippocampus-overlay.service; do
  install -m 0644 "${repo_root}/packaging/systemd/${unit}" "${unit_dir}/${unit}"
  echo "  ${unit_dir}/${unit}"
done

systemctl --user daemon-reload

cat <<'NOTE'

Installed. Nothing is running yet — enable what you want:

  systemctl --user enable --now hippocampus-cortexd.service   # the daemon, at login
  systemctl --user enable --now hippocampus-overlay.service   # the overlay, resident and hidden

The overlay is summoned with Ctrl+Alt+Space. If your compositor already owns that combination,
the service logs a warning and you can bind `hippocampus --show` in the compositor instead.

Configuration lives in ~/.config/hippocampus/config.toml. Content extraction is opt-in and
empty by default: name the directories you want read in `content_roots`.
NOTE
