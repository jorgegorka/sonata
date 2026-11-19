#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
root="$(cd .. && pwd)"
triple="${TAURI_ENV_TARGET_TRIPLE:-${CARGO_BUILD_TARGET:-$(rustc -vV | sed -n 's/^host: //p')}}"
herdr="$(sh "$root/scripts/fetch-herdr.sh" "$triple")"
cargo build --locked --release --target "$triple" -p sbot --manifest-path "$root/Cargo.toml"
mkdir -p src-tauri/binaries src-tauri/runtime
cp "$root/target/$triple/release/sbot" "src-tauri/binaries/sbot-$triple"
cp "$herdr" "src-tauri/binaries/herdr-$triple"
case "$triple" in
  *-linux-*)
    cp "$herdr" src-tauri/runtime/herdr
    cp "$root/target/$triple/release/sbot" src-tauri/runtime/sbot
    chmod 755 src-tauri/runtime/herdr src-tauri/runtime/sbot ;;
esac
chmod 755 "src-tauri/binaries/sbot-$triple" "src-tauri/binaries/herdr-$triple"
echo "prepare-sidecars: verified runtime and sbot ready for $triple"
