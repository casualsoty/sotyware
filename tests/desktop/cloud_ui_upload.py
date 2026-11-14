# The upload phase of cloud_ui.py, run by it with exec() in its own namespace
# (pid, env, check, press, text, nodes, wait_for, type_text, ctrl, key, keysym,
# focus_window, add, remove, rows, CONTROL, SEED, BUCKET, ...). Kept in its own
# file only for length.
#
# Drives upload batches of the real release binary through the native GTK
# multi-file dialog (location bar, Ctrl+A in the file list, Open) against the
# in-process s3s-fs server of serve_uploads_for_ui (cloud-ui.sh, phase
# `upload`), behind a link that takes 6 MiB/s towards the server. After each
# batch it reads the server's state (every object's content, size and
# SHA-256 metadata through HeadObject, and the multipart files s3s-fs keeps)
# through the helper's `dump` command. At the end it deletes exactly the
# objects it knows about (`delete <key>`) and checks the bucket is empty.
import hashlib
import shutil

MIB = 1024 * 1024
SRC = os.path.join(os.environ["HOME"], "upload-src")
OUT = os.path.join(os.environ["HOME"], "upload-downloads")
os.makedirs(OUT, exist_ok=True)
EMPTY_SHA = hashlib.sha256(b"").hexdigest()
created = set()  # every key an upload of this phase may have created


def sha(data):
    return hashlib.sha256(data).hexdigest()


def sha_file(path):
    with open(path, "rb") as fh:
        return sha(fh.read())


def make(batch, name, data):
    """Writes a local test file; `name` may be bytes (a non-UTF-8 name)."""
    folder = os.path.join(SRC, batch)
    os.makedirs(folder, exist_ok=True)
    path = (os.fsencode(folder) + b"/" + name) if isinstance(name, bytes) else os.path.join(folder, name)
    with open(path, "wb") as fh:
        fh.write(data)
    return path


def blob(size, seed):
    # Not compressible and different per file; fast enough for 72 MiB.
    out = bytearray()
    block = hashlib.sha256(bytes([seed])).digest()
    while len(out) < size:
        block = hashlib.sha256(block).digest()
        out += block * 64
    return bytes(out[:size])


# ---- the helper's control file ----
def control(*commands):
    tmp = CONTROL + ".ctl.tmp"
    with open(tmp, "w", encoding="utf-8") as fh:
        fh.write("\n".join(commands) + "\n")
    os.rename(tmp, CONTROL + ".ctl")
    return bool(wait_for(lambda: not os.path.exists(CONTROL + ".ctl"), 30))


def state():
    """{key: object} and the open multipart files, as the server has them."""
    path = CONTROL + ".state.json"
    if os.path.exists(path):
        os.remove(path)
    if not control("dump") or not wait_for(lambda: os.path.exists(path), 10):
        raise SystemExit("the upload server did not answer dump")
    with open(path, encoding="utf-8") as fh:
        s = json.load(fh)
    os.remove(path)
    return {o["key"]: o for o in s["objects"]}, s["multipart"], s["ops"]


def stored_ok(obj, data):
    """The object holds exactly `data`, HEAD says so, and its SHA-256
    metadata is the one sotyware writes."""
    return (obj is not None and obj["size"] == len(data) and obj["sha256"] == sha(data)
            and obj["headSize"] == len(data) and obj["sha256Metadata"] == sha(data))


def under(objects, prefix):
    return sorted(k for k in objects if k.startswith(prefix))


# ---- the page ----
def find(node, role, name=None):
    out = []

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


def panel():
    """The upload panel's text, or ""."""
    for role, name, _, node in nodes(pid):
        if name == "Upload" and role not in (Atspi.Role.PUSH_BUTTON, Atspi.Role.LABEL):
            return cell_text(node)
    return ""


def file_rows():
    """The panel's per-file rows, each as one text."""
    # Matched by name only: how WebKitGTK maps an <ol> with aria-label
    # differs between versions.
    for role, name, _, node in nodes(pid):
        if name == "Files in this upload" and role != Atspi.Role.LIST_ITEM:
            items = find(node, Atspi.Role.LIST_ITEM)
            if not items:
                items = [node.get_child_at_index(j) for j in range(node.get_child_count())]
            return [cell_text(i) for i in items if i is not None]
    return []


def row(name):
    return next((r for r in file_rows() if r.startswith(name)), "")


seen = []  # panel texts sampled while waiting


def until(pred, timeout=60):
    """wait_for, keeping every panel text seen meanwhile."""
    def sample():
        t = panel()
        if not seen or seen[-1] != t:
            seen.append(t)
        return t if pred(t) else None
    return wait_for(sample, timeout)


def finished(t):
    return any(s in t for s in ("Upload finished.", "Upload cancelled.", "Upload stopped after an error."))


# ---- the native GTK dialogs ----
def chooser(title):
    for role, name, _, node in nodes(pid):
        if role == Atspi.Role.FILE_CHOOSER and name == title:
            return node
    return None


def give_up(dlg):
    """Closes the dialog and stops the phase: every later step would only
    fail behind an open modal dialog."""
    for b in find(dlg, Atspi.Role.PUSH_BUTTON, "Cancel"):
        Atspi.Action.do_action(b, 0)
    check("the native dialog could be driven (phase stopped here)", False)
    raise RuntimeError("could not drive the file dialog")


def pick(folder, expect_rows, before_choosing=None):
    """Presses Upload files…, goes to `folder` through the dialog's location
    bar, selects every file with Ctrl+A in the file list and presses Open.
    `before_choosing` runs while the dialog is open, before Open."""
    if not press(pid, "Upload files…"):
        return False
    dlg = wait_for(lambda: chooser("Choose files to upload"), 20)
    if dlg is None:
        print("  no file dialog", flush=True)
        return False
    focus_window(pid)
    table = find(dlg, Atspi.Role.TABLE, "Files")[0]
    Atspi.Component.grab_focus(table)
    wait_for(lambda: table.get_state_set().contains(Atspi.StateType.FOCUSED), 3)
    ctrl("l")
    entry = wait_for(lambda: next((e for e in find(dlg, Atspi.Role.TEXT)
                                   if e.get_state_set().contains(Atspi.StateType.SHOWING)
                                   and e.get_state_set().contains(Atspi.StateType.EDITABLE)), None), 5)
    if entry is None:
        print("  no location bar", flush=True)
        give_up(dlg)
    type_text(folder + "/")
    # In a folder with one file, GTK completes its name inline (selected
    # text after the cursor); Delete removes that completion.
    if wait_for(lambda: Atspi.Text.get_text(entry, 0, -1) not in ("", folder + "/") , 1.5):
        key(keysym("Delete"))
    if not wait_for(lambda: Atspi.Text.get_text(entry, 0, -1) == folder + "/", 5):
        print(f"  location bar: {Atspi.Text.get_text(entry, 0, -1)!r}", flush=True)
        give_up(dlg)
    # GTK's location bar completes asynchronously; a Return pressed before
    # its completion settles is swallowed.
    time.sleep(0.7)
    key(keysym("Return"))

    def names():
        # The dialog rebuilds its file list when it changes folder.
        nonlocal table
        table = find(dlg, Atspi.Role.TABLE, "Files")[0]
        return [Atspi.Text.get_text(c, 0, -1) for c in find(table, Atspi.Role.TABLE_CELL)
                if c.get_text_iface() is not None]
    want = set(expect_rows)
    if not wait_for(lambda: want <= set(names()), 5):
        print("  the dialog did not change folder on Return; pressing it once more", flush=True)
        Atspi.Component.grab_focus(entry)
        time.sleep(0.7)
        key(keysym("Return"))
    if not wait_for(lambda: want <= set(names()), 10):
        cells = [Atspi.Text.get_text(c, 0, -1) for c in find(dlg, Atspi.Role.TABLE_CELL) if c.get_text_iface()]
        print(f"  the Files table shows {names()}; all cells {cells[:30]}; "
              f"location bar {Atspi.Text.get_text(entry, 0, -1)!r}", flush=True)
        give_up(dlg)
    Atspi.Component.grab_focus(table)
    wait_for(lambda: table.get_state_set().contains(Atspi.StateType.FOCUSED), 3)
    ctrl("a")
    if not wait_for(lambda: Atspi.Selection.get_n_selected_children(table) >= len(expect_rows), 5):
        print(f"  selected {Atspi.Selection.get_n_selected_children(table)} of {len(expect_rows)}", flush=True)
        give_up(dlg)
    if before_choosing is not None:
        # After the keyboard work: pressing something on the page moves the
        # X input focus away from the dialog. Open is pressed through AT-SPI.
        before_choosing()
    open_button = find(dlg, Atspi.Role.PUSH_BUTTON, "Open")[0]
    Atspi.Action.do_action(open_button, 0)
    return bool(wait_for(lambda: chooser("Choose files to upload") is None, 10))


def save_dialog():
    for role, name, _, node in nodes(pid):
        if role == Atspi.Role.FILE_CHOOSER or (role == Atspi.Role.DIALOG and name == "Save download as"):
            found = [n for n in find(node, Atspi.Role.TEXT)
                     if n.get_state_set().contains(Atspi.StateType.EDITABLE)
                     and n.get_state_set().contains(Atspi.StateType.SHOWING)]
            if found:
                return found[0]
    return None


def save_as(entry, path):
    Atspi.Component.grab_focus(entry)
    wait_for(lambda: entry.get_state_set().contains(Atspi.StateType.FOCUSED), 3)
    ctrl("a")
    type_text(path)
    if not wait_for(lambda: Atspi.Text.get_text(entry, 0, -1) == path, 5):
        return False
    key(keysym("Return"))
    return True


def go(folder, marker):
    """Opens a folder of the bucket's root through its folder button."""
    if "Root of the bucket" not in text(pid):
        press(pid, BUCKET)
    return wait_for(lambda: "Root of the bucket" in text(pid), 20) \
        and press(pid, folder) and wait_for(lambda: marker in text(pid), 20)


def sending(name, at_least=0):
    def pred(t):
        r = row(name)
        if "Sending · " not in r:
            return False
        try:
            return int(r.split("Sending · ")[1].split("%")[0]) >= at_least
        except ValueError:
            return False
    return pred


def dismiss():
    press(pid, "Dismiss")
    wait_for(lambda: panel() == "", 5)


# ---- X: is the dashboard mapped? ----
# The structure (and XGetWindowAttributes' argument types) come from cloud_ui.py.
XWindowAttributes = _WinAttrs


def dashboard_viewable():
    """Whether one of the process's large windows is viewable (map state 2)."""
    for w in windows_of(pid):
        a = XWindowAttributes()
        if X.XGetWindowAttributes(dpy, w, ctypes.byref(a)) and a.width > 100 and a.map_state == 2:
            return True
    return False


# ---- the tray menu of the stub watcher's registered item ----
def tray_quit():
    bus = dbus.SessionBus()
    watcher = bus.get_object("org.kde.StatusNotifierWatcher", "/StatusNotifierWatcher")
    items = dbus.Interface(watcher, dbus.PROPERTIES_IFACE).Get("org.kde.StatusNotifierWatcher",
                                                               "RegisteredStatusNotifierItems")
    sender, path = str(items[-1]).split(" ", 1)
    props = dbus.Interface(bus.get_object(sender, path), dbus.PROPERTIES_IFACE).GetAll("org.kde.StatusNotifierItem")
    menu = dbus.Interface(bus.get_object(sender, props["Menu"]), "com.canonical.dbusmenu")
    _, layout = menu.GetLayout(0, -1, dbus.Array([], signature="s"))
    labels = {str(c[1]["label"]).replace("_", ""): int(c[0]) for c in layout[2] if "label" in c[1]}
    menu.Event(dbus.Int32(labels["Quit"]), "clicked", dbus.Int32(0, variant_level=1), dbus.UInt32(0),
               ignore_reply=True)


# ---- the local files ----
UNI = "Grüße Ünïcödé 日本.txt"
SPECIAL = "report 2024 (final) + notes & more #1.txt"
files = {}
files[UNI] = make("batch1", UNI, "Unicode name".encode())
files["empty.txt"] = make("batch1", "empty.txt", b"")
files[SPECIAL] = make("batch1", SPECIAL, b"spaces, parentheses, plus, ampersand and hash")
files["existing.txt"] = make("batch1", "existing.txt", b"mine, not the bucket's")
files["medium.bin"] = make("batch1", "medium.bin", blob(12 * MIB, 1))
make("batch1", b"ctrl\x01char.txt", b"control character")
make("batch1", b"latin1-\xe9.txt", b"not UTF-8")
os.symlink("/dev/null", os.path.join(SRC, "batch1", "device-link"))
os.symlink(os.path.join(SRC, "batch1", "gone"), os.path.join(SRC, "batch1", "broken-link"))
B1_ROWS = [UNI, "broken-link", "ctrl\x01char.txt", "device-link", "empty.txt", "existing.txt",
           "latin1-�.txt (invalid encoding)", "medium.bin", SPECIAL]

ok = add("Uploads", BUCKET, KEY1, SECRET1, ENDPOINT)
check("add an account for the local upload server",
      bool(ok and wait_for(lambda: "Added Uploads" in text(pid))), text(pid))
listening = subprocess.run(["ss", "-ltn"], capture_output=True, text=True).stdout
check("the page is the bundled frontend: nothing listens on the dev server port 1420",
      ":1420 " not in listening, listening)
ok = link(pid, "Overview") and wait_for(lambda: "Hide dashboard" in buttons(pid), 10)
check("tray mode (stub watcher): Overview has the Hide dashboard button", bool(ok), str(buttons(pid)))
ok = link(pid, "Cloud Storage") and wait_for(lambda: "Open bucket" in buttons(pid), 10)
check("back to Cloud Storage", bool(ok), str(buttons(pid)))
ok = press(pid, "Open bucket") and wait_for(lambda: "inbox" in text(pid) and "downloads" in text(pid), 30)
check("Open bucket lists the seeded folders", bool(ok), text(pid))
objects, multipart, _ = state()
check("before any upload the server holds exactly the three seeded objects, sotyware's SHA-256 with each",
      sorted(objects) == sorted(SEED) and all(objects[k]["sha256Metadata"] == SEED[k]["sha256"] for k in SEED)
      and multipart == [], f"{sorted(objects)} {multipart}")

# 1. Validation, odd names, an existing object, sequential uploads, progress,
#    and the destination fixed when the dialog opened.
ok = go("inbox", "existing.txt")
check("open inbox/", bool(ok), text(pid))
nav = []


def navigate_while_choosing():
    # The dialog is modal for a person; AT-SPI can still press the page's
    # bucket breadcrumb, which changes the folder the browser shows.
    nav.append(bool(press(pid, BUCKET) and wait_for(lambda: "Root of the bucket" in text(pid), 10)))


seen.clear()
ok = pick(os.path.join(SRC, "batch1"), B1_ROWS, navigate_while_choosing)
check("Upload files… opens the native GTK multi-file dialog; 9 entries chosen there (Ctrl+A, Open)", ok)
check("while the dialog was open the browser moved to the bucket root", nav == [True], str(nav))
t = until(lambda t: "Upload to" in t, 15) or ""
check("the panel names the folder captured when the dialog opened, not the one shown now",
      "Upload to upload-bucket / inbox/" in t and "Root of the bucket" in text(pid), t)
mid = until(sending("medium.bin"), 30)
objects_mid, _, _ = state()
check("while medium.bin is being sent, the server has no object inbox/medium.bin yet",
      bool(mid) and "inbox/medium.bin" not in objects_mid, f"{mid} {sorted(objects_mid)}")
done = until(finished, 90)
rows_ = file_rows()
print("  file rows:", rows_, flush=True)
check("final summary: 8 files: 4 uploaded, 4 skipped (the non-UTF-8 file never reached sotyware)",
      bool(done) and "Upload finished. 8 files: 4 uploaded, 4 skipped." in done, done)
check("8 rows, in the dialog's order; the non-UTF-8 name is not among them",
      len(rows_) == 8 and not any("latin1" in r for r in rows_), str(rows_))
check("the empty file, the Unicode name and the special characters: Uploaded and verified",
      all("Uploaded and verified" in row(n) for n in ("empty.txt", UNI, SPECIAL, "medium.bin")), str(rows_))
check("skip reasons from Rust: not a file, could not be read, control characters, already in this folder",
      "Skipped: not a file" in row("device-link") and "Skipped: could not be read" in row("broken-link")
      and "Skipped: name contains control characters" in row("ctrl")
      and "Skipped: already in this folder" in row("existing.txt"), str(rows_))
stages = [s for s in ("Computing the SHA-256 checksum", "Sending · ", "Verifying the stored object", "Starting…")
          if any(s in x for x in seen)]
print(f"  stages seen in the panel ({len(seen)} samples): {stages}", flush=True)
check("progress: medium.bin was seen Sending with a percentage", "Sending · " in stages, str(stages))
objects, multipart, _ = state()
created |= {"inbox/empty.txt", f"inbox/{UNI}", f"inbox/{SPECIAL}", "inbox/medium.bin"}
check("server: inbox/ holds exactly existing.txt and the four uploaded files under their exact keys",
      under(objects, "inbox/") == sorted({"inbox/existing.txt", "inbox/empty.txt", f"inbox/{UNI}",
                                          f"inbox/{SPECIAL}", "inbox/medium.bin"}), str(under(objects, "inbox/")))
check("server: contents, sizes and SHA-256 metadata equal the local files (empty file: 0 bytes, e3b0…)",
      stored_ok(objects.get("inbox/empty.txt"), b"") and objects["inbox/empty.txt"]["sha256"] == EMPTY_SHA
      and all(stored_ok(objects.get(f"inbox/{n}"), open(files[n], "rb").read()) for n in (UNI, SPECIAL, "medium.bin")),
      json.dumps({k: objects.get(k) for k in created}, ensure_ascii=False)[:800])
check("server: the existing object was left as it was; nothing went to the root",
      objects["inbox/existing.txt"]["sha256"] == SEED["inbox/existing.txt"]["sha256"]
      and not any("/" not in k for k in objects) and multipart == [], f"{sorted(objects)} {multipart}")
dismiss()

# 2. Cancel during a file's upload; the file before it stays, the one after
#    is not started; Try again takes exactly the two eligible files.
a2 = make("batch2", "a-first.txt", b"first")
b2 = make("batch2", "b-big.bin", blob(40 * MIB, 2))
c2 = make("batch2", "c-third.txt", b"third")
ok = go("inbox", "existing.txt") and pick(os.path.join(SRC, "batch2"), ["a-first.txt", "b-big.bin", "c-third.txt"])
running = ok and until(sending("b-big.bin", 10), 60)
check("batch 2: a-first.txt done, b-big.bin being sent", bool(running) and "Uploaded and verified" in row("a-first.txt"),
      panel())
press(pid, "Cancel upload")
done = until(finished, 60)
check("Cancel during b-big.bin: Upload cancelled. 3 files: 1 uploaded, 1 cancelled, 1 not started.",
      bool(done) and "Upload cancelled. 3 files: 1 uploaded, 1 cancelled, 1 not started." in done, done)
check("rows: cancelled during its upload with nothing stored; the next not started",
      "Cancelled during its upload" in row("b-big.bin") and "Nothing was stored for this file." in row("b-big.bin")
      and "Not started: the upload was cancelled" in row("c-third.txt"), str(file_rows()))
objects, multipart, _ = state()
created |= {"inbox/a-first.txt", "inbox/b-big.bin", "inbox/c-third.txt"}
check("server after the cancel: a-first.txt stored, no b-big.bin, no c-third.txt",
      stored_ok(objects.get("inbox/a-first.txt"), b"first") and "inbox/b-big.bin" not in objects
      and "inbox/c-third.txt" not in objects and multipart == [], f"{under(objects, 'inbox/')} {multipart}")
check("Try again offers exactly the 2 eligible files", "Try 2 files again" in buttons(pid), str(buttons(pid)))
press(pid, "Try 2 files again")
done = until(lambda t: "Retrying upload" in t and finished(t), 90)
check("retry: Upload finished. 2 files: 2 uploaded.", bool(done) and "Upload finished. 2 files: 2 uploaded." in done,
      done)
objects, multipart, _ = state()
check("server after the retry: b-big.bin and c-third.txt stored and verified; a-first.txt not sent again",
      stored_ok(objects.get("inbox/b-big.bin"), open(b2, "rb").read())
      and stored_ok(objects.get("inbox/c-third.txt"), b"third") and multipart == [], str(under(objects, "inbox/")))
dismiss()

# 3. Cancel a multipart upload: aborted, no parts left.
a3 = make("batch3", "a-multi.bin", blob(72 * MIB, 3))
ok = pick(os.path.join(SRC, "batch3"), ["a-multi.bin"])
running = ok and until(sending("a-multi.bin", 25), 60)
_, multipart_mid, _ = state()
check("batch 3: a 72 MiB multipart upload in progress; s3s-fs holds its parts", bool(running) and multipart_mid != [],
      f"{panel()} {multipart_mid}")
press(pid, "Cancel upload")
done = until(finished, 60)
objects, multipart, _ = state()
created.add("inbox/a-multi.bin")
check("multipart cancelled: nothing stored, the multipart upload aborted (no parts left)",
      bool(done) and "Upload cancelled. 1 file: 0 uploaded, 1 cancelled." in done
      and "Nothing was stored for this file." in row("a-multi.bin") and "inbox/a-multi.bin" not in objects
      and multipart == [], f"{done} {multipart}")
dismiss()

# 4. The abort is refused: parts may remain; not retryable itself; the retry
#    of the other file aborts the remembered upload again.
make("batch4", "a-parts.bin", blob(72 * MIB, 4))
make("batch4", "b-next.txt", b"next")
ok = control("abort-denied") and pick(os.path.join(SRC, "batch4"), ["a-parts.bin", "b-next.txt"])
running = ok and until(sending("a-parts.bin", 25), 60)
press(pid, "Cancel upload")
done = until(finished, 60)
objects, multipart, _ = state()
created |= {"inbox/a-parts.bin", "inbox/b-next.txt"}
check("abort refused: the row says parts may remain; nothing completed; parts are still on the server",
      bool(running) and bool(done) and "may remain at the provider" in row("a-parts.bin")
      and "inbox/a-parts.bin" not in objects and multipart != [], f"{done} {row('a-parts.bin')} {multipart}")
check("Try again offers only b-next.txt (1 file), never the file with incomplete parts",
      "Try 1 file again" in buttons(pid) and "Try 2 files again" not in buttons(pid), str(buttons(pid)))
press(pid, "Try 1 file again")
done = until(lambda t: "Retrying upload" in t and finished(t), 60)
objects, multipart, _ = state()
check("retry: the remembered upload was aborted (cleanup shown) and b-next.txt uploaded; no parts left",
      bool(done) and "were removed" in done and "Upload finished. 1 file: 1 uploaded." in done
      and stored_ok(objects.get("inbox/b-next.txt"), b"next") and "inbox/a-parts.bin" not in objects
      and multipart == [], f"{done} {multipart}")
dismiss()

# 5. Verification refused after the commit point: needs inspection, no retry
#    for that file, and the batch stops.
make("batch5", "a-inspect.txt", b"inspect me")
make("batch5", "b-after.txt", b"after")
ok = control("verify-denied") and pick(os.path.join(SRC, "batch5"), ["a-inspect.txt", "b-after.txt"])
done = ok and until(finished, 60)
check("verification refused: Upload stopped after an error. 2 files: 0 uploaded, 1 failed, 1 not started.",
      bool(done) and "Upload stopped after an error. 2 files: 0 uploaded, 1 failed, 1 not started." in done, done)
check("the row says the provider may have stored it and sotyware does not retry or delete it",
      "Failed" in row("a-inspect.txt") and "may have stored this file" in row("a-inspect.txt")
      and "Not started: the upload stopped after an error" in row("b-after.txt"), str(file_rows()))
objects, multipart, _ = state()
created |= {"inbox/a-inspect.txt", "inbox/b-after.txt"}
check("server: the ambiguous object really was stored, and was not deleted",
      stored_ok(objects.get("inbox/a-inspect.txt"), b"inspect me") and "inbox/b-after.txt" not in objects,
      str(under(objects, "inbox/")))
check("no unsafe retry: Try again offers 1 file (b-after.txt), not the one that needs inspection",
      "Try 1 file again" in buttons(pid) and "Try 2 files again" not in buttons(pid), str(buttons(pid)))
press(pid, "Try 1 file again")
done = until(lambda t: "Retrying upload" in t and finished(t), 60)
objects, _, ops = state()
check("retry uploads only b-after.txt", bool(done) and "Upload finished. 1 file: 1 uploaded." in done
      and stored_ok(objects.get("inbox/b-after.txt"), b"after") and "a-inspect" not in done, done)
dismiss()

# 6. A file that changes while it is sent fails with nothing stored; its
#    retry uploads the new content.
a6 = make("batch6", "a-changing.bin", blob(30 * MIB, 6))
ok = pick(os.path.join(SRC, "batch6"), ["a-changing.bin"])
running = ok and until(sending("a-changing.bin", 20), 60)
with open(a6, "ab") as fh:
    fh.write(b"appended while sending")
done = until(finished, 60)
objects, multipart, _ = state()
created.add("inbox/a-changing.bin")
check("changed while sent: Failed, nothing stored, Try 1 file again",
      bool(running) and bool(done) and "Failed" in row("a-changing.bin")
      and "Nothing was stored for this file." in row("a-changing.bin") and "Try 1 file again" in buttons(pid)
      and "inbox/a-changing.bin" not in objects, f"{done} {buttons(pid)}")
press(pid, "Try 1 file again")
done = until(lambda t: "Retrying upload" in t and finished(t), 90)
objects, _, _ = state()
check("retry uploads the file as it is now (new size and SHA-256)",
      bool(done) and "Upload finished. 1 file: 1 uploaded." in done
      and stored_ok(objects.get("inbox/a-changing.bin"), open(a6, "rb").read()), done)
dismiss()

# 7. A download beside an upload.
make("batch7", "a-alongside.bin", blob(40 * MIB, 7))
ok = go("downloads", "big.bin") and pick(os.path.join(SRC, "batch7"), ["a-alongside.bin"])
running = ok and until(sending("a-alongside.bin", 5), 60)
entry = running and press(pid, "Download big.bin") and wait_for(save_dialog, 20)
target = os.path.join(OUT, "big copy.bin")
ok = bool(entry) and save_as(entry, target)
downloaded = ok and wait_for(lambda: "Downloaded" in text(pid) and "matched" in text(pid), 60)
during = panel()
check("while the upload is being sent, a download through the save dialog completes and is verified",
      bool(downloaded) and "Sending · " in during and "Cancel upload" in buttons(pid)
      and sha_file(target) == SEED["downloads/big.bin"]["sha256"], f"{during} | {text(pid)[-300:]}")
done = until(finished, 90)
objects, _, _ = state()
created.add("downloads/a-alongside.bin")
check("the upload beside it finishes too", bool(done) and "Upload finished. 1 file: 1 uploaded." in done
      and stored_ok(objects.get("downloads/a-alongside.bin"), open(os.path.join(SRC, "batch7", "a-alongside.bin"),
                                                                  "rb").read()), done)
press(pid, "Dismiss")
dismiss()

# 8. Leaving the browser during a batch cancels it.
make("batch8", "a-away.bin", blob(40 * MIB, 8))
make("batch8", "b-away.txt", b"away")
ok = go("inbox", "existing.txt") and pick(os.path.join(SRC, "batch8"), ["a-away.bin", "b-away.txt"])
running = ok and until(sending("a-away.bin", 10), 60)
left = running and press(pid, "← Back to accounts") and wait_for(lambda: "Open bucket" in buttons(pid), 10)
time.sleep(10)  # longer than the rest of a-away.bin would take
objects, multipart, _ = state()
created |= {"inbox/a-away.bin", "inbox/b-away.txt"}
check("Back to accounts during the batch: neither file stored afterwards (the batch was cancelled)",
      bool(left) and "inbox/a-away.bin" not in objects and "inbox/b-away.txt" not in objects and multipart == [],
      str(under(objects, "inbox/")))
ok = press(pid, "Open bucket") and wait_for(lambda: "inbox" in text(pid), 30)
upload_button = [n for n, node in by_role(pid, Atspi.Role.PUSH_BUTTON) if n == "Upload files…"
                 and node.get_state_set().contains(Atspi.StateType.ENABLED)]
check("back in the browser: no panel, and Upload files… is enabled (the slot is free)",
      bool(ok) and panel() == "" and bool(upload_button), text(pid)[-300:])

# 9. Hiding and showing the dashboard during a batch. The Hide dashboard
#    button is on Overview, and switching sections leaves the browser (which
#    cancels); the window's close (✕), which tray mode turns into hide, keeps
#    the page as it is.
make("batch9", "a-hidden.bin", blob(40 * MIB, 9))
ok = go("inbox", "existing.txt") and pick(os.path.join(SRC, "batch9"), ["a-hidden.bin"])
running = ok and until(sending("a-hidden.bin", 5), 60)
hidden = False
if running:
    close_windows(pid)
    hidden = wait_for(lambda: not dashboard_viewable(), 10)
check("close (✕) during the upload hides the window in tray mode; the process keeps running",
      bool(hidden) and p.poll() is None)
time.sleep(3)
second = subprocess.run([BIN], env=env, capture_output=True, timeout=30)
shown = wait_for(dashboard_viewable, 10)
t = panel()
check("a second launch shows the dashboard again (exit 0); the panel still shows the same batch",
      second.returncode == 0 and bool(shown) and ("a-hidden.bin" in t), f"exit {second.returncode} {t}")
done = until(finished, 90)
objects, _, _ = state()
created.add("inbox/a-hidden.bin")
check("the batch finished while hidden/shown: uploaded and verified",
      bool(done) and "Upload finished. 1 file: 1 uploaded." in done
      and stored_ok(objects.get("inbox/a-hidden.bin"), open(os.path.join(SRC, "batch9", "a-hidden.bin"), "rb").read()),
      done)
dismiss()

# 10. Quitting from the tray during a multipart upload.
make("batch10", "a-quit.bin", blob(72 * MIB, 10))
ok = pick(os.path.join(SRC, "batch10"), ["a-quit.bin"])
running = ok and until(sending("a-quit.bin", 25), 60)
_, multipart_mid, _ = state()
t0 = time.time()
tray_quit()
try:
    code = p.wait(45)
except subprocess.TimeoutExpired:
    code = None
objects, multipart, _ = state()
created.add("inbox/a-quit.bin")
print(f"  quit took {time.time() - t0:.1f} s", flush=True)
check("tray Quit during a multipart upload: exit 0; no object, its parts aborted",
      bool(running) and multipart_mid != [] and code == 0 and "inbox/a-quit.bin" not in objects and multipart == [],
      f"exit {code}, {multipart_mid} -> {multipart}")

# Cleanup: exactly the keys this phase may have created, then the seed.
objects, multipart, ops = state()
unknown = sorted(set(objects) - created - set(SEED))
check("the server holds no object this phase does not know about", unknown == [], str(unknown))
print("  test prefixes: inbox/, downloads/ of bucket upload-bucket", flush=True)
print("  keys created by uploads:", sorted(k for k in created if k in objects), flush=True)
control(*[f"delete {k}" for k in sorted(set(objects) & (created | set(SEED)))])
objects, multipart, _ = state()
check("cleanup by exact key: the bucket is empty, no multipart upload left", objects == {} and multipart == [],
      f"{sorted(objects)} {multipart}")
shutil.rmtree(SRC, ignore_errors=True)
