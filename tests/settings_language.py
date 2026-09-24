#!/usr/bin/env python3
"""Localisation plumbing for the settings GUI.

The mechanism is deliberately *not* "two builds": one package ships one binary
and both catalogs, and the language is chosen at run time. These tests pin the
two places where a wrong answer would be silent — the choice order, and the
config merge.
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
    """Order: the operator's choice, then the desktop locale, then English."""

    def test_an_explicit_choice_beats_the_desktop_locale(self):
        self.assertEqual("it", settings.resolve_language("it", {"LANG": "en_US.UTF-8"}))
        self.assertEqual("en", settings.resolve_language("en", {"LANG": "it_IT.UTF-8"}))

    def test_auto_follows_the_desktop_locale(self):
        self.assertEqual("it", settings.resolve_language("auto", {"LANG": "it_IT.UTF-8"}))
        self.assertEqual("en", settings.resolve_language("auto", {"LANG": "en_US.UTF-8"}))

    def test_an_unknown_locale_falls_back_to_english(self):
        """Never to a half-translated screen: the source is English."""
        self.assertEqual("en", settings.resolve_language("auto", {"LANG": "de_DE.UTF-8"}))
        self.assertEqual("en", settings.resolve_language("auto", {}))
        self.assertEqual("en", settings.resolve_language(None, {}))

    def test_language_outranks_lc_all_outranks_lang(self):
        self.assertEqual(
            "it",
            settings.resolve_language(
                "auto", {"LANGUAGE": "it:en", "LC_ALL": "en_US.UTF-8", "LANG": "en_US.UTF-8"}
            ),
        )

    def test_a_stored_garbage_value_does_not_win(self):
        self.assertEqual("en", settings.resolve_language("klingon", {}))


class ConfigMergeTests(unittest.TestCase):
    """Saving one setting must not erase the other: the old writer rewrote the
    whole file, so storing the theme dropped every other key."""

    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        patcher = patch.object(
            settings, "GUI_THEME_CONFIG", Path(self.dir.name) / "gui.conf"
        )
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_saving_the_language_keeps_the_theme(self):
        settings.write_gui_theme("light")
        settings.write_gui_language("it")
        self.assertEqual("light", settings.read_gui_theme())
        self.assertEqual("it", settings.read_gui_language())

    def test_saving_the_theme_keeps_the_language(self):
        settings.write_gui_language("it")
        settings.write_gui_theme("light")
        self.assertEqual("it", settings.read_gui_language())

    def test_defaults_when_the_file_is_absent(self):
        self.assertEqual("auto", settings.read_gui_language())
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
