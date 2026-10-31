#!/bin/bash
# Loads the Link service and link-session as launchd agents of the current user, from the installed app.
set -euo pipefail
APP=${1:-/Applications/Andromeda Link.app}
AGENTS=$HOME/Library/LaunchAgents
LOGS=$HOME/Library/Logs/Andromeda\ Link
mkdir -p "$AGENTS" "$LOGS"
for label in org.andromeda.linkd org.andromeda.link-session; do
  launchctl bootout "gui/$(id -u)/$label" 2> /dev/null || true
  # The agents name the app's binaries; point them at wherever the app was put.
  sed -e "s|/Applications/Andromeda Link.app|$APP|" -e "s|@LOGS@|$LOGS|" \
    "$APP/Contents/Resources/LaunchAgents/$label.plist" > "$AGENTS/$label.plist"
  launchctl bootstrap "gui/$(id -u)" "$AGENTS/$label.plist"
done
