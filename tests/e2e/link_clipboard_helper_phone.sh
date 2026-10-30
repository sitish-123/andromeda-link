#!/usr/bin/env bash
# Phone-to-PC clipboard through the clipboard helper the desktop starts over adb, for a phone without root, Shizuku or
# READ_LOGS. Paired with a private andromeda-linkd through a UDP relay on a port the firewall lets in. Proves:
# 1. `andromeda-link-apps --helper` starts the helper as the shell user and lets Link draw over other apps; a second
#    start finds it running and starts no other.
# 2. On Automatic, Link moves its watcher to the helper without Android's log access dialog.
# 3. With Link in the background, another app's text copy reaches the desktop as a ClipboardOffered, pulled identical.
# Needs ANDROID_SERIAL, the debug APKs and the desktop binaries. Keeps the phone's other pairings; stops the helper and
# gives READ_LOGS back after. Holds /tmp/andromeda-link-phone.lock, as other phone runs do. Writes steps.txt,
# helper.log, linkd.log and signals.txt to $OUT (default ./artifacts/link-clipboard-helper-phone).
set -euo pipefail
OUT=${OUT:-$(pwd)/artifacts/link-clipboard-helper-phone}
source "$(dirname "$0")/link_phone_lib.sh"
RELAY_PORT=${RELAY_PORT:-48311}
FIXTURE_PACKAGE=org.andromeda.link.fixture
HELPER_LOG=/data/local/tmp/andromeda-link-helper.log

[[ -n ${LINK_PHONE_LOCKED:-} ]] || exec env LINK_PHONE_LOCKED=1 flock --close /tmp/andromeda-link-phone.lock "$0" "$@"
rm -rf "$OUT"
mkdir -p "$OUT"
RUNTIME=$(mktemp -d /tmp/link-clipboard-helper.XXXX)
helpers() { adb shell ps -A -o PID,USER,ARGS | awk '$2 == "shell" && /LogHelper/ {print $1}'; }
stop_helpers() { for pid in $(helpers); do adb shell kill "$pid" || true; done; }
cleanup() {
  adb shell cat "$HELPER_LOG" > "$OUT/helper.log" 2>/dev/null || true
  stop_helpers
  adb shell pm grant "$PACKAGE" android.permission.READ_LOGS 2>/dev/null || true
  forget_phone
  kill $(jobs -p) 2>/dev/null || true
  wait 2>/dev/null
  rm -rf "$RUNTIME"
}
trap cleanup EXIT
step() { printf '%s\n' "$*" | tee -a "$OUT/steps.txt"; }
[[ $(adb get-state 2>/dev/null) == device ]] || fail "no phone at $SERIAL"
[[ -s $APK && -s $FIXTURE ]] || fail "no APKs at $APK and $FIXTURE"
[[ -x $BIN/andromeda-link-apps ]] || fail "no $BIN/andromeda-link-apps"

private_bus
start_linkd
HOST=$(ip -4 route get 1.1.1.1 | grep -o 'src [0-9.]*' | cut -d' ' -f2)
echo clean > "$RUNTIME/impair"
python3 "$ROOT/tests/e2e/udp_impair.py" "$RELAY_PORT" "$PORT" "$RUNTIME/impair" "$RUNTIME/relay.tsv" &
stop_helpers
install_app
# A user who ran no command: no READ_LOGS, no "Display over other apps", and Automatic picks.
adb shell run-as "$PACKAGE" rm -f shared_prefs/log_access.xml
adb shell pm revoke "$PACKAGE" android.permission.READ_LOGS
adb shell appops set "$PACKAGE" SYSTEM_ALERT_WINDOW default
pair_phone "$HOST:$RELAY_PORT"
phone_switches clipboard
step "paired through the relay on $HOST:$RELAY_PORT; READ_LOGS revoked, overlay unset, Automatic"

offers() { grep -c "member=ClipboardOffered" "$OUT/signals.txt" || true; }
last_clip() { grep -A2 'member=ClipboardOffered' "$OUT/signals.txt" | grep -o 'uint64 [0-9]*' | tail -1 | cut -d' ' -f2; }
prompts() { adb logcat -d -b events | grep -c "wm_create_activity.*LogAccessDialogActivity" || true; }
adb logcat -c -b events

ANDROID_SERIAL=$SERIAL "$BIN/andromeda-link-apps" any --helper > "$RUNTIME/start.txt" 2>&1 \
  || fail "andromeda-link-apps --helper: $(cat "$RUNTIME/start.txt")"
grep -q "runs on the phone" "$RUNTIME/start.txt" || fail "no start message: $(cat "$RUNTIME/start.txt")"
[[ $(helpers | wc -l) == 1 ]] || fail "$(helpers | wc -l) helpers run"
HELPER=$(helpers)
adb shell cmd appops get "$PACKAGE" SYSTEM_ALERT_WINDOW | grep -q allow || fail "the overlay was not allowed"
ANDROID_SERIAL=$SERIAL "$BIN/andromeda-link-apps" any --helper > "$RUNTIME/again.txt" 2>&1 || fail "a second start failed"
[[ $(helpers) == "$HELPER" ]] || fail "a second start changed the helpers: $(helpers)"
step "helper pid $HELPER runs as shell; overlay allowed; a second start is a no-op"

# Link's provider gets a `content write` from the helper within a few seconds of seeing Link run.
writer() { adb shell ps -A -o PPID,ARGS | awk -v helper="$HELPER" '$1 == helper && /app_process/ {print}'; }
wait_for 15 "the helper did not connect to Link" eval '[[ -n $(writer) ]]'
step "the helper writes into Link's provider"

adb shell input keyevent KEYCODE_HOME
sleep 2 # real time: the watcher moves to the helper on the main thread
BEFORE=$(offers)
TEXT="copied through the helper $(date +%s%N)"
adb shell am start -W -n "$FIXTURE_PACKAGE/.TextClipboardActivity" --es text "'$TEXT'" > /dev/null
wait_for 20 "the phone's text copy never reached the desktop" eval '(( $(offers) > BEFORE ))'
PULLED=$(python3 "$ROOT/tests/e2e/link_clipboard_dbus.py" pull "$ID" "$(last_clip)" "text/plain;charset=utf-8") \
  || fail "PullClipboard for the text"
[[ $PULLED == "$TEXT" ]] || fail "the text arrived as \"$PULLED\""
[[ $(prompts) == 0 ]] || fail "Android asked for log access $(prompts) times"
step "background text copy: ClipboardOffered, pulled identical, no log access dialog"
echo "PASS; artifacts: $OUT"
