#!/usr/bin/env bash
# The Qt desktop app starts a stopped andromeda-linkd and the phone reconnects. Private dbus-daemon with a service
# file for org.andromeda.Link1 (stand-in for the systemd user unit that the installed D-Bus activation file names:
# the app calls StartServiceByName, so the bus activates the private andromeda-linkd). The emulator pairs, the
# script kills andromeda-linkd, the app (`--service-test`) shows serviceRunning false, calls Link.startService(),
# and the phone is connected again within 60 s. Artifacts in $OUT. Run under flock /tmp/link-emulator.lock.
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
OUT=${OUT:-$(pwd)/artifacts/link-desk-service}
BIN=${LINK_BIN:-$ROOT/target/debug}
ADB=${ADB:-$HOME/Android/Sdk/platform-tools/adb}
MAESTRO=${MAESTRO:-$HOME/.maestro/bin/maestro}
SERIAL=${ANDROID_SERIAL:-emulator-5554}
APK=${APK:-$ROOT/apps/android/app/build/outputs/apk/debug/app-debug.apk}
FLOWS=$ROOT/apps/android/maestro
PACKAGE=org.andromeda.link

rm -rf "$OUT"
mkdir -p "$OUT"
RUNTIME=$(mktemp -d /tmp/link-desk-service.XXXX)
# The bus starts andromeda-linkd, so it is no job of this script: stop it before the bus.
trap 'kill $(linkd_pid 2>/dev/null) $(jobs -p) 2>/dev/null || true; wait 2>/dev/null; rm -rf "$RUNTIME"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
wait_for() {
  local tries=$(( $1 * 10 )) what=$2; shift 2
  for _ in $(seq "$tries"); do "$@" && return; sleep 0.1; done
  fail "$what"
}
adb() { "$ADB" -s "$SERIAL" "$@"; }

[[ $(adb get-state 2>/dev/null) == device ]] || fail "no emulator at $SERIAL"
[[ -s $APK ]] || fail "no APK at $APK"
[[ -x $BIN/andromeda-link && -x $BIN/andromeda-linkd ]] || fail "no binaries in $BIN; cargo build -p link-desk"

mkdir -p "$RUNTIME/services"
cat > "$RUNTIME/services/org.andromeda.Link1.service" <<CONF
[D-BUS Service]
Name=org.andromeda.Link1
Exec=$BIN/andromeda-linkd
CONF
cat > "$RUNTIME/bus.conf" <<CONF
<busconfig>
  <type>session</type>
  <listen>unix:path=$RUNTIME/bus</listen>
  <auth>EXTERNAL</auth>
  <servicedir>$RUNTIME/services</servicedir>
  <policy context="default">
    <allow user="*"/>
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
CONF
# The bus passes its environment to the service it activates.
XDG_STATE_HOME=$RUNTIME/desktop dbus-daemon --config-file="$RUNTIME/bus.conf" --nofork 2> "$OUT/dbus.log" &
for _ in $(seq 50); do [[ -S $RUNTIME/bus ]] && break; sleep 0.02; done
export DBUS_SESSION_BUS_ADDRESS=unix:path=$RUNTIME/bus DBUS_SYSTEM_BUS_ADDRESS=unix:path=$RUNTIME/bus
link() { gdbus call --session -d org.andromeda.Link1 -o /org/andromeda/Link1 -m "org.andromeda.Link1.$1" "${@:2}"; }
devices() { gdbus call --session -d org.andromeda.Link1 -o /org/andromeda/Link1 -m org.freedesktop.DBus.Properties.Get org.andromeda.Link1 Devices; }
connected() { devices | grep -q "', true)"; }
said() { grep -q "SVC $1" "$OUT/app.log"; }
owned() { gdbus call --session -d org.freedesktop.DBus -o /org/freedesktop/DBus -m org.freedesktop.DBus.NameHasOwner org.andromeda.Link1 | grep -q true; }
linkd_pid() {
  gdbus call --session -d org.freedesktop.DBus -o /org/freedesktop/DBus \
    -m org.freedesktop.DBus.GetConnectionUnixProcessID org.andromeda.Link1 | sed -E 's/^\(uint32 ([0-9]+),\)$/\1/'
}

# Pairing needs the service: start it by activation from the script, as the first run.
link CancelPairing > /dev/null 2>&1 || true
wait_for 10 "andromeda-linkd was not activated" owned

adb shell cmd statusbar collapse
adb shell input keyevent KEYCODE_HOME
adb uninstall "$PACKAGE" > /dev/null 2>&1 || true
adb install "$APK" > /dev/null || fail "installing $APK"
adb shell pm grant "$PACKAGE" android.permission.CAMERA
URI=$(link StartPairing | sed -E "s/^\('[0-9]+', '([^']+)'\)$/\1/")
adb shell am start -W -a android.intent.action.VIEW -d "'$URI'" "$PACKAGE" > /dev/null
"$MAESTRO" --device "$SERIAL" test "$FLOWS/pair.yaml" > "$OUT/maestro.log" 2>&1 || fail "maestro pair.yaml"
wait_for 20 "D-Bus does not show the emulator connected: $(devices)" connected

QT_QPA_PLATFORM=offscreen QT_FORCE_STDERR_LOGGING=1 "$BIN/andromeda-link" --service-test > "$OUT/app.log" 2>&1 &
APP=$!
wait_for 30 "the app did not show the connected phone: $(tail -3 "$OUT/app.log")" said "up "

PID=$(linkd_pid) || fail "no andromeda-linkd process"
echo "stopping andromeda-linkd $PID"
kill "$PID"
wait_for 30 "the app did not show the service stopped: $(tail -3 "$OUT/app.log")" said "down serviceRunning false"
wait_for 60 "the phone did not reconnect after startService(): $(tail -3 "$OUT/app.log")" said "back "
NEW=$(linkd_pid) || fail "no restarted andromeda-linkd"
[[ $NEW != "$PID" ]] || fail "the same andromeda-linkd pid $PID"
wait "$APP" || fail "the app exited with $?"
grep "SVC" "$OUT/app.log"
echo "PASS: pid $PID -> $NEW; artifacts: $OUT"
