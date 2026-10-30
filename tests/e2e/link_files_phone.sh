#!/usr/bin/env bash
# Browsing writes from the Qt app's Files page against the emulator, through a private andromeda-linkd. Proves:
# 1. With All files access and the browse switch on, Send files on /Storage/Download copies a 150 KB file (4 writes)
#    onto the phone, byte for byte, and the page lists it.
# 2. Delete on that file removes it from the phone, and the page no longer lists it.
# Needs ANDROID_SERIAL (the emulator), the debug APKs, and the app built (cargo build -p link-desk). Run under flock
# /tmp/link-emulator.lock. Writes steps.txt, app.log, linkd.log, and files-*.png to $OUT (default
# ./artifacts/link-files-phone).
set -euo pipefail
OUT=${OUT:-$(pwd)/artifacts/link-files-phone}
source "$(dirname "$0")/link_phone_lib.sh"
NAME=e2e-copy-$$.txt

[[ -n ${LINK_PHONE_LOCKED:-} ]] || exec env LINK_PHONE_LOCKED=1 flock --close /tmp/andromeda-link-phone.lock "$0" "$@"
rm -rf "$OUT"
mkdir -p "$OUT"
RUNTIME=$(mktemp -d /tmp/link-files-phone.XXXX)
trap 'adb shell rm -f /sdcard/Download/$NAME; forget_phone; kill $(jobs -p) 2>/dev/null || true; wait 2>/dev/null; rm -rf "$RUNTIME"' EXIT
step() { printf '%s\n' "$*" | tee -a "$OUT/steps.txt"; }
[[ $(adb get-state 2>/dev/null) == device ]] || fail "no phone at $SERIAL"
[[ -s $APK && -s $FIXTURE ]] || fail "no APKs at $APK and $FIXTURE"
[[ -x $BIN/andromeda-link ]] || fail "no $BIN/andromeda-link; cargo build -p link-desk"
head -c 150000 /dev/urandom > "$RUNTIME/$NAME"

private_bus
start_linkd
HOST=$(ip -4 route get 1.1.1.1 | grep -o 'src [0-9.]*' | cut -d' ' -f2)
install_app
adb shell appops set "$PACKAGE" MANAGE_EXTERNAL_STORAGE allow
pair_phone "$HOST:$PORT"
phone_switches browse
storage() { link ListFiles "'$ID'" "'/'" 2>/dev/null | grep -q Storage; }
wait_for 30 "the phone's roots never showed Storage" storage
link SetGrant "$ID" browse true > /dev/null

QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=rhi QT_FORCE_STDERR_LOGGING=1 QML_XHR_ALLOW_FILE_READ=1 \
  "$BIN/andromeda-link" --page files "$OUT" cd /Storage/Download upload "$RUNTIME/$NAME" await "$RUNTIME/checked" \
  shot files-copied remove "/Storage/Download/$NAME" > "$OUT/app.log" 2>&1 &
page_state() { grep -o 'PAGE state {.*}' "$OUT/app.log" | tail -1 | cut -d' ' -f3-; }
listed() { page_state | python3 -c 'import json, sys; s = json.load(sys.stdin); sys.exit(sys.argv[1] not in s["names"])' "$NAME" 2>/dev/null; }
wait_for 60 "the page never copied the file: $(tail -3 "$OUT/app.log")" grep -q "PAGE upload true" "$OUT/app.log"

on_phone() { [[ $(adb shell "md5sum /sdcard/Download/$NAME 2>/dev/null" | cut -d' ' -f1) == $(md5sum < "$RUNTIME/$NAME" | cut -d' ' -f1) ]]; }
wait_for 60 "the copy never reached the phone whole" on_phone
step "$NAME (150000 bytes) is on the phone, same md5"
wait_for 30 "the page never listed $NAME: $(page_state)" listed
step "the Files page lists it in $(page_state | python3 -c 'import json, sys; print(json.load(sys.stdin)["path"])')"
echo checked > "$RUNTIME/checked"

wait_for 30 "the page never deleted the file: $(tail -3 "$OUT/app.log")" grep -q "PAGE remove true" "$OUT/app.log"
gone() { ! adb shell "ls /sdcard/Download/$NAME" > /dev/null 2>&1; }
wait_for 30 "$NAME is still on the phone" gone
step "Delete removed it from the phone"
wait_for 30 "the page still lists $NAME: $(page_state)" eval '! listed'
step "the Files page no longer lists it"
[[ -s $OUT/files-light.png && -s $OUT/files-copied-light.png ]] || fail "no files-light.png or files-copied-light.png"
echo PASS
