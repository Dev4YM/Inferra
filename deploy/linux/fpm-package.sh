#!/usr/bin/env bash
# Build .deb and .rpm using fpm (https://github.com/jordansissel/fpm).
# Prereqs: Ruby + `gem install fpm`, Python 3.11+ on PATH as python3.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
STAGE="$(mktemp -d)"
trap 'rm -rf "${STAGE}"' EXIT

cd "${ROOT}"
VERSION="$(python3 -c "import pathlib, tomllib; print(tomllib.loads(pathlib.Path('pyproject.toml').read_text(encoding='utf-8'))['project']['version'])")"

python3 -m venv "${STAGE}/opt/inferra/venv"
"${STAGE}/opt/inferra/venv/bin/pip" install --upgrade pip
"${STAGE}/opt/inferra/venv/bin/pip" install "${ROOT}[kubernetes]"

mkdir -p "${STAGE}/usr/bin"
cat >"${STAGE}/usr/bin/inferra" <<'EOF'
#!/bin/sh
exec /opt/inferra/venv/bin/inferra "$@"
EOF
chmod 0755 "${STAGE}/usr/bin/inferra"

mkdir -p "${STAGE}/lib/systemd/system"
cp "${ROOT}/deploy/systemd/inferra.service" "${STAGE}/lib/systemd/system/inferra.service"

mkdir -p "${STAGE}/etc/inferra"
cat >"${STAGE}/etc/inferra/inferra.toml" <<'EOF'
[server]
host = "0.0.0.0"
port = 7433
cors_origins = ["*"]
auth_token_env = ""
require_loopback = false

[storage]
data_dir = "/var/lib/inferra"
events_db = "events.db"
incidents_db = "incidents.db"
retention_hours = 72

[ai]
enabled = false
provider = "ollama"
base_url = "http://127.0.0.1:11434"
model = "gemma4:e4b"
EOF

AFTER="${ROOT}/deploy/linux/deb-after-install.sh"
fpm -s dir -t deb -n inferra -v "${VERSION}" \
  --license "Apache-2.0" \
  --vendor "Inferra" \
  --description "Local-first runtime failure explanation service" \
  --depends "python3 (>= 3.11)" \
  --after-install "${AFTER}" \
  -C "${STAGE}" \
  opt/inferra \
  usr/bin \
  lib/systemd/system \
  etc/inferra

fpm -s dir -t rpm -n inferra -v "${VERSION}" \
  --license "Apache-2.0" \
  --vendor "Inferra" \
  --description "Local-first runtime failure explanation service" \
  --depends "python3 >= 3.11" \
  -C "${STAGE}" \
  opt/inferra \
  usr/bin \
  lib/systemd/system \
  etc/inferra
