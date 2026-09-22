# DeskUnlock security model

DeskUnlock `v0.1.0-beta.1` is beta software. This document summarizes the
downstream project's intended security properties; it is not a substitute for
an independent professional security audit.

## Goals

DeskUnlock aims to provide a phone-assisted Linux authentication path where proximity alone is not sufficient.

The design inherited from upstream `syauth` uses cryptographic challenge-response,
local BLE/GATT communication, and Android-side biometric/device-credential
authorization for signing. No DeskUnlock cloud account or developer-operated
backend is required.

## Important properties

- A paired phone being nearby is not, by itself, authorization.
- Authentication uses a fresh challenge rather than a reusable unlock token.
- The phone-side signing key is intended to remain protected by Android Keystore mechanisms.
- PAM integration must not silently grant access when DeskUnlock is unavailable.
- Normal password or another configured PAM fallback remains available.
- Persistent key/bond state is not shipped inside the application package.
- The Android manifest does not request `INTERNET`.
- The desktop implementation should authenticate the PAM user, not assume a fixed UID.
- The Android biometric decision remains inside Android biometric/Keystore APIs.

## Failure behavior

Failures such as:

- Bluetooth unavailable;
- phone absent;
- GATT unavailable;
- daemon down;
- authentication socket missing;

must not turn into a DeskUnlock success.

The configured normal PAM authentication path should remain usable.

## Threat boundaries

The project relies on:

- Linux host integrity;
- the security of the dedicated release certificate; its private key is outside
  the repository and is not part of DeskUnlock's source distribution;
- correct PAM configuration;
- BlueZ behavior;
- Android device integrity;
- Android Keystore/biometric enforcement;
- correctness of cryptographic dependencies.

A compromised root account on the Linux host is outside the ability of a PAM helper to fully defend against.

## Proximity lock

Proximity may be used as a *locking* signal. It must not be treated as sufficient proof to *unlock* without the cryptographic/biometric authentication path.

## Release requirements

Before public binary releases:

- dependency licenses must be inventoried;
- personal/device identifiers must be absent;
- clean-machine install and fallback tests must pass;
- package permissions must be reviewed;
- security-sensitive changes must be regression-tested.
