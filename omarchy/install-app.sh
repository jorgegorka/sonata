#!/bin/bash
# Build Sonata and install it with pacman, the same package install Omarchy
# uses for its own applications. The build runs in a temporary copy so the
# plugin checkout stays clean for `omarchy plugin update`.
set -euo pipefail

fail() { printf 'Sonata: %s\n' "$*" >&2; exit 1; }

[ "$(uname -s)" = Linux ] || fail "This installer is for Omarchy on Linux."
[ "$(id -u)" -ne 0 ] || fail "Run this as your user. It will ask for sudo when it installs the package."

root="$(cd "$(dirname "$0")/.." && pwd)"
[ -f "$root/pkg/PKGBUILD" ] || fail "The Sonata package recipe is missing."

if pacman -Q sonata >/dev/null 2>&1; then
  printf 'Sonata is already installed.\n'
  exit 0
fi

exec 9>"/tmp/sonata-omarchy-install.lock"
flock -n 9 || fail "Sonata is already being installed."

printf 'Installing Sonata...\n'
command -v omarchy-pkg-add >/dev/null || fail "omarchy-pkg-add is missing."
omarchy-pkg-add \
  binutils fakeroot gcc make patch rust nodejs npm webkit2gtk-4.1 gtk3 \
  openssl libayatana-appindicator librsvg xdotool patchelf fuse2 pkgconf \
  squashfs-tools curl git

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/src" "$stage/pkg"
tar -C "$root" \
  --exclude .git \
  --exclude target \
  --exclude node_modules \
  --exclude app/node_modules \
  --exclude app/dist \
  -cf - . | tar -C "$stage/src" -xf -

export SONATA_SRC="$stage/src"
export PKGDEST="$stage/pkg"
export SRCDEST="$stage/srcdest"
export BUILDDIR="$stage/build"
mkdir -p "$SRCDEST" "$BUILDDIR"
cp "$stage/src/pkg/PKGBUILD" "$stage/pkg/PKGBUILD"
(
  cd "$stage/pkg"
  makepkg -f --noconfirm --nocheck
)
package="$(echo "$PKGDEST"/sonata-*.pkg.tar.zst)"
[ -f "$package" ] || fail "makepkg did not produce a package."
sudo pacman -U --noconfirm "$package"
printf 'Sonata is installed. Open it from the application launcher.\n'
