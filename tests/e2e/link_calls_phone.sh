#!/usr/bin/env bash
# The Calls page against the emulator's call log, through a private andromeda-linkd. Proves:
# 1. Calls seeded into the emulator's call log (a missed, an incoming, an outgoing) reach D-Bus Calls with their kind,
#    and the Qt app's Calls page lists them (its "PAGE state" line), with light and dark shots.
# 2. Dial from the page places the call on the emulator (its call state goes off-hook); after it ends, the log's new
#    outgoing call reaches the page through calls-changed.
# Dial only on the emulator: it places a real call on a phone. Needs ANDROID_SERIAL=emulator-*, the debug APKs, and the
# app built (cargo build -p link-desk). Run under flock /tmp/link-emulator.lock. Writes steps.txt, app.log, linkd.log,
# and calls-light.png / calls-dark.png to $OUT (default ./artifacts/link-calls-phone).
set -euo pipefail
OUT=${OUT:-$(pwd)/artifacts/link-calls-phone}
source "$(dirname "$0")/link_phone_lib.sh"
DIALED=5550199

[[ $SERIAL == emulator-* ]] || fail "dials a number: run it on an emulator, not on $SERIAL"
[[ -n ${LINK_PHONE_LOCKED:-} ]] || exec env LINK_PHONE_LOCKED=1 flock --close /tmp/andromeda-link-phone.lock "$0" "$@"
rm -rf "$OUT"
mkdir -p "$OUT"
RUNTIME=$(mktemp -d /tmp/link-calls-phone.XXXX)
unseed() { adb shell content delete --uri content://call_log/calls --where "\"number LIKE '555014%' OR number='$DIALED'\""; }
trap 'adb shell input keyevent KEYCODE_ENDCALL; unseed; forget_phone; kill $(jobs -p) 2>/dev/null || true; wait 2>/dev/null; rm -rf "$RUNTIME"' EXIT
step() { printf '%s\n' "$*" | tee -a "$OUT/steps.txt"; }
[[ $(adb get-state 2>/dev/null) == device ]] || fail "no emulator at $SERIAL"
[[ -s $APK && -s $FIXTURE ]] || fail "no APKs at $APK and $FIXTURE"
[[ -x $BIN/andromeda-link ]] || fail "no $BIN/andromeda-link; cargo build -p link-desk"

# seed NUMBER NAME TYPE MINUTES_AGO DURATION: one call in the log; TYPE 1 incoming, 2 outgoing, 3 missed.
seed() {
  adb shell content insert --uri content://call_log/calls --bind "number:s:$1" --bind "name:s:'$2'" \
    --bind "type:i:$3" --bind "date:l:$(( $(now_ms) - $4 * 60000 ))" --bind "duration:i:$5" --bind new:i:1
}
unseed
seed 5550141 "E2E Missed" 3 5 0
seed 5550142 "E2E Incoming" 1 90 125
seed 5550143 "E2E Outgoing" 2 1500 40

private_bus
start_linkd
HOST=$(ip -4 route get 1.1.1.1 | grep -o 'src [0-9.]*' | cut -d' ' -f2)
install_app
pair_phone "$HOST:$PORT"
calls() { prop Calls; }
wait_for 30 "D-Bus Calls never listed the seeded calls: $(calls)" eval 'calls | grep -q "5550143"'
for want in "'5550141', 'E2E Missed'.*'missed'" "'5550142', 'E2E Incoming'.*'incoming'" "'5550143', 'E2E Outgoing'.*'outgoing'"; do
  calls | grep -q "$want" || fail "D-Bus Calls lacks $want: $(calls)"
done
step "D-Bus Calls holds the 3 seeded calls with their kinds"

QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=rhi QT_FORCE_STDERR_LOGGING=1 \
  "$BIN/andromeda-link" --page calls "$OUT" dial "$DIALED" > "$OUT/app.log" 2>&1 &
page_state() { grep -o 'PAGE state {.*}' "$OUT/app.log" | tail -1 | cut -d' ' -f3-; }
wait_for 60 "the Calls page never got ready: $(tail -3 "$OUT/app.log")" grep -q "PAGE saved" "$OUT/app.log"
wait_for 5 "no page state" eval '[[ -n $(page_state) ]]'
python3 -c 'import json, sys
s = json.loads(sys.argv[1])
assert s["ready"] and s["refused"] == "", s
seeded = [(n, k) for n, k in zip(s["numbers"], s["kinds"]) if n.startswith("555014")]
assert seeded == [("5550141", "missed"), ("5550142", "incoming"), ("5550143", "outgoing")], seeded' "$(page_state)" || fail "Calls page state: $(page_state)"
step "the Calls page lists them newest first: $(page_state)"
[[ -s $OUT/calls-light.png && -s $OUT/calls-dark.png ]] || fail "no calls-light.png or calls-dark.png"

wait_for 10 "the page did not dial: $(tail -3 "$OUT/app.log")" grep -q "PAGE dial true" "$OUT/app.log"
offhook() { adb shell dumpsys telephony.registry | grep -q "mCallState=2"; }
wait_for 20 "the emulator never went off-hook for $DIALED" offhook
step "the emulator placed the call to $DIALED"
adb shell input keyevent KEYCODE_ENDCALL
wait_for 30 "the page never listed the outgoing call to $DIALED: $(page_state)" eval \
  'page_state | python3 -c "import json, sys; s = json.load(sys.stdin); assert s[\"numbers\"][0] == \"$DIALED\" and s[\"kinds\"][0] == \"outgoing\"" 2>/dev/null'
step "the ended call reached the page: $(page_state)"
echo PASS
