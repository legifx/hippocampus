#!/usr/bin/env bash
#
# Build an AppDir for the overlay and, if `appimagetool` is on PATH, an AppImage from it (M9).
#
# The AppImage carries the overlay and the daemon; the CLI and the MCP server are inside too,
# reachable as `Hippocampus.AppImage cortex …` through the AppRun below. The system's WebKitGTK
# is used rather than bundled: bundling it is a 100 MB argument with every distribution's
# graphics stack, and the .deb is the packaging that names its dependencies honestly.
#
#   packaging/appimage/build.sh              # → target/packaging/Hippocampus.AppDir (and .AppImage if possible)
#   packaging/appimage/build.sh --no-build
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

if [[ "${1:-}" != "--no-build" ]]; then
  cargo build --release --bins
  (cd ui && npm run tauri build -- --no-bundle)
fi

appdir="target/packaging/Hippocampus.AppDir"
rm -rf "$appdir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/applications" "$appdir/usr/share/icons/hicolor/512x512/apps"
for binary in cortexd cortex cortex-mcp cortex-preview hippocampus; do
  install -m 0755 "target/release/$binary" "$appdir/usr/bin/$binary"
done
install -m 0644 packaging/hippocampus.desktop "$appdir/hippocampus.desktop"
install -m 0644 packaging/hippocampus.desktop "$appdir/usr/share/applications/hippocampus.desktop"
install -m 0644 ui/src-tauri/icons/icon.png "$appdir/hippocampus.png"
install -m 0644 ui/src-tauri/icons/icon.png "$appdir/usr/share/icons/hicolor/512x512/apps/hippocampus.png"
cat > "$appdir/AppRun" <<'APPRUN'
#!/bin/sh
here="$(dirname "$(readlink -f "$0")")"
export PATH="$here/usr/bin:$PATH"
case "${1:-}" in
  cortexd|cortex|cortex-mcp|cortex-preview) bin="$1"; shift; exec "$here/usr/bin/$bin" "$@" ;;
  *) exec "$here/usr/bin/hippocampus" "$@" ;;
esac
APPRUN
chmod 0755 "$appdir/AppRun"
echo "$appdir"

if command -v appimagetool >/dev/null 2>&1; then
  ARCH="$(uname -m)" appimagetool "$appdir" "target/packaging/Hippocampus-$(uname -m).AppImage" >/dev/null
  echo "target/packaging/Hippocampus-$(uname -m).AppImage"
else
  echo "appimagetool not found; the AppDir above is complete, run appimagetool over it to get an AppImage" >&2
fi
