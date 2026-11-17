# Helpers shared by the phases of cloud_ui.py that use the pointer or the
# GTK folder dialog (cloud_ui_files.py, cloud_ui_sync.py), run with exec()
# in cloud_ui.py's namespace (pid, nodes, wait_for, focus_window, ctrl,
# key, keysym, X, XT, dpy, ...): find(), the XTest pointer and
# choose_folder().
XT.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
XT.XTestFakeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]


def find(node, role, name=None):
    out = []
    if node is None:
        return out

    def walk(n):
        try:
            if n.get_role() == role and (name is None or n.get_name() == name):
                out.append(n)
            for j in range(n.get_child_count()):
                c = n.get_child_at_index(j)
                if c is not None:
                    walk(c)
        except GLib.Error:
            pass

    walk(node)
    return out


def extents(node):
    e = Atspi.Component.get_extents(node, Atspi.CoordType.SCREEN)
    return e.x, e.y, e.width, e.height


def click_xy(x, y, button=1, mods=()):
    codes = [X.XKeysymToKeycode(dpy, X.XStringToKeysym(m.encode())) for m in mods]
    for c in codes:
        XT.XTestFakeKeyEvent(dpy, c, 1, 0)
    # The modifier must be down at the server before the button event.
    X.XSync(dpy, 0)
    time.sleep(0.15)
    XT.XTestFakeMotionEvent(dpy, -1, int(x), int(y), 0)
    X.XSync(dpy, 0)
    time.sleep(0.1)
    XT.XTestFakeButtonEvent(dpy, button, 1, 0)
    XT.XTestFakeButtonEvent(dpy, button, 0, 0)
    for c in reversed(codes):
        XT.XTestFakeKeyEvent(dpy, c, 0, 0)
    X.XSync(dpy, 0)
    time.sleep(0.25)


def folder_chooser(title):
    for role, name, _, node in nodes(pid):
        if role == Atspi.Role.FILE_CHOOSER and name == title:
            return node
    return None


def accept_button(dlg):
    """The folder dialog's accept button ("Open" with GTK 3.24 here, as rfd
    passes no label; "Select" in some versions)."""
    for label in ("Open", "_Open", "Select", "_Select"):
        found = find(dlg, Atspi.Role.PUSH_BUTTON, label)
        if found:
            return found[0]
    return None


def choose_folder(title, path):
    """In the GTK folder dialog (GtkFileChooserNative, folder mode): leaves
    "Recent", where the dialog starts and where Open stays disabled (a
    probe on this virtual display showed AT-SPI's press of a disabled Open
    does nothing, which is why earlier drivers could not choose a folder),
    by a real click on Home in the sidebar; then Ctrl+L, the folder's path
    with a trailing "/" and Return, which in folder mode accepts that
    folder. The caller checks the result on disk or at the server, never
    only what the dialog showed."""
    dlg = wait_for(lambda: folder_chooser(title), 20)
    if dlg is None:
        print(f"  no folder dialog {title!r}: choosers "
              f"{[n for r, n, _, _ in nodes(pid) if r in (Atspi.Role.FILE_CHOOSER, Atspi.Role.DIALOG)]}", flush=True)
        return False
    focus_window(pid)
    open_btn = wait_for(lambda: accept_button(dlg), 5)
    home = wait_for(lambda: next(iter(find(dlg, Atspi.Role.LABEL, "Home")), None), 5)
    if open_btn is None or home is None:
        print(f"  folder dialog: no Open button ({open_btn is not None}) or no Home place ({home is not None})", flush=True)
        close_choosers()
        return False
    x, y, w, h = extents(home)
    click_xy(x + w // 2, y + h // 2)
    if not wait_for(lambda: open_btn.get_state_set().contains(Atspi.StateType.ENABLED), 5):
        print("  folder dialog: Open still disabled after Home", flush=True)
        close_choosers()
        return False
    ctrl("l")
    entry = wait_for(lambda: next((e for e in find(dlg, Atspi.Role.TEXT)
                                   if e.get_state_set().contains(Atspi.StateType.SHOWING)
                                   and e.get_state_set().contains(Atspi.StateType.EDITABLE)), None), 5)
    if entry is None:
        print("  folder dialog: no location bar", flush=True)
        close_choosers()
        return False
    want = path.rstrip("/") + "/"
    # Set through AT-SPI (EditableText): Xvfb's keymap has no key for "ö"
    # or "ß", and typing would race GTK's inline completion.
    Atspi.EditableText.set_text_contents(entry, want)
    Atspi.Text.set_caret_offset(entry, len(want))
    if not wait_for(lambda: Atspi.Text.get_text(entry, 0, -1) == want, 5):
        print(f"  folder dialog: location bar {Atspi.Text.get_text(entry, 0, -1)!r}", flush=True)
        close_choosers()
        return False
    # GTK completes a unique child's name inline (selected text after the
    # cursor, "dlx/" in a folder that holds only dlx): Delete removes it,
    # or Return would choose that child (seen 2026-11-14).
    time.sleep(0.8)
    if Atspi.Text.get_text(entry, 0, -1) != want:
        key(keysym("Delete"))
    if not wait_for(lambda: Atspi.Text.get_text(entry, 0, -1) == want, 3):
        print(f"  folder dialog: location bar completed to {Atspi.Text.get_text(entry, 0, -1)!r}", flush=True)
        close_choosers()
        return False
    key(keysym("Return"))
    if wait_for(lambda: folder_chooser(title) is None, 6):
        return True
    # Return went into the folder instead: Open (enabled there) chooses it.
    if open_btn.get_state_set().contains(Atspi.StateType.ENABLED):
        Atspi.EditableText.set_text_contents(entry, "")
        Atspi.Action.do_action(open_btn, 0)
        if wait_for(lambda: folder_chooser(title) is None, 6):
            return True
    print(f"  folder dialog: still open; location bar {Atspi.Text.get_text(entry, 0, -1)!r}", flush=True)
    close_choosers()
    return False


def close_choosers():
    """Cancels a GTK dialog a failed step left open (it holds the keyboard)."""
    for role, _, _, node in nodes(pid):
        if role == Atspi.Role.FILE_CHOOSER:
            for b in find(node, Atspi.Role.PUSH_BUTTON, "Cancel") + find(node, Atspi.Role.PUSH_BUTTON, "_Cancel"):
                Atspi.Action.do_action(b, 0)
