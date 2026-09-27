#!/usr/bin/env bash
# Sanitize machine/operator identifiers from public documentation.
# This intentionally edits only tracked documentation/spec text. Production
# source code and test fixtures are not rewritten.
#
# Usage:
#   bash scripts/sanitize-public-docs.sh
#
# Review `git diff` after running. The script is idempotent.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

mapfile -d '' FILES < <(
  git ls-files -z -- '*.md' '*.txt' '*.rst' '*.adoc'
)

if (( ${#FILES[@]} == 0 )); then
  echo "No tracked documentation files found."
  exit 0
fi

python - "${FILES[@]}" <<'PY'
from pathlib import Path
import re
import sys

paths = [Path(p) for p in sys.argv[1:]]

serial = re.compile(r"\bR[A-Z0-9]{9,13}\b")
peer_assignment = re.compile(r"(?i)\b(peer_id|peer-id)(\s*[=:]\s*)[0-9a-f]{24,64}\b")
bond_key_assignment = re.compile(r"(?i)\bbond_key_hex(\s*[=:]\s*)[0-9a-f]{64}\b")
keystore_alias = re.compile(r"\bsyauth\.ed25519\.[A-Za-z0-9._-]{8,}\b")
home_path = re.compile(r"/(?:home|Users)/([A-Za-z0-9_.-]+)(?=/|\b)")
mount_path = re.compile(r"/mnt/(?:Dati|Backups|GoogleDrive)(?:/[^\s`\"')\]]*)?")
run_media = re.compile(r"/run/media/[A-Za-z0-9_.-]+(?:/[^\s`\"')\]]*)?")

allowed_home = {"user", "UID", ".config", "example"}
changed = []

for path in paths:
    try:
        text = path.read_text(encoding="utf-8")
    except UnicodeDecodeError:
        continue

    new = serial.sub("<DEVICE_SERIAL>", text)
    new = peer_assignment.sub(lambda m: f"{m.group(1)}{m.group(2)}<PEER_ID>", new)
    new = bond_key_assignment.sub(lambda m: f"bond_key_hex{m.group(1)}<BOND_KEY>", new)
    new = keystore_alias.sub("syauth.ed25519.<KEYSTORE_ALIAS>", new)

    def home_repl(m: re.Match[str]) -> str:
        user = m.group(1)
        return m.group(0) if user in allowed_home else "/home/user"

    new = home_path.sub(home_repl, new)
    new = mount_path.sub("<PRIVATE_MOUNT>", new)
    new = run_media.sub("<PRIVATE_MEDIA_MOUNT>", new)

    if new != text:
        path.write_text(new, encoding="utf-8")
        changed.append(str(path))

if changed:
    print("Sanitized files:")
    for name in changed:
        print(f"  {name}")
else:
    print("No sanitization changes required.")
PY

# The fast gate must pass after sanitization. It does not replace manual review.
bash scripts/privacy-check.sh

echo
echo "Review with: git diff --check && git diff"
