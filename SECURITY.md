# Security policy

DeskUnlock is authentication software and is currently beta software
(`v0.1.0-beta.1`). Do not treat it as independently certified or as a
replacement for evaluating the security of the Linux host and phone.

## Reporting a vulnerability

If GitHub Private Vulnerability Reporting is enabled, use it for
security-sensitive reports. Otherwise open a minimal non-sensitive issue
asking the maintainer for a private contact path.

Do not publish keys, tokens, bond secrets, private device identifiers,
signing material, exploit details, or proof-of-concept material in a public
issue.

## Security scope

Please report concerns involving:

- authentication bypass or fail-open behavior;
- replay, relay, signature, challenge, or pairing validation;
- incorrect PAM return behavior or privilege boundaries;
- leakage of key material or unsafe state permissions;
- authentication of the wrong user or device.

DeskUnlock uses local BLE/GATT communication and Android biometric/Keystore
APIs. No DeskUnlock cloud account or developer-operated backend is required.
The Android app does not request `INTERNET` in the committed manifest.

The release APK is signed with a dedicated DeskUnlock release certificate. The
private release key is outside this repository and must never be committed or
published. Users should verify the SHA-256 checksum published with a beta APK.

## Limitations

No independent professional security audit has been completed. DeskUnlock
cannot guarantee safety against:

- a compromised Linux host, PAM stack, or root account;
- a compromised or physically coerced phone;
- an attacker who knows the phone's device credential;
- malicious or broken Android, Bluetooth, DMS, or other operating-system
  components;
- physical attacks outside the normal host and phone threat model.

The configured password or other PAM fallback should remain available when
DeskUnlock is unavailable. See [docs/security-model.md](docs/security-model.md)
for the operator-facing model.
