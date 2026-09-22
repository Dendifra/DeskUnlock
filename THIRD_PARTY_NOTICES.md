# DeskUnlock release third-party notices

This notice bundle accompanies DeskUnlock distributions. It covers code that
is compiled into the Linux binaries or Android native library, and libraries
bundled into the Android APK.

## Project and upstream

DeskUnlock is MIT licensed; see `LICENSE`.

DeskUnlock is an independent downstream project derived from `syauth`:

- Repository: https://github.com/dmytrogajewski/syauth
- License: MIT
- Preserved notices: `Copyright (c) 2026 syauth contributors` and
  `Copyright (c) 2026 PhoneLogin contributors`

See `NOTICE.md` for the project-level attribution statement.

## License families in compiled Rust code

The locked production Rust closure is checked by `cargo-deny`. It contains
permitted MIT, Apache-2.0, BSD-1-Clause, BSD-2-Clause, BSD-3-Clause, ISC,
Unicode-3.0, CC0-1.0, Zlib, Unlicense, Apache-2.0 WITH LLVM-exception, and
MPL-2.0 expressions, including permitted dual-license choices. The exact
package/version inventory and evidence are in `THIRD_PARTY_LICENSES.md`.

The applicable upstream license and copyright notices must accompany compiled
Rust binaries and the Android native AAR. The project does not relicense those
components.

## Android runtime

The Android release runtime bundles Kotlin, AndroidX, Jetpack Compose,
Material/Material Icons, Biometric, Navigation, WorkManager, and transitive
Android runtime libraries under Apache-2.0 terms. JNA 5.14.0 is distributed
under its permitted Apache-2.0 option. The Rust native AAR also carries the
compiled Rust and UniFFI MPL-2.0 closure.

The Android APK includes this notice bundle, `LICENSE`, and `NOTICE.md` under
`assets/legal/`. Linux-only DankMaterialShell notices are intentionally not
included in the Android bundle.

## Linux DMS bridge

DankMaterialShell is built at the pinned revision
`aa4b99def48637d86a69620c0a8f3cc6aa0c4092` and compiled into the redistributed
`dms-syauth` executable. It is MIT licensed; the exact upstream notice is in
`legal/third_party/DankMaterialShell-LICENSE.txt`.

The DMS production Go closure also includes
`github.com/yeqown/reedsolomon` v1.0.0. Debian's version-specific source
package `golang-github-yeqown-reedsolomon` 1.0.0-2 identifies the upstream
`Files: *` license as Expat, copyright 2026 yeqown. Its 1.0.0 orig source
matches the Go module files byte-for-byte. The applicable notice is in
`legal/third_party/licenses/Expat.txt`.

## Standalone artwork

`assets/deskunlock-logo.png` is the DeskUnlock project asset. Its documented
provenance is in `assets/README.md`; no third-party artwork or trademark claim
is made.

## External system packages

PySide6/Qt, BlueZ, PAM, systemd, polkit, Wayland, D-Bus, Python, bash, and
`dms-shell` are external system dependencies. DeskUnlock does not redistribute
their packages; their distro-provided license notices remain applicable.
