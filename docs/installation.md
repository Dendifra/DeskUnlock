# Installation

## DeskUnlock 0.2.0

DeskUnlock 0.2.0 is validated on Arch Linux/CachyOS and Wayland with either
KDE Plasma or Niri + DankMaterialShell, together with the Android companion
using BLE / Companion Device APIs. Niri uses DMS; Plasma uses its native PAM
and lock-screen integration without starting DMS.

DeskUnlock is an independent downstream fork of `syauth`; internal `syauth`
command and service names remain for compatibility.

## Install the Arch/CachyOS package

When the 0.2.0 release is published:

1. Download the Arch package and its published SHA-256 checksum.
2. Verify the checksum with `sha256sum`.
3. Install it with the normal package manager:

   ```bash
   sudo pacman -U deskunlock-0.2.0-1-x86_64.pkg.tar.zst
   ```

Do not use `--nodeps`, `--overwrite`, or force options. The package installs
program files, services, PAM integration, desktop integration, and legal
notices. Pairing and cryptographic state remain outside the package payload.

## Install the Android companion

Download the signed APK from the same release and open it with Android's
normal package installer. Sideloading may produce a warning because the APK is
not distributed through Google Play.

The public APK is signed with the dedicated DeskUnlock release certificate.
The private release key is outside the repository and must never be published.
Verify the SHA-256 checksum published with the APK before installing.

A debug APK and the release APK have different signing identities. Existing
debug testers may need to uninstall the old app before installing the release
APK, then pair the computer and phone again. Do not bypass Android
signature checks or disable Play Protect globally.

## Pair and enable

1. Start the desktop pairing command:

   ```bash
   syauth pair --adapter hci0
   ```

2. Open DeskUnlock on Android and tap **Pair**.
3. Confirm the matching operating-system pairing numbers and the app-level
   confirmation.
4. Enable the desktop integration:

   ```bash
   syauth-control on
   syauth-control status
   ```

   The first activation may show a protected authorization popup so the
   per-user service manager can start at boot. On systems with Plasma Login,
   the package wires PAM and orders the greeter after Bluetooth; password
   fallback remains enabled.

5. Lock the desktop and interact with the lock screen. The first genuine mouse
   movement, keyboard input, or Enter causes one authentication request; the
   Android biometric approval then completes the unlock.

The lock surface appearing, or synthetic pointer initialization, does not
start authentication by itself.

## Boot readiness

The Plasma integration creates
`/etc/systemd/system/plasmalogin.service.d/deskunlock.conf` with
`After=bluetooth.service`. This prevents the greeter from becoming
interactive before the phone transport can initialize. If the integration was
enabled manually, run `/usr/lib/syauth/syauth-pam-sync install` as root.

## Source build

A source build requires the normal Rust, Cargo, Go, Wayland, Android/NDK, and
system package tooling described by the repository Makefile. The Arch package
build is the supported Linux packaging path. Source builds do not create or
require the private Android release key.

## Clean-machine checklist

Before treating another environment as supported, verify:

1. dependencies install;
2. the package installs without force flags;
3. the GUI opens;
4. one phone pairs;
5. biometric approval works;
6. password fallback remains available;
7. reboot and user services recover;
8. lock/unlock works after an upgrade;
9. package removal preserves state unless an explicit cleanup is chosen.

Other distributions, desktop environments, and Android OEMs are not claimed
as supported.

## Proximity and idle lock

The GUI exposes Proximity Lock and idle-lock settings. These are optional
locking mechanisms; proximity alone never authorizes an unlock. For their
operator-facing diagnostics, use the installed `syauth-proximity` and
`syauth-idle-lock` commands. See [troubleshooting.md](troubleshooting.md).
