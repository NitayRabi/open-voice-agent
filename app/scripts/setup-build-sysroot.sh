#!/usr/bin/env bash
# Build a local Fedora sysroot with the GTK3 / WebKitGTK 4.1 / libsoup3 -devel
# closure, for machines that have the runtime .so files but no -devel packages
# and no root. Downloads RPMs with `dnf download` (no root needed) and unpacks
# them under ~/.cache/ova-build-sysroot.
#
#   ./scripts/setup-build-sysroot.sh
#   source ./scripts/build-env.sh
#   cargo tauri build            # or: cd src-tauri && cargo build --release
#
# On a normal desktop with the -devel packages installed you do not need this;
# just `dnf install webkit2gtk4.1-devel gtk3-devel libsoup3-devel` (Fedora) or
# the equivalent, and skip straight to `cargo tauri build`.
set -euo pipefail

SYSROOT="${OVA_SYSROOT:-$HOME/.cache/ova-build-sysroot}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

PKGS=(
  webkit2gtk4.1-devel
  javascriptcoregtk4.1-devel
  gtk3-devel
  libsoup3-devel
  glib2-devel
  cairo-devel
  cairo-gobject-devel
  pango-devel
  gdk-pixbuf2-devel
  atk-devel
  libxdo-devel
  libappindicator-gtk3-devel
)

echo ">> downloading RPM closure for: ${PKGS[*]}"
( cd "$WORK" && dnf download --resolve --alldeps "${PKGS[@]}" )

echo ">> unpacking into $SYSROOT"
chmod -R u+rwX "$SYSROOT" 2>/dev/null || true
rm -rf "$SYSROOT"
mkdir -p "$SYSROOT"
umask 022
shopt -s nullglob
for rpm in "$WORK"/*.x86_64.rpm "$WORK"/*.noarch.rpm; do
  case "$(basename "$rpm")" in
    filesystem-*|basesystem-*|setup-*|man-db-*|man-pages-*|crypto-policies-*) continue ;;
  esac
  ( rpm2cpio "$rpm" | ( cd "$SYSROOT" && cpio -idmu --no-preserve-owner --quiet ) ) 2>/dev/null || true
done
chmod -R u+rwX "$SYSROOT"

# libappindicator ships only appindicator3-0.1.pc on some releases; Tauri's
# libappindicator-sys also accepts the ayatana variant. Nothing to do if absent.
if [ ! -f "$SYSROOT/usr/lib64/pkgconfig/webkit2gtk-4.1.pc" ]; then
  echo "!! webkit2gtk-4.1.pc missing after unpack -- the download step likely failed" >&2
  exit 1
fi

echo ">> sysroot ready. Now: source $(dirname "$0")/build-env.sh"
