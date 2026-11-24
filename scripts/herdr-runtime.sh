#!/bin/sh
# Shared by setup and packaging. Keep the runtime version and checksums together.
herdr_target() {
  HERDR_VERSION=0.9.5
  case "$1" in
    aarch64-apple-darwin)
      HERDR_ASSET=herdr-macos-aarch64
      HERDR_SHA256=fb77c2232a05a8035421382c894392f4174e4e38a18e48af3dd0290157c3a7e2 ;;
    x86_64-unknown-linux-gnu)
      HERDR_ASSET=herdr-linux-x86_64
      HERDR_SHA256=14d9e8bc8b4a9e0118d77c32266f76d461f09b40342eb23752960bd89f59929d ;;
    aarch64-unknown-linux-gnu)
      HERDR_ASSET=herdr-linux-aarch64
      HERDR_SHA256=17d7536f2da50859523680ef279dffcac8e4a235f4e46fe8da6c2ec44141897c ;;
    *) echo "Unsupported Sonata target: $1" >&2; return 1 ;;
  esac
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d ' ' -f 1
  else shasum -a 256 "$1" | cut -d ' ' -f 1; fi
}
