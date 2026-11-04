#!/usr/bin/env bash
# Wi-Fi Direct with real radios: the phone stays on its Wi-Fi, this laptop leaves its own, so Bluetooth is the only
# path until the phone forms a Wi-Fi Direct group that the laptop joins through the real NetworkManager.
#
# Proves:
# 1. With the laptop off its Wi-Fi, the phone's session comes over Bluetooth.
# 2. An 8 MiB SendFiles creates a group the phone owns instead of the hotspot: linkd joins its DIRECT- SSID with the
#    passphrase on the idle Wi-Fi device, the session moves to IP on the group's subnet (192.168.49.0/24), no
#    HotspotRequested is signalled, and the file arrives with a matching SHA-256. results.jsonl records the join path.
# Takes the laptop off its Wi-Fi for a few minutes. Stops the user's andromeda-linkd for the run (it holds Link's
# Bluetooth profile and port 4717) and starts it again after, and brings the laptop's Wi-Fi back even on failure.
# Writes results.jsonl, linkd.log, signals.txt, nm.txt, logcat.txt to $OUT (default ./artifacts/link-wifi-direct-phone).
set -euo pipefail
OUT=${OUT:-$(pwd)/artifacts/link-wifi-direct-phone}
source "$(dirname "$0")/link_phone_lib.sh"

[[ -n ${LINK_PHONE_LOCKED:-} ]] || exec env LINK_PHONE_LOCKED=1 flock --close /tmp/andromeda-link-phone.lock "$0" "$@"
rm -rf "$OUT"
mkdir -p "$OUT"
RUNTIME=$(mktemp -d /var/tmp/link-wifi-direct-phone.XXXX)
HOME_WIFI=$(nmcli -t -f NAME,TYPE con show --active | grep ':802-11-wireless$' | head -1 | cut -d: -f1)
[[ -n $HOME_WIFI ]] || fail "the laptop is not on Wi-Fi"
USER_LINKD=$(systemctl --user is-active andromeda-linkd || true)
restore() {
  forget_phone
  kill $(jobs -p) 2>/dev/null || true
  wait 2>/dev/null
  nmcli -t -f NAME con show --active | grep -qxF "$HOME_WIFI" || nmcli con up id "$HOME_WIFI" > /dev/null || true
  [[ $USER_LINKD != active ]] || systemctl --user start andromeda-linkd
  rm -rf "$RUNTIME"
}
trap restore EXIT
[[ $(adb get-state 2>/dev/null) == device ]] || fail "no phone at $SERIAL"
[[ -s $APK && -s $FIXTURE ]] || fail "no APKs at $APK and $FIXTURE"
[[ $(adb get-devpath) == usb* ]] || fail "adb must run over USB: the laptop leaves its Wi-Fi"

systemctl --user stop andromeda-linkd 2>/dev/null || true
private_bus
start_linkd
wait_for 10 "BlueZ did not take Link's profile" grep -q 'Bluetooth: listening on' "$OUT/linkd.log"
( nmcli monitor | stamp > "$OUT/nm.txt" ) &

install_app
adb logcat -c
adb logcat -v time > "$OUT/logcat.txt" 2>&1 &
HOST=$(ip -4 route get 1.1.1.1 | grep -o 'src [0-9.]*' | cut -d' ' -f2)
pair_phone "$HOST:$PORT"
record "{\"step\":\"paired\",\"device\":\"$ID\",\"home_wifi\":\"$HOME_WIFI\"}"

on_bluetooth() { grep "connected over" "$OUT/linkd.log" | tail -1 | grep -q "over Bluetooth"; }
on_group() { grep "connected over" "$OUT/linkd.log" | tail -1 | grep -q "over IP from .*192\.168\.49\."; }
delivered() {
  [[ $(adb shell sha256sum "/sdcard/Download/$1" 2>/dev/null | cut -d' ' -f1) == $(sha256sum < "$2" | cut -d' ' -f1) ]]
}

T0=$(now_ms)
nmcli con down id "$HOME_WIFI" > /dev/null
wait_for 120 "the phone did not come over Bluetooth" on_bluetooth
record "{\"step\":\"rfcomm-session\",\"after_wifi_off_ms\":$(( $(now_ms) - T0 ))}"

adb shell rm -f /sdcard/Download/direct-8m.bin
head -c 8388608 /dev/urandom > "$RUNTIME/direct-8m.bin"
link SetAutoAccept "'$ID'" true > /dev/null || true
T0=$(now_ms)
python3 "$ROOT/tests/e2e/link_send_files.py" "$ID" "$RUNTIME/direct-8m.bin" > "$RUNTIME/transfer" || fail "SendFiles 8 MiB"
wait_for 90 "the session did not move to a Wi-Fi Direct group: $(grep -E 'Direct|hotspot' "$OUT/linkd.log" | tail -2)" on_group
JOINED=$(( $(now_ms) - T0 ))
# Without auto-accept on the phone, its offer notification takes the tap.
arrived=""
for _ in $(seq 20); do delivered direct-8m.bin "$RUNTIME/direct-8m.bin" && { arrived=1; break; }; sleep 1; done
if [[ -z $arrived ]]; then
  wait_for 30 "no offer notification" eval 'adb shell dumpsys notification --noredact | grep -q "wants to send"'
  adb shell cmd statusbar expand-notifications
  tap_text offer "*wants to send*"
  tap_text accept Accept
  adb shell cmd statusbar collapse
  wait_for 120 "the 8 MiB file did not arrive intact" delivered direct-8m.bin "$RUNTIME/direct-8m.bin"
fi
! grep -q 'member=HotspotRequested' "$OUT/signals.txt" || fail "the phone offered its hotspot instead of a group"
JOIN_PATH=leave-wifi
grep -q "the Wi-Fi device is idle" "$OUT/linkd.log" && JOIN_PATH=idle-wifi
grep -q "added linkwifi0" "$OUT/linkd.log" && JOIN_PATH=second-interface
record "{\"step\":\"8mib-over-wifi-direct\",\"path\":\"$JOIN_PATH\",\"join_ms\":$JOINED,\"done_ms\":$(( $(now_ms) - T0 ))}"
adb shell rm -f /sdcard/Download/direct-8m.bin
cat "$OUT/results.jsonl"
echo "PASS; artifacts: $OUT"
