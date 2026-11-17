#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
if ! command -v gitleaks >/dev/null 2>&1; then
  echo "Secret check requires gitleaks on PATH; install it before committing." >&2
  exit 1
fi
case "${1:-}" in
  --staged) exec python3 scripts/scan-source.py --staged ;;
  "") exec python3 scripts/scan-source.py ;;
  *) echo "Usage: scripts/check-secrets.sh [--staged]" >&2; exit 2 ;;
esac
