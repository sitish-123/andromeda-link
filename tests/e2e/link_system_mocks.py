#!/usr/bin/env python3
"""Mock logind, UPower, power-profiles-daemon, and the Andromeda shell on the bus in DBUS_SESSION_BUS_ADDRESS, for
link_system.sh. Every method call goes to LOG. SetBrightness writes its value into BACKLIGHT/actual_brightness.

Usage: link_system_mocks.py LOG BACKLIGHT_DIR    (runs until killed)
"""
import subprocess
import sys
import time

import dbus

MOCK = "org.freedesktop.DBus.Mock"
SESSION = "org.freedesktop.login1.Session"


def spawn(log, name, path, interface):
    proc = subprocess.Popen([sys.executable, "-m", "dbusmock", "-l", log, name, path, interface])
    bus = dbus.SessionBus()
    for _ in range(100):
        if bus.name_has_owner(name):
            return proc, dbus.Interface(bus.get_object(name, path), MOCK)
        time.sleep(0.05)
    sys.exit(f"{name} never appeared")


def main():
    log, backlight = sys.argv[1], sys.argv[2]
    procs = []

    proc, logind = spawn(log, "org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager")
    procs.append(proc)
    logind.AddMethod("", "Suspend", "b", "", "")
    lock = f'self.UpdateProperties("{SESSION}", {{"LockedHint": dbus.Boolean(True)}})'
    brightness = f'open("{backlight}/actual_brightness", "w").write(str(args[2]))'
    logind.AddObject(
        "/org/freedesktop/login1/session/auto",
        SESSION,
        {"LockedHint": False, "IdleHint": False},
        [("Lock", "", "", lock), ("SetBrightness", "ssu", "", brightness)],
    )

    proc, upower = spawn(
        log, "org.freedesktop.UPower", "/org/freedesktop/UPower/devices/DisplayDevice", "org.freedesktop.UPower.Device"
    )
    procs.append(proc)
    upower.AddProperties(
        "",
        {"IsPresent": True, "Percentage": dbus.Double(64.0), "State": dbus.UInt32(1)},
    )

    proc, profiles = spawn(
        log,
        "org.freedesktop.UPower.PowerProfiles",
        "/org/freedesktop/UPower/PowerProfiles",
        "org.freedesktop.UPower.PowerProfiles",
    )
    procs.append(proc)
    profiles.AddProperty("", "ActiveProfile", "balanced")

    proc, shell = spawn(log, "org.andromeda.Shell1", "/org/andromeda/Shell1", "org.andromeda.Shell1")
    procs.append(proc)
    shell.AddProperty("", "DoNotDisturb", False)
    shell.AddMethod("", "ScreenOff", "", "", "")

    print("ready", flush=True)
    try:
        for proc in procs:
            proc.wait()
    finally:
        for proc in procs:
            proc.terminate()


if __name__ == "__main__":
    main()
