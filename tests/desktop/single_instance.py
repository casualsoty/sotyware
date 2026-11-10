"""Drives the real sotyware binary and checks single-instance + lifecycle behaviour.

usage: single_instance.py <binary> <sni-registrations-file>
Run it through run.sh, which provides a private D-Bus session bus and
GDK_BACKEND=x11. The driver starts the stub tray host (sni_watcher.py) on that bus
itself, and stops it for the dashboard-only scenarios. This is a SIMULATED
environment: the tray host is a stub, and the window manager is whatever serves
$DISPLAY. The page inside the webview is read and clicked through AT-SPI, the
accessibility interface WebKitGTK exposes.

Each check prints PASS, FAIL or SKIP. SKIP means the environment could not
exercise that behaviour (for example a window manager that ignores minimize
requests); it is never counted as a pass.
"""
import atexit
import ctypes
import ctypes.util
import os
import shutil
import subprocess
import sys
import tempfile
import time

import dbus
import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi  # noqa: E402

BIN, REG = os.path.abspath(sys.argv[1]), sys.argv[2]
HERE = os.path.dirname(os.path.abspath(__file__))
LOGS = tempfile.mkdtemp(prefix="sotyware-driver-")
atexit.register(shutil.rmtree, LOGS, True)
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
x.XGrabServer.argtypes = [ctypes.c_void_p]
x.XUngrabServer.argtypes = [ctypes.c_void_p]


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

    The walk holds a server grab so it sees one consistent tree. Without it, a
    walk racing the window manager's unmanage of a just-closed dashboard (client
    moved from its frame back to the root, frame destroyed) can miss the window:
    the primary's own dashboard then looks absent, and later "new".
    """
    x.XGrabServer(dpy)
    try:
        candidates = []
        for top in children(ROOT):
            candidates.append(top)
            candidates.extend(children(top))
        return [w for w in candidates if name(w) == "sotyware" and attrs(w).width > 100]
    finally:
        x.XUngrabServer(dpy)
        x.XFlush(dpy)


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
WATCHER_NAME = "org.kde.StatusNotifierWatcher"


def wait_for_bus_name(name, timeout=10.0, present=True):
    end = time.time() + timeout
    while time.time() < end:
        if bus.name_has_owner(name) == present:
            return True
        time.sleep(0.05)
    return False


# ---------- stub tray host ----------
watcher = None


def start_watcher(*flags):
    """Starts sni_watcher.py on this bus and waits until it owns its name."""
    global watcher
    assert watcher is None, "a stub watcher is already running"
    watcher = subprocess.Popen([sys.executable, os.path.join(HERE, "sni_watcher.py"), REG, *flags])
    assert wait_for_bus_name(WATCHER_NAME), f"stub tray host ({WATCHER_NAME}) did not appear on the session bus"


def stop_watcher():
    """Stops the stub watcher and waits until its name is gone from the bus."""
    global watcher
    if watcher is None:
        return
    watcher.terminate()
    watcher.wait(timeout=10)
    watcher = None
    assert wait_for_bus_name(WATCHER_NAME, present=False), f"{WATCHER_NAME} still owned after stopping the stub"


atexit.register(stop_watcher)


def sni_names(pid):
    """StatusNotifierItem bus names that process `pid` owns (ksni: ...-<pid>-<n>)."""
    return [str(n) for n in bus.list_names() if str(n).startswith(f"org.kde.StatusNotifierItem-{pid}-")]


# ---------- the page inside the webview, via AT-SPI ----------
def _page_nodes(pid):
    """(role, name, text) of every accessible node of process `pid`, [] if not found."""
    desktop = Atspi.get_desktop(0)
    for i in range(desktop.get_child_count()):
        app = desktop.get_child_at_index(i)
        try:
            if app is None or app.get_process_id() != pid:
                continue
            app.clear_cache()
        except gi.repository.GLib.Error:
            continue
        nodes = []

        def walk(node):
            try:
                text = ""
                if node.get_text_iface() is not None:
                    text = Atspi.Text.get_text(node, 0, Atspi.Text.get_character_count(node))
                nodes.append((node.get_role_name(), node.get_name() or "", text or "", node))
                for j in range(node.get_child_count()):
                    child = node.get_child_at_index(j)
                    if child is not None:
                        walk(child)
            except gi.repository.GLib.Error:
                pass  # node went away while walking

        walk(app)
        return nodes
    return []


def page_ready(pid):
    """True once the dashboard has its operating mode (a Hide button or a status note)."""
    roles = [r for r, *_ in _page_nodes(pid)]
    return "push button" in roles or "status bar" in roles


def page_text(pid):
    return " ".join(" ".join((n, t)) for _, n, t, _ in _page_nodes(pid)).replace("\ufffc", " ")


def page_buttons(pid):
    return [(n, node) for r, n, _, node in _page_nodes(pid) if r == "push button"]


def click_button(pid, name):
    """Activates the named button the way a screen reader or keyboard user would."""
    for n, node in page_buttons(pid):
        if n == name:
            return Atspi.Action.do_action(node, 0)
    return False


# ---------- instances whose output is read while they run ----------
def launch(label, env=None):
    """Starts the binary with its output in a log file; returns (process, log path)."""
    log = os.path.join(LOGS, f"{label.split()[0]}-{len(os.listdir(LOGS))}.log")
    with open(log, "wb") as f:
        p = subprocess.Popen([BIN], env=env, stdout=f, stderr=subprocess.STDOUT)
    return p, log


def read_log(log):
    with open(log, errors="replace") as f:
        return f.read()


def log_after_mode(log):
    """The output once the instance has logged its operating-mode decision.

    The window exists before `setup` creates the tray, so the dashboard can be on
    screen before the mode is logged. Bounded wait; returns the output regardless.
    """
    wait_for(lambda: "sotyware: running in" in read_log(log) or "cannot create" in read_log(log), 10)
    return read_log(log)


def wait_exit(p, timeout=10):
    try:
        return p.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        p.kill()
        p.wait()
        return None


DASHBOARD_ONLY_LOG = "sotyware: running in dashboard-only mode"
DASHBOARD_ONLY_TEXT = "No compatible system tray is available"


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
    # No reply awaited: "Quit" ends the process while it handles the call, so
    # whether a reply gets out first is a race (NoReply). Callers check the effect.
    menu.Event(dbus.Int32(labels[label]), "clicked", dbus.Int32(0, variant_level=1), dbus.UInt32(0),
               ignore_reply=True)


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
# Scenarios 1-11 run in tray mode. Without the stub tray host, sotyware would start
# in dashboard-only mode and every tray check would fail for the wrong reason.
start_watcher()

# 1. First launch.
proc = subprocess.Popen([BIN], stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
alive = lambda: proc.poll() is None
ok = wait_for(lambda: len(dashboards(proc.pid)) == 1 and viewable(dashboards(proc.pid)[0]), 20)
check("1 first launch: process running and dashboard visible", ok and alive())
win = dashboards(proc.pid)[0]
ok = wait_for(lambda: len(tray_registrations()) == 1, 10)
if not ok:
    # Every later scenario needs the tray menu: stop with a FAIL and the
    # primary's output instead of a traceback from tray_menu().
    check("1 first launch: one tray item with Open Dashboard / Quit", False,
          f"registrations={len(tray_registrations())} after 10 s")
    proc.kill()
    print("--- primary instance output ---\n" + proc.communicate()[0].decode(errors="replace").strip())
    print(f"\n{sum(r == 'PASS' for _, r in results)} passed, {sum(r == 'FAIL' for _, r in results)} failed, "
          f"{sum(r == 'SKIP' for _, r in results)} skipped (aborted)")
    sys.exit(1)
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
registered = wait_for(lambda: len(tray_registrations()) == 1, 10)
check("relaunch after Quit: one tray item registered", registered,
      f"registrations={len(tray_registrations())} after 10 s")
if registered:
    m2, l2 = tray_menu()
    tray_click(m2, l2, "Quit")
try:
    code = again.wait(timeout=10)
except subprocess.TimeoutExpired:
    again.kill()
    code = None
check("relaunch after Quit: tray Quit exits with code 0", code == 0, f"exit code={code}")
if not (registered and code == 0):
    print("--- relaunched instance output ---\n" + again.communicate()[0].decode(errors="replace").strip())

# 10. Startup failure: the tray icon cannot be created for a reason other than a
# missing tray host, so sotyware must exit 1 instead of hiding a defect behind
# dashboard-only mode. The tray backend (ksni) registers with the
# StatusNotifierWatcher before tray creation returns; here a watcher exists but
# rejects the item with an unexpected D-Bus error (AccessDenied).
# (A missing watcher is no longer a failure: see scenario 13.)
open(REG, "w").close()
stop_watcher()
start_watcher("--reject")
bad, bad_log = launch("10 tray failure")
code = wait_exit(bad, 30)
bad_out = read_log(bad_log)
check("10 tray failure: process exits with code 1", code == 1, f"exit code={code}")
check("10 tray failure: reports the tray error", "sotyware: cannot create system tray icon" in bad_out
      and "AccessDenied" in bad_out and DASHBOARD_ONLY_LOG not in bad_out,
      f"output={bad_out.strip()!r}")
check("10 tray failure: no process or tray item left behind",
      wait_for(lambda: not sotyware_pids(), 5) and not tray_registrations(),
      f"pids={sotyware_pids()} tray={len(tray_registrations())}")
stop_watcher()
start_watcher()

# 11. As under WSL (/run/user/<uid> missing, /run/user owned by root),
# $XDG_RUNTIME_DIR names a directory that cannot be created
# (see docs/development/setup.md#wsl2-notes). The tray icon image travels over
# D-Bus and is never written to disk, so sotyware must start with its tray.
open(REG, "w").close()
read_only_parent = os.path.join(os.path.dirname(os.path.abspath(REG)), "read-only")
os.mkdir(read_only_parent, 0o500)
missing_dir = os.path.join(read_only_parent, "runtime")
odd = subprocess.Popen([BIN], env={**os.environ, "XDG_RUNTIME_DIR": missing_dir},
                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
ok = wait_for(lambda: len(dashboards(odd.pid)) == 1 and viewable(dashboards(odd.pid)[0]), 20)
registered = wait_for(lambda: len(tray_registrations()) == 1, 10)
check("11 unusable XDG_RUNTIME_DIR: starts with one tray item", ok and registered and odd.poll() is None,
      f"running={odd.poll() is None} registrations={len(tray_registrations())}")
if registered:
    m3, l3 = tray_menu()
    tray_click(m3, l3, "Quit")
try:
    code = odd.wait(timeout=10)
except subprocess.TimeoutExpired:
    odd.kill()
    code = None
check("11 unusable XDG_RUNTIME_DIR: tray Quit exits with code 0", code == 0, f"exit code={code}")
if not (ok and registered and code == 0):
    print("--- instance output ---\n" + odd.communicate()[0].decode(errors="replace").strip())

# 12. Tray mode, the dashboard's own Hide button (clicked through AT-SPI). Also the
# control for scenario 13: it proves that AT-SPI reaches the page and its button.
open(REG, "w").close()
tm, tm_log = launch("12 tray mode")
ok = wait_for(lambda: len(dashboards(tm.pid)) == 1 and viewable(dashboards(tm.pid)[0]), 20)
registered = wait_for(lambda: len(tray_registrations()) == 1, 10)
check("12 tray mode: starts with dashboard and one tray item", ok and registered and tm.poll() is None,
      f"registrations={len(tray_registrations())}")
tm_win = dashboards(tm.pid)[0] if ok else None
page = wait_for(lambda: page_ready(tm.pid), 20)
text = page_text(tm.pid) if page else ""
check("12 tray mode: page shows the tray hint and a Hide button, no dashboard-only note",
      page and "reopen it any time from the system tray" in text and DASHBOARD_ONLY_TEXT not in text
      and [n for n, _ in page_buttons(tm.pid)] == ["Hide dashboard"],
      f"text={text!r} buttons={[n for n, _ in page_buttons(tm.pid)]}")
can_click_hide = bool(tm_win) and page and click_button(tm.pid, "Hide dashboard")
hidden = can_click_hide and wait_for(lambda: not viewable(tm_win))
check("12 tray mode: Hide button hides the dashboard, process keeps running",
      hidden and tm.poll() is None and all_dashboards() == [tm_win], f"clicked={can_click_hide}")
if registered:
    m4, l4 = tray_menu()
    tray_click(m4, l4, "Open Dashboard")
    check("12 tray mode: tray Open Dashboard restores it after Hide",
          bool(tm_win) and wait_for(lambda: viewable(tm_win)) and all_dashboards() == [tm_win])
    tray_click(m4, l4, "Quit")
code = wait_exit(tm)
check("12 tray mode: tray Quit exits with code 0, nothing left",
      code == 0 and wait_for(lambda: not sotyware_pids(), 5), f"exit code={code} pids={sotyware_pids()}")
if not (ok and registered and hidden and code == 0):
    print("--- instance output ---\n" + read_log(tm_log).strip())

# 13. Dashboard-only mode: a session bus, but no StatusNotifierWatcher (vanilla
# GNOME). sotyware must start, keep its dashboard on screen, not pretend to have a
# tray, and end when the dashboard is closed.
stop_watcher()
open(REG, "w").close()
do, do_log = launch("13 dashboard-only")
ok = wait_for(lambda: len(dashboards(do.pid)) == 1 and viewable(dashboards(do.pid)[0]), 20)
check("13 dashboard-only: process running and dashboard visible", ok and do.poll() is None)
do_win = dashboards(do.pid)[0] if ok else None
page = wait_for(lambda: page_ready(do.pid), 20)
do_out = log_after_mode(do_log)
check("13 dashboard-only: reports the mode and the reason",
      DASHBOARD_ONLY_LOG in do_out and "(no StatusNotifierWatcher)" in do_out
      and "cannot create system tray icon" not in do_out, f"output={do_out.strip()!r}")
check("13 dashboard-only: no tray item registered or served",
      not tray_registrations() and not sni_names(do.pid),
      f"registrations={len(tray_registrations())} names={sni_names(do.pid)}")
text = page_text(do.pid) if page else ""
check("13 dashboard-only: page shows the no-tray note, not the tray hint",
      page and DASHBOARD_ONLY_TEXT in text and "Closing it quits sotyware" in text
      and "from the system tray" not in text, f"text={text!r}")
buttons = [n for n, _ in page_buttons(do.pid)] if page else None
if can_click_hide:
    check("13 dashboard-only: page offers no Hide button, dashboard stays visible",
          buttons == [] and not click_button(do.pid, "Hide dashboard") and viewable(do_win),
          f"buttons={buttons}")
else:
    check("13 dashboard-only: page offers no Hide button, dashboard stays visible", None,
          "scenario 12 could not click Hide through AT-SPI, so its absence proves nothing")

# Single instance works without a tray: a second launch shows the dashboard.
if do_win:
    x.XIconifyWindow(dpy, do_win, x.XDefaultScreen(dpy))
    x.XFlush(dpy)
    minimized = wait_for(lambda: iconic(do_win) or not viewable(do_win), 4)
    code, new_windows = second_launch("13 dashboard-only second launch")
    check("13 dashboard-only second launch: exits 0, no new window, one process",
          code == 0 and not new_windows and sotyware_pids() == [do.pid],
          f"exit code={code} new={[hex(w) for w in new_windows]} pids={sotyware_pids()}")
    check("13 dashboard-only second launch: minimized dashboard restored",
          wait_for(lambda: viewable(do_win) and not iconic(do_win)) if minimized else None,
          "" if minimized else "window manager ignored the minimize request")
    check("13 dashboard-only second launch: still one window, no tray item",
          all_dashboards() == [do_win] and not tray_registrations() and not sni_names(do.pid))

# A tray host that appears later is not picked up: no retry, no polling.
start_watcher()
time.sleep(2)  # a negative check needs a window to observe; nothing is awaited
check("13 dashboard-only: a watcher started later gets no registration",
      not tray_registrations() and not sni_names(do.pid) and do.poll() is None,
      f"registrations={len(tray_registrations())} names={sni_names(do.pid)}")

# Closing the only window ends the process instead of leaving it unreachable.
if do_win:
    send_wm_delete(do_win)
code = wait_exit(do)
check("13 dashboard-only close: process exits with code 0", code == 0, f"exit code={code}")
check("13 dashboard-only close: no process or window left",
      wait_for(lambda: not sotyware_pids(), 5) and not all_dashboards(),
      f"pids={sotyware_pids()} windows={[hex(w) for w in all_dashboards()]}")
if code != 0:
    print("--- instance output ---\n" + read_log(do_log).strip())

# 14. With the watcher back, the next launch is normal tray mode again.
open(REG, "w").close()
back, back_log = launch("14 relaunch with watcher")
ok = wait_for(lambda: len(dashboards(back.pid)) == 1 and viewable(dashboards(back.pid)[0]), 20)
registered = wait_for(lambda: len(tray_registrations()) == 1, 10)
check("14 relaunch with watcher: tray mode with one tray item", ok and registered and back.poll() is None
      and DASHBOARD_ONLY_LOG not in read_log(back_log), f"registrations={len(tray_registrations())}")
back_win = dashboards(back.pid)[0] if ok else None
if back_win:
    send_wm_delete(back_win)
check("14 relaunch with watcher: close hides again", bool(back_win) and wait_for(lambda: not viewable(back_win))
      and back.poll() is None)
if registered:
    m5, l5 = tray_menu()
    tray_click(m5, l5, "Quit")
code = wait_exit(back)
check("14 relaunch with watcher: tray Quit exits with code 0, nothing left",
      code == 0 and wait_for(lambda: not sotyware_pids(), 5), f"exit code={code}")
if not (ok and registered and code == 0):
    print("--- instance output ---\n" + read_log(back_log).strip())

# 15. A watcher that reports no StatusNotifierHost (nothing would display the
# item): dashboard-only mode too, and the item ksni announced is not left behind.
stop_watcher()
start_watcher("--no-host")
open(REG, "w").close()
nh, nh_log = launch("15 no host")
ok = wait_for(lambda: len(dashboards(nh.pid)) == 1 and viewable(dashboards(nh.pid)[0]), 20)
nh_out = log_after_mode(nh_log)
check("15 no host: dashboard-only mode, dashboard visible",
      ok and nh.poll() is None and DASHBOARD_ONLY_LOG in nh_out and "(no StatusNotifierHost)" in nh_out,
      f"output={nh_out.strip()!r}")
check("15 no host: no tray item served", wait_for(lambda: not sni_names(nh.pid), 5),
      f"names={sni_names(nh.pid)}")
nh_win = dashboards(nh.pid)[0] if ok else None
if nh_win:
    send_wm_delete(nh_win)
code = wait_exit(nh)
check("15 no host: close exits with code 0, nothing left",
      code == 0 and wait_for(lambda: not sotyware_pids(), 5), f"exit code={code}")
stop_watcher()

# 16. No D-Bus session bus at all: no address in the environment and no socket in
# $XDG_RUNTIME_DIR. Single instance is off without a bus (the plugin disables
# itself), so only the mode and the close behaviour are checked.
nobus_env = {k: v for k, v in os.environ.items() if k != "DBUS_SESSION_BUS_ADDRESS"}
nobus_runtime = os.path.join(LOGS, "runtime-without-bus")
os.mkdir(nobus_runtime, 0o700)
nobus_env["XDG_RUNTIME_DIR"] = nobus_runtime
nb, nb_log = launch("16 no session bus", env=nobus_env)
ok = wait_for(lambda: len(dashboards(nb.pid)) == 1 and viewable(dashboards(nb.pid)[0]), 20)
nb_out = log_after_mode(nb_log)
check("16 no session bus: dashboard-only mode, dashboard visible",
      ok and nb.poll() is None and DASHBOARD_ONLY_LOG in nb_out and "(no D-Bus session bus)" in nb_out,
      f"output={nb_out.strip()!r}")
nb_win = dashboards(nb.pid)[0] if ok else None
if nb_win:
    send_wm_delete(nb_win)
code = wait_exit(nb)
check("16 no session bus: close exits with code 0, nothing left",
      code == 0 and wait_for(lambda: not sotyware_pids(), 5), f"exit code={code} pids={sotyware_pids()}")
if not (ok and code == 0):
    print("--- instance output ---\n" + read_log(nb_log).strip())

out = proc.stdout.read().decode(errors="replace").strip()
if out:
    print("--- primary instance output ---\n" + out)

counts = {s: sum(1 for _, r in results if r == s) for s in ("PASS", "FAIL", "SKIP")}
print(f"\n{counts['PASS']} passed, {counts['FAIL']} failed, {counts['SKIP']} skipped")
sys.exit(1 if counts["FAIL"] else 0)
