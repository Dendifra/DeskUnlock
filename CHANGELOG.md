# Changelog

## [0.2.0] - 2026-10-01

- Add validated KDE Plasma lock-screen integration alongside Niri + DMS.
- Make desktop lock/authentication session-aware without starting DMS inside Plasma.
- Start biometric approval from genuine local lock-screen interaction rather than proximity alone.
- Update Arch/CachyOS and Android companion release metadata.

## [0.1.2] - 2026-09-28

- Harden the public repository privacy and history checks.
- Add deeper release, reference, and GitHub Actions supply-chain auditing.
- Sanitize public device and build-environment identifiers.
- Pin release workflow actions and keep release publication operator-controlled.
- Carry forward the functional fixes shipped in 0.1.1.

## [0.1.1] - 2026-09-26

- Retry BlueZ GATT initialization after a transient boot race.
- Recreate the DMS loaded-tree marker without restarting DMS when the tree is already adapted.
- Keep the GUI readiness indicator correct after daemon restarts.

## [0.1.0-beta.1] - 2026-09-22

First DeskUnlock public beta candidate.

- Linux desktop unlock integration with PAM fallback;
- Android companion with explicit biometric approval;
- BLE/GATT pairing and local phone-to-computer communication;
- proximity-aware locking and local re-engagement after lock-screen input;
- validated Arch Linux/CachyOS packaging scope;
- validated Niri + DankMaterialShell lock-screen integration;
- DeskUnlock branding across the desktop and Android companion;
- user-facing DeskUnlock notification wording;
- local privacy guardrails and release-signing separation;
- bundled third-party license and notice inventory.

This is beta software. See the README, installation guide, security policy,
and [the beta release notes](docs/releases/v0.1.0-beta.1.md) before use.
