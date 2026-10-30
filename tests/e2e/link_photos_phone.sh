#!/usr/bin/env bash
# Photo thumbnails from a real phone's media store, through a private andromeda-linkd that the phone reaches through a
# clean UDP relay on a port the firewall lets in. Proves:
# 1. A JPEG whose EXIF orientation says "turn 90 degrees" comes back upright: the red band stored at its left edge is
#    on top, and the thumbnail is 480 x 640 for a 640 px request.
# 2. A 1080 x 2376 screenshot comes back 291 x 640, scaled exactly, not subsampled by a whole number.
# Needs ANDROID_SERIAL, the debug APKs, and the phone on the same network as this machine. Keeps the phone's other
# pairings, and removes the run's desktop and pictures after. Holds /tmp/andromeda-link-phone.lock, as other phone runs
# do. Writes steps.txt, the thumbnails, linkd.log, and signals.txt to $OUT (default ./artifacts/link-photos-phone).
set -euo pipefail
OUT=${OUT:-$(pwd)/artifacts/link-photos-phone}
source "$(dirname "$0")/link_phone_lib.sh"
RELAY_PORT=${RELAY_PORT:-48307}
ALBUM=LinkPhotosE2E

[[ -n ${LINK_PHONE_LOCKED:-} ]] || exec env LINK_PHONE_LOCKED=1 flock --close /tmp/andromeda-link-phone.lock "$0" "$@"
rm -rf "$OUT"
mkdir -p "$OUT"
RUNTIME=$(mktemp -d /tmp/link-photos-phone.XXXX)
scan() { adb shell content call --method scan_volume --uri content://media --arg external_primary > /dev/null 2>&1 || true; }
trap 'adb shell rm -rf /sdcard/Pictures/$ALBUM; scan; forget_phone; kill $(jobs -p) 2>/dev/null || true; wait 2>/dev/null; rm -rf "$RUNTIME"' EXIT
step() { printf '%s\n' "$*" | tee -a "$OUT/steps.txt"; }
[[ $(adb get-state 2>/dev/null) == device ]] || fail "no phone at $SERIAL"
[[ -s $APK && -s $FIXTURE ]] || fail "no APKs at $APK and $FIXTURE"

# Stored 1600 x 1200 with a red band at the left; EXIF orientation 6 shows it 1200 x 1600 with the band on top.
magick -size 1600x1200 xc:white -fill red -draw "rectangle 0,0 300,1199" "$RUNTIME/turned.jpg"
exiv2 -M"set Exif.Image.Orientation Short 6" "$RUNTIME/turned.jpg"
magick -size 1080x2376 gradient:white-navy "$RUNTIME/screenshot.png"

private_bus
start_linkd
HOST=$(ip -4 route get 1.1.1.1 | grep -o 'src [0-9.]*' | cut -d' ' -f2)
echo clean > "$RUNTIME/impair"
python3 "$ROOT/tests/e2e/udp_impair.py" "$RELAY_PORT" "$PORT" "$RUNTIME/impair" "$RUNTIME/relay.tsv" &
install_app
pair_phone "$HOST:$RELAY_PORT"
phone_switches browse

adb shell mkdir -p "/sdcard/Pictures/$ALBUM"
adb push -q "$RUNTIME/turned.jpg" "$RUNTIME/screenshot.png" "/sdcard/Pictures/$ALBUM/"
scan
listed() { link ListFiles "'$ID'" "'/Photos/$ALBUM'" 2>/dev/null | grep -q screenshot.png; }
wait_for 30 "the album never listed the pictures" listed
step "album $ALBUM lists $(link ListFiles "'$ID'" "'/Photos/$ALBUM'")"

# thumb NAME: the 640 px thumbnail of NAME into $OUT/NAME.png; prints its size.
thumb() {
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --json=short call org.andromeda.Link1 /org/andromeda/Link1 \
    org.andromeda.Link1 Thumbnail ssq "$ID" "/Photos/$ALBUM/$1" 640 > "$RUNTIME/reply.json" || fail "Thumbnail $1"
  python3 -c 'import json,sys; open(sys.argv[2],"wb").write(bytes(json.load(open(sys.argv[1]))["data"][0]))' \
    "$RUNTIME/reply.json" "$OUT/$1.png"
  magick identify -format '%wx%h' "$OUT/$1.png"
}

size=$(thumb turned.jpg)
[[ $size == 480x640 ]] || fail "the turned JPEG's thumbnail is $size, not 480x640"
top=$(magick "$OUT/turned.jpg.png" -crop 480x60+0+0 -resize 1x1\! -format '%[fx:int(255*r)] %[fx:int(255*g)]' info:)
bottom=$(magick "$OUT/turned.jpg.png" -crop 480x60+0+580 -resize 1x1\! -format '%[fx:int(255*g)]' info:)
[[ ${top% *} -gt 200 && ${top#* } -lt 60 && $bottom -gt 200 ]] || fail "the band is not on top: top rg $top, bottom g $bottom"
step "turned.jpg: $size, red band on top (top rg $top, bottom g $bottom)"

size=$(thumb screenshot.png)
[[ $size == 291x640 ]] || fail "the screenshot's thumbnail is $size, not 291x640"
step "screenshot.png: $size"
echo PASS
