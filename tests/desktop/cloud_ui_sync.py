# The folder sync phase of cloud_ui.py (ADR-043, ADR-046), run by it with
# exec() in its own namespace (pid, p, env, log, check, press, text, nodes,
# by_role, buttons, fill, wait_for, add, close_windows, PROFILE, BIN, ...).
# Kept in its own file only for length.
#
# Drives the bucket's Sync tab of the release binary (a test-profile build,
# ADR-027) against the in-memory versioned S3 server of serve_sync_for_ui
# (src-tauri/src/cloud/test_server/sync_ui.rs; cloud-ui.sh phase `sync`),
# on 127.0.0.1 with a made-up key. The server plays a second computer that
# uses the same bucket (`put`, `delete`), and its `dump` says what is
# really stored. Every check reads the disk or the server, not only the
# page: the folder is chosen in the native GTK folder dialog, the first
# pass brings the bucket's files down, a new local file goes up, a file
# added elsewhere comes down, a local rename moves the object, a file
# deleted elsewhere goes to the sync trash, a file changed on both sides
# is kept twice, sotyware is quit and started again (the startup
# connection check reaches the server with no click, ADR-046, and the
# sync goes on), the interval select, focused once, follows several keys
# in a row (a burst, then half a second apart), each change saved and in
# use, and turning sync off deletes nothing on either side. The server
# fails at the end if a real version was ever deleted for good.
import glob
import hashlib
import json
import re

import test_profile as tp  # cloud_ui.py put this folder on sys.path

exec(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "cloud_ui_gtk.py"), encoding="utf-8").read())

SYNC = os.path.join(os.environ["HOME"], f"sync-folder-{RUN}")
os.makedirs(SYNC)
TITLE = f"Choose the folder to sync with {BUCKET}"


def control(*commands):
    tmp = CONTROL + ".ctl.tmp"
    with open(tmp, "w", encoding="utf-8") as fh:
        fh.write("\n".join(commands) + "\n")
    os.rename(tmp, CONTROL + ".ctl")
    return bool(wait_for(lambda: not os.path.exists(CONTROL + ".ctl"), 30))


def server():
    """(keys, operations): {key: {"current": {"size", "sha256"} | None,
    "versions", "markers"}}, {operation: count}."""
    path = CONTROL + ".state.json"
    if os.path.exists(path):
        os.remove(path)
    if not control("dump") or not wait_for(lambda: os.path.exists(path), 10):
        raise SystemExit("the sync server did not answer dump")
    with open(path, encoding="utf-8") as fh:
        s = json.load(fh)
    os.remove(path)
    return s["keys"], s["operations"]


def remote_text_sha(text_):
    return hashlib.sha256(text_.encode()).hexdigest()


def remote_current():
    keys, _ = server()
    return {k: v["current"]["sha256"] for k, v in keys.items() if v["current"] is not None}


def local_files():
    """{relative path: SHA-256} of the sync folder, without .sotyware-sync."""
    out = {}
    for base, dirs, files in os.walk(SYNC):
        dirs[:] = [d for d in dirs if d != ".sotyware-sync"]
        for f in files:
            path = os.path.join(base, f)
            # sotyware may move the file (into place, into the sync trash)
            # between the walk and the read: skip it, the next poll sees it.
            try:
                with open(path, "rb") as fh:
                    out[os.path.relpath(path, SYNC).replace(os.sep, "/")] = hashlib.sha256(fh.read()).hexdigest()
            except FileNotFoundError:
                continue
    return out


def write_local(rel, content):
    path = os.path.join(SYNC, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(content)


def sync_now():
    """Presses Sync now when it is enabled (a pass may be running)."""
    wait_for(lambda: press_enabled("Sync now"), 20)


def press_enabled(name):
    for n, node in by_role(pid, Atspi.Role.PUSH_BUTTON):
        if n.strip() == name and node.get_state_set().contains(Atspi.StateType.ENABLED):
            return Atspi.Action.do_action(node, 0)
    return False


def until(pred, timeout=90, nudge=True):
    """Waits for `pred`, pressing Sync now every 10 s so a change made at
    the server does not wait for the listing interval (5 minutes at
    least)."""
    end = time.time() + timeout
    while time.time() < end:
        v = pred()
        if v:
            return v
        if nudge:
            sync_now()
        wait_for(pred, 10)
    return pred()


def open_sync_tab():
    for n, node in by_role(pid, Atspi.Role.PAGE_TAB):
        if n.strip() == "Sync":
            return Atspi.Action.do_action(node, 0)
    print(f"  no Sync tab; tabs: {[n for n, _ in by_role(pid, Atspi.Role.PAGE_TAB)]}", flush=True)
    return False


def relaunch():
    """Quits sotyware as the window's close does in dashboard-only mode,
    then starts it again in the same profile."""
    global p, pid
    # ADR-027: the restart is a launch of its own; it starts only a
    # test-profile build, in this run's profile, and waits for its
    # acknowledgement before anything is clicked.
    tp.require_test_profile_build(BIN)
    close_windows(pid)
    try:
        code = p.wait(timeout=60)
    except subprocess.TimeoutExpired:
        return False, "sotyware did not quit within 60 s"
    tp.clear_ack(PROFILE)
    with open(log, "ab") as f:
        p = subprocess.Popen([BIN], env=env, stdout=f, stderr=subprocess.STDOUT)
    pid = p.pid
    try:
        tp.verify_ack(PROFILE, pid, timeout=30, alive=lambda: p.poll() is None)
    except tp.IsolationError as e:
        p.kill()
        raise SystemExit(f"the restarted app is not provably isolated: {e}")
    return True, f"quit with exit code {code}"


SEED = {"welcome.txt": "welcome from the bucket\n", "docs/plan.txt": "the plan, written elsewhere\n"}

ok = add("Synced", BUCKET, KEY1, SECRET1, ENDPOINT)
check("add an account for the local versioned server",
      bool(ok and wait_for(lambda: "Added Synced" in text(pid), 30)), text(pid)[-300:])
ok = press(pid, "Open bucket") and wait_for(lambda: "welcome.txt" in text(pid), 30)
check("Open bucket lists the seeded files", bool(ok), text(pid)[-300:])
ok = open_sync_tab() and wait_for(lambda: "Choose folder…" in [b.strip() for b in buttons(pid)], 15)
check("the Sync tab offers Choose folder… for a bucket that is not synced", bool(ok), text(pid)[-300:])

# ---- choose the folder in the native dialog, then start ----
ok = press(pid, "Choose folder…") and choose_folder(TITLE, SYNC)
check("the GTK folder dialog chooses the empty sync folder", bool(ok))
ok = wait_for(lambda: "Sync this folder?" in text(pid) and SYNC in text(pid), 20)
check("the page asks to confirm, showing the chosen folder and that it is empty",
      bool(ok) and "Empty" in text(pid), text(pid)[-500:])
ok = press(pid, "Start sync") and wait_for(lambda: "Turn sync off…" in [b.strip() for b in buttons(pid)], 30)
check("Start sync: the tab shows the running sync", bool(ok), text(pid)[-400:])
marker = os.path.join(SYNC, ".sotyware-sync", "root.json")
check("the folder is marked as a sync folder (.sotyware-sync/root.json)", wait_for(lambda: os.path.exists(marker), 30))

# ---- the first pass brings the bucket down ----
want = {k: remote_text_sha(v) for k, v in SEED.items()}
got = until(lambda: local_files() == want or None, 90)
check("the first pass brings both seeded files down, docs/plan.txt into its folder, byte for byte",
      got is not None, local_files())

# ---- a new local file goes up ----
write_local("notes.txt", "written on this computer\n")
sha_notes = remote_text_sha("written on this computer\n")
ok = until(lambda: remote_current().get("notes.txt") == sha_notes, 90)
check("a file created in the folder appears in the bucket with the same content", bool(ok), remote_current())

# ---- a file added elsewhere comes down ----
control("put from-elsewhere.txt added on another computer")
sha_else = remote_text_sha("added on another computer\n")
ok = until(lambda: local_files().get("from-elsewhere.txt") == sha_else, 90)
check("a file added to the bucket elsewhere comes down after Sync now", bool(ok), local_files())

# ---- a local rename moves the object ----
os.rename(os.path.join(SYNC, "notes.txt"), os.path.join(SYNC, "renamed.txt"))
ok = until(lambda: (lambda r: r.get("renamed.txt") == sha_notes and "notes.txt" not in r)(remote_current()), 90)
keys, _ = server()
check("renaming a file in the folder renames it in the bucket (old name a delete marker, not removed)",
      bool(ok) and keys.get("notes.txt", {}).get("markers", 0) >= 1 and keys["notes.txt"]["versions"] >= 1,
      {k: v for k, v in keys.items() if k in ("notes.txt", "renamed.txt")})

# ---- a file deleted elsewhere goes to the sync trash ----
control("delete welcome.txt")
ok = until(lambda: "welcome.txt" not in local_files(), 90)
# The move into the trash may still be finishing: wait for it.
trashed = wait_for(lambda: glob.glob(os.path.join(SYNC, ".sotyware-sync", "trash", "*", "welcome.txt")), 15) or []
trash_ok = bool(trashed) and hashlib.sha256(open(trashed[0], "rb").read()).hexdigest() == want["welcome.txt"]
check("a file deleted in the bucket elsewhere leaves the folder and is kept in .sotyware-sync/trash/<day>/",
      bool(ok) and trash_ok, (local_files(), trashed))

# ---- changed on both sides: kept twice ----
local_edit, remote_edit = "edited here\n", "edited elsewhere"
control(f"put docs/plan.txt {remote_edit}")
write_local("docs/plan.txt", local_edit)
sha_local, sha_remote = remote_text_sha(local_edit), remote_text_sha(remote_edit + "\n")


def both_kept():
    loc, rem = local_files(), remote_current()
    names = set(loc) | set(rem)
    copies = [n for n in names if n.startswith("docs/plan (conflicted copy")]
    contents = set(loc.values()) | set(rem.values())
    return copies if copies and sha_local in contents and sha_remote in contents else None


copies = until(both_kept, 120)
check("a file changed on both sides is kept twice: both contents survive, one under a "
      "'plan (conflicted copy …)' name", bool(copies), (local_files(), remote_current()))

# ---- quit and start again: the startup check and the sync go on ----
_, ops_before = server()
heads_before = ops_before.get("HeadBucket", 0)
ok, why = relaunch()
check("quit and start sotyware again in the same profile", ok, why)
ok = wait_for(lambda: server()[1].get("HeadBucket", 0) > heads_before, 60)
check("at startup the connection check reaches the bucket with no click (ADR-046)", bool(ok),
      (heads_before, server()[1]))
ok = link(pid, "Cloud Storage") and wait_for(lambda: "Connected" in text(pid), 30)
check("Cloud Storage shows the account Connected after the startup check", bool(ok), text(pid)[-300:])
write_local("after-restart.txt", "after the restart\n")
sha_after = remote_text_sha("after the restart\n")
ok = wait_for(lambda: remote_current().get("after-restart.txt") == sha_after, 120)
check("after the restart the sync goes on by itself: a new local file goes up with no click", bool(ok),
      remote_current())

# ---- turning sync off deletes nothing ----
ok = press(pid, "Open bucket") and wait_for(lambda: any(n.strip() == "Sync" for n, _ in
                                                        by_role(pid, Atspi.Role.PAGE_TAB)), 30)
ok = ok and open_sync_tab() and wait_for(lambda: "Turn sync off…" in [b.strip() for b in buttons(pid)], 20)
check("the Sync tab shows the sync after the restart", bool(ok), text(pid)[-300:])

# ---- several keys in a row on the interval select, focused once ----
# Each change is saved at once, one after another, and the select keeps the
# keyboard (until 2026-11-25 the tab disabled the select while a change was
# saved: a key sent meanwhile was lost, and WebKitGTK could take the focus
# from the select). Ends on 15 minutes, as it began.
INTERVAL = "Check the bucket for changes made elsewhere"
SYNC_DB = PROFILE.db("folder-sync.sqlite3")


def combo(label):
    for role, name, _, node in nodes(pid):
        if role == Atspi.Role.COMBO_BOX and (name == label or name.startswith(label + " ")):
            return node
    return None


def shown_option(node):
    """The option the <select> `node` shows (its SELECTED menu item)."""
    return next((i.get_name().strip() for i in find(node, Atspi.Role.MENU_ITEM)
                 if i.get_state_set().contains(Atspi.StateType.SELECTED)), None)


def focused(node):
    return node is not None and node.get_state_set().contains(Atspi.StateType.FOCUSED)


def saved_interval():
    """The interval in the sync's configuration in folder-sync.sqlite3."""
    try:
        with sqlite3.connect(f"file:{SYNC_DB}?mode=ro", uri=True) as db:
            rows = db.execute("SELECT data FROM syncs").fetchall()
    except sqlite3.Error:
        return None
    return json.loads(rows[0][0]).get("intervalMinutes") if len(rows) == 1 else None


def reported_interval():
    """The interval in use, as the tab writes Rust's live status."""
    m = re.search(r"checked for changes made elsewhere every (\d+) minutes", text(pid))
    return int(m.group(1)) if m else None


def interval_state():
    shown = shown_option(interval) if interval is not None else None
    return f"select shows {shown!r}, focused {focused(interval)}, saved {saved_interval()}, in use {reported_interval()}"


def interval_at(minutes):
    return wait_for(lambda: shown_option(interval) == f"Every {minutes} minutes" and saved_interval() == minutes
                    and reported_interval() == minutes, 10)


interval = wait_for(lambda: combo(INTERVAL), 10)
focus_window(pid)
ready = interval is not None and shown_option(interval) == "Every 15 minutes" and Atspi.Component.grab_focus(
    interval) and wait_for(lambda: focused(interval), 3)
check("the interval select shows Every 15 minutes and takes the keyboard", bool(ready), interval_state())
for _ in range(2):
    key(keysym("Down"))
ok = ready and interval_at(60)
check("two Downs in one burst on it, focused once: Every 60 minutes, saved and in use, the select keeping the keyboard",
      bool(ok) and focused(interval), interval_state())
for i in range(3):
    if i:
        time.sleep(0.5)
    key(keysym("Up"))
ok = ready and interval_at(5)
check("three Ups half a second apart, still not focused again: Every 5 minutes, saved and in use",
      bool(ok) and focused(interval), interval_state())
key(keysym("Down"))
ok = ready and interval_at(15)
check("one more Down brings it back to Every 15 minutes, saved and in use", bool(ok) and focused(interval),
      interval_state())

local_before, remote_before = local_files(), remote_current()
ok = press(pid, "Turn sync off…") and wait_for(lambda: "Turn sync off" in [b.strip() for b in buttons(pid)], 10)
ok = ok and press(pid, "Turn sync off") and wait_for(lambda: "Choose folder…" in [b.strip() for b in buttons(pid)], 30)
check("Turn sync off: the tab offers Choose folder… again", bool(ok), text(pid)[-300:])
time.sleep(3)
check("turning sync off changed nothing in the folder or the bucket",
      local_files() == local_before and remote_current() == remote_before,
      (local_files(), remote_current()))
