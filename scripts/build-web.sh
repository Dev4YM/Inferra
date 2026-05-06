#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT/src/web/frontend"
npm ci
npm run build
echo "Built web UI to src/web/ui_dist"
