#!/usr/bin/env bash
# Build, sign and publish a DeskUnlock pre-release, in one command.
#
# Why this is a local script and not a CI job
# ------------------------------------------
# The two artefacts that matter cannot be produced in CI, and that is not a
# permissions problem:
#
#   * the Arch package: `packaging/arch/PKGBUILD` depends on `dms-shell`, a
#     CachyOS package a stock Arch container cannot resolve;
#   * the signed APK: signing needs the release keystore, and that key is not in
#     CI — nor should it be.
#
# The source tarball *is* built by CI (.github/workflows/release.yml), because it
# needs nothing but git.
#
# What this script does NOT do: write the release notes. They are written and
# reviewed by hand, and `softprops/action-gh-release` in CI would blank them.
#
# Usage:
#   bash scripts/release-local.sh v0.1.0-beta.3
#
# The keystore password is prompted by scripts/sign-release-apk.sh and never
# touches this script, its arguments, or your shell history.

set -Eeuo pipefail

TAG="${1:-}"
[[ -n "$TAG" ]] || { echo "usage: $0 <tag>   e.g. $0 v0.1.0-beta.3" >&2; exit 2; }

REPO="${SYAUTH_REPO:-Dendifra/DeskUnlock}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${SYAUTH_RELEASE_DIR:-$HOME/deskunlock-release-$TAG}"

step() { printf '\n==> %s\n' "$*"; }
die() { printf 'STOP: %s\n' "$*" >&2; exit 1; }

cd "$ROOT"

# --- 0. refuse to package a dirty tree ---------------------------------------
# The package is built from the working tree, while `make dist` archives HEAD.
# A dirty tree therefore ships a tarball that does not match the binary, which
# is the exact failure this guard exists to prevent.
if [[ -n "$(git status --porcelain)" ]]; then
    die "working tree is dirty: commit first, or the tarball and the package will disagree"
fi

# --- 1. the tag must exist and match HEAD ------------------------------------
step "checking the tag"
git rev-parse -q --verify "refs/tags/$TAG" >/dev/null || die "tag $TAG does not exist"
[[ "$(git rev-parse "$TAG^{commit}")" == "$(git rev-parse HEAD)" ]] \
    || die "tag $TAG does not point at HEAD: the package would not match the tag"

# --- 2. pkgrel bump, so the package number moves with the content ------------
PKGREL="$(grep -m1 '^pkgrel=' packaging/arch/PKGBUILD | cut -d= -f2)"
step "pkgrel is $PKGREL (bump it here if the content changed since the last build)"

# --- 3. source tarball -------------------------------------------------------
step "building the source tarball"
make dist

# --- 4. Arch package ---------------------------------------------------------
step "building the Arch package (this compiles the workspace)"
( cd packaging/arch && makepkg -f )
PKG="$(ls -t packaging/arch/deskunlock-*-x86_64.pkg.tar.zst | head -1)"
[[ -f "$PKG" ]] || die "no package produced"
step "package: $PKG"
# The package must carry the GUI catalogs, or the bilingual GUI silently
# degrades to Italian-on-English-desktop with no error anywhere.
tar --zstd -tf "$PKG" | grep -q 'locale/.*deskunlock\.mo' \
    || die "package carries no locale catalogs: the GUI would not be bilingual"

# --- 5. APK, signed by the operator -----------------------------------------
# The Rust libraries must be rebuilt BEFORE gradle, and this is the hole that
# let a leak reach the published APK: gradle consumes the prebuilt AAR from
# crates/syauth-mobile/target/ and reports "up-to-date" if it exists, so a
# stale AAR silently ships old binaries. Rebuilding it here is what makes the
# --remap-path-prefix in scripts/build_aar.sh take effect at all.
step "rebuilding the Rust libraries (the .aar gradle consumes)"
NDK_HOME="${NDK_HOME:-$(ls -d /opt/android-sdk/ndk/* 2>/dev/null | sort -V | tail -1)}"
[[ -n "$NDK_HOME" && -d "$NDK_HOME" ]] || die "no Android NDK found; set NDK_HOME"
export NDK_HOME
make android-aar

step "building the APK (unsigned)"
( cd syauth-android && JAVA_HOME="${JAVA_HOME:-/usr/lib/jvm/java-21-openjdk}" ./gradlew :app:assembleRelease )
APK_RAW="syauth-android/app/build/outputs/apk/release/app-release-unsigned.apk"
[[ -f "$APK_RAW" ]] || die "no APK produced"

# Prove the compiled libraries do not carry the builder's home directory. The
# signed APK is what reaches strangers, and `strings` reads it in one command;
# checking here is the difference between knowing and assuming.
step "checking the shipped libraries carry no build paths"
LEAK_DIR="$(mktemp -d)"
unzip -q -o "$APK_RAW" 'lib/*/libsyauth_mobile.so' -d "$LEAK_DIR"
LEAKS="$(find "$LEAK_DIR" -name '*.so' -exec strings {} + | grep -c '/home/')"
rm -rf "$LEAK_DIR"
[[ "$LEAKS" == "0" ]] || die "the APK embeds $LEAKS build paths from this machine; rebuild the .aar (NDK_HOME) before shipping"
step "libraries clean"

step "signing the APK — the keystore password prompt follows"
mkdir -p "$OUT"
cp -f "$APK_RAW" "$OUT/deskunlock-$TAG-unsigned.apk"
bash scripts/sign-release-apk.sh "$OUT/deskunlock-$TAG-unsigned.apk"
# The signer rewrites the file in place, so the name has to change too or the
# asset lies about what it is.
mv -f "$OUT/deskunlock-$TAG-unsigned.apk" "$OUT/deskunlock-$TAG-signed.apk"
# Verify with the same discovery order the signing script uses, so this check
# cannot pass by finding a different tool than the one that signed.
APKSIGNER="$(command -v apksigner || true)"
if [[ -z "$APKSIGNER" && -x /opt/android-sdk/build-tools/34.0.0/apksigner ]]; then
    APKSIGNER=/opt/android-sdk/build-tools/34.0.0/apksigner
fi
[[ -n "$APKSIGNER" ]] || die "apksigner not found: cannot verify what was signed"
"$APKSIGNER" verify --print-certs "$OUT/deskunlock-$TAG-signed.apk" | grep -q "CN=DeskUnlock Release" \
    || die "the signed APK does not carry the DeskUnlock release certificate"

# --- 6. collect and checksum ------------------------------------------------
step "collecting artefacts in $OUT"
cp -f "$PKG" "$OUT/"
# Exactly one tarball must exist, or the checksum would cover an ambiguous name.
mapfile -t TARBALLS < <(ls target/syauth-*.tar.gz)
[[ ${#TARBALLS[@]} -eq 1 ]] || die "expected one target/syauth-*.tar.gz, found ${#TARBALLS[@]}"
cp -f "${TARBALLS[0]}" "$OUT/"
rm -f "$OUT"/*.idsig "$OUT/SHA256SUMS"
( cd "$OUT" && sha256sum ./*.pkg.tar.zst ./*-signed.apk ./*.tar.gz > SHA256SUMS && cat SHA256SUMS )
# Every checksum must resolve: a checksum for a file nobody can download is
# worse than none, because it looks like verification.
( cd "$OUT" && sha256sum -c SHA256SUMS >/dev/null ) || die "checksums do not verify"


# --- 7. publish --------------------------------------------------------------
step "attaching to the GitHub release $TAG"
gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1 \
    || die "release $TAG does not exist: create it with --notes-file first"
gh release upload "$TAG" --repo "$REPO" --clobber \
    "$OUT"/*.pkg.tar.zst "$OUT"/*-signed.apk "$OUT"/*.tar.gz "$OUT/SHA256SUMS"

step "done — verify from the published release, not from here"
cat <<EOF
  gh release view $TAG --repo $REPO
  mkdir -p /tmp/v && cd /tmp/v
  gh release download $TAG --repo $REPO --clobber
  sha256sum -c SHA256SUMS
EOF
