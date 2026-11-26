#!/bin/bash
# Run inside scripts/linux.Dockerfile, from the repository root.
set -euo pipefail
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
# Live shell-guard probes require Bash; herdr otherwise falls back to /bin/sh in Docker.
export SHELL=/bin/bash
python3 scripts/release.py check-version
npm --prefix app ci --engine-strict
python3 -m unittest discover -s scripts/tests -v
npm --prefix app test
runtime="$(sh scripts/fetch-herdr.sh)"
SONATA_HERDR_BIN="$runtime" cargo test --locked --workspace
if [ "${1:-}" = --package ]; then
  npm --prefix app run bundle:unsigned
  arch="$(uname -m)"
  python3 scripts/linux-smoke.py --bundle target/release/bundle
  python3 scripts/release.py collect --platform "linux-$arch"
fi
