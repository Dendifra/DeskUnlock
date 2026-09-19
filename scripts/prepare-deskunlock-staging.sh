#!/usr/bin/env bash
set -Eeuo pipefail

SOURCE="${1:-$HOME/syauth}"
DEST="${2:-$HOME/DeskUnlock}"
KIT="${3:-$HOME/Downloads/deskunlock-github-kit.zip}"

fail() {
    echo "STOP: $*" >&2
    exit 1
}

[[ -d "$SOURCE" ]] || fail "source tree not found: $SOURCE"
[[ -f "$SOURCE/Cargo.toml" ]] || fail "Cargo.toml not found in source tree"
[[ -f "$SOURCE/LICENSE" ]] || fail "upstream LICENSE not found"
[[ ! -e "$DEST" ]] || fail "destination already exists: $DEST"
command -v rsync >/dev/null || fail "rsync not installed"
command -v unzip >/dev/null || fail "unzip not installed"

if [[ ! -f "$KIT" ]]; then
    alt="/mnt/GoogleDrive/Download/deskunlock-github-kit.zip"
    [[ -f "$alt" ]] && KIT="$alt"
fi
[[ -f "$KIT" ]] || fail "publication kit zip not found"

echo "Creating isolated staging tree:"
echo "  source: $SOURCE"
echo "  dest:   $DEST"

mkdir -p "$DEST"

rsync -a \
    --exclude='.git/' \
    --exclude='target/' \
    --exclude='build/' \
    --exclude='.gradle/' \
    --exclude='*.pkg.tar.zst' \
    --exclude='*.pkg.tar.zst.sig' \
    --exclude='.env' \
    --exclude='.env.*' \
    --exclude='*.pem' \
    --exclude='*.p12' \
    --exclude='*.pfx' \
    --exclude='*.cred' \
    --exclude='bonds.toml' \
    "$SOURCE/" "$DEST/"

# Overlay publication docs/templates. This intentionally replaces staging docs,
# never the original working source tree.
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
unzip -q "$KIT" -d "$TMP"
cp -a "$TMP/deskunlock-github-kit/." "$DEST/"

chmod +x "$DEST/scripts/audit-public-release.sh"

echo
echo "Running publication audit..."
set +e
"$DEST/scripts/audit-public-release.sh" "$DEST"
AUDIT_RC=$?
set -e

echo
echo "========================================"
echo "DESKUNLOCK STAGING CREATED"
echo "========================================"
echo "Original source untouched: $SOURCE"
echo "Staging tree:             $DEST"
echo "Audit report:             $DEST/PUBLICATION_AUDIT.md"
echo
if [[ "$AUDIT_RC" -eq 0 ]]; then
    echo "Audit: no hard failures; warnings still require review."
else
    echo "Audit: hard failures found. Do NOT publish yet."
fi
echo
echo "No GitHub repository was created or published."
