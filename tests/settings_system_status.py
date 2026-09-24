#!/usr/bin/env python3
"""Regression test for the settings GUI system-status card.

The card used to carry a redundant "Servizio DeskUnlock" row: the daemon
liveness is already surfaced by the Proximity Lock row, and the extra row
turned red (and read like a crash) the moment the master switch was
toggled off. The row is gone; this pins it.
"""

import importlib.machinery
import importlib.util
import os
import unittest
from pathlib import Path
from unittest.mock import patch


os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

MODULE_PATH = Path(__file__).parents[1] / "desktop/bin/syauth-settings"
loader = importlib.machinery.SourceFileLoader("syauth_settings", str(MODULE_PATH))
spec = importlib.util.spec_from_loader(loader.name, loader)
settings = importlib.util.module_from_spec(spec)
loader.exec_module(settings)


class SystemStatusCardTests(unittest.TestCase):
    def test_redundant_daemon_row_is_gone(self):
        app = settings.QApplication.instance() or settings.QApplication([])
        with patch.object(settings, "run_command", return_value=""):
            window = settings.SettingsWindow()
            try:
                self.assertFalse(hasattr(window, "daemon_row"))
                labels = {
                    label.text()
                    for label in window.findChildren(settings.QLabel)
                }
                self.assertNotIn("Servizio DeskUnlock", labels)
            finally:
                window.timer.stop()
                window.close()
                window.deleteLater()
                app.processEvents()


class ProximityDisplayTests(unittest.TestCase):
    def test_absent_phone_is_not_reported_as_active(self):
        label, active = settings.proximity_display_state(
            {"available": True, "enabled": True, "state": "ABSENT", "profile_id": "near"}
        )
        self.assertEqual(label, "Telefono assente")
        self.assertFalse(active)

    def test_near_phone_is_reported_as_active(self):
        label, active = settings.proximity_display_state(
            {"available": True, "enabled": True, "state": "NEAR", "profile_id": "near"}
        )
        self.assertEqual(label, "Vicino")
        self.assertTrue(active)

    def test_disabled_proximity_is_not_active(self):
        label, active = settings.proximity_display_state(
            {"available": True, "enabled": False, "state": "NEAR", "profile_id": "near"}
        )
        self.assertEqual(label, "Disattivato")
        self.assertFalse(active)


if __name__ == "__main__":
    unittest.main()
