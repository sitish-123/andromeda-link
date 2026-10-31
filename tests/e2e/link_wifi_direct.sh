#!/usr/bin/env bash
# Link's Wi-Fi Direct upgrade without radios, on link_hotspot.sh's set-up: the phone reaches the desktop over the
# Bluetooth stand-in, its Wi-Fi Direct side is --wifi-direct (a group it owns, with fixed credentials), and
# NetworkManager is nm_mock.py; joining brings up the same second veth pair. The home path is cut with 100% loss for
# the whole run.
#
# Proves:
# 1. A phone send over 1 MiB to a desktop whose hello lists feature:wifi-direct-join creates a group instead of the
#    hotspot: the phone sends the group's credentials, the desktop, its Wi-Fi idle, joins that SSID with that
#    passphrase as a volatile, not autoconnected profile without asking, the session moves (via "wifi-direct"), and
#    8 MiB arrives with a matching SHA-256. No hotspot starts and D-Bus signals no Hotspot or HotspotRequested.
# 2. A desktop SendFiles over 1 MiB while the desktop's Wi-Fi has a connection of its own and no second interface can
#    be added: the desktop asks the user (HotspotRequested with the group's SSID) before it leaves its network, joins
#    once allowed, and 8 MiB reaches the phone over the group.
# 3. A group NetworkManager fails to join falls back to the hotspot: the phone starts it, the desktop joins it, the
#    session moves via "hotspot", and the file arrives; the phone does not try a group again in that session.
# 4. In a new session, a group join still running when the phone gives up on it does not end the hotspot the phone
#    falls back to: the file arrives over the hotspot, and the desktop drops the late group without telling the phone.
# Every control message is validated against api/messages.cddl.
# Writes results.jsonl, hold.jsonl, hold.log, nm.jsonl, signals.txt, transcript.jsonl, linkd.log to $OUT (default
# ./artifacts/link-wifi-direct).
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
OUT=${OUT:-$(pwd)/artifacts/link-wifi-direct}
BIN=${LINK_BIN:-$ROOT/target/debug}
if [[ -z ${LINK_IN_NS:-} ]]; then
  rm -rf "$OUT"
  mkdir -p "$OUT"
  exec env LINK_IN_NS=1 OUT="$OUT" BIN="$BIN" unshare --user --map-root-user --net bash "$0"
fi

RUNTIME=$(mktemp -d /tmp/link-wifi-direct.XXXX)
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
ip addr add 10.81.0.1/24 dev d0
ip link set d0 up
in_phone() { nsenter -t "$PHONE_NS" -n -- "$@"; }
in_phone ip link set lo up
in_phone ip addr add 10.81.0.2/24 dev p0
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

JOIN="ip link add h0 type veth peer name hp0 && ip link set hp0 netns $PHONE_NS && ip addr add 10.80.0.1/24 dev h0 \
  && ip link set h0 up && nsenter -t $PHONE_NS -n ip addr add 10.80.0.2/24 dev hp0 \
  && nsenter -t $PHONE_NS -n ip link set hp0 up"
python3 "$ROOT/tests/e2e/nm_mock.py" "$OUT/nm.jsonl" "$JOIN" "ip link del h0" "$RUNTIME/nm-fail" 2> "$OUT/nm.log" &
wait_for 5 "the NetworkManager stand-in did not come up" \
  gdbus introspect --system -d org.freedesktop.NetworkManager -o /org/freedesktop/NetworkManager > /dev/null

RFCOMM=$RUNTIME/rfcomm.sock
DOWNLOADS=$RUNTIME/downloads
PHONE_DL=$RUNTIME/phone-downloads
mkdir -p "$DOWNLOADS" "$PHONE_DL" "$RUNTIME/src"
link() { gdbus call --session -d org.andromeda.Link1 -o /org/andromeda/Link1 -m "org.andromeda.Link1.$1" "${@:2}"; }
devices() { gdbus call --session -d org.andromeda.Link1 -o /org/andromeda/Link1 -m org.freedesktop.DBus.Properties.Get org.andromeda.Link1 Devices; }
XDG_STATE_HOME=$RUNTIME/desktop XDG_DOWNLOAD_DIR=$DOWNLOADS ANDROMEDA_LINK_TEST_BLUETOOTH_SOCKET=$RFCOMM \
  RUST_LOG=info "$BIN/andromeda-linkd" >> "$OUT/linkd.log" 2>&1 &
for _ in $(seq 100); do link CancelPairing > /dev/null 2>&1 && break; sleep 0.05; done
link CancelPairing > /dev/null || fail "andromeda-linkd did not come up"
PORT=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["port"])' "$RUNTIME/desktop/andromeda-link/devices.json")
events() { grep -c "\"event\":\"$1\"" "$OUT/hold.jsonl" || true; }
last_via() { grep '"event":"connected"' "$OUT/hold.jsonl" | tail -1 | grep -o '"via":"[a-z-]*"' | cut -d'"' -f4; }
nm_calls() { grep -c "\"call\": \"$1\"" "$OUT/nm.jsonl" 2>/dev/null || true; }
say() { printf '%s\n' "$*" >&3; }
sha() { sha256sum < "$1" | cut -d' ' -f1; }

CODE=$(link StartPairing | sed -E "s/^\('([0-9]+)', .*/\1/")
in_phone "$BIN/andromeda-link-phone" --state "$RUNTIME/phone" --name phone pair --code "$CODE" --addr "10.81.0.1:$PORT" \
  > /dev/null || fail "pairing"
ID=$(devices | grep -o "'[0-9a-f]\{32\}', 'phone'" | cut -d"'" -f2)
[[ -n $ID ]] || fail "desktop does not list the phone: $(devices)"
link SetAutoAccept "'$ID'" true > /dev/null || fail "SetAutoAccept"
in_phone tc qdisc add dev p0 root netem loss 100%

dbus-monitor --session "type='signal',interface='org.andromeda.Link1'" > "$OUT/signals.txt" 2>&1 &
wait_for 5 "dbus-monitor did not start" test -s "$OUT/signals.txt"
# The user allows every question to leave the desktop's Wi-Fi.
gdbus monitor --session -d org.andromeda.Link1 | while read -r line; do
  [[ $line =~ HotspotRequested\ \(\'([0-9a-f]+)\',\ \'([^\']+)\'\) ]] || continue
  echo "${BASH_REMATCH[2]}" >> "$OUT/asks.txt"
  link AnswerHotspot "${BASH_REMATCH[1]}" true > /dev/null || true
done &
mkfifo "$RUNTIME/phone.in"
exec 3<> "$RUNTIME/phone.in"
GROUP=DIRECT-ab-Link
hold() {
  RUST_LOG=info nsenter -t "$PHONE_NS" -n -- "$BIN/andromeda-link-phone" --state "$RUNTIME/phone" --name phone \
    --downloads "$PHONE_DL" --transcript "$OUT/transcript.jsonl" --bluetooth-socket "$RFCOMM" \
    --hotspot "AndromedaTest:secret pass 1" --wifi-direct "$GROUP:group pass 1" hold --on-offer accept < "$RUNTIME/phone.in" \
    >> "$OUT/hold.jsonl" 2>> "$OUT/hold.log" &
  HOLD=$!
}
hold
wait_for 30 "the held phone never connected over Bluetooth" eval '[[ $(last_via) == bluetooth ]]'

signalled() { grep -q "member=$1\$" "$OUT/signals.txt"; }
head -c 8388608 /dev/urandom > "$RUNTIME/src/group.bin"
say "send $RUNTIME/src/group.bin"
wait_for 40 "the session never moved to the Wi-Fi Direct group" eval '[[ $(last_via) == wifi-direct ]]'
wait_for 60 "the 8 MiB never reached the desktop over the group" test -f "$DOWNLOADS/group.bin"
[[ $(sha "$RUNTIME/src/group.bin") == $(sha "$DOWNLOADS/group.bin") ]] || fail "the file differs on the desktop"
grep -q "wifi-direct group created" "$OUT/hold.log" || fail "the phone did not create a group"
! grep -q "hotspot started" "$OUT/hold.log" || fail "the phone started its hotspot too"
python3 - "$OUT/nm.jsonl" "$GROUP" <<'PY' || fail "the desktop joined the group with other settings: $(cat "$OUT/nm.jsonl")"
import json, sys
calls = [json.loads(line) for line in open(sys.argv[1])]
join = next(call for call in calls if call["call"] == "add-and-activate")
del join["t"]
assert join == {"call": "add-and-activate", "device": "/org/freedesktop/NetworkManager/Devices/2",
                "specific": "/org/freedesktop/NetworkManager/AccessPoint/1", "ssid": sys.argv[2],
                "psk": "group pass 1", "key_mgmt": "wpa-psk", "autoconnect": False, "persist": "volatile",
                "fails": False, "stuck": False}, join
PY
! signalled Hotspot || fail "D-Bus signalled Hotspot for a group joined beside the desktop's network"
! signalled HotspotRequested || fail "the desktop asked to leave a Wi-Fi it was not on"
record '{"step":"phone-send-joins-group","path":"idle-wifi","bytes":8388608,"sha256_match":true,"hotspot_started":false}'

wait_for 90 "the phone never ended the idle group" grep -q "wifi-direct stopped" "$OUT/hold.log"
wait_for 10 "the desktop did not leave the group" eval '[[ $(nm_calls deactivate) == 1 ]]'
wait_for 20 "the phone was not back on Bluetooth after the group" eval '[[ $(last_via) == bluetooth ]]'
record '{"step":"idle-group-ends-back-on-bluetooth"}'

touch "$RUNTIME/nm-fail-wifi-busy"
JOINS=$(nm_calls add-and-activate)
python3 "$ROOT/tests/e2e/link_send_files.py" "$ID" "$RUNTIME/src/group.bin" > /dev/null || fail "desktop SendFiles"
[[ $(last_via) == wifi-direct ]] || fail "SendFiles returned before the session moved to the group"
wait_for 60 "the desktop's 8 MiB never reached the phone over the group" test -f "$PHONE_DL/group.bin"
[[ $(sha "$RUNTIME/src/group.bin") == $(sha "$PHONE_DL/group.bin") ]] || fail "the desktop's file differs on the phone"
(( $(nm_calls add-and-activate) > JOINS )) || fail "the desktop's send did not join a group"
grep -qx "$GROUP" "$OUT/asks.txt" || fail "the desktop left its Wi-Fi for the group without asking"
grep -q "no spare Wi-Fi interface" "$OUT/linkd.log" || fail "the desktop did not look for a second interface first"
record '{"step":"desktop-send-asks-then-joins-group","path":"leave-wifi","bytes":8388608,"sha256_match":true}'
wait_for 90 "the second group never ended" eval '[[ $(nm_calls deactivate) == 2 ]]'
wait_for 20 "the phone was not back on Bluetooth after the second group" eval '[[ $(last_via) == bluetooth ]]'
rm "$RUNTIME/nm-fail-wifi-busy"

touch "$RUNTIME/nm-fail-group"
cp "$RUNTIME/src/group.bin" "$RUNTIME/src/fallback.bin"
say "send $RUNTIME/src/fallback.bin"
wait_for 60 "a failed group did not fall back to the hotspot" eval '[[ $(last_via) == hotspot ]]'
wait_for 60 "the file never arrived over the fallback hotspot" test -f "$DOWNLOADS/fallback.bin"
[[ $(sha "$RUNTIME/src/fallback.bin") == $(sha "$DOWNLOADS/fallback.bin") ]] || fail "the fallback file differs"
grep -q '"fails": true' "$OUT/nm.jsonl" || fail "the desktop did not try the failing group"
grep -q "hotspot started" "$OUT/hold.log" || fail "the phone did not start its hotspot after the group failed"
record '{"step":"failed-group-falls-back-to-hotspot","bytes":8388608,"sha256_match":true}'

wait_for 90 "the fallback hotspot never ended" eval '[[ $(nm_calls deactivate) == 3 ]]'
wait_for 20 "the phone was not back on Bluetooth after the fallback hotspot" eval '[[ $(last_via) == bluetooth ]]'
kill "$HOLD"
wait "$HOLD" || true
rm "$RUNTIME/nm-fail-group"
touch "$RUNTIME/nm-fail-group-stuck"
SESSIONS=$(grep -c '"via":"bluetooth"' "$OUT/hold.jsonl")
hold
wait_for 30 "the new session did not come over Bluetooth" eval '(( $(grep -c "\"via\":\"bluetooth\"" "$OUT/hold.jsonl") > SESSIONS ))'
cp "$RUNTIME/src/group.bin" "$RUNTIME/src/late.bin"
say "send $RUNTIME/src/late.bin"
wait_for 100 "a stuck group did not fall back to the hotspot" eval '[[ $(last_via) == hotspot ]]'
wait_for 60 "the file never arrived over the hotspot after a stuck group" test -f "$DOWNLOADS/late.bin"
[[ $(sha "$RUNTIME/src/late.bin") == $(sha "$DOWNLOADS/late.bin") ]] || fail "the file after a stuck group differs"
wait_for 60 "the desktop's stuck group join never ended" grep -q "dropping the Wi-Fi Direct group the phone gave up on" \
  "$OUT/linkd.log"
! grep -q "the desktop ended the hotspot" "$OUT/hold.log" || fail "the late group failure ended the phone's hotspot"
[[ $(last_via) == hotspot ]] || fail "the session left the hotspot after the late group failure"
record '{"step":"late-group-failure-keeps-hotspot","bytes":8388608,"sha256_match":true}'

CHECK=$("$BIN/andromeda-link-phone" check-transcript "$ROOT/api/messages.cddl" "$OUT/transcript.jsonl") \
  || fail "transcript violates the schema"
record "$(echo "$CHECK" | sed 's/^{/{"step":"transcript-schema",/')"
cat "$OUT/results.jsonl"
echo "PASS; artifacts: $OUT"
