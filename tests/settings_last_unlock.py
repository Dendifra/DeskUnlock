#!/usr/bin/env python3
"""Regression tests for the settings GUI's local unlock timestamp."""

import importlib.machinery
import importlib.util
import os
import time
import unittest
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


if __name__ == "__main__":
    unittest.main()
