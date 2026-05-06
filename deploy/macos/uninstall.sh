#!/bin/sh
set -e
sudo launchctl bootout system/com.inferra.agent 2>/dev/null || true
sudo launchctl unload /Library/LaunchDaemons/com.inferra.agent.plist 2>/dev/null || true
sudo rm -f /Library/LaunchDaemons/com.inferra.agent.plist
echo "Removed com.inferra.agent launch daemon."
