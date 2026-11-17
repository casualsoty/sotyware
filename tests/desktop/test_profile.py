"""Test profiles for the Linux desktop drivers (ADR-027).

A desktop test never runs sotyware against the user's own data. It runs a
test-profile build (Cargo feature `test-profile`), which refuses to start
unless SOTYWARE_TEST_PROFILE names a directory holding a marker this module
writes, keeps every database, its webview data and its credentials inside
that directory (credentials under service com.sotyware.desktop.test and a
per-profile nonce), and writes an acknowledgement once its stores are open.

`verify_ack` compares that acknowledgement with what the driver set up and
launched: same run, nonce and process id, the test identifier, and every
path inside the profile. A driver calls it right after each launch and
before it touches the window; anything else raises IsolationError and the
driver stops. `remove` deletes a profile only if it is one this module
created (under the given parent, with the same nonce in its marker).

Only the standard library; `python3 -m unittest tests/desktop/test_profile_test.py`
tests the helpers without a display.
"""

import hashlib
import json
import os
import secrets
import shutil
import time

ENV = "SOTYWARE_TEST_PROFILE"
MARKER_FILE = "sotyware-test-profile.json"
ACK_FILE = "sotyware-test-profile-ack.json"
# The databases the app reports in its acknowledgement (storage.rs).
STORES = ("preferences.sqlite3", "cloud-accounts.sqlite3", "cloud-places.sqlite3",
          "operations-journal.sqlite3", "backup.sqlite3", "account.sqlite3",
          "updates.sqlite3", "window-state.sqlite3", "storage-usage.sqlite3",
          "folder-sync.sqlite3")
PURPOSE = "sotyware-desktop-test-profile"
BUILD_MARKER = b"sotyware-test-profile-build"
IDENTIFIER = "com.sotyware.desktop.test"
NORMAL_IDENTIFIER = "com.sotyware.desktop"
CREDENTIAL_SERVICE = "com.sotyware.desktop.test"
EXIT_REFUSED = 3


class IsolationError(Exception):
    """The app is not provably running in this run's test profile."""


class Profile:
    def __init__(self, parent, root, run, nonce):
        self.parent = parent
        self.root = root
        self.run = run
        self.nonce = nonce

    @property
    def data_dir(self):
        return os.path.join(self.root, "data")

    def db(self, name):
        """A database file of the app inside this profile."""
        return os.path.join(self.data_dir, name)

    @property
    def credential_user_prefix(self):
        return f"{self.nonce}/"

    def credential_user(self, account_id):
        """The Secret Service `username` attribute of an account's entry."""
        return f"{self.nonce}/cloud-account/{account_id}"

    def env(self, base=None):
        e = dict(os.environ if base is None else base)
        e[ENV] = self.root
        return e


def is_test_profile_build(binary):
    with open(binary, "rb") as f:
        return BUILD_MARKER in f.read()


def require_test_profile_build(binary):
    if not is_test_profile_build(binary):
        raise IsolationError(
            f"{binary} is not a test-profile build; build it with "
            "npm run tauri build -- --no-bundle --features test-profile -- --target-dir target/test-profile")


def create(parent, run):
    """A new, empty profile directory `<parent>/<run>` with its marker."""
    if not run or not all(c.isalnum() or c in "-_" for c in run) or len(run) > 64:
        raise ValueError(f"bad run id {run!r}")
    parent = os.path.realpath(parent)
    root = os.path.join(parent, run)
    os.makedirs(root, mode=0o700)  # fails if it exists: never reuse a profile
    nonce = secrets.token_hex(16)
    with open(os.path.join(root, MARKER_FILE), "w", encoding="utf-8") as f:
        json.dump({"format": 1, "purpose": PURPOSE, "run": run, "nonce": nonce}, f)
    return Profile(parent, root, run, nonce)


def _within(path, base):
    path, base = os.path.realpath(path), os.path.realpath(base)
    return path == base or path.startswith(base + os.sep)


def check_ack(profile, ack, pid):
    """Raises IsolationError unless `ack` (parsed JSON) is this launch's."""
    def need(cond, what):
        if not cond:
            raise IsolationError(f"acknowledgement: {what}")
    need(isinstance(ack, dict), "not an object")
    need(ack.get("format") == 1 and ack.get("purpose") == PURPOSE, "wrong format or purpose")
    need(ack.get("build") == BUILD_MARKER.decode(), "not from a test-profile build")
    need(ack.get("run") == profile.run, f"run {ack.get('run')!r}, expected {profile.run!r}")
    need(ack.get("nonce") == profile.nonce, "nonce differs from this profile's")
    need(ack.get("pid") == pid, f"pid {ack.get('pid')}, launched {pid}")
    need(ack.get("identifier") == IDENTIFIER, f"identifier {ack.get('identifier')!r}")
    need(ack.get("identifier") != NORMAL_IDENTIFIER, "the app's own identifier")
    need(ack.get("credential_service") == CREDENTIAL_SERVICE, "credential service")
    need(ack.get("credential_target_prefix") == f"{CREDENTIAL_SERVICE}/{profile.nonce}/",
         "credential prefix")
    root = os.path.realpath(profile.root)
    need(os.path.realpath(str(ack.get("root"))) == root, f"root {ack.get('root')!r}")
    stores = list(ack.get("stores") or [])
    paths = [ack.get("data_dir"), ack.get("webview_dir")] + stores
    need(len(stores) == len(STORES), f"{len(STORES)} stores")
    need(sorted(os.path.basename(str(s)) for s in stores) == sorted(STORES),
         f"stores {stores!r}")
    for p in paths:
        need(isinstance(p, str) and _within(p, root), f"{p!r} is outside the profile")


def verify_ack(profile, pid, timeout=30, alive=None):
    """Waits for this launch's acknowledgement and checks it. `alive()`,
    if given, stops the wait early when the process has ended."""
    path = os.path.join(profile.root, ACK_FILE)
    deadline = time.time() + timeout
    last = None
    while time.time() < deadline:
        try:
            with open(path, encoding="utf-8") as f:
                ack = json.load(f)
            if ack.get("pid") == pid:
                check_ack(profile, ack, pid)
                return ack
            last = f"acknowledgement is from pid {ack.get('pid')}, not {pid}"
        except FileNotFoundError:
            last = "no acknowledgement yet"
        except (ValueError, OSError) as e:
            last = f"acknowledgement unreadable: {e}"
        if alive is not None and not alive():
            raise IsolationError(f"the app ended before acknowledging its profile ({last})")
        time.sleep(0.1)
    raise IsolationError(f"no acknowledgement from pid {pid} in {timeout} s ({last})")


def clear_ack(profile):
    """Removes an earlier launch's acknowledgement (inside the profile only)."""
    path = os.path.join(profile.root, ACK_FILE)
    if not _within(path, profile.root):
        raise IsolationError(f"{path} is outside the profile")
    try:
        os.remove(path)
    except FileNotFoundError:
        pass


def remove(profile):
    """Deletes the profile, and nothing else: only a directory directly
    under the parent it was created in, whose marker holds its nonce."""
    root = os.path.realpath(profile.root)
    if os.path.dirname(root) != os.path.realpath(profile.parent) or os.path.basename(root) != profile.run:
        raise IsolationError(f"refusing to remove {root}: not this run's profile directory")
    try:
        with open(os.path.join(root, MARKER_FILE), encoding="utf-8") as f:
            marker = json.load(f)
    except (OSError, ValueError) as e:
        raise IsolationError(f"refusing to remove {root}: no readable marker ({e})")
    if marker.get("nonce") != profile.nonce or marker.get("purpose") != PURPOSE:
        raise IsolationError(f"refusing to remove {root}: the marker is not this run's")
    shutil.rmtree(root)


def normal_profile_dirs(home):
    """The app's own folders for a user whose home is `home` (XDG defaults)."""
    return [os.path.join(home, ".local", "share", NORMAL_IDENTIFIER),
            os.path.join(home, ".config", NORMAL_IDENTIFIER),
            os.path.join(home, ".cache", NORMAL_IDENTIFIER)]


def fingerprint(dirs):
    """Names, sizes, modification times and SHA-256 of every file under
    `dirs` (read only), to show a run changed nothing there."""
    out = []
    for d in dirs:
        if not os.path.exists(d):
            out.append((d, "absent"))
            continue
        for base, subdirs, files in os.walk(d):
            subdirs.sort()
            for name in sorted(files):
                p = os.path.join(base, name)
                try:
                    st = os.lstat(p)
                    h = hashlib.sha256()
                    if os.path.isfile(p) and not os.path.islink(p):
                        with open(p, "rb") as f:
                            for chunk in iter(lambda: f.read(1 << 20), b""):
                                h.update(chunk)
                    out.append((p, st.st_size, st.st_mtime_ns, h.hexdigest()))
                except OSError as e:
                    out.append((p, f"unreadable: {e}"))
    return out
