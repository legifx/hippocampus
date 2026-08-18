#!/usr/bin/env bash
#
# Undo `install-user.sh`. Leaves the index and the configuration alone — deleting somebody's
# data because they uninstalled a binary is not a thing a script should decide.

set -euo pipefail

bin_dir="${HOME}/.local/bin"
unit_dir="${XDG_CONFIG_HOME:-${HOME}/.config}/systemd/user"

for unit in hippocampus-overlay.service hippocampus-cortexd.service; do
  systemctl --user disable --now "${unit}" 2>/dev/null || true
  rm -f "${unit_dir}/${unit}"
done
systemctl --user daemon-reload

for binary in hippocampus cortex-preview cortex-mcp cortex cortexd; do
  rm -f "${bin_dir}/${binary}"
done

cat <<'NOTE'
Removed the binaries and the units.

Left alone, on purpose:
  ~/.config/hippocampus/       your configuration
  ~/.local/state/hippocampus/  the index, the trash and the undo journal
  ~/.cache/thumbnails/         the shared thumbnail cache, which is the desktop's
NOTE
