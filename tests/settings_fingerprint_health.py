#!/usr/bin/env python3
"""Regression tests for the settings GUI's fingerprint-unlock health check.

The row used to say "Attivo" whenever the lock tree was patched on disk, even
when DMS had loaded the unpatched copy, the runtime marker was missing, or the
daemon was down. Each failure must now name itself.
"""

import importlib.machinery
import importlib.util
import os
import tempfile
import unittest
from pathlib import Path


os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

MODULE_PATH = Path(__file__).parents[1] / "desktop/bin/syauth-settings"
loader = importlib.machinery.SourceFileLoader("syauth_settings", str(MODULE_PATH))
spec = importlib.util.spec_from_loader(loader.name, loader)
settings = importlib.util.module_from_spec(spec)
loader.exec_module(settings)


def make_tree(root, patched=True):
    tree = root / "hash1234"
    lock = tree / "Modules" / "Lock"
    lock.mkdir(parents=True)
    content = lock / "LockScreenContent.qml"
    pam = lock / "Pam.qml"
    if patched:
        content.write_text(
            "function requestPhoneUnlock() {}\n"
            "FileView { id: syauthUnlockReady }\n"
            "Process { id: syauthUnlockProcess }\n"
        )
        pam.write_text("property bool fprintSuppressedByPrimaryPam\n")
    else:
        content.write_text("MouseArea {}\n")
        pam.write_text("property bool lockFingerprintReady\n")
    return tree


class FingerprintHealthTests(unittest.TestCase):
    def test_master_off_is_named(self):
        ok, reason = settings.fingerprint_unlock_health("off", 1)
        self.assertFalse(ok)
        self.assertEqual(reason, "DeskUnlock disattivato")

    def test_no_bond_is_named(self):
        ok, reason = settings.fingerprint_unlock_health("on", 0)
        self.assertFalse(ok)
        self.assertEqual(reason, "Nessun telefono")

    def test_unpatched_tree_is_named(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root, patched=False)
            ok, reason = settings.fingerprint_unlock_health("on", 1, runtime=root)
        self.assertFalse(ok)
        self.assertEqual(reason, "Lock screen non adattato")

    def test_patched_tree_without_marker_is_named(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root, patched=True)
            ok, reason = settings.fingerprint_unlock_health(
                "on",
                1,
                runtime=root,
                marker=root / "unlock-ready",
                loaded_marker=root / "dms-lock-loaded",
            )
        self.assertFalse(ok)
        self.assertEqual(reason, "Sblocco telefono non armato")

    def test_loaded_marker_controls_attivo(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            tree = make_tree(root, patched=True)
            ready = root / "unlock-ready"
            ready.write_text("1")
            loaded = root / "dms-lock-loaded"

            # Patched on disk, marker present, but DMS has not loaded it yet.
            ok, reason = settings.fingerprint_unlock_health(
                "on", 1, runtime=root, marker=ready, loaded_marker=loaded
            )
            self.assertFalse(ok)
            self.assertEqual(reason, "Riavvia DMS")

            # The patch recorded that the running DMS loaded this tree, but the
            # phone is not connected -> not "Attivo".
            loaded.write_text(tree.name)
            ok, reason = settings.fingerprint_unlock_health(
                "on", 1, runtime=root, marker=ready, loaded_marker=loaded, present=False
            )
            self.assertFalse(ok)
            self.assertEqual(reason, "Telefono non connesso")

            # Phone connected -> Attivo.
            ok, reason = settings.fingerprint_unlock_health(
                "on", 1, runtime=root, marker=ready, loaded_marker=loaded, present=True
            )
            self.assertTrue(ok)
            self.assertEqual(reason, "Attivo")

    def test_lock_screen_adapted_returns_the_patched_tree(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            tree = make_tree(root, patched=True)
            self.assertEqual(settings.lock_screen_adapted(root), tree)

    def test_lock_screen_adapted_is_none_without_a_patched_tree(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            make_tree(root, patched=False)
            self.assertIsNone(settings.lock_screen_adapted(root))


if __name__ == "__main__":
    unittest.main()
