#!/bin/sh
set -e
sudo launchctl bootout system/com.inferra.agent 2>/dev/null || true
sudo launchctl unload /Library/LaunchDaemons/com.inferra.agent.plist 2>/dev/null || true
sudo rm -f /Library/LaunchDaemons/com.inferra.agent.plist
sudo rm -f /usr/local/bin/inferra
sudo rm -rf /usr/local/lib/inferra
echo "Removed com.inferra.agent launch daemon."
