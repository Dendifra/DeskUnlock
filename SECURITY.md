# Security policy

DeskUnlock is authentication software.

## Supported versions

Until the project reaches a stable release, only the most recent public pre-release should be considered supported.

## Reporting a vulnerability

If GitHub Private Vulnerability Reporting is enabled for the repository, use it for security-sensitive reports.

If private reporting is not yet enabled, do **not** place secrets, exploit details, private device identifiers, or proof-of-concept material in a public issue. Open a minimal non-sensitive issue asking the maintainer for a private contact path.

## Security expectations

A security report is especially important if it concerns:

- authentication bypass;
- fail-open behavior;
- replay or relay acceptance;
- signature or challenge validation;
- pairing/bond substitution;
- incorrect PAM return behavior;
- privilege-boundary mistakes;
- leakage of key material;
- unsafe package/state permissions;
- unintended authentication of the wrong user.

## Scope and limitations

DeskUnlock has not received an independent professional security audit.

The project depends on the security properties of the Linux host, PAM configuration, BlueZ, the Android device, Android Keystore/biometric mechanisms, and the cryptographic libraries it uses.

See `docs/security-model.md`.
