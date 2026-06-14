#!/bin/sh
set -e

FULL=0
KEEP_DATA=0

usage() {
  cat <<'EOF'
Usage: sudo ./deploy/macos/uninstall.sh [--full] [--keep-data]

  --full       Also remove /usr/local/etc/inferra and /usr/local/var/inferra
  --keep-data  With --full, keep config/data directories
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --full) FULL=1 ;;
    --keep-data) KEEP_DATA=1 ;;
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

sudo launchctl bootout system/com.inferra.agent 2>/dev/null || true
sudo launchctl unload /Library/LaunchDaemons/com.inferra.agent.plist 2>/dev/null || true
sudo rm -f /Library/LaunchDaemons/com.inferra.agent.plist
sudo rm -f /usr/local/bin/inferra
sudo rm -rf /usr/local/lib/inferra

if [ "$FULL" -eq 1 ] && [ "$KEEP_DATA" -eq 0 ]; then
  sudo rm -rf /usr/local/etc/inferra /usr/local/var/inferra
  echo "Removed launch daemon, binaries, and program data."
else
  echo "Removed launch daemon and binaries. Kept /usr/local/etc/inferra and /usr/local/var/inferra."
fi

if command -v inferra >/dev/null 2>&1; then
  echo "Warning: inferra is still on PATH -> $(command -v inferra)" >&2
else
  echo "inferra is no longer on PATH in this shell."
fi
