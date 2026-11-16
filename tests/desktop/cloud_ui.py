"""Cloud Storage end-to-end test: drives the Cloud Storage page of the real
release binary through AT-SPI (finding and pressing controls) and XTest
(typing: this WebKitGTK exposes no AT-SPI EditableText).

usage: cloud_ui.py local|hang|b2|browse|download|upload|files|usage <binary>   (run by cloud-ui.sh, which
sets up Xvfb + openbox, a private D-Bus session and the Secret Service)

Phases:
  local  a real GNOME Keyring; made-up keys and an endpoint nobody listens on;
         contacts no provider
  hang   a Secret Service that never answers; the window must stay usable
  browse a real GNOME Keyring and the in-process listing server of the Rust
         tests (endpoint in SOTYWARE_UI_ENDPOINT, started by cloud-ui.sh):
         open the bucket, folders, breadcrumbs, Up, Load more, Refresh, Back
  download a real GNOME Keyring and the in-process object server of the
         Rust tests (endpoint in SOTYWARE_UI_ENDPOINT, objects' SHA-256 in
         SOTYWARE_UI_MANIFEST): download through the native GTK save dialog
         (cancel it, save, replace an existing file after its confirmation),
         progress, Cancel, failures with Try again, and quitting during a
         download; every saved file is compared with the server's SHA-256
  upload a real GNOME Keyring, the stub tray watcher and the in-process
         s3s-fs server of serve_uploads_for_ui: upload batches through the
         native GTK multi-file dialog (cloud_ui_upload.py)
  files  file management against the in-memory versioned server of
         serve_files_for_ui (cloud_ui_files.py)
  usage  the bucket's Usage & costs tab against the in-memory versioned
         server of serve_usage_for_ui (figures a measurement must find in
         SOTYWARE_UI_MANIFEST): measure, the figures, the estimate, the
         history, and at three window sizes nothing on the Usage & costs
         and Snapshots & backups tabs past its section or the page's scroll
         area (cloud_ui_usage.py)
  b2     a real GNOME Keyring and the real Backblaze B2 test bucket, with the
         key from SOTYWARE_CLOUD_IT_B2_ACCESS_KEY_ID / _SECRET_ACCESS_KEY and
         SOTYWARE_IT_B2_BUCKET / _REGION (the opt-in integration test's
         variables). Never prints them.
"""
import ctypes
import ctypes.util
import json
import os
import sqlite3
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import test_profile as tp  # noqa: E402
import time

import dbus
import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi, GLib  # noqa: E402

PHASE, BIN = sys.argv[1], sys.argv[2]
RUN = f"{time.time_ns()}"[-12:]

# ---- typing through XTest ----
X = ctypes.cdll.LoadLibrary(ctypes.util.find_library("X11"))
XT = ctypes.cdll.LoadLibrary(ctypes.util.find_library("Xtst"))
W = ctypes.c_ulong
X.XOpenDisplay.restype = ctypes.c_void_p
X.XOpenDisplay.argtypes = [ctypes.c_char_p]
X.XDefaultRootWindow.restype = W
X.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
X.XInternAtom.restype = W
X.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
X.XQueryTree.argtypes = [ctypes.c_void_p, W, ctypes.POINTER(W), ctypes.POINTER(W),
                         ctypes.POINTER(ctypes.POINTER(W)), ctypes.POINTER(ctypes.c_uint)]
X.XGetWindowProperty.argtypes = [ctypes.c_void_p, W, W, ctypes.c_long, ctypes.c_long, ctypes.c_int, W,
                                 ctypes.POINTER(W), ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_ulong),
                                 ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_void_p)]
X.XStringToKeysym.restype = W
X.XStringToKeysym.argtypes = [ctypes.c_char_p]
X.XKeysymToKeycode.restype = ctypes.c_ubyte
X.XKeysymToKeycode.argtypes = [ctypes.c_void_p, W]
X.XKeycodeToKeysym.restype = W
X.XKeycodeToKeysym.argtypes = [ctypes.c_void_p, ctypes.c_ubyte, ctypes.c_int]
X.XSetInputFocus.argtypes = [ctypes.c_void_p, W, ctypes.c_int, W]
X.XRaiseWindow.argtypes = [ctypes.c_void_p, W]
X.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
XT.XTestFakeKeyEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
dpy = X.XOpenDisplay(None)
# Some of the process's windows are unmapped; focusing them is a BadMatch,
# which Xlib's default handler turns into exit().
ERR = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_void_p, ctypes.c_void_p)
_ignore = ERR(lambda d, e: 0)
X.XSetErrorHandler.argtypes = [ERR]
X.XSetErrorHandler(_ignore)
NET_WM_PID = X.XInternAtom(dpy, b"_NET_WM_PID", 0)


def windows_of(pid):
    found = []

    def walk(w):
        t, prop = W(), ctypes.c_void_p()
        fmt, n, after = ctypes.c_int(), ctypes.c_ulong(), ctypes.c_ulong()
        if X.XGetWindowProperty(dpy, w, NET_WM_PID, 0, 1, 0, 6, ctypes.byref(t), ctypes.byref(fmt),
                                ctypes.byref(n), ctypes.byref(after), ctypes.byref(prop)) == 0 \
                and prop.value and n.value:
            if ctypes.cast(prop, ctypes.POINTER(ctypes.c_ulong))[0] == pid:
                found.append(w)
        root, parent, kids, nk = W(), W(), ctypes.POINTER(W)(), ctypes.c_uint()
        if X.XQueryTree(dpy, w, ctypes.byref(root), ctypes.byref(parent), ctypes.byref(kids), ctypes.byref(nk)):
            for i in range(nk.value):
                walk(kids[i])

    walk(X.XDefaultRootWindow(dpy))
    return found


NAMES = {" ": "space", "-": "minus", ":": "colon", "/": "slash", ".": "period", "_": "underscore",
         "+": "plus", "=": "equal"}
SHIFT = X.XKeysymToKeycode(dpy, X.XStringToKeysym(b"Shift_L"))


def key(code, shift=False):
    if shift:
        XT.XTestFakeKeyEvent(dpy, SHIFT, 1, 0)
    XT.XTestFakeKeyEvent(dpy, code, 1, 0)
    XT.XTestFakeKeyEvent(dpy, code, 0, 0)
    if shift:
        XT.XTestFakeKeyEvent(dpy, SHIFT, 0, 0)
    X.XSync(dpy, 0)


def type_text(value):
    for ch in value:
        sym = X.XStringToKeysym(NAMES.get(ch, ch).encode())
        if not sym:
            raise SystemExit("cannot type one of the characters (not printed)")
        code = X.XKeysymToKeycode(dpy, sym)
        key(code, X.XKeycodeToKeysym(dpy, code, 0) != sym)
        time.sleep(0.01)


def keysym(name):
    return X.XKeysymToKeycode(dpy, X.XStringToKeysym(name.encode()))


def ctrl(letter):
    c = keysym("Control_L")
    XT.XTestFakeKeyEvent(dpy, c, 1, 0)
    key(keysym(letter))
    XT.XTestFakeKeyEvent(dpy, c, 0, 0)
    X.XSync(dpy, 0)


NET_CLOSE = X.XInternAtom(dpy, b"_NET_CLOSE_WINDOW", 0)


class XClientMessage(ctypes.Structure):
    _fields_ = [("type", ctypes.c_int), ("serial", ctypes.c_ulong), ("send_event", ctypes.c_int),
                ("display", ctypes.c_void_p), ("window", W), ("message_type", W), ("format", ctypes.c_int),
                ("data", ctypes.c_long * 5)]


class XEvent(ctypes.Union):
    _fields_ = [("xclient", XClientMessage), ("pad", ctypes.c_long * 24)]


X.XSendEvent.argtypes = [ctypes.c_void_p, W, ctypes.c_int, ctypes.c_long, ctypes.POINTER(XEvent)]


def close_windows(pid):
    """Asks the window manager to close the process's windows, as the title
    bar's X does (_NET_CLOSE_WINDOW)."""
    root = X.XDefaultRootWindow(dpy)
    for w in windows_of(pid):
        ev = XEvent()
        ev.xclient.type, ev.xclient.window, ev.xclient.message_type, ev.xclient.format = 33, w, NET_CLOSE, 32
        ev.xclient.data[0], ev.xclient.data[1] = 0, 2
        X.XSendEvent(dpy, root, 0, (1 << 20) | (1 << 19), ctypes.byref(ev))
    X.XSync(dpy, 0)


class _WinAttrs(ctypes.Structure):
    _fields_ = [("x", ctypes.c_int), ("y", ctypes.c_int), ("width", ctypes.c_int), ("height", ctypes.c_int),
                ("border_width", ctypes.c_int), ("depth", ctypes.c_int), ("visual", ctypes.c_void_p),
                ("root", W), ("class_", ctypes.c_int), ("bit_gravity", ctypes.c_int),
                ("win_gravity", ctypes.c_int), ("backing_store", ctypes.c_int),
                ("backing_planes", ctypes.c_ulong), ("backing_pixel", ctypes.c_ulong),
                ("save_under", ctypes.c_int), ("colormap", W), ("map_installed", ctypes.c_int),
                ("map_state", ctypes.c_int), ("all_event_masks", ctypes.c_long),
                ("your_event_mask", ctypes.c_long), ("do_not_propagate_mask", ctypes.c_long),
                ("override_redirect", ctypes.c_int), ("screen", ctypes.c_void_p)]


X.XGetWindowAttributes.argtypes = [ctypes.c_void_p, W, ctypes.POINTER(_WinAttrs)]


def focus_window(pid):
    """Gives the X input focus to the process's viewable windows, the
    largest last (the dashboard, or a native dialog above it). Windows that
    are not mapped are skipped: GTK keeps a closed dialog's or popup menu's
    window, and focusing it last sent every later key nowhere (seen
    2026-11-14 after a right-click menu and the native folder dialogs)."""
    shown = []
    for w in windows_of(pid):
        a = _WinAttrs()
        if X.XGetWindowAttributes(dpy, w, ctypes.byref(a)) and a.map_state == 2 and not a.override_redirect:
            shown.append((a.width * a.height, w))
    for _, w in sorted(shown):
        X.XRaiseWindow(dpy, w)
        X.XSetInputFocus(dpy, w, 2, 0)
    X.XSync(dpy, 0)


# ---- the keys this phase types ----
if PHASE == "b2":
    def need(name):
        v = os.environ.get(name, "")
        if not v:
            raise SystemExit(f"set {name} for the b2 phase")
        return v

    KEY1 = need("SOTYWARE_CLOUD_IT_B2_ACCESS_KEY_ID")
    SECRET1 = need("SOTYWARE_CLOUD_IT_B2_SECRET_ACCESS_KEY")
    BUCKET = need("SOTYWARE_IT_B2_BUCKET")
    REGION = need("SOTYWARE_IT_B2_REGION")
    # A wrong secret for the same key id: shows that a replaced key is the one used.
    KEY2, SECRET2 = KEY1, "sotywareE2eWrongSecret0123456789abcdef"
elif PHASE == "b2-browse":
    def need(name):
        v = os.environ.get(name, "")
        if not v:
            raise SystemExit(f"set {name} for the b2-browse phase")
        return v

    # The read-only key (no file name prefix), so the root can be listed.
    KEY1 = need("SOTYWARE_CLOUD_IT_B2_READ_ACCESS_KEY_ID")
    SECRET1 = need("SOTYWARE_CLOUD_IT_B2_READ_SECRET_ACCESS_KEY")
    KEY2, SECRET2 = "", ""
    BUCKET = need("SOTYWARE_IT_B2_BUCKET")
    REGION = need("SOTYWARE_IT_B2_REGION")
    with open(need("SOTYWARE_UI_MANIFEST"), encoding="utf-8") as mf:
        MANIFEST = json.load(mf)
    MANIFEST["folders"] = {f["path"]: f for f in MANIFEST["folders"]}
elif PHASE == "download":
    # The made-up key of serve_objects_for_ui (src-tauri/src/cloud/test_server.rs).
    KEY1, SECRET1 = "e2eDownloadKey00000001", "e2e-Download-Secret-0123456789"
    KEY2, SECRET2 = "", ""
    BUCKET, REGION = "download-bucket", "eu-central-003"
    ENDPOINT = os.environ["SOTYWARE_UI_ENDPOINT"]
    with open(os.environ["SOTYWARE_UI_MANIFEST"], encoding="utf-8") as mf:
        OBJECTS = {o["key"]: o for o in json.load(mf)}
elif PHASE == "upload":
    # The made-up key of serve_uploads_for_ui (src-tauri/src/cloud/test_server.rs).
    KEY1, SECRET1 = "e2eUploadKey0000000001", "e2e-Upload-Secret-0123456789"
    KEY2, SECRET2 = "", ""
    BUCKET, REGION = "upload-bucket", "eu-central-003"
    ENDPOINT = os.environ["SOTYWARE_UI_ENDPOINT"]
    CONTROL = os.environ["SOTYWARE_UI_CONTROL"]
    with open(os.environ["SOTYWARE_UI_MANIFEST"], encoding="utf-8") as mf:
        SEED = {o["key"]: o for o in json.load(mf)}
elif PHASE == "files":
    # The made-up key of serve_files_for_ui (src-tauri/src/cloud/test_server/files_ui.rs).
    KEY1, SECRET1 = "e2eFilesKey00000000001", "e2e-Files-Secret-0123456789"
    KEY2, SECRET2 = "", ""
    BUCKET, REGION = "files-versioned", "eu-central-003"
    ENDPOINT = os.environ["SOTYWARE_UI_ENDPOINT"]
    CONTROL = os.environ["SOTYWARE_UI_CONTROL"]
elif PHASE == "usage":
    # The made-up key of serve_usage_for_ui (src-tauri/src/cloud/test_server/usage_ui.rs).
    KEY1, SECRET1 = "e2eUsageKey00000000001", "e2e-Usage-Secret-0123456789"
    KEY2, SECRET2 = "", ""
    BUCKET, REGION = "usage-versioned", "eu-central-003"
    ENDPOINT = os.environ["SOTYWARE_UI_ENDPOINT"]
    with open(os.environ["SOTYWARE_UI_MANIFEST"], encoding="utf-8") as mf:
        EXPECTED = json.load(mf)
elif PHASE == "browse":
    # The made-up key of serve_listing_for_ui (src-tauri/src/cloud/test_server.rs).
    KEY1, SECRET1 = "e2eBrowseKey0000000001", "e2e-Browse-Secret-0123456789"
    KEY2, SECRET2 = "", ""
    BUCKET, REGION = "browse-bucket", "eu-central-003"
    ENDPOINT = os.environ["SOTYWARE_UI_ENDPOINT"]
else:
    SECRET1, KEY1 = "e2e-Secret-One-0123456789abcdef", "e2eKeyIdOne000000000001"
    SECRET2, KEY2 = "e2e-Secret-Two-fedcba9876543210", "e2eKeyIdTwo000000000002"
    BUCKET, REGION = "soty-photos", "eu-central-003"
REDACT = [(s, r) for s, r in ((SECRET1, "<secret>"), (SECRET2, "<secret2>"), (KEY1, "<keyID>"),
                              (KEY2, "<keyID2>"), (BUCKET, "<bucket>")) if s]


def redact(s):
    for value, placeholder in REDACT:
        s = s.replace(value, placeholder)
    return s


results = []


def check(name, ok, detail=""):
    results.append(bool(ok))
    shown = f" — {redact(str(detail))[:800]}" if detail and not ok else ""
    print(f"[{'PASS' if ok else 'FAIL'}] {PHASE}: {name}{shown}", flush=True)


def wait_for(pred, timeout=10.0):
    end = time.time() + timeout
    while time.time() < end:
        try:
            v = pred()
            if v:
                return v
        except GLib.Error:
            pass
        time.sleep(0.1)
    return None


def nodes(pid):
    desktop = Atspi.get_desktop(0)
    for i in range(desktop.get_child_count()):
        app = desktop.get_child_at_index(i)
        try:
            if app is None or app.get_process_id() != pid:
                continue
            app.clear_cache()
        except GLib.Error:
            continue
        out = []

        def walk(n):
            try:
                t = ""
                if n.get_text_iface() is not None:
                    t = Atspi.Text.get_text(n, 0, Atspi.Text.get_character_count(n))
                out.append((n.get_role(), n.get_name() or "", t or "", n))
                for j in range(n.get_child_count()):
                    c = n.get_child_at_index(j)
                    if c is not None:
                        walk(c)
            except GLib.Error:
                pass

        walk(app)
        return out
    return []


def text(pid):
    return " ".join(" ".join((n, t)) for _, n, t, _ in nodes(pid)).replace("￼", " ")


def by_role(pid, *roles):
    return [(n, node) for r, n, _, node in nodes(pid) if r in roles]


def act(pid, roles, name):
    for n, node in by_role(pid, *roles):
        if n == name:
            return Atspi.Action.do_action(node, 0)
    return False


def button(pid, name):
    return act(pid, (Atspi.Role.PUSH_BUTTON,), name)


def link(pid, name):
    return act(pid, (Atspi.Role.LINK,), name)


def buttons(pid):
    return [n for n, _ in by_role(pid, Atspi.Role.PUSH_BUTTON)]


ENTRY_ROLES = (Atspi.Role.ENTRY, Atspi.Role.PASSWORD_TEXT)


def fields(pid):
    return [n for n, _ in by_role(pid, *ENTRY_ROLES)]


def fill(pid, name, value):
    """Focuses the field and types `value` as real key events, after clearing
    it with BackSpace."""
    for n, node in by_role(pid, *ENTRY_ROLES):
        if n == name:
            focus_window(pid)
            Atspi.Component.grab_focus(node)
            if not wait_for(lambda: node.get_state_set().contains(Atspi.StateType.FOCUSED), 3):
                print(f"  fill {name!r}: could not focus", flush=True)
                return False
            end = X.XKeysymToKeycode(dpy, X.XStringToKeysym(b"End"))
            bs = X.XKeysymToKeycode(dpy, X.XStringToKeysym(b"BackSpace"))
            key(end)
            for _ in range(Atspi.Text.get_character_count(node)):
                key(bs)
            type_text(value)
            ok = wait_for(lambda: Atspi.Text.get_character_count(node) == len(value), 5)
            if not ok:
                print(f"  fill {name!r}: field has {Atspi.Text.get_character_count(node)} characters, "
                      f"expected {len(value)}", flush=True)
            return bool(ok)
    print(f"  no field {name!r}; fields: {fields(pid)}", flush=True)
    return False


def press(pid, name):
    """Presses the button whose accessible name, without surrounding
    whitespace, is `name` (folder buttons also hold a hidden icon)."""
    for n, node in by_role(pid, Atspi.Role.PUSH_BUTTON):
        if n.strip() == name or n.replace("📁", "").strip() == name:
            try:
                return Atspi.Action.do_action(node, 0)
            except GLib.Error as e:
                # The press was sent, but the reply did not come in time (seen
                # once while a 1,000-row page was torn down under load). The
                # caller's wait_for checks whether it took effect.
                print(f"  press {name!r}: no AT-SPI reply ({e.message[:60]})", flush=True)
                return True
    print(f"  no button {name!r}; buttons: {[b.strip() for b in buttons(pid)][:40]}", flush=True)
    return False


def checkbox(pid, name):
    return act(pid, (Atspi.Role.CHECK_BOX,), name)


# ---- the Secret Service, read from outside sotyware ----
def secret_service():
    bus = dbus.SessionBus()
    return dbus.Interface(bus.get_object("org.freedesktop.secrets", "/org/freedesktop/secrets"),
                          "org.freedesktop.Secret.Service")


def keyring_items(account_id):
    """(unlocked items, locked items, stored values); the values are only
    compared, never printed."""
    svc = secret_service()
    # Only this run's namespace (ADR-027): service com.sotyware.desktop.test,
    # user names prefixed with the profile's nonce.
    unlocked, locked = svc.SearchItems({"service": tp.CREDENTIAL_SERVICE,
                                        "username": PROFILE.credential_user(account_id)})
    values = []
    if unlocked:
        _, session = svc.OpenSession("plain", dbus.String("", variant_level=1))
        secrets = svc.GetSecrets(unlocked, session)
        values = [bytes(s[2]).decode() for s in secrets.values()]
    return list(unlocked), list(locked), values


def stored(key_id, secret):
    return [f"sotyware-s3-credentials-v1\n{key_id}\n{secret}"]


def lock_login():
    return secret_service().Lock([dbus.ObjectPath("/org/freedesktop/secrets/collection/login")])


# Where GNOME Keyring's system prompter is installed (gnome-keyring from the
# distribution: org.gnome.keyring.SystemPrompter.service), the private session
# bus starts gcr-prompter for a locked collection and its unlock dialog waits
# for a person. An unpacked gnome-keyring has no prompter and refuses at once.
# Either way the user did not unlock; the test answers the dialog the way a
# person who declines would.
prompts_dismissed = 0


def dismiss_prompts():
    """Presses Cancel in every open gcr-prompter dialog; how many it pressed."""
    global prompts_dismissed
    found = subprocess.run(["pgrep", "-x", "gcr-prompter"], capture_output=True, text=True).stdout.split()
    pressed = 0
    for prompter in found:
        for n, node in by_role(int(prompter), Atspi.Role.PUSH_BUTTON):
            if n == "Cancel" and Atspi.Action.do_action(node, 0):
                pressed += 1
    prompts_dismissed += pressed
    return pressed


def declining_prompts(pred):
    """`pred`, dismissing any unlock prompt while it is not yet true."""
    def wrapped():
        dismiss_prompts()
        return pred()
    return wrapped


# ---- the app, in this run's test profile and with none of the test variables ----
# ADR-027: a test-profile build that refuses to start without the profile,
# keeps its databases and credentials in it, and acknowledges it; nothing is
# clicked before that acknowledgement matches this launch. XDG_DATA_HOME
# points into the throwaway HOME as a second layer.
tp.require_test_profile_build(BIN)
data = tempfile.mkdtemp(prefix=f"data-{PHASE}-", dir=os.environ["HOME"])
PROFILE = tp.create(os.environ["HOME"], f"profile-{PHASE}-{os.getpid()}")
DB = PROFILE.db("cloud-accounts.sqlite3")
log = os.path.join(os.environ["HOME"], f"sotyware-{PHASE}.log")
env = {k: v for k, v in os.environ.items() if not k.startswith(("SOTYWARE_IT_", "SOTYWARE_CLOUD_IT_"))}
env["XDG_DATA_HOME"] = data
env[tp.ENV] = PROFILE.root
with open(log, "wb") as f:
    p = subprocess.Popen([BIN], env=env, stdout=f, stderr=subprocess.STDOUT)
pid = p.pid
try:
    ACK = tp.verify_ack(PROFILE, pid, timeout=30, alive=lambda: p.poll() is None)
except tp.IsolationError as e:
    p.kill()
    print(f"FAIL  the app runs in this run's test profile: {e}", flush=True)
    print(f"ABORT {PHASE}: the app is not provably isolated; nothing was clicked", flush=True)
    sys.exit(1)
print(f"PASS  the app runs in this run's test profile (pid {pid}, {PROFILE.root})", flush=True)

def rows():
    with sqlite3.connect(f"file:{DB}?mode=ro", uri=True) as db:
        return db.execute("SELECT account_id, label, provider, region, endpoint FROM storage_accounts "
                          "ORDER BY account_id").fetchall()


def db_has_no_key():
    raw = open(DB, "rb").read()
    return all(s.encode() not in raw for s in (SECRET1, SECRET2, KEY1, KEY2) if s)


def page_has_no_key(t):
    return all(s not in t for s in (SECRET1, SECRET2, KEY1, KEY2) if s)


def add(label, bucket, key_id, secret, endpoint, region=REGION):
    ok = button(pid, "Add account") and wait_for(lambda: "Secret access key" in fields(pid))
    ok = ok and fill(pid, "Name", label) and fill(pid, "Bucket", bucket) and fill(pid, "Region", region)
    if endpoint:
        ok = ok and fill(pid, "Custom endpoint (optional)", endpoint)
    ok = ok and fill(pid, "Access key ID", key_id) and fill(pid, "Secret access key", secret)
    return ok and button(pid, "Add account")


LABELS = ["Connected", "Invalid credentials", "Access denied", "Bucket unavailable", "Network error",
          "Credential store unavailable", "Credential store locked", "Credentials missing",
          "Configuration problem", "Check failed"]


def run_check(timeout=60, decline_prompts=False):
    """Presses Check connection on the only account; the status label shown
    afterwards, or None. With `decline_prompts`, an unlock prompt that opens
    meanwhile is cancelled."""
    if not button(pid, "Check connection"):
        return None

    def done():
        if decline_prompts:
            dismiss_prompts()
        t = text(pid)
        if "Checking connection…" in t:
            return None
        return next((label for label in LABELS if label in t), None)

    return wait_for(done, timeout)


def edit(pid_):
    return button(pid_, "Edit") and wait_for(lambda: "Save changes" in buttons(pid_))


def replace_key(label, key_id, secret):
    ok = edit(pid) and checkbox(pid, "Replace access key") and wait_for(lambda: "Secret access key" in fields(pid))
    ok = ok and fill(pid, "Access key ID", key_id) and fill(pid, "Secret access key", secret)
    ok = ok and button(pid, "Save changes")
    return bool(ok and wait_for(lambda: f"Saved {label}" in text(pid) and "Save changes" not in buttons(pid)))


def remove(label):
    ok = button(pid, "Remove") and wait_for(lambda: "Remove account" in buttons(pid))
    confirm = text(pid)
    removed = ok and button(pid, "Remove account") and wait_for(lambda: f"Removed {label}" in text(pid), 30)
    return bool(ok), confirm, bool(removed)


# ---- the bucket browser's table, read through AT-SPI ----
def cell_text(cell):
    parts = []

    def walk(n):
        try:
            if n.get_text_iface() is not None:
                parts.append(Atspi.Text.get_text(n, 0, Atspi.Text.get_character_count(n)).replace("\ufffc", " "))
            elif n.get_name():
                parts.append(n.get_name())
            for j in range(n.get_child_count()):
                c = n.get_child_at_index(j)
                if c is not None:
                    walk(c)
        except GLib.Error:
            pass

    walk(cell)
    return " ".join(" ".join(parts).replace("📁", " ").split())


def table_rows(pid):
    """The browser table's body rows, each as [name, size, date] texts."""
    out = []
    for role, _, _, node in nodes(pid):
        if role != Atspi.Role.TABLE_ROW:
            continue
        # Name, size and date; the fourth cell holds the Download action.
        cells = [cell_text(node.get_child_at_index(j)) for j in range(min(3, node.get_child_count()))]
        if cells != ["Name", "Size", "Last modified"]:
            out.append(cells)
    return out


def format_size(n):
    if n < 1024:
        return f"{n} B"
    value, unit = n / 1024, 0
    while value >= 1024 and unit < 4:
        value, unit = value / 1024, unit + 1
    return f"{value:.{1 if value < 10 else 0}f} {['KiB', 'MiB', 'GiB', 'TiB', 'PiB'][unit]}"


def format_dates(dates):
    """The page's formatDate for each RFC 3339 string, by the same Intl call
    in Node (same locale and time zone as the app's environment)."""
    js = ("const a=JSON.parse(require('fs').readFileSync(0,'utf8'));"
          "console.log(JSON.stringify(a.map(s=>new Date(s).toLocaleString([],{year:'numeric',month:'short',"
          "day:'numeric',hour:'2-digit',minute:'2-digit'}).replace(/[\\u202f\\u00a0]/g,' '))))")
    out = subprocess.run(["node", "-e", js], input=json.dumps(dates), capture_output=True, text=True, check=True)
    return dict(zip(dates, json.loads(out.stdout)))


def same_text(a):
    return " ".join(a.replace("\u202f", " ").replace("\u00a0", " ").split())


def expected_rows(rel, first_page=False):
    f = MANIFEST["folders"][rel]
    files = f["files"]
    folders = f["folders"]
    if first_page:  # the provider's first page: the first 1000 entries in key order
        folders, files = [], files[:1000]
    dates = format_dates([x["lastModified"] for x in files])
    return ([[n if n else "(no name)", "Folder", "—"] for n in folders]
            + [[x["name"], format_size(x["size"]), dates[x["lastModified"]]] for x in files])


def shows(rel, first_page=False):
    """Waits until the browser shows folder `rel` of the run (all of it, or
    its first page) with every row as the provider listed it: name, size
    and date, in order. Returns (ok, what differs)."""
    want = expected_rows(rel, first_page)
    path = MANIFEST["prefix"] + rel
    norm = lambda rows: [[same_text(c) for c in r] for r in rows]  # noqa: E731
    last = []

    def ready():
        ns = nodes(pid)
        if not any(path in (n, t.strip()) for _, n, t, _ in ns) or any("Loading" in t for _, _, t, _ in ns):
            return None
        if not want:
            return "This folder is empty." in text(pid)
        last[:] = norm(table_rows(pid))
        return last == norm(want)

    if wait_for(ready, 120):
        return True, ""
    diff = [(i, g, w) for i, (g, w) in enumerate(zip(last, norm(want))) if g != w][:5]
    return False, f"{path!r}: {len(last)} rows shown, {len(want)} expected; first differences {diff}"


try:
    ready = wait_for(lambda: "Welcome" in text(pid), 30)
    check("dashboard is up (tray mode, stub watcher)" if PHASE == "upload"
          else "dashboard is up (dashboard-only mode, no tray host here)", bool(ready))
    check("Cloud Storage link opens the empty page",
          link(pid, "Cloud Storage") and bool(wait_for(lambda: "No storage accounts yet" in text(pid))))

    if PHASE == "local":
        endpoint = "http://127.0.0.1:9"  # discard port: nothing listens
        ok = add("Photos", BUCKET, KEY1, SECRET1, endpoint)
        check("add account through the form", bool(ok and wait_for(lambda: "Added Photos" in text(pid))), text(pid))
        unl, lck, vals = keyring_items("photos")
        check("the key is in GNOME Keyring under cloud-account/photos, exactly as typed",
              len(unl) == 1 and vals == stored(KEY1, SECRET1), f"{len(unl)} unlocked, {len(lck)} locked")
        check("the account database has the account and no key",
              rows() == [("photos", "Photos", "backblaze-b2", REGION, endpoint)] and db_has_no_key(), str(rows()))
        t = text(pid)
        check("page shows the account, not the key", BUCKET in t and page_has_no_key(t) and fields(pid) == [], t)
        status = run_check(40)
        check("Check connection shows a network error for an endpoint nobody listens on",
              status == "Network error", f"{status}: {text(pid)}")
        ok = press(pid, "Open bucket") and wait_for(lambda: "Could not load this folder." in text(pid), 60)
        t = text(pid)
        check("Open bucket: the browser shows the network error with Try again",
              bool(ok) and "The provider could not be reached" in t and "Try again" in buttons(pid)
              and page_has_no_key(t), t)
        ok = press(pid, "Back to accounts") and wait_for(lambda: "Check connection" in buttons(pid), 10)
        check("Back to accounts from the error", bool(ok), str(buttons(pid)))
        ok = edit(pid) and fill(pid, "Name", "Family photos") and button(pid, "Save changes")
        check("rename without re-entering the key",
              bool(ok and wait_for(lambda: "Saved Family photos" in text(pid)))
              and rows()[0][1] == "Family photos" and keyring_items("photos")[2] == stored(KEY1, SECRET1),
              str(rows()))
        replaced = replace_key("Family photos", KEY2, SECRET2)
        unl, _, vals = keyring_items("photos")
        check("replace the key: one entry, now the new key",
              replaced and len(unl) == 1 and vals == stored(KEY2, SECRET2))
        ok, confirm, removed = remove("Family photos")
        check("removal asks first and says remote data stays", ok and "are not deleted or changed" in confirm, confirm)
        check("remove: account gone from page, database and keyring",
              removed and rows() == [] and keyring_items("photos")[:2] == ([], []), str(rows()))
        # A locked keyring: the check reports it as its own status.
        ok = add("Second", "soty-second", KEY1, SECRET1, endpoint) and wait_for(lambda: "Added Second" in text(pid))
        lock_login()
        status = run_check(40, decline_prompts=True)
        print(f"  locked keyring, check: {status}; unlock prompts declined: {prompts_dismissed}", flush=True)
        check("locked keyring: check says Credential store locked", ok and status == "Credential store locked",
              f"{status}: {text(pid)}")
        rem = button(pid, "Remove") and wait_for(lambda: "Remove account" in buttons(pid)) \
            and button(pid, "Remove account")
        outcome = wait_for(declining_prompts(lambda: ("Removed Second" in text(pid) and "removed")
                                             or ("still listed" in text(pid) and "kept")), 30)
        print(f"  locked keyring, remove: {outcome}; db: {rows()}; "
              f"unlock prompts declined in total: {prompts_dismissed}", flush=True)
        check("locked keyring: removal never claims success while the key stays",
              rem and (outcome == "kept" and rows() != []
                       or outcome == "removed" and keyring_items("second")[:2] == ([], [])), str(outcome))

    elif PHASE == "browse":
        ok = add("Browse", BUCKET, KEY1, SECRET1, ENDPOINT)
        check("add an account for the local listing server",
              bool(ok and wait_for(lambda: "Added Browse" in text(pid))), text(pid))
        ok = press(pid, "Open bucket") and wait_for(lambda: "readme.txt" in text(pid), 30)
        t = text(pid)
        check("Open bucket lists the root: folders, files, size, the bucket and the account",
              bool(ok) and all(s in t for s in ("Root of the bucket", "photos", "Ferien 2024 ü", "empty", "many",
                                                "readme.txt", "12 B", "Browse", BUCKET))
              and "cat.jpg" not in t and "item-0000" not in t, t)
        check("the account list is replaced by the browser", "Add account" not in buttons(pid), str(buttons(pid)))
        ok = press(pid, "Ferien 2024 ü") and wait_for(lambda: "Strand Bild (1).jpg" in text(pid), 20)
        t = text(pid)
        check("open a folder with spaces and Unicode: its files, exact names",
              bool(ok) and "a+b=c&d.txt" in t and "Ferien 2024 ü/" in t and "readme.txt" not in t, t)
        ok = press(pid, BUCKET) and wait_for(lambda: "readme.txt" in text(pid) and "Root of the bucket" in text(pid), 20)
        check("the bucket breadcrumb goes back to the root", bool(ok), text(pid))
        ok = press(pid, "photos") and wait_for(lambda: "cat.jpg" in text(pid), 20)
        t = text(pid)
        check("open photos/: subfolder and file, not the nested file, not the marker",
              bool(ok) and "2024" in t and "beach.jpg" not in t and "2.0 KiB" in t, t)
        ok = press(pid, "2024") and wait_for(lambda: "beach.jpg" in text(pid), 20)
        check("open photos/2024/: the nested file with its size", bool(ok) and "3.0 MiB" in text(pid), text(pid))
        ok = press(pid, "Up one folder") and wait_for(lambda: "cat.jpg" in text(pid) and "beach.jpg" not in text(pid), 20)
        check("Up one folder goes to photos/", bool(ok), text(pid))
        ok = press(pid, BUCKET) and wait_for(lambda: "readme.txt" in text(pid), 20)
        ok = ok and press(pid, "empty") and wait_for(lambda: "This folder is empty." in text(pid), 20)
        check("a folder that is only a marker opens as empty", bool(ok), text(pid))
        ok = press(pid, BUCKET) and wait_for(lambda: "readme.txt" in text(pid), 20)
        ok = ok and press(pid, "many") and wait_for(lambda: "1000 items shown; more are available" in text(pid), 60)
        t = text(pid)
        check("a folder of 1,005 entries loads one page of 1,000 with Load more",
              bool(ok) and "Load more" in buttons(pid) and "item-0999.txt" in t and "item-1000.txt" not in t,
              t[-300:])
        ok = press(pid, "Load more") and wait_for(lambda: "1005 items shown." in text(pid), 60)
        t = text(pid)
        check("Load more appends the rest, no Load more after the last page",
              bool(ok) and "item-1004.txt" in t and "Load more" not in buttons(pid), t[-300:])
        ok = press(pid, "Refresh") and wait_for(lambda: "1000 items shown; more are available" in text(pid), 60)
        check("Refresh starts again from the first page", bool(ok), text(pid)[-300:])
        ok = press(pid, "Back to accounts") and wait_for(lambda: "Open bucket" in buttons(pid), 20)
        check("Back to accounts shows the account list again", bool(ok), str(buttons(pid)))
        ok, confirm, removed = remove("Browse")
        check("remove the account afterwards", removed and rows() == [], str(rows()))

    elif PHASE == "download":
        import hashlib
        out = os.path.join(os.environ["HOME"], "downloads")
        os.makedirs(out, exist_ok=True)

        def saved():
            return sorted(os.listdir(out))

        def sha_of(path):
            with open(path, "rb") as fh:
                return hashlib.sha256(fh.read()).hexdigest()

        def chooser():
            """The open GTK save dialog's name field, or None."""
            for role, name, _, node in nodes(pid):
                if role == Atspi.Role.FILE_CHOOSER or (role == Atspi.Role.DIALOG and name == "Save download as"):
                    found = []

                    def walk(n):
                        try:
                            if n.get_role() == Atspi.Role.TEXT and n.get_state_set().contains(Atspi.StateType.EDITABLE) \
                                    and n.get_state_set().contains(Atspi.StateType.SHOWING):
                                found.append(n)
                            for j in range(n.get_child_count()):
                                c = n.get_child_at_index(j)
                                if c is not None:
                                    walk(c)
                        except GLib.Error:
                            pass

                    walk(node)
                    if found:
                        return found[0]
            return None

        def entry_text(node):
            return Atspi.Text.get_text(node, 0, Atspi.Text.get_character_count(node))

        def open_dialog(button_name):
            if not press(pid, button_name):
                return None
            return wait_for(chooser, 20)

        def save_as(entry, path):
            """Types `path` into the dialog's name field and presses Enter."""
            Atspi.Component.grab_focus(entry)
            wait_for(lambda: entry.get_state_set().contains(Atspi.StateType.FOCUSED), 3)
            ctrl("a")
            type_text(path)
            if not wait_for(lambda: entry_text(entry) == path, 5):
                print(f"  name field: {entry_text(entry)!r}", flush=True)
                return False
            key(keysym("Return"))
            return True

        def panel_says(*parts, timeout=60):
            return wait_for(lambda: all(x in text(pid) for x in parts), timeout)

        ok = add("Downloads", BUCKET, KEY1, SECRET1, ENDPOINT)
        check("add an account for the local object server",
              bool(ok and wait_for(lambda: "Added Downloads" in text(pid))), text(pid))
        ok = press(pid, "Open bucket") and wait_for(lambda: "readme.txt" in text(pid), 30)
        check("Open bucket lists the files with a Download action each",
              bool(ok) and "Download readme.txt" in buttons(pid) and "Download foreign.txt" in buttons(pid),
              str(buttons(pid)))

        entry = open_dialog("Download readme.txt")
        check("Download opens the native save dialog with the file name suggested",
              entry is not None and entry_text(entry) == "readme.txt",
              entry_text(entry) if entry else text(pid)[-300:])
        check("while the dialog is open, every Download action is disabled",
              entry is not None and "Choose where to save the file" in text(pid))
        if entry is not None:
            Atspi.Component.grab_focus(entry)
            key(keysym("Escape"))
        gone = wait_for(lambda: chooser() is None and "Choose where to save" not in text(pid), 10)
        check("cancelling the dialog starts nothing and leaves no file", bool(gone) and saved() == [], str(saved()))

        target = os.path.join(out, "readme copy.txt")
        entry = open_dialog("Download readme.txt")
        ok = entry is not None and save_as(entry, target)
        done = ok and panel_says("Downloaded", "SHA-256 checksum stored with the file matched")
        check("save: the file is downloaded and its SHA-256 verified",
              bool(done) and saved() == ["readme copy.txt"] and sha_of(target) == OBJECTS["readme.txt"]["sha256"],
              f"{saved()} {text(pid)[-300:]}")
        press(pid, "Dismiss")

        mine = os.path.join(out, "existing.txt")
        with open(mine, "w") as fh:
            fh.write("an older file of mine")
        entry = open_dialog("Download readme.txt")
        ok = entry is not None and save_as(entry, mine)
        confirm = ok and wait_for(lambda: "Replace" in buttons(pid) and "already exists" in text(pid), 10)
        check("an existing file: the dialog asks before replacing it, and nothing is written meanwhile",
              bool(confirm) and open(mine).read() == "an older file of mine", text(pid)[-300:])
        ok = bool(confirm) and press(pid, "Replace") and panel_says("Downloaded", "matched")
        check("after Replace the file is replaced in place, no temporary file left",
              bool(ok) and sha_of(mine) == OBJECTS["readme.txt"]["sha256"]
              and saved() == ["existing.txt", "readme copy.txt"], str(saved()))
        press(pid, "Dismiss")

        ok = press(pid, "docs") and wait_for(lambda: "Bericht 2024 ü & co.pdf" in text(pid), 20)
        key_ = "docs/Bericht 2024 ü & co.pdf"
        entry = ok and open_dialog("Download Bericht 2024 ü & co.pdf")
        check("a Unicode name with & and spaces is suggested unchanged",
              bool(entry) and entry_text(entry) == "Bericht 2024 ü & co.pdf", entry_text(entry) if entry else "")
        ok = bool(entry) and save_as(entry, os.path.join(out, "bericht.pdf"))
        done = ok and panel_says("Downloaded 3.0 MiB", "matched")
        check("the 3 MiB file under its exact key, SHA-256 verified",
              bool(done) and sha_of(os.path.join(out, "bericht.pdf")) == OBJECTS[key_]["sha256"], text(pid)[-300:])
        press(pid, "Dismiss")
        press(pid, BUCKET) and wait_for(lambda: "readme.txt" in text(pid), 20)

        entry = open_dialog("Download foreign.txt")
        ok = entry is not None and save_as(entry, os.path.join(out, "foreign.txt"))
        done = ok and panel_says("Downloaded", "no SHA-256 checksum stored by sotyware")
        check("a file without a stored SHA-256: completed, and the page says only the size was checked",
              bool(done) and "checksum stored with the file matched" not in text(pid)
              and sha_of(os.path.join(out, "foreign.txt")) == OBJECTS["foreign.txt"]["sha256"], text(pid)[-300:])
        press(pid, "Dismiss")

        ok = press(pid, "slow") and wait_for(lambda: "large.bin" in text(pid), 20)
        entry = ok and open_dialog("Download large.bin")
        ok = bool(entry) and save_as(entry, os.path.join(out, "large.bin"))
        progress = ok and panel_says("% ·", "of 16 MiB", timeout=20)
        partial = [n for n in saved() if n.endswith(".sotyware-partial")]
        check("progress shows percent and bytes; the data goes to a hidden temporary file",
              bool(progress) and len(partial) == 1 and partial[0].startswith(".large.bin.")
              and "large.bin" not in saved(), f"{saved()} {text(pid)[-200:]}")
        check("while downloading, the other Download actions are disabled and Cancel is offered",
              "Cancel download" in buttons(pid))
        ok = press(pid, "Cancel download") and panel_says("Download cancelled. Nothing was saved.", timeout=15)
        check("Cancel stops it: no file and no temporary file",
              bool(ok) and not any(n.startswith(".large.bin") or n == "large.bin" for n in saved()), str(saved()))
        press(pid, "Dismiss")
        press(pid, BUCKET) and wait_for(lambda: "readme.txt" in text(pid), 20)

        ok = press(pid, "broken") and wait_for(lambda: "cut.bin" in text(pid), 20)
        entry = ok and open_dialog("Download cut.bin")
        ok = bool(entry) and save_as(entry, os.path.join(out, "cut.bin"))
        failed = ok and panel_says("Download failed. Nothing was saved.", "could not be reached", timeout=30)
        check("a broken connection: failed, with Try again, nothing saved",
              bool(failed) and "Try download again" in buttons(pid) and "cut.bin" not in saved()
              and not any(n.endswith(".sotyware-partial") for n in saved()), f"{saved()} {text(pid)[-300:]}")
        ok = press(pid, "Try download again") and wait_for(
            lambda: "Try download again" in buttons(pid) and "Download failed" in text(pid), 30)
        check("Try again repeats it without a dialog (and fails the same way here)",
              bool(ok) and chooser() is None and "cut.bin" not in saved(), text(pid)[-300:])
        press(pid, "Dismiss")
        entry = open_dialog("Download tampered.bin")
        ok = bool(entry) and save_as(entry, os.path.join(out, "tampered.bin"))
        failed = ok and panel_says("Download failed", "did not pass the integrity check", timeout=30)
        check("content that does not match its SHA-256 is never saved",
              bool(failed) and "tampered.bin" not in saved(), f"{saved()} {text(pid)[-300:]}")
        press(pid, "Dismiss")

        # Quitting while a download runs: in dashboard-only mode closing the
        # window quits; the temporary file must be gone afterwards.
        press(pid, BUCKET) and wait_for(lambda: "slow" in text(pid), 20)
        ok = press(pid, "slow") and wait_for(lambda: "large.bin" in text(pid), 20)
        entry = ok and open_dialog("Download large.bin")
        ok = bool(entry) and save_as(entry, os.path.join(out, "quit.bin"))
        running = ok and panel_says("% ·", timeout=20) and any(n.startswith(".quit.bin.") for n in saved())
        close_windows(pid)
        try:
            code = p.wait(15)
        except subprocess.TimeoutExpired:
            code = None
        check("closing the window during a download quits (exit 0) and removes the temporary file",
              bool(running) and code == 0 and not any(n.startswith(".quit.bin") or n == "quit.bin" for n in saved()),
              f"exit {code}, {saved()}")
        check("the saved files are exactly the completed downloads",
              saved() == ["bericht.pdf", "existing.txt", "foreign.txt", "readme copy.txt"], str(saved()))

    elif PHASE == "b2":
        label = f"sotyware e2e {RUN}"
        account = f"sotyware-e2e-{RUN}"
        check("no account with this run's id in the keyring yet", keyring_items(account)[:2] == ([], []))
        ok = add(label, BUCKET, KEY1, SECRET1, None)
        check("add a Backblaze B2 account (bucket, region, key) through the form",
              bool(ok and wait_for(lambda: f"Added {label}" in text(pid))), text(pid))
        unl, lck, vals = keyring_items(account)
        check(f"the key is in GNOME Keyring under cloud-account/{account}, exactly as typed",
              len(unl) == 1 and vals == stored(KEY1, SECRET1), f"{len(unl)} unlocked, {len(lck)} locked")
        check("the account database has the account, default endpoint, and no key",
              rows() == [(account, label, "backblaze-b2", REGION, None)] and db_has_no_key(), str(rows()))
        t = text(pid)
        check("page shows the account (Backblaze B2, bucket, region), not the key",
              BUCKET in t and REGION in t and "Backblaze B2" in t and page_has_no_key(t) and fields(pid) == [], t)
        status = run_check()
        check("Check connection: the bucket-restricted key is Connected", status == "Connected",
              f"{status}: {text(pid)}")
        # Browsing the root, read-only. The test key may list only under its
        # file name prefix (sotyware-it/), so Backblaze may refuse the root.
        ok = press(pid, "Open bucket") and wait_for(
            lambda: "Could not load this folder." in text(pid) or "Root of the bucket" in text(pid)
            and "Loading folder…" not in text(pid), 60)
        t = text(pid)
        outcome = ("access denied" if "the provider refused this request" in t
                   else "error" if "Could not load this folder." in t else "listed")
        print(f"  Open bucket at the root: {outcome}", flush=True)
        check("Open bucket at the root: a listing, or Access denied for the prefix-restricted key",
              bool(ok) and outcome in ("listed", "access denied") and page_has_no_key(t), t)
        ok = press(pid, "Back to accounts") and wait_for(lambda: "Check connection" in buttons(pid), 10)
        check("Back to accounts from the bucket", bool(ok), str(buttons(pid)))

        renamed = f"{label} renamed"
        ok = edit(pid) and fill(pid, "Name", renamed) and button(pid, "Save changes")
        check("rename without re-entering the key",
              bool(ok and wait_for(lambda: f"Saved {renamed}" in text(pid)))
              and rows()[0][1] == renamed and keyring_items(account)[2] == stored(KEY1, SECRET1), str(rows()))
        status = run_check()
        check("after the rename: still Connected", status == "Connected", f"{status}: {text(pid)}")

        check("replace the key with a wrong secret: one entry, now that key",
              replace_key(renamed, KEY2, SECRET2) and keyring_items(account)[2] == stored(KEY2, SECRET2))
        status = run_check()
        check("the replaced key is the one used: Invalid credentials", status == "Invalid credentials",
              f"{status}: {text(pid)}")
        check("replace it again with the real key: one entry, the real key",
              replace_key(renamed, KEY1, SECRET1) and keyring_items(account)[2] == stored(KEY1, SECRET1))
        status = run_check()
        check("after the replacement: Connected again", status == "Connected", f"{status}: {text(pid)}")

        # A bucket this key may not use (a new name each run): the key is
        # valid, so this must not be reported as invalid credentials.
        missing = f"sotyware-it-missing-{RUN}"
        ok = edit(pid) and fill(pid, "Bucket", missing) and button(pid, "Save changes")
        ok = ok and wait_for(lambda: f"Saved {renamed}" in text(pid) and missing in text(pid))
        status = run_check()
        check("a bucket the key may not use: Access denied or Bucket unavailable, not Invalid credentials",
              ok and status in ("Access denied", "Bucket unavailable"), f"{status}: {text(pid)}")
        print(f"  bucket the key may not use: {status}", flush=True)
        ok = edit(pid) and fill(pid, "Bucket", BUCKET) and button(pid, "Save changes")
        ok = ok and wait_for(lambda: f"Saved {renamed}" in text(pid) and missing not in text(pid))
        status = run_check()
        check("bucket set back: Connected", ok and status == "Connected", f"{status}: {text(pid)}")

        ok, confirm, removed = remove(renamed)
        check("removal asks first and says the bucket and its files stay",
              ok and "are not deleted or changed" in confirm and BUCKET in confirm, confirm)
        check("remove: account gone from page, database and keyring",
              removed and rows() == [] and keyring_items(account)[:2] == ([], []), str(rows()))

    elif PHASE == "b2-browse":
        run = MANIFEST["run"]
        label = f"sotyware e2e browse {RUN}"
        ok = add(label, BUCKET, KEY1, SECRET1, None)
        check("add a Backblaze B2 account with the read-only key",
              bool(ok and wait_for(lambda: f"Added {label}" in text(pid))), text(pid))
        status = run_check()
        check("Check connection: Connected", status == "Connected", f"{status}: {text(pid)}")
        ok = press(pid, "Open bucket") and wait_for(
            lambda: "Root of the bucket" in text(pid) and "Loading" not in text(pid), 60)
        root = table_rows(pid) if ok else []
        check("Open bucket lists the bucket's root, with the sotyware-it folder",
              ["sotyware-it", "Folder", "—"] in root, str(root)[:400])
        ok = press(pid, "sotyware-it") and wait_for(lambda: "browse-validation" in text(pid), 60)
        ok = ok and press(pid, "browse-validation") and wait_for(lambda: run in text(pid), 60)
        check("navigate sotyware-it → browse-validation → this run's folder",
              bool(ok and press(pid, run)))
        ok, why = shows("")
        check("run root: exactly its immediate folders and files, names, sizes and dates as listed", ok, why)
        for path in ["docs", "2024", "q1", "deep", "er"]:
            pressed = press(pid, path)
            rel = "/".join(["docs", "2024", "q1", "deep", "er"][:["docs", "2024", "q1", "deep", "er"].index(path) + 1]) + "/"
            ok, why = shows(rel)
            check(f"open {rel}: only its own entries", pressed and ok, why)
        ok, why = (press(pid, "2024") and shows("docs/2024/")) or (False, "breadcrumb 2024 missing")
        check("breadcrumb 2024 opens docs/2024/", ok, why)
        ok, why = (press(pid, "Up one folder") and shows("docs/")) or (False, "")
        check("Up one folder opens docs/", ok, why)
        ok, why = (press(pid, "empty-nested") and shows("docs/empty-nested/")) or (False, "")
        check("docs/empty-nested/ (marker only) opens as empty", ok, why)
        for folder, rel in [("marker-only", "marker-only/"), ("photos", "photos/"), ("special", "special/")]:
            ok = press(pid, run) and shows("")[0]
            ok2, why = (press(pid, folder) and shows(rel)) or (False, "")
            check(f"open {rel}" + (": the marker is not a file" if folder == "photos" else
                                   ": empty" if folder == "marker-only" else ": + & ? # % exactly"), ok and ok2, why)
        ok, why = (press(pid, "f+o&l?d#e%r") and shows("special/f+o&l?d#e%r/")) or (False, "")
        check("open special/f+o&l?d#e%r/", ok, why)
        uni = "ünïcødé"
        ok = press(pid, run) and shows("")[0]
        ok2, why = (press(pid, uni) and shows(uni + "/")) or (False, "")
        ok3, why3 = (press(pid, "привет") and shows(uni + "/привет/")) or (False, "")
        check("Unicode folders open and show their files exactly", ok and ok2 and ok3, why + why3)
        ok = press(pid, run) and shows("")[0]
        ok2, why = (press(pid, "dots") and shows("dots/")) or (False, "")
        ok3, why3 = (press(pid, "..") and shows("dots/../")) or (False, "")
        check("dots/ shows '..' and '.' as names; '..' opens dots/../ (escape.txt), not the run root",
              ok and ok2 and ok3 and "escape.txt" in text(pid), why + why3)
        ok, why = (press(pid, "Up one folder") and shows("dots/")) or (False, "")
        check("Up one folder from dots/../ goes to dots/", ok, why)
        ok = press(pid, run) and shows("")[0]
        ok2, why = (press(pid, "double") and shows("double/")) or (False, "")
        ok3, why3 = (press(pid, "(no name)") and shows("double//")) or (False, "")
        ok4, why4 = (press(pid, "(no name)") and shows("double///")) or (False, "")
        check("double/ → (no name) → (no name): repeated separators kept", ok and ok2 and ok3 and ok4,
              why + why3 + why4)
        ok = press(pid, run) and shows("")[0]
        ok2, why = (press(pid, "many") and shows("many/", first_page=True)) or (False, "")
        check("many/: the first 1,000 entries, with Load more",
              ok and ok2 and "1000 items shown; more are available" in text(pid) and "Load more" in buttons(pid), why)
        ok, why = (press(pid, "Load more") and shows("many/")) or (False, "")
        check("Load more: all 1,010 entries, every row as listed, no Load more after",
              ok and "1010 items shown." in text(pid) and "Load more" not in buttons(pid), why)
        ok, why = (press(pid, "Refresh") and shows("many/", first_page=True)) or (False, "")
        check("Refresh starts again at the first page", ok and "1000 items shown; more" in text(pid), why)
        ok = press(pid, "Back to accounts") and wait_for(lambda: "Open bucket" in buttons(pid), 20)
        check("Back to accounts", bool(ok), str(buttons(pid)))
        ok = press(pid, "Open bucket") and wait_for(
            lambda: "Root of the bucket" in text(pid) and "Loading" not in text(pid), 60)
        check("Open bucket again starts at the bucket's root",
              bool(ok) and ["sotyware-it", "Folder", "—"] in table_rows(pid), text(pid)[-300:])
        ok = press(pid, "Back to accounts") and wait_for(lambda: "Open bucket" in buttons(pid), 20)
        ok2, _, removed = remove(label)
        check("remove the account: gone from page, database and keyring",
              bool(ok) and removed and rows() == [], str(rows()))

    elif PHASE == "files":
        # A phase that stops early is a failure, never a pass with fewer checks.
        try:
            exec(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "cloud_ui_files.py"),
                      encoding="utf-8").read())
            check("the files phase ran to its end", True)
        # SystemExit too: a helper that gives up (type_text) must fail the
        # phase visibly, not end the driver without this line.
        except (Exception, SystemExit) as e:  # noqa: BLE001
            import traceback
            check("the files phase ran to its end", False, f"{type(e).__name__}: {e}")
            traceback.print_exc()

    elif PHASE == "usage":
        # As for files: a phase that stops early is a failure.
        try:
            exec(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "cloud_ui_usage.py"),
                      encoding="utf-8").read())
            check("the usage phase ran to its end", True)
        except (Exception, SystemExit) as e:  # noqa: BLE001
            import traceback
            check("the usage phase ran to its end", False, f"{type(e).__name__}: {e}")
            traceback.print_exc()

    elif PHASE == "upload":
        try:
            exec(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "cloud_ui_upload.py"),
                      encoding="utf-8").read())
        except RuntimeError as e:
            print(f"  upload phase stopped: {e}", flush=True)

    else:  # hang
        ok = add("Photos", BUCKET, KEY1, SECRET1, "http://127.0.0.1:9")
        saving = ok and wait_for(lambda: "Saving…" in buttons(pid), 5)
        check("add waits on the Secret Service (it never answers)", bool(saving), str(buttons(pid)))
        t0 = time.time()
        # The features are under Settings › Features since the redesign.
        nav = link(pid, "Settings") and wait_for(
            lambda: any(n == "Features" for n, _ in by_role(pid, Atspi.Role.PAGE_TAB)), 5)
        tab = [node for n, node in by_role(pid, Atspi.Role.PAGE_TAB) if n == "Features"] if nav else []
        nav = nav and tab and Atspi.Action.do_action(tab[0], 0) and wait_for(
            lambda: any(n == "Scheduled backups" for n, _ in by_role(pid, Atspi.Role.CHECK_BOX)), 5)
        check("while it waits, the window still switches sections", bool(nav) and time.time() - t0 < 5,
              f"{time.time() - t0:.1f}s")
        box = [node for n, node in by_role(pid, Atspi.Role.CHECK_BOX) if n == "Scheduled backups"]
        toggled = box and Atspi.Action.do_action(box[0], 0) and wait_for(lambda: "Disabled" in text(pid), 5)
        check("while it waits, other commands still run (Scheduled backups disabled)", bool(toggled), text(pid))
        back = link(pid, "Cloud Storage") and wait_for(lambda: "Cloud Storage" in text(pid), 5)
        check("back to Cloud Storage while the add waits", bool(back))
        check("process still running", p.poll() is None)

    logtext = open(log, errors="replace").read()
    check("no key id or secret in sotyware's output",
          all(s not in logtext for s in (SECRET1, SECRET2, KEY1, KEY2) if s), "(output withheld)")
    print("  sotyware output:", redact(logtext.strip()).replace("\n", " | ")[:600], flush=True)
finally:
    p.kill()
    p.wait()
print(f"{PHASE}: {sum(1 for r in results if r)} passed, {sum(1 for r in results if not r)} failed", flush=True)
sys.exit(0 if results and all(results) else 1)
