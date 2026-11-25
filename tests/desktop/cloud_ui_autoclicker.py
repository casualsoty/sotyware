# The autoclicker phase of cloud_ui.py (ADR-050), run by it with exec() in
# its own namespace (pid, p, check, press, text, by_role, buttons, fill,
# wait_for, link, key, keysym, ctrl, type_text, focus_window, windows_of, X,
# XT, dpy, W, XEvent, ERR, PROFILE, ...). Kept in its own file only for
# length.
#
# Drives the release binary (a test-profile build, ADR-027) on the private
# Xvfb display of cloud-ui.sh, where the X11 input backend works
# (WAYLAND_DISPLAY is unset). The input is real, through XTest, but only on
# this display, which no person uses. A plain window of this driver plays
# the program being automated: it records every click and key it receives.
#
# Off by default: nothing grabbed (this driver can grab F6 and F8 itself),
# F6 does nothing. Turn on registers the six suggested shortcuts (now this
# driver cannot grab them). A bounded run from the Clicker tab clicks the
# test window exactly 25 times at the fixed position typed into the tab,
# after the countdown. From the test window: F6 starts and stops the
# clicker, F7 pauses and resumes it, F8 (the emergency stop) stops it, the
# pointer pushed into a corner of the screen stops it, and with the pointer
# on sotyware's own window every click is skipped. F9 records two clicks
# and a key typed into the test window, F10 stops; the draft is saved as a
# macro (checked in automation.sqlite3: no F9 or F10 in it, nor a click on
# sotyware's own window) and Play replays it into the test window.
#
# Then what a person would otherwise check by hand: the right button and
# double clicks at 5 per second (timed by the X server); a shortcut changed
# by pressing it in its field, and the keys the field refuses; with no
# emergency stop an endless run asks first, and the page's Stop stops it;
# the Settings tab's countdown select, focused once, follows several keys
# in a row (a burst, then half a second apart), each change saved;
# playback with repetitions at 2x speed (timed), paused with F7, stopped
# with F8, and the profile's macro played with Shift+F11; the editor
# (delay, add, move, duplicate, delete, Preview sends nothing, leaving
# asks, Save, Revert, the saved changes played); a restart keeps
# everything and starts nothing; a second profile with its own shortcut,
# switched and started with; a macro's own shortcut, export through the
# native save dialog, delete, import through the native open dialog (its
# shortcut switched off); keys a macro holds are released by F8, by the
# page's Stop and by quitting; at 100 clicks per second the page stays
# responsive. After every stop no button or key is held. Turning the
# feature off in Settings › Features releases every shortcut, and F6 then
# does nothing; while it is off, the emergency stop's key is refused for
# another action with the reason; and in a Wayland session the page says
# the autoclicker is not available, and why.
import re

import test_profile as tp  # cloud_ui.py put this folder on sys.path

exec(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "cloud_ui_gtk.py"), encoding="utf-8").read())

AUTO_DB = PROFILE.db("automation.sqlite3")
ROOT = X.XDefaultRootWindow(dpy)
NET_ACTIVE = X.XInternAtom(dpy, b"_NET_ACTIVE_WINDOW", 0)

X.XCreateSimpleWindow.restype = W
X.XCreateSimpleWindow.argtypes = [ctypes.c_void_p, W, ctypes.c_int, ctypes.c_int, ctypes.c_uint, ctypes.c_uint,
                                  ctypes.c_uint, ctypes.c_ulong, ctypes.c_ulong]
X.XSelectInput.argtypes = [ctypes.c_void_p, W, ctypes.c_long]
X.XMapWindow.argtypes = [ctypes.c_void_p, W]
X.XDestroyWindow.argtypes = [ctypes.c_void_p, W]
X.XStoreName.argtypes = [ctypes.c_void_p, W, ctypes.c_char_p]
X.XChangeProperty.argtypes = [ctypes.c_void_p, W, W, W, ctypes.c_int, ctypes.c_int, ctypes.c_void_p, ctypes.c_int]
X.XSetWMNormalHints.argtypes = [ctypes.c_void_p, W, ctypes.c_void_p]
X.XPending.argtypes = [ctypes.c_void_p]
X.XNextEvent.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
X.XTranslateCoordinates.argtypes = [ctypes.c_void_p, W, W, ctypes.c_int, ctypes.c_int, ctypes.POINTER(ctypes.c_int),
                                    ctypes.POINTER(ctypes.c_int), ctypes.POINTER(W)]
X.XQueryPointer.argtypes = [ctypes.c_void_p, W, ctypes.POINTER(W), ctypes.POINTER(W), ctypes.POINTER(ctypes.c_int),
                            ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int),
                            ctypes.POINTER(ctypes.c_uint)]
X.XQueryKeymap.argtypes = [ctypes.c_void_p, ctypes.c_char * 32]
X.XGrabKey.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_uint, W, ctypes.c_int, ctypes.c_int, ctypes.c_int]
X.XUngrabKey.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_uint, W]
XT.XTestFakeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]
XT.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]

KEY_PRESS, KEY_RELEASE, BUTTON_PRESS, BUTTON_RELEASE = 2, 3, 4, 5
SHIFT_MASK = 1


class XSizeHints(ctypes.Structure):
    _fields_ = [("flags", ctypes.c_long), ("x", ctypes.c_int), ("y", ctypes.c_int), ("width", ctypes.c_int),
                ("height", ctypes.c_int), ("min_width", ctypes.c_int), ("min_height", ctypes.c_int),
                ("max_width", ctypes.c_int), ("max_height", ctypes.c_int), ("width_inc", ctypes.c_int),
                ("height_inc", ctypes.c_int), ("min_aspect_x", ctypes.c_int), ("min_aspect_y", ctypes.c_int),
                ("max_aspect_x", ctypes.c_int), ("max_aspect_y", ctypes.c_int), ("base_width", ctypes.c_int),
                ("base_height", ctypes.c_int), ("win_gravity", ctypes.c_int)]


class XInputEvent(ctypes.Structure):
    """XKeyEvent and XButtonEvent, which differ only in the name of
    `detail` (keycode, button)."""
    _fields_ = [("type", ctypes.c_int), ("serial", ctypes.c_ulong), ("send_event", ctypes.c_int),
                ("display", ctypes.c_void_p), ("window", W), ("root", W), ("subwindow", W),
                ("time", ctypes.c_ulong), ("x", ctypes.c_int), ("y", ctypes.c_int),
                ("x_root", ctypes.c_int), ("y_root", ctypes.c_int), ("state", ctypes.c_uint),
                ("detail", ctypes.c_uint), ("same_screen", ctypes.c_int)]


class AnyXEvent(ctypes.Union):
    _fields_ = [("input", XInputEvent), ("pad", ctypes.c_long * 24)]


class XErrorEvent(ctypes.Structure):
    _fields_ = [("type", ctypes.c_int), ("display", ctypes.c_void_p), ("resourceid", W),
                ("serial", ctypes.c_ulong), ("error_code", ctypes.c_ubyte), ("request_code", ctypes.c_ubyte),
                ("minor_code", ctypes.c_ubyte)]


# ---- the program being automated: a plain window of this driver ----
EVENTS = []  # (type, button or keycode, x, y), x and y inside the window
TIMES = []  # the X server's time of each event in EVENTS, in ms


def origin(w):
    x, y, child = ctypes.c_int(), ctypes.c_int(), W()
    X.XTranslateCoordinates(dpy, w, ROOT, 0, 0, ctypes.byref(x), ctypes.byref(y), ctypes.byref(child))
    return x.value, y.value


def make_target(x, y, width, height):
    """A managed top-level window at (x, y) that cannot be resized, with
    this driver's pid, selecting key and button events."""
    w = X.XCreateSimpleWindow(dpy, ROOT, x, y, width, height, 0, 0, 0xF0F0F0)
    X.XStoreName(dpy, w, b"sotyware e2e target")
    me = (ctypes.c_ulong * 1)(os.getpid())
    X.XChangeProperty(dpy, w, NET_WM_PID, 6, 32, 0, me, 1)  # XA_CARDINAL, PropModeReplace
    hints = XSizeHints()
    # USPosition | USSize | PMinSize | PMaxSize: openbox keeps it where it is.
    hints.flags = 1 | 2 | 16 | 32
    hints.x, hints.y, hints.width, hints.height = x, y, width, height
    hints.min_width = hints.max_width = width
    hints.min_height = hints.max_height = height
    X.XSetWMNormalHints(dpy, w, ctypes.byref(hints))
    X.XSelectInput(dpy, w, 1 | 2 | 4 | 8)  # key and button press and release
    X.XMapWindow(dpy, w)
    X.XSync(dpy, 0)

    def viewable():
        a = _WinAttrs()
        return X.XGetWindowAttributes(dpy, w, ctypes.byref(a)) and a.map_state == 2

    wait_for(viewable, 5)
    return w


def pump():
    ev = AnyXEvent()
    while X.XPending(dpy) > 0:
        X.XNextEvent(dpy, ctypes.byref(ev))
        e = ev.input
        if e.window == TARGET and KEY_PRESS <= e.type <= BUTTON_RELEASE:
            EVENTS.append((e.type, e.detail, e.x, e.y))
            TIMES.append(e.time)
    return EVENTS


def received(mark, kind):
    return [(d, x, y) for t, d, x, y in pump()[mark:] if t == kind]


def presses(mark):
    return received(mark, BUTTON_PRESS)


def releases(mark):
    return received(mark, BUTTON_RELEASE)


def active_window():
    t, prop = W(), ctypes.c_void_p()
    fmt, n, after = ctypes.c_int(), ctypes.c_ulong(), ctypes.c_ulong()
    if X.XGetWindowProperty(dpy, ROOT, NET_ACTIVE, 0, 1, 0, 33, ctypes.byref(t), ctypes.byref(fmt),
                            ctypes.byref(n), ctypes.byref(after), ctypes.byref(prop)) == 0 \
            and prop.value and n.value:
        return ctypes.cast(prop, ctypes.POINTER(ctypes.c_ulong))[0]
    return 0


def activate(w):
    """Asks the window manager to bring `w` to the front, as a taskbar
    would (_NET_ACTIVE_WINDOW, source 2)."""
    ev = XEvent()
    ev.xclient.type, ev.xclient.window, ev.xclient.message_type, ev.xclient.format = 33, w, NET_ACTIVE, 32
    ev.xclient.data[0], ev.xclient.data[1] = 2, 0
    X.XSendEvent(dpy, ROOT, 0, (1 << 20) | (1 << 19), ctypes.byref(ev))
    X.XSync(dpy, 0)
    return wait_for(lambda: active_window() == w, 5)


def dashboard_rect():
    best = None
    for w in windows_of(pid):
        a = _WinAttrs()
        if X.XGetWindowAttributes(dpy, w, ctypes.byref(a)) and a.map_state == 2 and not a.override_redirect:
            if best is None or a.width * a.height > best[2] * best[3]:
                best = (*origin(w), a.width, a.height)
    return best


# ---- input through XTest, on this private display only ----
def move_to(x, y):
    XT.XTestFakeMotionEvent(dpy, -1, x, y, 0)
    X.XSync(dpy, 0)


def click_at(x, y):
    move_to(x, y)
    XT.XTestFakeButtonEvent(dpy, 1, 1, 0)
    XT.XTestFakeButtonEvent(dpy, 1, 0, 0)
    X.XSync(dpy, 0)


def hotkey(name, shift=False):
    key(keysym(name), shift)


def held():
    """Pointer buttons and keys down now, as the server sees them."""
    r, c = W(), W()
    rx, ry, wx, wy, mask = ctypes.c_int(), ctypes.c_int(), ctypes.c_int(), ctypes.c_int(), ctypes.c_uint()
    X.XQueryPointer(dpy, ROOT, ctypes.byref(r), ctypes.byref(c), ctypes.byref(rx), ctypes.byref(ry),
                    ctypes.byref(wx), ctypes.byref(wy), ctypes.byref(mask))
    km = (ctypes.c_char * 32)()
    X.XQueryKeymap(dpy, km)
    keys = [i * 8 + b for i, byte in enumerate(km.raw) for b in range(8) if byte & (1 << b)]
    return (mask.value >> 8) & 0x1F, keys


GRAB_ERRORS = []


def _on_grab_error(_d, e):
    GRAB_ERRORS.append(ctypes.cast(e, ctypes.POINTER(XErrorEvent)).contents.error_code)
    return 0


_grab_handler = ERR(_on_grab_error)


def can_grab(name, mods=0):
    """Whether this driver can grab the key on the root window itself: not
    while another client (sotyware) holds that shortcut. Called only while
    sotyware does not register shortcuts, so it never takes one from it."""
    code = keysym(name)
    del GRAB_ERRORS[:]
    X.XSetErrorHandler(_grab_handler)
    try:
        X.XGrabKey(dpy, code, mods, ROOT, 0, 1, 1)
        X.XSync(dpy, 0)
        ok = not GRAB_ERRORS
        if ok:
            X.XUngrabKey(dpy, code, mods, ROOT)
            X.XSync(dpy, 0)
        return ok
    finally:
        X.XSetErrorHandler(_ignore)


SHORTCUTS = [("F6", 0), ("F7", 0), ("F8", 0), ("F9", 0), ("F10", 0), ("F11", SHIFT_MASK)]


# ---- the page ----
def page_tab(name):
    for n, node in by_role(pid, Atspi.Role.PAGE_TAB):
        if n.strip() == name:
            return Atspi.Action.do_action(node, 0)
    print(f"  no tab {name!r}; tabs: {[n for n, _ in by_role(pid, Atspi.Role.PAGE_TAB)]}", flush=True)
    return False


def radio(name, prefix=False):
    for n, node in by_role(pid, Atspi.Role.RADIO_BUTTON):
        if n.strip() == name or (prefix and n.strip().startswith(name)):
            return Atspi.Action.do_action(node, 0)
    print(f"  no option {name!r}; options: {[n for n, _ in by_role(pid, Atspi.Role.RADIO_BUTTON)]}", flush=True)
    return False


NUMBER_ROLES = (Atspi.Role.SPIN_BUTTON, Atspi.Role.ENTRY)


def number_field(name):
    for n, node in by_role(pid, *NUMBER_ROLES):
        if n.strip() == name and node.get_state_set().contains(Atspi.StateType.ENABLED):
            return node
    return None


def set_number(name, value):
    """Types `value` into the number field `name` as real keys (select all,
    type, Tab), so the page sees a change as from a person."""
    node = wait_for(lambda: number_field(name), 5)
    if node is None:
        print(f"  no enabled number field {name!r}; fields: "
              f"{[n for n, _ in by_role(pid, *NUMBER_ROLES)]}", flush=True)
        return False
    focus_window(pid)
    Atspi.Component.grab_focus(node)
    if not wait_for(lambda: node.get_state_set().contains(Atspi.StateType.FOCUSED), 3):
        print(f"  set {name!r}: could not focus", flush=True)
        return False
    ctrl("a")
    type_text(str(value))
    key(keysym("Tab"))
    return True


def profile_doc():
    with sqlite3.connect(f"file:{AUTO_DB}?mode=ro", uri=True) as db:
        row = db.execute("SELECT data FROM profiles ORDER BY position, id").fetchone()
    return json.loads(row[0]) if row else None


def macro_docs():
    with sqlite3.connect(f"file:{AUTO_DB}?mode=ro", uri=True) as db:
        return [json.loads(r[0]) for r in db.execute("SELECT data FROM macros ORDER BY id").fetchall()]


def clicker_saved(pred):
    def ok():
        doc = profile_doc()
        return doc is not None and pred(doc["clicker"])
    return wait_for(ok, 10)


def last_is(message, timeout=10):
    """The Overview's line about the last run says `message` (it shows that
    line only while nothing runs)."""
    return wait_for(lambda: message in text(pid), timeout)


def none_held():
    b, k = held()
    return b == 0 and not k


# ---- for the checks that replace the manual check (testing.md) ----
CTRL_MASK, ALT_MASK = 4, 8


def chord(mods, name):
    """Presses the modifiers `mods` (keysym names), then the key `name`, as
    real keys, and releases them in reverse order."""
    codes = [keysym(m) for m in mods]
    for c in codes:
        XT.XTestFakeKeyEvent(dpy, c, 1, 0)
    k = keysym(name)
    XT.XTestFakeKeyEvent(dpy, k, 1, 0)
    XT.XTestFakeKeyEvent(dpy, k, 0, 0)
    for c in reversed(codes):
        XT.XTestFakeKeyEvent(dpy, c, 0, 0)
    X.XSync(dpy, 0)


def btns():
    return [b.strip() for b in buttons(pid)]


def press_times(mark):
    """The X server's times of the button presses the test window got since `mark`."""
    pump()
    return [TIMES[i] for i in range(mark, len(EVENTS)) if EVENTS[i][0] == BUTTON_PRESS]


def sequence(mark):
    """Button presses (("button", x, y)) and presses of the key A (("A",))
    the test window got since `mark`, in order: what a macro did."""
    a = keysym("a")
    out = []
    for t, dd, x, y in pump()[mark:]:
        if t == BUTTON_PRESS:
            out.append(("button", x, y))
        elif t == KEY_PRESS and dd == a:
            out.append(("A",))
    return out


def shortcut_field(label):
    """The shortcut field `label` (a button named "<label>: <keys>") as
    (name, node), or None."""
    for n, node in by_role(pid, Atspi.Role.PUSH_BUTTON):
        if n.startswith(label + ": "):
            return n, node
    return None


def listening(label):
    f = shortcut_field(label)
    return f is not None and "press the keys" in f[0]


def set_shortcut(label, keys):
    """Clicks the shortcut field `label`, as a person does, then presses
    `keys()` there as real keys."""
    found = wait_for(lambda: shortcut_field(label), 5)
    if found is None:
        print(f"  no shortcut field {label!r}; buttons: {btns()[:60]}", flush=True)
        return False
    node = found[1]
    focus_window(pid)
    Atspi.Component.grab_focus(node)
    if not wait_for(lambda: node.get_state_set().contains(Atspi.StateType.FOCUSED), 3):
        print(f"  shortcut field {label!r}: could not focus", flush=True)
        return False
    Atspi.Action.do_action(node, 0)
    if not wait_for(lambda: listening(label), 3):
        print(f"  shortcut field {label!r} does not listen: {(shortcut_field(label) or ('?',))[0]!r}", flush=True)
        return False
    keys()
    return True


def row_state(label):
    """The state column of the Settings tab's shortcut row `label` ("Ready",
    "Not set", "Off", why it is not registered), or None."""
    for role, _, _, node in nodes(pid):
        if role == Atspi.Role.TABLE_ROW and node.get_child_count() >= 4 \
                and cell_text(node.get_child_at_index(0)).startswith(label):
            return cell_text(node.get_child_at_index(node.get_child_count() - 1))
    return None


def combo(label):
    for role, name, _, node in nodes(pid):
        if role == Atspi.Role.COMBO_BOX and (name == label or name.startswith(label + " ")):
            return node
    return None


def choose(label, option):
    """Chooses `option` in the <select> named `label` with the keyboard
    (focus, Home, Down × its index): WebKitGTK fires the change event for
    keyboard changes, not for an AT-SPI selection."""
    node = wait_for(lambda: combo(label), 5)
    if node is None:
        print(f"  no select {label!r}: {[n for r, n, _, _ in nodes(pid) if r == Atspi.Role.COMBO_BOX]}", flush=True)
        return False
    names = [n.get_name().strip() for n in find(node, Atspi.Role.MENU_ITEM)]
    if option not in names:
        print(f"  select {label!r} has no option {option!r}: {names[:60]}", flush=True)
        return False
    focus_window(pid)
    Atspi.Component.grab_focus(node)
    if not wait_for(lambda: node.get_state_set().contains(Atspi.StateType.FOCUSED), 3):
        print(f"  select {label!r}: could not focus", flush=True)
        return False
    key(keysym("Home"))
    for _ in range(names.index(option)):
        key(keysym("Down"))
    time.sleep(0.4)
    return True


def toggle_button(name):
    return act(pid, (Atspi.Role.TOGGLE_BUTTON, Atspi.Role.PUSH_BUTTON), name)


PREVIEW_STATUS = re.compile(r"Preview \(no input is sent\): action (\d+), \S")


def watch_preview(timeout=30):
    """Presses the macro editor's Preview and follows the preview until it
    ends by itself. Returns whether Stop preview was offered, the status
    lines in the order they were seen, whether it ended, how many reads
    that took and the seconds from the press to the end.

    Reads only the children of Preview's parent, the editor's section,
    which holds its buttons and, while the playhead is on an action, the
    status line (WebKitGTK leaves the <div>s in between out of the tree).
    A walk of the whole editor page, about 700 nodes, took 1.1 to 1.6 s
    from the first preview on, as long as the preview of a short macro at
    2× (1.15 s), and once missed its status line (2026-11-23); reading
    these children takes 20 to 50 ms."""
    button = next((node for n, node in by_role(pid, Atspi.Role.PUSH_BUTTON) if n.strip() == "Preview"), None)
    section = button.get_parent() if button is not None else None
    if section is None:
        print(f"  no Preview button in the editor: {btns()[-30:]}", flush=True)
        return False, [], False, 0, 0.0
    started, seen, reads = False, [], 0
    t0 = time.time()
    try:
        Atspi.Action.do_action(button, 0)
    except GLib.Error as e:
        # As in press(): the press was sent; the reads show whether it acted.
        print(f"  press 'Preview': no AT-SPI reply ({e.message[:60]})", flush=True)
    while time.time() < t0 + timeout:
        try:
            names, status = [], None
            for j in range(section.get_child_count()):
                c = section.get_child_at_index(j)
                role = c.get_role() if c is not None else None
                if role == Atspi.Role.PUSH_BUTTON:
                    names.append((c.get_name() or "").strip())
                elif role == Atspi.Role.STATUS_BAR:
                    t = Atspi.Text.get_text(c, 0, Atspi.Text.get_character_count(c))
                    if t.startswith("Preview (no input is sent)"):
                        status = t
        except GLib.Error:
            # A child went away while it was read (Preview and Stop preview
            # swap places): read again.
            time.sleep(0.02)
            continue
        reads += 1
        started = started or "Stop preview" in names
        if status is not None and (not seen or seen[-1] != status):
            seen.append(status)
        if started and "Preview" in names and "Stop preview" not in names and status is None:
            return True, seen, True, reads, time.time() - t0
        time.sleep(0.02)
    return started, seen, False, reads, time.time() - t0


def settings_doc():
    with sqlite3.connect(f"file:{AUTO_DB}?mode=ro", uri=True) as db:
        row = db.execute("SELECT data FROM settings WHERE id = 1").fetchone()
    return json.loads(row[0]) if row else None


def profile_docs():
    with sqlite3.connect(f"file:{AUTO_DB}?mode=ro", uri=True) as db:
        return [json.loads(r[0]) for r in db.execute("SELECT data FROM profiles ORDER BY position, id").fetchall()]


def profile_named(name):
    return next((d for d in profile_docs() if d["name"] == name), None)


def macro_named(name):
    return next((m for m in macro_docs() if m["name"] == name), None)


def binding(doc, action):
    return (((doc or {}).get("hotkeys") or {}).get(action) or {}).get("chord")


def emergency_chord():
    return ((settings_doc() or {}).get("emergencyStop") or {}).get("chord", "unread")


def stored():
    """Everything the autoclicker keeps: settings, profiles and macros."""
    return settings_doc(), profile_docs(), macro_docs()


def registered_count():
    """How many shortcuts the Overview lists as registered."""
    return text(pid).count("registered")


def shift_held():
    return keysym("Shift_L") in held()[1]


# The native GTK dialogs of export and import (Rust opens them, ADR-050).
def chooser_named(title):
    for role, name, _, node in nodes(pid):
        if role in (Atspi.Role.FILE_CHOOSER, Atspi.Role.DIALOG) and name == title:
            return node
    return None


def editable_text(dlg):
    return next((e for e in find(dlg, Atspi.Role.TEXT)
                 if e.get_state_set().contains(Atspi.StateType.SHOWING)
                 and e.get_state_set().contains(Atspi.StateType.EDITABLE)), None)


def save_to(title, path):
    """In the GTK save dialog `title`: the file's whole path into its name
    field, then Return."""
    dlg = wait_for(lambda: chooser_named(title), 20)
    entry = dlg is not None and wait_for(lambda: editable_text(dlg), 5)
    if not entry:
        print(f"  no save dialog {title!r} with a name field", flush=True)
        return False
    focus_window(pid)
    Atspi.Component.grab_focus(entry)
    wait_for(lambda: entry.get_state_set().contains(Atspi.StateType.FOCUSED), 3)
    ctrl("a")
    type_text(path)
    if not wait_for(lambda: Atspi.Text.get_text(entry, 0, -1) == path, 5):
        print(f"  name field: {Atspi.Text.get_text(entry, 0, -1)!r}", flush=True)
        return False
    key(keysym("Return"))
    return bool(wait_for(lambda: chooser_named(title) is None, 10))


def open_from(title, path):
    """In the GTK open dialog `title`: the file list focused, Ctrl+L, the
    file's whole path into the location bar, then Return."""
    dlg = wait_for(lambda: chooser_named(title), 20)
    if dlg is None:
        print(f"  no open dialog {title!r}", flush=True)
        return False
    focus_window(pid)
    tables = find(dlg, Atspi.Role.TABLE, "Files")
    if tables:
        Atspi.Component.grab_focus(tables[0])
        wait_for(lambda: tables[0].get_state_set().contains(Atspi.StateType.FOCUSED), 3)
    ctrl("l")
    entry = wait_for(lambda: editable_text(dlg), 5)
    if entry is None:
        print("  no location bar", flush=True)
        return False
    type_text(path)
    # GTK completes a name inline (selected text after the cursor); Delete
    # removes that completion.
    if wait_for(lambda: Atspi.Text.get_text(entry, 0, -1) != path, 1.5):
        key(keysym("Delete"))
    if not wait_for(lambda: Atspi.Text.get_text(entry, 0, -1) == path, 5):
        print(f"  location bar: {Atspi.Text.get_text(entry, 0, -1)!r}", flush=True)
        return False
    # The location bar completes asynchronously; a Return pressed before
    # its completion settles is swallowed.
    time.sleep(0.7)
    key(keysym("Return"))
    return bool(wait_for(lambda: chooser_named(title) is None, 10))


def quit_app():
    """Quits sotyware as the window's close does in dashboard-only mode;
    its exit code, or None if it did not end within 60 s."""
    close_windows(pid)
    try:
        return p.wait(timeout=60)
    except subprocess.TimeoutExpired:
        return None


def start_app(extra=None):
    """Starts sotyware again in this run's profile, with `extra` variables.
    ADR-027: only a test-profile build, acknowledged before anything is
    clicked."""
    global p, pid
    tp.require_test_profile_build(BIN)
    tp.clear_ack(PROFILE)
    with open(log, "ab") as f:
        p = subprocess.Popen([BIN], env={**env, **(extra or {})}, stdout=f, stderr=subprocess.STDOUT)
    pid = p.pid
    try:
        tp.verify_ack(PROFILE, pid, timeout=30, alive=lambda: p.poll() is None)
    except tp.IsolationError as e:
        p.kill()
        raise SystemExit(f"the restarted app is not provably isolated: {e}")
    return wait_for(lambda: any(n == "Autoclicker" for n, _ in by_role(pid, Atspi.Role.LINK)), 30)


def cpu_seconds(of):
    """User and system CPU time of process `of` so far, in seconds."""
    with open(f"/proc/{of}/stat") as f:
        fields = f.read().rsplit(")", 1)[1].split()
    return (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")


try:
    # Clear of the dashboard (900 × 540 in the middle of 1280 × 900), away
    # from the screen's corners.
    TARGET = make_target(1110, 40, 150, 110)
    tx, ty = origin(TARGET)
    CLICK, A, B = (tx + 75, ty + 55), (tx + 35, ty + 30), (tx + 115, ty + 80)
    d = dashboard_rect()
    clear = d is not None and (tx >= d[0] + d[2] or tx + 150 <= d[0] or ty >= d[1] + d[3] or ty + 110 <= d[1] - 30)
    check("the test window is up, clear of the dashboard", bool(TARGET) and clear, f"window at {tx},{ty}; dashboard {d}")

    # ---- off by default ----
    ok = link(pid, "Autoclicker") and wait_for(lambda: "The autoclicker is off." in text(pid), 15)
    check("the Autoclicker page says it is off (the feature is off by default)", bool(ok), text(pid)[:400])
    check("while it is off nothing is grabbed: this driver can grab F6 and F8 itself",
          can_grab("F6") and can_grab("F8"))
    activate(TARGET)
    mark = len(pump())
    hotkey("F6")
    time.sleep(1.0)
    check("F6 does nothing while it is off", not presses(mark) and "Clicking" not in text(pid))

    # ---- turn on ----
    ok = press(pid, "Turn on") and wait_for(
        lambda: "The autoclicker is off." not in text(pid) and text(pid).count("registered") >= 6, 15)
    t = text(pid)
    check("Turn on registers the six suggested shortcuts (F6, F7, F8, F9, F10, Shift+F11)",
          bool(ok) and "could not be registered" not in t, t[:1500])
    taken = [f"{'Shift+' if m else ''}{k}" for k, m in SHORTCUTS if can_grab(k, m)]
    check("sotyware holds them: this driver can grab none of them", not taken, f"still free: {taken}")
    labels = ["Emergency stop", "Start or stop the autoclicker", "Pause or resume", "Start recording",
              "Stop recording", "Play the profile's macro"]
    t = text(pid)
    check("a screen reader reads each shortcut's label on the Overview (AT-SPI)",
          all(lbl in t for lbl in labels), [lbl for lbl in labels if lbl not in t])

    # ---- a bounded run from the Clicker tab, at a fixed position ----
    ok = page_tab("Clicker") and wait_for(lambda: by_role(pid, Atspi.Role.RADIO_BUTTON), 5)
    ok = ok and radio("At a fixed position") and set_number("X", CLICK[0]) and set_number("Y", CLICK[1])
    ok = ok and radio("Stop after", prefix=True) and set_number("Number of clicks", 25)
    ok = ok and set_number("Speed", 20)
    saved = clicker_saved(lambda c: c["target"]["mode"] == "points"
                          and c["target"]["points"] == [{"x": CLICK[0], "y": CLICK[1]}]
                          and c["maxClicks"] == 25 and abs(c["intervalMs"] - 50) < 0.01)
    check("the Clicker tab saves a fixed position, 25 clicks and 20 per second to the profile",
          bool(ok and saved), (profile_doc() or {}).get("clicker"))
    mark = len(pump())
    ok = press(pid, "Start autoclicker")
    early = wait_for(lambda: presses(mark), 2.0)
    check("Start autoclicker counts down first: no click in the first 2 s", bool(ok) and not early, early)
    wait_for(lambda: len(releases(mark)) >= 25, 20)
    time.sleep(0.5)
    ps, rs = presses(mark), releases(mark)
    here = (1, CLICK[0] - tx, CLICK[1] - ty)
    check("then it clicks the test window exactly 25 times, left button, at the fixed position",
          len(ps) == 25 and len(rs) == 25 and set(ps) == {here} and set(rs) == {here},
          f"{len(ps)} presses, {len(rs)} releases, at {sorted(set(ps))}")
    ok = page_tab("Overview") and last_is("Autoclicker: Finished: 25 clicks")
    check("the Overview says it finished: 25 clicks", bool(ok), text(pid)[:800])
    check("nothing is held after it", none_held(), held())

    # ---- shortcuts from another program ----
    ok = page_tab("Clicker") and radio("Until stopped") and clicker_saved(lambda c: c["maxClicks"] is None)
    check("Repeat: until stopped, saved", bool(ok))
    page_tab("Overview")
    activate(TARGET)
    mark = len(pump())
    hotkey("F6")
    started = wait_for(lambda: len(presses(mark)) >= 5, 5)
    hotkey("F6")
    stopped = last_is("Autoclicker: Stopped with F6")
    n = len(presses(mark))
    time.sleep(0.6)
    check("F6 in the test window starts the clicker at once, F6 again stops it; no click after",
          bool(started and stopped) and len(presses(mark)) == n and n == len(releases(mark)) and none_held(),
          f"{n} presses then {len(presses(mark))}, {len(releases(mark))} releases, held {held()}")

    mark = len(pump())
    hotkey("F6")
    started = wait_for(lambda: len(presses(mark)) >= 3, 5)
    hotkey("F7")
    paused = wait_for(lambda: "Autoclicker paused" in text(pid), 5)
    n = len(presses(mark))
    time.sleep(0.6)
    still = len(presses(mark)) == n and n == len(releases(mark)) and none_held()
    check("F7 pauses it: no click while paused, nothing held", bool(started and paused) and still,
          f"{n} then {len(presses(mark))} presses, held {held()}")
    hotkey("F7")
    resumed = wait_for(lambda: len(presses(mark)) >= n + 3, 5)
    check("F7 again resumes it", bool(resumed))
    hotkey("F8")
    stopped = last_is("Autoclicker: Emergency stop (F8)")
    n = len(presses(mark))
    time.sleep(0.6)
    check("F8, the emergency stop, stops it; no click after, nothing held",
          bool(stopped) and len(presses(mark)) == n and n == len(releases(mark)) and none_held(),
          f"{n} then {len(presses(mark))} presses, held {held()}")

    mark = len(pump())
    hotkey("F6")
    started = wait_for(lambda: len(presses(mark)) >= 3, 5)
    # As a hand does: it keeps pushing the mouse into the corner for a
    # moment (a single push right after the clicker's check would be undone
    # by its next move; the engines narrow that to the instant before the
    # move, which a hand's stream of motion outlasts).
    for _ in range(25):
        move_to(0, 0)
        time.sleep(0.02)
    stopped = last_is("Autoclicker: Stopped: the pointer was pushed into a corner of the screen")
    n = len(presses(mark))
    time.sleep(0.6)
    check("pushing the pointer into a corner of the screen stops it",
          bool(started and stopped) and len(presses(mark)) == n and none_held(), f"{n} presses")

    # ---- never sotyware's own window ----
    ok = page_tab("Clicker") and radio("Where the pointer is") and \
        clicker_saved(lambda c: c["target"]["mode"] == "cursor")
    page_tab("Overview")
    d = dashboard_rect()
    move_to(d[0] + d[2] - 40, d[1] + d[3] - 40)
    mark = len(pump())
    hotkey("F6")
    skipped = wait_for(lambda: re.search(r"[1-9][0-9,]* clicks skipped over sotyware's window", text(pid)), 5)
    hotkey("F6")
    stopped = last_is("Autoclicker: Stopped with F6")
    check("with the pointer on sotyware's window it clicks nothing and counts the clicks as skipped",
          bool(ok and skipped and stopped) and not presses(mark), text(pid)[:600])

    # ---- record with F9 and F10 ----
    page_tab("Recorder")
    click_at(*CLICK)
    front = wait_for(lambda: active_window() == TARGET, 5)
    time.sleep(0.3)
    hotkey("F9")
    recording = wait_for(lambda: "Stop recording" in [b.strip() for b in buttons(pid)]
                         and "Recording starts" not in text(pid), 5)
    check("F9 in the test window starts recording at once", bool(front and recording), text(pid)[:600])
    time.sleep(0.3)
    click_at(*A)
    time.sleep(0.25)
    # A click on sotyware's own window while it records (on the page's
    # heading, which does nothing): never recorded.
    heading = next((node for n, node in by_role(pid, Atspi.Role.HEADING) if n.strip() == "Autoclicker"), None)
    OWN = None
    if heading is not None:
        hx, hy, hw, hh = extents(heading)
        OWN = (hx + min(hw // 2, 40), hy + hh // 2)
        click_at(*OWN)
        time.sleep(0.25)
    click_at(*B)
    time.sleep(0.25)
    hotkey("a")
    time.sleep(0.3)
    hotkey("F10")
    ready = wait_for(lambda: "Recording ready" in text(pid), 10)
    check("F10 stops it: the recording is ready to save", bool(ready), text(pid)[:600])
    ok = fill(pid, "Name", "E2E clicks") and press(pid, "Save macro") and \
        wait_for(lambda: "Saved “E2E clicks”" in text(pid), 10)
    docs = macro_docs()
    check("Save macro stores it in automation.sqlite3", bool(ok) and [m["name"] for m in docs] == ["E2E clicks"],
          [m.get("name") for m in docs])
    acts = docs[0]["actions"] if docs else []
    pressed = [(a["state"], a["x"], a["y"]) for a in acts if a["type"] == "button"]
    typed = [(a["key"], a["state"]) for a in acts if a["type"] == "key"]
    check("the macro holds both clicks where they were made and the key A, nothing of F9 or F10",
          pressed == [("down", *A), ("up", *A), ("down", *B), ("up", *B)]
          and typed == [("KeyA", "down"), ("KeyA", "up")]
          and {a["type"] for a in acts} <= {"move", "button", "key"},
          acts)
    d = dashboard_rect()
    inside = [(a["x"], a["y"]) for a in acts if a["type"] == "button" and d is not None
              and d[0] <= a["x"] < d[0] + d[2] and d[1] <= a["y"] < d[1] + d[3]]
    check("the click on sotyware's own window while recording is not in the macro",
          OWN is not None and d is not None and not inside, f"clicked at {OWN}; window {d}; buttons inside it: {inside}")

    # ---- play it ----
    page_tab("Macros")
    wait_for(lambda: "Play" in [b.strip() for b in buttons(pid)], 5)
    move_to(*CLICK)
    mark = len(pump())
    ok = press(pid, "Play")
    wait_for(lambda: received(mark, KEY_RELEASE), 15)
    time.sleep(0.5)
    ps = presses(mark)
    keys_in = [(t, dd) for t, dd, _, _ in pump()[mark:] if t in (KEY_PRESS, KEY_RELEASE)]
    a_code = keysym("a")
    check("Play replays it into the test window: both clicks where they were made, then the key A",
          bool(ok) and ps == [(1, A[0] - tx, A[1] - ty), (1, B[0] - tx, B[1] - ty)]
          and keys_in == [(KEY_PRESS, a_code), (KEY_RELEASE, a_code)],
          f"presses {ps}, keys {keys_in}")
    ok = page_tab("Overview") and last_is("Playback: Finished")
    check("the Overview says the playback finished; nothing is held", bool(ok) and none_held(), held())

    # ==== what testing.md's manual check asked a person to do ====
    pA, pB = ("button", A[0] - tx, A[1] - ty), ("button", B[0] - tx, B[1] - ty)
    TOGGLE, STOP_ALL = "Start or stop the autoclicker", "Emergency stop"

    def edit_macro(name):
        """Opens the macro's editor from the library (leaving the tab first
        closes an editor that is open)."""
        ok = page_tab("Overview") and page_tab("Macros") and press(pid, f"Edit {name}")
        return ok and wait_for(lambda: "Back to macros" in btns() and "Loading…" not in text(pid), 10)

    def save_macro():
        return press(pid, "Save") and wait_for(lambda: "Saved." in text(pid), 10)

    def idle():
        """Nothing runs: the Overview offers Start autoclicker and no Stop."""
        return "Start autoclicker" in btns() and "Stop" not in btns()

    # ---- the right button, double clicks and the speed, timed ----
    ok = page_tab("Clicker") and wait_for(lambda: by_role(pid, Atspi.Role.RADIO_BUTTON), 5)
    ok = ok and radio("Right", prefix=True) and radio("Double") and radio("At a fixed position") \
        and set_number("X", CLICK[0]) and set_number("Y", CLICK[1])
    ok = ok and radio("Stop after", prefix=True) and set_number("Number of clicks", 4) and set_number("Speed", 5)
    saved = clicker_saved(lambda c: c["button"] == "right" and c["clickType"] == "double" and c["maxClicks"] == 4
                          and abs(c["intervalMs"] - 200) < 0.01 and c["target"]["mode"] == "points")
    check("the Clicker tab saves the right button, double clicks, 4 clicks at 5 per second",
          bool(ok and saved), (profile_doc() or {}).get("clicker"))
    check("its hint says: one click every 200 ms", wait_for(lambda: "One click every 200 ms." in text(pid), 5),
          re.findall(r"Speed.{0,240}", text(pid), re.S))
    mark = len(pump())
    ok = press(pid, "Start autoclicker")
    early = wait_for(lambda: presses(mark), 2.0)
    wait_for(lambda: len(releases(mark)) >= 8, 20)
    time.sleep(0.6)
    ps, rs = presses(mark), releases(mark)
    right = (3, CLICK[0] - tx, CLICK[1] - ty)
    check("after the countdown, 4 double clicks: 8 presses of the right button at the fixed position, no more",
          bool(ok) and not early and len(ps) == 8 and len(rs) == 8 and set(ps) == {right} and set(rs) == {right},
          f"early {early}; {len(ps)} presses, {len(rs)} releases, at {sorted(set(ps))}")
    at = press_times(mark)
    gaps = [b - a for a, b in zip(at[0::2], at[2::2])]
    pairs = [at[i + 1] - at[i] for i in range(0, len(at) - 1, 2)]
    check("timed by the X server: a double click every 200 ms (150 to 250), its two clicks 50 ms apart (20 to 120)",
          len(gaps) == 3 and all(150 <= g <= 250 for g in gaps)
          and len(pairs) == 4 and all(20 <= q <= 120 for q in pairs),
          f"between double clicks {gaps} ms, inside each {pairs} ms")
    ok = page_tab("Overview") and last_is("Autoclicker: Finished: 4 clicks")
    check("a double click counts as one click: Finished: 4 clicks; nothing held",
          bool(ok) and none_held(), text(pid)[:800])

    # ---- a shortcut is changed by pressing it in its field; keys it refuses ----
    ok = page_tab("Clicker") and radio("Left", prefix=True) and radio("Single") and radio("Where the pointer is") \
        and radio("Until stopped") and set_number("Speed", 20)
    saved = clicker_saved(lambda c: c["button"] == "left" and c["clickType"] == "single" and c["maxClicks"] is None
                          and c["target"]["mode"] == "cursor" and abs(c["intervalMs"] - 50) < 0.01)
    check("back to single left clicks at the pointer, until stopped, 20 per second",
          bool(ok and saved), (profile_doc() or {}).get("clicker"))
    ok = page_tab("Settings") and set_shortcut(TOGGLE, lambda: chord(["Control_L", "Alt_L"], "k"))
    saved = wait_for(lambda: binding(profile_doc(), "toggle-clicker") == "Ctrl+Alt+KeyK", 10)
    ready = bool(saved) and wait_for(lambda: row_state(TOGGLE) == "Ready", 10)
    check("Ctrl+Alt+K pressed in the toggle's field becomes its shortcut: saved and registered",
          bool(ok and ready), f"{binding(profile_doc(), 'toggle-clicker')}, {row_state(TOGGLE)!r}")
    check("sotyware gave F6 back and holds Ctrl+Alt+K",
          bool(ready) and bool(wait_for(lambda: can_grab("F6"), 5)) and not can_grab("k", CTRL_MASK | ALT_MASK))
    ok = set_shortcut(TOGGLE, lambda: key(keysym("a")))
    letter = ok and wait_for(lambda: "A alone would stop that key from working everywhere" in text(pid), 5)
    key(keysym("F12"))
    f12 = wait_for(lambda: "F12 alone is reserved by Windows for debuggers" in text(pid), 5)
    check("the field refuses a letter alone and F12 alone, with the reason, and keeps listening",
          bool(letter and f12) and listening(TOGGLE) and binding(profile_doc(), "toggle-clicker") == "Ctrl+Alt+KeyK",
          text(pid)[:900])
    key(keysym("F8"))
    time.sleep(1.0)
    unchanged = listening(TOGGLE) and binding(profile_doc(), "toggle-clicker") == "Ctrl+Alt+KeyK"
    key(keysym("Escape"))
    check("F8 is the emergency stop's while it is on: it never reaches the field, nothing changes; Escape stops listening",
          unchanged and bool(wait_for(lambda: not listening(TOGGLE), 3)),
          f"{(shortcut_field(TOGGLE) or ('?',))[0]!r}, {binding(profile_doc(), 'toggle-clicker')}")
    page_tab("Overview")
    activate(TARGET)
    move_to(*CLICK)
    mark = len(pump())
    hotkey("F6")
    time.sleep(1.0)
    f6_free = not presses(mark)
    chord(["Control_L", "Alt_L"], "k")
    started = wait_for(lambda: len(presses(mark)) >= 3, 5)
    chord(["Control_L", "Alt_L"], "k")
    stopped = wait_for(lambda: re.search(r"Autoclicker: Stopped with Ctrl\W*Alt\W*K", text(pid)), 10)
    n = len(presses(mark))
    time.sleep(0.6)
    check("from the test window F6 now does nothing, and Ctrl+Alt+K starts and stops the clicker",
          f6_free and bool(started and stopped) and len(presses(mark)) == n and none_held(),
          f"F6 free {f6_free}; {n} then {len(presses(mark))} presses; {text(pid)[:300]}")
    ok = page_tab("Settings") and set_shortcut(TOGGLE, lambda: key(keysym("F6")))
    back = wait_for(lambda: binding(profile_doc(), "toggle-clicker") == "F6", 10) \
        and wait_for(lambda: row_state(TOGGLE) == "Ready", 10)
    check("F6 pressed in the field makes it the toggle again", bool(ok and back), repr(row_state(TOGGLE)))

    # ---- without an emergency stop, an endless run from the page asks first ----
    ok = set_shortcut(STOP_ALL, lambda: key(keysym("BackSpace")))
    cleared = wait_for(lambda: emergency_chord() is None, 10)
    free = bool(cleared) and wait_for(lambda: row_state(STOP_ALL) == "Not set", 5) \
        and wait_for(lambda: can_grab("F8"), 5)
    check("Backspace in the emergency stop's field clears it: saved, and F8 is free",
          bool(ok and free), f"{emergency_chord()!r}, {row_state(STOP_ALL)!r}")
    ok = page_tab("Clicker") and radio("At a fixed position") and set_number("X", CLICK[0]) \
        and set_number("Y", CLICK[1])
    saved = clicker_saved(lambda c: c["target"]["mode"] == "points" and c["maxClicks"] is None)
    mark = len(pump())
    asked = bool(ok and saved) and press(pid, "Start autoclicker") \
        and wait_for(lambda: "Run until you stop it?" in text(pid), 5)
    check("with no emergency stop, Start autoclicker (until stopped) asks first: Run until you stop it?",
          bool(asked), text(pid)[:900])
    ok = press(pid, "Cancel") and wait_for(lambda: "Run until you stop it?" not in text(pid), 5)
    time.sleep(4.0)
    check("Cancel starts nothing", bool(ok) and not presses(mark) and "Autoclicker starting" not in text(pid)
          and "Clicking" not in text(pid), text(pid)[:600])
    ok = press(pid, "Start autoclicker") and wait_for(lambda: "Start anyway" in btns(), 5) \
        and press(pid, "Start anyway")
    started = wait_for(lambda: len(presses(mark)) >= 5, 10)
    check("Start anyway starts it, after the countdown", bool(ok and started), f"{len(presses(mark))} presses")
    ok = press(pid, "Stop")
    stopped = wait_for(lambda: "Clicking" not in text(pid), 5)
    n = len(presses(mark))
    time.sleep(0.6)
    check("the page's Stop stops it at once: no click after, nothing held",
          bool(ok and stopped) and len(presses(mark)) == n and none_held(), f"{n} then {len(presses(mark))}")
    ok = page_tab("Overview") and last_is("Autoclicker: Stopped from sotyware")
    check("the Overview says it was stopped from sotyware", bool(ok), text(pid)[:800])
    ok = page_tab("Settings") and press(pid, "Use F8")
    back = wait_for(lambda: emergency_chord() == "F8", 10) and wait_for(lambda: row_state(STOP_ALL) == "Ready", 10)
    check("Use F8 sets the emergency stop again: saved and registered", bool(ok and back), repr(row_state(STOP_ALL)))

    # ---- several keys in a row on a select of the tab, focused once ----
    # Each change is saved at once, one after another, and the select keeps
    # the keyboard (until 2026-11-25 the tab disabled its controls while a
    # change was saved: the keys sent meanwhile were lost, and WebKitGTK
    # could take the focus from the select). Ends on 3 s, as it began.
    countdown = combo("Wait before starting from this window")

    def countdown_shows():
        return next((i.get_name().strip() for i in find(countdown, Atspi.Role.MENU_ITEM)
                     if i.get_state_set().contains(Atspi.StateType.SELECTED)), None)

    def countdown_focused():
        return countdown.get_state_set().contains(Atspi.StateType.FOCUSED)

    def countdown_ms():
        return (settings_doc() or {}).get("dashboardCountdownMs")

    def countdown_state():
        if countdown is None:
            return "no select \"Wait before starting from this window\""
        return f"shows {countdown_shows()!r}, focused {countdown_focused()}, saved {countdown_ms()}"

    focus_window(pid)
    countdown_ready = countdown is not None and countdown_shows() == "3 s" \
        and Atspi.Component.grab_focus(countdown) and wait_for(countdown_focused, 3)
    for _ in range(3):
        key(keysym("Up"))
    ok = bool(countdown_ready) and wait_for(lambda: countdown_shows() == "No wait" and countdown_ms() == 0, 10)
    check("three Ups in one burst on the countdown select, focused once: No wait, saved, the select keeps the keyboard",
          bool(ok) and countdown_focused(), countdown_state())
    for i in range(3):
        if i:
            time.sleep(0.5)
        key(keysym("Down"))
    ok = bool(countdown_ready) and wait_for(lambda: countdown_shows() == "3 s" and countdown_ms() == 3000, 10)
    check("three Downs half a second apart, still not focused again, bring it back to 3 s, saved",
          bool(ok) and countdown_focused(), countdown_state())

    # ---- playback: repetitions at 2x speed, F7, F8, and the profile's macro on Shift+F11 ----
    ok = edit_macro("E2E clicks") and choose("Repeat", "Several times") \
        and set_number("Number of repetitions", 3) and toggle_button("2×") and save_macro()
    doc = macro_named("E2E clicks") or {}
    t = text(pid)
    check("a screen reader reads the playback form's hint (AT-SPI)",
          "Delays are divided by the speed; the delays below are not." in t, re.findall(r"Speed.{0,200}", t, re.S))
    check("the editor saves 3 repetitions at 2× speed",
          bool(ok) and doc.get("playback", {}).get("repeat") == 3 and doc.get("playback", {}).get("speed") == 2,
          doc.get("playback"))
    acts = doc.get("actions", [])
    downs = [i for i, a in enumerate(acts) if a["type"] == "button" and a["state"] == "down"]
    recorded = sum(a["delayMs"] for a in acts[downs[0] + 1:downs[1] + 1]) if len(downs) == 2 else None
    move_to(*CLICK)
    mark = len(pump())
    ok = press(pid, "Play")
    wait_for(lambda: sequence(mark).count(("A",)) >= 3, 40)
    time.sleep(0.5)
    seq = sequence(mark)
    check("Play plays it 3 times: both clicks, then the key A, each time",
          bool(ok) and seq == [pA, pB, ("A",)] * 3, seq)
    at = press_times(mark)
    played = [at[i + 1] - at[i] for i in range(0, len(at) - 1, 2)]
    want = recorded / 2 if recorded else None
    check("at 2× the time between the two clicks is halved (timed by the X server)",
          want is not None and len(played) == 3 and all(abs(x - want) <= max(50, want * 0.3) for x in played),
          f"recorded {recorded} ms, played {played} ms, expected about {want}")
    ok = choose("Repeat", "Until stopped") and save_macro() \
        and (macro_named("E2E clicks") or {}).get("playback", {}).get("repeat", 0) is None
    mark = len(pump())
    ok = ok and press(pid, "Play")
    started = wait_for(lambda: len(sequence(mark)) >= 4, 15)
    hotkey("F7")
    paused = wait_for(lambda: "Paused:" in text(pid), 5)
    # A key held at the pause is released at the pause: let that arrive.
    time.sleep(0.2)
    n = len(pump())
    time.sleep(0.8)
    quiet = len(pump()) == n and none_held()
    check("an endless playback paused with F7: nothing sent while paused, nothing held",
          bool(ok and started and paused) and quiet, f"{n} then {len(pump())} events, held {held()}")
    hotkey("F7")
    resumed = wait_for(lambda: len(pump()) > n + 2, 10)
    hotkey("F8")
    stopped = page_tab("Overview") and last_is("Playback: Emergency stop (F8)")
    n = len(pump())
    time.sleep(0.6)
    check("F7 again resumes it; F8 stops it: nothing sent after, nothing held",
          bool(resumed and stopped) and len(pump()) == n and none_held(), f"{n} then {len(pump())}; {text(pid)[:300]}")
    ok = edit_macro("E2E clicks") and choose("Repeat", "Once") and save_macro() \
        and (macro_named("E2E clicks") or {}).get("playback", {}).get("repeat") == 1
    mid = (macro_named("E2E clicks") or {}).get("id")
    ok = ok and page_tab("Profiles") and choose("Macro", "E2E clicks") \
        and wait_for(lambda: (profile_doc() or {}).get("macroId") == mid, 10)
    check("the Profiles tab makes it the profile's macro", bool(ok), (profile_doc() or {}).get("macroId"))
    page_tab("Overview")
    activate(TARGET)
    move_to(*CLICK)
    mark = len(pump())
    hotkey("F11", shift=True)
    soon = wait_for(lambda: sequence(mark), 2.0)
    wait_for(lambda: ("A",) in sequence(mark), 15)
    time.sleep(0.5)
    check("Shift+F11 in the test window plays the profile's macro at once (no countdown)",
          bool(soon) and sequence(mark) == [pA, pB, ("A",)], sequence(mark))

    # ---- the editor ----
    ok = edit_macro("E2E clicks")
    before = (macro_named("E2E clicks") or {}).get("actions", [])
    n0 = len(before)
    ok = ok and set_number("Action 1: delay before it in ms", 400) and wait_for(lambda: "Unsaved changes" in text(pid), 5)
    ok = ok and choose("Kind of action to add", "Wait") and press(pid, "Add at the end") \
        and wait_for(lambda: f"Delete action {n0 + 1}" in btns(), 5)
    ok = ok and choose("Kind of action to add", "Click") and press(pid, "Add after selection") \
        and wait_for(lambda: f"Delete action {n0 + 2}" in btns(), 5)
    # The click (action n0 + 2, after the wait) moves above the wait, is
    # duplicated, and the copy is deleted again.
    ok = ok and press(pid, f"Move action {n0 + 2} up") and press(pid, f"Duplicate action {n0 + 1}") \
        and wait_for(lambda: f"Delete action {n0 + 3}" in btns(), 5)
    ok = ok and press(pid, f"Delete action {n0 + 2}") and wait_for(lambda: f"Delete action {n0 + 3}" not in btns(), 5)
    check("the editor sets a delay, adds a wait and a click, moves one up, duplicates and deletes: unsaved",
          bool(ok) and "Unsaved changes" in text(pid), btns()[-30:])
    # At 2× this macro's preview lasts about 1.15 s: only the editor's own
    # section is read meanwhile (watch_preview). The playhead never goes
    # back, so the actions its status line names only increase.
    mark = len(pump())
    started, seen, ended, reads, took = watch_preview()
    steps = [int(m.group(1)) for m in map(PREVIEW_STATUS.match, seen) if m]
    print(f"  preview: {reads} reads in {took:.2f} s; the status line named actions {steps}", flush=True)
    check("Preview runs along the timeline and sends nothing",
          started and ended and seen and len(steps) == len(seen) and steps == sorted(steps) and not pump()[mark:],
          f"Stop preview offered: {started}, ended by itself: {ended}, status lines: {seen}; "
          f"{len(pump()) - mark} events")
    ok = press(pid, "Back to macros") and wait_for(lambda: "Leave without saving?" in text(pid), 5)
    ok = ok and press(pid, "Keep editing") and wait_for(lambda: "Leave without saving?" not in text(pid), 5)
    check("leaving with unsaved changes asks first; Keep editing keeps them", bool(ok) and "Unsaved changes" in text(pid))
    ok = save_macro()
    after = (macro_named("E2E clicks") or {}).get("actions", [])

    def same_but_delay(a, b):
        return {k: v for k, v in a.items() if k != "delayMs"} == {k: v for k, v in b.items() if k != "delayMs"}

    good = len(after) == n0 + 2 and n0 > 0 and after[0]["delayMs"] == 400 and same_but_delay(after[0], before[0]) \
        and after[1:n0] == before[1:n0] and after[n0].get("type") == "click" \
        and (after[n0].get("x"), after[n0].get("y")) == B and after[n0 + 1].get("type") == "wait"
    check("Save stores exactly that: 400 ms before action 1, then a click where the last one was, then a wait",
          bool(ok) and good, after[:1] + after[max(0, n0 - 1):])
    ok = set_number("Action 1: delay before it in ms", 999) and wait_for(lambda: "Unsaved changes" in text(pid), 5)
    ok = ok and press(pid, "Revert") and wait_for(lambda: "Unsaved changes" not in text(pid), 5)
    check("Revert drops a change that was not saved",
          bool(ok) and ((macro_named("E2E clicks") or {}).get("actions") or [{}])[0].get("delayMs") == 400)
    move_to(*CLICK)
    mark = len(pump())
    ok = press(pid, "Play")
    wait_for(lambda: len(sequence(mark)) >= 4, 20)
    time.sleep(1.0)
    check("Play plays the saved changes: both clicks, the key A, then the added click",
          bool(ok) and sequence(mark) == [pA, pB, ("A",), pB], sequence(mark))
    ok = page_tab("Overview") and last_is("Playback: Finished")
    check("the playback finished; nothing held", bool(ok) and none_held(), held())

    # ---- a restart keeps everything and starts nothing ----
    kept = stored()
    code = quit_app()
    check("closing the window quits sotyware (dashboard-only mode): exit code 0", code == 0, code)
    ok = start_app() and link(pid, "Autoclicker") and wait_for(
        lambda: "The autoclicker is off." not in text(pid) and registered_count() >= 6, 20)
    check("started again, the autoclicker is still on, its six shortcuts registered", bool(ok), text(pid)[:800])
    taken = [f"{'Shift+' if m else ''}{k}" for k, m in SHORTCUTS if can_grab(k, m)]
    check("sotyware holds them again: this driver can grab none of them", bool(ok) and not taken, f"free: {taken}")
    mark = len(pump())
    time.sleep(3.0)
    check("nothing starts by itself", not pump()[mark:] and idle(), btns())
    check("its settings, profiles and macros are exactly as they were", stored() == kept)

    # ---- a second profile with its own shortcut; switched; started with ----
    first = profile_doc()
    ok = page_tab("Profiles") and fill(pid, "New profile", "Second") and press(pid, "Create profile") \
        and wait_for(lambda: profile_named("Second") is not None, 10)
    second = profile_named("Second") or {}
    check("Create profile adds a second profile with the active one's settings and shortcuts",
          bool(ok) and second.get("hotkeys") == first["hotkeys"] and second.get("clicker") == first["clicker"],
          [d["name"] for d in profile_docs()])
    ok = press(pid, "Use this profile") and wait_for(lambda: "“Second” is the active profile" in text(pid), 10)
    ok = ok and page_tab("Settings") and set_shortcut(TOGGLE, lambda: chord(["Control_L", "Alt_L"], "j"))
    ready = wait_for(lambda: binding(profile_named("Second"), "toggle-clicker") == "Ctrl+Alt+KeyJ", 10) \
        and wait_for(lambda: row_state(TOGGLE) == "Ready", 10)
    check("in the second profile the toggle becomes Ctrl+Alt+J; the first keeps F6",
          bool(ok and ready) and binding(profile_doc(), "toggle-clicker") == "F6", repr(row_state(TOGGLE)))
    check("with the second profile active, F6 is free and Ctrl+Alt+J is held",
          bool(ready) and bool(wait_for(lambda: can_grab("F6"), 5)) and not can_grab("j", CTRL_MASK | ALT_MASK))
    ok = page_tab("Profiles") and press(pid, "Start with it") \
        and wait_for(lambda: (settings_doc() or {}).get("defaultProfile") == second.get("id"), 10)
    check("Start with it makes the second profile the one sotyware starts with", bool(ok))
    ok = press(pid, "Use this profile") and wait_for(lambda: "is the active profile" in text(pid)
                                                     and "“Second” is the active profile" not in text(pid), 10)
    check("switching back to the first profile changes the shortcuts at once: F6 held, Ctrl+Alt+J free",
          bool(ok) and bool(wait_for(lambda: can_grab("j", CTRL_MASK | ALT_MASK), 5)) and not can_grab("F6"))
    code = quit_app()
    ok = code == 0 and start_app() and link(pid, "Autoclicker") and wait_for(lambda: registered_count() >= 6, 20)
    check("after a restart sotyware starts with the second profile: Ctrl+Alt+J held, F6 free",
          bool(ok) and not can_grab("j", CTRL_MASK | ALT_MASK) and can_grab("F6"), f"exit {code}")
    ok = page_tab("Profiles") and press(pid, "Use this profile") \
        and wait_for(lambda: "is the active profile" in text(pid), 10) and press(pid, "Start with it") \
        and wait_for(lambda: (settings_doc() or {}).get("defaultProfile") == first["id"], 10)
    check("back to the first profile, which sotyware starts with again", bool(ok) and not can_grab("F6"))

    # ---- a macro's own shortcut; export, delete, import ----
    ok = edit_macro("E2E clicks") and set_shortcut("Shortcut that plays E2E clicks",
                                                   lambda: chord(["Control_L", "Alt_L"], "m"))
    ok = ok and wait_for(lambda: "Unsaved changes" in text(pid), 5) and save_macro()
    sc = (macro_named("E2E clicks") or {}).get("shortcut")
    check("the macro's own shortcut is set in its editor: Ctrl+Alt+M, saved", bool(ok)
          and sc == {"chord": "Ctrl+Alt+KeyM", "enabled": True}, sc)
    ok = page_tab("Overview") and wait_for(lambda: registered_count() >= 7, 10)
    check("it is registered: this driver cannot grab Ctrl+Alt+M",
          bool(ok) and not can_grab("m", CTRL_MASK | ALT_MASK), text(pid)[:600])
    activate(TARGET)
    move_to(*CLICK)
    mark = len(pump())
    chord(["Control_L", "Alt_L"], "m")
    wait_for(lambda: len(sequence(mark)) >= 4, 15)
    time.sleep(1.0)
    check("Ctrl+Alt+M in the test window plays the macro", sequence(mark) == [pA, pB, ("A",), pB], sequence(mark))
    exported = os.path.join(os.environ["HOME"], "e2e-macro.sotyware.json")
    ok = page_tab("Macros") and press(pid, "Export E2E clicks") and save_to("Export macros and profiles", exported)
    done = ok and wait_for(lambda: os.path.exists(exported) and "Exported the macro to" in text(pid), 20)
    data = {}
    if done:
        with open(exported, encoding="utf-8") as f:
            data = json.load(f)
    out = data.get("macros") or [{}]
    check("Export… writes it through the native save dialog, with its shortcut",
          bool(done) and [m.get("name") for m in data.get("macros", [])] == ["E2E clicks"]
          and out[0].get("shortcut", {}).get("chord") == "Ctrl+Alt+KeyM", str(data)[:400])
    ok = press(pid, "Delete E2E clicks") and wait_for(lambda: "Delete macro" in btns(), 5) and press(pid, "Delete macro")
    gone = ok and wait_for(lambda: macro_named("E2E clicks") is None, 10)
    check("Delete asks, removes it and gives Ctrl+Alt+M back",
          bool(gone) and bool(wait_for(lambda: can_grab("m", CTRL_MASK | ALT_MASK), 5)))
    ok = press(pid, "Import…") and open_from("Import macros and profiles", exported)
    report = ok and wait_for(lambda: "Imported 1 macro" in text(pid), 20)
    back = macro_named("E2E clicks") or {}
    check("Import… brings it back through the native open dialog, its shortcut switched off",
          bool(report) and back.get("shortcut") == {"chord": "Ctrl+Alt+KeyM", "enabled": False}
          and back.get("actions") == after and "Imported shortcuts are switched off" in text(pid),
          (back.get("shortcut"), text(pid)[:400]))
    check("the imported shortcut is not registered: this driver can grab Ctrl+Alt+M",
          can_grab("m", CTRL_MASK | ALT_MASK))

    # ---- keys a macro holds are released: by F8, by the page's Stop, by quitting ----
    ok = page_tab("Overview") and page_tab("Macros") and press(pid, "New macro") \
        and fill(pid, "Name", "E2E hold") and press(pid, "Create and edit") \
        and wait_for(lambda: "Back to macros" in btns() and "No actions yet" in text(pid), 10)
    ok = ok and choose("Kind of action to add", "Key") and press(pid, "Add at the end") \
        and wait_for(lambda: combo("Action 1: key") is not None, 5) and choose("Action 1: key", "Left Shift")
    ok = ok and choose("Kind of action to add", "Wait") and press(pid, "Add after selection") \
        and wait_for(lambda: f"Delete action 2" in btns(), 5) \
        and set_number("Action 2: delay before it in ms", 30000) and save_macro()
    hold = macro_named("E2E hold") or {}
    check("a macro written in the editor: Left Shift down, then a 30 s wait",
          bool(ok) and [(a["type"], a.get("key"), a.get("state"), a["delayMs"]) for a in hold.get("actions", [])]
          == [("key", "ShiftLeft", "down", 100), ("wait", None, None, 30000)], hold.get("actions"))

    def hold_shift():
        """Plays E2E hold with the test window in front (playback sends no
        keys while sotyware's own window is active); True once Shift is down."""
        activate(TARGET)
        return press(pid, "Play") and wait_for(shift_held, 10)

    ok = hold_shift()
    hotkey("F8")
    released = wait_for(lambda: not shift_held(), 3)
    stopped = wait_for(lambda: "Play" in btns() and "Stop" not in btns(), 5)
    check("F8 stops a playback that holds Shift down, and Shift is released", bool(ok and released and stopped),
          f"Shift down at first: {ok}; held now {held()}")
    if shift_held():
        press(pid, "Stop")
        wait_for(lambda: not shift_held(), 5)
    ok = hold_shift()
    ok2 = press(pid, "Stop")
    released = wait_for(lambda: not shift_held(), 3)
    check("the page's Stop stops it, and Shift is released", bool(ok and ok2 and released), held())
    ok = hold_shift()
    code = quit_app()
    check("quitting sotyware while a macro holds Shift releases it (exit code 0)",
          bool(ok) and code == 0 and not shift_held(), f"exit {code}, held {held()}")
    ok = start_app() and link(pid, "Autoclicker") and wait_for(lambda: registered_count() >= 6, 20)
    check("sotyware starts again afterwards", bool(ok), text(pid)[:400])

    # ---- at 100 clicks per second the page stays responsive ----
    ok = page_tab("Clicker") and radio("At a fixed position") and set_number("X", CLICK[0]) \
        and set_number("Y", CLICK[1]) and radio("Until stopped") and set_number("Speed", 100)
    saved = clicker_saved(lambda c: c["target"]["mode"] == "points" and c["maxClicks"] is None
                          and abs(c["intervalMs"] - 10) < 0.01)
    check("100 clicks per second at a fixed position, until stopped, saved", bool(ok and saved))
    page_tab("Overview")
    activate(TARGET)
    mark = len(pump())
    hotkey("F6")
    started = wait_for(lambda: len(presses(mark)) >= 20, 5)
    time.sleep(0.5)
    n1, c1, t1 = len(presses(mark)), cpu_seconds(pid), time.time()
    slowest = 0.0
    for tab, marker in (("Macros", "Recorded or written sequences"), ("Profiles", "Each profile keeps"),
                        ("Settings", "They work in every program"), ("Overview", "Profile “")):
        t = time.time()
        shown = page_tab(tab) and wait_for(lambda: marker in text(pid), 10)
        slowest = max(slowest, (time.time() - t) if shown else 99.0)
    time.sleep(max(0.0, 2.0 - (time.time() - t1)))
    n2, c2, t2 = len(presses(mark)), cpu_seconds(pid), time.time()
    cpu = (c2 - c1) / (t2 - t1)
    # Timed by the X server, as the clicks reached the window: this driver
    # reads its events late or in bursts, so its own clock would not do.
    stamps = press_times(mark)[n1:n2]
    rate = (len(stamps) - 1) * 1000 / (stamps[-1] - stamps[0]) if len(stamps) > 1 and stamps[-1] > stamps[0] else 0
    check("it clicks about 100 times per second (85 to 115, timed by the X server) while the page is used",
          bool(started) and 85 <= rate <= 115,
          f"{rate:.1f} per second over {len(stamps)} clicks; this driver counted {(n2 - n1) / (t2 - t1):.0f} per second")
    check("switching tabs meanwhile takes under 3 s each", slowest < 3.0, f"slowest {slowest:.2f} s")
    check("sotyware's own process uses under half a CPU core meanwhile", cpu < 0.5, f"{cpu * 100:.0f} % of one core")
    t = time.time()
    ok = press(pid, "Stop") and wait_for(idle, 5)
    took = time.time() - t
    n = len(presses(mark))
    time.sleep(0.5)
    check("the page's Stop answers within 1 s: no click after, nothing held",
          bool(ok) and took < 1.0 and len(presses(mark)) == n and none_held(), f"{took:.2f} s, held {held()}")

    # ---- turning it off gives the shortcuts back ----
    ok = link(pid, "Settings") and wait_for(
        lambda: any(n == "Features" for n, _ in by_role(pid, Atspi.Role.PAGE_TAB)), 5)
    ok = ok and page_tab("Features") and wait_for(
        lambda: any(n == "Autoclicker" for n, _ in by_role(pid, Atspi.Role.CHECK_BOX)), 5)
    check("Settings › Features lists the autoclicker as running",
          bool(ok and re.search(r"Autoclicker [0-9.]+ · Running", text(pid))), text(pid)[:600])
    ok = ok and checkbox(pid, "Autoclicker") and wait_for(
        lambda: re.search(r"Autoclicker [0-9.]+ · Disabled", text(pid)), 10)
    check("Settings › Features turns the autoclicker off", bool(ok), text(pid)[:600])
    free = [f"{'Shift+' if m else ''}{k}" for k, m in SHORTCUTS if can_grab(k, m)]
    check("every shortcut is released: this driver can grab all six again", len(free) == 6, f"free: {free}")
    activate(TARGET)
    mark = len(pump())
    hotkey("F6")
    time.sleep(1.0)
    check("F6 does nothing once it is off", not presses(mark))
    ok = link(pid, "Autoclicker") and wait_for(lambda: "The autoclicker is off." in text(pid), 10)
    check("the page says it is off again", bool(ok))

    # ---- while it is off, F8 reaches the field, and is refused for another action ----
    ok = page_tab("Settings") and set_shortcut(TOGGLE, lambda: key(keysym("F8")))
    refused = ok and wait_for(lambda: 'F8 is already the shortcut of "Emergency stop"' in text(pid), 10)
    check("while it is off, F8 pressed in the toggle's field is refused: it is the emergency stop's",
          bool(refused) and binding(profile_doc(), "toggle-clicker") == "F6", text(pid)[:900])

    # ---- in a Wayland session the page says it is not available, and why ----
    code = quit_app()
    ok = code == 0 and start_app({"XDG_SESSION_TYPE": "wayland"}) and link(pid, "Autoclicker") \
        and wait_for(lambda: "Not available on this computer." in text(pid), 15)
    t = text(pid)
    check("in a Wayland session the page says the autoclicker is not available, why, and offers no Turn on",
          bool(ok) and "uses Wayland, which does not let applications send or record input for other programs" in t
          and "Macros and profiles can still be edited, imported and exported." in t and "Turn on" not in btns(),
          t[:900])
    ok = link(pid, "Settings") and wait_for(
        lambda: any(n == "Features" for n, _ in by_role(pid, Atspi.Role.PAGE_TAB)), 5) and page_tab("Features") \
        and checkbox(pid, "Autoclicker") and wait_for(lambda: re.search(r"Autoclicker [0-9.]+ · Failed", text(pid)), 10)
    check("switched on there, the feature fails with the reason, and holds no shortcut",
          bool(ok) and "Wayland" in text(pid) and all(can_grab(k, m) for k, m in SHORTCUTS), text(pid)[:900])
    check("process still running", p.poll() is None)
    logtext = open(log, errors="replace").read()
    check("no panic or failed session in sotyware's output",
          "panicked" not in logtext and not re.search(r"sotyware: (clicker|playback|recording) failed", logtext),
          logtext[-600:])
finally:
    if "TARGET" in globals() and TARGET:
        X.XDestroyWindow(dpy, TARGET)
        X.XSync(dpy, 0)
