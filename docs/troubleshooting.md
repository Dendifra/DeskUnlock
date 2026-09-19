# Troubleshooting

DeskUnlock should expose health information that distinguishes application failure from ordinary phone absence.

## Basic checks

Check package installation, user units, Bluetooth, pairing state, and the health command supplied by the current build.

During the branding migration, some internal commands may still retain `syauth` names. Public documentation should be updated only after the command rename is complete.

## Common categories

### Phone not detected

Check:

- Bluetooth is powered;
- the expected phone is bonded;
- the presence daemon is active;
- the phone has connected/subscribed to the GATT service.

### Authentication falls back to password

This can be normal if:

- the phone is absent;
- Bluetooth is unavailable;
- the daemon is unavailable;
- the phone-side biometric request is not approved.

The fallback itself is an important safety property.

### GUI says degraded but backend is healthy

Compare GUI state parsing with the actual backend status output. A UI parser must not assume a different output format than the control command provides.

### Service does not start after reboot

Inspect the systemd user unit source and current-boot journal. Packaged units should come from the package-owned system location rather than stale user-local copies.

### Multiple bonded peers

The current downstream policy is single-device. More than one active bonded peer should be treated as degraded until the policy is intentionally changed.

## Bug reports

Include:

- distro and version;
- desktop/session;
- DeskUnlock version;
- phone OS/version;
- sanitized service status;
- sanitized logs.

Never post keys, tokens, bond secrets, raw private state, full home paths if unnecessary, or hardware addresses unless a maintainer explicitly requests a safely redacted diagnostic.
