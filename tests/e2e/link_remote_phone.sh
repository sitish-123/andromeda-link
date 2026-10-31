#!/usr/bin/env bash
# Remote control on a real phone or emulator: a headless Andromeda session with two outputs, its ScreenCast portal
# (xdg-desktop-portal-andromeda, whose chooser shares every output and window), PipeWire with a 440 Hz tone on the
# default output, andromeda-link-session, and a private andromeda-linkd the phone reaches through a UDP relay. Proves:
# 1. With the input grant, the Device page offers Remote control, and the viewer shows the desktop's picture.
# 2. The menu lists the shared monitors and window; picking the second output, where a fullscreen window that logs its
#    input opened, streams that output instead.
# 3. Direct touch: an adb tap in the picture's centre reaches that window as pointer motion near the output's centre
#    and a left button press and release.
# 4. Sound crosses: link-session counts the units it sent when Disconnect ends the stream, and the phone plays them.
# Needs ANDROID_SERIAL, the debug APKs, a built Andromeda desktop (the superproject's compositor, shell and portal
# build-debug), and the phone on this machine's network. Writes steps.txt, viewer.png, menu.png, viewer-2.png, window.jsonl,
# linkd.log, session.log, and phone.log to $OUT (default ./artifacts/link-remote-phone).
set -euo pipefail
OUT=${OUT:-$(pwd)/artifacts/link-remote-phone}
source "$(dirname "$0")/link_phone_lib.sh"
DESKTOP=${DESKTOP_ROOT:-$(cd "$ROOT/.." && pwd)}
ANDROMEDA=$DESKTOP/compositor/build-debug/andromeda
NOCTALIA=$DESKTOP/shell/build-debug/noctalia
PORTAL=$DESKTOP/portal/build-debug/xdg-desktop-portal-andromeda
RELAY_PORT=${RELAY_PORT:-48311}

[[ -n ${LINK_PHONE_LOCKED:-} ]] || exec env LINK_PHONE_LOCKED=1 flock --close /tmp/andromeda-link-phone.lock "$0" "$@"
rm -rf "$OUT"
mkdir -p "$OUT"
RUNTIME=$(mktemp -d /tmp/link-remote-phone.XXXX)
trap 'forget_phone; kill $(jobs -p) 2>/dev/null || true; wait 2>/dev/null; rm -rf "$RUNTIME"' EXIT
[[ $(adb get-state 2>/dev/null) == device ]] || fail "no phone at $SERIAL"
[[ -s $APK && -x $ANDROMEDA && -x $NOCTALIA && -x $PORTAL ]] || fail "build the APK and the desktop first"
step() { printf '%s\n' "$*" | tee -a "$OUT/steps.txt"; }
shot() { adb exec-out screencap -p > "$OUT/$1.png"; }
# "${SESSION[@]}" CMD...: inside the headless session, with a private HOME so nothing touches the user's config. An
# array, not a function: a background job is then the command itself, which the trap's kill reaches.
SESSION=(env -u DISPLAY -u XDG_CONFIG_HOME -u XDG_DATA_HOME -u XDG_CACHE_HOME HOME="$RUNTIME/home" XDG_RUNTIME_DIR="$RUNTIME"
  WAYLAND_DISPLAY=wayland-0 ANDROMEDA_SOCKET="$RUNTIME/andromeda-wayland-0.sock" XDG_CURRENT_DESKTOP=andromeda)

# The desktop: Andromeda with two headless outputs and the shell, as the superproject's E2Es boot it.
mkdir -p "$RUNTIME/home/.config/noctalia" "$RUNTIME/home/.config/xdg-desktop-portal-andromeda"
printf '[plugins]\nauto_update = "none"\n\n[shell]\nsetup_wizard_enabled = false\n' > "$RUNTIME/home/.config/noctalia/config.toml"
printf '[general]\nautostart = []\nshow_cheatsheet = false\n' > "$RUNTIME/andromeda.toml"
cat > "$RUNTIME/chooser" <<'PY'
#!/usr/bin/env python3
import json, sys
asked = json.loads(sys.stdin.readline())
picked = [{"kind": "monitor", "output": output["name"]} for output in asked["outputs"]]
picked += [{"kind": "window", "identifier": window["identifier"]} for window in asked["windows"]]
print(json.dumps({"selections": picked}))
PY
chmod +x "$RUNTIME/chooser"
printf '[screencast]\nchooser_cmd = "%s"\n' "$RUNTIME/chooser" > "$RUNTIME/home/.config/xdg-desktop-portal-andromeda/config.toml"
private_bus system
env -u WAYLAND_DISPLAY -u DISPLAY XDG_RUNTIME_DIR="$RUNTIME" WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 \
  WLR_HEADLESS_OUTPUTS=2 "$ANDROMEDA" -c "$RUNTIME/andromeda.toml" > "$OUT/andromeda.log" 2>&1 &
wait_for 5 "andromeda did not start" test -S "$RUNTIME/wayland-0"
"${SESSION[@]}" "$NOCTALIA" > "$OUT/noctalia.log" 2>&1 &
wait_for 10 "the shell did not start" eval '"${SESSION[@]}" "$NOCTALIA" msg settings-close > /dev/null 2>&1'

# Sound: PipeWire with its Pulse server, a null sink as the default output, and a tone playing into it.
# The real BlueZ is on the system bus: a bluez monitor here takes headphones from the user's WirePlumber.
mkdir -p "$RUNTIME/pw-config/wireplumber/wireplumber.conf.d"
cat > "$RUNTIME/pw-config/wireplumber/wireplumber.conf.d/90-e2e.conf" << 'CONF'
wireplumber.profiles = { main = { monitor.alsa = disabled, monitor.alsa-midi = disabled, monitor.bluez = disabled,
  monitor.bluez-midi = disabled, monitor.v4l2 = disabled, monitor.libcamera = disabled } }
CONF
"${SESSION[@]}" XDG_CONFIG_HOME="$RUNTIME/pw-config" pipewire > "$OUT/pipewire.log" 2>&1 &
wait_for 5 "PipeWire did not start" test -S "$RUNTIME/pipewire-0"
"${SESSION[@]}" XDG_CONFIG_HOME="$RUNTIME/pw-config" wireplumber > "$OUT/wireplumber.log" 2>&1 &
"${SESSION[@]}" XDG_CONFIG_HOME="$RUNTIME/pw-config" pipewire-pulse > "$OUT/pipewire-pulse.log" 2>&1 &
wait_for 10 "WirePlumber did not come up" eval '"${SESSION[@]}" wpctl status > /dev/null 2>&1'
sink() { "${SESSION[@]}" pw-cli ls Node | grep -q 'node.name = "e2e-sink"'; }
# WirePlumber answers before it loads its policy; a sink made sooner is not linked, so it is made until it shows.
make_sink() {
  sink || "${SESSION[@]}" pw-cli create-node adapter "{ factory.name=support.null-audio-sink node.name=e2e-sink \
    media.class=Audio/Sink object.linger=true audio.position=[FL FR] }" >> "$OUT/pw-cli.log" 2>&1
  sleep 0.5
  sink
}
wait_for 10 "no null sink: $(cat "$OUT/pw-cli.log" 2> /dev/null)" make_sink
SINK=$("${SESSION[@]}" pw-cli ls Node | awk '/^\tid / {id = $2} /node.name = "e2e-sink"/ {print id}' | tr -d ,)
"${SESSION[@]}" wpctl set-default "$SINK"
ffmpeg -v error -f lavfi -i "sine=frequency=440:duration=600" -ac 2 "$RUNTIME/tone.wav"
"${SESSION[@]}" pw-play --target e2e-sink "$RUNTIME/tone.wav" &

# The portal, a fullscreen window that logs its input, then Link's daemon and session.
"${SESSION[@]}" "$PORTAL" > "$OUT/portal.log" 2>&1 &
wait_for 5 "the Andromeda portal did not start" eval 'gdbus introspect --session -d org.freedesktop.impl.portal.desktop.andromeda -o /org/freedesktop/portal/desktop > /dev/null 2>&1'
"${SESSION[@]}" /usr/lib/xdg-desktop-portal -r > "$OUT/xdg-desktop-portal.log" 2>&1 &
wait_for 10 "xdg-desktop-portal did not start" eval 'gdbus introspect --session -d org.freedesktop.portal.Desktop -o /org/freedesktop/portal/desktop 2> /dev/null | grep -q ScreenCast'
"${SESSION[@]}" GDK_BACKEND=wayland python3 "$DESKTOP/tests/e2e/link_input_window.py" "$OUT/window.jsonl" 2> "$OUT/window.log" &
wait_for 10 "the input window did not open" grep -q '"ready"' "$OUT/window.jsonl"
start_linkd
"${SESSION[@]}" RUST_LOG=info,zbus=warn "$BIN/andromeda-link-session" > "$OUT/session.log" 2>&1 &
wait_for 15 "andromeda-link-session did not start" grep -q running "$OUT/session.log"
grep -q 'can: .*screen' "$OUT/session.log" || fail "link-session cannot share the screen: $(grep can: "$OUT/session.log")"
HOST=$(ip -4 route get 1.1.1.1 | grep -o 'src [0-9.]*' | cut -d' ' -f2)
echo clean > "$RUNTIME/impair"
python3 "$ROOT/tests/e2e/udp_impair.py" "$RELAY_PORT" "$PORT" "$RUNTIME/impair" "$OUT/relay.tsv" &
step "desktop: two headless outputs, the ScreenCast portal, a tone on the default output, link-session can share"

install_app
adb shell wm dismiss-keyguard
pair_phone "$HOST:$RELAY_PORT"
link SetGrant "'$ID'" "'input'" true > /dev/null
adb logcat -c
adb shell am start -W --activity-clear-task -n "$PACKAGE/.MainActivity" > /dev/null
tap_text home "Features and pairing"
for _ in $(seq 8); do
  ui device
  at=$(bounds device "Remote control")
  [[ -z $at ]] || break
  adb shell input swipe 540 1700 540 900 400
done
[[ -n $at ]] || fail "the Device page offers no Remote control"
adb shell input tap $at
wait_for 60 "the desktop never showed its screen" grep -q 'showing the screen at' "$OUT/linkd.log"
# Android explains full screen once per app; Got it closes that.
ui immersive
at=$(bounds immersive "Got it")
[[ -z $at ]] || adb shell input tap $at
sleep 2 # real time: the first frames decode and paint
shot viewer
adb shell dumpsys window | grep mCurrentFocus | grep -q "$PACKAGE" || fail "Link is not in front: $(adb shell dumpsys window | grep mCurrentFocus)"
# Portrait fits the landscape picture across the middle: the desktop there, the viewer's black above it.
read -r W H <<< "$(magick identify -format '%w %h' "$OUT/viewer.png")"
# mean Y: the brightness of a band a tenth of the screen high from Y down.
mean() { magick "$OUT/viewer.png" -alpha off -crop "${W}x$(( H / 10 ))+0+$1" +repage -colorspace gray -format '%[fx:mean]' info:; }
MIDDLE=$(mean $(( H * 45 / 100 ))) TOP=$(mean 0)
awk -v m="$MIDDLE" -v t="$TOP" 'BEGIN {exit !(m > 0.05 && t < 0.02)}' || fail "the viewer shows no picture (middle $MIDDLE, top $TOP)"
step "viewer: the desktop's picture shows ($(grep -o 'showing the screen at [0-9x]*' "$OUT/linkd.log" | head -1))"

# The menu: every shared source; the input window opened on the second output, so that one is picked.
ui menu-button
adb shell input tap $(bounds menu-button "Remote control menu")
wait_for 10 "the menu did not open" eval 'ui menu; [[ -n $(bounds menu Disconnect) ]]'
shot menu
for source in HEADLESS-1 HEADLESS-2 "Window 1"; do
  [[ -n $(bounds menu "$source") ]] || fail "the menu does not list $source"
done
adb shell input tap $(bounds menu HEADLESS-2)
wait_for 15 "picking the second output did not stream it" grep -q 'showing HEADLESS-2' "$OUT/session.log"
sleep 2 # real time: the new stream's first frames paint
shot viewer-2
step "menu: lists HEADLESS-1, HEADLESS-2, and Window 1; picking HEADLESS-2 streams it"

# Direct touch: a tap in the middle of the screen, which is the middle of the fitted picture.
adb shell input tap $(( W / 2 )) $(( H / 2 ))
wait_for 10 "the tap did not press the left button in the window" grep -q '"press", "button": 1' "$OUT/window.jsonl"
wait_for 5 "the tap did not release the left button" grep -q '"release", "button": 1' "$OUT/window.jsonl"
MOTION=$(grep '"motion"' "$OUT/window.jsonl" | tail -1)
python3 - "$MOTION" <<'PY' || fail "the pointer is not near the output's centre: $MOTION"
import json, sys
motion = json.loads(sys.argv[1])
assert abs(motion["x"] - 640) < 40 and abs(motion["y"] - 360) < 40, motion
PY
step "direct touch: a tap at the picture's centre clicked at $MOTION"

sleep 2 # real time: sound crosses while the stream runs
ui menu-button
adb shell input tap $(bounds menu-button "Remote control menu")
tap_text disconnect Disconnect
wait_for 10 "Disconnect did not end the stream" grep -q 'remote control ended' "$OUT/session.log"
SOUND=$(grep -o 'remote control ended: [0-9]* frames, [0-9]* sound packets' "$OUT/session.log" | tail -1)
[[ $SOUND =~ ([0-9]+)\ sound ]] && (( BASH_REMATCH[1] > 20 )) || fail "too little sound crossed: $SOUND"
adb logcat -d > "$OUT/phone.log"
grep -q "playing the desktop's sound" "$OUT/phone.log" || fail "the phone did not play the sound"
step "sound: $SOUND; the phone played it"
echo "PASS; artifacts: $OUT"
