#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
root="$(cd .. && pwd)"
triple="$(rustc -vV | sed -n 's/^host: //p')"
herdr="${SONATA_HERDR_BIN:-$root/.tools/herdr/herdr}"
want="herdr 0.9.3"
got="$("$herdr" --version 2>/dev/null || true)"
if [ "$got" != "$want" ]; then
  echo "prepare-sidecars: version check failed: $herdr says \"$got\"; the app bundles \"$want\" (pinned runtime)" >&2
  exit 1
fi
want_sha="5173a3e0ae42d5d1ab7ebfa5d5e6329f7c3d23f8e1a3677c7ce3231da2884157"
got_sha="$(shasum -a 256 "$herdr" | cut -d ' ' -f 1)"
if [ "$got_sha" != "$want_sha" ]; then
  echo "prepare-sidecars: SHA-256 check failed: $herdr is $got_sha; the app bundles $want_sha (pinned runtime)" >&2
  exit 1
fi
cargo build --release -p sbot --manifest-path "$root/Cargo.toml"
mkdir -p src-tauri/binaries
cp "$root/target/release/sbot" "src-tauri/binaries/sbot-$triple"
cp "$herdr" "src-tauri/binaries/herdr-$triple"
chmod 755 "src-tauri/binaries/sbot-$triple" "src-tauri/binaries/herdr-$triple"
echo "prepare-sidecars: sbot and $want ready for $triple"
