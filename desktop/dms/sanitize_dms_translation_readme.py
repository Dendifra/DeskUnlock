#!/usr/bin/env python3
import re
import sys
from pathlib import Path


UPSTREAM_PATH = re.compile(
    r"(?m)^cd /home/[^/\r\n]+/\.config/quickshell/DankMaterialShellGit/translations$"
)
NEUTRAL_PATH = "cd /home/user/.config/quickshell/DankMaterialShellGit/translations"


def sanitize_translation_readme(dms_root: Path) -> None:
    readme = dms_root / "quickshell/translations/README.md"
    content, replacements = UPSTREAM_PATH.subn(NEUTRAL_PATH, readme.read_text(encoding="utf-8"))
    if replacements != 1:
        raise ValueError(f"expected exactly one anchored README path, found {replacements}")
    readme.write_text(content, encoding="utf-8")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: sanitize_dms_translation_readme.py DMS_SOURCE_ROOT")
    try:
        sanitize_translation_readme(Path(sys.argv[1]))
    except (OSError, ValueError) as error:
        raise SystemExit(f"DMS README privacy sanitization failed: {error}") from error
