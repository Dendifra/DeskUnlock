#!/usr/bin/env python3
"""Regression tests for the settings GUI's proximity on/off toggle.

The bug this pins: ``syauth-settings`` used to hard-code the helper
scripts to ``/usr/bin``. A checkout therefore drove the *packaged*
``syauth-proximity`` and silently ignored the checkout's own copy, so
the toggle wrote the config but never stopped/started the service.
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


class HelperResolutionTests(unittest.TestCase):
    def test_helpers_resolve_to_the_checkout_next_to_the_settings_script(self):
        expected_dir = MODULE_PATH.parent
        self.assertEqual(settings.PROXIMITY_CONTROL, expected_dir / "syauth-proximity")
        self.assertEqual(settings.CONTROL, expected_dir / "syauth-control")
        self.assertEqual(settings.IDLE_CONTROL, expected_dir / "syauth-idle-lock")

    def test_missing_helper_falls_back_to_usr_bin(self):
        self.assertEqual(
            settings.helper_script("syauth-not-a-real-helper"),
            Path("/usr/bin/syauth-not-a-real-helper"),
        )


class ProximityToggleTests(unittest.TestCase):
    def setUp(self):
        self.app = settings.QApplication.instance() or settings.QApplication([])

    def _window(self, command):
        patcher = patch.object(settings, "run_command", side_effect=command)
        patcher.start()
        self.addCleanup(patcher.stop)
        window = settings.SettingsWindow()
        self.addCleanup(window.deleteLater)
        self.addCleanup(window.timer.stop)
        window.show()
        self.app.processEvents()
        return window

    def test_toggling_off_invokes_the_resolved_helper_disable(self):
        calls = []

        def command(args, **_kwargs):
            calls.append(args)
            if args and args[0] == "syauth" and args[1:2] == ["list"]:
                return ""
            return ""

        window = self._window(command)
        window.set_proximity_enabled(False)
        self.assertIn([str(settings.PROXIMITY_CONTROL), "disable"], calls)

    def test_toggling_on_invokes_the_resolved_helper_enable(self):
        calls = []

        def command(args, **_kwargs):
            calls.append(args)
            return ""

        window = self._window(command)
        window.set_proximity_enabled(True)
        self.assertIn([str(settings.PROXIMITY_CONTROL), "enable"], calls)


if __name__ == "__main__":
    unittest.main()
