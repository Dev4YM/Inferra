#!/usr/bin/env bash
# Build .deb and .rpm using fpm (https://github.com/jordansissel/fpm).
# Prereqs: Ruby + `gem install fpm`, Rust toolchain with cargo.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
STAGE="$(mktemp -d)"
trap 'rm -rf "${STAGE}"' EXIT

cd "${ROOT}"
VERSION="$(awk -F '"' '
  /^\[workspace\.package\]/ { in_workspace = 1; next }
  in_workspace && /^version = "/ { print $2; exit }
' "${ROOT}/src/Cargo.toml")"
if [ -z "${VERSION}" ]; then
  echo "Could not determine version from ${ROOT}/src/Cargo.toml" >&2
  exit 1
fi

cargo build --manifest-path "${ROOT}/src/Cargo.toml" -p inferra-cli --release

mkdir -p "${STAGE}/usr/bin"
cat >"${STAGE}/usr/bin/inferra" <<'EOF'
#!/bin/sh
exec /opt/inferra/inferra "$@"
EOF
chmod 0755 "${STAGE}/usr/bin/inferra"

mkdir -p "${STAGE}/opt/inferra/runtime-assets"
cp "${ROOT}/src/target/release/inferra" "${STAGE}/opt/inferra/inferra"
chmod 0755 "${STAGE}/opt/inferra/inferra"
cp -R "${ROOT}/src" "${STAGE}/opt/inferra/runtime-assets/src"
cp -R "${ROOT}/src/web/ui_dist" "${STAGE}/opt/inferra/runtime-assets/ui_dist"

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
  -C "${STAGE}" \
  opt/inferra \
  usr/bin \
  lib/systemd/system \
  etc/inferra
