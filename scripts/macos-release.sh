#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
for name in APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY APPLE_API_KEY APPLE_API_ISSUER APPLE_API_PRIVATE_KEY; do
  [ -n "${!name:-}" ] || { echo "Missing release secret: $name" >&2; exit 1; }
done
case "$APPLE_SIGNING_IDENTITY" in 'Developer ID Application:'*) ;; *) echo "A Developer ID Application certificate is required." >&2; exit 1 ;; esac
release_temp="$(mktemp -d)"
export SONATA_RELEASE_KEYCHAIN="$release_temp/signing.keychain-db"
export SONATA_KEYCHAIN_LIST="$release_temp/keychains.json"
cleanup() {
  python3 - <<'PY'
import json, os, pathlib, subprocess
p = pathlib.Path(os.environ['SONATA_KEYCHAIN_LIST'])
if p.exists(): subprocess.run(['security', 'list-keychains', '-d', 'user', '-s', *json.loads(p.read_text())], check=False)
PY
  security delete-keychain "$SONATA_RELEASE_KEYCHAIN" >/dev/null 2>&1 || true
  rm -rf "$release_temp"
}
trap cleanup EXIT
python3 - <<'PY'
import base64, json, os, pathlib, shlex, subprocess
p = pathlib.Path(os.environ['SONATA_KEYCHAIN_LIST'])
p.write_text(json.dumps(shlex.split(subprocess.check_output(['security','list-keychains','-d','user'], text=True))))
p.with_name('certificate.p12').write_bytes(base64.b64decode(os.environ['APPLE_CERTIFICATE'], validate=True))
p.with_name('AuthKey.p8').write_text(os.environ['APPLE_API_PRIVATE_KEY'])
PY
chmod 600 "$release_temp/certificate.p12" "$release_temp/AuthKey.p8"
sonata_keychain_password="$(openssl rand -hex 32)"
security create-keychain -p "$sonata_keychain_password" "$SONATA_RELEASE_KEYCHAIN"
security unlock-keychain -p "$sonata_keychain_password" "$SONATA_RELEASE_KEYCHAIN"
security set-keychain-settings -lut 21600 "$SONATA_RELEASE_KEYCHAIN"
security import "$release_temp/certificate.p12" -k "$SONATA_RELEASE_KEYCHAIN" -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign -T /usr/bin/security
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$sonata_keychain_password" "$SONATA_RELEASE_KEYCHAIN" >/dev/null
python3 - <<'PY'
import json, os, pathlib, subprocess
original = json.loads(pathlib.Path(os.environ['SONATA_KEYCHAIN_LIST']).read_text())
subprocess.run(['security','list-keychains','-d','user','-s',os.environ['SONATA_RELEASE_KEYCHAIN'],*original], check=True)
PY
export APPLE_API_KEY_PATH="$release_temp/AuthKey.p8"
export MACOSX_DEPLOYMENT_TARGET=11.0
npm --prefix app run bundle
bundle=target/release/bundle/macos/Sonata.app
for binary in sonata sbot herdr; do
  file "$bundle/Contents/MacOS/$binary" | grep -q 'arm64' || { echo "Wrong architecture: $binary" >&2; exit 1; }
  codesign --verify --strict "$bundle/Contents/MacOS/$binary"
done
codesign --verify --deep --strict "$bundle"
xcrun stapler validate "$bundle"
spctl --assess --type execute "$bundle"
npm --prefix app run dmg
version="$(python3 scripts/release.py check-version)"
xcrun stapler validate "target/release/bundle/dmg/Sonata_$version.dmg"
python3 scripts/release.py collect --platform macos-aarch64

python3 scripts/updates.py archive-mac
