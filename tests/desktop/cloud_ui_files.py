# The file-management phase of cloud_ui.py (ADR-026), run by it with exec()
# in its own namespace (pid, env, check, press, text, nodes, wait_for,
# type_text, ctrl, key, keysym, focus_window, add, table_rows, cell_text,
# CONTROL, PROFILE, BUCKET, ...). Kept in its own file only for length.
#
# Drives the file manager of the release binary (a test-profile build,
# ADR-027) against the in-memory S3 server of serve_files_for_ui
# (src-tauri/src/cloud/test_server/files_ui.rs; cloud-ui.sh phase `files`):
# a versioned bucket and an unversioned one, on 127.0.0.1, made-up key.
# After each step it reads the server's state through the helper's `dump`
# and checks what is really stored (current objects and every version),
# not only what the page says. The helper itself fails at the end if any
# real version of the versioned bucket was deleted for good.
#
# What it cannot drive here (reported as not tested, never as passed): a
# drag and drop from another program (no drag source on this virtual
# display) and the system clipboard (no clipboard manager).
import hashlib
import re
import urllib.request

XT.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
XT.XTestFakeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]
OUT = os.path.join(os.environ["HOME"], "files-downloads")
SRC = os.path.join(os.environ["HOME"], "files-upload-src")
os.makedirs(OUT, exist_ok=True)
not_tested = []


# ---- the server ----
def control(*commands):
    tmp = CONTROL + ".ctl.tmp"
    with open(tmp, "w", encoding="utf-8") as fh:
        fh.write("\n".join(commands) + "\n")
    os.rename(tmp, CONTROL + ".ctl")
    return bool(wait_for(lambda: not os.path.exists(CONTROL + ".ctl"), 30))


def state(bucket=None):
    """{key: {"current": {...} | None, "versions": [[id, marker], ...]}}."""
    path = CONTROL + ".state.json"
    if os.path.exists(path):
        os.remove(path)
    if not control("dump") or not wait_for(lambda: os.path.exists(path), 10):
        raise SystemExit("the files server did not answer dump")
    with open(path, encoding="utf-8") as fh:
        s = json.load(fh)
    os.remove(path)
    return s["buckets"][bucket or BUCKET]["keys"], s


def current(keys):
    return sorted(k for k, v in keys.items() if v["current"] is not None)


def under(keys, prefix):
    return [k for k in current(keys) if k.startswith(prefix)]


def content(keys, k):
    c = keys.get(k, {}).get("current")
    return None if c is None else c.get("text")


# ---- the page ----
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


def dialog():
    """The open modal dialog of the page (role dialog), or None."""
    for role, name, _, node in nodes(pid):
        if role in (Atspi.Role.DIALOG, Atspi.Role.ALERT) and node.get_child_count() > 0 \
                and role != Atspi.Role.FILE_CHOOSER:
            return node
    return None


def dialog_text():
    d = dialog()
    return cell_text(d) if d is not None else ""


def press_in(node, name):
    for b in find(node, Atspi.Role.PUSH_BUTTON, name):
        if b.get_state_set().contains(Atspi.StateType.ENABLED):
            return Atspi.Action.do_action(b, 0)
    return False


def press_dialog(name, timeout=10):
    return bool(wait_for(lambda: dialog() is not None and press_in(dialog(), name), timeout))


def press_prefix(node, prefix):
    """Presses the first enabled button in `node` whose name starts with
    `prefix` (the picker's "Move to “made”")."""
    for b in find(node, Atspi.Role.PUSH_BUTTON):
        if b.get_name().startswith(prefix) and b.get_state_set().contains(Atspi.StateType.ENABLED):
            return Atspi.Action.do_action(b, 0)
    return False


def picker():
    for role, name, _, node in nodes(pid):
        if role == Atspi.Role.DIALOG and re.search(r" to…$", name):
            return node
    return None


def pick_destination(path, confirm):
    """In the destination picker: from the bucket's root, opens each folder
    of `path` ("" for the root) and presses `confirm` ("Copy to …")."""
    d = wait_for(picker, 10)
    if d is None:
        print("  no destination picker", flush=True)
        return False
    press_in(d, BUCKET)
    time.sleep(0.6)
    for seg in [x for x in path.strip("/").split("/") if x]:
        if not wait_for(lambda: press_in(picker(), seg), 10):
            print(f"  picker: no folder {seg!r}", flush=True)
            return False
        time.sleep(0.6)
    segs = [x for x in path.strip("/").split("/") if x]
    # The confirm button names the folder shown: "Move to “made”".
    label = f"{confirm} “{segs[-1]}”" if segs else confirm
    if not wait_for(lambda: press_prefix(picker(), label), 10):
        print(f"  picker: no button {label!r}: {[b.get_name() for b in find(picker(), Atspi.Role.PUSH_BUTTON)]}", flush=True)
        return False
    return True


def chain(label, *steps):
    """Runs the steps in order; on the first that fails says which."""
    for i, step in enumerate(steps, 1):
        if not step():
            print(f"  {label}: step {i} of {len(steps)} failed", flush=True)
            return False
    return True


def fill_field(label, value):
    """Types into the field labelled `label`. A field inside its <label>
    is named by the label and its current value ("New name one.txt")."""
    for n in fields(pid):
        if n == label or n.startswith(label + " "):
            return fill(pid, n, value)
    print(f"  no field {label!r}; fields: {fields(pid)}", flush=True)
    return False


def recover():
    """Closes what a failed step left open (a dialog, the operation panel)
    so the next section starts from the folder view."""
    for _ in range(3):
        d = dialog()
        if d is None:
            break
        if not (press_in(d, "Cancel") or press_in(d, "Close")):
            keystroke("Escape")
        wait_for(lambda: dialog() is None, 3)
    r = op_region()
    if r is not None:
        press_in(r, "Dismiss")


def press_toggle(name):
    """A button with aria-pressed is a toggle button to AT-SPI."""
    return act(pid, (Atspi.Role.TOGGLE_BUTTON, Atspi.Role.PUSH_BUTTON), name)


def checkbox_node(name):
    for n, node in by_role(pid, Atspi.Role.CHECK_BOX):
        if n == name:
            return node
    return None


def is_checked(node):
    try:
        return node.get_state_set().contains(Atspi.StateType.CHECKED)
    except GLib.Error:
        return None


def toggle(name, want=True):
    """Sets the checkbox `name` ("Select x.txt") to `want` (checked by
    default), whatever it was; waits until the page shows it. The list may
    be re-rendering after a navigation, so it looks the node up again."""
    for _ in range(3):
        node = wait_for(lambda: checkbox_node(name), 10)
        if node is None:
            print(f"  no checkbox {name!r}", flush=True)
            return False
        if is_checked(node) == want:
            return True
        try:
            Atspi.Action.do_action(node, 0)
        except GLib.Error:
            pass
        if wait_for(lambda: (lambda n: n is not None and is_checked(n) == want)(checkbox_node(name)), 3):
            return True
    print(f"  checkbox {name!r} did not become {'checked' if want else 'unchecked'}", flush=True)
    return False


def selected_count():
    """How many rows are selected: their checkboxes ("Select <name>") are
    checked (the selection bar's "n selected" is not in the AT-SPI tree)."""
    n = 0
    for name, node in by_role(pid, Atspi.Role.CHECK_BOX):
        if name.startswith("Select ") and name != "Select all shown" and is_checked(node):
            n += 1
    return n


def row_names():
    return [r[0] for r in table_rows(pid)]


def wait_rows(pred, timeout=20):
    return wait_for(lambda: pred(row_names()), timeout)


def at_folder(path_text, marker=None, timeout=20):
    def ok():
        t = text(pid)
        return path_text in t and "Loading" not in t and (marker is None or marker in t)
    return wait_for(ok, timeout)


def go_root():
    if "Root of the bucket" not in text(pid):
        press(pid, BUCKET)
    return at_folder("Root of the bucket")


def go(folder):
    """Opens `folder` (a/b/) from the root, segment by segment."""
    if not go_root():
        return False
    path = ""
    for seg in folder.strip("/").split("/"):
        path += seg + "/"
        if not (press(pid, seg) and at_folder(path)):
            return False
    return True


def op_region():
    """The operation panel (region "File operation"), or None."""
    for role, name, _, node in nodes(pid):
        if name == "File operation" and role != Atspi.Role.PUSH_BUTTON:
            return node
    return None


def op_text():
    r = op_region()
    return cell_text(r) if r is not None else ""


def op_finished():
    r = op_region()
    return r is not None and bool(find(r, Atspi.Role.PUSH_BUTTON, "Dismiss"))


def review_dialog(verb):
    """The review of a plan: a dialog titled "<Verb> …?"."""
    for role, name, _, node in nodes(pid):
        if role == Atspi.Role.DIALOG and name.startswith(verb) and name.rstrip().endswith("?"):
            return node
    return None


def run_reviewed(verb, before_start=None, timeout=60):
    """Waits for the review of a `verb` plan, runs `before_start(dialog)`,
    presses `verb` in it and waits for the operation's final state.
    Returns the review's text and the panel's, or None."""
    d = wait_for(lambda: review_dialog(verb), 30)
    if d is None:
        print(f"  no review for {verb}: dialogs {[n for r, n, _, _ in nodes(pid) if r == Atspi.Role.DIALOG]}; "
              f"{op_text()[:200]}", flush=True)
        return None
    review = cell_text(d)
    if before_start is not None:
        before_start(d)
    if not press_in(d, verb):
        print(f"  {verb} could not be pressed in the review", flush=True)
        return None
    if not wait_for(lambda: review_dialog(verb) is None and op_finished(), timeout):
        print(f"  {verb} did not finish: {op_text()[:300]}", flush=True)
        return None
    return review + " || " + op_text()


def retry_operation(timeout=60):
    """Presses Try again in the panel and waits for the new final state."""
    before = op_text()
    if op_region() is None or not press_in(op_region(), "Try again"):
        return None
    wait_for(lambda: op_text() != before or not op_finished(), 10)
    return op_text() if wait_for(op_finished, timeout) else None


def dismiss_operation():
    r = op_region()
    if r is not None:
        press_in(r, "Dismiss")
        wait_for(lambda: op_region() is None, 5)


# ---- XTest pointer and keys ----
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


def row_node(name):
    for role, _, _, node in nodes(pid):
        if role == Atspi.Role.TABLE_ROW and node.get_child_count() > 2:
            if cell_text(node.get_child_at_index(0)).startswith(name):
                return node
    return None


def click_row(name, button=1, mods=()):
    """Clicks the row's Size cell (not its buttons) with the real pointer."""
    r = row_node(name)
    if r is None:
        return False
    cell = r.get_child_at_index(1)
    x, y, w, h = extents(cell)
    if w <= 0 or h <= 0:
        print(f"  row {name!r}: no extents ({x}, {y}, {w}, {h})", flush=True)
        return False
    focus_window(pid)
    click_xy(x + w / 2, y + h / 2, button, mods)
    if not getattr(click_row, "told", False):
        click_row.told = True
        print(f"  pointer: clicked {name!r} at ({x + w / 2:.0f}, {y + h / 2:.0f}), cell {w}x{h}", flush=True)
    return True


def keystroke(name, mods=()):
    focus_window(pid)
    codes = [X.XKeysymToKeycode(dpy, X.XStringToKeysym(m.encode())) for m in mods]
    for c in codes:
        XT.XTestFakeKeyEvent(dpy, c, 1, 0)
    key(keysym(name))
    for c in reversed(codes):
        XT.XTestFakeKeyEvent(dpy, c, 0, 0)
    X.XSync(dpy, 0)
    time.sleep(0.25)


SELECTS = {
    "Sort by": ["Bucket order", "Name", "Size", "Last modified", "Type"],
    "Search where": ["In this folder and below", "In the whole bucket"],
    "Search how": ["Name contains", "Key starts with"],
    "Valid for": ["1 hour", "1 day", "7 days"],
}


def select_option(label, option):
    """Chooses `option` in the <select> named `label` with the keyboard
    (focus, Home, Down × index): WebKitGTK fires the change event for
    keyboard changes, not for an AT-SPI selection."""
    index = SELECTS[label].index(option)
    for role, name, _, node in nodes(pid):
        if role == Atspi.Role.COMBO_BOX and (name == label or name.startswith(label)):
            focus_window(pid)
            Atspi.Component.grab_focus(node)
            if not wait_for(lambda: node.get_state_set().contains(Atspi.StateType.FOCUSED), 3):
                print(f"  select {label!r}: could not focus", flush=True)
                return False
            key(keysym("Home"))
            for _ in range(index):
                key(keysym("Down"))
            time.sleep(0.3)
            return True
    print(f"  no select {label!r}: {[n for r, n, _, _ in nodes(pid) if r == Atspi.Role.COMBO_BOX]}", flush=True)
    return False


# ---- GTK folder dialog ----
def folder_chooser(title):
    for role, name, _, node in nodes(pid):
        if role == Atspi.Role.FILE_CHOOSER and name == title:
            return node
    return None


def choose_folder(title, path):
    """In the GTK folder dialog: goes to the parent of `path` through the
    location bar, selects `path`'s row in the file list and presses Open
    (how GTK's folder mode accepts a folder)."""
    dlg = wait_for(lambda: folder_chooser(title), 20)
    if dlg is None:
        print(f"  no folder dialog {title!r}: choosers "
              f"{[n for r, n, _, _ in nodes(pid) if r in (Atspi.Role.FILE_CHOOSER, Atspi.Role.DIALOG)]}", flush=True)
        return False
    focus_window(pid)
    parent, name = os.path.split(path.rstrip("/"))
    table = wait_for(lambda: (find(dlg, Atspi.Role.TABLE) or [None])[0], 5)
    if table is not None:
        Atspi.Component.grab_focus(table)
        wait_for(lambda: table.get_state_set().contains(Atspi.StateType.FOCUSED), 3)
    ctrl("l")
    entry = wait_for(lambda: next((e for e in find(dlg, Atspi.Role.TEXT)
                                   if e.get_state_set().contains(Atspi.StateType.SHOWING)
                                   and e.get_state_set().contains(Atspi.StateType.EDITABLE)), None), 5)
    if entry is None:
        print("  folder dialog: no location bar", flush=True)
        return False
    Atspi.Component.grab_focus(entry)
    ctrl("a")
    type_text(parent + "/")
    if wait_for(lambda: Atspi.Text.get_text(entry, 0, -1) != parent + "/", 1.5):
        key(keysym("Delete"))
    time.sleep(0.7)
    key(keysym("Return"))

    def row_index():
        t = (find(dlg, Atspi.Role.TABLE) or [None])[0]
        if t is None:
            return None
        cells = [c for c in find(t, Atspi.Role.TABLE_CELL) if c.get_text_iface() is not None]
        for c in cells:
            if Atspi.Text.get_text(c, 0, -1) == name:
                return t, c
        return None
    found = wait_for(row_index, 10)
    if found is None:
        print(f"  folder dialog: no row {name!r} in {parent!r}", flush=True)
        return False
    t, cell = found
    # Select the folder's row: through its cell's parent row index.
    try:
        idx = cell.get_index_in_parent()
        cols = max(1, Atspi.Table.get_n_columns(t))
        Atspi.Table.add_row_selection(t, idx // cols)
    except Exception:  # noqa: BLE001
        Atspi.Component.grab_focus(cell)
        key(keysym("space"))
    time.sleep(0.4)
    for b in find(dlg, Atspi.Role.PUSH_BUTTON, "Open"):
        Atspi.Action.do_action(b, 0)
        if wait_for(lambda: folder_chooser(title) is None, 6):
            return True
    print(f"  folder dialog: still open after Open (row {name!r} selected?)", flush=True)
    for b in find(dlg, Atspi.Role.PUSH_BUTTON, "Cancel"):
        Atspi.Action.do_action(b, 0)
    return False


def close_choosers():
    """Cancels a GTK dialog a failed step left open (it holds the keyboard)."""
    for role, _, _, node in nodes(pid):
        if role == Atspi.Role.FILE_CHOOSER:
            for b in find(node, Atspi.Role.PUSH_BUTTON, "Cancel") + find(node, Atspi.Role.PUSH_BUTTON, "_Cancel"):
                Atspi.Action.do_action(b, 0)


def tree(dest):
    out = []
    for base, dirs, files in os.walk(dest):
        dirs.sort()
        rel = os.path.relpath(base, dest)
        for d in dirs:
            out.append(os.path.normpath(os.path.join(rel, d)) + "/")
        for f in sorted(files):
            out.append(os.path.normpath(os.path.join(rel, f)))
    return sorted(out)


# =================================================================== setup
ok = add("Files", BUCKET, KEY1, SECRET1, ENDPOINT)
check("files: add an account for the local versioned bucket (in the test profile's namespace)",
      bool(ok and wait_for(lambda: "Added Files" in text(pid))), text(pid))
ok = press(pid, "Open bucket") and at_folder("Root of the bucket", "photos-backup")
check("files: Open bucket lists the root", bool(ok), text(pid)[-300:])
keys0, _ = state()
SEEDED = current(keys0)

# ============================================================ folder management
# Create a folder, navigate into it, breadcrumbs, a nested folder, Up.
ok = press(pid, "New folder…") and wait_for(lambda: dialog() is not None, 10) \
    and fill_field("Name", "made") and press_dialog("Create folder")
ok = ok and wait_for(lambda: "Folder “made” created." in text(pid), 20)
keys, _ = state()
check("folders: New folder creates the marker made/ (and nothing else)",
      bool(ok) and current(keys) == sorted(SEEDED + ["made/"]), str(ok))
ok = press(pid, "made") and at_folder("made/", "This folder is empty.")
check("folders: opening the new folder shows it empty, with made in the breadcrumbs", bool(ok), text(pid)[-200:])
ok = press(pid, "New folder…") and wait_for(lambda: dialog() is not None, 10) \
    and fill_field("Name", "inner") and press_dialog("Create folder") \
    and wait_for(lambda: "Folder “inner” created." in text(pid), 20)
ok = ok and press(pid, "inner") and at_folder("made/inner/", "This folder is empty.")
check("folders: a nested folder made/inner/ is created and opened", bool(ok))
ok = press(pid, "Up one folder") and at_folder("made/", "inner")
ok2 = ok and press(pid, BUCKET) and at_folder("Root of the bucket")
check("folders: Up one folder and the bucket breadcrumb go back up", bool(ok and ok2))

recover()
# Rename photos/ → pictures/: every object moves, photos-backup/ and photosx stay.
ok = chain("rename photos", lambda: wait_rows(lambda r: "photos" in r), lambda: toggle("Select photos"),
           lambda: wait_for(lambda: selected_count() == 1, 5), lambda: press(pid, "Rename"),
           lambda: wait_for(lambda: dialog() is not None, 10), lambda: fill_field("New name", "pictures"),
           lambda: press_dialog("Rename"))
review = ok and run_reviewed("Rename")
keys, _ = state()
check("folders: Rename photos/ → pictures/ is reviewed with its 3 objects, then done",
      bool(review) and "3" in review, str(review)[:300])
check("folders: the renamed folder holds exactly the three objects, same content",
      under(keys, "pictures/") == ["pictures/a.txt", "pictures/b.txt", "pictures/sub/c.txt"]
      and content(keys, "pictures/sub/c.txt") == "content of photos/sub/c.txt\n", str(under(keys, "pictures/")))
check("folders: prefix boundary: photos-backup/ (marker and file) and photosx are untouched",
      under(keys, "photos-backup") == ["photos-backup/", "photos-backup/keep.txt"]
      and keys["photosx"]["current"] is not None
      and len(keys["photos-backup/keep.txt"]["versions"]) == 1, str(under(keys, "photos")))
check("folders: the originals are hidden behind delete markers (versions kept)",
      under(keys, "photos/") == [] and all(len(keys[k]["versions"]) == 2 and keys[k]["versions"][-1][1]
                                         for k in ("photos/a.txt", "photos/b.txt", "photos/sub/c.txt")))

recover()
# Copy pictures/ into made/ through the destination picker.
go_root()
ok = chain("copy pictures", lambda: toggle("Select pictures"), lambda: press(pid, "Copy to…"),
           lambda: pick_destination("made/", "Copy to"))
review = ok and run_reviewed("Copy")
keys, _ = state()
check("folders: Copy pictures/ into made/ (picker) copies every object and keeps the originals",
      bool(review) and under(keys, "made/pictures/") == ["made/pictures/a.txt", "made/pictures/b.txt",
                                                         "made/pictures/sub/c.txt"]
      and len(under(keys, "pictures/")) == 3, str(under(keys, "made/")))

recover()
# Move made/pictures/ to the root, where pictures/ exists: keep both.
ok = chain("move made/pictures", lambda: go("made/"), lambda: toggle("Select pictures"), lambda: press(pid, "Move to…"),
           lambda: pick_destination("", "Move to"))


def keep_both(d):
    for r in find(d, Atspi.Role.RADIO_BUTTON):
        if r.get_name().startswith("Keep both"):
            Atspi.Action.do_action(r, 0)
    time.sleep(0.3)


review = ok and run_reviewed("Move", keep_both)
keys, _ = state()
check("folders: Move into a folder where the name exists shows the conflict and keeps both as “pictures (1)”",
      bool(review) and "already exist" in review
      and under(keys, "pictures (1)/") == ["pictures (1)/a.txt", "pictures (1)/b.txt", "pictures (1)/sub/c.txt"]
      and under(keys, "made/pictures/") == [] and len(under(keys, "pictures/")) == 3, str(review)[:300])

recover()
# Delete pictures/: only it; pictures (1)/ and photos-backup/ stay.
go_root()
ok = toggle("Select pictures") and press(pid, "Delete")
review = ok and run_reviewed("Delete")
keys, _ = state()
check("folders: Delete pictures/ says the bucket keeps versions, hides exactly its 3 objects",
      bool(review) and "keeps versions" in review and under(keys, "pictures/") == []
      and len(under(keys, "pictures (1)/")) == 3 and len(under(keys, "photos-backup")) == 2, str(review)[:300])
recover()
# An empty folder (marker only).
ok = go("made/") and toggle("Select inner") and press(pid, "Delete")
review = ok and run_reviewed("Delete")
keys, _ = state()
check("folders: deleting an empty folder removes (hides) only its marker", bool(review)
      and "made/inner/" not in current(keys) and "made/" in current(keys))

recover()
# ============================================================== files
ok = go("files/") and toggle("Select one.txt") and press(pid, "Rename") and wait_for(lambda: dialog() is not None, 10) \
    and fill_field("New name", "uno.txt") and press_dialog("Rename")
review = ok and run_reviewed("Rename")
keys, _ = state()
check("files: Rename one.txt → uno.txt", bool(review) and content(keys, "files/uno.txt") == "content of files/one.txt\n"
      and keys["files/one.txt"]["current"] is None)
ok = wait_rows(lambda r: "uno.txt" in r) and toggle("Select two.txt") and press(pid, "Properties")
props = ok and wait_for(lambda: "SHA-256" in dialog_text() and "Preview" in dialog_text()
                        and "content of files/two.txt" in dialog_text(), 20) and dialog_text()
check("files: Properties show key, size, content type, the stored SHA-256 and a text preview",
      bool(props) and "files/two.txt" in props and "text/plain" in props
      and hashlib.sha256(b"content of files/two.txt\n").hexdigest() in props, str(props)[:400])
press_dialog("Close")
toggle("Select two.txt", want=False)

recover()
# Conflicts: skip, then keep both.
ok = chain("copy x.txt", lambda: go("conflict/src/"), lambda: toggle("Select x.txt"), lambda: press(pid, "Copy to…"),
           lambda: pick_destination("conflict/dest/", "Copy to"))
d = ok and wait_for(lambda: review_dialog("Copy"), 30)
review = cell_text(d) if d else ""
start_disabled = bool(d) and not any(b.get_state_set().contains(Atspi.StateType.ENABLED)
                                     for b in find(d, Atspi.Role.PUSH_BUTTON, "Copy"))
cancelled = bool(d) and press_in(d, "Cancel") and wait_for(lambda: review_dialog("Copy") is None, 10)
keys, _ = state()
check("files: an existing destination is shown; with Skip (the default) nothing can start; Cancel; nothing replaced",
      "already exist" in review and start_disabled and bool(cancelled)
      and content(keys, "conflict/dest/x.txt") == "already there\n" and "conflict/dest/x (1).txt" not in current(keys),
      review[:300])
ok = chain("copy x.txt again", lambda: go("conflict/src/"), lambda: toggle("Select x.txt"), lambda: press(pid, "Copy to…"),
           lambda: pick_destination("conflict/dest/", "Copy to"))
review = ok and run_reviewed("Copy", keep_both)
keys, _ = state()
check("files: Keep both copies to “x (1).txt”, the existing file unchanged",
      bool(review) and content(keys, "conflict/dest/x (1).txt") == "content of conflict/src/x.txt\n"
      and content(keys, "conflict/dest/x.txt") == "already there\n")

recover()
# A move whose delete is refused: copy kept, original kept, retry finishes.
ok = chain("move two.txt", lambda: go("files/"), lambda: toggle("Select two.txt"), lambda: press(pid, "Move to…"),
           lambda: pick_destination("made/", "Move to"), lambda: control("fail DeleteObject 1"))
review = ok and run_reviewed("Move")
t = text(pid)
keys, _ = state()
check("files: a move whose delete is refused reports “copied, original kept”; both exist",
      bool(review) and "original could not be deleted" in review
      and content(keys, "made/two.txt") == "content of files/two.txt\n" and content(keys, "files/two.txt") is not None, t[-400:])
ok = retry_operation()
keys, _ = state()
check("files: Try again deletes only the original left (after checking the copy)",
      bool(ok) and keys["files/two.txt"]["current"] is None and content(keys, "made/two.txt") is not None)
dismiss_operation()

recover()
# A copy stopped by a refused request: one done or none, the rest cancelled; retry.
ok = chain("copy two files", lambda: go("files/"), lambda: toggle("Select three.txt"), lambda: toggle("Select uno.txt"),
           lambda: press(pid, "Copy to…"), lambda: pick_destination("made/", "Copy to"),
           lambda: control("fail CopyObject 1"))
review = ok and run_reviewed("Copy")
t = text(pid)
keys, _ = state()
check("files: a copy stopped by a refused request says so and copies nothing it did not report",
      bool(review) and bool(find(op_region(), Atspi.Role.PUSH_BUTTON, "Try again"))
      and content(keys, "made/three.txt") is None, str(review)[-400:])
ok = retry_operation()
keys, _ = state()
check("files: Try again copies what was left, nothing twice",
      bool(ok) and content(keys, "made/three.txt") == "content of files/three.txt\n"
      and content(keys, "made/uno.txt") == "content of files/one.txt\n"
      and len(keys["made/three.txt"]["versions"]) == 1)
dismiss_operation()

recover()
# Cancel before confirmation changes nothing.
before, _ = state()
ok = toggle("Select three.txt") and press(pid, "Delete") and wait_for(lambda: dialog() is not None, 20) \
    and press_dialog("Cancel") and wait_for(lambda: dialog() is None, 10)
after, _ = state()
check("delete: Cancel in the review changes nothing", bool(ok) and before == after)
toggle("Select three.txt", want=False)

recover()
# Delete a file, then restore it from Deleted items.
ok = toggle("Select uno.txt") and press(pid, "Delete")
review = ok and run_reviewed("Delete")
keys, _ = state()
check("delete: deleting uno.txt adds a delete marker; the version is kept",
      bool(review) and keys["files/uno.txt"]["current"] is None and keys["files/uno.txt"]["versions"][-1][1]
      and not keys["files/uno.txt"]["versions"][0][1])
ok = press_toggle("Deleted items") and wait_for(lambda: "Restore uno.txt" in buttons(pid), 20)
check("delete: Deleted items lists uno.txt as restorable", bool(ok), text(pid)[-300:])
ok = ok and press(pid, "Restore uno.txt")
review = ok and run_reviewed("Restore")
keys, _ = state()
check("delete: Restore removes exactly the delete marker; uno.txt is back with its content",
      bool(review) and content(keys, "files/uno.txt") == "content of files/one.txt\n"
      and len(keys["files/uno.txt"]["versions"]) == 1)
press(pid, "← Back to the folder")

recover()
# Partial deletion of a folder: one delete refused.
ok = go_root() and toggle("Select del")
ok = ok and press(pid, "Delete") and control("fail DeleteObject 1")
review = ok and run_reviewed("Delete")
keys, _ = state()
left = under(keys, "del/")
check("delete: a folder delete with one refused request is not reported done; one object is left",
      bool(review) and len(left) == 1 and ("Only partly done" in review or "Failed" in review), f"{left} {str(review)[-300:]}")
if op_region() is not None and find(op_region(), Atspi.Role.PUSH_BUTTON, "Try again"):
    retry_operation()
keys, _ = state()
check("delete: Try again finishes the folder", under(keys, "del/") == [])
dismiss_operation()

recover()
# ============================================================ selection, sorting, filters
ok = go("sort/") and wait_rows(lambda r: sorted(r) == ["alpha.txt", "beta.pdf", "gamma.jpg"])
check("selection: sort/ lists its three files", bool(ok), str(row_names()))
ok = click_row("alpha.txt") and wait_for(lambda: selected_count() == 1, 5)
check("selection: a click with the real pointer selects one row", bool(ok), f"{selected_count()} selected")
# Modified clicks through XTest: Ctrl-click and Shift-click with the pointer
# did not reach WebKitGTK as modified clicks on this virtual display (plain
# clicks do); counted only if they work, else reported as not verified. The
# same selection logic is checked below through the keyboard and in Vitest.
ok2 = ok and click_row("gamma.jpg", mods=("Control_L",)) and wait_for(lambda: selected_count() == 2, 3)
if ok2:
    check("selection: Ctrl-click with the real pointer adds a row", True)
else:
    not_tested.append("Ctrl-click and Shift-click with the real pointer (XTest modifiers not seen by WebKitGTK on Xvfb)")
keystroke("Escape")
ok = click_row("alpha.txt") and wait_for(lambda: selected_count() == 1, 5)
keystroke("Down", ("Shift_L",))
keystroke("Down", ("Shift_L",))
ok = ok and wait_for(lambda: selected_count() == 3, 5)
check("selection: Shift+Down extends the selection from the clicked row", bool(ok), f"{selected_count()} selected")
keystroke("Escape")
ok = wait_for(lambda: selected_count() == 0, 5)
keystroke("a", ("Control_L",))
ok2 = wait_for(lambda: selected_count() == 3, 5)
check("selection: Escape clears, Ctrl+A selects all shown", bool(ok and ok2), f"{selected_count()} selected")


def menu_items():
    return [n for r, n, _, _ in nodes(pid) if r == Atspi.Role.MENU_ITEM and n not in SELECTS["Sort by"]
            and n not in SELECTS["Search where"] and n not in SELECTS["Search how"]]


ok = click_row("beta.pdf", button=3) and wait_for(lambda: "Copy to…" in menu_items(), 3)
if ok:
    check("selection: right-click opens the context menu", True)
    keystroke("Escape")
else:
    not_tested.append("right-click with the real pointer (XTest button 3 opened no menu on Xvfb)")
# A button with aria-haspopup is a "push button menu" to AT-SPI.
keystroke("Escape")  # clear the selection: the menu then acts on beta.pdf alone
wait_for(lambda: selected_count() == 0, 5)
more = next((node for _, n, _, node in nodes(pid) if n == "More actions for beta.pdf"), None)
if more is None:
    print(f"  no node named 'More actions for beta.pdf'; ⋯ nodes: "
          f"{[(str(r), n) for r, n, t, _ in nodes(pid) if '⋯' in (n + t)][:5]}", flush=True)
ok = more is not None and Atspi.Action.do_action(more, 0) and wait_for(lambda: "Copy to…" in menu_items(), 5)
items = menu_items()
check("selection: the row's More actions button opens the menu with the actions for the selection",
      bool(ok) and "Copy to…" in items and "Delete (Del)" in items and "Rename (F2)" in items, str(items))
keystroke("Escape")
for option, want in (("Name", ["alpha.txt", "beta.pdf", "gamma.jpg"]), ("Size", ["beta.pdf", "gamma.jpg", "alpha.txt"]),
                     ("Last modified", ["gamma.jpg", "alpha.txt", "beta.pdf"])):
    ok = select_option("Sort by", option) and wait_rows(lambda r: r == want, 5)
    check(f"sorting: by {option.lower()} ascending", bool(ok), str(row_names()))
ok = press(pid, "Ascending; switch to descending") and wait_rows(lambda r: r == ["beta.pdf", "alpha.txt", "gamma.jpg"], 5)
check("sorting: descending reverses it", bool(ok), str(row_names()))
press(pid, "Descending; switch to ascending")
ok = select_option("Sort by", "Type") and wait_rows(lambda r: len(r) == 3, 5)
check("sorting: by type (no error, all rows)", bool(ok), str(row_names()))
ok = press(pid, "Filter") and wait_for(lambda: "Show only" in text(pid), 5)
# The filter's text fields are found by their label.
ok = ok and fill_field("Extensions", "pdf") and press(pid, "Apply") and wait_rows(lambda r: r == ["beta.pdf"], 5)
check("filters: Extensions pdf shows only beta.pdf", bool(ok), str(row_names()))
ok = press(pid, "Show everything") and wait_rows(lambda r: len(r) == 3, 5)
check("filters: Show everything brings the rest back", bool(ok), str(row_names()))
toggle("Select alpha.txt")
ok = press(pid, "Up one folder") and at_folder("Root of the bucket") and wait_for(lambda: selected_count() == 0, 5)
check("selection: navigating away clears the selection", bool(ok))

recover()
# ============================================================== search
ok = go("search/") and fill_field("Search", "needle") and press(pid, "Search") and wait_for(lambda: "3 results for “needle”" in text(pid), 30)
check("search: in this folder and below finds the three needles", bool(ok), text(pid)[-300:])
ok = select_option("Search where", "In the whole bucket") and press(pid, "Search") \
    and wait_for(lambda: "results for “needle”" in text(pid) and "Searching" not in text(pid), 60)
more = ok and "Search further" in buttons(pid)
check("search: the whole bucket scans 10,000 objects per call and offers Search further", bool(more), text(pid)[-300:])
ok = more and press(pid, "Search further") and wait_for(lambda: "zz-needle.txt" in text(pid), 60)
check("search: Search further continues where it stopped and finds bulk/zz-needle.txt", bool(ok))
ok = select_option("Search how", "Key starts with") and fill_field("Search", "search/deep/") and press(pid, "Search") and wait_for(lambda: "2 results for “search/deep/”" in text(pid), 30)
check("search: Key starts with finds the two keys under search/deep/", bool(ok), text(pid)[-300:])
press(pid, "Clear search")
ok = go("bulk/") and wait_for(lambda: "1000 items shown; more are available" in text(pid), 60)
ok2 = ok and press(pid, "Load more") and wait_for(lambda: "2000 items shown" in text(pid), 60)
check("browse: a folder of 10,051 objects shows 1,000, then Load more adds the next page", bool(ok and ok2))

recover()
# ======================================================== previews and share links
def preview_of(folder, name, want, timeout=20):
    if not (go(folder) and toggle(f"Select {name}") and press(pid, "Properties")):
        return None
    t = wait_for(lambda: (lambda d: d if want(d) else None)(dialog_text()), timeout)
    press_dialog("Close")
    wait_for(lambda: dialog() is None, 5)
    return t


t = preview_of("docs/", "readme.txt", lambda d: "Hello from the readme." in d)
check("preview: plain text is shown as text", bool(t))
t = preview_of("docs/", "data.json", lambda d: '"name": "sotyware"' in d)
check("preview: JSON is shown formatted", bool(t), str(t)[-300:])
t = preview_of("docs/", "cars.txt", lambda d: "BMW, Audi" in d)
check("preview: a text starting with “BM” is text, not an image (audit fix)", bool(t))
title_before = [n for r, n, _, _ in nodes(pid) if r == Atspi.Role.FRAME]
t = preview_of("docs/", "page.html", lambda d: "<script>" in d)
check("preview: HTML is shown as text (its <script> visible as characters), never rendered",
      bool(t) and not any("PWNED" in n for r, n, _, _ in nodes(pid)))
t = preview_of("docs/", "logo.svg", lambda d: "<svg" in d)
check("preview: SVG is shown as text, never as an image",
      bool(t) and not any(r == Atspi.Role.IMAGE for r, _, _, _ in nodes(pid)) and not any("PWNED" in n for _, n, _, _ in nodes(pid)))
t = preview_of("docs/", "big.txt", lambda d: "Only the first 256 KiB are shown" in d)
check("preview: a text over 256 KiB is cut and says so", bool(t))
img = None
if go("media/") and toggle("Select pixel.png") and press(pid, "Properties"):
    img = wait_for(lambda: next((n for r, n, _, _ in nodes(pid) if r == Atspi.Role.IMAGE and n == "Preview of pixel.png"), None), 20)
    press_dialog("Close")
check("preview: a PNG is shown as an image", bool(img))
t = preview_of("media/", "huge.png", lambda d: "larger than 10 MiB" in d)
check("preview: an image over 10 MiB is not read; it says so", bool(t))

link_url = None
if go("docs/") and toggle("Select readme.txt") and press(pid, "Properties") \
        and wait_for(lambda: "Create link" in [b.get_name() for b in find(dialog(), Atspi.Role.PUSH_BUTTON)] if dialog() else False, 20):
    select_option("Valid for", "1 day")
    press_dialog("Create link")
    for _ in range(50):
        for e in find(dialog(), Atspi.Role.ENTRY) + find(dialog(), Atspi.Role.TEXT):
            if e.get_name() == "Download link":
                link_url = Atspi.Text.get_text(e, 0, -1) or None
        if link_url:
            break
        time.sleep(0.2)
check("share: a link for 1 day is created for the local endpoint, without the secret",
      bool(link_url) and link_url.startswith(ENDPOINT) and "X-Amz-Expires=86400" in link_url
      and SECRET1 not in link_url, redact(str(link_url))[:200])
if link_url:
    try:
        body = urllib.request.urlopen(link_url, timeout=10).read()
    except Exception as e:  # noqa: BLE001
        body = repr(e).encode()
    check("share: the link downloads the file without credentials", body == b"Hello from the readme.\n", str(body[:80]))
not_tested.append("copying the link to the system clipboard (no clipboard manager on this virtual display)")
if dialog() is not None:
    press_dialog("Close")

recover()
# ======================================================== downloads of several items
dest = os.path.join(OUT, "batch")
os.makedirs(dest)
ok = chain("download unusual", lambda: go("unusual/"), lambda: keystroke("a", ("Control_L",)) or True,
           lambda: wait_for(lambda: selected_count() == 3, 5), lambda: press(pid, "Download"))
dialog_driven = ok and choose_folder("Choose a folder to download into", dest)
if ok and not dialog_driven:
    # The folder dialog opened (Rust asked for it), but this virtual display's
    # AT-SPI/XTest could not choose a folder in it: the rest of this section
    # is reported as not tested, never as passed or failed.
    close_choosers()
    gone = wait_for(lambda: folder_chooser("Choose a folder to download into") is None, 10)
    time.sleep(1)
    check("download: the native GTK folder dialog opens (titled, folder mode); Cancel in it starts nothing and writes nothing",
          bool(gone) and tree(dest) == [] and "Cancel download" not in buttons(pid), str(tree(dest)))
    dismiss_bulk = [b for b in buttons(pid) if b == "Dismiss"]
    if dismiss_bulk:
        press(pid, "Dismiss")
    not_tested.append("several-item downloads through the GTK folder dialog (the dialog opened; it could not be "
                      "driven through AT-SPI/XTest on this virtual display); covered by the Rust tests")
else:
    ok = ok and wait_for(lambda: "Downloaded" in text(pid) or "saved" in text(pid).lower(), 60)
    check("download: three unusual names are saved with safe local names",
          bool(ok) and tree(dest) == sorted(["a_b#c%.txt", "größe +test.txt", "_con.txt"]), str(tree(dest)))
    press(pid, "Dismiss")
    recover()
    # A folder and a file: structure kept; a second time, numbered names.
    dest2 = os.path.join(OUT, "folders")
    os.makedirs(dest2)
    for round_ in (1, 2):
        ok = go("dl/") and toggle("Select tree") and toggle("Select one.txt") and wait_for(lambda: selected_count() == 2, 5) \
            and press(pid, "Download") and choose_folder("Choose a folder to download into", dest2)
        ok = ok and wait_for(lambda: "Dismiss" in buttons(pid) and "Cancel download" not in buttons(pid), 60)
        press(pid, "Dismiss")
    want = ["one.txt", "tree/", "tree/a.txt", "tree/deep/", "tree/deep/b.txt"]
    want2 = sorted(want + ["one (1).txt", "tree (1)/", "tree (1)/a.txt", "tree (1)/deep/", "tree (1)/deep/b.txt"])
    check("download: a folder keeps its structure; a second download is numbered, nothing replaced",
          bool(ok) and tree(dest2) == want2
          and open(os.path.join(dest2, "tree", "deep", "b.txt"), encoding="utf-8").read() == "content of dl/tree/deep/b.txt\n",
          str(tree(dest2)))
    recover()
    # A refused GetObject: that item fails, nothing of it is left; retry.
    dest3 = os.path.join(OUT, "retry")
    os.makedirs(dest3)
    ok = go("dl/") and toggle("Select tree") and toggle("Select one.txt") and control("fail GetObject 1") \
        and press(pid, "Download") and choose_folder("Choose a folder to download into", dest3)
    ok = ok and wait_for(lambda: "Download the rest again" in buttons(pid), 60)
    first = tree(dest3)
    check("download: a refused file fails its item, which leaves nothing; the other item is saved",
          bool(ok) and len([x for x in first if "/" not in x.rstrip("/")]) == 1 and not any(".sotyware" in x for x in first), str(first))
    ok = press(pid, "Download the rest again") and wait_for(lambda: "Download the rest again" not in buttons(pid)
                                                             and "Cancel download" not in buttons(pid), 60)
    check("download: Download the rest again completes it", bool(ok) and tree(dest3) == want, str(tree(dest3)))
    press(pid, "Dismiss")

    recover()
close_choosers()
# ======================================================== unversioned bucket
press(pid, "← Back to accounts")
ok = wait_for(lambda: "Open bucket" in buttons(pid), 20) and add("Plain", "files-plain", KEY1, SECRET1, ENDPOINT) \
    and wait_for(lambda: "Added Plain" in text(pid), 20)
opened = False
check("plain: add an account for the unversioned bucket", bool(ok))
if ok:
    # The second card's Open bucket: by the account's region of the page, pressed through its own card.
    cards = [n for r, nm, _, n in nodes(pid) if r == Atspi.Role.PUSH_BUTTON and nm == "Open bucket"]
    if len(cards) == 2:
        Atspi.Action.do_action(cards[1], 0)
        opened = at_folder("Root of the bucket", "files")
ok = opened and go("files/") and toggle("Select one.txt") and press(pid, "Delete")
review = ok and run_reviewed("Delete")
keys, _ = state("files-plain")
check("plain: deleting on a bucket without versions warns that it is final, and the object is gone",
      bool(review) and "gone for good" in review and "files/one.txt" not in keys, str(review)[:300])

not_tested.append("drag and drop from another program (no drag source on this virtual display)")
for n in not_tested:
    print(f"NOT TESTED  {n}", flush=True)
