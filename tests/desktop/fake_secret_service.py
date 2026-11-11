"""A Secret Service that owns org.freedesktop.secrets and never answers:
every method call is accepted and its reply withheld, as when a keyring
waits for an unlock prompt nobody answers."""
import dbus, dbus.service, dbus.mainloop.glib
from gi.repository import GLib
dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
bus = dbus.SessionBus()
name = dbus.service.BusName("org.freedesktop.secrets", bus)
pending = []
class Service(dbus.service.Object):
    @dbus.service.method("org.freedesktop.Secret.Service", in_signature="sv", out_signature="vo",
                         async_callbacks=("ok", "err"))
    def OpenSession(self, algorithm, inp, ok, err):
        print(f"fake secret service: OpenSession({algorithm}) received, never answering", flush=True)
        pending.append((ok, err))
Service(bus, "/org/freedesktop/secrets")
GLib.MainLoop().run()
