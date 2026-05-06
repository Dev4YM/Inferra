#!/bin/sh
set -e
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
export PATH="/usr/local/bin:/opt/homebrew/bin:$PATH"

if ! command -v inferra >/dev/null 2>&1; then
  echo "Install inferra on PATH first (for example: python3 -m pip install .)." >&2
  exit 1
fi

INFERRA_BIN="$(command -v inferra)"
sudo mkdir -p /usr/local/bin /usr/local/etc/inferra /usr/local/var/log
if [ "$INFERRA_BIN" != "/usr/local/bin/inferra" ]; then
  sudo ln -sf "$INFERRA_BIN" /usr/local/bin/inferra
  echo "Symlinked /usr/local/bin/inferra -> $INFERRA_BIN"
fi

if [ ! -f /usr/local/etc/inferra/inferra.toml ]; then
  inferra --config /usr/local/etc/inferra/inferra.toml setup --yes --skip-connection-test --data-dir /usr/local/var/inferra
fi

sudo cp "${ROOT}/deploy/macos/com.inferra.agent.plist" /Library/LaunchDaemons/com.inferra.agent.plist
sudo chown root:wheel /Library/LaunchDaemons/com.inferra.agent.plist
sudo chmod 0644 /Library/LaunchDaemons/com.inferra.agent.plist
sudo launchctl bootout system/com.inferra.agent 2>/dev/null || true
sudo launchctl bootstrap system /Library/LaunchDaemons/com.inferra.agent.plist 2>/dev/null \
  || sudo launchctl load -w /Library/LaunchDaemons/com.inferra.agent.plist
echo "Installed com.inferra.agent (config /usr/local/etc/inferra/inferra.toml)."
