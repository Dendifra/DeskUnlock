<p align="center">
  <img src="assets/deskunlock-banner.svg" alt="DeskUnlock — smartphone authentication and proximity unlock for Linux" width="100%">
</p>

<p align="center"><strong>Smartphone authentication and proximity unlock for Linux.</strong></p>

<p align="center">
  <img alt="Status" src="https://img.shields.io/badge/status-public%20beta-f59e0b?style=flat-square">
  <img alt="Platform" src="https://img.shields.io/badge/platform-Linux-0ea5e9?style=flat-square">
  <img alt="Companion" src="https://img.shields.io/badge/companion-Android-22c55e?style=flat-square">
  <img alt="License" src="https://img.shields.io/badge/license-MIT-64748b?style=flat-square">
</p>

# DeskUnlock

DeskUnlock is an independent downstream fork of the MIT-licensed
[`syauth`](https://github.com/dmytrogajewski/syauth) project. It lets a paired
Android phone approve a Linux desktop unlock with Android biometric
confirmation, while the configured PAM password fallback remains available.

> **Public beta:** this repository documents the `v0.1.0-beta.1` candidate.
> It is beta software, not a stable release or a security-certified product.

## What it does

- Linux desktop authentication through PAM;
- Android biometric approval for an explicit unlock request;
- local Bluetooth LE / GATT communication between the computer and phone;
- pairing and cryptographic state kept locally on the participating devices;
- proximity-aware locking and desktop lock-screen integration;
- normal PAM fallback when the phone or Bluetooth path is unavailable.

No DeskUnlock cloud account or developer-operated backend is required. The
Android manifest does not request `INTERNET`; the authentication path is
BLE/GATT-based. Normal operating-system services and network behavior outside
DeskUnlock remain outside this project's control.

## Supported beta scope

The validated beta scope is intentionally narrow:

- Arch Linux / CachyOS;
- Wayland session with the validated Niri + DankMaterialShell (DMS)
  integration;
- Android companion using Bluetooth LE and Companion Device APIs.

Other distributions, desktop environments, Android devices, and OEM ROMs may
work, but are not universal compatibility claims. Android BLE behavior,
battery management, and background-service policy vary by device and ROM.

## Architecture

```text
Linux desktop / PAM / DMS
          │ local BLE/GATT
          ▼
Android companion ── Android biometric approval
```

The desktop creates a fresh authentication request. The phone presents an
explicit approval action and protects its signing operation with Android
biometric/device-credential APIs. Proximity may lock the session, but proximity
alone is not an unlock authorization.

## Quick start

The public beta is distributed as a signed Android APK and an Arch/CachyOS
package when the release is published. Verify the published SHA-256 checksums
before installing.

### 1. Install the Linux package

```bash
sudo pacman -U deskunlock-0.1.0-18-x86_64.pkg.tar.zst
```

Use normal package-manager authentication. Do not use `--nodeps`,
`--overwrite`, or force options.

For a source checkout, see [docs/installation.md](docs/installation.md) and
[docs/packaging.md](docs/packaging.md).

### 2. Install the Android companion

Download the signed beta APK from the future release page and sideload it
through Android's normal package installer. Android may display a warning
because this beta is not distributed through Google Play. See
[docs/android-setup.md](docs/android-setup.md).

The release APK uses the dedicated DeskUnlock release certificate. Release
signing material is private and is not stored in this repository. Verify the
published SHA-256 before installation.

### 3. Pair the devices

1. Start the desktop pairing flow:

   ```bash
   syauth pair --adapter hci0
   ```

2. Open DeskUnlock on Android and tap **Pair**.
3. Select the computer, confirm the operating-system pairing numbers match,
   and complete the app-level confirmation shown by both devices.
4. Confirm the desktop sees the bond:

   ```bash
   syauth list
   ```

The `syauth` command/file names are compatibility identifiers inherited from
upstream. The product and user-facing name is DeskUnlock.

### 4. Enable DeskUnlock

```bash
syauth-control on
syauth-control status
```

### 5. Unlock normally

The normal flow is:

```text
lock screen active
→ no authentication from pointer initialization alone
→ first real mouse movement, keyboard input, or Enter
→ one DeskUnlock request
→ Android biometric approval
→ desktop unlock
```

The lock surface appearing does not itself start an approval request. An
ignored request can expire; a later genuine interaction can produce another
request.

## Privacy and security

DeskUnlock is designed for local phone-to-computer communication:

- no DeskUnlock cloud service or remote account;
- no Android `INTERNET` permission in the committed manifest;
- pairing material and cryptographic state stay on the participating devices;
- biometric decisions remain inside Android biometric/Keystore APIs;
- no DeskUnlock analytics or developer telemetry is part of the project.

These statements do not cover behavior of Android, Linux, Bluetooth, DMS, or
other operating-system components. A compromised desktop, compromised phone,
physical coercion, or a stolen device with its unlock credential is outside the
guarantees DeskUnlock can make.

DeskUnlock has not received an independent professional security audit. Report
security issues through [SECURITY.md](SECURITY.md), not in a public issue with
keys, tokens, private state, or exploit details.

## Known beta limitations

- Arch Linux/CachyOS and the validated Niri + DMS path are the supported beta
  scope;
- Android OEM battery and BLE policies can affect background operation;
- an ignored approval request can expire and require a later interaction;
- retry timing and broader desktop/distro portability are not yet polished;
- replacing an older debug APK with the release-signed APK may require a
  one-time uninstall/reinstall and pairing again;
- other distributions and desktop environments are not claimed as supported.

See [docs/troubleshooting.md](docs/troubleshooting.md) for reversible checks.

## Uninstall and rollback

To disable DeskUnlock without removing persistent state:

```bash
syauth-control off
```

To remove the Linux package while preserving pairing and cryptographic state:

```bash
sudo pacman -R deskunlock
```

To remove the Android companion, use Android Settings → Apps → DeskUnlock.
Removing the app removes its app-private state; pair the phone again if it is
installed later. Desktop pairing state is a separate explicit concern: revoke
an old peer with `syauth revoke <peer-id>` when you intentionally retire it.
Do not delete system state as a routine troubleshooting step.

## License and notices

DeskUnlock is MIT licensed and preserves upstream attribution. See:

- [LICENSE](LICENSE)
- [NOTICE.md](NOTICE.md)
- [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md)
- [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)

The project logo provenance is documented in [assets/README.md](assets/README.md).
