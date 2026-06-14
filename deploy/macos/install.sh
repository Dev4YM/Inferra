#!/bin/sh
set -e

FULL=0
SKIP_BUILD=0

usage() {
  cat <<'EOF'
Usage: sudo ./deploy/macos/install.sh [--full] [--skip-build]

  --full        Clean npm ci + release cargo build before install
  --skip-build  Reuse existing src/target/release/inferra and src/web/ui_dist
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --full) FULL=1 ;;
    --skip-build) SKIP_BUILD=1 ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
export PATH="/usr/local/bin:/opt/homebrew/bin:$PATH"

INFERRA_BUILD="${ROOT}/src/target/release/inferra"
UI_DIST="${ROOT}/src/web/ui_dist"
DEFAULTS_TOML="${ROOT}/src/config/defaults.toml"
INSTALL_LIB="/usr/local/lib/inferra"
INSTALL_BIN="/usr/local/bin/inferra"
CONFIG_DIR="/usr/local/etc/inferra"
DATA_DIR="/usr/local/var/inferra"
LOG_DIR="/usr/local/var/log"

if [ "$SKIP_BUILD" -eq 0 ]; then
  if [ "$FULL" -eq 1 ]; then
    (cd "${ROOT}/src/web/frontend" && npm ci && npm run build)
    cargo build --manifest-path "${ROOT}/src/Cargo.toml" -p inferra-cli --release
  else
    if [ ! -d "${UI_DIST}" ]; then
      (cd "${ROOT}/src/web/frontend" && npm run build)
    fi
    if [ ! -x "${INFERRA_BUILD}" ]; then
      cargo build --manifest-path "${ROOT}/src/Cargo.toml" -p inferra-cli --release
    fi
  fi
fi

if [ ! -x "${INFERRA_BUILD}" ]; then
  echo "Build the Rust CLI first (expected ${INFERRA_BUILD})." >&2
  exit 1
fi
if [ ! -d "${UI_DIST}" ]; then
  echo "UI bundle missing at ${UI_DIST}. Run npm run build in src/web/frontend." >&2
  exit 1
fi

sudo mkdir -p "${INSTALL_LIB}" "${INSTALL_LIB}/runtime-assets/ui_dist" "${INSTALL_LIB}/share" \
  "${CONFIG_DIR}" "${DATA_DIR}" "${LOG_DIR}" /usr/local/bin

sudo cp "${INFERRA_BUILD}" "${INSTALL_LIB}/inferra"
sudo chmod 0755 "${INSTALL_LIB}/inferra"
sudo rm -rf "${INSTALL_LIB}/runtime-assets/ui_dist"
sudo cp -R "${UI_DIST}/." "${INSTALL_LIB}/runtime-assets/ui_dist/"
if [ -f "${DEFAULTS_TOML}" ]; then
  sudo cp "${DEFAULTS_TOML}" "${INSTALL_LIB}/runtime-assets/defaults.toml"
fi
"${INFERRA_BUILD}" --version | sudo tee "${INSTALL_LIB}/share/version.txt" >/dev/null
sudo ln -sf "${INSTALL_LIB}/inferra" "${INSTALL_BIN}"

if ! printf '%s' "$PATH" | tr ':' '\n' | grep -qx /usr/local/bin; then
  echo "Warning: /usr/local/bin is not on PATH in this shell. Add it to your shell profile." >&2
fi
if command -v inferra >/dev/null 2>&1; then
  echo "PATH resolves inferra -> $(command -v inferra)"
else
  echo "inferra is not on PATH in this shell yet."
fi

if [ ! -f "${CONFIG_DIR}/inferra.toml" ]; then
  sudo "${INSTALL_LIB}/inferra" --config "${CONFIG_DIR}/inferra.toml" setup --yes --skip-connection-test --data-dir "${DATA_DIR}"
fi
sudo "${INSTALL_LIB}/inferra" --config "${CONFIG_DIR}/inferra.toml" init-db

sudo cp "${ROOT}/deploy/macos/com.inferra.agent.plist" /Library/LaunchDaemons/com.inferra.agent.plist
sudo chown root:wheel /Library/LaunchDaemons/com.inferra.agent.plist
sudo chmod 0644 /Library/LaunchDaemons/com.inferra.agent.plist
sudo launchctl bootout system/com.inferra.agent 2>/dev/null || true
sudo launchctl bootstrap system /Library/LaunchDaemons/com.inferra.agent.plist 2>/dev/null \
  || sudo launchctl load -w /Library/LaunchDaemons/com.inferra.agent.plist

echo "Installed Inferra to ${INSTALL_LIB}"
echo "  CLI: ${INSTALL_BIN}"
echo "  Web: ${INSTALL_LIB}/runtime-assets/ui_dist"
echo "  Config: ${CONFIG_DIR}/inferra.toml"
echo "  Data: ${DATA_DIR}"
