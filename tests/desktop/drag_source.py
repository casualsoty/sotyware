"""A drag source for the Linux desktop tests (virtual display only).

Shows a small GTK 3 window whose single button can be dragged; the drag
offers the paths given on the command line as a text/uri-list, the way a
file manager does (XDND). cloud_ui_files.py starts it, moves the X pointer
with XTest from the button to sotyware's window and releases it there, so
sotyware receives a real drop from another program.

usage: python3 drag_source.py <x> <y> <path>...
Prints "ready <x> <y> <width> <height>" (the button, in root coordinates)
once it is shown, "sent <n>" when the drop target asked for the data, and
exits when stdin closes or after 120 s. Touches no file.
"""

import sys

import gi

gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
from gi.repository import Gdk, GLib, Gtk  # noqa: E402

x, y = int(sys.argv[1]), int(sys.argv[2])
uris = [GLib.filename_to_uri(p, None) for p in sys.argv[3:]]

win = Gtk.Window(title="sotyware-drag-source")
win.set_default_size(160, 60)
win.move(x, y)
button = Gtk.Button(label="drag me")
win.add(button)
button.drag_source_set(Gdk.ModifierType.BUTTON1_MASK, [], Gdk.DragAction.COPY)
button.drag_source_add_uri_targets()


def on_get(_widget, _ctx, data, _info, _time):
    data.set_uris(uris)
    print(f"sent {len(uris)}", flush=True)


button.connect("drag-data-get", on_get)


def report():
    alloc = button.get_allocation()
    origin = button.get_window().get_origin()
    print(f"ready {origin.x + alloc.x} {origin.y + alloc.y} {alloc.width} {alloc.height}", flush=True)
    return False


win.connect("map-event", lambda *_: GLib.timeout_add(300, report) and False)
win.connect("destroy", Gtk.main_quit)
GLib.io_add_watch(sys.stdin, GLib.IO_HUP | GLib.IO_IN, lambda *_: Gtk.main_quit() or False)
GLib.timeout_add_seconds(120, Gtk.main_quit)
win.show_all()
Gtk.main()
