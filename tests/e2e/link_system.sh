#!/usr/bin/env bash
# Desktop status and control both ways (Stage 4), against mock logind, UPower, power-profiles-daemon, and Andromeda
# shell services on a private bus, a stand-in wpctl, and a fake backlight; nothing reaches the real session. Proves: the
# phone gets the desktop's volume, battery, brightness, lock, DND, and profile; with the system grant the phone sets
# volume and brightness, locks, suspends, turns DND on, and sets the profile, each answered with a system-result; a
# lock reaches the phone as locked; without the grant each command fails with a reason and changes nothing; the
# desktop sets the phone's ringer and waits for its answer; with DND sync the desktop's DND goes to the phone and the
# phone's to the desktop; a phone media volume change arrives within 1 s. Every message is validated against api/messages.cddl. Writes results.jsonl, hold.jsonl,
# mocks.log, session.log, and linkd.log to $OUT (default ./artifacts/link-system).
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
OUT=${OUT:-$(pwd)/artifacts/link-system}
BIN=${LINK_BIN:-$ROOT/target/debug}
if [[ -z ${LINK_IN_NS:-} ]]; then
  rm -rf "$OUT"
  mkdir -p "$OUT"
  exec env LINK_IN_NS=1 OUT="$OUT" BIN="$BIN" unshare --user --map-root-user --net bash "$0"
fi

RUNTIME=$(mktemp -d /tmp/link-system.XXXX)
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
# One private bus stands in for both, so the mocks never meet the real logind.
export DBUS_SESSION_BUS_ADDRESS=unix:path=$RUNTIME/bus DBUS_SYSTEM_BUS_ADDRESS=unix:path=$RUNTIME/bus
unset WAYLAND_DISPLAY DISPLAY

# A stand-in wpctl keeps the volume in a file; pactl fails, so link-session has no watcher.
mkdir -p "$RUNTIME/bin" "$RUNTIME/backlight/test_backlight"
echo "0.40" > "$RUNTIME/volume"
cat > "$RUNTIME/bin/wpctl" <<EOF
#!/usr/bin/env bash
case \$1 in
  get-volume) printf 'Volume: %s%s\n' "\$(cat $RUNTIME/volume)" "\$([[ -e $RUNTIME/muted ]] && echo ' [MUTED]')" ;;
  set-volume) python3 -c "import sys; print(f'{int(sys.argv[1].rstrip(\"%\")) / 100:.2f}')" "\$3" > $RUNTIME/volume ;;
  set-mute) if [[ \$3 == 1 ]]; then touch $RUNTIME/muted; else rm -f $RUNTIME/muted; fi ;;
esac
EOF
printf '#!/bin/sh\nexit 1\n' > "$RUNTIME/bin/pactl"
chmod +x "$RUNTIME/bin/wpctl" "$RUNTIME/bin/pactl"
echo 1000 > "$RUNTIME/backlight/test_backlight/max_brightness"
echo 500 > "$RUNTIME/backlight/test_backlight/actual_brightness"

python3 "$ROOT/tests/e2e/link_system_mocks.py" "$OUT/mocks.log" "$RUNTIME/backlight/test_backlight" \
  > "$RUNTIME/mocks.out" 2>> "$OUT/mocks.log" &
wait_for 10 "the mock services never started" grep -q ready "$RUNTIME/mocks.out"

link() { gdbus call --session -d org.andromeda.Link1 -o /org/andromeda/Link1 -m "org.andromeda.Link1.$1" "${@:2}"; }
prop() { gdbus call --session -d org.andromeda.Link1 -o /org/andromeda/Link1 -m org.freedesktop.DBus.Properties.Get org.andromeda.Link1 "$1"; }
mock_prop() { gdbus call --session -d "$1" -o "$2" -m org.freedesktop.DBus.Properties.Get "$3" "$4"; }
mock_set() { gdbus call --session -d "$1" -o "$2" -m org.freedesktop.DBus.Properties.Set "$3" "$4" "$5" > /dev/null; }
shell_dnd() { mock_prop org.andromeda.Shell1 /org/andromeda/Shell1 org.andromeda.Shell1 DoNotDisturb; }
connected() { prop Devices | grep -q "'phone', true"; }
say() { printf '%s\n' "$*" >&3; }
last() { grep "\"event\":\"$1\"" "$OUT/hold.jsonl" | tail -1; }
count() { grep -c "\"event\":\"$1\"" "$OUT/hold.jsonl" || true; }
called() { cat "$OUT"/mocks.log.* | grep -c "$1" || true; }
# Sends a system-command from the phone and waits for its answer; prints the answer.
ask() {
  local before; before=$(count system-result)
  say "system {\"command\":\"$1\"${2:+,\"value\":$2}}"
  wait_for 10 "no system-result for $1" eval "(( \$(count system-result) > $before ))"
  last system-result
}

XDG_STATE_HOME=$RUNTIME/desktop XDG_DOWNLOAD_DIR=$RUNTIME/downloads "$BIN/andromeda-linkd" > "$OUT/linkd.log" 2>&1 &
for _ in $(seq 100); do link CancelPairing > /dev/null 2>&1 && break; sleep 0.05; done
PATH=$RUNTIME/bin:$PATH ANDROMEDA_LINK_BACKLIGHT=$RUNTIME/backlight "$BIN/andromeda-link-session" \
  > "$OUT/session.log" 2>&1 &
wait_for 10 "andromeda-link-session did not start" grep -q running "$OUT/session.log"
grep -q "can: .*volume.*brightness.*lock.*suspend.*profile.*dnd.*screen-off" "$OUT/session.log" \
  || fail "link-session lacks a backend: $(grep 'can:' "$OUT/session.log")"
PORT=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["port"])' "$RUNTIME/desktop/andromeda-link/devices.json")
CODE=$(link StartPairing | sed -E "s/^\('([0-9]+)', .*/\1/")
in_phone "$BIN/andromeda-link-phone" --state "$RUNTIME/phone" --name phone pair --code "$CODE" --addr "10.79.0.1:$PORT" \
  > /dev/null || fail "pairing"
ID=$(prop Devices | grep -o "'[0-9a-f]\{32\}', 'phone'" | cut -d"'" -f2)

mkfifo "$RUNTIME/phone.in"
exec 3<> "$RUNTIME/phone.in"
in_phone "$BIN/andromeda-link-phone" --state "$RUNTIME/phone" --name phone --transcript "$OUT/transcript.jsonl" \
  hold --status 80,0,wifi,,,dnd=0 < "$RUNTIME/phone.in" > "$OUT/hold.jsonl" 2> "$OUT/hold.log" &
wait_for 10 "the phone never connected" connected

# The desktop's state reaches the phone on connect, the grant off.
full_status() {
  local status; status=$(last desktop-status)
  for field in '"volume":40' '"battery":64' '"brightness":50' '"locked":false' '"dnd":false' '"profile":"balanced"'; do
    [[ $status == *"$field"* ]] || return 1
  done
}
wait_for 10 "no full desktop-status: $(last desktop-status)" full_status
record '{"step":"desktop-status-on-connect","volume":40,"battery":64,"brightness":50,"profile":"balanced"}'

# Without the grant, each command fails with a reason and changes nothing.
for cmd in "lock" "set-volume 70" "set-dnd 1"; do
  read -r name value <<< "$cmd"
  ask "$name" "${value:-}" | grep -q '"error":"Control system is off for this phone on the desktop"' \
    || fail "$name without the grant: $(last system-result)"
done
[[ $(cat "$RUNTIME/volume") == 0.40 && $(called '\bLock\b') == 0 ]] || fail "a command without the grant took effect"
shell_dnd | grep -q false || fail "DND changed without the grant"
record '{"step":"commands-refused-without-grant","reason":"Control system is off for this phone on the desktop"}'

link SetGrant "$ID" system true > /dev/null
wait_for 5 "the phone never heard it may be controlled" eval "last capabilities | grep -q '\"control\"'"

ask set-volume 70 | grep -q '"error":null' || fail "set-volume: $(last system-result)"
[[ $(cat "$RUNTIME/volume") == 0.70 ]] || fail "volume is $(cat "$RUNTIME/volume"), not 0.70"
ask set-brightness 25 | grep -q '"error":null' || fail "set-brightness: $(last system-result)"
[[ $(cat "$RUNTIME/backlight/test_backlight/actual_brightness") == 250 ]] || fail "brightness not set to 250"
wait_for 5 "the phone never saw brightness 25" eval "last desktop-status | grep -q '\"brightness\":25'"
ask set-dnd 1 | grep -q '"error":null' || fail "set-dnd: $(last system-result)"
shell_dnd | grep -q true || fail "the shell's DND is $(shell_dnd)"
ask set-profile 2 | grep -q '"error":null' || fail "set-profile: $(last system-result)"
mock_prop org.freedesktop.UPower.PowerProfiles /org/freedesktop/UPower/PowerProfiles \
  org.freedesktop.UPower.PowerProfiles ActiveProfile | grep -q performance || fail "the profile did not change"
ask lock | grep -q '"error":null' || fail "lock: $(last system-result)"
wait_for 5 "the phone never saw the desktop locked" eval "last desktop-status | grep -q '\"locked\":true'"
ask suspend | grep -q '"error":null' || fail "suspend: $(last system-result)"
[[ $(called 'Suspend False') == 1 ]] || fail "Suspend(false) called $(called 'Suspend False') times"
ask screen-off | grep -q '"error":null' || fail "screen-off: $(last system-result)"
record '{"step":"commands-with-grant","volume":70,"brightness":25,"dnd":true,"profile":"performance","locked":true}'

# The desktop sets the phone's ringer and waits for the phone's answer.
BEFORE=$(count system-command)
link SendSystemCommand "$ID" set-ringer 1 > /dev/null || fail "SendSystemCommand set-ringer"
(( $(count system-command) > BEFORE )) || fail "the phone never got set-ringer"
[[ $(last system-command) == *'"command":"set-ringer"'*'"value":1'* ]] || fail "the phone got $(last system-command)"
record '{"step":"desktop-sets-phone-ringer","value":1}'

# DND sync: the desktop's DND goes to the phone, and the phone's to the desktop.
link SetGrant "$ID" dnd-sync true > /dev/null
desktop_dnd() { [[ $(last desktop-status) == *"\"dnd\":$1"* ]]; }
mock_set org.andromeda.Shell1 /org/andromeda/Shell1 org.andromeda.Shell1 DoNotDisturb '<false>'
wait_for 5 "the phone never saw the desktop's DND off" desktop_dnd false
mock_set org.andromeda.Shell1 /org/andromeda/Shell1 org.andromeda.Shell1 DoNotDisturb '<true>'
wait_for 5 "the phone never saw the desktop's DND on" desktop_dnd true
dnd_sent() { [[ $(last system-command) == *'"command":"set-dnd"'*'"value":1'* ]]; }
wait_for 5 "the phone never got the desktop's DND; controls $(prop DeviceControls); grants $(prop Grants)" dnd_sent
mock_set org.andromeda.Shell1 /org/andromeda/Shell1 org.andromeda.Shell1 DoNotDisturb '<false>'
wait_for 5 "the phone never saw the desktop's DND off again" desktop_dnd false
say "status 80 0 wifi   dnd=1"
wait_for 5 "the desktop's DND never followed the phone's: $(shell_dnd)" eval "shell_dnd | grep -q true"
record '{"step":"dnd-sync-both-ways"}'

# A media volume drag on the phone reaches the desktop live, not after the 10 s status window.
media() { prop DeviceControls | grep -qF "'$ID': ($1,"; }
say "status 80 0 wifi   dnd=1,media=30"
wait_for 15 "DeviceControls never showed media 30: $(prop DeviceControls)" media 30
say "status 80 0 wifi   dnd=1,media=55"
wait_for 1 "media 55 did not arrive within 1 s: $(prop DeviceControls)" media 55
record '{"step":"media-volume-live","within_seconds":1}'

CHECK=$("$BIN/andromeda-link-phone" check-transcript "$ROOT/api/messages.cddl" "$OUT/transcript.jsonl") \
  || fail "transcript violates the schema"
record "$(echo "$CHECK" | sed 's/^{/{"step":"transcript-schema",/')"
cat "$OUT/results.jsonl"
echo "PASS; artifacts: $OUT"
