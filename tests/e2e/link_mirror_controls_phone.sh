#!/usr/bin/env bash
# Mirror controls on a real phone with Accessibility off, so only the shell helper can act: paired with a private
# andromeda-linkd through tests/e2e/udp_impair.py, the helper started as `andromeda-link-apps --helper` does.
#
# Proves:
# 1. Touch: touch-down, touch-move x5, touch-up reach the fixture as one finger that moved.
# 2. Keys: key-down/up of DPAD_LEFT and of C with Ctrl held arrive with that code and meta state; text "ab" arrives
#    as the keys A and B.
# 3. Wheel: one step down arrives as a scroll of -1.
# 4. Rotation: the fixture turns landscape, the phone restarts its encoder at the turned size, and the viewer goes on
#    decoding frames of that size; back to portrait the same.
# 5. Screen off: screen-off turns the panel off while frames still arrive; screen-on turns it back on.
# Needs ANDROID_SERIAL, the debug APKs and the desktop binaries. Keeps the phone's other pairings, stops the helper,
# turns the panel on and lets the phone rotate freely after. Holds /tmp/andromeda-link-phone.lock, as other phone runs
# do. Writes results.jsonl, luma.tsv, logcat.txt, helper.log, linkd.log, viewer.log to $OUT (default
# ./artifacts/link-mirror-controls-phone).
set -euo pipefail
OUT=${OUT:-$(pwd)/artifacts/link-mirror-controls-phone}
source "$(dirname "$0")/link_phone_lib.sh"
RELAY_PORT=${RELAY_PORT:-48307}
HELPER_LOG=/data/local/tmp/andromeda-link-helper.log

[[ -n ${LINK_PHONE_LOCKED:-} ]] || exec env LINK_PHONE_LOCKED=1 flock --close /tmp/andromeda-link-phone.lock "$0" "$@"
rm -rf "$OUT"
mkdir -p "$OUT"
RUNTIME=$(mktemp -d /tmp/link-mirror-controls.XXXX)
helpers() { adb shell ps -A -o PID,USER,ARGS | awk '$2 == "shell" && /LogHelper/ {print $1}'; }
restore() {
  link MirrorInput "'${ID:-x}'" screen-on "@a{sv} {}" > /dev/null 2>&1 || true
  adb shell cat "$HELPER_LOG" > "$OUT/helper.log" 2>/dev/null || true
  for pid in $(helpers); do adb shell kill "$pid" || true; done
  adb shell am force-stop org.andromeda.link.fixture || true
  forget_phone
  kill $(jobs -p) 2>/dev/null || true
  wait 2>/dev/null
  rm -rf "$RUNTIME"
}
trap restore EXIT
[[ $(adb get-state 2>/dev/null) == device ]] || fail "no phone at $SERIAL"
[[ -s $APK && -s $FIXTURE ]] || fail "no APKs at $APK and $FIXTURE"

private_bus system
start_linkd
HOST=$(ip -4 route get 1.1.1.1 | grep -o 'src [0-9.]*' | cut -d' ' -f2)
echo clean > "$RUNTIME/impair"
python3 "$ROOT/tests/e2e/udp_impair.py" "$RELAY_PORT" "$PORT" "$RUNTIME/impair" "$OUT/relay.tsv" &
install_app
pair_phone "$HOST:$RELAY_PORT"
phone_switches screen
adb shell settings put secure enabled_accessibility_services '""'
adb shell settings put secure accessibility_enabled 0
for pid in $(helpers); do adb shell kill "$pid" || true; done
"$BIN/andromeda-link-apps" any --helper > "$RUNTIME/start.txt" 2>&1 || fail "andromeda-link-apps --helper: $(cat "$RUNTIME/start.txt")"
[[ -n $(helpers) ]] || fail "no helper runs as the shell"
record '{"step":"helper","accessibility":"off"}'

adb logcat -c
adb logcat -v epoch ScreenCapture:I ScreenInput:I LinkFixture:I '*:S' > "$OUT/logcat.txt" &
adb shell am start -W -n org.andromeda.link.fixture/.MirrorProbeActivity > /dev/null
"$BIN/andromeda-link-mirror" "$ID" --luma "$OUT/luma.tsv" --count 1000000 >> "$OUT/viewer.log" 2>&1 &
allow_mirror controls
wait_for 20 "the mirror did not start" eval 'grep -q "mirroring at" "$OUT/linkd.log"'
adb shell am start -W -n org.andromeda.link.fixture/.MirrorProbeActivity > /dev/null
wait_for 15 "no frames decoded" test -s "$OUT/luma.tsv"

input() { link MirrorInput "'$ID'" "$1" "@a{sv} {${2:-}}" > /dev/null || fail "MirrorInput $1 {${2:-}}"; }
seen() { grep -q "$1" "$OUT/logcat.txt"; }
# The helper connects to Link on its own tick; the first touch goes through once it does.
at() { echo "'x': <int32 $1>, 'y': <int32 $2>"; }
touch_once() {
  input touch-down "$(at 5000 3000)"
  for step in 1 2 3 4 5; do input touch-move "$(at 5000 $(( 3000 + step * 600 )))"; done
  input touch-up "$(at 5000 6000)"
  seen "up after"
}
wait_for 20 "Link did not hear from the helper" seen "the shell helper reads input"
touch_once || wait_for 5 "the fixture saw no touch from the helper" seen "up after"
MOVES=$(grep -o 'up after [0-9]*' "$OUT/logcat.txt" | tail -1 | grep -o '[0-9]*$')
(( MOVES >= 3 )) || fail "the finger moved only $MOVES times"
record "{\"step\":\"touch\",\"moves\":$MOVES}"

key() { input key-down "'code': <int32 $1>, 'meta': <int32 ${2:-0}>"; input key-up "'code': <int32 $1>, 'meta': <int32 ${2:-0}>"; }
key 21
key 31 12288
input text "'text': <'ab'>"
wait_for 10 "no DPAD_LEFT" seen "key 21 meta 0"
wait_for 10 "no Ctrl+C" seen "key 31 meta 12288"
wait_for 10 "text did not arrive as keys" eval 'seen "key 29 " && seen "key 30 "'
record '{"step":"keys","dpad_left":true,"ctrl_c":true,"text_as_keys":true}'

input wheel "$(at 5000 5000), 'x2': <int32 0>, 'y2': <int32 100>"
wait_for 10 "no wheel" seen "wheel -1.0"
record '{"step":"wheel"}'

# turned W H: the phone's latest encoder size.
encoder_size() { grep -o 'resized: [0-9]*x[0-9]*' "$OUT/logcat.txt" | tail -1 | grep -o '[0-9]*x[0-9]*'; }
frames() { wc -l < "$OUT/luma.tsv"; }
adb shell am start -W -n org.andromeda.link.fixture/.MirrorProbeActivity --ez landscape true > /dev/null
wait_for 15 "the encoder did not restart for landscape" eval '[[ -n $(encoder_size) ]]'
LANDSCAPE=$(encoder_size)
(( ${LANDSCAPE%x*} > ${LANDSCAPE#*x} )) || fail "landscape encoder is $LANDSCAPE"
BEFORE=$(frames)
input touch-down "$(at 5000 5000)"; input touch-up "$(at 5000 5000)"
wait_for 10 "no frames after turning landscape" eval '(( $(frames) > BEFORE + 10 ))'
adb shell am start -W -n org.andromeda.link.fixture/.MirrorProbeActivity --ez landscape false > /dev/null
wait_for 15 "the encoder did not restart for portrait" eval '[[ $(encoder_size) != "$LANDSCAPE" ]]'
PORTRAIT=$(encoder_size)
BEFORE=$(frames)
input touch-down "$(at 5000 5000)"; input touch-up "$(at 5000 5000)"
wait_for 10 "no frames after turning back" eval '(( $(frames) > BEFORE + 10 ))'
record "{\"step\":\"rotation\",\"landscape\":\"$LANDSCAPE\",\"portrait\":\"$PORTRAIT\"}"

panel() { adb shell dumpsys SurfaceFlinger | grep -o -m1 -i 'powerMode=[A-Za-z]*' | cut -d= -f2; }
input screen-off
wait_for 10 "the panel did not turn off: $(panel)" eval '[[ $(panel) == Off ]]'
BEFORE=$(frames)
input touch-down "$(at 5000 5000)"; input touch-up "$(at 5000 5000)"
wait_for 10 "no frames with the panel off" eval '(( $(frames) > BEFORE + 10 ))'
input screen-on
wait_for 10 "the panel did not turn on: $(panel)" eval '[[ $(panel) == On ]]'
record '{"step":"screen-off","frames_while_off":true}'

cat "$OUT/results.jsonl"
echo "PASS; artifacts: $OUT"
