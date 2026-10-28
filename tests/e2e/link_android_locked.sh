#!/usr/bin/env bash
# The Android app on an emulator against a private andromeda-linkd (own dbus-daemon, own XDG_STATE_HOME), with "full
# content while locked" off and the lock screen's "Hide sensitive content" on. Proves: a private chat message (the
# fixture app, with a public version) posted while the phone is locked reaches D-Bus as its public version only, and
# one without a public version not at all; at unlock the full text reaches D-Bus under the same key; when notification
# access goes off, the desktop's copy goes.
# Writes screenshots, results.jsonl, signals.txt, linkd.log to $OUT (default ./artifacts/link-android-locked). Needs the
# debug APK, the fixture APK and a running emulator; run under flock /tmp/link-emulator.lock, since the emulator is shared.
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
OUT=${OUT:-$(pwd)/artifacts/link-android-locked}
BIN=${LINK_BIN:-$ROOT/target/debug}
ADB=${ADB:-$HOME/Android/Sdk/platform-tools/adb}
MAESTRO=${MAESTRO:-$HOME/.maestro/bin/maestro}
SERIAL=${ANDROID_SERIAL:-emulator-5554}
APK=${APK:-$ROOT/apps/android/app/build/outputs/apk/debug/app-debug.apk}
FIXTURE_APK=${FIXTURE_APK:-$ROOT/apps/android/fixture/build/outputs/apk/debug/fixture-debug.apk}
FLOWS=$ROOT/apps/android/maestro
PACKAGE=org.andromeda.link
FULL="Dinner at 8, bring the cake"
PUBLIC="New message"

rm -rf "$OUT"
mkdir -p "$OUT"
RUNTIME=$(mktemp -d /tmp/link-android-locked.XXXX)
adb() { "$ADB" -s "$SERIAL" "$@"; }
SENSITIVE=$(adb shell settings get secure lock_screen_allow_private_notifications | tr -d '\r')
cleanup() {
  adb shell settings put secure lock_screen_allow_private_notifications "$SENSITIVE" > /dev/null 2>&1 || true
  adb shell cmd notification allow_listener "$PACKAGE/$PACKAGE.notifications.MirrorService" > /dev/null 2>&1 || true
  kill $(jobs -p) 2>/dev/null || true
  wait 2>/dev/null
  rm -rf "$RUNTIME"
}
trap cleanup EXIT
fail() { echo "FAIL: $*" >&2; adb logcat -d > "$OUT/logcat.txt" 2>&1 || true; exit 1; }
record() { printf '%s\n' "$1" >> "$OUT/results.jsonl"; }
wait_for() {
  local tries=$(( $1 * 10 )) what=$2; shift 2
  for _ in $(seq "$tries"); do "$@" && return; sleep 0.1; done
  fail "$what"
}
maestro() { "$MAESTRO" --device "$SERIAL" test -e OUT="$OUT" "$@" >> "$OUT/maestro.log" 2>&1 || fail "maestro $*"; }
screenshot() { adb exec-out screencap -p > "$OUT/$1.png"; }
locked() { adb shell dumpsys window | grep -q 'isKeyguardShowing=true'; }
unlocked() { ! locked; }

[[ $(adb get-state 2>/dev/null) == device ]] || fail "no emulator at $SERIAL"
[[ -s $APK ]] || fail "no APK at $APK"
[[ -s $FIXTURE_APK ]] || fail "no fixture APK at $FIXTURE_APK"
[[ $(adb shell locksettings get-disabled | tr -d '\r') == false ]] || fail "the emulator's lock screen is disabled"

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
devices() { gdbus call --session -d org.andromeda.Link1 -o /org/andromeda/Link1 -m org.freedesktop.DBus.Properties.Get org.andromeda.Link1 Devices; }
connected() { devices | grep -q "', true)"; }
signal() { grep -F "org.andromeda.Link1.$1 " "$OUT/signals.txt" | grep -F -- "$2" | tail -1; }
heard() { [[ -n $(signal "$1" "$2") ]]; }

XDG_STATE_HOME=$RUNTIME/desktop "$BIN/andromeda-linkd" > "$OUT/linkd.log" 2>&1 &
for _ in $(seq 100); do link CancelPairing > /dev/null 2>&1 && break; sleep 0.05; done
link CancelPairing > /dev/null || fail "andromeda-linkd did not come up"
gdbus monitor --session -d org.andromeda.Link1 -o /org/andromeda/Link1 > "$OUT/signals.txt" 2>&1 &
wait_for 5 "gdbus monitor did not start" test -s "$OUT/signals.txt"

adb shell input keyevent KEYCODE_WAKEUP
adb shell wm dismiss-keyguard
adb shell cmd statusbar collapse
adb shell input keyevent KEYCODE_HOME
adb uninstall "$PACKAGE" > /dev/null 2>&1 || true
adb install "$APK" > /dev/null || fail "installing $APK"
# The app reads the switch once per process, so it goes in before the first launch.
echo '<map><boolean name="full_while_locked" value="false" /></map>' \
  | adb shell "run-as $PACKAGE sh -c 'mkdir -p shared_prefs && cat > shared_prefs/mirror.xml'"
adb shell run-as "$PACKAGE" cat shared_prefs/mirror.xml | grep -q 'full_while_locked" value="false"' \
  || fail "could not write the locked-content switch"
adb shell pm grant "$PACKAGE" android.permission.CAMERA
adb shell pm revoke "$PACKAGE" android.permission.READ_LOGS
adb shell cmd notification allow_listener "$PACKAGE/$PACKAGE.notifications.MirrorService"
adb uninstall org.andromeda.link.fixture > /dev/null 2>&1 || true
adb install "$FIXTURE_APK" > /dev/null || fail "installing $FIXTURE_APK"
adb shell pm grant org.andromeda.link.fixture android.permission.POST_NOTIFICATIONS
adb shell settings put secure lock_screen_allow_private_notifications 0
record '{"step":"installed","full_while_locked":false,"hide_sensitive":true}'

URI=$(link StartPairing | sed -E "s/^\('[0-9]+', '([^']+)'\)$/\1/")
adb shell am start -W -a android.intent.action.VIEW -d "'$URI'" "$PACKAGE" > /dev/null
maestro "$FLOWS/pair.yaml"
wait_for 20 "D-Bus does not show the emulator connected: $(devices)" connected
record '{"step":"paired-and-connected"}'

adb shell input keyevent KEYCODE_SLEEP
wait_for 10 "the phone did not lock" locked
# A private message without a public version first: it must not leave the phone before the public one replaces it.
adb shell am broadcast -f 32 -n org.andromeda.link.fixture/.PostReceiver --es text "'$FULL'" > /dev/null
adb shell am broadcast -f 32 -n org.andromeda.link.fixture/.PostReceiver --es text "'$FULL'" --es public "'$PUBLIC'" > /dev/null
KEY="0|org.andromeda.link.fixture|7|null|$(adb shell pm list packages -U org.andromeda.link.fixture | sed 's/.*uid://' | tr -d '\r')"
wait_for 30 "no NotificationPosted for the message while locked" heard NotificationPosted "$KEY"
LOCKED_POST=$(signal NotificationPosted "$KEY")
[[ $LOCKED_POST == *"$PUBLIC"* ]] || fail "the locked post is not the public version: $LOCKED_POST"
heard NotificationPosted "$FULL" && fail "the full text left the phone while it was locked: $(signal NotificationPosted "$FULL")"
record "{\"step\":\"locked\",\"dbus\":\"$PUBLIC\",\"full_text_sent\":false}"

adb shell input keyevent KEYCODE_WAKEUP
adb shell wm dismiss-keyguard
wait_for 10 "the phone did not unlock" unlocked
wait_for 30 "the full text did not arrive after unlock" heard NotificationPosted "$FULL"
[[ $(signal NotificationPosted "$FULL") == *"$KEY"* ]] || fail "the full text came under another key"
screenshot unlocked
record "{\"step\":\"unlocked\",\"dbus\":\"$FULL\",\"same_key\":true}"

adb shell cmd notification disallow_listener "$PACKAGE/$PACKAGE.notifications.MirrorService"
wait_for 30 "the desktop kept the message after notification access went off" heard NotificationRemoved "$KEY"
adb shell cmd notification allow_listener "$PACKAGE/$PACKAGE.notifications.MirrorService"
record '{"step":"access-off","removed":true}'

echo "PASS; artifacts: $OUT"
