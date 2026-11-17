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
# macro (checked in automation.sqlite3: no F9 or F10 in it) and Play
# replays it into the test window. After every stop no button or key is
# held. Turning the feature off in Settings › Features releases every
# shortcut, and F6 then does nothing.
import re

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

    # ---- turning it off gives the shortcuts back ----
    ok = link(pid, "Settings") and wait_for(
        lambda: any(n == "Features" for n, _ in by_role(pid, Atspi.Role.PAGE_TAB)), 5)
    ok = ok and page_tab("Features") and wait_for(
        lambda: any(n == "Autoclicker" for n, _ in by_role(pid, Atspi.Role.CHECK_BOX)), 5)
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
    check("process still running", p.poll() is None)
    logtext = open(log, errors="replace").read()
    check("no panic or failed session in sotyware's output",
          "panicked" not in logtext and not re.search(r"sotyware: (clicker|playback|recording) failed", logtext),
          logtext[-600:])
finally:
    if "TARGET" in globals() and TARGET:
        X.XDestroyWindow(dpy, TARGET)
        X.XSync(dpy, 0)
