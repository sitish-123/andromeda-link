#!/usr/bin/env bash
# The Qt desktop app (andromeda-link-qt) against a private andromeda-linkd (own dbus-daemon, own XDG_STATE_HOME) and the
# emulator paired with it. Proves the app shows real phone data: connected, a battery level, the fixture app's media
# session as the player, and 3 phone notifications (one a chat with a reply action); its Ring reaches the phone as
# PhoneRinging. Then andromeda-linkd stops and the app shows the phone disconnected. The app runs offscreen with
# `--shot`, which saves light-/dark-connected.png and light-/dark-disconnected.png to $OUT (default
# ./artifacts/link-desk-ui) with app.log, linkd.log, signals.txt. Run under flock /tmp/link-emulator.lock.
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
OUT=${OUT:-$(pwd)/artifacts/link-desk-ui}
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
RUNTIME=$(mktemp -d /tmp/link-desk-ui.XXXX)
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
[[ -x $BIN/andromeda-link-qt ]] || fail "no $BIN/andromeda-link-qt; cargo build -p link-desk"

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

# Texts, photos and files on the phone, and the phone's own grants for this computer (as link_texts_phone.sh).
for permission in READ_SMS SEND_SMS READ_CONTACTS READ_MEDIA_IMAGES READ_MEDIA_VIDEO; do
  adb shell pm grant "$PACKAGE" "android.permission.$permission"
done
adb shell appops set "$PACKAGE" MANAGE_EXTERNAL_STORAGE allow
adb shell am force-stop "$PACKAGE"
adb exec-out run-as "$PACKAGE" cat files/devices.json > "$RUNTIME/devices.json"
python3 - "$RUNTIME/devices.json" <<'PY'
import json, sys
store = json.load(open(sys.argv[1]))
store["peers"][0].setdefault("grants", {}).update(messages=True, browse=True, screen=True)
json.dump(store, open(sys.argv[1], "w"))
PY
adb shell run-as "$PACKAGE" sh -c "'cat > files/devices.json'" < "$RUNTIME/devices.json"
adb shell am start -W -n "$PACKAGE/.MainActivity" > /dev/null
wait_for 20 "the phone did not reconnect: $(devices)" connected
adb emu sms send 5551234 "e2e hello from the emulator" > /dev/null
for n in 1 2; do
  python3 -c 'import sys, zlib, struct
def chunk(t, d): return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
row = b"\0" + bytes([40 * int(sys.argv[1]), 120, 200]) * 64
sys.stdout.buffer.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 64, 64, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(row * 64)) + chunk(b"IEND", b""))' "$n" > "$RUNTIME/e2e$n.png"
  adb push "$RUNTIME/e2e$n.png" /sdcard/DCIM/Camera/e2e$n.jpg > /dev/null
  adb shell am broadcast -a android.intent.action.MEDIA_SCANNER_SCAN_FILE -d file:///sdcard/DCIM/Camera/e2e$n.jpg > /dev/null
done
echo "e2e note" > "$RUNTIME/e2e-note.txt"

adb shell am start -W -n org.andromeda.link.fixture/.PlayerActivity > /dev/null
wait_for 30 "the phone's media session never reached the bus" eval \
  "gdbus call --session -d org.freedesktop.DBus -o /org/freedesktop/DBus -m org.freedesktop.DBus.ListNames | grep -q andromeda_link_$ID"

# The service has no call that lists notifications already posted, so the app must be listening before they come.
# The software scenegraph, offscreen's default, cannot draw MultiEffect (the player artwork).
QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=rhi QT_FORCE_STDERR_LOGGING=1 "$BIN/andromeda-link-qt" --shot "$OUT" --pages "$RUNTIME/e2e-note.txt" > "$OUT/app.log" 2>&1 &
APP=$!
wait_for 30 "the app never saw the service: $(tail -3 "$OUT/app.log")" said listening

# Tags of this run: a notification an earlier run left reaches the service at pairing, before the app listens.
adb shell cmd notification post -S bigtext -t "'Build finished'" "e2e-build-$$" "'All 42 checks passed'" > /dev/null
adb shell cmd notification post -t "'Parcel'" "e2e-parcel-$$" "'Your parcel arrives today'" > /dev/null
adb shell am start -W -n org.andromeda.link.fixture/.MessageActivity > /dev/null
wait_for 60 "the app did not show the connected phone: $(tail -3 "$OUT/app.log")" said "connected done"
# A file sent to the phone waits for its owner's Accept there.
wait_for 120 "the app did not send a file: $(tail -3 "$OUT/app.log")" said sent
maestro "$FLOWS/activity-accept.yaml"
wait_for 240 "the app did not visit every page: $(tail -3 "$OUT/app.log")" said "pages done"

STATE=$(shot_state)
echo "connected state: $STATE"
python3 -c 'import json, sys
s = json.loads(sys.argv[1])
assert s["connected"] is True, "connected"
assert s["battery"] >= 0, "battery"
assert s["hasPlayer"] is True and s["playerTitle"].startswith("Emulator Song"), "player"
assert s["notifications"] >= 3, "notifications"' "$STATE" || fail "connected state: $STATE"
page_state() { grep -o "SHOT page {\"page\":\"$1\".*}" "$OUT/app.log" | tail -1 | cut -d' ' -f3-; }
for page in messages photos files activity settings screen pairing; do
  PSTATE=$(page_state "$page")
  echo "$page state: $PSTATE"
  case $page in
    messages) check='s["ready"] and s["threads"] > 0' ;;
    photos) check='s["ready"] and s["thumbs"] == s["photos"] > 0' ;;
    files) check='s["ready"] and s["entries"] > 0' ;;
    activity) check='s["ready"] and s["items"] > 0 and set(s["states"]) == {"done"}' ;;
    settings) check='s["ready"] and "messages" in s["grants"]' ;;
    screen) check='s["state"] == "idle"' ;;
    pairing) check='s["state"] == "waiting" and s["codeLength"] == 6' ;;
  esac
  python3 -c 'import json, sys; s = json.loads(sys.argv[1]); assert '"$check" "${PSTATE:-null}" || fail "$page state: $PSTATE"
  [[ -s $OUT/$page-light.png ]] || fail "no $page-light.png"
done
[[ -s $OUT/settings-dark.png ]] || fail "no settings-dark.png"
wait_for 10 "the app's Ring did not ring the phone" heard PhoneRinging "'$ID', true"
link Ring "$ID" false > /dev/null || fail "D-Bus Ring off"

kill "$LINKD"
wait_for 30 "the app did not show the phone disconnected: $(tail -3 "$OUT/app.log")" eval '! kill -0 $APP 2>/dev/null'
wait "$APP" || fail "the app exited with $?: $(tail -3 "$OUT/app.log")"
STATE=$(shot_state)
echo "disconnected state: $STATE"
python3 -c 'import json, sys; assert json.loads(sys.argv[1])["connected"] is False' "$STATE" || fail "disconnected state: $STATE"
for shot in light-connected dark-connected light-disconnected dark-disconnected; do
  [[ -s $OUT/$shot.png ]] || fail "no $shot.png"
done
grep -E "qml:|QML|TypeError|ReferenceError" "$OUT/app.log" | grep -v "SHOT" > "$OUT/qml-warnings.txt" || true
[[ -s $OUT/qml-warnings.txt ]] && echo "QML warnings: $OUT/qml-warnings.txt"
echo "PASS; artifacts: $OUT"
