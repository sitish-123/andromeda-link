#!/usr/bin/env python3
"""A fake iwd on the bus in DBUS_STARTER_ADDRESS: one station on `link-test0`, the networks "Home" (connected) and
"Phone-AP". Prints each call it gets, one per line, as `Method path [args]`, and asks a registered agent for the
passphrase on Connect of Phone-AP."""
import sys

import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib

DBusGMainLoop(set_as_default=True)
bus = dbus.bus.BusConnection(sys.argv[1])
name = dbus.service.BusName("net.connman.iwd", bus)
STATION = "/net/connman/iwd/0/3"
HOME = STATION + "/486f6d65_psk"
PHONE = STATION + "/50686f6e652d4150_psk"
KNOWN = "/net/connman/iwd/50686f6e652d4150_psk"
agent = [None]


def log(*parts):
    print(" ".join(str(p) for p in parts), flush=True)


class Root(dbus.service.Object):
    @dbus.service.method("org.freedesktop.DBus.ObjectManager", out_signature="a{oa{sa{sv}}}")
    def GetManagedObjects(self):
        return {STATION: {"net.connman.iwd.Station": {}, "net.connman.iwd.Device": {"Name": "link-test0"}}}


class Manager(dbus.service.Object):
    @dbus.service.method("net.connman.iwd.AgentManager", in_signature="o", sender_keyword="sender")
    def RegisterAgent(self, path, sender):
        agent[0] = (sender, path)
        log("RegisterAgent", path)

    @dbus.service.method("net.connman.iwd.AgentManager", in_signature="o")
    def UnregisterAgent(self, path):
        log("UnregisterAgent", path)


class Props(dbus.service.Object):
    props = {}

    @dbus.service.method("org.freedesktop.DBus.Properties", in_signature="ss", out_signature="v")
    def Get(self, interface, prop):
        if prop not in self.props:
            raise dbus.exceptions.DBusException("no such property", name="org.freedesktop.DBus.Error.InvalidArgs")
        return self.props[prop]

    @dbus.service.method("org.freedesktop.DBus.Properties", in_signature="s", out_signature="a{sv}")
    def GetAll(self, interface):
        return self.props


class Station(Props):
    props = {"ConnectedNetwork": dbus.ObjectPath(HOME)}

    @dbus.service.method("net.connman.iwd.Station")
    def Scan(self):
        log("Scan", STATION)

    @dbus.service.method("net.connman.iwd.Station", out_signature="a(on)")
    def GetOrderedNetworks(self):
        return [(dbus.ObjectPath(HOME), dbus.Int16(-5000)), (dbus.ObjectPath(PHONE), dbus.Int16(-6000))]

    @dbus.service.method("net.connman.iwd.Station")
    def Disconnect(self):
        log("Disconnect", STATION)


class Network(Props):
    def __init__(self, path, ssid, known):
        super().__init__(bus, path)
        self.path = path
        self.props = {"Name": ssid}
        if known:
            self.props["KnownNetwork"] = dbus.ObjectPath(known)

    @dbus.service.method("net.connman.iwd.Network", async_callbacks=("ok", "err"))
    def Connect(self, ok, err):
        if self.path != PHONE:
            log("Connect", self.path)
            return ok()
        sender, path = agent[0]
        proxy = dbus.Interface(bus.get_object(sender, path), "net.connman.iwd.Agent")

        def answered(passphrase):
            log("Connect", self.path, passphrase)
            ok()

        proxy.RequestPassphrase(dbus.ObjectPath(self.path), reply_handler=answered, error_handler=err)


class Known(dbus.service.Object):
    @dbus.service.method("net.connman.iwd.KnownNetwork")
    def Forget(self):
        log("Forget", KNOWN)


Root(bus, "/")
Manager(bus, "/net/connman/iwd")
Station(bus, STATION)
Network(HOME, "Home", None)
Network(PHONE, "Phone-AP", KNOWN)
Known(bus, KNOWN)
log("ready")
GLib.MainLoop().run()
