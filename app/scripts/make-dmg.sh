#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
root="$(cd .. && pwd)"
name="$(node -p "require('./src-tauri/tauri.conf.json').productName")"
version="$(node -p "require('./src-tauri/tauri.conf.json').version")"
app="$root/target/release/bundle/macos/$name.app"
if [ ! -d "$app" ]; then
  echo "make-dmg: $app is missing; run npm run bundle first" >&2
  exit 1
fi
built="$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$app/Contents/Info.plist")"
if [ "$built" != "$version" ]; then
  echo "make-dmg: $app is version $built but tauri.conf.json says $version; run npm run bundle first" >&2
  exit 1
fi
identity="${APPLE_SIGNING_IDENTITY:-}"
if [ -n "$identity" ] && ! codesign -dv "$app" 2>&1 | grep -q '^TeamIdentifier=[A-Z0-9]*$'; then
  echo "make-dmg: $app has no Developer ID signature; build it with npm run bundle, or use npm run dmg:unsigned" >&2
  exit 1
fi

out="$root/target/release/bundle/dmg"
dmg="$out/$(echo "$name" | tr ' ' _)_$version.dmg"
stage="$(mktemp -d)"
trap 'hdiutil detach "$stage/mount" -quiet 2>/dev/null || true; rm -rf "$stage"' EXIT
mkdir "$stage/files" "$stage/mount"
ditto "$app" "$stage/files/$name.app"
ln -s /Applications "$stage/files/Applications"
swift scripts/volume-icon.swift app-icon.png "$stage/volume.iconset"
iconutil -c icns -o "$stage/files/.VolumeIcon.icns" "$stage/volume.iconset"
mkdir -p "$out"
hdiutil create -volname "$name" -srcfolder "$stage/files" -format UDRW -fs HFS+ "$stage/rw.dmg" >/dev/null
hdiutil attach -nobrowse -noautoopen -mountpoint "$stage/mount" "$stage/rw.dmg" >/dev/null
SetFile -a C "$stage/mount"
hdiutil detach "$stage/mount" >/dev/null
hdiutil convert "$stage/rw.dmg" -format UDZO -ov -o "$dmg" >/dev/null

if [ -z "$identity" ]; then
  echo "make-dmg: $dmg (unsigned)"
  exit 0
fi
codesign --sign "$identity" --timestamp "$dmg"
if [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
  set -- --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID"
elif [ -n "${APPLE_API_KEY:-}" ] && [ -n "${APPLE_API_ISSUER:-}" ] && [ -n "${APPLE_API_KEY_PATH:-}" ]; then
  set -- --key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER"
else
  echo "make-dmg: $dmg (signed, not notarized)"
  exit 0
fi
result="$(xcrun notarytool submit "$dmg" "$@" --wait 2>&1)" || true
if ! echo "$result" | grep -q 'status: Accepted'; then
  echo "$result" >&2
  echo "make-dmg: Apple did not accept $dmg; xcrun notarytool log <id> says why" >&2
  exit 1
fi
xcrun stapler staple "$dmg" >/dev/null
echo "make-dmg: $dmg (signed and notarized)"
