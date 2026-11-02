#!/usr/bin/env bash
#
# Build a Debian package with nothing but dpkg-deb (M9).
#
#   packaging/deb/build.sh              # → target/packaging/hippocampus_<version>_<arch>.deb
#   packaging/deb/build.sh --no-build   # package what target/release already holds
#
# What goes where:
#   /usr/bin/{cortexd,cortex,cortex-mcp,cortex-preview,cortex-presence,hippocampus}
#   /usr/lib/systemd/user/hippocampus-{cortexd,overlay,presence}.service   (user units; enable per user)
#   /usr/share/applications/hippocampus.desktop
#   /usr/share/icons/hicolor/512x512/apps/hippocampus.png
#   /usr/share/doc/hippocampus/{README.md,copyright}
#
# The units in packaging/systemd start `%h/.local/bin/…`, which is the per-user install; the
# packaged copies are rewritten to /usr/bin. Nothing runs at install time — a package that
# starts a daemon on behalf of every user has decided something on their behalf (R5).
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

version="$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')"
case "$(uname -m)" in
  x86_64) arch=amd64 ;;
  aarch64) arch=arm64 ;;
  *) arch="$(uname -m)" ;;
esac

if [[ "${1:-}" != "--no-build" ]]; then
  echo "Building…"
  cargo build --release --bins
  (cd ui && npm run tauri build -- --no-bundle)
fi

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
pkg="$stage/hippocampus_${version}_${arch}"
mkdir -p "$pkg/DEBIAN" "$pkg/usr/bin" "$pkg/usr/lib/systemd/user" \
         "$pkg/usr/share/applications" "$pkg/usr/share/icons/hicolor/512x512/apps" \
         "$pkg/usr/share/doc/hippocampus"

for binary in cortexd cortex cortex-mcp cortex-preview cortex-presence hippocampus; do
  source="target/release/$binary"
  [[ -x "$source" ]] || { echo "missing: $source (build first, or drop --no-build)" >&2; exit 1; }
  install -m 0755 "$source" "$pkg/usr/bin/$binary"
done
for unit in hippocampus-cortexd.service hippocampus-overlay.service hippocampus-presence.service; do
  sed 's|%h/.local/bin/|/usr/bin/|' "packaging/systemd/$unit" > "$pkg/usr/lib/systemd/user/$unit"
  chmod 0644 "$pkg/usr/lib/systemd/user/$unit"
done
install -m 0644 packaging/hippocampus.desktop "$pkg/usr/share/applications/hippocampus.desktop"
install -m 0644 ui/src-tauri/icons/icon.png "$pkg/usr/share/icons/hicolor/512x512/apps/hippocampus.png"
install -m 0644 README.md "$pkg/usr/share/doc/hippocampus/README.md"
license="$(grep -m1 '^license' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')"
cat > "$pkg/usr/share/doc/hippocampus/copyright" <<COPYRIGHT
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: hippocampus
Source: $(grep -m1 '^repository' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')

Files: *
License: $license
COPYRIGHT

installed_kib=$(du -sk "$pkg/usr" | cut -f1)
{
cat <<CONTROL
Package: hippocampus
Version: $version
Section: utils
Maintainer: Hippocampus maintainers <maintainers@example.com>
Priority: optional
Architecture: $arch
Installed-Size: $installed_kib
CONTROL
cat <<'CONTROL'
Depends: libc6, libgtk-3-0, libwebkit2gtk-4.1-0
Recommends: poppler-utils, ffmpeg, udisks2, wl-clipboard
Suggests: trash-cli
Description: A memory daemon for your filesystem, and the file manager that grew out of it
 Hippocampus indexes what you have, learns what matters from how you use it,
 and lets you find files by what they are about. The daemon (cortexd) answers
 a CLI, an MCP server for agents and a search overlay; all of it works offline
 and nothing leaves the machine unless configured to.
 .
 After installing, run `cortex setup` as your user: it writes the
 configuration, enables the user services and runs the first index.
CONTROL
} > "$pkg/DEBIAN/control"

mkdir -p target/packaging
out="target/packaging/hippocampus_${version}_${arch}.deb"
dpkg-deb --build --root-owner-group "$pkg" "$out" >/dev/null
echo "$out"
