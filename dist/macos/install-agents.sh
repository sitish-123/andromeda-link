#!/bin/bash
# Loads the Link service and link-session as launchd agents of the current user, from the installed app.
set -euo pipefail
APP=${1:-/Applications/Andromeda Link.app}
AGENTS=$HOME/Library/LaunchAgents
mkdir -p "$AGENTS"
for label in org.andromeda.linkd org.andromeda.link-session; do
  launchctl bootout "gui/$(id -u)/$label" 2> /dev/null || true
  cp "$APP/Contents/Resources/LaunchAgents/$label.plist" "$AGENTS/"
  launchctl bootstrap "gui/$(id -u)" "$AGENTS/$label.plist"
done
