#!/bin/sh
# Shared by setup and packaging. Keep the runtime version and checksums together.
herdr_target() {
  HERDR_VERSION=0.9.3
  case "$1" in
    aarch64-apple-darwin)
      HERDR_ASSET=herdr-macos-aarch64
      HERDR_SHA256=5173a3e0ae42d5d1ab7ebfa5d5e6329f7c3d23f8e1a3677c7ce3231da2884157 ;;
    x86_64-unknown-linux-gnu)
      HERDR_ASSET=herdr-linux-x86_64
      HERDR_SHA256=18a8dc65f1c2fa485884344356dea1cfd911c6f06cf46fa78e193f4087f4dba7 ;;
    aarch64-unknown-linux-gnu)
      HERDR_ASSET=herdr-linux-aarch64
      HERDR_SHA256=4de7aa3e25678812e92960de64f7c2aaa1bca1f0f80a3c5e559837e231e1f5c0 ;;
    *) echo "Unsupported Sonata target: $1" >&2; return 1 ;;
  esac
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d ' ' -f 1
  else shasum -a 256 "$1" | cut -d ' ' -f 1; fi
}
