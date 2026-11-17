#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
if [ "$(uname -s)" != Darwin ] || [ "$(uname -m)" != arm64 ]; then
  echo "The bundled runtime is pinned for Apple Silicon macOS." >&2
  exit 1
fi
runtime=".tools/herdr/herdr"
expected="5173a3e0ae42d5d1ab7ebfa5d5e6329f7c3d23f8e1a3677c7ce3231da2884157"
if [ ! -f "$runtime" ]; then
  mkdir -p .tools/herdr
  download="$(mktemp .tools/herdr/download.XXXXXX)"
  trap 'rm -f "$download"' EXIT HUP INT TERM
  curl -fsSL --connect-timeout 15 --max-time 120 \
    https://github.com/herdrdev/herdr/releases/download/v0.9.3/herdr-macos-aarch64 -o "$download"
  actual="$(shasum -a 256 "$download" | cut -d ' ' -f 1)"
  [ "$actual" = "$expected" ] || { echo "Runtime checksum mismatch." >&2; exit 1; }
  chmod 755 "$download"
  mv "$download" "$runtime"
  trap - EXIT HUP INT TERM
fi
actual="$(shasum -a 256 "$runtime" | cut -d ' ' -f 1)"
[ "$actual" = "$expected" ] || { echo "Runtime checksum mismatch." >&2; exit 1; }
npm --prefix app ci --engine-strict
git config --local core.hooksPath .githooks
echo "Dependencies ready. Build: cd app && npm run bundle:unsigned"
echo "Commits require gitleaks on PATH; run scripts/check-secrets.sh before sharing."
