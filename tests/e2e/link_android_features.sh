#!/usr/bin/env bash
# Slice C on the Android emulator against a private andromeda-linkd (own dbus-daemon, own XDG_STATE_HOME). Proves:
# with notification access granted, a notification posted with `cmd notification post` reaches D-Bus
# NotificationPosted with its app name and icon, and a D-Bus dismissal cancels it on the phone and comes back as
# NotificationRemoved; a chat notification (the fixture app) is mirrored with its RemoteInput reply action, a reply sent
# with D-Bus NotificationAction reaches the RemoteInput, and the updated conversation comes back to D-Bus. A media
# session on the phone (the fixture app) becomes the MPRIS player andromeda_link_<device> with its title and artwork,
# and playerctl's pause, next, and seek reach that session; a desktop MPRIS test player shows on the app's Media
# screen, and its Pause button reaches it as PlayPause. D-Bus Ring rings the phone on the alarm stream at full volume
# while Do Not Disturb (priority, which lets alarms through) stays as the user set it, and a stop puts the volume back; the app's Ring desktop reaches D-Bus as
# RingRequested, and once the desktop reports ringing, Stop ringing stops it. `adb emu gsm call` from a contact reaches
# D-Bus Call with the contact's name and number, pauses the desktop's playing player, D-Bus CallAction mutes the ringer
# and declines the call, and the player resumes; a call from an unknown number carries only the number and goes idle
# when the caller hangs up (`adb emu gsm cancel`).
# Writes screenshots, results.jsonl, results.json, signals.txt, notifications.txt, linkd.log to $OUT
# (default ./artifacts/link-android-features). Needs the debug APK and a running emulator; run under
# flock /tmp/link-emulator.lock, since the emulator is shared.
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
OUT=${OUT:-$(pwd)/artifacts/link-android-features}
BIN=${LINK_BIN:-$ROOT/target/debug}
ADB=${ADB:-$HOME/Android/Sdk/platform-tools/adb}
MAESTRO=${MAESTRO:-$HOME/.maestro/bin/maestro}
SERIAL=${ANDROID_SERIAL:-emulator-5554}
APK=${APK:-$ROOT/apps/android/app/build/outputs/apk/debug/app-debug.apk}
FIXTURE_APK=${FIXTURE_APK:-$ROOT/apps/android/fixture/build/outputs/apk/debug/fixture-debug.apk}
SECOND_APK=${SECOND_APK:-$ROOT/apps/android/fixture/build/outputs/apk/second/fixture-second.apk}
FLOWS=$ROOT/apps/android/maestro
PACKAGE=org.andromeda.link

rm -rf "$OUT"
mkdir -p "$OUT"
# A fresh directory per run: dbus-daemon unlinks its socket path on exit, so a reused path would race the old one.
RUNTIME=$(mktemp -d /tmp/link-android-features.XXXX)
trap 'kill $(jobs -p) 2>/dev/null || true; wait 2>/dev/null; rm -rf "$RUNTIME"' EXIT
fail() { echo "FAIL: $*" >&2; "$ADB" -s "$SERIAL" logcat -d > "$OUT/logcat.txt" 2>&1 || true; exit 1; }
record() { printf '%s\n' "$1" >> "$OUT/results.jsonl"; }
wait_for() {
  local tries=$(( $1 * 10 )) what=$2; shift 2
  for _ in $(seq "$tries"); do "$@" && return; sleep 0.1; done
  fail "$what"
}
adb() { "$ADB" -s "$SERIAL" "$@"; }
maestro() { "$MAESTRO" --device "$SERIAL" test -e OUT="$OUT" "$@" >> "$OUT/maestro.log" 2>&1 || fail "maestro $*"; }
screenshot() { adb exec-out screencap -p > "$OUT/$1.png"; }
notifications() { adb shell dumpsys notification --noredact > "$OUT/notifications.txt"; cat "$OUT/notifications.txt"; }

[[ $(adb get-state 2>/dev/null) == device ]] || fail "no emulator at $SERIAL"
[[ -s $APK ]] || fail "no APK at $APK"
[[ -s $FIXTURE_APK ]] || fail "no fixture APK at $FIXTURE_APK"

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
devices() { gdbus call --session -d org.andromeda.Link1 -o /org/andromeda/Link1 -m org.freedesktop.DBus.Properties.Get org.andromeda.Link1 Devices; }
connected() { devices | grep -q "', true)"; }
# One line per signal: gdbus monitor prints each with all its arguments.
signal() { grep -F "org.andromeda.Link1.$1 " "$OUT/signals.txt" | grep -F -- "$2" | tail -1; }
heard() { [[ -n $(signal "$1" "$2") ]]; }
posted() { notifications | grep -qF "$1"; }
gone() { ! notifications | grep -qF "$1"; }
# The notification key and the first reply action of the latest NotificationPosted matching $1.
posted_field() {
  signal NotificationPosted "$1" | python3 -c '
import re, sys
line = sys.stdin.read()
args = re.findall(r"'"'"'((?:[^'"'"'\\]|\\.)*)'"'"'", line)
replies = re.findall(r"\('"'"'([^'"'"']*)'"'"', '"'"'[^'"'"']*'"'"', true\)", line)
print({"key": args[1], "app": args[2], "reply": replies[0] if replies else ""}[sys.argv[1]])' "$2"
}

XDG_STATE_HOME=$RUNTIME/desktop "$BIN/andromeda-linkd" > "$OUT/linkd.log" 2>&1 &
for _ in $(seq 100); do link CancelPairing > /dev/null 2>&1 && break; sleep 0.05; done
link CancelPairing > /dev/null || fail "andromeda-linkd did not come up"
gdbus monitor --session -d org.andromeda.Link1 -o /org/andromeda/Link1 > "$OUT/signals.txt" 2>&1 &
wait_for 5 "gdbus monitor did not start" test -s "$OUT/signals.txt"

adb shell cmd statusbar collapse
adb shell input keyevent KEYCODE_HOME
adb uninstall "$PACKAGE" > /dev/null 2>&1 || true
adb install "$APK" > /dev/null || fail "installing $APK"
# As link_android_ui.sh: the scanner gets the camera, and no READ_LOGS kept from an earlier run asks to read logs.
adb shell pm grant "$PACKAGE" android.permission.CAMERA
adb shell pm revoke "$PACKAGE" android.permission.READ_LOGS
adb shell cmd notification allow_listener "$PACKAGE/$PACKAGE.notifications.MirrorService"
record '{"step":"installed","notification_access":true}'

URI=$(link StartPairing | sed -E "s/^\('[0-9]+', '([^']+)'\)$/\1/")
adb shell am start -W -a android.intent.action.VIEW -d "'$URI'" "$PACKAGE" > /dev/null
maestro "$FLOWS/pair.yaml"
wait_for 20 "D-Bus does not show the emulator connected: $(devices)" connected
ID=$(devices | grep -o "'[0-9a-f]\{32\}'" | head -1 | tr -d "'")
record "{\"step\":\"paired-and-connected\",\"device\":\"$ID\"}"

adb shell cmd notification post -S bigtext -t "'Build finished'" e2e-build "'All 42 checks passed'" > /dev/null
wait_for 20 "no NotificationPosted for the adb notification" heard NotificationPosted "All 42 checks passed"
KEY=$(posted_field "All 42 checks passed" key)
APP=$(posted_field "All 42 checks passed" app)
signal NotificationPosted "All 42 checks passed" | grep -q "\[byte 0x89, 0x50, 0x4e, 0x47" || fail "no PNG icon on the mirrored notification"
record "{\"step\":\"cmd-notification-mirrored\",\"key\":\"$KEY\",\"app\":\"$APP\",\"icon\":\"png\"}"

link NotificationDismiss "$ID" "$KEY" > /dev/null || fail "D-Bus NotificationDismiss"
wait_for 10 "the dismissal did not cancel the phone notification" gone "All 42 checks passed"
wait_for 10 "no NotificationRemoved after the dismissal" heard NotificationRemoved "$KEY"
record '{"step":"dismissed-from-desktop","phone":"cancelled","dbus":"NotificationRemoved"}'

# A fresh install: replacing the package cancels its notifications late, racing the reply below.
adb uninstall org.andromeda.link.fixture > /dev/null 2>&1 || true
adb uninstall org.andromeda.link.fixture.second > /dev/null 2>&1 || true
adb install "$FIXTURE_APK" > /dev/null || fail "installing $FIXTURE_APK"
adb shell pm grant org.andromeda.link.fixture android.permission.POST_NOTIFICATIONS
adb logcat -c
adb shell am start -W -n org.andromeda.link.fixture/.MessageActivity > /dev/null
# The key names the fresh install's uid, so a notification a previous run left behind cannot stand in for it.
FIXTURE_KEY="0|org.andromeda.link.fixture|7|null|$(adb shell pm list packages -U org.andromeda.link.fixture | sed 's/.*uid://' | tr -d '\r')"
wait_for 30 "no NotificationPosted for the chat message" heard NotificationPosted "$FIXTURE_KEY"
CHAT_KEY=$(posted_field "$FIXTURE_KEY" key)
REPLY=$(posted_field "$FIXTURE_KEY" reply)
[[ $(signal NotificationPosted "$FIXTURE_KEY") == *"Are we still on for dinner?"* ]] || fail "the chat text did not arrive"
[[ -n $REPLY ]] || fail "the chat notification carries no RemoteInput action: $(signal NotificationPosted "Are we still on for dinner?")"
link NotificationAction "$ID" "$CHAT_KEY" "$REPLY" "'Yes, leaving now'" > /dev/null || fail "D-Bus NotificationAction reply"
wait_for 20 "the RemoteInput did not receive the reply" eval 'adb logcat -d -s LinkFixture | grep -q "reply Yes, leaving now"'
wait_for 20 "the replied conversation did not come back to the desktop" heard NotificationPosted "Yes, leaving now"
adb shell cmd statusbar expand-notifications
sleep 2 # real time: the shade animates open
screenshot 1-reply-on-phone
adb shell cmd statusbar collapse
record "{\"step\":\"reply-through-remote-input\",\"action\":\"$REPLY\",\"text\":\"Yes, leaving now\"}"

adb shell am start -W -n org.andromeda.link.fixture/.PlayerActivity > /dev/null
PLAYER=andromeda_link_$ID
pc() { playerctl -p "$PLAYER" "$@" 2> /dev/null || true; }
fixture() { adb logcat -d -s LinkFixture | grep -q "$1"; }
wait_for 30 "the phone's media session never reached MPRIS: $(playerctl -l 2>&1)" eval '[[ $(pc metadata title) == "Emulator Song" ]]'
[[ $(pc metadata artist) == "The Emulators" && $(pc status) == Playing ]] || fail "MPRIS: $(pc metadata artist) $(pc status)"
ART=$(pc metadata mpris:artUrl)
[[ $ART == file://*.jpg && -s ${ART#file://} ]] || fail "no artwork file: $ART"
pc pause
wait_for 10 "the media session was not paused" fixture " pause"
wait_for 10 "MPRIS does not show the phone paused: $(pc status)" eval '[[ $(pc status) == Paused ]]'
pc next
wait_for 10 "the media session did not skip" fixture " next"
wait_for 10 "the next title did not reach MPRIS" eval '[[ $(pc metadata title) == "Emulator Song, part 2" ]]'
pc position 90
wait_for 10 "the media session was not seeked" fixture " seek 90000"
record "{\"step\":\"phone-media-on-mpris\",\"player\":\"$PLAYER\",\"commands\":[\"pause\",\"next\",\"seek\"],\"art\":\"jpeg\"}"

# A second media app that plays takes over the player; when it stops, the paused first app does not come back over it.
adb install "$SECOND_APK" > /dev/null || fail "installing $SECOND_APK"
adb shell am start -W -n org.andromeda.link.fixture.second/org.andromeda.link.fixture.PlayerActivity --es title "'Second Song'" > /dev/null
wait_for 20 "the second app's session did not become the player: $(pc metadata title)" eval '[[ $(pc metadata title) == "Second Song" && $(pc status) == Playing ]]'
pc pause
wait_for 10 "the pause did not reach the second app: $(pc status)" eval '[[ $(pc metadata title) == "Second Song" && $(pc status) == Paused ]]'
adb uninstall org.andromeda.link.fixture.second > /dev/null
wait_for 10 "the first app did not come back after the second left: $(pc metadata title)" eval '[[ $(pc metadata title) == "Emulator Song, part 2" ]]'
record '{"step":"two-phone-players","newest_playing":"Second Song","after_leave":"Emulator Song, part 2"}'

python3 "$ROOT/tests/e2e/mpris_test_player.py" "$OUT/player-calls.txt" &
# A stand-in link-session reports the desk's volume at 30%; the phone's Desktop volume bar needs the system grant.
link SetGrant "$ID" system true > /dev/null || fail "D-Bus SetGrant system"
python3 -c 'import dbus, time
link = dbus.Interface(dbus.SessionBus().get_object("org.andromeda.Link1", "/org/andromeda/Link1"), "org.andromeda.Link1")
link.SetSessionCapabilities(dbus.Array(["volume"], signature="s"))
link.SetDesktopStatus(dbus.UInt32(30), False)
time.sleep(3600)' &
adb shell am start -W -n "$PACKAGE/.MainActivity" > /dev/null
maestro "$FLOWS/media.yaml"
wait_for 10 "the app's Pause did not reach the desktop player" grep -qx PlayPause "$OUT/player-calls.txt"
wait_for 10 "the Position bar did not seek the desktop player near 100 s: $(tr "\n" " " < "$OUT/player-calls.txt")" \
  grep -qE "^SetPosition /org/e2e/track/1 (99|100)[0-9]{6}$" "$OUT/player-calls.txt"
wait_for 10 "the Player volume bar did not set 0.50: $(tr "\n" " " < "$OUT/player-calls.txt")" \
  grep -qx "Set Volume 0.50" "$OUT/player-calls.txt"
wait_for 10 "the Desktop volume bar sent no set-volume near 50: $(signal SystemCommand set-volume)" \
  eval '[[ $(signal SystemCommand set-volume) =~ uint32\ (4[5-9]|5[0-5]) ]]'
record '{"step":"desktop-media-on-phone","player":"E2E Player","commands":["PlayPause","seek by the bar","volume by the bar","desktop volume by the bar"]}'

# The desktop's player is also an Android media session (system media card, lock screen), paused as the app left it.
# Android routes media keys only to a session that plays audio on the phone, so a key cannot reach it.
session() { adb shell dumpsys media_session | grep -A12 "link-desktop-"; }
wait_for 10 "no Android media session for the desktop: $(session | head -5)" eval 'session | grep -q "Desk Session"'
wait_for 10 "the desktop session is not paused: $(session | grep state=)" eval 'session | grep -q "state=PAUSED"'
session | grep -E "state=|description=" | head -2 > "$OUT/desktop-session.txt"
record '{"step":"desktop-media-session","session":"link-desktop","state":"paused"}'

# The system media card drives the same session: its Play reaches the desktop player. Quick settings, since
# OxygenOS shows media cards only there.
adb shell cmd statusbar expand-settings
maestro "$FLOWS/media-card.yaml"
wait_for 10 "the media card's Play did not reach the desktop player: $(tr "\n" " " < "$OUT/player-calls.txt")" \
  grep -qx Play "$OUT/player-calls.txt"
# The card's seek bar takes a drag only from its thumb (elsewhere the card itself swipes) and held like a finger, so the drag
# is built from motion events, from the thumb to 75% (150 s).
# uiautomator dump never sees the animated card idle; Maestro's hierarchy does not wait for idle, but sometimes misses it.
# OxygenOS closes quick settings after the card's Play, so each try opens them again.
seek_bar() {
  adb shell cmd statusbar expand-settings
  "$MAESTRO" --device "$SERIAL" hierarchy 2> /dev/null | python3 -c '
import json, re, sys
def bars(node):
    attributes = node.get("attributes", {})
    # With quick settings closed, the app'"'"'s own Position bar shows; it has no id or one of the app.
    bar_id = attributes.get("resource-id", "")
    if attributes.get("class") == "android.widget.SeekBar" and ":" in bar_id and not bar_id.startswith(sys.argv[1] + ":"):
        yield attributes
    for child in node.get("children", []):
        yield from bars(child)
# Quick settings may hold other sliders (brightness); the media one names itself so in its id.
found = sorted(bars(json.load(sys.stdin)), key=lambda bar: "media" not in bar.get("resource-id", ""))
print(*re.findall(r"\d+", found[0]["bounds"] if found else ""))' "$PACKAGE" > "$OUT/seek-bar.txt"
  cat "$OUT/seek-bar.txt" >> "$OUT/maestro.log"
  [[ -s $OUT/seek-bar.txt && $(wc -w < "$OUT/seek-bar.txt") == 4 ]]
}
wait_for 30 "the media card shows no seek bar" seek_bar
read -r x1 y1 x2 y2 < "$OUT/seek-bar.txt"
y=$(((y1 + y2) / 2))
screenshot 4b-before-drag
# The track is the bounds less a padding of 1/20 at each end; the thumb sits at the live position of 200 s.
read -r position updated < <(session | grep -o 'position=[0-9]*, .*updated=[0-9]*' | head -1 | grep -oE '[0-9]+' | sed -n '1p;$p' | xargs)
now=$(adb shell cat /proc/uptime | awk '{printf "%d", $1 * 1000}')
pad=$(((x2 - x1) / 20)) track=$((x2 - x1 - 2 * (x2 - x1) / 20))
thumb=$((x1 + pad + track * (position + now - updated) / 200000)) target=$((x1 + pad + track * 3 / 4))
adb shell "input motionevent DOWN $thumb $y; sleep 0.3
for step in 1 2 3 4 5 6 7 8 9 10; do input motionevent MOVE \$(($thumb + ($target - $thumb) * step / 10)) $y; done
sleep 0.3; input motionevent UP $target $y"
screenshot 4c-after-drag
wait_for 10 "dragging the card's seek bar did not seek past 120 s: $(tr "\n" " " < "$OUT/player-calls.txt")" \
  eval 'grep -E "^SetPosition " "$OUT/player-calls.txt" | tail -1 | awk "{exit !(\$3 > 120000000 && \$3 < 190000000)}"'
adb shell cmd statusbar collapse
# The emulator has no lock screen until the swipe lock is on.
adb shell locksettings set-disabled false > /dev/null
adb shell input keyevent KEYCODE_SLEEP
adb shell input keyevent KEYCODE_WAKEUP
sleep 2 # real time: the lock screen draws its media card
adb exec-out screencap -p > "$OUT/5-lock-screen-media.png"
adb shell wm dismiss-keyguard
record '{"step":"media-card","card":"Play reached the desktop player","lock_screen":"5-lock-screen-media.png"}'


alarm_volume() { adb shell cmd media_session volume --stream 4 --get 2> /dev/null | sed -n 's/.*volume is \([0-9]*\).*/\1/p' | tr -d '\r'; }
ALARM_MAX=$(adb shell cmd media_session volume --stream 4 --get 2> /dev/null | sed -n 's/.*\.\.\([0-9]*\)\].*/\1/p' | tr -d '\r')
zen() { adb shell settings get global zen_mode | tr -d '\r'; }
adb shell cmd notification allow_dnd "$PACKAGE"
# The emulator ignores `cmd media_session volume` for the alarm stream, which stays at its maximum, so the restore
# is checked against whatever it was.
VOLUME_BEFORE=$(alarm_volume)
adb shell cmd notification set_dnd priority
wait_for 10 "Do Not Disturb did not turn on" eval '[[ $(zen) != 0 ]]'
ZEN_BEFORE=$(zen)
link Ring "$ID" true > /dev/null || fail "D-Bus Ring"
wait_for 10 "the phone did not report ringing" heard PhoneRinging "'$ID', true"
wait_for 10 "the alarm stream is not at full volume: $(alarm_volume)" eval '[[ $(alarm_volume) == "$ALARM_MAX" ]]'
[[ $(zen) == "$ZEN_BEFORE" ]] || fail "the ring changed the user's Do Not Disturb on Android 15: zen_mode $(zen)"
adb shell dumpsys audio > "$OUT/audio.txt"
PIID=$(grep "new AudioAttributes:AudioAttributes: usage=USAGE_ALARM" "$OUT/audio.txt" | tail -1 | sed -n 's/.*player piid:\([0-9]*\).*/\1/p')
[[ -n $PIID ]] && grep -q "player piid:$PIID event:started" "$OUT/audio.txt" || fail "no alarm player started in dumpsys audio"
adb shell cmd statusbar expand-notifications
sleep 2 # real time: the shade animates open
screenshot 4-phone-ringing
adb shell cmd statusbar collapse
link Ring "$ID" false > /dev/null || fail "D-Bus Ring off"
wait_for 10 "the phone did not report the stop" heard PhoneRinging "'$ID', false"
wait_for 10 "the alarm player was not released" eval 'adb shell dumpsys audio | grep -q "releasing player piid:$PIID"'
[[ $(alarm_volume) == "$VOLUME_BEFORE" ]] || fail "the alarm volume was not restored: $(alarm_volume), was $VOLUME_BEFORE"
[[ $(zen) == "$ZEN_BEFORE" ]] || fail "Do Not Disturb was not restored: zen_mode $(zen)"
adb shell cmd notification set_dnd off
record "{\"step\":\"find-my-phone\",\"alarm_volume\":\"$VOLUME_BEFORE -> 7 -> $(alarm_volume)\",\"dnd\":\"priority, untouched\",\"player\":\"USAGE_ALARM piid $PIID, released on stop\"}"

adb shell am start -W -n "$PACKAGE/.MainActivity" > /dev/null
maestro "$FLOWS/ring-desktop.yaml"
wait_for 10 "the phone did not ring the desktop" heard RingRequested "'$ID', true"
link DesktopRinging "$ID" true > /dev/null || fail "D-Bus DesktopRinging"
maestro "$FLOWS/stop-desktop.yaml"
wait_for 10 "the phone did not stop the desktop" heard RingRequested "'$ID', false"
record '{"step":"find-my-desktop","ring":"RingRequested true","stop":"RingRequested false"}'

for permission in READ_PHONE_STATE READ_CALL_LOG READ_CONTACTS ANSWER_PHONE_CALLS; do
  adb shell pm grant "$PACKAGE" "android.permission.$permission"
done
adb shell content insert --uri content://com.android.contacts/raw_contacts --bind account_type:n: --bind account_name:n:
CONTACT=$(adb shell content query --uri content://com.android.contacts/raw_contacts --projection _id | sed -n 's/.*_id=\([0-9]*\).*/\1/p' | sort -n | tail -1)
adb shell content insert --uri content://com.android.contacts/data --bind raw_contact_id:i:"$CONTACT" \
  --bind mimetype:s:vnd.android.cursor.item/name --bind data1:s:"'Ada Lovelace'"
adb shell content insert --uri content://com.android.contacts/data --bind raw_contact_id:i:"$CONTACT" \
  --bind mimetype:s:vnd.android.cursor.item/phone_v2 --bind data1:s:5551234 --bind data2:i:2
playerctl -p e2e play
wait_for 10 "the test player did not play again" eval '[[ $(grep -cx Play "$OUT/player-calls.txt") -ge 1 ]]'
PLAYS=$(grep -cx Play "$OUT/player-calls.txt")
PAUSES=$(grep -cx Pause "$OUT/player-calls.txt" || true)
adb emu gsm call 5551234 > /dev/null
wait_for 30 "no ringing Call on D-Bus" heard Call "'$ID', 'ringing'"
wait_for 10 "the contact's name did not arrive: $(signal Call "'ringing'")" heard Call "'Ada Lovelace'"
signal Call "'Ada Lovelace'" | grep -q "5551234" || fail "no number with the contact: $(signal Call "'Ada Lovelace'")"
wait_for 10 "the desktop player was not paused for the call" eval '[[ $(grep -cx Pause "$OUT/player-calls.txt") -gt $PAUSES ]]'
link CallAction "$ID" mute > /dev/null || fail "D-Bus CallAction mute"
wait_for 10 "the ringer was not muted" eval 'adb shell dumpsys audio | grep -A2 "\- STREAM_RING:" | grep -q "Muted: true"'
screenshot 6-incoming-call
link CallAction "$ID" decline > /dev/null || fail "D-Bus CallAction decline"
wait_for 20 "the call was not declined: $(adb emu gsm list 2>&1)" heard Call "'$ID', 'idle'"
adb emu gsm list | grep -q 5551234 && fail "the declined call is still up"
wait_for 10 "the desktop player was not resumed" eval '[[ $(grep -cx Play "$OUT/player-calls.txt") -gt $PLAYS ]]'
wait_for 10 "the ringer stayed muted" eval 'adb shell dumpsys audio | grep -A2 "\- STREAM_RING:" | grep -q "Muted: false"'
record '{"step":"call-from-contact","name":"Ada Lovelace","mute":"STREAM_RING muted","decline":"call ended","desktop_player":"paused, resumed"}'

adb emu gsm call 5550000 > /dev/null
wait_for 30 "no Call for an unknown number" heard Call "'5550000', ''"
adb emu gsm cancel 5550000 > /dev/null
wait_for 20 "no idle Call after the caller hung up" eval '[[ $(grep -c "org.andromeda.Link1.Call (.*idle" "$OUT/signals.txt") -ge 2 ]]'
record '{"step":"call-from-number","name":"","cancel":"idle"}'

python3 - "$OUT" <<'PY'
import json, os, sys
out = sys.argv[1]
steps = [json.loads(line) for line in open(os.path.join(out, "results.jsonl"))]
shots = sorted(name for name in os.listdir(out) if name.endswith(".png"))
json.dump({"pass": True, "steps": steps, "screenshots": shots}, open(os.path.join(out, "results.json"), "w"), indent=2)
PY
cat "$OUT/results.jsonl"
echo "PASS; artifacts: $OUT"
