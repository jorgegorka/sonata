#!/bin/sh
set -eu
root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
. "$root/scripts/herdr-runtime.sh"
triple="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
herdr_target "$triple"
runtime="${SONATA_HERDR_BIN:-$root/.tools/herdr/$HERDR_VERSION/$triple/herdr}"
if [ ! -f "$runtime" ]; then
  [ -z "${SONATA_HERDR_BIN:-}" ] || { echo "SONATA_HERDR_BIN does not exist: $runtime" >&2; exit 1; }
  mkdir -p "$(dirname "$runtime")"
  download="$(mktemp "$(dirname "$runtime")/download.XXXXXX")"
  trap 'rm -f "$download"' EXIT HUP INT TERM
  curl --proto '=https' --tlsv1.2 -fSL --retry 3 --connect-timeout 15 --max-time 180 \
    "https://github.com/herdrdev/herdr/releases/download/v$HERDR_VERSION/$HERDR_ASSET" -o "$download"
  [ "$(sha256_file "$download")" = "$HERDR_SHA256" ] || { echo "Runtime checksum mismatch." >&2; exit 1; }
  chmod 755 "$download"
  mv "$download" "$runtime"
  trap - EXIT HUP INT TERM
fi
[ "$(sha256_file "$runtime")" = "$HERDR_SHA256" ] || { echo "Runtime checksum mismatch: $runtime" >&2; exit 1; }
runtime="$(cd "$(dirname "$runtime")" && pwd)/$(basename "$runtime")"
printf '%s\n' "$runtime"
