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


class AssociationRowTests(unittest.TestCase):
    """A bond file is not an association (BUG-20260924-app-revoke-desync)."""

    def test_a_bond_without_the_phone_is_not_associated(self):
        label, active = settings.association_display(bonded=1, present=False)
        self.assertEqual(label, "Telefono non connesso")
        self.assertFalse(active)

    def test_no_bond_is_not_associated(self):
        label, active = settings.association_display(bonded=0, present=False)
        self.assertEqual(label, "Non associato")
        self.assertFalse(active)

    def test_a_bond_with_the_phone_present_is_associated(self):
        label, active = settings.association_display(bonded=1, present=True)
        self.assertEqual(label, "Associato")
        self.assertTrue(active)


class LastUnlockRowTests(unittest.TestCase):
    def test_a_past_unlock_is_not_green_without_a_live_association(self):
        label, active = settings.last_unlock_display(
            "2026-09-24T14:58:06Z", associated=False
        )
        self.assertEqual(label, "2026-09-24T14:58:06Z")
        self.assertFalse(active)

    def test_never_is_not_green_even_when_associated(self):
        _, active = settings.last_unlock_display("Mai", associated=True)
        self.assertFalse(active)

    def test_a_live_association_with_a_past_unlock_is_green(self):
        _, active = settings.last_unlock_display(
            "2026-09-24T14:58:06Z", associated=True
        )
        self.assertTrue(active)


class SystemStatusCardHonestyTests(unittest.TestCase):
    """Wiring: the card must not read green while the phone is gone."""

    def _build(self, phone_present):
        app = settings.QApplication.instance() or settings.QApplication([])

        def fake_run(command, *args, **kwargs):
            if command[:2] == ["syauth", "status"]:
                return (
                    "adapter-state: Powered\n"
                    "bonds-count: 1\n"
                    "last-successful-unlock: 2026-09-24T14:58:06Z\n"
                )
            if command[:2] == ["syauth", "list"]:
                return "5a5481f3e60858d387de52e12f4552ec\tPixel 8\tbonded\n"
            return ""

        phone_info = {
            "name": "Pixel 8",
            "paired": True,
            "samples_fresh": phone_present,
            "sample_age_ms": 0 if phone_present else None,
        }
        for patcher in (
            patch.object(settings, "run_command", side_effect=fake_run),
            patch.object(settings, "phone_present", return_value=phone_present),
            patch.object(settings, "get_phone_info", return_value=phone_info),
        ):
            patcher.start()
            self.addCleanup(patcher.stop)

        window = settings.SettingsWindow()
        window.timer.stop()
        self.addCleanup(window.close)
        self.addCleanup(window.deleteLater)
        self.addCleanup(app.processEvents)
        return window

    def test_the_card_is_not_green_while_the_phone_is_gone(self):
        window = self._build(phone_present=False)
        self.assertFalse(window.bond_row.good)
        self.assertEqual(window.bond_row.value.text(), "Telefono non connesso")
        self.assertFalse(window.unlock_row.good)

    def test_the_card_is_green_only_with_a_live_phone(self):
        window = self._build(phone_present=True)
        self.assertTrue(window.bond_row.good)
        self.assertEqual(window.bond_row.value.text(), "Associato")


if __name__ == "__main__":
    unittest.main()
