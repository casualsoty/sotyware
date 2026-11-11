"""Cloud Storage end-to-end test: drives the Cloud Storage page of the real
release binary through AT-SPI (finding and pressing controls) and XTest
(typing: this WebKitGTK exposes no AT-SPI EditableText).

usage: cloud_ui.py local|hang|b2|browse <binary>   (run by cloud-ui.sh, which sets up
Xvfb + openbox, a private D-Bus session and the Secret Service)

Phases:
  local  a real GNOME Keyring; made-up keys and an endpoint nobody listens on;
         contacts no provider
  hang   a Secret Service that never answers; the window must stay usable
  browse a real GNOME Keyring and the in-process listing server of the Rust
         tests (endpoint in SOTYWARE_UI_ENDPOINT, started by cloud-ui.sh):
         open the bucket, folders, breadcrumbs, Up, Load more, Refresh, Back
  b2     a real GNOME Keyring and the real Backblaze B2 test bucket, with the
         key from SOTYWARE_CLOUD_IT_B2_ACCESS_KEY_ID / _SECRET_ACCESS_KEY and
         SOTYWARE_IT_B2_BUCKET / _REGION (the opt-in integration test's
         variables). Never prints them.
"""
import ctypes
import ctypes.util
import os
import sqlite3
import subprocess
import sys
import tempfile
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


def focus_window(pid):
    for w in windows_of(pid):
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
            return Atspi.Action.do_action(node, 0)
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
    unlocked, locked = svc.SearchItems({"service": "com.sotyware.desktop", "username": f"cloud-account/{account_id}"})
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



# ---- the app, with its own data folder and none of the test variables ----
data = tempfile.mkdtemp(prefix=f"data-{PHASE}-", dir=os.environ["HOME"])
DB = os.path.join(data, "com.sotyware.desktop", "cloud-accounts.sqlite3")
log = os.path.join(os.environ["HOME"], f"sotyware-{PHASE}.log")
env = {k: v for k, v in os.environ.items() if not k.startswith(("SOTYWARE_IT_", "SOTYWARE_CLOUD_IT_"))}
env["XDG_DATA_HOME"] = data
with open(log, "wb") as f:
    p = subprocess.Popen([BIN], env=env, stdout=f, stderr=subprocess.STDOUT)
pid = p.pid


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


def run_check(timeout=60):
    """Presses Check connection on the only account; the status label shown
    afterwards, or None."""
    if not button(pid, "Check connection"):
        return None

    def done():
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


try:
    ready = wait_for(lambda: "Welcome" in text(pid), 30)
    check("dashboard is up (dashboard-only mode, no tray host here)", bool(ready))
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
        ok = press(pid, "← Back to accounts") and wait_for(lambda: "Check connection" in buttons(pid), 10)
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
        status = run_check(40)
        check("locked keyring: check says Credential store locked", ok and status == "Credential store locked",
              f"{status}: {text(pid)}")
        rem = button(pid, "Remove") and wait_for(lambda: "Remove account" in buttons(pid)) \
            and button(pid, "Remove account")
        outcome = wait_for(lambda: ("Removed Second" in text(pid) and "removed")
                           or ("still listed" in text(pid) and "kept"), 30)
        print(f"  locked keyring, remove: {outcome}; db: {rows()}", flush=True)
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
        ok = press(pid, "← Back to accounts") and wait_for(lambda: "Open bucket" in buttons(pid), 20)
        check("Back to accounts shows the account list again", bool(ok), str(buttons(pid)))
        ok, confirm, removed = remove("Browse")
        check("remove the account afterwards", removed and rows() == [], str(rows()))

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
        ok = press(pid, "← Back to accounts") and wait_for(lambda: "Check connection" in buttons(pid), 10)
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

    else:  # hang
        ok = add("Photos", BUCKET, KEY1, SECRET1, "http://127.0.0.1:9")
        saving = ok and wait_for(lambda: "Saving…" in buttons(pid), 5)
        check("add waits on the Secret Service (it never answers)", bool(saving), str(buttons(pid)))
        t0 = time.time()
        nav = link(pid, "Overview") and wait_for(lambda: "Welcome" in text(pid) and "Features" in text(pid), 5)
        check("while it waits, the window still switches sections", bool(nav) and time.time() - t0 < 5,
              f"{time.time() - t0:.1f}s")
        box = [node for n, node in by_role(pid, Atspi.Role.CHECK_BOX) if n == "Example"]
        toggled = box and Atspi.Action.do_action(box[0], 0) and wait_for(lambda: "Disabled" in text(pid), 5)
        check("while it waits, other commands still run (example feature disabled)", bool(toggled), text(pid))
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
