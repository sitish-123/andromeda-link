#!/usr/bin/env bash
# RecentPhotos, SavePhoto, SavePhotoTo, DeviceInfo, and Disconnect/Connect through the real daemon and the headless phone (a folder of albums
# stands in for Android's media store).
#
# Proves:
# 1. With the phone's browse switch off, RecentPhotos is refused (not-allowed).
# 2. With it on, RecentPhotos lists the photos newest first and stops at the count; a count of 0 or 65 is rejected.
# 3. A `.denied` marker (Android withholding photo access) makes RecentPhotos fail with Refused "denied", not an empty list.
# 4. SavePhoto copies a listed photo byte for byte into the downloads folder, tagged with its origin xattrs, adds " (2)"
#    instead of overwriting, and rejects a path RecentPhotos did not give. SavePhotoTo does the same into a given
#    folder and rejects a missing folder, a file, a relative path, and a folder of another user.
# 5. DeviceInfo shows connected, battery, the phone's reported storage, and last_seen; after Disconnect it keeps the
#    storage without the battery; after a daemon restart last_seen stays and the storage is gone.
# 6. Disconnect ends the session, the phone is refused with close code 7 (paused), the flag survives a daemon restart,
#    and Connect lets the phone back in.
# Every message is checked against api/messages.cddl. Writes results.jsonl, transcript.jsonl, linkd.log, hold.log to
# $OUT (default ./artifacts/link-photos-recent).
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
OUT=${OUT:-$(pwd)/artifacts/link-photos-recent}
BIN=${LINK_BIN:-$ROOT/target/debug}
if [[ -z ${LINK_IN_NS:-} ]]; then
  rm -rf "$OUT"
  mkdir -p "$OUT"
  exec env LINK_IN_NS=1 OUT="$OUT" BIN="$BIN" unshare --user --map-root-user --net bash "$0"
fi

RUNTIME=$(mktemp -d /tmp/link-photos-recent.XXXX)
trap 'kill $(jobs -p) 2>/dev/null || true; wait 2>/dev/null; rm -rf "$RUNTIME"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
record() { printf '%s\n' "$1" >> "$OUT/results.jsonl"; }
wait_for() {
  local tries=$(( $1 * 10 )) what=$2; shift 2
  for _ in $(seq "$tries"); do "$@" && return; sleep 0.1; done
  fail "$what"
}

ip link set lo up
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
connected() { prop Devices | grep -q "'phone', true"; }
disconnected() { ! connected; }
start_daemon() {
  XDG_STATE_HOME=$RUNTIME/desktop XDG_DOWNLOAD_DIR=$RUNTIME/downloads "$BIN/andromeda-linkd" >> "$OUT/linkd.log" 2>&1 &
  DAEMON=$!
  for _ in $(seq 100); do link CancelPairing > /dev/null 2>&1 && break; sleep 0.05; done
  link CancelPairing > /dev/null || fail "andromeda-linkd did not come up"
}
start_daemon
STORE=$RUNTIME/desktop/andromeda-link/devices.json
PORT=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["port"])' "$STORE")

LIBRARY=$RUNTIME/library
mkdir -p "$LIBRARY/Camera" "$LIBRARY/Screenshots"
python3 - "$LIBRARY" <<'PY'
import os, struct, sys, time, zlib
def png(path, seed, age):
    rows = b"".join(b"\0" + bytes((x * seed + y) % 256 for x in range(3 * 16)) for y in range(16))
    chunk = lambda kind, data: struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    open(path, "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 16, 16, 8, 2, 0, 0, 0))
                           + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))
    t = time.time() - age
    os.utime(path, (t, t))
png(os.path.join(sys.argv[1], "Camera", "oldest.png"), 3, 300)
png(os.path.join(sys.argv[1], "Screenshots", "middle.png"), 5, 200)
png(os.path.join(sys.argv[1], "Camera", "newest.png"), 7, 100)
PY

CODE=$(link StartPairing | sed -E "s/^\('([0-9]+)', .*/\1/")
"$BIN/andromeda-link-phone" --state "$RUNTIME/phone" --name phone pair --code "$CODE" --addr "127.0.0.1:$PORT" > /dev/null \
  || fail "pairing"
ID=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["peers"][0]["id"])' "$STORE")
hold() {
  RUST_LOG=info "$BIN/andromeda-link-phone" --state "$RUNTIME/phone" --name phone --transcript "$OUT/transcript.jsonl" \
    --photos "$LIBRARY" hold --status 80,0,wifi,storage=1000/5000 >> "$OUT/hold.jsonl" 2>> "$OUT/hold.log" &
  HOLD=$!
  wait_for 20 "the phone never connected" connected
}
hold

OUTPUT=$(link RecentPhotos "$ID" 5 2>&1) && fail "browse off listed photos: $OUTPUT"
[[ $OUTPUT == *"not-allowed"* ]] || fail "browse off did not refuse with not-allowed: $OUTPUT"
record '{"step":"browse-off-refused","reason":"not-allowed"}'

kill "$HOLD"; wait "$HOLD" 2>/dev/null || true
python3 - "$RUNTIME/phone/devices.json" <<'PY'
import json, sys
store = json.load(open(sys.argv[1]))
store["peers"][0].setdefault("grants", {})["browse"] = True
json.dump(store, open(sys.argv[1], "w"))
PY
hold
OUTPUT=$(link RecentPhotos "$ID" 5)
names=$(grep -oE "'[a-z]+\.png'" <<< "$OUTPUT" | tr -d "'" | tr '\n' ' ')
[[ $names == "newest.png middle.png oldest.png " ]] || fail "photos not newest first: $OUTPUT"
OUTPUT=$(link RecentPhotos "$ID" 2)
names=$(grep -oE "'[a-z]+\.png'" <<< "$OUTPUT" | tr -d "'" | tr '\n' ' ')
[[ $names == "newest.png middle.png " ]] || fail "count 2 gave: $OUTPUT"
for bad in 0 65; do
  OUTPUT=$(link RecentPhotos "$ID" "$bad" 2>&1) && fail "count $bad was accepted"
  [[ $OUTPUT == *"Rejected"* ]] || fail "count $bad: $OUTPUT"
done
record '{"step":"recent-newest-first","count_limits":true}'

touch "$LIBRARY/.denied"
OUTPUT=$(link RecentPhotos "$ID" 5 2>&1) && fail "denied photo access returned a list: $OUTPUT"
[[ $OUTPUT == *"Refused"* && $OUTPUT == *"denied"* ]] || fail "denied gave: $OUTPUT"
rm "$LIBRARY/.denied"
link RecentPhotos "$ID" 5 > /dev/null || fail "photos did not come back after the grant"
record '{"step":"denied-is-an-error","reason":"denied"}'

link SavePhoto "$ID" "/Photos/Camera/newest.png" > /dev/null || fail "SavePhoto"
link SavePhoto "$ID" "/Photos/Camera/newest.png" > /dev/null || fail "SavePhoto again"
cmp "$RUNTIME/downloads/newest.png" "$LIBRARY/Camera/newest.png" || fail "the saved photo differs"
cmp "$RUNTIME/downloads/newest (2).png" "$LIBRARY/Camera/newest.png" || fail "the second save differs or replaced the first"
OUTPUT=$(link SavePhoto "$ID" "/Photos/Camera/../../etc/passwd" 2>&1) && fail "SavePhoto took a path RecentPhotos did not give"
[[ $OUTPUT == *"Rejected"* ]] || fail "SavePhoto of an unlisted path: $OUTPUT"
[[ $(ls "$RUNTIME/downloads" | wc -l) == 2 ]] || fail "downloads holds: $(ls -a "$RUNTIME/downloads")"
record '{"step":"save-photo","never_overwrites":true,"unlisted_rejected":true}'

origin() { python3 -c 'import os,sys; print(os.getxattr(sys.argv[1], sys.argv[2]).decode())' "$1" "$2"; }
[[ $(origin "$RUNTIME/downloads/newest.png" user.xdg.origin.url) == "link://$ID" ]] || fail "no origin url on the saved photo"
[[ $(origin "$RUNTIME/downloads/newest.png" user.andromeda.origin) == phone ]] || fail "no origin name on the saved photo"
record '{"step":"saved-photo-origin"}'

PICKED=$RUNTIME/picked
mkdir -p "$PICKED"
SAVED=$(link SavePhotoTo "$ID" "/Photos/Screenshots/middle.png" "$PICKED") || fail "SavePhotoTo"
[[ $SAVED == *"$PICKED/middle.png"* ]] || fail "SavePhotoTo returned $SAVED"
link SavePhotoTo "$ID" "/Photos/Screenshots/middle.png" "$PICKED" > /dev/null || fail "SavePhotoTo again"
cmp "$PICKED/middle.png" "$LIBRARY/Screenshots/middle.png" || fail "the photo saved to a folder differs"
cmp "$PICKED/middle (2).png" "$LIBRARY/Screenshots/middle.png" || fail "the second save to a folder differs"
for bad in "$RUNTIME/missing" "$PICKED/middle.png" "picked" /; do
  OUTPUT=$(link SavePhotoTo "$ID" "/Photos/Screenshots/middle.png" "$bad" 2>&1) && fail "SavePhotoTo took folder $bad"
  [[ $OUTPUT == *"Rejected"* ]] || fail "SavePhotoTo into $bad: $OUTPUT"
done
OUTPUT=$(link SavePhotoTo "$ID" "/Photos/Camera/../../etc/passwd" "$PICKED" 2>&1) && fail "SavePhotoTo took an unlisted path"
[[ $OUTPUT == *"Rejected"* ]] || fail "SavePhotoTo of an unlisted path: $OUTPUT"
[[ $(ls -A "$PICKED" | wc -l) == 2 ]] || fail "the picked folder holds: $(ls -A "$PICKED")"
record '{"step":"save-photo-to","never_overwrites":true,"bad_folders_rejected":true}'

info() { prop DeviceInfo; }
INFO=$(info)
for want in "'connected': <true>" "'storage_free': <uint64 1000>" "'storage_total': <uint64 5000>" "'battery': <80>"; do
  [[ $INFO == *"$want"* ]] || fail "DeviceInfo lacks $want: $INFO"
done
SEEN=$(sed -nE "s/.*'last_seen': <int64 ([0-9]+)>.*/\1/p" <<< "$INFO")
(( SEEN > 0 )) || fail "DeviceInfo has no last_seen: $INFO"
record '{"step":"device-info-connected","storage":[1000,5000],"battery":80}'

link Disconnect "$ID" > /dev/null || fail "Disconnect"
wait_for 10 "the phone stayed connected after Disconnect" disconnected
prop Paused | grep -q "$ID" || fail "Paused does not list the phone: $(prop Paused)"
wait_for 20 "the phone was not told code 7" grep -qi "paused" "$OUT/hold.log"
record '{"step":"disconnect-refuses-the-phone","close_code":7}'

INFO=$(info)
[[ $INFO == *"'connected': <false>"* && $INFO == *"'storage_free': <uint64 1000>"* ]] \
  || fail "DeviceInfo after Disconnect: $INFO"
[[ $INFO != *"'battery'"* ]] || fail "DeviceInfo keeps the battery of a disconnected phone: $INFO"
record '{"step":"device-info-keeps-storage-offline"}'

kill "$DAEMON"; wait "$DAEMON" 2>/dev/null || true
grep -q '"paused": true' "$STORE" || fail "the flag was not stored: $(cat "$STORE")"
start_daemon
for _ in $(seq 80); do connected && fail "a paused phone connected after a daemon restart"; sleep 0.1; done
prop Paused | grep -q "$ID" || fail "Paused lost after the restart"
record '{"step":"paused-survives-restart"}'
INFO=$(info)
AFTER=$(sed -nE "s/.*'last_seen': <int64 ([0-9]+)>.*/\1/p" <<< "$INFO" || true)
(( ${AFTER:-0} >= SEEN )) || fail "last_seen lost after the restart: $INFO"
[[ $INFO != *"storage_"* ]] || fail "storage survived a daemon restart: $INFO"
record '{"step":"last-seen-survives-restart"}'

link Connect "$ID" > /dev/null || fail "Connect"
wait_for 45 "the phone did not come back after Connect" connected
! prop Paused | grep -q "$ID" || fail "Paused still lists the phone"
record '{"step":"connect-lets-the-phone-in"}'

kill "$HOLD"; wait "$HOLD" 2>/dev/null || true
"$BIN/andromeda-link-phone" check-transcript "$ROOT/api/messages.cddl" "$OUT/transcript.jsonl" > "$OUT/schema.txt" \
  || fail "transcript violates the schema: $(cat "$OUT/schema.txt")"
record "{\"step\":\"schema\",\"result\":$(cat "$OUT/schema.txt")}"

cat "$OUT/results.jsonl"
echo "PASS; artifacts: $OUT"
