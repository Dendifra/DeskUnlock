import tempfile
import unittest
from pathlib import Path

from desktop.dms.sanitize_dms_translation_readme import sanitize_translation_readme


class DmsPrivacyPathsTest(unittest.TestCase):
    def test_replaces_the_single_translation_home_path(self):
        suffix = "/.config/quickshell/DankMaterialShellGit/translations"
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            readme = root / "quickshell/translations/README.md"
            readme.parent.mkdir(parents=True)
            readme.write_text("cd /home/" + "builder" + suffix + "\n")
            sanitize_translation_readme(root)
            self.assertEqual(readme.read_text(), "cd /home/user" + suffix + "\n")

    def test_rejects_missing_or_duplicate_anchor_without_writing(self):
        target = "cd /home/" + "builder" + "/.config/quickshell/DankMaterialShellGit/translations\n"
        for content in ("", target * 2):
            with self.subTest(content=content), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                readme = root / "quickshell/translations/README.md"
                readme.parent.mkdir(parents=True)
                readme.write_text(content)
                with self.assertRaisesRegex(ValueError, "exactly one"):
                    sanitize_translation_readme(root)
                self.assertEqual(readme.read_text(), content)

    def test_leaves_generic_greeter_examples_untouched(self):
        suffix = "/.config/quickshell/custom-dms"
        examples = "sway /home/user" + suffix + "\nscroll /home/user" + suffix + "\n"
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            readme = root / "quickshell/translations/README.md"
            readme.parent.mkdir(parents=True)
            readme.write_text("cd /home/" + "builder" + "/.config/quickshell/DankMaterialShellGit/translations\n")
            greeter = root / "quickshell/Modules/Greetd/assets/dms-greeter"
            greeter.parent.mkdir(parents=True)
            greeter.write_text(examples)
            sanitize_translation_readme(root)
            self.assertEqual(greeter.read_text(), examples)
