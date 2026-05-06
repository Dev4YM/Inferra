#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
exec npx --yes tailwindcss@3.4.17 \
  -i ./src/web/static/tailwind-input.css \
  -o ./src/web/static/tailwind.css \
  --minify
