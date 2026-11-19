#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
if [ "${1:-}" = --unsigned ]; then
  shift
  unset APPLE_SIGNING_IDENTITY APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD
  unset APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_PATH TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD
fi
case "$(uname -s)" in
  Darwin) config=src-tauri/tauri.bundle.json ;;
  Linux) config=src-tauri/tauri.linux.bundle.json ;;
  *) echo "Sonata packages support macOS and Linux." >&2; exit 1 ;;
esac
exec npm exec -- tauri build --config "$config" "$@"
