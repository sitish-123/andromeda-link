#!/usr/bin/env bash
# Take a photo from the Qt app's Photos page with the emulator's camera, through a private andromeda-linkd. Proves:
# 1. The page's camera button opens the phone's camera (Link may draw over apps, so it starts at once).
# 2. After the shot (KEYCODE_CAMERA, then the camera's Done), the JPEG lands in Pictures/Phone on this computer and
#    is handed to the desktop's opener (offscreen Qt has none; it logs the openDocument request instead).
# Needs ANDROID_SERIAL (the emulator, whose virtual camera works), the debug APKs, and the app built (cargo build -p
# link-desk). Run under flock /tmp/link-emulator.lock. Writes steps.txt, app.log, linkd.log, ui-*.xml, the photo, and
# photos-light.png / photos-dark.png to $OUT (default ./artifacts/link-camera-phone).
set -euo pipefail
OUT=${OUT:-$(pwd)/artifacts/link-camera-phone}
source "$(dirname "$0")/link_phone_lib.sh"

[[ -n ${LINK_PHONE_LOCKED:-} ]] || exec env LINK_PHONE_LOCKED=1 flock --close /tmp/andromeda-link-phone.lock "$0" "$@"
rm -rf "$OUT"
mkdir -p "$OUT"
RUNTIME=$(mktemp -d /tmp/link-camera-phone.XXXX)
scan() { adb shell content call --method scan_volume --uri content://media --arg external_primary > /dev/null 2>&1 || true; }
trap 'adb shell "rm -f /sdcard/Pictures/Link/Link_*.jpg"; scan; adb shell input keyevent KEYCODE_HOME; forget_phone; kill $(jobs -p) 2>/dev/null || true; wait 2>/dev/null; rm -rf "$RUNTIME"' EXIT
step() { printf '%s\n' "$*" | tee -a "$OUT/steps.txt"; }
[[ $(adb get-state 2>/dev/null) == device ]] || fail "no phone at $SERIAL"
[[ -s $APK && -s $FIXTURE ]] || fail "no APKs at $APK and $FIXTURE"
[[ -x $BIN/andromeda-link ]] || fail "no $BIN/andromeda-link; cargo build -p link-desk"

# Pictures in the run's folder.
mkdir -p "$RUNTIME/config" "$RUNTIME/pictures"
printf 'XDG_PICTURES_DIR="%s"\n' "$RUNTIME/pictures" > "$RUNTIME/config/user-dirs.dirs"

private_bus
start_linkd
HOST=$(ip -4 route get 1.1.1.1 | grep -o 'src [0-9.]*' | cut -d' ' -f2)
install_app
adb shell appops set "$PACKAGE" SYSTEM_ALERT_WINDOW allow
pair_phone "$HOST:$PORT"
phone_switches browse
link SetGrant "$ID" browse true > /dev/null
# The page is ready with at least one photo on the phone.
magick -size 64x64 xc:teal "$RUNTIME/seed.jpg"
adb shell mkdir -p /sdcard/Pictures/Link
adb push -q "$RUNTIME/seed.jpg" /sdcard/Pictures/Link/Link_seed.jpg
scan

XDG_CONFIG_HOME=$RUNTIME/config \
  QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=rhi QT_FORCE_STDERR_LOGGING=1 \
  "$BIN/andromeda-link" --page photos "$OUT" camera now > "$OUT/app.log" 2>&1 &
wait_for 60 "the page never asked for a photo: $(tail -3 "$OUT/app.log")" grep -q "PAGE camera true" "$OUT/app.log"
camera_up() { adb shell dumpsys window | grep -E "mCurrentFocus|mFocusedApp" | grep -qi camera; }
wait_for 30 "the camera never opened" camera_up
step "the camera opened on the phone"

# First runs of the camera ask about locations and tips; then the shutter key takes the shot and Done hands it back.
shot=""
for try in $(seq 40); do
  ui "camera-$try"
  for text in "No thanks" "NEXT" "Next" "Got it" "While using the app" "Only this time" "Don't allow"; do
    at=$(bounds "camera-$try" "$text")
    [[ -z $at ]] || { adb shell input tap $at; continue 2; }
  done
  at=$(bounds "camera-$try" Done)
  [[ -n $at ]] || at=$(bounds "camera-$try" "*done*")
  if [[ -n $at ]]; then adb shell input tap $at; break; fi
  if [[ -z $shot ]] && [[ -n $(bounds "camera-$try" "Shutter") ]]; then
    adb shell input keyevent KEYCODE_CAMERA
    shot=1
  fi
  camera_up || break
  sleep 1
done
[[ -n $shot ]] || fail "the camera never showed its shutter"
step "took the shot and tapped Done"

photo() { ls "$RUNTIME/pictures/Phone/"*.jpg 2>/dev/null | head -1; }
wait_for 60 "no photo in Pictures/Phone: $(tail -3 "$OUT/app.log")" eval '[[ -n $(photo) ]]'
file -b "$(photo)" | grep -q "JPEG image" || fail "$(photo) is not a JPEG: $(file -b "$(photo)")"
cp "$(photo)" "$OUT/"
step "$(basename "$(photo)") arrived in Pictures/Phone, $(stat -c %s "$(photo)") bytes"
wait_for 20 "the app never opened the photo: $(tail -3 "$OUT/app.log")" grep -qF "openDocument() for 'file://$(photo)'" "$OUT/app.log"
step "the app opened it"
echo PASS
