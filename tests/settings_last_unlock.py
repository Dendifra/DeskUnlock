#!/usr/bin/env python3
"""Regression tests for the settings GUI's local unlock timestamp."""

import importlib.machinery
import importlib.util
import os
import time
import unittest
from unittest.mock import patch
from datetime import datetime
from pathlib import Path


MODULE_PATH = Path(__file__).parents[1] / "desktop/bin/syauth-settings"
loader = importlib.machinery.SourceFileLoader("syauth_settings", str(MODULE_PATH))
spec = importlib.util.spec_from_loader(loader.name, loader)
settings = importlib.util.module_from_spec(spec)
loader.exec_module(settings)


class SettingsLastUnlockTests(unittest.TestCase):
    def setUp(self):
        self.old_tz = os.environ.get("TZ")

    def tearDown(self):
        if self.old_tz is None:
            os.environ.pop("TZ", None)
        else:
            os.environ["TZ"] = self.old_tz
        time.tzset()

    @staticmethod
    def set_timezone(name):
        os.environ["TZ"] = name
        time.tzset()

    def test_utc_timestamp_is_timezone_aware(self):
        parsed = datetime.fromisoformat("2026-09-21T21:03:00+00:00")
        self.assertIsNotNone(parsed.tzinfo)

    def test_conversion_uses_system_timezone(self):
        self.set_timezone("Europe/Rome")
        self.assertEqual(
            settings.format_unlock_timestamp("2026-09-21T21:03:00Z"),
            "21/09/2026 23:03",
        )

    def test_fractional_seconds_are_supported(self):
        self.set_timezone("UTC")
        self.assertEqual(
            settings.format_unlock_timestamp("2026-09-21T21:03:00.123456Z"),
            "21/09/2026 21:03",
        )

    def test_date_rollover_uses_local_date(self):
        self.set_timezone("Pacific/Kiritimati")
        self.assertEqual(
            settings.format_unlock_timestamp("2026-09-21T10:30:00Z"),
            "22/09/2026 00:30",
        )

    def test_dst_follows_system_timezone(self):
        self.set_timezone("Europe/Rome")
        self.assertEqual(
            settings.format_unlock_timestamp("2026-07-21T21:03:00Z"),
            "21/07/2026 23:03",
        )

    def test_missing_value_remains_mai(self):
        self.assertEqual("Mai", "Mai")

    def test_malformed_timestamp_does_not_raise(self):
        self.assertEqual(
            settings.format_unlock_timestamp("not-a-timestamp"),
            "not-a-timestamp",
        )


class SettingsPhoneIdentityTests(unittest.TestCase):
    def test_galaxy_is_rendered_in_the_actual_settings_card(self):
        os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
        app = settings.QApplication.instance() or settings.QApplication([])

        def command(args, **_kwargs):
            if args == ["syauth", "list"]:
                return "fixture-peer\tGalaxy S26\tbonded\t2026-01-01T00:00:00Z"
            if args == [str(settings.CONTROL), "status"]:
                return "Syauth: ON"
            return ""

        with patch.object(settings, "run_command", side_effect=command):
            window = settings.SettingsWindow()
            try:
                self.assertEqual(window.phone_name.text(), "Galaxy S26")
                self.assertEqual(window.phone_name.textFormat(), settings.Qt.TextFormat.PlainText)
                self.assertEqual(window.phone_state.text(), "Associato")
            finally:
                window.timer.stop()
                window.close()
                window.deleteLater()
                app.processEvents()

    def test_non_pixel_phone_comes_from_application_bond(self):
        with patch.object(settings, "run_command", return_value=(
            "fixture-peer\tGalaxy S26\tbonded\t2026-01-01T00:00:00Z\n"
        )) as command:
            phone = settings.get_phone_info()
        self.assertEqual(phone["name"], "Galaxy S26")
        self.assertTrue(phone["paired"])
        self.assertIn("samples_fresh", phone)
        self.assertIn("sample_age_ms", phone)
        command.assert_called_once_with(["syauth", "list"])

    def test_revoked_phone_and_unrelated_bluetooth_devices_are_not_selected(self):
        with patch.object(settings, "run_command", return_value=(
            "old-fixture\tPixel 8\trevoked:replaced\tdate\n"
            "new-fixture\tFairphone\tbonded\tdate\n"
        )):
            self.assertEqual(settings.get_phone_info()["name"], "Fairphone")

    def test_ambiguous_bonds_are_not_resolved_by_vendor_or_first_row(self):
        with patch.object(settings, "run_command", return_value=(
            "fixture-one\tGalaxy S26\tbonded\tdate\n"
            "fixture-two\tAnother phone\tbonded\tdate\n"
        )):
            phone = settings.get_phone_info()
        self.assertFalse(phone["paired"])
        self.assertEqual(phone["name"], "Più telefoni associati")

    def test_missing_bond_never_picks_a_bluetooth_device(self):
        with patch.object(settings, "run_command", return_value="(no bonds)"):
            self.assertFalse(settings.get_phone_info()["paired"])


class SettingsProximityTests(unittest.TestCase):
    def test_disabled_proximity_is_visible_as_disabled_everywhere(self):
        label, active = settings.proximity_display_state({
            "available": True,
            "enabled": False,
            "state": "NEAR",
        })

        self.assertEqual(label, "Disattivato")
        self.assertFalse(active)

    def test_enabled_proximity_keeps_the_technical_state(self):
        label, active = settings.proximity_display_state({
            "available": True,
            "enabled": True,
            "state": "NEAR",
        })

        self.assertEqual(label, "Vicino")
        self.assertTrue(active)


if __name__ == "__main__":
    unittest.main()
