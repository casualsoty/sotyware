"""Drives the real sotyware binary and checks single-instance + lifecycle behaviour.

usage: single_instance.py <binary> <sni-registrations-file>
Run it through run.sh, which provides a private D-Bus session bus, a stub tray host
(sni_watcher.py) and GDK_BACKEND=x11. This is a SIMULATED environment: the tray
host is a stub, and the window manager is whatever serves $DISPLAY.

Each check prints PASS, FAIL or SKIP. SKIP means the environment could not
exercise that behaviour (for example a window manager that ignores minimize
requests); it is never counted as a pass.
"""
import ctypes
import ctypes.util
import os
import subprocess
import sys
import time

import dbus

BIN, REG = os.path.abspath(sys.argv[1]), sys.argv[2]
results = []


def check(name, ok, detail=""):
    status = "SKIP" if ok is None else ("PASS" if ok else "FAIL")
    results.append((name, status))
    print(f"[{status}] {name}{' — ' + detail if detail else ''}", flush=True)


# ---------- X11 via ctypes (no extra Python packages needed) ----------
x = ctypes.cdll.LoadLibrary(ctypes.util.find_library("X11"))
W = ctypes.c_ulong
x.XOpenDisplay.restype = ctypes.c_void_p
x.XOpenDisplay.argtypes = [ctypes.c_char_p]
x.XDefaultRootWindow.restype = W
x.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
x.XDefaultScreen.argtypes = [ctypes.c_void_p]
x.XInternAtom.restype = W
x.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
x.XQueryTree.argtypes = [ctypes.c_void_p, W, ctypes.POINTER(W), ctypes.POINTER(W),
                         ctypes.POINTER(ctypes.POINTER(W)), ctypes.POINTER(ctypes.c_uint)]
x.XFetchName.argtypes = [ctypes.c_void_p, W, ctypes.POINTER(ctypes.c_char_p)]
x.XFree.argtypes = [ctypes.c_void_p]
x.XGetWindowProperty.argtypes = [ctypes.c_void_p, W, W, ctypes.c_long, ctypes.c_long, ctypes.c_int, W,
                                 ctypes.POINTER(W), ctypes.POINTER(ctypes.c_int),
                                 ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_ulong),
                                 ctypes.POINTER(ctypes.c_void_p)]
x.XFlush.argtypes = [ctypes.c_void_p]
x.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
x.XSendEvent.argtypes = [ctypes.c_void_p, W, ctypes.c_int, ctypes.c_long, ctypes.c_void_p]
x.XIconifyWindow.argtypes = [ctypes.c_void_p, W, ctypes.c_int]
x.XGetInputFocus.argtypes = [ctypes.c_void_p, ctypes.POINTER(W), ctypes.POINTER(ctypes.c_int)]


class XWindowAttributes(ctypes.Structure):
    _fields_ = [("x", ctypes.c_int), ("y", ctypes.c_int), ("width", ctypes.c_int), ("height", ctypes.c_int),
                ("border_width", ctypes.c_int), ("depth", ctypes.c_int), ("visual", ctypes.c_void_p),
                ("root", W), ("class_", ctypes.c_int), ("bit_gravity", ctypes.c_int),
                ("win_gravity", ctypes.c_int), ("backing_store", ctypes.c_int),
                ("backing_planes", ctypes.c_ulong), ("backing_pixel", ctypes.c_ulong),
                ("save_under", ctypes.c_int), ("colormap", W), ("map_installed", ctypes.c_int),
                ("map_state", ctypes.c_int), ("all_event_masks", ctypes.c_long),
                ("your_event_mask", ctypes.c_long), ("do_not_propagate_mask", ctypes.c_long),
                ("override_redirect", ctypes.c_int), ("screen", ctypes.c_void_p)]


class XClientMessageEvent(ctypes.Structure):
    _fields_ = [("type", ctypes.c_int), ("serial", ctypes.c_ulong), ("send_event", ctypes.c_int),
                ("display", ctypes.c_void_p), ("window", W), ("message_type", W),
                ("format", ctypes.c_int), ("data", ctypes.c_long * 5)]


x.XGetWindowAttributes.argtypes = [ctypes.c_void_p, W, ctypes.POINTER(XWindowAttributes)]

# Windows can disappear between XQueryTree and the next request (e.g. the brief
# leader windows of a second instance); Xlib's default handler would abort on that.
XErrorHandler = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_void_p, ctypes.c_void_p)
X_ERROR_HANDLER = XErrorHandler(lambda d, e: 0)  # module-level: must outlive the call
x.XSetErrorHandler.argtypes = [XErrorHandler]
x.XSetErrorHandler.restype = ctypes.c_void_p
x.XSetErrorHandler(X_ERROR_HANDLER)
IS_VIEWABLE = 2
ICONIC_STATE = 3
dpy = x.XOpenDisplay(None)
assert dpy, "cannot open X display"
ROOT = x.XDefaultRootWindow(dpy)
A_PID = x.XInternAtom(dpy, b"_NET_WM_PID", 0)
A_WM_STATE = x.XInternAtom(dpy, b"WM_STATE", 0)
A_ACTIVE = x.XInternAtom(dpy, b"_NET_ACTIVE_WINDOW", 0)
A_PROTOCOLS = x.XInternAtom(dpy, b"WM_PROTOCOLS", 0)
A_DELETE = x.XInternAtom(dpy, b"WM_DELETE_WINDOW", 0)


def children(w):
    r, p, ch, n = W(), W(), ctypes.POINTER(W)(), ctypes.c_uint()
    if not x.XQueryTree(dpy, w, ctypes.byref(r), ctypes.byref(p), ctypes.byref(ch), ctypes.byref(n)):
        return []
    out = [ch[i] for i in range(n.value)]
    if ch:
        x.XFree(ch)
    return out


def prop_long(w, atom):
    t, f, n, rest, data = W(), ctypes.c_int(), ctypes.c_ulong(), ctypes.c_ulong(), ctypes.c_void_p()
    if x.XGetWindowProperty(dpy, w, atom, 0, 1, 0, 0, ctypes.byref(t), ctypes.byref(f), ctypes.byref(n),
                            ctypes.byref(rest), ctypes.byref(data)) != 0 or not data.value:
        return None
    v = ctypes.cast(data, ctypes.POINTER(ctypes.c_ulong))[0] if n.value else None
    x.XFree(data)
    return v


def name(w):
    s = ctypes.c_char_p()
    if x.XFetchName(dpy, w, ctypes.byref(s)) and s.value:
        return s.value.decode()
    return None


def attrs(w):
    a = XWindowAttributes()
    x.XGetWindowAttributes(dpy, w, ctypes.byref(a))
    return a


def all_dashboards():
    """Client windows titled 'sotyware' (any pid), top level or inside a WM frame.

    Excludes GTK's tiny client-leader window (10x10, never mapped).
    """
    candidates = []
    for top in children(ROOT):
        candidates.append(top)
        candidates.extend(children(top))
    return [w for w in candidates if name(w) == "sotyware" and attrs(w).width > 100]


def dashboards(pid):
    return [w for w in all_dashboards() if prop_long(w, A_PID) == pid]


def viewable(w):
    return attrs(w).map_state == IS_VIEWABLE


def iconic(w):
    return prop_long(w, A_WM_STATE) == ICONIC_STATE


def wait_for(pred, timeout=8.0):
    end = time.time() + timeout
    while time.time() < end:
        x.XSync(dpy, 0)
        if pred():
            return True
        time.sleep(0.1)
    return False


def send_wm_delete(w):
    """Exactly what a window manager sends when the user clicks the title-bar X."""
    ev = XClientMessageEvent(type=33, send_event=1, display=dpy, window=w, message_type=A_PROTOCOLS, format=32)
    ev.data[0] = A_DELETE
    x.XSendEvent(dpy, w, 0, 0, ctypes.byref(ev))
    x.XFlush(dpy)


x.XCreateSimpleWindow.restype = W
x.XCreateSimpleWindow.argtypes = [ctypes.c_void_p, W, ctypes.c_int, ctypes.c_int, ctypes.c_uint,
                                  ctypes.c_uint, ctypes.c_uint, ctypes.c_ulong, ctypes.c_ulong]
x.XMapWindow.argtypes = [ctypes.c_void_p, W]
x.XStoreName.argtypes = [ctypes.c_void_p, W, ctypes.c_char_p]
x.XDestroyWindow.argtypes = [ctypes.c_void_p, W]


def request_activate(w):
    """EWMH _NET_ACTIVE_WINDOW request, as a pager or taskbar would send it."""
    ev = XClientMessageEvent(type=33, send_event=1, display=dpy, window=w, message_type=A_ACTIVE, format=32)
    ev.data[0] = 2  # source: pager
    # SubstructureRedirectMask | SubstructureNotifyMask
    x.XSendEvent(dpy, ROOT, 0, (1 << 20) | (1 << 19), ctypes.byref(ev))
    x.XFlush(dpy)


def focused_on(w):
    """True/False if focus can be observed, None if the WM exposes neither signal."""
    active = prop_long(ROOT, A_ACTIVE)
    if active:
        return active == w or w in children(active)
    fw, rev = W(), ctypes.c_int()
    x.XGetInputFocus(dpy, ctypes.byref(fw), ctypes.byref(rev))
    if fw.value in (0, 1):  # None / PointerRoot: nothing to compare against
        return None
    return fw.value == w or fw.value in children(w)


# ---------- processes ----------
def sotyware_pids():
    """PIDs of every process running this exact binary (via /proc/<pid>/exe)."""
    pids = []
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            if os.path.realpath(f"/proc/{entry}/exe") == BIN:
                pids.append(int(entry))
        except OSError:
            pass
    return sorted(pids)


def tray_registrations():
    if not os.path.exists(REG):
        return []
    with open(REG) as f:
        return [l.split() for l in f if l.strip()]


def second_launch(label):
    """Launches the binary again; returns (exit code, output, new windows seen)."""
    before = set(all_dashboards())
    p = subprocess.Popen([BIN], stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    seen_new = set()
    end = time.time() + 15
    while p.poll() is None and time.time() < end:
        x.XSync(dpy, 0)
        seen_new |= set(all_dashboards()) - before
        time.sleep(0.02)
    try:
        code = p.wait(timeout=1)
    except subprocess.TimeoutExpired:
        p.kill()
        code = None
    out = p.stdout.read().decode(errors="replace").strip()
    if out:
        print(f"    ({label} output: {out})")
    return code, seen_new


# ---------- D-Bus tray menu ----------
bus = dbus.SessionBus()


def tray_menu():
    sender, path = tray_registrations()[-1]
    item = bus.get_object(sender, path)
    props = dbus.Interface(item, dbus.PROPERTIES_IFACE).GetAll("org.kde.StatusNotifierItem")
    menu = dbus.Interface(bus.get_object(sender, props["Menu"]), "com.canonical.dbusmenu")
    _, layout = menu.GetLayout(0, -1, dbus.Array([], signature="s"))
    labels = {}
    for child in layout[2]:
        cid, cprops = child[0], child[1]
        if "label" in cprops:
            labels[str(cprops["label"]).replace("_", "")] = int(cid)
    return menu, labels


def tray_click(menu, labels, label):
    menu.Event(dbus.Int32(labels[label]), "clicked", dbus.Int32(0, variant_level=1), dbus.UInt32(0))


def expect_reused(label, win, primary_pid):
    """A further launch must exit 0, create nothing, and leave one visible dashboard."""
    code, new_windows = second_launch(label)
    check(f"{label}: new process exits with code 0", code == 0, f"exit code={code}")
    check(f"{label}: no other window appeared, even briefly", not new_windows,
          f"new={[hex(w) for w in new_windows]}")
    check(f"{label}: only the original process is running", sotyware_pids() == [primary_pid],
          f"pids={sotyware_pids()}")
    check(f"{label}: still exactly one tray registration", len(tray_registrations()) == 1,
          f"registrations={len(tray_registrations())}")
    check(f"{label}: existing dashboard visible", wait_for(lambda: viewable(win)))
    check(f"{label}: still one dashboard window", all_dashboards() == [win])


# ---------- scenario ----------
assert not sotyware_pids(), f"sotyware already running from {BIN}: {sotyware_pids()}"

# 1. First launch.
proc = subprocess.Popen([BIN], stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
alive = lambda: proc.poll() is None
ok = wait_for(lambda: len(dashboards(proc.pid)) == 1 and viewable(dashboards(proc.pid)[0]), 20)
check("1 first launch: process running and dashboard visible", ok and alive())
win = dashboards(proc.pid)[0]
ok = wait_for(lambda: len(tray_registrations()) == 1, 10)
menu, labels = tray_menu()
check("1 first launch: one tray item with Open Dashboard / Quit",
      ok and set(labels) == {"Open Dashboard", "Quit"}, f"labels={list(labels)}")
time.sleep(2)  # let the webview finish loading

# 5. Second launch while the dashboard is visible but another window has focus.
# 2 + 3 are asserted on every further launch inside expect_reused().
other = x.XCreateSimpleWindow(dpy, ROOT, 50, 50, 300, 200, 0, 0, 0xFFFFFF)
x.XStoreName(dpy, other, b"focus-thief")
x.XMapWindow(dpy, other)
x.XFlush(dpy)
wait_for(lambda: viewable(other), 4)
stolen = wait_for(lambda: focused_on(other) is True and focused_on(win) is False, 4)
# Control: does this window manager honour activation requests at all? Some
# (e.g. WSLg's Weston XWayland WM) only focus a window when it is mapped.
honours_activation = False
if stolen:
    request_activate(win)
    honours_activation = wait_for(lambda: focused_on(win) is True, 2)
    if honours_activation:
        request_activate(other)
        stolen = wait_for(lambda: focused_on(other) is True, 2)
expect_reused("5 launch while visible", win, proc.pid)
if stolen and honours_activation:
    check("5 launch while visible: focus moves back to the dashboard",
          wait_for(lambda: focused_on(win) is True, 4))
else:
    check("5 launch while visible: focus moves back to the dashboard", None,
          "window manager ignores activation requests" if stolen
          else "could not move focus to another window first")
x.XDestroyWindow(dpy, other)
x.XFlush(dpy)

# 4. Second launch while hidden (close button turns into hide).
send_wm_delete(win)
check("8 close: dashboard hidden, not destroyed",
      wait_for(lambda: not viewable(win)) and all_dashboards() == [win] and alive())
expect_reused("4 launch while hidden", win, proc.pid)
f = focused_on(win) if wait_for(lambda: focused_on(win) is True, 4) else focused_on(win)
check("4 launch while hidden: restored dashboard has focus", f,
      "focus not observable on this display" if f is None else "")

# 6. Second launch while minimized.
x.XIconifyWindow(dpy, win, x.XDefaultScreen(dpy))
x.XFlush(dpy)
if wait_for(lambda: iconic(win) or not viewable(win), 4):
    expect_reused("6 launch while minimized", win, proc.pid)
    check("6 launch while minimized: no longer iconic", wait_for(lambda: not iconic(win)))
else:
    check("6 launch while minimized", None, "window manager ignored the minimize request")

# 7. Many more launches, sequential then concurrent.
for i in range(5):
    if i % 2 == 0:
        send_wm_delete(win)
        wait_for(lambda: not viewable(win))
    expect_reused(f"7 sequential launch #{i + 1}", win, proc.pid)

send_wm_delete(win)
wait_for(lambda: not viewable(win))
before = set(all_dashboards())
burst = [subprocess.Popen([BIN], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL) for _ in range(4)]
codes = []
for p in burst:
    try:
        codes.append(p.wait(timeout=15))
    except subprocess.TimeoutExpired:
        p.kill()
        codes.append(None)
check("7 four concurrent launches: all exit with code 0", codes == [0, 0, 0, 0], f"codes={codes}")
check("7 four concurrent launches: one process, one tray item, one window",
      sotyware_pids() == [proc.pid] and len(tray_registrations()) == 1 and all_dashboards() == [win],
      f"pids={sotyware_pids()} tray={len(tray_registrations())}")
check("7 four concurrent launches: dashboard restored", wait_for(lambda: viewable(win)))

# Tray "Open Dashboard" still works alongside the plugin.
send_wm_delete(win)
wait_for(lambda: not viewable(win))
menu, labels = tray_menu()
tray_click(menu, labels, "Open Dashboard")
check("tray Open Dashboard: restores the same window",
      wait_for(lambda: viewable(win)) and all_dashboards() == [win])

# 9. Quit from the tray, then confirm the lock was released.
tray_click(menu, labels, "Quit")
try:
    code = proc.wait(timeout=10)
except subprocess.TimeoutExpired:
    code = None
check("9 tray Quit: process exits with code 0", code == 0, f"exit code={code}")
check("9 tray Quit: no sotyware process left", wait_for(lambda: not sotyware_pids(), 5),
      f"pids={sotyware_pids()}")

open(REG, "w").close()
again = subprocess.Popen([BIN], stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
ok = wait_for(lambda: len(dashboards(again.pid)) == 1 and viewable(dashboards(again.pid)[0]), 20)
check("relaunch after Quit: starts as a new primary instance", ok and again.poll() is None)
if wait_for(lambda: len(tray_registrations()) == 1, 10):
    m2, l2 = tray_menu()
    tray_click(m2, l2, "Quit")
try:
    code = again.wait(timeout=10)
except subprocess.TimeoutExpired:
    again.kill()
    code = None
check("relaunch after Quit: tray Quit exits with code 0", code == 0, f"exit code={code}")

out = proc.stdout.read().decode(errors="replace").strip()
if out:
    print("--- primary instance output ---\n" + out)

counts = {s: sum(1 for _, r in results if r == s) for s in ("PASS", "FAIL", "SKIP")}
print(f"\n{counts['PASS']} passed, {counts['FAIL']} failed, {counts['SKIP']} skipped")
sys.exit(1 if counts["FAIL"] else 0)
