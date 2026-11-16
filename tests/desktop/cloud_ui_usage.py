# The storage usage phase of cloud_ui.py (ADR-041), run by it with exec()
# in its own namespace (pid, check, press, text, nodes, by_role, buttons,
# fill, wait_for, windows_of, _WinAttrs, add, BUCKET, ENDPOINT, EXPECTED,
# ...). Kept in its own file only for length.
#
# Drives a bucket's "Usage & costs" tab of the release binary (a
# test-profile build, ADR-027) against the in-memory versioned S3 server of
# serve_usage_for_ui (src-tauri/src/cloud/test_server/usage_ui.rs;
# cloud-ui.sh phase `usage`): one versioned bucket with current files,
# older versions and a deleted file, on 127.0.0.1, made-up key. It measures
# the bucket and compares the figures on the page with what the server says
# a measurement finds (EXPECTED), changes the downloads the estimate
# assumes, measures again for the history, and checks at three window sizes
# that nothing on the Usage & costs and Snapshots & backups tabs reaches
# past the edges of its section or of the page's scroll area (<main>). The
# Estimated cost table once ran past its card (docs/status/status.md, "The
# cost table inside its card"), which no headless test can see: happy-dom
# lays nothing out. Without that fix this check fails (status.md, "The Usage
# & costs tab in the Linux desktop test").
X.XResizeWindow.argtypes = [ctypes.c_void_p, W, ctypes.c_uint, ctypes.c_uint]
X.XResizeWindow.restype = ctypes.c_int


def decimal_size(n):
    """src/format.ts formatDecimalSize: decimal units, as providers bill."""
    if n < 1000:
        return f"{n} bytes"
    value, unit = n / 1000, 0
    while value >= 1000 and unit < 4:
        value, unit = value / 1000, unit + 1
    digits = 2 if value < 10 else 1 if value < 100 else 0
    return f"{value:.{digits}f} {['kB', 'MB', 'GB', 'TB', 'PB'][unit]}"


def cost_rows():
    """The Estimated cost table, the only table shown on the tab: each row as
    (row header with its sentence, amount without white space). The amount's
    spacing follows the locale: "$ 0.00" here, under C.UTF-8."""
    out = []
    for role, _, _, node in nodes(pid):
        if role == Atspi.Role.TABLE_ROW and node.get_child_count() >= 2:
            head = same_text(cell_text(node.get_child_at_index(0)))
            amount = "".join(cell_text(node.get_child_at_index(1)).split())
            out.append((head, amount))
    return out


def usage_text():
    """The page's text from the usage section's heading to the first card,
    for failure details."""
    t = same_text(text(pid))
    start = t.find("Storage usage")
    return t[start:start + 700] if start >= 0 else t[-700:]


def extents(node):
    try:
        e = Atspi.Component.get_extents(node, Atspi.CoordType.SCREEN)
        return e.x, e.y, e.width, e.height
    except (GLib.Error, AttributeError, TypeError):
        return None


def tab(name):
    """Presses the bucket view's tab `name` (role tab)."""
    for n, node in by_role(pid, Atspi.Role.PAGE_TAB):
        if n.strip() == name:
            return Atspi.Action.do_action(node, 0)
    print(f"  no tab {name!r}; tabs: {[n for n, _ in by_role(pid, Atspi.Role.PAGE_TAB)]}", flush=True)
    return False


def section(name):
    """The page section labelled `name` (its heading has the same name and
    is skipped)."""
    for role, n, _, node in nodes(pid):
        if n == name and role not in (Atspi.Role.HEADING, Atspi.Role.LABEL, Atspi.Role.STATIC):
            return node
    return None


def scroll_area(node):
    """The page's scroll area above `node`: <main>, which keeps the window's
    width less the sidebar whatever its content needs, so anything past its
    edges has to be scrolled to sideways."""
    n = node
    while n is not None and n.get_role() not in (Atspi.Role.DOCUMENT_WEB, Atspi.Role.FRAME):
        try:
            attrs = n.get_attributes() or {}
        except GLib.Error:
            attrs = {}
        if attrs.get("tag") == "main" or attrs.get("xml-roles") == "main":
            return n
        n = n.get_parent()
    return None


def ancestors(node):
    """`node` and its ancestors up to the page, with their left edge and
    width, for failure details."""
    chain, n = [], node
    while n is not None and len(chain) < 12:
        e = extents(n)
        chain.append(f"{n.get_role_name()} {(n.get_name() or '')[:16]!r} {e[0] if e else '?'}+{e[2] if e else '?'}")
        if n.get_role() in (Atspi.Role.DOCUMENT_WEB, Atspi.Role.FRAME):
            break
        n = n.get_parent()
    return " < ".join(chain)


def past_edges(container, area):
    """The section `container` and every visible element in it whose box
    reaches past the section's left or right edge or past the scroll area
    `area` (1 px of rounding allowed): (box of the section, box of the
    area, the roles of the boxes compared, what reaches past). Both,
    because the engines differ: Chromium keeps the section's width and an
    unbreakable table spills out of its card; WebKitGTK widens the card,
    the section and the bucket view with it, up to <main>. VISIBLE, not
    SHOWING: content scrolled out of the window (the cost card, below the
    fold of a small window) counts too."""
    box, page = extents(container), extents(area) if area is not None else None
    if box is None or page is None:
        return box, page, [], ["the section or the page's scroll area has no extents"]
    left, right = max(box[0], page[0]), min(box[0] + box[2], page[0] + page[2])
    roles, bad = [], []
    if box[0] < page[0] - 1 or box[0] + box[2] > page[0] + page[2] + 1:
        bad.append(f"the section itself at x {box[0]}..{box[0] + box[2]}")

    def walk(n):
        try:
            for j in range(n.get_child_count()):
                c = n.get_child_at_index(j)
                if c is None:
                    continue
                e = extents(c) if c.get_state_set().contains(Atspi.StateType.VISIBLE) else None
                if e is not None and e[2] > 0 and e[3] > 0:
                    roles.append(c.get_role_name())
                    if e[0] < left - 1 or e[0] + e[2] > right + 1:
                        what = (c.get_name() or cell_text(c))[:50]
                        bad.append(f"{c.get_role_name()} {what!r} at x {e[0]}..{e[0] + e[2]}")
                walk(c)
        except GLib.Error:
            pass

    walk(container)
    return box, page, roles, bad


def dashboard_window():
    """The dashboard's X window: the process's largest mapped top-level one
    (only top-level windows carry _NET_WM_PID)."""
    shown = []
    for w in windows_of(pid):
        a = _WinAttrs()
        if X.XGetWindowAttributes(dpy, w, ctypes.byref(a)) and a.map_state == 2 and not a.override_redirect:
            shown.append((a.width * a.height, w))
    return max(shown)[1] if shown else None


def window_size():
    """The dashboard's own size, without the window manager's frame (what
    AT-SPI reports for the frame includes openbox's title bar)."""
    w, a = dashboard_window(), _WinAttrs()
    if w is None or not X.XGetWindowAttributes(dpy, w, ctypes.byref(a)):
        return None
    return a.width, a.height


def resize(width, height):
    """Resizes the dashboard through the window manager, as dragging its
    border would, and waits until the page has laid itself out again."""
    w = dashboard_window()
    if w is None:
        return False
    X.XResizeWindow(dpy, w, width, height)
    X.XSync(dpy, 0)
    ok = wait_for(lambda: window_size() == (width, height), 10)
    settled(SECTIONS[0])
    return bool(ok)


def settled(name):
    """Waits until section `name` keeps its box for a moment."""
    last = [None]

    def same():
        node = section(name)
        e = extents(node) if node is not None else None
        stable = e is not None and e == last[0]
        last[0] = e
        return stable

    wait_for(lambda: time.sleep(0.3) or same(), 10)


SECTIONS = ("Storage usage", "Snapshots and backups")


def layout(size_label):
    """Both tabs at the current window size: nothing past its section."""
    for tab_name, name in (("Usage & costs", SECTIONS[0]), ("Snapshots & backups", SECTIONS[1])):
        shown = tab(tab_name) and wait_for(lambda: section(name) is not None, 15)
        if shown:
            settled(name)
        else:
            named = [(r.value_nick, n) for r, n, _, _ in nodes(pid) if n == name]
            print(f"  no section named {name!r}; nodes with that name: {named}", flush=True)
        node = section(name) if shown else None
        box, page, roles, bad = (past_edges(node, scroll_area(node)) if node is not None
                                 else (None, None, [], [f"the {tab_name} tab did not open"]))
        counts = {r: roles.count(r) for r in sorted(set(roles))}
        print(f"  {size_label}, {tab_name}: section {box}, scroll area {page}, {len(roles)} boxes compared {counts}",
              flush=True)
        # Not vacuous: the usage tab's comparison includes the cost table's
        # row headers (Storage, Downloads, API calls, the total), whose
        # sentences once ran past the card; the snapshots tab has its cards.
        enough = (sum(n for r, n in counts.items() if "row header" in r) >= 4 if name == SECTIONS[0]
                  else len(roles) >= 5)
        check(f"{size_label}: nothing on the {tab_name} tab reaches past its section or the page",
              bool(shown) and enough and not bad,
              f"section {box}, scroll area {page}, {len(roles)} boxes compared; past the edges: {bad[:8]}; "
              f"{ancestors(node) if node is not None else ''}")
    tab("Usage & costs")
    wait_for(lambda: "Measure again" in [b.strip() for b in buttons(pid)], 10)


ok = add("Usage", BUCKET, KEY1, SECRET1, ENDPOINT)
check("add an account for the local versioned server",
      bool(ok and wait_for(lambda: "Added Usage" in text(pid))), text(pid))
ok = press(pid, "Open bucket") and wait_for(lambda: "notes.txt" in text(pid), 30)
check("Open bucket lists the bucket's root on the Files tab", bool(ok), text(pid)[-300:])

ok = tab("Usage & costs") and wait_for(lambda: "Measure now" in [b.strip() for b in buttons(pid)], 15)
t = text(pid)
check("the Usage & costs tab: not measured yet, says why it lists the bucket",
      bool(ok) and "No measurement for Usage yet." in t and "lists the bucket to measure it" in t, t[-400:])

ok = press(pid, "Measure now") and wait_for(lambda: "Measure again" in [b.strip() for b in buttons(pid)], 60)
t = same_text(text(pid))
check("Measure now: the measurement finishes with one listing of versions and one of unfinished uploads",
      bool(ok) and "Measured just now" in t and "with 2 listing requests" in t, t[-400:])

stored = EXPECTED["bytes"] + EXPECTED["noncurrentBytes"] + EXPECTED["incompleteBytes"]
want = {
    "the Stored tile": f"Stored {decimal_size(stored)} everything kept in the bucket",
    "the Files tile": f"Files {EXPECTED['objects']} {EXPECTED['noncurrentVersions']} older versions",
    "current files": f"Current files {EXPECTED['objects']} {decimal_size(EXPECTED['bytes'])}",
    "older versions": (f"Older versions and deleted files {EXPECTED['noncurrentVersions']} "
                       f"{decimal_size(EXPECTED['noncurrentBytes'])}"),
    "unfinished uploads": f"Unfinished uploads {EXPECTED['incompleteUploads']} {decimal_size(EXPECTED['incompleteBytes'])}",
    "delete markers": f"{EXPECTED['deleteMarkers']} delete marker (they store nothing)",
}
flat = same_text(t)
missing = {k: v for k, v in want.items() if v not in flat}
check("the figures are what the server says a measurement finds (tiles, parts, delete markers)",
      not missing, f"missing {missing}; page from the section: {usage_text()}")

rows_ = wait_for(lambda: (r := cost_rows()) and len(r) == 4 and r, 10) or cost_rows()
heads = (f"Storage {decimal_size(stored)} stored, the first 10.0 GB free, at USD 6.95 per TB for 30 days.",
         f"Downloads (egress) 0 bytes assumed for 30 days; {decimal_size(3 * stored)} free (3 times the stored data)",
         "API calls Class A, B and C API calls (every call sotyware makes) are free for pay-as-you-go accounts",
         "Total for 30 days, without taxes")
check("the estimate: Backblaze B2's price list, each component with its sentence and $0.00, the total",
      len(rows_) == 4 and all(h.startswith(w) and a == "$0.00" for (h, a), w in zip(rows_, heads)), str(rows_))


def open_details(name, shows):
    """Opens the <details> whose summary is `name`: presses the first node
    of that name with an action until `shows` is on the page."""
    for r, n, _, node in nodes(pid):
        if n.strip() != name or node.get_action_iface() is None:
            continue
        try:
            Atspi.Action.do_action(node, 0)
        except GLib.Error:
            continue
        if wait_for(lambda: shows in text(pid), 5):
            return True
    return False


# "How this is estimated" open, so its lists are laid out too.
ok = open_details("How this is estimated", "Price list: Backblaze B2, pay as you go, checked")
check("How this is estimated opens: the assumptions and the price list's date", ok, text(pid)[-400:])

downloads = [n for n, _ in by_role(pid, *ENTRY_ROLES) if n.startswith("Downloads you expect per 30 days")]
ok = bool(downloads) and fill(pid, downloads[0], "700")
# 700 GB less 3 times the stored data, at $0.01 per GB: $7.00 (6.9999…).
ok = ok and wait_for(lambda: [(h.split(";")[0], a) for h, a in cost_rows()][1::2] == [
    ("Downloads (egress) 700 GB assumed for 30 days", "$7.00"), ("Total for 30 days, without taxes", "$7.00")], 15)
check("700 GB of expected downloads: egress beyond 3 times the stored data is estimated, the total follows",
      bool(ok), str(cost_rows()))

ok = press(pid, "Measure again") and wait_for(
    lambda: any(r == Atspi.Role.IMAGE and n.startswith("Stored data in 2 measurements")
                for r, n, _, _ in nodes(pid)), 60)
check("Measure again: the history draws both measurements", bool(ok), text(pid)[-400:])

size = window_size()
print(f"  window at first launch: {size}", flush=True)
layout(f"{size[0]} x {size[1]} (first launch)" if size else "first launch")
for width, height, label in ((1240, 820, "wide"), (700, 540, "narrow, navigation as an overlay")):
    resized = resize(width, height)
    check(f"the window resizes to {width} x {height}", resized, str(window_size()))
    if resized:
        layout(f"{width} x {height} ({label})")
