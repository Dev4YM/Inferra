#!/bin/sh
set -e
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
export PATH="/usr/local/bin:/opt/homebrew/bin:$PATH"

INFERRA_BUILD="${ROOT}/src/target/release/inferra"
if [ ! -x "${INFERRA_BUILD}" ]; then
  echo "Build the Rust CLI first (expected ${INFERRA_BUILD})." >&2
  echo "Run: cargo build --manifest-path src/Cargo.toml -p inferra-cli --release" >&2
  exit 1
fi

sudo mkdir -p /usr/local/bin /usr/local/etc/inferra /usr/local/var/log /usr/local/lib/inferra/runtime-assets
sudo cp "${INFERRA_BUILD}" /usr/local/lib/inferra/inferra
sudo chmod 0755 /usr/local/lib/inferra/inferra
sudo rm -rf /usr/local/lib/inferra/runtime-assets/src /usr/local/lib/inferra/runtime-assets/ui_dist
sudo cp -R "${ROOT}/src" /usr/local/lib/inferra/runtime-assets/src
sudo cp -R "${ROOT}/src/web/ui_dist" /usr/local/lib/inferra/runtime-assets/ui_dist
sudo ln -sf /usr/local/lib/inferra/inferra /usr/local/bin/inferra
echo "Installed Rust inferra runtime to /usr/local/lib/inferra"

if [ ! -f /usr/local/etc/inferra/inferra.toml ]; then
  /usr/local/lib/inferra/inferra --config /usr/local/etc/inferra/inferra.toml setup --yes --skip-connection-test --data-dir /usr/local/var/inferra
fi
/usr/local/lib/inferra/inferra --config /usr/local/etc/inferra/inferra.toml init-db

sudo cp "${ROOT}/deploy/macos/com.inferra.agent.plist" /Library/LaunchDaemons/com.inferra.agent.plist
sudo chown root:wheel /Library/LaunchDaemons/com.inferra.agent.plist
sudo chmod 0644 /Library/LaunchDaemons/com.inferra.agent.plist
sudo launchctl bootout system/com.inferra.agent 2>/dev/null || true
sudo launchctl bootstrap system /Library/LaunchDaemons/com.inferra.agent.plist 2>/dev/null \
  || sudo launchctl load -w /Library/LaunchDaemons/com.inferra.agent.plist
echo "Installed com.inferra.agent (config /usr/local/etc/inferra/inferra.toml)."
