"""Unit tests of tests/desktop/test_profile.py (ADR-027): no display, no app.

Run: python3 -m unittest tests/desktop/test_profile_test.py
"""

import json
import os
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import test_profile as tp  # noqa: E402


class ProfileTest(unittest.TestCase):
    def setUp(self):
        self.parent = tempfile.mkdtemp(prefix="profile-test-")
        self.addCleanup(shutil.rmtree, self.parent, True)

    def ack_for(self, p, launched, **changes):
        root = os.path.realpath(p.root)
        ack = {
            "format": 1, "purpose": tp.PURPOSE, "build": tp.BUILD_MARKER.decode(),
            "run": p.run, "nonce": p.nonce, "pid": launched, "identifier": tp.IDENTIFIER,
            "root": root, "data_dir": os.path.join(root, "data"),
            "webview_dir": os.path.join(root, "webview"),
            "credential_service": tp.CREDENTIAL_SERVICE,
            "credential_target_prefix": f"{tp.CREDENTIAL_SERVICE}/{p.nonce}/",
            "stores": [os.path.join(root, "data", n) for n in
                       ("preferences.sqlite3", "cloud-accounts.sqlite3", "cloud-places.sqlite3")],
        }
        ack.update(changes)
        return ack

    def test_create_writes_a_marker_with_a_fresh_nonce(self):
        a, b = tp.create(self.parent, "r1"), tp.create(self.parent, "r2")
        self.assertNotEqual(a.nonce, b.nonce)
        self.assertRegex(a.nonce, r"^[0-9a-f]{32}$")
        with open(os.path.join(a.root, tp.MARKER_FILE)) as f:
            m = json.load(f)
        self.assertEqual(m, {"format": 1, "purpose": tp.PURPOSE, "run": "r1", "nonce": a.nonce})
        self.assertEqual(a.env({})[tp.ENV], a.root)
        self.assertEqual(a.credential_user("x"), f"{a.nonce}/cloud-account/x")

    def test_a_profile_is_never_reused(self):
        tp.create(self.parent, "r1")
        with self.assertRaises(FileExistsError):
            tp.create(self.parent, "r1")

    def test_bad_run_ids_are_refused(self):
        for run in ["", "a/b", "..", "x y", "a" * 65]:
            with self.assertRaises(ValueError):
                tp.create(self.parent, run)

    def test_a_matching_acknowledgement_passes(self):
        p = tp.create(self.parent, "r")
        tp.check_ack(p, self.ack_for(p, 42), 42)

    def test_every_mismatch_is_refused(self):
        p = tp.create(self.parent, "r")
        other = tempfile.mkdtemp(prefix="elsewhere-")
        self.addCleanup(shutil.rmtree, other, True)
        home_normal = os.path.join(other, ".local", "share", tp.NORMAL_IDENTIFIER)
        bad = [
            {"pid": 43},
            {"nonce": "0" * 32},
            {"run": "other"},
            {"identifier": tp.NORMAL_IDENTIFIER},
            {"build": "x"},
            {"purpose": "x"},
            {"format": 2},
            {"credential_service": tp.NORMAL_IDENTIFIER},
            {"credential_target_prefix": f"{tp.NORMAL_IDENTIFIER}/"},
            {"root": other},
            {"data_dir": home_normal},
            {"webview_dir": os.path.join(p.root, "..", "elsewhere")},
            {"stores": [os.path.join(home_normal, "cloud-accounts.sqlite3")] * 3},
            {"stores": []},
        ]
        for change in bad:
            with self.subTest(change=change), self.assertRaises(tp.IsolationError):
                tp.check_ack(p, self.ack_for(p, 42, **change), 42)

    def test_verify_waits_for_this_launch_and_stops_when_the_app_ends(self):
        p = tp.create(self.parent, "r")
        with self.assertRaises(tp.IsolationError) as e:
            tp.verify_ack(p, 7, timeout=0.3)
        self.assertIn("no acknowledgement", str(e.exception))
        # An earlier launch's acknowledgement does not count.
        with open(os.path.join(p.root, tp.ACK_FILE), "w") as f:
            json.dump(self.ack_for(p, 6), f)
        with self.assertRaises(tp.IsolationError):
            tp.verify_ack(p, 7, timeout=0.3)
        with self.assertRaises(tp.IsolationError) as e:
            tp.verify_ack(p, 7, timeout=5, alive=lambda: False)
        self.assertIn("ended before", str(e.exception))
        with open(os.path.join(p.root, tp.ACK_FILE), "w") as f:
            json.dump(self.ack_for(p, 7), f)
        self.assertEqual(tp.verify_ack(p, 7, timeout=1)["pid"], 7)
        tp.clear_ack(p)
        self.assertFalse(os.path.exists(os.path.join(p.root, tp.ACK_FILE)))

    def test_remove_deletes_only_this_runs_profile(self):
        p = tp.create(self.parent, "r")
        keep = os.path.join(self.parent, "keep.txt")
        open(keep, "w").close()
        # A forged profile object pointing elsewhere is refused.
        forged = tp.Profile(self.parent, self.parent, os.path.basename(self.parent), p.nonce)
        with self.assertRaises(tp.IsolationError):
            tp.remove(forged)
        outside = tempfile.mkdtemp(prefix="outside-")
        self.addCleanup(shutil.rmtree, outside, True)
        with self.assertRaises(tp.IsolationError):
            tp.remove(tp.Profile(self.parent, outside, os.path.basename(outside), p.nonce))
        # A directory with another nonce's marker is refused.
        q = tp.create(self.parent, "q")
        with self.assertRaises(tp.IsolationError):
            tp.remove(tp.Profile(self.parent, q.root, "q", p.nonce))
        # A profile whose marker was removed is refused.
        os.remove(os.path.join(q.root, tp.MARKER_FILE))
        with self.assertRaises(tp.IsolationError):
            tp.remove(q)
        # A symbolic link named like the run, pointing out of the parent.
        link_parent = tempfile.mkdtemp(prefix="links-")
        self.addCleanup(shutil.rmtree, link_parent, True)
        os.symlink(p.root, os.path.join(link_parent, "r"))
        with self.assertRaises(tp.IsolationError):
            tp.remove(tp.Profile(link_parent, os.path.join(link_parent, "r"), "r", p.nonce))
        self.assertTrue(os.path.isdir(p.root))
        tp.remove(p)
        self.assertFalse(os.path.exists(p.root))
        self.assertTrue(os.path.exists(keep) and os.path.isdir(q.root))

    def test_the_normal_profile_is_named_by_the_real_identifier_only(self):
        dirs = tp.normal_profile_dirs("/home/u")
        self.assertIn("/home/u/.local/share/com.sotyware.desktop", dirs)
        self.assertTrue(all(tp.IDENTIFIER not in d for d in dirs))

    def test_fingerprint_sees_a_change(self):
        d = os.path.join(self.parent, "n")
        os.makedirs(d)
        with open(os.path.join(d, "a"), "w") as f:
            f.write("1")
        before = tp.fingerprint([d, os.path.join(self.parent, "absent")])
        self.assertEqual(before, tp.fingerprint([d, os.path.join(self.parent, "absent")]))
        with open(os.path.join(d, "a"), "w") as f:
            f.write("2")
        self.assertNotEqual(before, tp.fingerprint([d, os.path.join(self.parent, "absent")]))

    def test_build_marker_detection(self):
        b = os.path.join(self.parent, "bin")
        with open(b, "wb") as f:
            f.write(b"\0" * 100)
        self.assertFalse(tp.is_test_profile_build(b))
        with self.assertRaises(tp.IsolationError):
            tp.require_test_profile_build(b)
        with open(b, "ab") as f:
            f.write(tp.BUILD_MARKER)
        tp.require_test_profile_build(b)


if __name__ == "__main__":
    unittest.main()
