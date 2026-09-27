# Contributing to DeskUnlock

Contributions are welcome.

DeskUnlock is authentication software, so changes that affect PAM, pairing, cryptography, Bluetooth trust, privilege boundaries, or fallback behavior require extra care.

## Good contribution areas

- distro packaging;
- desktop/lock-screen adapters;
- Android compatibility;
- BLE/GATT recovery;
- PAM portability;
- test coverage;
- accessibility and GUI work;
- documentation and translations;
- security review.

## Development principles

1. **Fail closed for DeskUnlock itself, but preserve the configured normal authentication fallback.**
2. Do not add hidden network/cloud dependencies to the authentication path.
3. Do not commit private keys, bonds, device identifiers, MAC addresses, tokens, credentials, or personal paths.
4. Protocol changes require tests and documentation.
5. Security-sensitive behavior should have a regression test.
6. Avoid host-specific assumptions such as a fixed UID, username, home directory, phone model, MAC address, compositor, or distro.
7. Keep persistent user cryptographic state outside package payloads.
8. Prefer standard Linux mechanisms and explicit adapters over hardcoded desktop-specific behavior.

## Pull requests

A pull request should explain:

- what problem it solves;
- what behavior changes;
- what security boundary is touched, if any;
- how it was tested;
- whether backward compatibility is affected.

By submitting a contribution, you agree that your contribution may be distributed under the project's MIT license.

## Tests

The intended CI gate includes:

```text
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Additional desktop, Android, PAM, packaging, and clean-machine tests are expected as the project becomes portable.

## Security issues

Do not publish exploit details for an unpatched vulnerability in a normal issue. See `SECURITY.md`.
