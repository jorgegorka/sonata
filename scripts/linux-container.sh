#!/bin/bash
set -euo pipefail
# Build a disposable copy. Never change ownership or write build artifacts in /source.
git -c safe.directory=/source -C /source ls-files -z --cached --others --exclude-standard |
  tar -C /source --null -T - -cf - | tar -C /work -xf -
chown -R sonata:sonata /work
runuser -u sonata -- bash scripts/linux-validate.sh "$@"
if [ "${1:-}" = --package ]; then
  dpkg --install /work/target/release/bundle/deb/*.deb
  runuser -u sonata -- python3 scripts/linux-ui-smoke.py /usr/bin/sonata
fi
if [ -d /artifacts ] && [ -d /work/release-dist ]; then cp -a /work/release-dist/. /artifacts/; fi
