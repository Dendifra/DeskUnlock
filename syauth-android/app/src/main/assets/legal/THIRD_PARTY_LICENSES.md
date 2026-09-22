# Third-party license inventory

Release-oriented inventory for DeskUnlock 0.1.0. This file records the
licenses and distribution class of code that is compiled, linked, bundled, or
required externally. It is not a replacement for the license terms of each
upstream component.

Evidence used:

- `Cargo.toml`, `Cargo.lock`, and `cargo metadata --locked`;
- `deny.toml` and `cargo deny check licenses`;
- Gradle `releaseRuntimeClasspath` dependency report and cached Maven POM
  license metadata;
- the exact pinned DankMaterialShell `LICENSE` file;
- Arch package metadata for external runtime dependencies.

## Production and redistributed components

| Component / version | Role | Distribution class | License | Attribution / obligation |
|---|---|---|---|---|
| DeskUnlock workspace crates 0.1.0 | Rust desktop, PAM, daemon, transport, and mobile code | compiled/linked | MIT | Preserve `LICENSE` and upstream notices. |
| `syauth` upstream | Source basis for the downstream project | compiled/linked | MIT | Preserve `Copyright (c) 2026 syauth contributors` and MIT terms. |
| RustCrypto and security closure, including `ed25519-dalek` 2.2.0, `blake3` 1.8.5, `hkdf` 0.13.x, `sha2` 0.11.0, `subtle` 2.6.1, `zeroize` 1.8.2 | Cryptography and secret handling | compiled/linked | MIT, Apache-2.0, BSD-3-Clause, CC0-1.0, and permitted dual expressions | Preserve applicable upstream notices and license texts. Exact closure is in `Cargo.lock`; policy is checked by `cargo-deny`. |
| Tokio, Serde, Clap, tracing, time, UUID, and their runtime closure | Async runtime, serialization, CLI, logging, and data handling | compiled/linked | Primarily MIT / Apache-2.0 and permitted dual expressions | Preserve applicable upstream notices and license texts. |
| `bluer` 0.17.4, `dbus` 0.9.11, `nix` 0.29.x, `libc` 0.2.x, `notify` 8.2.0 | Linux Bluetooth, D-Bus, OS, and file-watch integration | compiled/linked | BSD-2-Clause, Apache-2.0/MIT, MIT/Apache-2.0, and ISC where applicable | Preserve applicable upstream notices and license texts. |
| UniFFI runtime/build closure 0.29.5 | Rust-to-Kotlin bindings and generated native AAR support | compiled/linked for the Android native library; build tooling otherwise | MPL-2.0 | Preserve MPL-2.0 notice and terms for the distributed runtime code. Build-only generator components are not runtime dependencies. |
| Android Rust native AAR | Native `libsyauth_mobile.so` libraries for Android ABIs | redistributed/bundled and compiled/linked | DeskUnlock/upstream MIT plus the Rust closure above | APK legal bundle carries project and third-party attribution. |
| Kotlin stdlib 1.9.22 and runtime transitive closure | Android runtime | redistributed/bundled | Apache-2.0 | Preserve Apache-2.0 attribution. |
| AndroidX Core 1.12.0, Activity 1.8.2, Lifecycle 2.7.0, Fragment 1.6.2, Navigation 2.7.7, WorkManager 2.9.0, Biometric 1.2.0-alpha05 | Android runtime and UI lifecycle | redistributed/bundled | Apache-2.0 | Preserve AndroidX Apache-2.0 attribution. |
| Jetpack Compose 1.6.1, Material/Material3 1.2.0, Material Icons, Compose BOM 2024.02.00 | Android UI runtime | redistributed/bundled | Apache-2.0 | Preserve Apache-2.0 attribution. |
| JNA 5.14.0 | Android native library loading/runtime bridge | redistributed/bundled | Apache-2.0 / LGPL-2.1-or-later | DeskUnlock elects the Apache-2.0 option for redistribution; preserve the JNA notice and Apache-2.0 terms. |
| DankMaterialShell `aa4b99def48637d86a69620c0a8f3cc6aa0c4092` | Source patched and compiled into `dms-syauth` | compiled/linked and redistributed in the Linux package | MIT | Preserve `legal/third_party/DankMaterialShell-LICENSE.txt`. |
| `assets/deskunlock-logo.png` | Android and Linux application artwork | redistributed/bundled | DeskUnlock project asset; provenance documented in `assets/README.md` | Distributed with the project. No third-party ownership or trademark claim is made. |

The Rust normal-production dependency closure has 297 package records in the
locked graph. `cargo deny check licenses` passes with the allow-list in
`deny.toml`; no package in that closure has missing license metadata. The
closure includes these additional permitted license families where required:
BSD-1-Clause, BSD-2-Clause, ISC, Unicode-3.0, CC0-1.0, Zlib, Unlicense, and
Apache-2.0 WITH LLVM-exception.

## External system dependencies

These are declared or used as system packages; DeskUnlock does not copy their
full source or binary packages into its distribution:

| Component | Role | Distribution class | Package-level license handling |
|---|---|---|---|
| PySide6 / Qt | Desktop settings GUI | external system dependency | Distro package owns its GPL/LGPL/Qt-commercial metadata. |
| BlueZ / bluez-utils | Bluetooth system service and tools | external system dependency | Distro package owns its GPL metadata. |
| PAM | Authentication ABI and module host | external system dependency | Distro package owns its GPL metadata. |
| systemd | User services and runtime integration | external system dependency | Distro package owns its mixed LGPL/GPL/MIT-0/CC0 metadata. |
| polkit | Desktop authorization integration | external system dependency | Distro package owns its LGPL metadata. |
| Wayland | Lock-helper protocol/build interface | external system dependency | Distro package owns its MIT metadata. |
| D-Bus, Python, bash, dbus, and related base libraries | Runtime/tooling support | external system dependency | Respective distro packages provide their license terms. |
| `dms-shell` | Optional desktop shell integration dependency | external system dependency | Distro package provides its MIT metadata; the separately built `dms-syauth` binary is covered by the DankMaterialShell notice above. |

## Development and test only

Rust dev-dependencies (`assert_cmd`, `insta`, `predicates`, `proptest`, test
helpers, and test-only feature expansions), Android `testImplementation`,
`androidTestImplementation`, `debugImplementation`, Gradle plugins, the JDK,
Android SDK/build tools, Cargo, Go, and `wayland-scanner` build tooling are not
shipped runtime components. They are excluded from the production attribution
set, although their licenses remain subject to their own development use.
