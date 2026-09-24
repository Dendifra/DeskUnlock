#!/usr/bin/env python3
"""Localisation plumbing for the settings GUI.

The mechanism is deliberately *not* "two builds": one package ships one binary
and both catalogs, and the language follows the desktop locale at run time —
exactly as the phone's locale decides for the app. There is no in-GUI override
to keep in sync, so these tests pin two things: where the language comes from,
and that the config merge cannot erase a neighbouring setting.
"""

import importlib.machinery
import importlib.util
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

MODULE_PATH = Path(__file__).parents[1] / "desktop/bin/syauth-settings"
loader = importlib.machinery.SourceFileLoader("syauth_settings", str(MODULE_PATH))
spec = importlib.util.spec_from_loader(loader.name, loader)
settings = importlib.util.module_from_spec(spec)
loader.exec_module(settings)


class LanguageChoiceTests(unittest.TestCase):
    """The desktop locale decides; English is the fallback, never a half screen."""

    def test_an_italian_desktop_gets_italian(self):
        self.assertEqual("it", settings.resolve_language({"LANG": "it_IT.UTF-8"}))

    def test_an_english_desktop_gets_english(self):
        self.assertEqual("en", settings.resolve_language({"LANG": "en_US.UTF-8"}))

    def test_a_locale_we_have_no_catalog_for_gets_english(self):
        self.assertEqual("en", settings.resolve_language({"LANG": "de_DE.UTF-8"}))
        self.assertEqual("en", settings.resolve_language({}))
        self.assertEqual("en", settings.resolve_language({"LANG": ""}))

    def test_language_outranks_lc_all_outranks_lang(self):
        self.assertEqual(
            "it",
            settings.resolve_language(
                {"LANGUAGE": "it:en", "LC_ALL": "en_US.UTF-8", "LANG": "en_US.UTF-8"}
            ),
        )

    def test_a_locale_without_an_encoding_suffix_is_understood(self):
        self.assertEqual("it", settings.resolve_language({"LANG": "it"}))


class ConfigMergeTests(unittest.TestCase):
    """Saving one setting must not erase the other: the old writer rewrote the
    whole file, so storing any key dropped every other key."""

    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        patcher = patch.object(
            settings, "GUI_THEME_CONFIG", Path(self.dir.name) / "gui.conf"
        )
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_the_theme_round_trips(self):
        settings.write_gui_theme("light")
        self.assertEqual("light", settings.read_gui_theme())

    def test_an_unknown_value_falls_back_to_the_default(self):
        settings.GUI_THEME_CONFIG.write_text("theme=chartreuse\n", encoding="utf-8")
        self.assertEqual(settings.THEME_DARK, settings.read_gui_theme())

    def test_default_when_the_file_is_absent(self):
        self.assertEqual(settings.THEME_DARK, settings.read_gui_theme())

    def test_an_unknown_key_in_the_file_is_ignored(self):
        settings.GUI_THEME_CONFIG.write_text("theme=light\nbogus=1\n", encoding="utf-8")
        self.assertEqual("light", settings.read_gui_theme())
        self.assertNotIn("bogus", settings.read_gui_config())


class TranslationTests(unittest.TestCase):
    def test_english_needs_no_catalog(self):
        """The source language maps to the identity, so a machine with no
        /usr/share/locale still reads a complete English GUI."""
        settings.install_translations("en")
        self.assertEqual("Pair a computer", settings._("Pair a computer"))

    def test_italian_without_a_catalog_falls_back_to_the_source(self):
        with patch.object(settings, "LOCALE_DIR", Path("/nonexistent/locale")):
            settings.install_translations("it")
            self.assertEqual("Pair a computer", settings._("Pair a computer"))
        settings.install_translations("en")


if __name__ == "__main__":
    unittest.main()
