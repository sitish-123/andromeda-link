#!/usr/bin/env python3
"""A stand-in for NetworkManager on the test's system bus: the calls andromeda-linkd makes to join and leave a phone's
hotspot or Wi-Fi Direct group (an SSID starting "DIRECT-"), recorded as JSON lines. Joining runs JOIN_CMD (the E2E
brings up the link there); leaving runs LEAVE_CMD. While the file FAIL_FLAG exists, a hotspot join ends deactivated, as
a wrong passphrase would; FAIL_FLAG-group does the same for a group. While FAIL_FLAG-group-stuck exists, a group shows
up in scans only after 20 s and its join stays activating, so the desktop's join outlasts the phone's wait. The Wi-Fi
device is idle unless FAIL_FLAG-wifi-busy exists, when it has a connection of its own (on an interface with no radio
behind it, so no second interface can be added). It lists an access point only for an SSID a scan asked for, as iwd
joins only what its last scan saw.

Usage: nm_mock.py LOG JOIN_CMD LEAVE_CMD FAIL_FLAG
"""
import json
import os
import subprocess
import sys
import time

from gi.repository import Gio, GLib

NM = "org.freedesktop.NetworkManager"
ROOT = "/org/freedesktop/NetworkManager"
ACTIVATING, ACTIVATED, DEACTIVATED = 1, 2, 4
XML = f"""
<node>
  <interface name="{NM}">
    <method name="AddAndActivateConnection2">
      <arg direction="in" type="a{{sa{{sv}}}}" name="connection"/>
      <arg direction="in" type="o" name="device"/>
      <arg direction="in" type="o" name="specific_object"/>
      <arg direction="in" type="a{{sv}}" name="options"/>
      <arg direction="out" type="o" name="path"/>
      <arg direction="out" type="o" name="active_connection"/>
      <arg direction="out" type="a{{sv}}" name="result"/>
    </method>
    <method name="DeactivateConnection">
      <arg direction="in" type="o" name="active_connection"/>
    </method>
    <property name="Devices" type="ao" access="read"/>
  </interface>
  <interface name="{NM}.Connection.Active">
    <property name="State" type="u" access="read"/>
    <property name="Ip4Config" type="o" access="read"/>
  </interface>
  <interface name="{NM}.IP4Config">
    <property name="AddressData" type="aa{{sv}}" access="read"/>
  </interface>
  <interface name="{NM}.Device">
    <property name="DeviceType" type="u" access="read"/>
    <property name="ActiveConnection" type="o" access="read"/>
    <property name="Interface" type="s" access="read"/>
  </interface>
  <interface name="{NM}.Device.Wireless">
    <method name="RequestScan">
      <arg direction="in" type="a{{sv}}" name="options"/>
    </method>
    <method name="GetAllAccessPoints">
      <arg direction="out" type="ao" name="access_points"/>
    </method>
  </interface>
  <interface name="{NM}.AccessPoint">
    <property name="Ssid" type="ay" access="read"/>
  </interface>
</node>
"""


def main() -> int:
    log_path, join_cmd, leave_cmd, fail_flag = sys.argv[1:5]
    info = Gio.DBusNodeInfo.new_for_xml(XML)
    bus = Gio.bus_get_sync(Gio.BusType.SYSTEM)
    state = {"count": 0, "active": {}, "scanned": None}
    wifi, access_point = f"{ROOT}/Devices/2", f"{ROOT}/AccessPoint/1"

    def log(entry):
        with open(log_path, "a") as out:
            out.write(json.dumps({**entry, "t": round(time.time(), 3)}) + "\n")

    def active_property(path):
        def get(_conn, _sender, _path, _iface, name):
            if name == "State":
                return GLib.Variant("u", state["active"].get(path, DEACTIVATED))
            return GLib.Variant("o", path.replace("ActiveConnection", "IP4Config"))
        return get

    def ip4_property(_conn, _sender, _path, _iface, _name):
        address = {"address": GLib.Variant("s", "10.80.0.1"), "prefix": GLib.Variant("u", 24)}
        return GLib.Variant("aa{sv}", [address])

    def call(_conn, _sender, _path, _iface, method, params, invocation):
        if method == "AddAndActivateConnection2":
            settings, _device, _specific, options = params.unpack()
            state["count"] += 1
            active = f"{ROOT}/ActiveConnection/{state['count']}"
            ssid = bytes(settings["802-11-wireless"]["ssid"]).decode()
            group = ssid.startswith("DIRECT-")
            failing = os.path.exists(fail_flag + "-group" if group else fail_flag)
            stuck = group and os.path.exists(fail_flag + "-group-stuck")
            log({
                "call": "add-and-activate",
                "device": _device,
                "specific": _specific,
                "ssid": ssid,
                "psk": settings["802-11-wireless-security"]["psk"],
                "key_mgmt": settings["802-11-wireless-security"]["key-mgmt"],
                "autoconnect": settings["connection"]["autoconnect"],
                "persist": options.get("persist"),
                "fails": failing,
                **({"stuck": stuck} if group else {}),
            })
            if not failing and not stuck:
                subprocess.run(join_cmd, shell=True, check=True)
            state["active"][active] = ACTIVATING if stuck else DEACTIVATED if failing else ACTIVATED
            bus.register_object(active, info.interfaces[1], None, active_property(active), None)
            bus.register_object(active.replace("ActiveConnection", "IP4Config"), info.interfaces[2], None,
                                ip4_property, None)
            invocation.return_value(GLib.Variant("(ooa{sv})", (f"{ROOT}/Settings/{state['count']}", active, {})))
        elif method == "DeactivateConnection":
            (active,) = params.unpack()
            log({"call": "deactivate", "active": active})
            if state["active"].get(active) == ACTIVATED:
                subprocess.run(leave_cmd, shell=True, check=False)
            state["active"][active] = DEACTIVATED
            invocation.return_value(None)
        elif method == "RequestScan":
            ssids = params.unpack()[0].get("ssids", [])
            scanned = bytes(ssids[0]) if ssids else None
            if scanned != state["scanned"]:
                state["scanned"], state["scanned_at"] = scanned, time.time()
            log({"call": "request-scan", "ssid": state["scanned"].decode() if state["scanned"] else None})
            invocation.return_value(None)
        elif method == "GetAllAccessPoints":
            scanned = state["scanned"]
            hidden = scanned and scanned.startswith(b"DIRECT-") and os.path.exists(fail_flag + "-group-stuck") \
                and time.time() - state["scanned_at"] < 20
            invocation.return_value(GLib.Variant("(ao)", ([access_point] if scanned and not hidden else [],)))

    def root_property(_conn, _sender, _path, _iface, _name):
        return GLib.Variant("ao", [wifi])

    def wifi_property(_conn, _sender, _path, _iface, name):
        if name == "DeviceType":
            return GLib.Variant("u", 2)
        if name == "Interface":
            return GLib.Variant("s", "mockwlan0")
        busy = os.path.exists(fail_flag + "-wifi-busy")
        return GLib.Variant("o", f"{ROOT}/ActiveConnection/home" if busy else "/")

    def access_point_property(_conn, _sender, _path, _iface, _name):
        return GLib.Variant("ay", state["scanned"] or b"")

    bus.register_object(ROOT, info.interfaces[0], call, root_property, None)
    bus.register_object(wifi, info.interfaces[3], None, wifi_property, None)
    bus.register_object(wifi, info.interfaces[4], call, wifi_property, None)
    bus.register_object(access_point, info.interfaces[5], None, access_point_property, None)
    Gio.bus_own_name_on_connection(bus, NM, Gio.BusNameOwnerFlags.NONE, None, None)
    GLib.MainLoop().run()
    return 0


if __name__ == "__main__":
    sys.exit(main())
