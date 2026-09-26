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

DeskUnlock turns an Android phone into a secure key for a Linux desktop. A
paired phone approves an explicit unlock request with Android biometric
confirmation, while the configured PAM password fallback remains available.

DeskUnlock is an independent project derived from the MIT-licensed
[`syauth`](https://github.com/dmytrogajewski/syauth) project. It is developed
and distributed under its own name; it is not an official upstream `syauth`
release. See [Origins and attribution](#origins-and-attribution).

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

## High-level architecture

DeskUnlock is layered. Each layer has one owner and one responsibility, and
the layers are deliberately not interchangeable: the Bluetooth/BlueZ transport
carries the Android CDM association, the DeskUnlock protocol and its real
confirmation establish trust, and presence/PAM consume an already-established
trust.

<p align="center">
  <img src="assets/deskunlock-architecture.svg" alt="DeskUnlock layered architecture" width="100%">
</p>

The full layer contract, the target state machine, and the ownership map are
frozen in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Bluetooth pairing is not DeskUnlock pairing

This is the single most important distinction in the project:

- **Bluetooth bond = transport.** Being paired with the computer in the
  operating system's Bluetooth settings, or with DMS, only means the radio
  link can come up. It does **not** authorize anything. A phone paired with
  CachyOS is still a stranger to DeskUnlock.
- **Android CDM association = OS/app association.** A Companion Device
  Manager association lets Android wake the app near a device. It is not a
  Bluetooth bond and it is not DeskUnlock trust.
- **DeskUnlock pairing = authenticated application handshake.** DeskUnlock
  always runs its own discovery, GATT exchange and protocol transaction over
  the transport. A phone that is already Bluetooth-paired can and must still
  complete this handshake to be associated later.
- **DeskUnlock trust = actual authorization.** Only the committed
  DeskUnlock bond record and per-peer key make a phone a key. The `peer_id`
  is derived from the phone's public key, never from a Bluetooth name or
  address.
- **Presence / PAM consumes trust.** Proximity and PAM only use phones that
  already hold DeskUnlock trust; they never create it.

Consequences that follow from the invariant:

- Bluetooth paired ≠ DeskUnlock authorized.
- Android CDM associated ≠ DeskUnlock authorized.
- A BlueZ LESC numeric-comparison confirmation is a **transport** event and
  is not the DeskUnlock confirmation. DeskUnlock has its own application-level
  confirmation.
- A GUI confirmation is only shown when there is a real action to confirm.
- DMS remains the general Bluetooth frontend of the desktop. DeskUnlock does
  not modify `dms.service`, does not create a second DMS bar, and does not
  become the global manager of every Bluetooth device.
- The desktop Bluetooth adapter keeps its normal host identity (for example
  `cachyos-x8664`). The DeskUnlock identity is carried by the authenticated
  application protocol, not by renaming the adapter.

## Desktop components

| Component | Responsibility |
|---|---|
| `pam_syauth.so` | PAM module: requests authentication from the presence daemon and maps the result to PAM return codes. |
| `syauth-presenced` | Long-running user daemon: owns the BlueZ adapter usage, the GATT services, the pairing transaction and presence. |
| `syauth` CLI | Operator surface: `pair`, `list`, `revoke`, `status`, `doctor`, `reconcile`, install helpers. |
| `syauth-device` | High-level device control (`status` / `pair` / `change` / `revoke`) used by the settings GUI. |
| `syauth-settings` | Qt settings window: device state, pairing dialog, proximity, idle lock, health. |
| `syauth-control` | Master ON/OFF switch for the user services. |
| `syauth-idle-lock` | Idle-lock integration. |
| `syauth-reconcile` | Crash recovery for a staged pairing transaction. |
| `syauth-health` | Read-only health checks and doctor output. |
| DMS bridge | Lock-screen integration for the validated DankMaterialShell setup. |

Packaged executables live under `/usr/bin` and `/usr/lib/syauth`; persistent
pairing and cryptographic state lives under `/var/lib/syauth`; runtime
sockets and markers live under `$XDG_RUNTIME_DIR/syauth` and `/run/syauth`.

## Desktop app and configuration

DeskUnlock includes a desktop settings application for managing the paired
Android device, controlling proximity authentication, and checking the
current system status.

<p align="center">
  <picture>
    <source srcset="assets/deskunlock-settings-overview.webp" type="image/webp">
    <img src="assets/deskunlock-settings-overview.svg" alt="DeskUnlock desktop settings and configuration overview" width="100%">
  </picture>
</p>

The application provides quick access to activation controls, paired-device
management, security features, Bluetooth and service status, and recent
unlock information.

## Android components

| Component | Responsibility |
|---|---|
| Home / settings UI | Shows the paired computer, service state, history and revoke action. |
| Pairing screen | Projects the pairing state machine and the application confirmation. |
| `AndroidCdmPairCompanionScanner` | Companion Device Manager discovery, association and presence observation. |
| `RealPairBackend` / GATT exchange | Bluetooth bond trigger, authenticated public-key exchange and the versioned pairing transaction. |
| `BondStore` + Android Keystore | Stores the DeskUnlock trust record and the hardware-backed signing key. |
| `SyauthCompanionService` | Background service that receives unlock challenges and drives the approval activity. |
| Approve screen | Explicit approve/deny action gated by `BiometricPrompt`. |

## Supported beta scope

The validated beta scope is intentionally narrow:

- Arch Linux / CachyOS;
- Wayland session with the validated Niri + DankMaterialShell (DMS)
  integration;
- Android companion using Bluetooth LE and Companion Device APIs.

Other distributions, desktop environments, Android devices, and OEM ROMs may
work, but are not universal compatibility claims. Android BLE behavior,
battery management, and background-service policy vary by device and ROM.

## Quick start

The current build is the **[v0.1.0-beta.2 pre-release](https://github.com/Dendifra/DeskUnlock/releases/tag/v0.1.0-beta.2)**: a signed Android APK and an Arch/CachyOS package. Verify the published `SHA256SUMS` before installing. Because it is a pre-release, GitHub does not mark it as the latest release; open the [releases list](https://github.com/Dendifra/DeskUnlock/releases) to find it.

### 1. Install the Linux package

Download the `deskunlock-*-x86_64.pkg.tar.zst` asset, then:

```bash
sudo pacman -U deskunlock-0.1.0-71-x86_64.pkg.tar.zst
```

When Plasma Login is installed, the package wires its PAM service and orders
it after Bluetooth. The normal password fallback remains in the stack.

Use normal package-manager authentication. Do not use `--nodeps`,
`--overwrite`, or force options.

For a source checkout, see [docs/installation.md](docs/installation.md) and
[docs/packaging.md](docs/packaging.md).

### 2. Install the Android companion

Download the signed APK from the same release page and sideload it through
Android's normal package installer. Android may display a warning because this
beta is not distributed through Google Play. See
[docs/android-setup.md](docs/android-setup.md).

The release APK uses the dedicated DeskUnlock release certificate
(`CN=DeskUnlock Release`). Release signing material is private and is not stored
in this repository. Verify the published SHA-256 before installation.

### 3. Pair the devices

Pairing is confirmed on **both** devices, and the computer refuses an inbound
request it was not armed for — that is the mitigation against pairing a device
that is not yours, so the arm step is not optional.

1. On the computer, open `syauth-settings` and click **Associa telefono**. The
   window stays open and armed.
2. Open DeskUnlock on Android and tap **Pair**.
3. Select the computer. If the operating system asks for a Bluetooth numeric
   comparison, confirm that the numbers match. That is the transport step and is
   separate from the DeskUnlock confirmation.
4. Confirm the code shown in the computer's pairing window.
5. Confirm the desktop sees the bond:

   ```bash
   syauth list
   ```

If the computer refuses the request, the app now says so and tells you to arm
pairing on the computer first. "Nothing happens" is not an expected outcome.

The `syauth` command/file names are compatibility identifiers inherited from
upstream. The product and user-facing name is DeskUnlock.

### 4. Enable DeskUnlock

```bash
syauth-control on
syauth-control status
```

The first `on` may show a protected system authorization popup to enable the
user service at boot. No manual service restart is required.

### 5. Unlock normally

The normal flow is:

1. lock screen active;
2. no authentication from pointer initialization alone;
3. first real mouse movement, keyboard input, or Enter;
4. one DeskUnlock request;
5. Android biometric approval;
6. desktop unlock.

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

The security model is described in [docs/security-model.md](docs/security-model.md)
and [docs/security.md](docs/security.md). In short: the DeskUnlock bond key is
established by an authenticated application handshake, the phone signs each
approval with a hardware-backed key, and PAM accepts only a valid response for
a peer that holds DeskUnlock trust.

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
- **after a desktop daemon restart the phone does not always re-attach by
  itself.** The Bluetooth link still reads *connected*, but the GATT
  subscription is gone, so proximity and unlock stay dead until the app is
  reopened (force-stop, then launch). Symptom to recognise: `presence.last`
  missing from `/run/user/<uid>/syauth/`;
- **the first activation after a pairing can take up to ~20 seconds.** The phone
  connects while the computer is still serving its pairing-time GATT app, finds
  the wrong characteristics, and has to rebuild the connection;
- **repeated unpair/re-pair in quick succession** can need the same app reopen as
  the restart case above;
- the app cannot remove the operating system's own Bluetooth bond; do that from
  Android settings, or re-pair (which reuses it);
- other distributions and desktop environments are not claimed as supported.

See [docs/troubleshooting.md](docs/troubleshooting.md) for reversible checks.

## Languages

Both surfaces ship Italian and English in **one** build — there is no
per-language package and nothing to select.

| Surface | Follows | How to change it |
|---|---|---|
| Desktop GUI | the desktop locale (`LANGUAGE`/`LC_ALL`/`LANG`) | change the session language, or launch a single run with `env LANG=en_US.UTF-8 syauth-settings` |
| Android app | the phone's locale | Android settings, or the per-app language entry (Android 13+) |

A locale with no catalog — `de_DE`, say — falls back to English rather than to a
half-translated screen. English is not the source language of the GUI: the
source is Italian, so English exists because the shipped catalog provides it,
and `tests/settings_language.py` fails the suite if any string is left
untranslated.

## Compatibility identifiers

DeskUnlock is an independent project, but it intentionally preserves a number
of identifiers inherited from upstream `syauth` for internal and historical
compatibility. These names are **not** a statement that DeskUnlock is still
the operational fork of `syauth`:

- `pam_syauth.so`
- `syauth-presenced`
- `syauth-idle-lock`
- `/usr/lib/syauth`, `/usr/bin/syauth-*`
- Android namespace `com.sy.syauth.android`
- the `syauth://` URL scheme
- `SYAUTH_*` environment variables and constants
- BLE UUIDs and rotating session UUID derivation
- HKDF info strings and namespaces
- `bonds.toml`, key-file formats, journal and log formats
- protocol identifiers and version numbers

Renaming these would break installed packages, stored bonds, the Android app
identity and the wire protocol. They may only change as a deliberate,
tested migration.

## Origins and attribution

DeskUnlock was originally derived from
[`dmytrogajewski/syauth`](https://github.com/dmytrogajewski/syauth) and its
contributors.

DeskUnlock has since evolved into an independent project with substantial
changes to its architecture, desktop daemon, Android integration, pairing
transaction, recovery, presence and authentication workflow.

The original `syauth` code is licensed under the MIT License. See
[LICENSE](LICENSE) for copyright and licensing information.

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
