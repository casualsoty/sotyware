# The notes overlay phase of cloud_ui.py (ADR-052), run by it with exec()
# in its own namespace (pid, p, check, text, by_role, buttons, link, press,
# wait_for, key, keysym, ctrl, type_text, focus_window, windows_of,
# close_windows, X, XT, dpy, W, XEvent, ERR, PROFILE, env, log, BIN, ...).
# Kept in its own file only for length.
#
# Drives the release binary (a test-profile build, ADR-027) on the private
# Xvfb display of cloud-ui.sh, with openbox as the window manager and no
# tray (dashboard-only mode). Input is real, through XTest, but only on
# this display, which no person uses. A plain window of this driver plays
# the game: it records every click it receives.
#
# Off by default: no overlay window, nothing grabbed (this driver can grab
# the suggested shortcuts itself). Settings › Notes overlay › Turn on shows
# the overlay at once without taking the keyboard: above other windows
# (_NET_WM_STATE_ABOVE), without a frame or taskbar entry, at the top right
# of the screen, 24 px in, 360 × 440; the five shortcuts are registered
# (now this driver cannot grab them). A note is created in the overlay, its
# title and a checklist typed as real keys (Enter continues the list), and
# autosaved to notes.sqlite3; ticking a checkbox saves at once. From the
# game window: Ctrl+Shift+F1 hides and shows the overlay, the game keeps
# the keyboard; interactive, a click on the overlay stays there; after
# Ctrl+Shift+F2 (passive) the same click reaches the game below, and
# Ctrl+Shift+F2 again makes it interactive with the keyboard; Ctrl+Shift+F3
# creates a note and puts the cursor in its title; Ctrl+Shift+F5 and
# Ctrl+Shift+F4 switch notes. Collapse leaves only the title bar, Expand
# brings the note back; a new corner in the settings moves the open
# overlay there; its close button, and the window manager's close (Alt+F4),
# hide it without destroying anything; Show overlay brings it back. After a
# restart the overlay stays hidden (it is not set to show at startup) but
# its shortcuts work, and it opens where it was with its notes. Turning the
# feature off destroys the window and releases every shortcut; the notes
# stay.
import test_profile as tp  # cloud_ui.py put this folder on sys.path

exec(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "cloud_ui_gtk.py"), encoding="utf-8").read())

NOTES_DB = PROFILE.db("notes.sqlite3")
# Checklists are typed: "[" and "]" by their keysym names.
NAMES.update({"[": "bracketleft", "]": "bracketright"})
ROOT = X.XDefaultRootWindow(dpy)
TITLE = "sotyware notes (test profile)"
SCREEN = (1280, 900)  # cloud-ui.sh's Xvfb


def atom(name):
    return X.XInternAtom(dpy, name.encode(), 0)


NET_ACTIVE = atom("_NET_ACTIVE_WINDOW")
NET_WM_STATE = atom("_NET_WM_STATE")
STATE_ABOVE = atom("_NET_WM_STATE_ABOVE")
STATE_SKIP_TASKBAR = atom("_NET_WM_STATE_SKIP_TASKBAR")
NET_WM_NAME = atom("_NET_WM_NAME")
UTF8 = atom("UTF8_STRING")
NET_FRAME_EXTENTS = atom("_NET_FRAME_EXTENTS")

X.XCreateSimpleWindow.restype = W
X.XCreateSimpleWindow.argtypes = [ctypes.c_void_p, W, ctypes.c_int, ctypes.c_int, ctypes.c_uint, ctypes.c_uint,
                                  ctypes.c_uint, ctypes.c_ulong, ctypes.c_ulong]
X.XSelectInput.argtypes = [ctypes.c_void_p, W, ctypes.c_long]
X.XMapWindow.argtypes = [ctypes.c_void_p, W]
X.XStoreName.argtypes = [ctypes.c_void_p, W, ctypes.c_char_p]
X.XChangeProperty.argtypes = [ctypes.c_void_p, W, W, W, ctypes.c_int, ctypes.c_int, ctypes.c_void_p, ctypes.c_int]
X.XSetWMNormalHints.argtypes = [ctypes.c_void_p, W, ctypes.c_void_p]
X.XPending.argtypes = [ctypes.c_void_p]
X.XNextEvent.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
X.XTranslateCoordinates.argtypes = [ctypes.c_void_p, W, W, ctypes.c_int, ctypes.c_int, ctypes.POINTER(ctypes.c_int),
                                    ctypes.POINTER(ctypes.c_int), ctypes.POINTER(W)]
X.XGrabKey.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_uint, W, ctypes.c_int, ctypes.c_int, ctypes.c_int]
X.XUngrabKey.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_uint, W]
X.XFree.argtypes = [ctypes.c_void_p]

BUTTON_PRESS = 4
CTRL_MASK, SHIFT_MASK = 4, 1


class XSizeHints(ctypes.Structure):
    _fields_ = [("flags", ctypes.c_long), ("x", ctypes.c_int), ("y", ctypes.c_int), ("width", ctypes.c_int),
                ("height", ctypes.c_int), ("min_width", ctypes.c_int), ("min_height", ctypes.c_int),
                ("max_width", ctypes.c_int), ("max_height", ctypes.c_int), ("width_inc", ctypes.c_int),
                ("height_inc", ctypes.c_int), ("min_aspect_x", ctypes.c_int), ("min_aspect_y", ctypes.c_int),
                ("max_aspect_x", ctypes.c_int), ("max_aspect_y", ctypes.c_int), ("base_width", ctypes.c_int),
                ("base_height", ctypes.c_int), ("win_gravity", ctypes.c_int)]


class XButtonEvent(ctypes.Structure):
    _fields_ = [("type", ctypes.c_int), ("serial", ctypes.c_ulong), ("send_event", ctypes.c_int),
                ("display", ctypes.c_void_p), ("window", W), ("root", W), ("subwindow", W),
                ("time", ctypes.c_ulong), ("x", ctypes.c_int), ("y", ctypes.c_int),
                ("x_root", ctypes.c_int), ("y_root", ctypes.c_int), ("state", ctypes.c_uint),
                ("detail", ctypes.c_uint), ("same_screen", ctypes.c_int)]


class AnyXEvent(ctypes.Union):
    _fields_ = [("button", XButtonEvent), ("pad", ctypes.c_long * 24)]


class XErrorEvent(ctypes.Structure):
    _fields_ = [("type", ctypes.c_int), ("display", ctypes.c_void_p), ("resourceid", W),
                ("serial", ctypes.c_ulong), ("error_code", ctypes.c_ubyte), ("request_code", ctypes.c_ubyte),
                ("minor_code", ctypes.c_ubyte)]


# ---- X window properties ----
def prop_values(w, prop, kind):
    """The 32-bit values of property `prop` of type `kind` on `w`."""
    t, data = W(), ctypes.c_void_p()
    fmt, n, after = ctypes.c_int(), ctypes.c_ulong(), ctypes.c_ulong()
    if X.XGetWindowProperty(dpy, w, prop, 0, 64, 0, kind, ctypes.byref(t), ctypes.byref(fmt),
                            ctypes.byref(n), ctypes.byref(after), ctypes.byref(data)) != 0 or not data.value:
        return []
    values = [ctypes.cast(data, ctypes.POINTER(ctypes.c_ulong))[i] for i in range(n.value)]
    X.XFree(data)
    return values


def utf8_name(w):
    t, data = W(), ctypes.c_void_p()
    fmt, n, after = ctypes.c_int(), ctypes.c_ulong(), ctypes.c_ulong()
    if X.XGetWindowProperty(dpy, w, NET_WM_NAME, 0, 256, 0, UTF8, ctypes.byref(t), ctypes.byref(fmt),
                            ctypes.byref(n), ctypes.byref(after), ctypes.byref(data)) != 0 or not data.value:
        return ""
    name = ctypes.string_at(data, n.value).decode("utf-8", "replace")
    X.XFree(data)
    return name


def attrs(w):
    a = _WinAttrs()
    return a if X.XGetWindowAttributes(dpy, w, ctypes.byref(a)) else None


def overlay_window():
    """The overlay's X window (mapped or not), or None while none exists."""
    for w in windows_of(pid):
        if utf8_name(w) == TITLE:
            return w
    return None


def mapped(w):
    a = attrs(w) if w else None
    return a is not None and a.map_state == 2


def overlay_shown():
    w = overlay_window()
    return w if w and mapped(w) else None


def origin(w):
    x, y, child = ctypes.c_int(), ctypes.c_int(), W()
    X.XTranslateCoordinates(dpy, w, ROOT, 0, 0, ctypes.byref(x), ctypes.byref(y), ctypes.byref(child))
    return x.value, y.value


def geometry(w):
    a = attrs(w)
    return (*origin(w), a.width, a.height) if a else None


def near(g, want, slack=2):
    return g is not None and all(abs(a - b) <= slack for a, b in zip(g, want))


def active_window():
    values = prop_values(ROOT, NET_ACTIVE, 33)  # XA_WINDOW
    return values[0] if values else 0


def top_level(w):
    """The child of the root that holds `w` (its window manager frame)."""
    while True:
        root, parent, kids, nk = W(), W(), ctypes.POINTER(W)(), ctypes.c_uint()
        if not X.XQueryTree(dpy, w, ctypes.byref(root), ctypes.byref(parent), ctypes.byref(kids), ctypes.byref(nk)):
            return w
        if nk.value:
            X.XFree(kids)
        if parent.value in (0, ROOT):
            return w
        w = parent.value


def above(a, b):
    """Whether window `a` is stacked above window `b` (root's children are
    listed bottom to top)."""
    root, parent, kids, nk = W(), W(), ctypes.POINTER(W)(), ctypes.c_uint()
    if not X.XQueryTree(dpy, ROOT, ctypes.byref(root), ctypes.byref(parent), ctypes.byref(kids), ctypes.byref(nk)):
        return False
    order = [kids[i] for i in range(nk.value)]
    if nk.value:
        X.XFree(kids)
    ta, tb = top_level(a), top_level(b)
    return ta in order and tb in order and order.index(ta) > order.index(tb)


# ---- the game: a plain window of this driver ----
CLICKS = []  # (x_root, y_root) of every button press it got


def make_game(x, y, width, height):
    w = X.XCreateSimpleWindow(dpy, ROOT, x, y, width, height, 0, 0, 0x303030)
    X.XStoreName(dpy, w, b"sotyware e2e game")
    me = (ctypes.c_ulong * 1)(os.getpid())
    X.XChangeProperty(dpy, w, NET_WM_PID, 6, 32, 0, me, 1)
    hints = XSizeHints()
    hints.flags = 1 | 2 | 16 | 32  # USPosition | USSize | PMinSize | PMaxSize
    hints.x, hints.y, hints.width, hints.height = x, y, width, height
    hints.min_width = hints.max_width = width
    hints.min_height = hints.max_height = height
    X.XSetWMNormalHints(dpy, w, ctypes.byref(hints))
    X.XSelectInput(dpy, w, 4)  # ButtonPressMask
    X.XMapWindow(dpy, w)
    X.XSync(dpy, 0)
    wait_for(lambda: mapped(w), 5)
    return w


def pump():
    ev = AnyXEvent()
    while X.XPending(dpy) > 0:
        X.XNextEvent(dpy, ctypes.byref(ev))
        if ev.button.type == BUTTON_PRESS and ev.button.window == GAME:
            CLICKS.append((ev.button.x_root, ev.button.y_root))
    return CLICKS


def activate(w):
    """Asks the window manager to bring `w` to the front, as a taskbar
    would (_NET_ACTIVE_WINDOW, source 2)."""
    ev = XEvent()
    ev.xclient.type, ev.xclient.window, ev.xclient.message_type, ev.xclient.format = 33, w, NET_ACTIVE, 32
    ev.xclient.data[0], ev.xclient.data[1] = 2, 0
    X.XSendEvent(dpy, ROOT, 0, (1 << 20) | (1 << 19), ctypes.byref(ev))
    X.XSync(dpy, 0)
    return wait_for(lambda: active_window() == w, 5)


def chord(mods, name):
    """Presses the modifiers, then the key, as real keys, and lets go."""
    codes = [keysym(m) for m in mods]
    for c in codes:
        XT.XTestFakeKeyEvent(dpy, c, 1, 0)
    k = keysym(name)
    XT.XTestFakeKeyEvent(dpy, k, 1, 0)
    XT.XTestFakeKeyEvent(dpy, k, 0, 0)
    for c in reversed(codes):
        XT.XTestFakeKeyEvent(dpy, c, 0, 0)
    X.XSync(dpy, 0)


def shortcut(name):
    chord(["Control_L", "Shift_L"], name)


GRAB_ERRORS = []


def _on_grab_error(_d, e):
    GRAB_ERRORS.append(ctypes.cast(e, ctypes.POINTER(XErrorEvent)).contents.error_code)
    return 0


_grab_handler = ERR(_on_grab_error)


def can_grab(name, mods):
    """Whether this driver can grab the key on the root window itself: not
    while sotyware holds that shortcut."""
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


SUGGESTED = ["F1", "F2", "F3", "F4", "F5"]  # with Ctrl+Shift


def none_grabbable():
    return not any(can_grab(k, CTRL_MASK | SHIFT_MASK) for k in SUGGESTED)


def all_grabbable():
    return all(can_grab(k, CTRL_MASK | SHIFT_MASK) for k in SUGGESTED)


# ---- the pages ----
def btns():
    return [b.strip() for b in buttons(pid)]


def page_tab(name):
    for n, node in by_role(pid, Atspi.Role.PAGE_TAB):
        if n.strip() == name:
            return Atspi.Action.do_action(node, 0)
    print(f"  no tab {name!r}; tabs: {[n for n, _ in by_role(pid, Atspi.Role.PAGE_TAB)]}", flush=True)
    return False


def note_tabs():
    """The overlay's note tabs (the settings' tabs have other names), and
    which one is selected."""
    settings_tabs = {"General", "Features", "Notes overlay", "About & updates"}
    out = []
    for n, node in by_role(pid, Atspi.Role.PAGE_TAB):
        if n.strip() not in settings_tabs:
            out.append((n.strip(), node.get_state_set().contains(Atspi.StateType.SELECTED)))
    return out


def selected_note():
    """The one selected note tab (None when none, or more than one, is)."""
    selected = [n for n, sel in note_tabs() if sel]
    return selected[0] if len(selected) == 1 else None


def entry(name):
    for r, n, _, node in nodes(pid):
        if r in (Atspi.Role.ENTRY, Atspi.Role.TEXT) and n == name:
            return node
    return None


def focused(node):
    return node is not None and node.get_state_set().contains(Atspi.StateType.FOCUSED)


def element_center(node):
    x, y, w, h = extents(node)
    return x + w // 2, y + h // 2


def combo(label):
    for role, name, _, node in nodes(pid):
        if role == Atspi.Role.COMBO_BOX and (name == label or name.startswith(label + " ")):
            return node
    return None


def choose(label, option):
    """Chooses `option` in the <select> `label` with the keyboard: WebKitGTK
    fires the change event for keyboard changes."""
    node = wait_for(lambda: combo(label), 5)
    if node is None:
        print(f"  no select {label!r}: {[n for r, n, _, _ in nodes(pid) if r == Atspi.Role.COMBO_BOX]}", flush=True)
        return False
    names = [n.get_name().strip() for n in find(node, Atspi.Role.MENU_ITEM)]
    if option not in names:
        print(f"  select {label!r} has no option {option!r}: {names[:20]}", flush=True)
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


def notes_rows():
    """(title, body, pinned) of every note, by position."""
    try:
        with sqlite3.connect(f"file:{NOTES_DB}?mode=ro", uri=True) as db:
            return db.execute("SELECT title, body, pinned FROM notes ORDER BY pinned DESC, position, id").fetchall()
    except sqlite3.Error:
        return []


def settings_status():
    """The Settings tab's status line."""
    for r, _n, t, _node in nodes(pid):
        if r in (Atspi.Role.PARAGRAPH, Atspi.Role.STATUS_BAR, Atspi.Role.SECTION) and (
                t.strip().startswith("Off.") or t.strip().startswith("Shown") or t.strip().startswith("Hidden")):
            return t.strip()
    return ""


def quit_app():
    close_windows(pid)
    try:
        return p.wait(timeout=60)
    except subprocess.TimeoutExpired:
        return None


def start_app():
    """Starts sotyware again in this run's profile. ADR-027: only a
    test-profile build, acknowledged before anything is clicked."""
    global p, pid
    tp.require_test_profile_build(BIN)
    tp.clear_ack(PROFILE)
    with open(log, "ab") as f:
        p = subprocess.Popen([BIN], env=env, stdout=f, stderr=subprocess.STDOUT)
    pid = p.pid
    try:
        tp.verify_ack(PROFILE, pid, timeout=30, alive=lambda: p.poll() is None)
    except tp.IsolationError as e:
        p.kill()
        raise SystemExit(f"the restarted app is not provably isolated: {e}")
    return wait_for(lambda: any(n == "Settings" for n, _ in by_role(pid, Atspi.Role.LINK)), 30)


def open_overlay_settings():
    return link(pid, "Settings") and wait_for(lambda: page_tab("Notes overlay"), 10) and wait_for(
        lambda: settings_status() != "", 10)


# ---- the phase ----
GAME = 0
# The overlay opens at the top right, 24 px in, 360 × 440 (the defaults).
FIRST = (SCREEN[0] - 24 - 360, 24, 360, 440)

check("off by default: no overlay window", overlay_window() is None)
check("off by default: none of the suggested shortcuts is grabbed (this driver can grab them)", all_grabbable())
ok = open_overlay_settings()
check("Settings › Notes overlay says it is off", bool(ok) and settings_status().startswith("Off."), settings_status())

dash = active_window()
ok = press(pid, "Turn on") and wait_for(overlay_shown, 15)
ov = overlay_window()
check("Turn on shows the overlay at once", bool(ok))
g = geometry(ov) if ov else None
check("at the top right of the screen, 24 px in, 360 × 440", near(g, FIRST), f"geometry {g}, want {FIRST}")
states = prop_values(ov, NET_WM_STATE, 4) if ov else []
check("above other windows (_NET_WM_STATE_ABOVE) and without a taskbar entry",
      STATE_ABOVE in states and STATE_SKIP_TASKBAR in states, f"states {states}")
frame = prop_values(ov, NET_FRAME_EXTENTS, 6) if ov else []
check("without a window frame", frame == [] or frame == [0, 0, 0, 0], f"frame extents {frame}")
check("shown without taking the keyboard: the dashboard keeps it", active_window() == dash,
      f"active {active_window():#x}, dashboard {dash:#x}, overlay {ov or 0:#x}")
check("its five shortcuts are registered (this driver cannot grab any)", wait_for(none_grabbable, 10))
check("the status line says it is shown and interactive",
      wait_for(lambda: settings_status().startswith("Shown, interactive"), 10), settings_status())
check("the overlay has no note yet", wait_for(lambda: "No notes yet." in text(pid), 10))

# ---- a note, typed as real keys into the overlay ----
ok = press(pid, "New note") and wait_for(lambda: note_tabs() == [("New note", True)], 10)
check("New note in the overlay: one tab, selected", bool(ok), str(note_tabs()))
title = entry("Note title")
x, y = element_center(title) if title else (0, 0)
click_xy(x, y)
ok = wait_for(lambda: focused(entry("Note title")), 5)
check("a click in the title gives the overlay the keyboard", bool(ok))
ctrl("a")
type_text("Boss strategy")
key(keysym("Return"))
editor = wait_for(lambda: entry("Note text (Markdown)"), 5)
check("Enter in the title opens the editor", editor is not None and wait_for(lambda: focused(entry("Note text (Markdown)")), 5))
type_text("- [ ] Prepare healing items")
key(keysym("Return"))
type_text("Equip gear")
key(keysym("Escape"))
want = ("Boss strategy", "- [ ] Prepare healing items\n- [ ] Equip gear", 0)
ok = wait_for(lambda: notes_rows() == [want], 10)
check("autosaved to notes.sqlite3, the list continued by Enter", bool(ok), str(notes_rows()))
check("Escape shows the note again, its checkboxes named by their text",
      wait_for(lambda: any(n == "Prepare healing items" for n, _ in by_role(pid, Atspi.Role.CHECK_BOX)), 5))
box = next((node for n, node in by_role(pid, Atspi.Role.CHECK_BOX) if n == "Prepare healing items"), None)
ok = box is not None and Atspi.Action.do_action(box, 0)
ok = ok and wait_for(lambda: notes_rows() and notes_rows()[0][1].startswith("- [x] Prepare healing items"), 5)
check("ticking a checkbox saves at once", bool(ok), str(notes_rows()))

# ---- the game below, and the shortcuts from it ----
ov = overlay_window()
og = geometry(ov)
GAME = make_game(SCREEN[0] - 540, og[1] + 80, 520, 520)
check("the game window is up, partly under the overlay", mapped(GAME))
check("the game has the keyboard", bool(activate(GAME)))
ov = overlay_window()
check("the overlay stays above the game, which was raised after it", above(ov, GAME))

shortcut("F1")
ok = wait_for(lambda: overlay_shown() is None, 5)
check("Ctrl+Shift+F1 from the game hides the overlay (its window kept)", bool(ok) and overlay_window() is not None)
shortcut("F1")
ok = wait_for(overlay_shown, 5)
ok = ok and wait_for(lambda: near(geometry(overlay_window()), og), 5)
time.sleep(0.5)
check("Ctrl+Shift+F1 shows it again where it was, and the game keeps the keyboard",
      bool(ok) and active_window() == GAME,
      f"geometry {geometry(overlay_window()) if overlay_window() else None}, want {og}; "
      f"active {active_window():#x}, game {GAME:#x}")

# A point of the overlay's note, above the game.
og = geometry(overlay_window())
point = (og[0] + og[2] // 2, og[1] + og[3] - 40)
mark = len(pump())
click_xy(*point)
time.sleep(0.6)
check("interactive: a click on the overlay stays there (the game gets nothing)", len(pump()) == mark, str(CLICKS[mark:]))
key(keysym("Escape"))  # the click opened the editor
activate(GAME)
shortcut("F2")
ok = wait_for(lambda: "Click-through" in text(pid), 5)
check("Ctrl+Shift+F2: passive, the overlay says so", bool(ok))
mark = len(pump())
click_xy(*point)
ok = wait_for(lambda: len(pump()) > mark, 3)
check("passive: the same click goes through to the game", bool(ok), f"clicks {CLICKS[mark:]}, point {point}")
check("passive: nothing of the overlay to click (no note tab, no title field)",
      note_tabs() == [] and entry("Note title") is None, str(note_tabs()))
shortcut("F2")
ok = wait_for(lambda: "Click-through" not in text(pid) and note_tabs() != [], 5)
check("Ctrl+Shift+F2 again: interactive", bool(ok))
check("switched by shortcut, the overlay takes the keyboard",
      wait_for(lambda: active_window() == overlay_window(), 5), f"active {active_window():#x}, overlay {overlay_window() or 0:#x}")
mark = len(pump())
click_xy(*point)
time.sleep(0.6)
check("interactive again: the click stays on the overlay", len(pump()) == mark)
key(keysym("Escape"))

# ---- notes by shortcut ----
activate(GAME)
shortcut("F3")
ok = wait_for(lambda: len(notes_rows()) == 2 and selected_note() == "New note", 10)
check("Ctrl+Shift+F3 creates a note and shows it", bool(ok), f"{notes_rows()} {note_tabs()}")
check("…with the cursor in its title", wait_for(lambda: focused(entry("Note title")), 5))
type_text("Route")
ok = wait_for(lambda: [r[0] for r in notes_rows()] == ["Boss strategy", "Route"], 10)
check("its title, typed at once, is saved", bool(ok), str(notes_rows()))
activate(GAME)
shortcut("F5")
check("Ctrl+Shift+F5: the next note (round to the first)", wait_for(lambda: selected_note() == "Boss strategy", 5),
      str(note_tabs()))
shortcut("F4")
check("Ctrl+Shift+F4: the previous note", wait_for(lambda: selected_note() == "Route", 5), str(note_tabs()))
check("switching notes by shortcut leaves the keyboard to the game", active_window() == GAME)

# ---- collapse, placement, hiding ----
ok = press(pid, "Collapse to the title bar") and wait_for(lambda: (geometry(overlay_window()) or (0, 0, 0, 0))[3] <= 40, 5)
check("Collapse leaves only the title bar (38 px high)", bool(ok), str(geometry(overlay_window())))
ok = press(pid, "Expand") and wait_for(lambda: near(geometry(overlay_window()), (*og[:2], 360, 440)), 5)
check("Expand brings it back at its size", bool(ok), str(geometry(overlay_window())))
ok = open_overlay_settings() and choose("Opens at", "Top left")
ok = ok and wait_for(lambda: near(geometry(overlay_window()), (24, 24, 360, 440)), 10)
check("a new corner in the settings moves the open overlay there", bool(ok), str(geometry(overlay_window())))
ok = press(pid, "Hide the overlay (notes are kept)") and wait_for(lambda: overlay_shown() is None, 5)
check("its close button hides it", bool(ok) and overlay_window() is not None)
check("the settings say it is hidden", wait_for(lambda: settings_status().startswith("Hidden"), 5), settings_status())
ok = press(pid, "Show overlay") and wait_for(overlay_shown, 5)
check("Show overlay brings it back where it was", bool(ok) and near(geometry(overlay_window()), (24, 24, 360, 440)))
ov = overlay_window()
ev = XEvent()
ev.xclient.type, ev.xclient.window, ev.xclient.message_type, ev.xclient.format = 33, ov, NET_CLOSE, 32
ev.xclient.data[0], ev.xclient.data[1] = 0, 2
X.XSendEvent(dpy, ROOT, 0, (1 << 20) | (1 << 19), ctypes.byref(ev))
X.XSync(dpy, 0)
ok = wait_for(lambda: overlay_shown() is None, 5)
check("the window manager's close (Alt+F4) hides it too; sotyware runs on",
      bool(ok) and overlay_window() is not None and p.poll() is None)

# ---- a restart ----
code = quit_app()
check("closing the dashboard quits (dashboard-only mode)", code == 0, f"exit code {code}")
started = start_app()
check("sotyware starts again", bool(started))
time.sleep(2)
check("after a restart the overlay stays hidden (not set to show at startup)", overlay_shown() is None)
check("…but its shortcuts are registered again", wait_for(none_grabbable, 10))
shortcut("F1")
ok = wait_for(overlay_shown, 15)
check("Ctrl+Shift+F1 shows it where it was", bool(ok) and near(geometry(overlay_window()), (24, 24, 360, 440)),
      str(geometry(overlay_window()) if overlay_window() else None))
ok = wait_for(lambda: [n for n, _ in note_tabs()] == ["Boss strategy", "Route"], 10)
check("with its notes", bool(ok), str(note_tabs()))
page = "Prepare healing items" in text(pid) or (page_tab("Boss strategy") and wait_for(
    lambda: "Prepare healing items" in text(pid), 5))
check("and their text", bool(page))

# ---- turned off ----
ok = link(pid, "Settings") and page_tab("Features") and wait_for(
    lambda: any(n == "Gaming notes overlay" for n, _ in by_role(pid, Atspi.Role.CHECK_BOX)), 10)
box = next((node for n, node in by_role(pid, Atspi.Role.CHECK_BOX) if n == "Gaming notes overlay"), None)
ok = ok and box is not None and Atspi.Action.do_action(box, 0)
ok = ok and wait_for(lambda: overlay_window() is None, 10)
check("turned off in Settings › Features: the overlay window is gone", bool(ok))
check("…and every shortcut is released (this driver can grab them all)", wait_for(all_grabbable, 10))
check("the notes stay in notes.sqlite3", [r[0] for r in notes_rows()] == ["Boss strategy", "Route"], str(notes_rows()))
check("sotyware is still running", p.poll() is None)
