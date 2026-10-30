#!/usr/bin/env bash
# The Qt desktop app inside a headless GNOME Shell (own dbus-daemon, own XDG_RUNTIME_DIR, no AppIndicator
# extension) against a private andromeda-linkd (own bus) and the emulator paired with it. The app's `--hold` hook
# shows Home and Settings, light and dark, and saves each window to $OUT/gnome-{home,settings}-{light,dark}.png.
# GNOME Shell refuses org.gnome.Shell.Screenshot to this script, so the shots show the app window only. The script also records whether a
# StatusNotifierWatcher (needed for a tray icon) exists. Run under flock /tmp/link-emulator.lock.
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
OUT=${OUT:-$(pwd)/artifacts/link-desk-gnome}
BIN=${LINK_BIN:-$ROOT/target/debug}
ADB=${ADB:-$HOME/Android/Sdk/platform-tools/adb}
MAESTRO=${MAESTRO:-$HOME/.maestro/bin/maestro}
SERIAL=${ANDROID_SERIAL:-emulator-5554}
APK=${APK:-$ROOT/apps/android/app/build/outputs/apk/debug/app-debug.apk}
FIXTURE_APK=${FIXTURE_APK:-$ROOT/apps/android/fixture/build/outputs/apk/debug/fixture-debug.apk}
FLOWS=$ROOT/apps/android/maestro
PACKAGE=org.andromeda.link

rm -rf "$OUT"
mkdir -p "$OUT"
RUNTIME=$(mktemp -d /tmp/link-desk-gnome.XXXX)
trap 'kill $(jobs -p) 2>/dev/null || true; wait 2>/dev/null; rm -rf "$RUNTIME"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
wait_for() {
  local tries=$(( $1 * 10 )) what=$2; shift 2
  for _ in $(seq "$tries"); do "$@" && return; sleep 0.1; done
  fail "$what"
}
adb() { "$ADB" -s "$SERIAL" "$@"; }
maestro() { "$MAESTRO" --device "$SERIAL" test -e OUT="$OUT" "$@" >> "$OUT/maestro.log" 2>&1 || fail "maestro $*"; }

[[ $(adb get-state 2>/dev/null) == device ]] || fail "no emulator at $SERIAL"
[[ -s $APK ]] || fail "no APK at $APK"
[[ -s $FIXTURE_APK ]] || fail "no fixture APK at $FIXTURE_APK"
[[ -x $BIN/andromeda-link ]] || fail "no $BIN/andromeda-link; cargo build -p link-desk"

cat > "$RUNTIME/bus.conf" <<CONF
<busconfig>
  <type>session</type>
  <listen>unix:path=$RUNTIME/bus</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow user="*"/>
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
CONF
dbus-daemon --config-file="$RUNTIME/bus.conf" --nofork 2> "$OUT/dbus.log" &
for _ in $(seq 50); do [[ -S $RUNTIME/bus ]] && break; sleep 0.02; done
export DBUS_SESSION_BUS_ADDRESS=unix:path=$RUNTIME/bus DBUS_SYSTEM_BUS_ADDRESS=unix:path=$RUNTIME/bus
link() { gdbus call --session -d org.andromeda.Link1 -o /org/andromeda/Link1 -m "org.andromeda.Link1.$1" "${@:2}"; }
prop() { gdbus call --session -d org.andromeda.Link1 -o /org/andromeda/Link1 -m org.freedesktop.DBus.Properties.Get org.andromeda.Link1 "$1"; }
devices() { prop Devices; }
connected() { devices | grep -q "', true)"; }
heard() { grep -F "org.andromeda.Link1.$1 " "$OUT/signals.txt" | grep -qF -- "$2"; }
said() { grep -q "SHOT $1" "$OUT/app.log"; }
# shot_state: the JSON the app printed with its last "SHOT state" line.
shot_state() { grep -o 'SHOT state {.*}' "$OUT/app.log" | tail -1 | cut -d' ' -f3-; }

XDG_STATE_HOME=$RUNTIME/desktop "$BIN/andromeda-linkd" > "$OUT/linkd.log" 2>&1 &
LINKD=$!
for _ in $(seq 100); do link CancelPairing > /dev/null 2>&1 && break; sleep 0.05; done
link CancelPairing > /dev/null || fail "andromeda-linkd did not come up"
gdbus monitor --session -d org.andromeda.Link1 -o /org/andromeda/Link1 > "$OUT/signals.txt" 2>&1 &
wait_for 5 "gdbus monitor did not start" test -s "$OUT/signals.txt"

# As link_android_features.sh: a fresh install with notification access, paired through the link.
adb shell cmd statusbar collapse
adb shell input keyevent KEYCODE_HOME
adb uninstall "$PACKAGE" > /dev/null 2>&1 || true
adb install "$APK" > /dev/null || fail "installing $APK"
adb shell pm grant "$PACKAGE" android.permission.CAMERA
adb shell pm revoke "$PACKAGE" android.permission.READ_LOGS
adb shell cmd notification allow_listener "$PACKAGE/$PACKAGE.notifications.MirrorService"
adb uninstall org.andromeda.link.fixture > /dev/null 2>&1 || true
adb install "$FIXTURE_APK" > /dev/null || fail "installing $FIXTURE_APK"
adb shell pm grant org.andromeda.link.fixture android.permission.POST_NOTIFICATIONS

URI=$(link StartPairing | sed -E "s/^\('[0-9]+', '([^']+)'\)$/\1/")
adb shell am start -W -a android.intent.action.VIEW -d "'$URI'" "$PACKAGE" > /dev/null
maestro "$FLOWS/pair.yaml"
wait_for 20 "D-Bus does not show the emulator connected: $(devices)" connected
ID=$(devices | grep -o "'[0-9a-f]\{32\}'" | head -1 | tr -d "'")

adb shell am start -W -n org.andromeda.link.fixture/.PlayerActivity > /dev/null
wait_for 30 "the phone's media session never reached the bus" eval \
  "gdbus call --session -d org.freedesktop.DBus -o /org/freedesktop/DBus -m org.freedesktop.DBus.ListNames | grep -q andromeda_link_$ID"

# GNOME Shell on its own bus and runtime dir; the app sees the Link bus only.
HOST_WAYLAND=${XDG_RUNTIME_DIR:?}/${WAYLAND_DISPLAY:?no host Wayland compositor}
GNOME_RT=$RUNTIME/gnome
mkdir -p "$GNOME_RT"; chmod 700 "$GNOME_RT"
GNOME_BUS=unix:path=$GNOME_RT/bus
gbus() { DBUS_SESSION_BUS_ADDRESS=$GNOME_BUS XDG_RUNTIME_DIR=$GNOME_RT "$@"; }
env XDG_RUNTIME_DIR=$GNOME_RT dbus-daemon --session --address="$GNOME_BUS" --nofork 2> "$OUT/gnome-dbus.log" &
for _ in $(seq 50); do [[ -S $GNOME_RT/bus ]] && break; sleep 0.02; done
# --devkit shows the shell in a window on the host's compositor (absolute socket path) through the host's PipeWire.
env DBUS_SESSION_BUS_ADDRESS=$GNOME_BUS XDG_RUNTIME_DIR=$GNOME_RT XDG_CURRENT_DESKTOP=GNOME XDG_SESSION_TYPE=wayland \
  WAYLAND_DISPLAY=$HOST_WAYLAND PIPEWIRE_RUNTIME_DIR=$XDG_RUNTIME_DIR gnome-shell --devkit --wayland --no-x11 > "$OUT/gnome-shell.log" 2>&1 &
wait_for 30 "gnome-shell has no Wayland socket: $(tail -3 "$OUT/gnome-shell.log")" test -S "$GNOME_RT/wayland-0"
gbus gdbus call --session -d org.freedesktop.DBus -o /org/freedesktop/DBus -m org.freedesktop.DBus.NameHasOwner org.kde.StatusNotifierWatcher \
  | tee "$OUT/status-notifier-watcher.txt"
gbus gdbus call --session -d org.gnome.Shell.Extensions -o /org/gnome/Shell/Extensions -m org.gnome.Shell.Extensions.ListExtensions \
  > "$OUT/extensions.txt" 2>&1 || true

env DBUS_SESSION_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS XDG_RUNTIME_DIR=$GNOME_RT WAYLAND_DISPLAY=wayland-0 \
  QT_QPA_PLATFORM=wayland QT_FORCE_STDERR_LOGGING=1 "$BIN/andromeda-link" --hold "$OUT" > "$OUT/app.log" 2>&1 &
APP=$!
wait_for 60 "the app never saw the service: $(tail -3 "$OUT/app.log")" grep -q "HOLD listening" "$OUT/app.log"
# The service has no call that lists notifications posted before the app listens.
adb shell cmd notification post -S bigtext -t "'Build finished'" "e2e-build-$$" "'All 42 checks passed'" > /dev/null
adb shell cmd notification post -t "'Parcel'" "e2e-parcel-$$" "'Your parcel arrives today'" > /dev/null
adb shell am start -W -n org.andromeda.link.fixture/.MessageActivity > /dev/null
for step in "0 false home-light" "0 true home-dark" "7 false settings-light" "7 true settings-dark"; do
  set -- $step
  wait_for 90 "the app did not reach HOLD $1 $2: $(tail -3 "$OUT/app.log")" grep -q "HOLD $1 $2" "$OUT/app.log"
  wait_for 20 "no HOLD saved $3" grep -q "HOLD saved $3" "$OUT/app.log"
  [[ -s $OUT/gnome-$3.png ]] || fail "no gnome-$3.png"
done
wait "$APP" || fail "the app exited with $?"
echo "PASS; artifacts: $OUT"
