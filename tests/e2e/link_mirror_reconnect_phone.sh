#!/usr/bin/env bash
# The Qt app's Screen page gets mirroring back after the network drops, with a real phone through
# tests/e2e/udp_impair.py, which drops every packet for a while and then lets them through again.
#
# Proves:
# 1. The page goes live with the phone's screen (video size known).
# 2. Total loss: within seconds the page shows "reconnecting" instead of a frozen picture. After 12 s the link is
#    clean again and the page is live on the same stream, with no new consent on the phone.
# Needs ANDROID_SERIAL, the debug APKs and the app built (cargo build -p link-desk). Keeps the phone's other pairings.
# Holds /tmp/andromeda-link-phone.lock, as other phone runs do. Writes steps.txt, app.log, linkd.log, relay.tsv to $OUT
# (default ./artifacts/link-mirror-reconnect-phone).
set -euo pipefail
OUT=${OUT:-$(pwd)/artifacts/link-mirror-reconnect-phone}
source "$(dirname "$0")/link_phone_lib.sh"
RELAY_PORT=${RELAY_PORT:-48307}

[[ -n ${LINK_PHONE_LOCKED:-} ]] || exec env LINK_PHONE_LOCKED=1 flock --close /tmp/andromeda-link-phone.lock "$0" "$@"
rm -rf "$OUT"
mkdir -p "$OUT"
RUNTIME=$(mktemp -d /tmp/link-mirror-reconnect.XXXX)
trap 'adb shell input keyevent KEYCODE_HOME; forget_phone; kill $(jobs -p) 2>/dev/null || true; wait 2>/dev/null; rm -rf "$RUNTIME"' EXIT
step() { printf '%s\n' "$*" | tee -a "$OUT/steps.txt"; }
[[ $(adb get-state 2>/dev/null) == device ]] || fail "no phone at $SERIAL"
[[ -s $APK && -s $FIXTURE ]] || fail "no APKs at $APK and $FIXTURE"
[[ -x $BIN/andromeda-link ]] || fail "no $BIN/andromeda-link; cargo build -p link-desk"

private_bus system
start_linkd
HOST=$(ip -4 route get 1.1.1.1 | grep -o 'src [0-9.]*' | cut -d' ' -f2)
echo clean > "$RUNTIME/impair"
python3 "$ROOT/tests/e2e/udp_impair.py" "$RELAY_PORT" "$PORT" "$RUNTIME/impair" "$OUT/relay.tsv" &
install_app
pair_phone "$HOST:$RELAY_PORT"
phone_switches screen

QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=rhi QT_FORCE_STDERR_LOGGING=1 \
  "$BIN/andromeda-link" --page screen "$OUT" start now > "$OUT/app.log" 2>&1 &
state() { grep -o 'PAGE state {"state":"[a-z]*"' "$OUT/app.log" | tail -1 | cut -d'"' -f4; }
asks() { grep -c "asking the phone to mirror" "$OUT/linkd.log" || true; }
wait_for 30 "the page did not start mirroring: $(tail -3 "$OUT/app.log")" grep -q "PAGE start true" "$OUT/app.log"
allow_mirror first
wait_for 30 "the page did not go live: $(state)" eval '[[ $(state) == live ]]'
step "live: $(grep -o '"width":[0-9]*,"height":[0-9]*' "$OUT/app.log" | tail -1)"

echo "loss=1.0" > "$RUNTIME/impair"
T0=$(now_ms)
wait_for 10 "the page still shows live with no picture: $(state)" eval '[[ $(state) == reconnecting ]]'
step "reconnecting $(( ($(now_ms) - T0) / 1000 )) s after the drop began"
read -rt 12 < /dev/zero 2> /dev/null || true
echo clean > "$RUNTIME/impair"
T0=$(now_ms)
# A drop shorter than the session's idle timeout ends nothing: the same stream goes on, with no new consent.
wait_for 60 "the page did not go live again: $(state)" eval '[[ $(state) == live ]]'
(( $(asks) == 1 )) || fail "the page asked the phone again: $(asks) asks"
step "live again $(( ($(now_ms) - T0) / 1000 )) s after the link came back, same stream"
echo "PASS; artifacts: $OUT"
