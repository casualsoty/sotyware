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
import collections
import ctypes
import ctypes.util
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
from typing import NamedTuple

import dbus
import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi  # noqa: E402

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import test_profile as tp  # noqa: E402

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


def one_dashboard_shown(pid):
    """`pid` has exactly one dashboard and it is mapped, judged on one
    snapshot: while sotyware starts, GTK destroys its first dashboard
    window and creates another (create, destroy, create within about 15 ms,
    traced on 2026-11-17), so two walks in a row can disagree, and indexing
    the second with what the first counted failed."""
    ws = dashboards(pid)
    return len(ws) == 1 and viewable(ws[0])


def iconic(w):
    return prop_long(w, A_WM_STATE) == ICONIC_STATE


def wait_for(pred, timeout=8.0, interval=0.1):
    # The monotonic clock: WSL2 steps the wall clock by most of a second now and then.
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        x.XSync(dpy, 0)
        if pred():
            return True
        time.sleep(interval)
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
RUN_DISPLAY = os.environ.get("DISPLAY")
RUN_BUS = os.environ.get("DBUS_SESSION_BUS_ADDRESS")
assert RUN_DISPLAY and RUN_BUS, "run through run.sh: needs DISPLAY and a private DBUS_SESSION_BUS_ADDRESS"
# Directories only this run uses; launches that strip the bus address (scenario 16)
# point XDG_RUNTIME_DIR into one of them.
RUN_DIRS = (LOGS, os.path.dirname(os.path.abspath(REG)))
# Every launch runs in this run's test profile (ADR-027): a test-profile build
# that refuses to start without one, keeps its databases in it and says so in
# an acknowledgement, checked after each launch that becomes the primary
# instance. XDG_DATA_HOME (private to this run, set by run.sh) is a second
# layer: even a forgotten path stays out of the user's data.
DATA_HOME = os.path.realpath(os.environ.get("XDG_DATA_HOME", "/nonexistent"))
assert DATA_HOME.startswith(os.path.realpath(RUN_DIRS[1]) + os.sep), \
    f"run through run.sh: XDG_DATA_HOME must be private to this run, not {DATA_HOME}"
tp.require_test_profile_build(BIN)
PROFILE = tp.create(RUN_DIRS[1], f"si-{os.getpid()}")
os.environ[tp.ENV] = PROFILE.root
PREFS_DB = PROFILE.db("preferences.sqlite3")
NORMAL_DIRS = tp.normal_profile_dirs(os.path.expanduser("~"))
NORMAL_BEFORE = tp.fingerprint(NORMAL_DIRS)

def _environ(pid):
    with open(f"/proc/{pid}/environ", "rb") as f:
        pairs = (v.split(b"=", 1) for v in f.read().split(b"\0") if b"=" in v)
        return {k.decode(errors="replace"): v.decode(errors="replace") for k, v in pairs}


def _started_by_this_run(env):
    """Same display, and the same session bus (or no bus and a runtime dir of this run).

    Another sotyware from the same binary (a concurrent run on another display, a
    probe, the developer's own instance) must not count as this run's process.
    """
    if env.get("DISPLAY") != RUN_DISPLAY:
        return False
    bus_address = env.get("DBUS_SESSION_BUS_ADDRESS")
    if bus_address is not None:
        return bus_address == RUN_BUS
    runtime = os.path.realpath(env.get("XDG_RUNTIME_DIR", "/"))
    return any(runtime.startswith(os.path.realpath(d) + os.sep) for d in RUN_DIRS)


def sotyware_pids():
    """PIDs of the processes this run started from this exact binary.

    Matched by /proc/<pid>/exe, then scoped by /proc/<pid>/environ (see
    _started_by_this_run), as virtual.sh's cleanup scopes by DISPLAY.
    """
    pids = []
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            if os.path.realpath(f"/proc/{entry}/exe") == BIN and _started_by_this_run(_environ(entry)):
                pids.append(int(entry))
        except OSError:
            pass
    return sorted(pids)


# Every launch carries its label in this variable, which sotyware never
# reads. WebKit's helpers inherit sotyware's environment, so a helper names
# the launch that started it, also once that sotyware has ended and the
# helper has been given another parent.
LAUNCH_ENV = "SI_DRIVER_LAUNCH"
LAUNCHED = {}  # sotyware pid -> its launch's label


def start(label, env=None, **popen_args):
    """Starts the binary, `label` in its environment (LAUNCH_ENV)."""
    assert label not in LAUNCHED.values(), f"two launches labelled {label!r}"
    env = dict(os.environ if env is None else env)
    env[LAUNCH_ENV] = label
    p = subprocess.Popen([BIN], env=env, **popen_args)
    LAUNCHED[p.pid] = label
    return p


# WebKitGTK keeps a page's localStorage (the theme, ADR-035) in its network
# process, which commits a write 0.5 s after it is made or, when sotyware
# ends first, as soon as it sees sotyware gone, and only then exits
# (NetworkProcess::didClose; a watchdog ends it after 10 s). A launch that
# reads the storage before that commit shows the previous value, and can
# delete the late commit for good
# (docs/status/limitations.md#dashboard-adr-035): scenario 17's one failure
# on GitHub looked exactly so. So every quit of 17 to 19 waits for the
# quitting launch's helpers, and before 17 starts a check waits until no
# helper of an earlier launch is left. On WSL2 the helpers ended within
# about 0.1 s of sotyware; 15 s leaves room for WebKit's own 10 s.
WEBKIT_HELPERS_WAIT = 15
HELPERS_POLL = 0.02  # s: fine enough to say how long they outlived sotyware
# The process that commits the page's storage: one per sotyware.
NETWORK_PROCESS = "WebKitNetworkProcess"
CLK_TCK = os.sysconf("SC_CLK_TCK")


def boot_clock():
    """Now, in seconds on the clock of /proc/<pid>/stat's start times (CLOCK_BOOTTIME)."""
    return time.clock_gettime(time.CLOCK_BOOTTIME)


class Helper(NamedTuple):
    """A WebKit helper process as /proc shows it at one moment."""
    pid: int
    started: float  # seconds since boot (/proc/<pid>/stat); with the pid, the process's identity
    program: str  # the name of /proc/<pid>/exe: WebKitNetworkProcess, WebKitWebProcess, ...
    parent: int  # its parent now: sotyware, or 1 or a subreaper once sotyware has ended
    state: str  # R running, S sleeping, D waiting on a device, T stopped, ...
    threads: int
    launch: str  # the label of the launch whose sotyware started it (LAUNCH_ENV)


def _stat(task):
    """(state, parent pid, threads, start in clock ticks since boot) of /proc/<task>/stat."""
    with open(f"/proc/{task}/stat") as f:
        fields = f.read().rsplit(")", 1)[1].split()  # the name before ")" may hold anything
    return fields[0], int(fields[1]), int(fields[17]), int(fields[19])


def webkit_helpers():
    """WebKit's helper processes (network, web, ...) of this run's
    instances, which outlive their sotyware briefly. Started by sotyware,
    they inherit its environment, and with it this run's test profile and
    the label of its launch."""
    found = []
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            program = os.path.basename(os.readlink(f"/proc/{entry}/exe"))
            if not program.startswith("WebKit"):
                continue
            env = _environ(entry)
            if env.get(tp.ENV) != PROFILE.root:
                continue
            state, parent, threads, ticks = _stat(entry)
        except OSError:
            continue  # ended meanwhile (an ended process has no exe left), or not ours to read
        found.append(Helper(int(entry), ticks / CLK_TCK, program, parent, state, threads,
                            env.get(LAUNCH_ENV, "?")))
    return sorted(found)


def _comm(task):
    """The name of /proc/<task> (a process, or a process's thread), '?' once it is gone."""
    try:
        with open(f"/proc/{task}/comm") as f:
            return f.read().strip()
    except OSError:
        return "?"


def _core_dumping(pid):
    """True while the kernel writes the process's core dump, that is after it
    crashed (CoreDumping in /proc/<pid>/status). On GitHub's runner the core
    goes to systemd-coredump, which took more than 15 s for a WebKit web
    process."""
    try:
        with open(f"/proc/{pid}/status") as f:
            return any(line.split() == ["CoreDumping:", "1"] for line in f)
    except OSError:
        return False


# A helper the kernel is dumping core for after a crash runs none of its
# own code any more and cannot touch the page's storage. On GitHub's
# Ubuntu 26.04 runner WebKit's web process crashed so as sotyware quit, in
# a static destructor of Mesa's libgallium, and its core took more than
# 15 s (docs/status/limitations.md#dashboard-adr-035). The waits count such
# a helper as ended and name it, and the run lists them at its end; a
# crashed network process, which may not have committed the page's
# storage, still fails.
CRASHED = []  # (where it was seen, helper) of every helper seen dumping core


def split_crashed(helpers, crashed):
    """The helpers in `helpers` still running their own code. Those the
    kernel is dumping core for go into `crashed` (keyed by pid and start)."""
    running = []
    for h in helpers:
        if _core_dumping(h.pid):
            crashed.setdefault((h.pid, h.started), h)
        else:
            running.append(h)
    return running


CRASHED_ABOUT = ("dumping core after a crash (a web process's crash at quit is WebKit's and Mesa's: "
                 "docs/status/limitations.md#dashboard-adr-035)")


def crashed_text(h, since, event):
    """A helper seen dumping core, for a check's message (it may have ended since)."""
    offset = h.started - since
    return (f"{h.program} {h.pid} of launch '{h.launch}' (started {abs(offset):.2f} s "
            f"{'after' if offset >= 0 else 'before'} {event})")


def helper_text(h, since, event):
    """`h` for a check's message, its start relative to `event` (at `since` on boot_clock())."""
    offset = h.started - since
    crashed = ", dumping core after a crash" if _core_dumping(h.pid) else ""
    return (f"{h.program} {h.pid} (parent {h.parent} {_comm(h.parent)}, state {h.state}, {h.threads} threads"
            f"{crashed}, started {abs(offset):.2f} s {'after' if offset >= 0 else 'before'} {event}, "
            f"of launch '{h.launch}')")


def show_helper(h, with_output=True):
    """Prints what a helper that did not end is doing, as far as /proc tells
    without tracing it: its command line, its threads by name (numbered
    ones together), state and kernel wait channel, the main thread first,
    and the output of its launch, which WebKit's helpers write to as well
    (launches through launch() only)."""
    print(f"--- WebKit helper {h.pid} ({h.program}) of launch '{h.launch}' ---")
    try:
        with open(f"/proc/{h.pid}/cmdline", "rb") as f:
            print("command line: " + f.read().replace(b"\0", b" ").decode(errors="replace").strip())
        tids = os.listdir(f"/proc/{h.pid}/task")
    except OSError as e:
        print(f"(could not read it any more: {e})")
        tids = []
    threads = collections.Counter()
    for tid in tids:
        task = f"{h.pid}/task/{tid}"
        try:
            with open(f"/proc/{task}/wchan") as f:
                wchan = f.read().strip()
            state = _stat(task)[0]
        except OSError:
            continue  # this thread ended meanwhile
        threads[(int(tid) != h.pid, re.sub(r"\d+$", "<n>", _comm(task)), state, wchan)] += 1
    for (not_main, name, state, wchan), n in sorted(threads.items()):
        print(f"{n} × {'thread' if not_main else 'main thread'} {name!r}: state {state}, in {wchan}")
    if with_output and h.launch in LAUNCH_LOGS:
        print(f"--- output of launch '{h.launch}' ---\n" + read_log(LAUNCH_LOGS[h.launch]).strip())


def tray_registrations():
    if not os.path.exists(REG):
        return []
    with open(REG) as f:
        return [l.split() for l in f if l.strip()]


def second_launch(label):
    """Launches the binary again; returns (exit code, output, new windows seen)."""
    before = set(all_dashboards())
    p = start(label, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
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
    """(role, name, text, node) of every accessible node of process `pid`, [] if not found.

    `role` is the Atspi.Role enum, never get_role_name(): role names are not stable
    across at-spi2-core releases (2.53.1 renamed "push button" to "button"), while
    the enum values are (Atspi.Role.PUSH_BUTTON == 43 on 2.52 and on 2.60).
    """
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
                nodes.append((node.get_role(), node.get_name() or "", text or "", node))
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
    """True once the dashboard has its operating mode (the Overview's Hide button or
    its no-tray status note). The sidebar's own buttons exist before that."""
    nodes = _page_nodes(pid)
    return any(r == Atspi.Role.PUSH_BUTTON and n == "Hide dashboard" for r, n, *_ in nodes) or any(
        r == Atspi.Role.STATUS_BAR for r, *_ in nodes)


def page_text(pid):
    return " ".join(" ".join((n, t)) for _, n, t, _ in _page_nodes(pid)).replace("\ufffc", " ")


def page_buttons(pid):
    return [(n, node) for r, n, _, node in _page_nodes(pid) if r == Atspi.Role.PUSH_BUTTON]


def page_checkboxes(pid):
    return {n: node for r, n, _, node in _page_nodes(pid) if r == Atspi.Role.CHECK_BOX}


def checked(node):
    return node.get_state_set().contains(Atspi.StateType.CHECKED)


def open_settings_tab(pid, name):
    """Goes to Settings and one of its tabs (General, Features, About & updates),
    the way a keyboard or screen-reader user would: the Settings link, then the tab."""
    if not click_link(pid, "Settings"):
        return False
    for _ in range(20):
        tab = next((node for r, n, _, node in _page_nodes(pid)
                    if r == Atspi.Role.PAGE_TAB and n == name), None)
        if tab is not None:
            return Atspi.Action.do_action(tab, 0)
        time.sleep(0.25)
    return False


def open_features(pid):
    """Settings › Features, where the built-in features are switched since the redesign."""
    return open_settings_tab(pid, "Features")


def theme_radio(pid, name):
    """The Appearance radio button `name` (System, Light, Dark) in Settings › General."""
    return next((node for r, n, _, node in _page_nodes(pid)
                 if r == Atspi.Role.RADIO_BUTTON and n == name), None)


def themes_shown(pid):
    """The Appearance choices shown as chosen (one, unless something is wrong)."""
    return [n for n in ("System", "Light", "Dark")
            if (radio := theme_radio(pid, n)) is not None and checked(radio)]


def switched_feature(pid):
    """The switched feature's checkbox and the page text, once the list has loaded.
    Opens Settings › Features when the list is not shown."""
    box = page_checkboxes(pid).get(SWITCHED_FEATURE)
    if box is None and open_features(pid):
        time.sleep(0.3)
        box = page_checkboxes(pid).get(SWITCHED_FEATURE)
    return box, page_text(pid)


def page_links(pid):
    return [(n, node) for r, n, _, node in _page_nodes(pid) if r == Atspi.Role.LINK]


def click_link(pid, name):
    """Follows the named navigation link, as a keyboard or screen-reader user would."""
    for n, node in page_links(pid):
        if n == name:
            return Atspi.Action.do_action(node, 0)
    return False


def page_headings(pid):
    return [n or t for r, n, t, _ in _page_nodes(pid) if r == Atspi.Role.HEADING]


def click_button(pid, name):
    """Activates the named button the way a screen reader or keyboard user would."""
    for n, node in page_buttons(pid):
        if n == name:
            return Atspi.Action.do_action(node, 0)
    return False


# ---------- instances whose output is read while they run ----------
LAUNCH_LOGS = {}  # launch label -> the file its output goes to


def launch(label, env=None):
    """Starts the binary with its output in a log file; returns (process, log path)."""
    log = os.path.join(LOGS, f"{label.split()[0]}-{len(os.listdir(LOGS))}.log")
    with open(log, "wb") as f:
        p = start(label, env, stdout=f, stderr=subprocess.STDOUT)
    LAUNCH_LOGS[label] = log
    return p, log


def verified(p, label):
    """Stops the whole run unless `p` acknowledges this run's test profile
    (ADR-027): nothing is clicked in a window whose data is not provably
    this run's."""
    try:
        tp.verify_ack(PROFILE, p.pid, timeout=30, alive=lambda: p.poll() is None)
    except tp.IsolationError as e:
        check(f"{label}: runs in this run's test profile (acknowledged)", False, str(e))
        for pid in sotyware_pids():
            os.kill(pid, 9)
        print("ABORT: the app is not provably isolated; nothing else is run")
        sys.exit(1)
    check(f"{label}: runs in this run's test profile (acknowledged)", True)
    return p


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
# The built-in feature the checks switch: backup-scheduler
# (src-tauri/src/features/backup.rs), enabled by default; in a fresh test
# profile there is no backup job, so switching it starts and stops nothing
# visible. Its checkbox is labelled with its name.
SWITCHED_FEATURE = "Scheduled backups"
SWITCHED_ID = "backup-scheduler"


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

# 0. Fail closed (ADR-027): without a profile, or with an invalid one, the
# test-profile build exits 3 before any window or tray item exists; it never
# falls back to the normal profile.
def refused(label, env):
    before = set(all_dashboards())
    p = start(f"0 {label}", env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    try:
        out, _ = p.communicate(timeout=15)
    except subprocess.TimeoutExpired:
        p.kill()
        out, _ = p.communicate()
    text = out.decode(errors="replace")
    check(f"0 {label}: exits 3 at once, no window, no tray item",
          p.returncode == tp.EXIT_REFUSED and set(all_dashboards()) == before and not tray_registrations()
          and "test profile refused" in text, f"exit={p.returncode} output={text.strip()!r}")


open(REG, "w").close()
refused("no SOTYWARE_TEST_PROFILE", {k: v for k, v in os.environ.items() if k != tp.ENV})
bad_profile = os.path.join(RUN_DIRS[1], "bad-profile")
os.mkdir(bad_profile)
with open(os.path.join(bad_profile, tp.MARKER_FILE), "w") as f:
    f.write('{"format":1,"purpose":"%s","run":"bad","nonce":"not-hex"}' % tp.PURPOSE)
refused("a profile with an invalid marker", {**os.environ, tp.ENV: bad_profile})
check("0 refused launches created nothing in either profile",
      sorted(os.listdir(bad_profile)) == [tp.MARKER_FILE] and sorted(os.listdir(PROFILE.root)) == [tp.MARKER_FILE])

# 1. First launch.
proc = start("1 first launch", stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
verified(proc, "1 first launch")
alive = lambda: proc.poll() is None
ok = wait_for(lambda: one_dashboard_shown(proc.pid), 20)
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
burst = [start(f"7 concurrent launch #{i + 1}", stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
         for i in range(4)]
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
again = verified(start("9 relaunch", stdout=subprocess.PIPE, stderr=subprocess.STDOUT), "9 relaunch")
ok = wait_for(lambda: one_dashboard_shown(again.pid), 20)
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
verified(bad, "10 tray failure")
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
odd = verified(start("11 unusable XDG_RUNTIME_DIR", {**os.environ, "XDG_RUNTIME_DIR": missing_dir},
                     stdout=subprocess.PIPE, stderr=subprocess.STDOUT), "11 unusable XDG_RUNTIME_DIR")
ok = wait_for(lambda: one_dashboard_shown(odd.pid), 20)
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
verified(tm, "12 tray mode")
ok = wait_for(lambda: one_dashboard_shown(tm.pid), 20)
registered = wait_for(lambda: len(tray_registrations()) == 1, 10)
check("12 tray mode: starts with dashboard and one tray item", ok and registered and tm.poll() is None,
      f"registrations={len(tray_registrations())}")
tm_win = dashboards(tm.pid)[0] if ok else None
page = wait_for(lambda: page_ready(tm.pid), 20)
text = page_text(tm.pid) if page else ""
check("12 tray mode: page shows the tray hint and a Hide button, no dashboard-only note",
      page and "reopen it any time from the system tray" in text and DASHBOARD_ONLY_TEXT not in text
      and "Hide dashboard" in [n for n, _ in page_buttons(tm.pid)],
      f"text={text!r} buttons={[n for n, _ in page_buttons(tm.pid)]}")
# Built-in features through the real capability and webview: list_features and
# set_feature_enabled are allowed, and Scheduled backups runs from startup.
listed = page and wait_for(lambda: switched_feature(tm.pid)[0] is not None, 10)
box, text = switched_feature(tm.pid) if listed else (None, "")
check("12 tray mode: page lists Scheduled backups as running, checkbox checked",
      listed and checked(box) and "Running" in text and "Could not" not in text,
      f"checkboxes={list(page_checkboxes(tm.pid)) if page else None} text={text!r}")


def switched_to(want_checked, state):
    box, text = switched_feature(tm.pid)
    return box is not None and checked(box) == want_checked and state in text and "Could not" not in text


off = listed and Atspi.Action.do_action(switched_feature(tm.pid)[0], 0) and wait_for(
    lambda: switched_to(False, "Disabled"), 5)
check("12 tray mode: unchecking Scheduled backups disables it", off,
      f"text={switched_feature(tm.pid)[1] if page else ''!r}")
on = off and Atspi.Action.do_action(switched_feature(tm.pid)[0], 0) and wait_for(
    lambda: switched_to(True, "Running"), 5)
check("12 tray mode: checking it again starts it", on,
      f"text={switched_feature(tm.pid)[1] if page else ''!r}")
# The Cloud Storage section through the real webview and capability: the link
# switches sections, list_cloud_providers and list_cloud_accounts are allowed,
# and with no accounts (fresh test profile) the empty state and the Add
# account action are shown. Listing never touches the credential store, so no
# Secret Service is needed. Then back to the overview, whose buttons are
# unchanged.
links = [n for n, _ in page_links(tm.pid)] if page else []
check("12 tray mode: the sidebar has the section links (no storage account yet)",
      links == ["Overview", "Cloud Storage", "Backups", "Autoclicker", "Account", "Settings"], f"links={links}")
to_cloud = page and click_link(tm.pid, "Cloud Storage") and wait_for(
    lambda: "No storage accounts yet" in page_text(tm.pid), 10)
text = page_text(tm.pid) if page else ""
check("12 tray mode: Cloud Storage shows its heading, empty state and Add account, no error",
      to_cloud and "Cloud Storage" in page_headings(tm.pid)
      and [n for n, _ in page_buttons(tm.pid)] == ["Add storage account", "Add account"]
      and "Could not" not in text and "could not be opened" not in text,
      f"headings={page_headings(tm.pid) if page else None} buttons={[n for n, _ in page_buttons(tm.pid)] if page else None} text={text!r}")
back = to_cloud and click_link(tm.pid, "Overview") and wait_for(
    lambda: "Hide dashboard" in [n for n, _ in page_buttons(tm.pid)], 10)
check("12 tray mode: Overview link returns to the Hide button and the summaries",
      back and "Overview" in page_headings(tm.pid) and "Storage" in page_headings(tm.pid),
      f"buttons={[n for n, _ in page_buttons(tm.pid)] if page else None} headings={page_headings(tm.pid) if page else None}")
cloud_db = PROFILE.db("cloud-accounts.sqlite3")
check("12 tray mode: the account database is created in the test profile, with no accounts",
      os.path.exists(cloud_db) and sqlite3.connect(f"file:{cloud_db}?mode=ro", uri=True)
      .execute("SELECT count(*) FROM storage_accounts").fetchone() == (0,),
      cloud_db)
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
verified(do, "13 dashboard-only")
ok = wait_for(lambda: one_dashboard_shown(do.pid), 20)
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
# On the Overview, before the feature checks leave it for Settings.
buttons = [n for n, _ in page_buttons(do.pid)] if page else None
no_hide = page and not click_button(do.pid, "Hide dashboard")
listed = page and wait_for(lambda: switched_feature(do.pid)[0] is not None, 10)
box, text = switched_feature(do.pid) if listed else (None, "")
check("13 dashboard-only: features run too: Scheduled backups listed as running",
      listed and checked(box) and "Running" in text, f"text={text!r}")
if can_click_hide:
    check("13 dashboard-only: page offers no Hide button, dashboard stays visible",
          buttons is not None and "Hide dashboard" not in buttons and no_hide and viewable(do_win),
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
verified(back, "14 relaunch with watcher")
ok = wait_for(lambda: one_dashboard_shown(back.pid), 20)
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
verified(nh, "15 no host")
ok = wait_for(lambda: one_dashboard_shown(nh.pid), 20)
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

# 17-19: feature preferences across real restarts: every "restart" ends the
# process (tray Quit, exit code 0, no process left) and starts a new one; the
# page is never just reloaded. Tray mode, with the stub tray host.
# They run before 16: under this AT-SPI setup, the first instance started after
# the no-bus instance of 16 never appears on the accessibility desktop (the next
# one does), so its page could not be read.
start_watcher()


def feature_is(pid, want_checked, state):
    box, text = switched_feature(pid)
    return box is not None and checked(box) == want_checked and state in text and "Could not" not in text


def stored_switched():
    """The switched feature's row in the database, read after sotyware quit."""
    if not os.path.exists(PREFS_DB):
        return "no database"
    with sqlite3.connect(f"file:{PREFS_DB}?mode=ro", uri=True) as db:
        row = db.execute("SELECT enabled FROM feature_preferences WHERE feature_id = ?",
                         (SWITCHED_ID,)).fetchone()
    return None if row is None else bool(row[0])


def start_tray_instance(label, env=None):
    """Launches in tray mode and waits for the page; returns (process, log, page ready)."""
    open(REG, "w").close()
    p, log = launch(label, env)
    verified(p, label)
    up = wait_for(lambda: one_dashboard_shown(p.pid), 20)
    registered = wait_for(lambda: len(tray_registrations()) == 1, 10)
    page = up and registered and wait_for(lambda: page_ready(p.pid), 20) and wait_for(
        lambda: switched_feature(p.pid)[0] is not None, 10)
    return p, log, bool(page)


def quit_completely(label, p, log):
    """Tray Quit; checks the process really ended, and with it the WebKit
    helpers it started, so that the next launch reads what this one stored,
    as a later start would (see WEBKIT_HELPERS_WAIT). Its helpers are those
    carrying its launch's label: the ones listed before the Quit, children
    of this sotyware, among which its network process must be, and any
    started later. One dumping core after a crash counts as ended and is
    named, unless it is the network process (see CRASHED). Helpers of
    other launches are not this quit's (the check before 17 looks for
    those) and are only named."""
    launch_label = LAUNCHED[p.pid]

    def own():
        return [h for h in webkit_helpers() if h.launch == launch_label]

    before = own()
    quit_at = boot_clock()
    if tray_registrations():
        menu, labels = tray_menu()
        tray_click(menu, labels, "Quit")
    code = wait_exit(p)
    exited_at = boot_clock()
    gone = code == 0 and wait_for(lambda: not sotyware_pids(), 5)
    crashed = {}
    helpers_gone = gone and wait_for(lambda: not split_crashed(own(), crashed), WEBKIT_HELPERS_WAIT,
                                     HELPERS_POLL)
    waited = boot_clock() - exited_at
    network = any(h.program == NETWORK_PROCESS and h.parent == p.pid for h in before)
    network_crashed = [h for h in crashed.values() if h.program == NETWORK_PROCESS]
    ended = gone and helpers_gone and network and not network_crashed
    left = split_crashed(own(), crashed)
    others = [h for h in webkit_helpers() if h.launch != launch_label]
    detail = f"exit code={code} pids={sotyware_pids()} sotyware {p.pid}; its helpers before the Quit: " + (
        ", ".join(f"{h.program} {h.pid} (parent {h.parent})" for h in before) or "none")
    if not network:
        detail += f"; no {NETWORK_PROCESS} of this sotyware among them"
    if helpers_gone:
        detail += (f"; within {waited:.2f} s of sotyware all had ended or were dumping core" if crashed
                   else f"; all ended within {waited:.2f} s of sotyware")
    elif left:
        detail += (f"; left {waited:.2f} s after sotyware: " if gone else "; still running: ") + "; ".join(
            helper_text(h, quit_at, "the Quit") for h in left)
    if crashed:
        detail += f"; {CRASHED_ABOUT}: " + "; ".join(crashed_text(h, quit_at, "the Quit")
                                                     for h in crashed.values())
        CRASHED.extend((label, h) for h in crashed.values())
    if network_crashed:
        detail += f"; its {NETWORK_PROCESS} crashed, so the page's storage may not have been committed"
    if others:
        detail += "; helpers of other launches still running, not waited for: " + "; ".join(
            helper_text(h, quit_at, "the Quit") for h in others)
    check(f"{label}: tray Quit ends the process and its WebKit helpers (exit 0, no process left)", ended, detail)
    if not ended:
        for h in left + network_crashed:
            show_helper(h, with_output=False)  # its output is the instance's, printed next
        print("--- instance output ---\n" + read_log(log).strip())
    return ended


def switch_feature(pid, want_checked, state):
    box = switched_feature(pid)[0]
    return box is not None and Atspi.Action.do_action(box, 0) and wait_for(
        lambda: feature_is(pid, want_checked, state), 5)


# Scenarios 1 to 15 never wait for WebKit's helpers, and 17 to 19 wait at
# each Quit only for the quitting launch's own. A helper of an earlier
# launch still running here, a network process above all, could still
# write the page's storage that 17 reads; on GitHub one was blamed on 17's
# first Quit. It must be gone before 17 starts, and is named if not. One
# dumping core after a crash counts as gone and is named, unless it is a
# network process (see CRASHED).
waited_from = boot_clock()
early_crashed = {}
quiet = wait_for(lambda: not split_crashed(webkit_helpers(), early_crashed), WEBKIT_HELPERS_WAIT,
                 HELPERS_POLL)
early = split_crashed(webkit_helpers(), early_crashed)
early_network = [h for h in early_crashed.values() if h.program == NETWORK_PROCESS]
detail = (f"none still running after {boot_clock() - waited_from:.2f} s" if quiet and not early else
          "left: " + "; ".join(helper_text(h, waited_from, "this wait") for h in early))
if early_crashed:
    detail += f"; {CRASHED_ABOUT}: " + "; ".join(crashed_text(h, waited_from, "this wait")
                                                 for h in early_crashed.values())
    CRASHED.extend(("before 17", h) for h in early_crashed.values())
if early_network:
    detail += f"; a crashed {NETWORK_PROCESS} among them may not have committed the page's storage"
check(f"17 persistence: before it starts, no WebKit helper of an earlier launch is left "
      f"(at most {WEBKIT_HELPERS_WAIT} s)", quiet and not early and not early_network, detail)
for h in early + early_network:
    show_helper(h)

# 17. Disable, quit, relaunch: still disabled. Enable, quit, relaunch: enabled.
# Scenario 12 left Scheduled backups enabled (and saved).
p, log, page = start_tray_instance("17 persistence")
check("17 persistence: starts with Scheduled backups running (saved as enabled by 12)",
      page and feature_is(p.pid, True, "Running") and os.path.exists(PREFS_DB),
      f"text={switched_feature(p.pid)[1] if page else ''!r} db={os.path.exists(PREFS_DB)}")
off = page and switch_feature(p.pid, False, "Disabled")
check("17 persistence: unchecking disables it, no error shown", off,
      f"text={switched_feature(p.pid)[1] if page else ''!r}")
# The theme (ADR-035) is kept by the webview, not the database: choose Dark
# in Settings › General now and look for it after the restart. The quit
# follows within moments, usually before WebKit commits the write, so the
# choice must survive through the commit WebKit makes when sotyware ends,
# which quit_completely waits for (WEBKIT_HELPERS_WAIT).
dark_chosen = page and open_settings_tab(p.pid, "General") and wait_for(
    lambda: theme_radio(p.pid, "Dark") is not None, 5) and Atspi.Action.do_action(
    theme_radio(p.pid, "Dark"), 0) and wait_for(lambda: checked(theme_radio(p.pid, "Dark")), 5)
quit_completely("17 persistence after disabling", p, log)
check("17 persistence: the database stores Scheduled backups as disabled",
      stored_switched() is False, f"stored={stored_switched()!r}")

p, log, page = start_tray_instance("17 persistence relaunch 1")
check("17 persistence relaunch 1: a new process, Scheduled backups still disabled, checkbox unchecked",
      page and feature_is(p.pid, False, "Disabled"),
      f"text={switched_feature(p.pid)[1] if page else ''!r}")
check("17 persistence relaunch 1: no storage warning in the log",
      "feature preferences unavailable" not in read_log(log), read_log(log).strip())
dark_kept = page and open_settings_tab(p.pid, "General") and wait_for(
    lambda: theme_radio(p.pid, "Dark") is not None and checked(theme_radio(p.pid, "Dark")), 5)
check("17 persistence relaunch 1: the theme chosen before quitting (Dark) is still chosen",
      bool(dark_chosen) and bool(dark_kept) and not checked(theme_radio(p.pid, "System")),
      f"chosen={bool(dark_chosen)} kept={bool(dark_kept)} shown={themes_shown(p.pid) if page else []}")
on = page and switch_feature(p.pid, True, "Running")
check("17 persistence relaunch 1: checking enables it, no error shown", on,
      f"text={switched_feature(p.pid)[1] if page else ''!r}")
quit_completely("17 persistence after enabling", p, log)
check("17 persistence: the database stores Scheduled backups as enabled",
      stored_switched() is True, f"stored={stored_switched()!r}")

p, log, page = start_tray_instance("17 persistence relaunch 2")
check("17 persistence relaunch 2: a new process, Scheduled backups running, checkbox checked",
      page and feature_is(p.pid, True, "Running"),
      f"text={switched_feature(p.pid)[1] if page else ''!r}")
quit_completely("17 persistence relaunch 2", p, log)

# 18. A corrupt database: defaults, a visible warning, a visible save failure,
# and the file left exactly as it was.
GARBAGE = b"not an SQLite database\n" * 64
with open(PREFS_DB, "wb") as f:
    f.write(GARBAGE)
p, log, page = start_tray_instance("18 corrupt database")
text = switched_feature(p.pid)[1] if page else ""
check("18 corrupt database: starts, Scheduled backups running by default, page says the choices could not be loaded",
      page and p.poll() is None and checked(switched_feature(p.pid)[0]) and "Running" in text
      and "Could not load the saved feature choices" in text and "not a database" in text, f"text={text!r}")
check("18 corrupt database: logged", "sotyware: feature preferences unavailable, using defaults" in read_log(log),
      read_log(log).strip())
box = switched_feature(p.pid)[0] if page else None
switched = bool(box) and Atspi.Action.do_action(box, 0) and wait_for(
    lambda: "Could not save the choice for Scheduled backups" in switched_feature(p.pid)[1], 5)
text = switched_feature(p.pid)[1] if page else ""
check("18 corrupt database: disabling works for the session and says it was not saved",
      switched and not checked(switched_feature(p.pid)[0]) and "Disabled" in text
      and "It stays disabled until sotyware quits" in text, f"text={text!r}")
quit_completely("18 corrupt database", p, log)
with open(PREFS_DB, "rb") as f:
    check("18 corrupt database: file not repaired, replaced or deleted", f.read() == GARBAGE)
os.remove(PREFS_DB)

# 19. The database cannot be opened (a folder is in its place): defaults and a
# visible warning; sotyware still runs and quits normally. (An unusable data
# folder as a whole is not tested: Tauri needs the same folder for the webview
# and fails its own setup before sotyware's code matters.)
os.mkdir(PREFS_DB)
p, log, page = start_tray_instance("19 database cannot be opened")
text = switched_feature(p.pid)[1] if page else ""
check("19 database cannot be opened: starts, Scheduled backups running by default, page says the choices could not be loaded",
      page and p.poll() is None and "Running" in text and "Could not load the saved feature choices" in text
      and "cannot open" in text, f"text={text!r}")
quit_completely("19 database cannot be opened", p, log)
check("19 database cannot be opened: the folder in its place is left alone", os.path.isdir(PREFS_DB))
os.rmdir(PREFS_DB)
stop_watcher()

# 16. No D-Bus session bus at all: no address in the environment and no socket in
# $XDG_RUNTIME_DIR. Single instance is off without a bus (the plugin disables
# itself), so only the mode and the close behaviour are checked.
nobus_env = {k: v for k, v in os.environ.items() if k != "DBUS_SESSION_BUS_ADDRESS"}
nobus_runtime = os.path.join(LOGS, "runtime-without-bus")
os.mkdir(nobus_runtime, 0o700)
nobus_env["XDG_RUNTIME_DIR"] = nobus_runtime
nb, nb_log = launch("16 no session bus", env=nobus_env)
verified(nb, "16 no session bus")
ok = wait_for(lambda: one_dashboard_shown(nb.pid), 20)
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

check("the user's own sotyware folders are unchanged (names, sizes, times, SHA-256)",
      tp.fingerprint(NORMAL_DIRS) == NORMAL_BEFORE, ", ".join(NORMAL_DIRS))
check("nothing was written under the app's real identifier in the private data folder",
      not os.path.exists(os.path.join(DATA_HOME, tp.NORMAL_IDENTIFIER)))

if CRASHED:
    print(f"\nnote: {len(CRASHED)} WebKit helper(s) crashed as their sotyware quit, each named in the check "
          "given here; a web process's crash fails no check, a network process's does: "
          + "; ".join(f"{h.program} {h.pid} of launch '{h.launch}' ({where})" for where, h in CRASHED))

counts = {s: sum(1 for _, r in results if r == s) for s in ("PASS", "FAIL", "SKIP")}
print(f"\n{counts['PASS']} passed, {counts['FAIL']} failed, {counts['SKIP']} skipped")
sys.exit(1 if counts["FAIL"] else 0)
