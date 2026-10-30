#!/usr/bin/env bash
# The desktop stays awake while a phone presents or mirrors here, against a mock org.freedesktop.ScreenSaver on a
# private bus; nothing reaches the real session. Proves: with the input grant, the phone's `presenting on` makes
# KeepAwake true and link-session calls Inhibit; `presenting off` makes it false and link-session calls UnInhibit with
# the same cookie; an OpenMirror stream holds the inhibit until the viewer closes it. Writes results.jsonl,
# screensaver.log, session.log, hold.jsonl, and linkd.log to $OUT (default ./artifacts/link-awake).
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
OUT=${OUT:-$(pwd)/artifacts/link-awake}
BIN=${LINK_BIN:-$ROOT/target/debug}
if [[ -z ${LINK_IN_NS:-} ]]; then
  rm -rf "$OUT"
  mkdir -p "$OUT"
  exec env LINK_IN_NS=1 OUT="$OUT" BIN="$BIN" unshare --user --map-root-user --net bash "$0"
fi

RUNTIME=$(mktemp -d /tmp/link-awake.XXXX)
trap 'kill $(jobs -p) 2>/dev/null || true; wait 2>/dev/null; rm -rf "$RUNTIME"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
record() { printf '%s\n' "$1" >> "$OUT/results.jsonl"; }
wait_for() {
  local tries=$(( $1 * 10 )) what=$2; shift 2
  for _ in $(seq "$tries"); do "$@" && return; sleep 0.1; done
  fail "$what"
}

ip link set lo up
ip link add d0 type veth peer name p0
unshare --net sleep infinity &
PHONE_NS=$!
for _ in $(seq 50); do [[ $(readlink /proc/$PHONE_NS/ns/net) != "$(readlink /proc/self/ns/net)" ]] && break; sleep 0.02; done
ip link set p0 netns "$PHONE_NS"
ip addr add 10.79.0.1/24 dev d0
ip link set d0 up
in_phone() { nsenter -t "$PHONE_NS" -n -- "$@"; }
in_phone ip link set lo up
in_phone ip addr add 10.79.0.2/24 dev p0
in_phone ip link set p0 up

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
unset WAYLAND_DISPLAY DISPLAY

SAVER=(-d org.freedesktop.ScreenSaver -o /org/freedesktop/ScreenSaver)
python3 -m dbusmock -l "$OUT/screensaver.log" org.freedesktop.ScreenSaver /org/freedesktop/ScreenSaver \
  org.freedesktop.ScreenSaver 2>> "$OUT/dbus.log" &
wait_for 10 "the mock ScreenSaver never started" gdbus introspect --session "${SAVER[@]}" > /dev/null 2>&1
add_method() { gdbus call --session "${SAVER[@]}" -m org.freedesktop.DBus.Mock.AddMethod org.freedesktop.ScreenSaver "$@" > /dev/null; }
add_method Inhibit ss u 'ret = dbus.UInt32(4242)'
add_method UnInhibit u '' ''
add_method GetActive '' b 'ret = False'

link() { gdbus call --session -d org.andromeda.Link1 -o /org/andromeda/Link1 -m "org.andromeda.Link1.$1" "${@:2}"; }
prop() { gdbus call --session -d org.andromeda.Link1 -o /org/andromeda/Link1 -m org.freedesktop.DBus.Properties.Get org.andromeda.Link1 "$1"; }
connected() { prop Devices | grep -q "'phone', true"; }
awake() { prop KeepAwake | grep -q "$1"; }
calls() { grep -c "$1" "$OUT/screensaver.log" || true; }
say() { printf '%s\n' "$*" >&3; }

XDG_STATE_HOME=$RUNTIME/desktop XDG_DOWNLOAD_DIR=$RUNTIME/downloads "$BIN/andromeda-linkd" > "$OUT/linkd.log" 2>&1 &
for _ in $(seq 100); do link CancelPairing > /dev/null 2>&1 && break; sleep 0.05; done
"$BIN/andromeda-link-session" > "$OUT/session.log" 2>&1 &
wait_for 10 "andromeda-link-session did not start" grep -q running "$OUT/session.log"
PORT=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["port"])' "$RUNTIME/desktop/andromeda-link/devices.json")
CODE=$(link StartPairing | sed -E "s/^\('([0-9]+)', .*/\1/")
in_phone "$BIN/andromeda-link-phone" --state "$RUNTIME/phone" --name phone pair --code "$CODE" --addr "10.79.0.1:$PORT" \
  > /dev/null || fail "pairing"
ID=$(prop Devices | grep -o "'[0-9a-f]\{32\}', 'phone'" | cut -d"'" -f2)
link SetGrant "$ID" input true > /dev/null
# The phone's Screen switch for this desktop, which the app's user turns on.
python3 - "$(find "$RUNTIME/phone" -name devices.json)" <<'PY'
import json, sys
store = json.load(open(sys.argv[1]))
for peer in store["peers"]:
    peer.setdefault("grants", {})["screen"] = True
json.dump(store, open(sys.argv[1], "w"))
PY

# Two seconds of 64x64 H.264 with access unit delimiters, as the phone's capture would send.
ffmpeg -loglevel error -f lavfi -i testsrc=size=64x64:rate=30 -t 2 -c:v libx264 -x264-params aud=1 -bsf:v h264_mp4toannexb \
  -f h264 "$RUNTIME/screen.h264"
mkfifo "$RUNTIME/phone.in"
exec 3<> "$RUNTIME/phone.in"
in_phone "$BIN/andromeda-link-phone" --state "$RUNTIME/phone" --name phone --mirror-file "$RUNTIME/screen.h264" \
  --mirror-size 64x64 hold < "$RUNTIME/phone.in" > "$OUT/hold.jsonl" 2> "$OUT/hold.log" &
wait_for 10 "the phone never connected" connected
awake false || fail "KeepAwake is on before anything runs: $(prop KeepAwake)"

say "presenting on"
wait_for 5 "KeepAwake never went on: $(prop KeepAwake); $(tail -2 "$OUT/hold.jsonl")" awake true
wait_for 5 "link-session never called Inhibit: $(tail -3 "$OUT/session.log")" eval '(( $(calls "Inhibit \"Andromeda Link\"") == 1 ))'
record '{"step":"presenting-on","keep_awake":true,"inhibit":1}'
say "presenting off"
wait_for 5 "KeepAwake never went off: $(prop KeepAwake)" awake false
wait_for 5 "link-session never called UnInhibit: $(tail -3 "$OUT/session.log")" eval '(( $(calls "UnInhibit 4242") == 1 ))'
record '{"step":"presenting-off","keep_awake":false,"uninhibit":1}'

# A viewer holds the mirror's socket open until told to close it.
python3 - "$RUNTIME/close" > "$RUNTIME/viewer.out" 2>&1 <<'PY' &
import os, sys, time
import dbus
link = dbus.Interface(dbus.SessionBus().get_object("org.andromeda.Link1", "/org/andromeda/Link1"), "org.andromeda.Link1")
device = [d for d, name, _ in link.get_dbus_method("Get", "org.freedesktop.DBus.Properties")("org.andromeda.Link1", "Devices") if name == "phone"][0]
fd, width, height = link.OpenMirror(device, timeout=60)
stream = os.fdopen(fd.take(), "rb", buffering=0)
print("streaming", len(stream.read(4096)), width, height, flush=True)
while not os.path.exists(sys.argv[1]):
    time.sleep(0.1)
stream.close()
print("closed", flush=True)
PY
wait_for 30 "the mirror never streamed: $(cat "$RUNTIME/viewer.out")" grep -q streaming "$RUNTIME/viewer.out"
wait_for 5 "KeepAwake never went on for the mirror: $(prop KeepAwake)" awake true
wait_for 5 "no Inhibit for the mirror" eval '(( $(calls "Inhibit \"Andromeda Link\"") == 2 ))'
record '{"step":"mirror-open","keep_awake":true,"inhibit":2}'
touch "$RUNTIME/close"
wait_for 10 "KeepAwake stayed on after the viewer closed: $(prop KeepAwake)" awake false
wait_for 5 "no UnInhibit after the mirror" eval '(( $(calls "UnInhibit 4242") == 2 ))'
record '{"step":"mirror-closed","keep_awake":false,"uninhibit":2}'
cat "$OUT/results.jsonl"
echo "PASS; artifacts: $OUT"
