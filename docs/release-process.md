# DeskUnlock release process

This runbook describes the current release pipeline. The GitHub Actions release
workflow builds a source tarball only; it does not build or publish the Arch
package or the signed Android APK.

## Release sequence

1. Prepare version metadata and release notes on a branch. Open a PR to `master`
   and wait for the required CI checks to finish successfully.
2. Merge the PR, fetch the updated `master`, and verify the exact commit and a
   clean worktree. Run packaging from a fresh worktree with no stale `target/`
   artifacts; do not clean existing build or backup directories.
3. Create and verify a signed version tag (for example, `vX.Y.Z`) on that
   commit. Never move or overwrite an existing release tag. Pushing the new tag
   starts `.github/workflows/release.yml`, which creates a source-tarball
   workflow artifact; it does not create a GitHub Release.
4. On CachyOS/Arch with `dms-shell` and the Android NDK available, build the
   Arch package and Android app using `scripts/release-local.sh <tag>`. The
   script rebuilds the Rust AAR, assembles the release APK, checks for embedded
   build paths, and signs through `scripts/sign-release-apk.sh`. That script
   verifies the APK's actual signer certificate against the v0.2.1 SHA-256
   fingerprint `9908dc83186cb41f15acc5d4091e9ac59e0770b615c5cfa9c55b9a4fe25ef277`
   before moving it into the artifact directory. The release script
   then generates `SHA256SUMS` for the package, signed APK, and source tarball.
5. The local script requires a GitHub Release for the tag to exist before its
   upload step. Create it as a draft with reviewed notes, then run the script
   with a fresh, tag-specific `SYAUTH_RELEASE_DIR`. The script may remove stale
   APK/package assets for that same tag and uploads with `--clobber`; do not run
   it against an existing published release or an earlier version tag.
6. Verify the uploaded asset names and sizes, download the complete asset set,
   run `sha256sum -c SHA256SUMS`, verify the APK with `apksigner`, and inspect
   the Arch package contents. Publish the draft only after these checks pass.

## Expected artifacts

The release script produces the source archive, one Arch/CachyOS package, the
signed APK, and `SHA256SUMS`. Names derive from the tag and `packaging/arch/PKGBUILD`;
for DeskUnlock 0.2.2 they are expected to be:

- `syauth-0.2.2.tar.gz`
- `deskunlock-0.2.2-1-x86_64.pkg.tar.zst`
- `deskunlock-v0.2.2-signed.apk`
- `SHA256SUMS`

These names do not mean the artifacts exist: attach them only after the builds
and verification steps above succeed.

## Signing and private state

The release keystore remains outside the repository and CI. Use only the
already authorized `SYAUTH_RELEASE_KEYSTORE` and the password prompt provided
by `scripts/sign-release-apk.sh`; never place passwords in command arguments,
shell history, repository files, or logs. Do not create or export signing keys.

Pairing databases, private keys, local build outputs, and host-specific setup
scripts are not release assets. Keep existing local backups; do not clean or
replace them as part of release preparation.
