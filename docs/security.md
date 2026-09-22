# DeskUnlock security model

DeskUnlock `v0.1.0-beta.1` is beta authentication software. This operator guide
summarizes intended properties; it is not an independent security audit.

## Should you use DeskUnlock?

DeskUnlock can replace repeated password entry with an explicit approval on a
nearby Android phone. The phone still requires biometric or device-credential
approval. The configured password remains the fallback when the phone,
Bluetooth, or DeskUnlock service is unavailable.

The validated beta scope is Arch Linux/CachyOS with Niri + DankMaterialShell.
Other platforms may work but are not universal compatibility claims.

## What it protects against

- password entry during an ordinary DeskUnlock approval;
- replay of an already accepted unlock request;
- an unlock based on proximity alone;
- a phone notification being sufficient without explicit approval.

The desktop creates a fresh request and the Android app protects its signing
operation with Android biometric/device-credential APIs. Communication is local
BLE/GATT; no DeskUnlock cloud account or developer-operated backend is needed.
The Android manifest does not request `INTERNET`.

## What it does not protect against

- a compromised Linux host, PAM stack, or root account;
- a compromised or physically coerced phone;
- an attacker who knows the phone's device credential;
- relay, jamming, or other attacks outside the validated threat model;
- malicious or broken Android, Bluetooth, DMS, or operating-system components;
- a stolen phone whose screen protection has been defeated.

DeskUnlock has not received an independent professional security audit.

## Operational hygiene

- Keep Linux and Android security updates current.
- Keep USB debugging disabled on the phone unless it is actively needed.
- Review `syauth list` and revoke retired or unknown peers with
  `syauth revoke <peer-id>`.
- Pair in a private environment and verify both pairing confirmations.
- Keep the normal PAM password fallback in the stack.
- Never publish pairing state, keys, signing material, or raw diagnostic logs.

The public beta APK uses a dedicated DeskUnlock release certificate. Its
private key is outside this repository and must never be committed or
published. Verify the published SHA-256 checksum before installation.

## Source details

The protocol-level threat material is in
[`specs/threat/THREAT-2026-05-15.md`](../specs/threat/THREAT-2026-05-15.md).
The main implementation areas are `crates/syauth-core`, `crates/syauth-pam`,
and `crates/syauth-cli`.
