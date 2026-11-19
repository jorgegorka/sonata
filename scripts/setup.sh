#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
runtime="$(sh scripts/fetch-herdr.sh)"
mkdir -p .tools/herdr
if [ "$runtime" != "$PWD/.tools/herdr/herdr" ]; then ln -sf "$runtime" .tools/herdr/herdr; fi
npm --prefix app ci --engine-strict
git config --local core.hooksPath .githooks
echo "Dependencies ready. Build: cd app && npm run bundle:unsigned"
echo "Commits require gitleaks on PATH; run scripts/check-secrets.sh before sharing."
