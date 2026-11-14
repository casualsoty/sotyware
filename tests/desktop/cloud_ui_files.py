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
# Native dialogs and other programs: folder uploads and several-item
# downloads through the GTK folder dialog (choose_folder), a drag and drop
# from another GTK program over XDND (drag_source.py, XTest pointer), and
# Ctrl+C/X/V with real X key events on sotyware's own clipboard. What it
# cannot drive here is reported as NOT TESTED, never as passed: the system
# clipboard (no clipboard manager), Ctrl/Shift-click with the pointer.
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


def accept_button(dlg):
    """The folder dialog's accept button ("Open" with GTK 3.24 here, as rfd
    passes no label; "Select" in some versions)."""
    for label in ("Open", "_Open", "Select", "_Select"):
        found = find(dlg, Atspi.Role.PUSH_BUTTON, label)
        if found:
            return found[0]
    return None


def choose_folder(title, path):
    """In the GTK folder dialog (GtkFileChooserNative, folder mode): leaves
    "Recent", where the dialog starts and where Open stays disabled (a
    probe on this virtual display showed AT-SPI's press of a disabled Open
    does nothing, which is why earlier drivers could not choose a folder),
    by a real click on Home in the sidebar; then Ctrl+L, the folder's path
    with a trailing "/" and Return, which in folder mode accepts that
    folder. The caller checks the result on disk or at the server, never
    only what the dialog showed."""
    dlg = wait_for(lambda: folder_chooser(title), 20)
    if dlg is None:
        print(f"  no folder dialog {title!r}: choosers "
              f"{[n for r, n, _, _ in nodes(pid) if r in (Atspi.Role.FILE_CHOOSER, Atspi.Role.DIALOG)]}", flush=True)
        return False
    focus_window(pid)
    open_btn = wait_for(lambda: accept_button(dlg), 5)
    home = wait_for(lambda: next(iter(find(dlg, Atspi.Role.LABEL, "Home")), None), 5)
    if open_btn is None or home is None:
        print(f"  folder dialog: no Open button ({open_btn is not None}) or no Home place ({home is not None})", flush=True)
        close_choosers()
        return False
    x, y, w, h = extents(home)
    click_xy(x + w // 2, y + h // 2)
    if not wait_for(lambda: open_btn.get_state_set().contains(Atspi.StateType.ENABLED), 5):
        print("  folder dialog: Open still disabled after Home", flush=True)
        close_choosers()
        return False
    ctrl("l")
    entry = wait_for(lambda: next((e for e in find(dlg, Atspi.Role.TEXT)
                                   if e.get_state_set().contains(Atspi.StateType.SHOWING)
                                   and e.get_state_set().contains(Atspi.StateType.EDITABLE)), None), 5)
    if entry is None:
        print("  folder dialog: no location bar", flush=True)
        close_choosers()
        return False
    want = path.rstrip("/") + "/"
    # Set through AT-SPI (EditableText): Xvfb's keymap has no key for "ö"
    # or "ß", and typing would race GTK's inline completion.
    Atspi.EditableText.set_text_contents(entry, want)
    Atspi.Text.set_caret_offset(entry, len(want))
    if not wait_for(lambda: Atspi.Text.get_text(entry, 0, -1) == want, 5):
        print(f"  folder dialog: location bar {Atspi.Text.get_text(entry, 0, -1)!r}", flush=True)
        close_choosers()
        return False
    # GTK completes a unique child's name inline (selected text after the
    # cursor, "dlx/" in a folder that holds only dlx): Delete removes it,
    # or Return would choose that child (seen 2026-11-14).
    time.sleep(0.8)
    if Atspi.Text.get_text(entry, 0, -1) != want:
        key(keysym("Delete"))
    if not wait_for(lambda: Atspi.Text.get_text(entry, 0, -1) == want, 3):
        print(f"  folder dialog: location bar completed to {Atspi.Text.get_text(entry, 0, -1)!r}", flush=True)
        close_choosers()
        return False
    key(keysym("Return"))
    if wait_for(lambda: folder_chooser(title) is None, 6):
        return True
    # Return went into the folder instead: Open (enabled there) chooses it.
    if open_btn.get_state_set().contains(Atspi.StateType.ENABLED):
        Atspi.EditableText.set_text_contents(entry, "")
        Atspi.Action.do_action(open_btn, 0)
        if wait_for(lambda: folder_chooser(title) is None, 6):
            return True
    print(f"  folder dialog: still open; location bar {Atspi.Text.get_text(entry, 0, -1)!r}", flush=True)
    close_choosers()
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

# ======================================================== internal clipboard, real keys
# Ctrl+C / Ctrl+X / Ctrl+V act on sotyware's own clipboard (a list of
# items in the page), never on the system clipboard; a paste only opens the
# reviewed plan. Keys are real X key events (XTest) on this virtual display.
# On sort/ (read only here) and a folder clip/ made for it.
def ctrl_key(letter):
    keystroke(letter, ("Control_L",))


control(f"add {BUCKET} clip/keep.txt")
recover()
keys0, _ = state()
ok = go("sort/") and click_row("alpha.txt") and wait_for(lambda: selected_count() == 1, 5)
# Ctrl+X first: it tells a key that never reaches the page from a broken
# copy (Ctrl+C), if only one of them works.
ctrl_key("x")
cut_works = ok and wait_for(lambda: "marked to move" in text(pid) and "Move here (1)" in buttons(pid), 5)
check("clipboard: a real Ctrl+X on a selected file marks it to move (Move here (1)); nothing is sent",
      bool(cut_works) and current(state()[0]) == current(keys0), text(pid)[-200:])
if "Clear clipboard" in buttons(pid):
    press(pid, "Clear clipboard")
ok = ok and click_row("alpha.txt") and wait_for(lambda: selected_count() == 1, 5)
ctrl_key("c")
copied = ok and wait_for(lambda: "1 item copied" in text(pid) and "Paste copy (1)" in buttons(pid), 5)
check("clipboard: a real Ctrl+C on a selected file puts it on sotyware's clipboard (Paste copy (1)); nothing is sent",
      bool(copied) and current(state()[0]) == current(keys0), text(pid)[-200:])
if ok and not copied:
    # Go on with the same clipboard through the row's menu (Copy), so that
    # Ctrl+V and the rest are still exercised.
    more = next((node for _, n, _, node in nodes(pid) if n == "More actions for alpha.txt"), None)
    if more is not None and Atspi.Action.do_action(more, 0) and wait_for(lambda: any(n == "Copy (Ctrl+C)" for r, n, _, _ in nodes(pid) if r == Atspi.Role.MENU_ITEM), 5):
        item = next((node for r, n, _, node in nodes(pid) if r == Atspi.Role.MENU_ITEM and n == "Copy (Ctrl+C)"), None)
        if item is not None:
            Atspi.Action.do_action(item, 0)
    copied = wait_for(lambda: "Paste copy (1)" in buttons(pid), 5)
    print(f"  clipboard: Ctrl+C did not act; the menu's Copy {'did' if copied else 'did not either'}", flush=True)
ok = copied and go_root() and press(pid, "Refresh") and wait_for(lambda: "clip" in text(pid), 20) and go("clip/")
ctrl_key("v")
review = ok and wait_for(lambda: review_dialog("Copy"), 15)
ctrl_key("v")
time.sleep(1)
dialogs = [n for r, n, _, _ in nodes(pid) if r == Atspi.Role.DIALOG]
check("clipboard: Ctrl+V opens the review of a copy; a second Ctrl+V while it is open opens nothing; nothing changed yet",
      bool(review) and len(dialogs) == 1 and current(state()[0]) == current(keys0), str(dialogs))
if review:
    press_in(review, "Cancel")
wait_for(lambda: review_dialog("Copy") is None, 5)
check("clipboard: Cancel in that review changes nothing", current(state()[0]) == current(keys0))
ctrl_key("v")
res = run_reviewed("Copy")
k, _ = state()
check("clipboard: Ctrl+V and Copy in the review copy alpha.txt into clip/; the original stays",
      bool(res) and content(k, "clip/alpha.txt") == "a" * 30 and content(k, "sort/alpha.txt") == "a" * 30, str(res)[:300])
dismiss_operation()
# Cut, then paste where the name exists: the review shows the conflict and
# skips it by default; nothing is replaced.
def clear_clipboard():
    if "Clear clipboard" in buttons(pid):
        press(pid, "Clear clipboard")
    return wait_for(lambda: "Clear clipboard" not in buttons(pid), 5)


def close_any_review():
    d = dialog()
    if d is not None:
        print(f"  clipboard: closing an unexpected dialog: {cell_text(d)[:120]!r}", flush=True)
        press_in(d, "Cancel") or press_in(d, "Close")
        wait_for(lambda: dialog() is None, 5)


recover()
close_any_review()
clear_clipboard()
ok = go("clip/") and toggle("Select alpha.txt") and wait_for(lambda: selected_count() == 1, 5)
ctrl_key("x")
cut = ok and wait_for(lambda: "marked to move" in text(pid) and "Move here (1)" in buttons(pid), 5)
check("clipboard: a real Ctrl+X after a finished copy marks clip/alpha.txt to move", bool(cut),
      f"navigated and selected: {bool(ok)}; {selected_count()} selected; buttons {[b for b in buttons(pid) if 'clip' in b.lower() or 'here' in b.lower()]}")
ok = cut and go("sort/")
ctrl_key("v")
review = ok and wait_for(lambda: review_dialog("Move"), 15)
rtext = cell_text(review) if review else ""
start_enabled = review and any(b.get_state_set().contains(Atspi.StateType.ENABLED) for b in find(review, Atspi.Role.PUSH_BUTTON, "Move"))
if review:
    press_in(review, "Cancel")
wait_for(lambda: review_dialog("Move") is None, 5)
close_any_review()
k2, _ = state()
check("clipboard: Ctrl+V of that cut where alpha.txt exists: the review names the conflict, Move cannot start with Skip; nothing replaced",
      bool(review) and "already exist" in rtext and not start_enabled and current(k2) == current(k)
      and len(k2["sort/alpha.txt"]["versions"]) == 1, rtext[:300])
# A folder cut and pasted into itself is refused before anything runs.
recover()
clear_clipboard()
ok = go_root() and toggle("Select clip") and wait_for(lambda: selected_count() == 1, 5)
ctrl_key("x")
ok = ok and wait_for(lambda: "marked to move" in text(pid), 5) and go("clip/")
ctrl_key("v")
refused = ok and wait_for(lambda: "cannot be put inside itself" in text(pid), 15)
close_any_review()
check("clipboard: a folder pasted into itself is refused; nothing changed",
      bool(refused) and current(state()[0]) == current(k), f"navigated, cut and opened: {bool(ok)}; {text(pid)[-200:]}")
recover()
dismiss_operation()
# While typing in the search box the shortcuts belong to the box; a file
# is on sotyware's clipboard meanwhile, so a paste would have something to do.
recover()
close_any_review()
on_clip = go("sort/") and toggle("Select beta.pdf") and wait_for(lambda: selected_count() == 1, 5)
ctrl_key("c")
on_clip = on_clip and wait_for(lambda: "Paste copy (1)" in buttons(pid), 5)
toggle("Select beta.pdf", False)
search_box = next((n for r, nm, _, n in nodes(pid) if nm == "Search" and n.get_text_iface() is not None
                   and r != Atspi.Role.PUSH_BUTTON), None)
typed_ok = False
if search_box is not None:
    sx, sy, sw, sh = extents(search_box)
    focus_window(pid)
    click_xy(sx + sw / 2, sy + sh / 2)
    wait_for(lambda: search_box.get_state_set().contains(Atspi.StateType.FOCUSED), 3)
    type_text("zz")
    ctrl_key("a")
    ctrl_key("v")
    ctrl_key("x")
    time.sleep(1)
    typed_ok = review_dialog("Copy") is None and review_dialog("Move") is None and selected_count() == 0 \
        and "marked to move" not in text(pid)
    ctrl_key("a")
    key(keysym("BackSpace"))
    keystroke("Escape")
check("clipboard: in the search box Ctrl+A, Ctrl+V and Ctrl+X edit the text: no paste (with a file on the clipboard), no selection, nothing cut",
      bool(on_clip) and typed_ok, f"file on the clipboard: {bool(on_clip)}; search box found: {search_box is not None}")
# Then: does a real click on a row give the keyboard back to the list?
close_any_review()
went = go("sort/")
ok = went and click_row("alpha.txt") and wait_for(lambda: selected_count() == 1, 5)
keystroke("a", ("Control_L",))
back = ok and wait_for(lambda: selected_count() == 3, 5)
detail = f"in sort/: {bool(went)}; clicked: {bool(ok)}; {selected_count()} selected after the click and Ctrl+A"
if ok and not back:
    row = row_node("alpha.txt")
    if row is not None:
        Atspi.Component.grab_focus(row)
        time.sleep(0.5)
        keystroke("a", ("Control_L",))
        detail += f"; after focusing the row through AT-SPI: {selected_count()} selected"
check("clipboard: after typing in the search box, a real click on a row gives the keyboard back to the list (Ctrl+A)",
      bool(back), detail)
keystroke("Escape")
recover()
dismiss_operation()
if "Clear clipboard" in buttons(pid):
    press(pid, "Clear clipboard")
go("sort/")
not_tested.append("the system clipboard: Ctrl+C/X/V never use it (by design); Copy link in Properties writes the link "
                  "to it, which cannot be read back here (no clipboard manager on this virtual display)")




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
if dialog() is not None:
    press_dialog("Close")

recover()
# ======================================================== folder uploads (native GTK folder dialog)
# The folder is chosen in the native folder dialog that Rust opens; its
# path never reaches the page. Checked at the server: exact keys, content,
# the stored SHA-256; empty folders add nothing (S3 folders exist only
# through their objects); a second upload of the same folder skips every
# existing object and replaces nothing.
UP_NAME = "Ordner größe +1"
UP = os.path.join(SRC, UP_NAME)
up_files = {
    "top.txt": "top level\n",
    "a&b#c%.txt": "special characters\n",
    "größe ü +.txt": "unicode name\n",
    "nested/deep/inner.txt": "two levels down\n",
}
for rel, body in up_files.items():
    os.makedirs(os.path.dirname(os.path.join(UP, rel)), exist_ok=True)
    with open(os.path.join(UP, rel), "w", encoding="utf-8") as fh:
        fh.write(body)
os.makedirs(os.path.join(UP, "nested", "empty"))
os.makedirs(os.path.join(UP, "only-empty", "inside"))
want_keys = sorted(f"{UP_NAME}/{rel}" for rel in up_files)


def upload_done(t=None):
    t = text(pid) if t is None else t
    return ("Upload finished" in t or "Upload stopped" in t or "Upload cancelled" in t) and "Dismiss" in buttons(pid)


def upload_summary():
    t = text(pid)
    m = re.search(r"Upload (finished|stopped after an error|cancelled)\. [^.]*\.", t)
    return m.group(0) if m else t[-300:]


def dismiss_upload():
    if "Dismiss" in buttons(pid):
        press(pid, "Dismiss")
    wait_for(lambda: "Upload folder…" in buttons(pid), 5)


recover()
go_root()
keys_before, _ = state()
ok = press(pid, "Upload folder…") and wait_for(lambda: folder_chooser("Choose a folder to upload") is not None, 20)
if ok:
    close_choosers()
gone = ok and wait_for(lambda: folder_chooser("Choose a folder to upload") is None, 10)
time.sleep(1)
keys_after, _ = state()
check("folder upload: Upload folder… opens the native GTK folder dialog; Cancel in it sends nothing",
      bool(gone) and current(keys_after) == current(keys_before) and "Cancel upload" not in buttons(pid))
dismiss_upload()

ok = press(pid, "Upload folder…") and choose_folder("Choose a folder to upload", UP)
check("folder upload: a folder is chosen through the native GTK folder dialog (location bar, Select)", bool(ok))
ok = ok and wait_for(upload_done, 60)
keys, _ = state()
got = under(keys, UP_NAME + "/")
check("folder upload: exactly the 4 files arrive under “Ordner größe +1/”, nested folders kept, empty folders add nothing",
      bool(ok) and got == want_keys and "Upload finished. 4 files: 4 uploaded." in text(pid), f"{got} | {upload_summary()}")
check("folder upload: every object holds its file's content and the stored SHA-256 of it",
      all(content(keys, f"{UP_NAME}/{rel}") == body
          and keys[f"{UP_NAME}/{rel}"]["current"]["sha256Metadata"] == keys[f"{UP_NAME}/{rel}"]["current"]["sha256"]
          for rel, body in up_files.items()), str({k: keys.get(k, {}).get("current") for k in want_keys})[:400])
dismiss_upload()
ok = press(pid, "Upload folder…") and choose_folder("Choose a folder to upload", UP) and wait_for(upload_done, 60)
keys2, _ = state()
check("folder upload: the same folder again skips all 4 (already in this folder); nothing replaced, no new version",
      bool(ok) and "Upload finished. 4 files: 0 uploaded, 4 skipped." in text(pid)
      and all(len(keys2[k]["versions"]) == 1 and content(keys2, k) == content(keys, k) for k in want_keys),
      upload_summary())
dismiss_upload()
# A refused PUT in the middle: in key order the three existing files are
# skipped, new1.txt is refused, and the batch stops there (access errors
# stop it): new2.txt and top.txt are not started. Try again offers only the
# files not started (an access-denied file is not retried: the account
# needs fixing first); top.txt is then skipped as existing. A third upload
# of the folder sends new1.txt and skips the other five.
for n in ("new1.txt", "new2.txt"):
    with open(os.path.join(UP, n), "w", encoding="utf-8") as fh:
        fh.write(f"{n} body\n")
ok = control("fail PutObject 1") and press(pid, "Upload folder…") and choose_folder("Choose a folder to upload", UP) \
    and wait_for(upload_done, 60)
keys3, _ = state()
first = upload_summary()
check("folder upload: a refused PUT stops the batch and says so (3 skipped, 1 failed, 2 not started); nothing new stored",
      bool(ok) and "Upload stopped after an error. 6 files: 0 uploaded, 3 skipped, 1 failed, 2 not started." == first
      and f"{UP_NAME}/new1.txt" not in current(keys3) and f"{UP_NAME}/new2.txt" not in current(keys3), first)
retry_btn = next((b for b in buttons(pid) if b.startswith("Try ") and b.endswith(" again")), None)
ok = retry_btn is not None and press(pid, retry_btn) and wait_for(lambda: upload_done() and upload_summary() != first, 60)
keys4, _ = state()
check("folder upload: Try again takes only the two not started: new2.txt is sent, top.txt skipped as existing",
      bool(ok) and retry_btn == "Try 2 files again" and upload_summary() == "Upload finished. 2 files: 1 uploaded, 1 skipped."
      and content(keys4, f"{UP_NAME}/new2.txt") == "new2.txt body\n" and f"{UP_NAME}/new1.txt" not in current(keys4)
      and all(len(keys4[k]["versions"]) == 1 for k in want_keys), f"{retry_btn} | {upload_summary()}")
dismiss_upload()
ok = press(pid, "Upload folder…") and choose_folder("Choose a folder to upload", UP) and wait_for(upload_done, 60)
keys5, _ = state()
check("folder upload: the folder once more sends only new1.txt; the other five are skipped, none gets a new version",
      bool(ok) and upload_summary() == "Upload finished. 6 files: 1 uploaded, 5 skipped."
      and content(keys5, f"{UP_NAME}/new1.txt") == "new1.txt body\n"
      and all(len(keys5[k]["versions"]) == 1 for k in want_keys + [f"{UP_NAME}/new2.txt"]), upload_summary())
dismiss_upload()
not_tested.append("cancelling a folder upload while a file is sent (the same batch code as Upload files…, "
                  "whose cancel the upload phase drives; only the dialog's Cancel is driven here)")

# ======================================================== downloads of several items (native GTK folder dialog)
def bulk_done():
    b = buttons(pid)
    return "Dismiss" in b and "Cancel download" not in b


def bulk_summary():
    m = re.search(r"Download (finished|finished with problems|cancelled|stopped after an error): .*?\(\d[\d,]* files?, [^)]*\)\.", text(pid))
    return m.group(0) if m else text(pid)[-300:]


def no_staging(dest):
    return not any(".sotyware-download" in x for x in tree(dest))


def digest(dest):
    out = {}
    for x in tree(dest):
        p = os.path.join(dest, x)
        if os.path.isfile(p):
            st = os.stat(p)
            with open(p, "rb") as fh:
                out[x] = (st.st_mtime_ns, hashlib.sha256(fh.read()).hexdigest())
    return out


recover()
dest = os.path.join(OUT, "batch")
os.makedirs(dest)
ok = chain("download dialog cancel", lambda: go("unusual/"), lambda: keystroke("a", ("Control_L",)) or True,
           lambda: wait_for(lambda: selected_count() == 3, 5), lambda: press(pid, "Download"),
           lambda: wait_for(lambda: folder_chooser("Choose a folder to download into") is not None, 20))
if ok:
    close_choosers()
gone = ok and wait_for(lambda: folder_chooser("Choose a folder to download into") is None, 10)
time.sleep(1)
check("download: the native GTK folder dialog opens (titled, folder mode); Cancel in it starts nothing and writes nothing",
      bool(gone) and tree(dest) == [] and "Cancel download" not in buttons(pid), str(tree(dest)))
if "Dismiss" in buttons(pid):
    press(pid, "Dismiss")
ok = wait_for(lambda: selected_count() == 3, 5) and press(pid, "Download") \
    and choose_folder("Choose a folder to download into", dest) and wait_for(bulk_done, 60)
check("download: a folder is chosen through the native GTK folder dialog; three unusual names saved with safe local names",
      bool(ok) and tree(dest) == sorted(["a_b#c%.txt", "größe +test.txt", "_con.txt"])
      and open(os.path.join(dest, "_con.txt"), encoding="utf-8").read() == "content of unusual/con.txt\n",
      f"{tree(dest)} | {bulk_summary()}")
press(pid, "Dismiss")

# Nested remote folders whose names need safe local names: characters
# Windows refuses, a reserved device name, a trailing dot, two keys that
# differ only in case, and two keys that become one local name.
dlx = ["dlx/sub:dir/x?.txt", "dlx/sub_dir/x_.txt", "dlx/CON/aux.txt", "dlx/trail./end .txt",
       "dlx/Case.txt", "dlx/case.txt"]
control(*[f"add {BUCKET} {k}" for k in dlx])
dest_x = os.path.join(OUT, "nested")
os.makedirs(dest_x)
recover()
ok = go_root() and press(pid, "Refresh") and wait_for(lambda: "dlx" in text(pid), 20) and toggle("Select dlx") \
    and wait_for(lambda: selected_count() == 1, 5) and press(pid, "Download") \
    and choose_folder("Choose a folder to download into", dest_x) and wait_for(bulk_done, 60)
got = tree(dest_x)
want_x = sorted(["dlx/", "dlx/Case.txt", "dlx/case (1).txt", "dlx/_CON/", "dlx/_CON/_aux.txt",
                 "dlx/sub_dir/", "dlx/sub_dir/x_.txt", "dlx/sub_dir/x_ (1).txt", "dlx/trail/", "dlx/trail/end .txt"])
check("download: a nested folder keeps its structure; unsafe names are made safe and clashes (also by case) numbered",
      bool(ok) and got == want_x and no_staging(dest_x), f"{got} | {bulk_summary()}")
texts = sorted(open(os.path.join(dest_x, "dlx", "sub_dir", n), encoding="utf-8").read()
               for n in ("x_.txt", "x_ (1).txt")) if ok and got == want_x else []
check("download: the two keys that share a local name are both saved, each with its own content",
      texts == ["added later: dlx/sub:dir/x?.txt\n", "added later: dlx/sub_dir/x_.txt\n"], str(texts))
press(pid, "Dismiss")
before = digest(dest_x)
ok = wait_for(lambda: selected_count() == 1, 5) and press(pid, "Download") \
    and choose_folder("Choose a folder to download into", dest_x) and wait_for(bulk_done, 60)
got2 = tree(dest_x)
check("download: the same folder again goes to “dlx (1)”; every existing local file unchanged (content and time)",
      bool(ok) and "Saved as “dlx (1)”" in text(pid) and all(x in got2 for x in want_x)
      and "dlx (1)/sub_dir/x_ (1).txt" in got2 and {k: v for k, v in digest(dest_x).items() if k in before} == before,
      f"{got2} | {bulk_summary()}")
press(pid, "Dismiss")

# A local file already has the name: the download is numbered, the local
# file is left as it is.
dest2 = os.path.join(OUT, "folders")
os.makedirs(dest2)
with open(os.path.join(dest2, "one.txt"), "w", encoding="utf-8") as fh:
    fh.write("my own local file\n")
recover()
ok = go("dl/") and toggle("Select tree") and toggle("Select one.txt") and wait_for(lambda: selected_count() == 2, 5) \
    and press(pid, "Download") and choose_folder("Choose a folder to download into", dest2) and wait_for(bulk_done, 60)
want = ["one.txt", "tree/", "tree/a.txt", "tree/deep/", "tree/deep/b.txt"]
check("download: an existing local one.txt is never replaced; the download is saved as “one (1).txt”",
      bool(ok) and tree(dest2) == sorted(want + ["one (1).txt"])
      and open(os.path.join(dest2, "one.txt"), encoding="utf-8").read() == "my own local file\n"
      and open(os.path.join(dest2, "one (1).txt"), encoding="utf-8").read() == "content of dl/one.txt\n"
      and open(os.path.join(dest2, "tree", "deep", "b.txt"), encoding="utf-8").read() == "content of dl/tree/deep/b.txt\n",
      f"{tree(dest2)} | {bulk_summary()}")
press(pid, "Dismiss")

# A refused GetObject: access errors stop the batch, so the refused item
# fails and the other is not started; nothing of either is left; retry.
recover()
dest3 = os.path.join(OUT, "retry")
os.makedirs(dest3)
ok = go("dl/") and toggle("Select tree") and toggle("Select one.txt") and control("fail GetObject 1") \
    and press(pid, "Download") and choose_folder("Choose a folder to download into", dest3)
ok = ok and wait_for(lambda: "Download the rest again" in buttons(pid), 60)
first = tree(dest3)
check("download: a refused file stops the batch and says so (0 saved, 1 failed, 1 not saved); nothing is left, no staging folder",
      bool(ok) and first == [] and os.listdir(dest3) == []
      and bulk_summary().startswith("Download stopped after an error: 0 saved, 1 failed, 1 not saved (cancelled)"),
      f"{first} | {bulk_summary()}")
ok = press(pid, "Download the rest again") and wait_for(lambda: "Download the rest again" not in buttons(pid) and bulk_done(), 60)
check("download: Download the rest again completes it", bool(ok) and tree(dest3) == want, str(tree(dest3)))
press(pid, "Dismiss")

# Cancel while a file is being fetched (the server holds its GET open):
# progress shows 0 files done, nothing is left behind, retry finishes it.
recover()
dest4 = os.path.join(OUT, "cancel")
os.makedirs(dest4)
ok = go("dl/") and toggle("Select tree") and toggle("Select one.txt") and control("hang GetObject 1") \
    and press(pid, "Download") and choose_folder("Choose a folder to download into", dest4)
running = ok and wait_for(lambda: "Cancel download" in buttons(pid) and "0 of 3 files" in text(pid), 20)
check("download: while a file is fetched the panel shows progress (0 of 3 files) and Cancel download", bool(running), text(pid)[-300:])
t0 = time.time()
ok = running and press(pid, "Cancel download") and wait_for(lambda: "Download cancelled" in text(pid) and bulk_done(), 30)
check("download: Cancel download stops it promptly; nothing is left in the chosen folder (no staging folder)",
      bool(ok) and tree(dest4) == [] and os.listdir(dest4) == [], f"{time.time() - t0:.1f} s | {os.listdir(dest4)} | {bulk_summary()}")
ok = ok and "Download the rest again" in buttons(pid) and press(pid, "Download the rest again") \
    and wait_for(lambda: "Download the rest again" not in buttons(pid) and bulk_done(), 60)
check("download: after the cancel, Download the rest again saves everything", bool(ok) and tree(dest4) == want, str(tree(dest4)))
press(pid, "Dismiss")
recover()
# ======================================================== do real keys still reach the page?
def keys_still_work(after):
    recover()
    # Selected through its checkbox: late in the run the panels above the
    # list push its rows out of the visible area, where a pointer click
    # lands on nothing (seen 2026-11-14). The keys are real.
    ok = go("dl/") and toggle("Select one.txt") and wait_for(lambda: selected_count() == 1, 5)
    keystroke("a", ("Control_L",))
    all_ = ok and wait_for(lambda: selected_count() == 2, 5)
    keystroke("Escape")
    cleared = all_ and wait_for(lambda: selected_count() == 0, 5)
    focused = [f"{r.value_nick}:{n[:40]}" for r, n, _, node in nodes(pid)
               if node.get_state_set().contains(Atspi.StateType.FOCUSED)]
    check(f"keys: after {after}, a real Ctrl+A selects every row (and Escape clears)",
          bool(cleared), f"selected one: {bool(ok)}; {selected_count()} selected; focused: {focused}")


keys_still_work("the native GTK folder dialogs")

# ======================================================== drag and drop from another program
# A real drop over XDND: drag_source.py (another GTK program) offers the
# paths as text/uri-list, and the X pointer is moved by XTest from its
# button onto sotyware's window and released. sotyware keeps the paths in
# Rust; the page only shows names and counts and offers Upload here, which
# runs the normal upload batch (checked, existing objects skipped).
DROP = os.path.join(SRC, "drops")
for rel, body in (("drop-a.txt", "dropped file a\n"), ("drop-b.txt", "dropped file b\n"),
                  ("dropdir/inner/z.txt", "inside a dropped folder\n")):
    os.makedirs(os.path.dirname(os.path.join(DROP, rel)), exist_ok=True)
    with open(os.path.join(DROP, rel), "w", encoding="utf-8") as fh:
        fh.write(body)


def page_extents():
    for role, _, _, n in nodes(pid):
        if role == Atspi.Role.DOCUMENT_WEB:
            e = n.get_extents(Atspi.CoordType.SCREEN)
            return e.x, e.y, e.width, e.height
    return None


def drag_drop(paths):
    """Drags `paths` from drag_source.py onto the middle of sotyware's page.
    Returns whether the source was asked for the data (the drop happened)."""
    page = page_extents()
    if page is None:
        print("  drag: no page extents", flush=True)
        return False
    # cloud_ui.py runs this file with exec(): __file__ is cloud_ui.py, in the same folder.
    helper = os.path.join(os.path.dirname(os.path.abspath(__file__)), "drag_source.py")
    src = subprocess.Popen([sys.executable, helper, "5", "5", *paths],
                           stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
    try:
        line = src.stdout.readline().split()
        if not line or line[0] != "ready":
            print(f"  drag: the drag source did not start ({line})", flush=True)
            return False
        bx, by, bw, bh = (int(v) for v in line[1:5])
        sx, sy = bx + bw // 2, by + bh // 2
        tx, ty = page[0] + page[2] // 2, page[1] + page[3] // 2
        XT.XTestFakeMotionEvent(dpy, -1, sx, sy, 0)
        X.XSync(dpy, 0)
        time.sleep(0.3)
        XT.XTestFakeButtonEvent(dpy, 1, 1, 0)
        X.XSync(dpy, 0)
        steps = 25
        for i in range(1, steps + 1):
            XT.XTestFakeMotionEvent(dpy, -1, sx + (tx - sx) * i // steps, sy + (ty - sy) * i // steps, 0)
            X.XSync(dpy, 0)
            time.sleep(0.04)
        # XDND needs a few motion events over the target before the drop.
        for d in (3, -3, 3):
            XT.XTestFakeMotionEvent(dpy, -1, tx + d, ty, 0)
            X.XSync(dpy, 0)
            time.sleep(0.15)
        XT.XTestFakeButtonEvent(dpy, 1, 0, 0)
        X.XSync(dpy, 0)
        sent = False
        end = time.time() + 5
        while time.time() < end:
            import select
            r, _, _ = select.select([src.stdout], [], [], 0.2)
            if r:
                out = src.stdout.readline()
                if out.startswith("sent"):
                    sent = True
                    break
        return sent
    finally:
        try:
            src.stdin.close()
        except OSError:
            pass
        try:
            src.wait(5)
        except subprocess.TimeoutExpired:
            src.kill()


recover()
ok = go_root()
drop_files = [os.path.join(DROP, "drop-a.txt"), os.path.join(DROP, "dropdir")]
sent = ok and drag_drop(drop_files)
notice = sent and wait_for(lambda: "Dropped: 1 file and 1 folder" in text(pid) and "Upload here" in buttons(pid), 10)
check("drag and drop: a file and a folder dragged from another program (XDND over XTest) are announced by name; nothing sent yet",
      bool(notice) and "drop-a.txt" in text(pid) and "dropdir" in text(pid)
      and not any(k.startswith(("drop-a.txt", "dropdir/")) for k in current(state()[0])),
      f"source asked for data: {sent} | {text(pid)[-300:]}")
ok = notice and press(pid, "Upload here") and wait_for(upload_done, 60)
kd, _ = state()
check("drag and drop: Upload here runs the upload batch into the folder shown; the folder keeps its structure",
      bool(ok) and content(kd, "drop-a.txt") == "dropped file a\n" and content(kd, "dropdir/inner/z.txt") == "inside a dropped folder\n"
      and "2 files: 2 uploaded" in upload_summary(), upload_summary())
dismiss_upload()
sent = drag_drop(drop_files)
ok = sent and wait_for(lambda: "Upload here" in buttons(pid), 10) and press(pid, "Upload here") and wait_for(upload_done, 60)
kd2, _ = state()
check("drag and drop: the same drop again skips both (existing objects are never replaced)",
      bool(ok) and "2 files: 0 uploaded, 2 skipped" in upload_summary() and len(kd2["drop-a.txt"]["versions"]) == 1,
      upload_summary())
dismiss_upload()
sent = drag_drop([os.path.join(DROP, "drop-a.txt"), os.path.join(DROP, "drop-b.txt")])
notice = sent and wait_for(lambda: "Dropped: 2 files" in text(pid), 10)
ok = notice and press(pid, "Ignore") and wait_for(lambda: "Upload here" not in buttons(pid), 5)
time.sleep(1)
check("drag and drop: two files are announced; Ignore uploads nothing",
      bool(ok) and "drop-b.txt" not in current(state()[0]), text(pid)[-200:])
keys_still_work("a drop from another program")

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

for n in not_tested:
    print(f"NOT TESTED  {n}", flush=True)
