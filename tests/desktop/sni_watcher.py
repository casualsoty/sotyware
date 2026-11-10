"""Minimal org.kde.StatusNotifierWatcher so sotyware (tray-icon's ksni backend) registers a tray item.

Stands in for the tray host of KDE / GNOME+AppIndicator extension. Records each
registration (sender unique name + object path) as a line in argv[1].
"""
import sys

import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib

IFACE = "org.kde.StatusNotifierWatcher"
OUT = sys.argv[1]


class Watcher(dbus.service.Object):
    def __init__(self, bus):
        self.items = []
        self.well_known = dbus.service.BusName(IFACE, bus)
        super().__init__(bus, "/StatusNotifierWatcher")

    @dbus.service.method(IFACE, in_signature="s", out_signature="", sender_keyword="sender")
    def RegisterStatusNotifierItem(self, service, sender=None):
        path = service if service.startswith("/") else "/StatusNotifierItem"
        bus_name = sender if service.startswith("/") else service
        entry = f"{bus_name} {path}"
        self.items.append(entry)
        with open(OUT, "a") as f:
            f.write(entry + "\n")
        self.StatusNotifierItemRegistered(entry)

    @dbus.service.method(IFACE, in_signature="s", out_signature="")
    def RegisterStatusNotifierHost(self, service):
        pass

    @dbus.service.signal(IFACE, signature="s")
    def StatusNotifierItemRegistered(self, service):
        pass

    @dbus.service.method(dbus.PROPERTIES_IFACE, in_signature="ss", out_signature="v")
    def Get(self, iface, prop):
        return self.GetAll(iface)[prop]

    @dbus.service.method(dbus.PROPERTIES_IFACE, in_signature="s", out_signature="a{sv}")
    def GetAll(self, iface):
        return {
            "RegisteredStatusNotifierItems": dbus.Array(self.items, signature="s"),
            "IsStatusNotifierHostRegistered": dbus.Boolean(True),
            "ProtocolVersion": dbus.Int32(0),
        }


DBusGMainLoop(set_as_default=True)
watcher = Watcher(dbus.SessionBus())
GLib.MainLoop().run()
