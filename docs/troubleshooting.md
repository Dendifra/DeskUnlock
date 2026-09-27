# Troubleshooting

Use read-only checks first. Replace example hostnames and peer IDs with your
own values; do not paste keys, tokens, raw bond files, MAC addresses, or full
home paths into public reports.

## Phone is not paired

On the computer:

```bash
syauth list
syauth-control status
systemctl --user is-active syauth-presenced.service
```

If no bonded phone is listed, start pairing again:

```bash
syauth pair --adapter hci0
```

Then open DeskUnlock, tap **Pair**, select the computer, and complete both
confirmation steps. Do not delete system state as a first step.

## No unlock request appears

Check that:

- Bluetooth is powered on on both devices;
- the phone is nearby and unlocked enough to receive the request;
- `syauth-control status` is healthy;
- the user services are active:

  ```bash
  systemctl --user is-active syauth-presenced.service
  systemctl --user is-active syauth-proximity.service
  ```

The lock surface appearing does not itself start authentication. Move the
mouse, press a key, or press Enter once. A genuine interaction should create
one request. Do not expect a request from pointer initialization alone.

## Biometric prompt does not appear

Check that Android notifications are allowed for DeskUnlock and that the app
is not restricted by the device's battery manager. Confirm that the phone has
an enrolled biometric or device credential and that the screen is usable.

If the request expired, interact with the lock screen again later. Do not
repeatedly tap approval buttons or alter Android security settings.

## Bluetooth is disabled or the phone is out of range

Restore Bluetooth and bring the phone into normal range, then wait for the
companion association to recover. Password/PAM fallback remains available when
the phone or Bluetooth path is unavailable.

## Phone login is unavailable immediately after boot

Check the boot ordering and readiness socket:

```bash
systemctl is-active bluetooth.service
systemctl is-active plasmalogin.service
test -S /run/user/$(id -u)/syauth/auth.sock && echo ready
```

The package orders Plasma Login after Bluetooth and `syauth-control on` enables
user-service persistence at boot. If the drop-in is missing, run
`sudo /usr/lib/syauth/syauth-pam-sync install`. Password fallback remains
available while the phone transport is unavailable.

## Service is not running

Read-only checks:

```bash
systemctl --user status syauth-presenced.service
systemctl --user status syauth-proximity.service
journalctl --user -u syauth-presenced.service -b --no-pager
syauth-control status
```

If DeskUnlock was intentionally disabled, re-enable it with:

```bash
syauth-control on
```

## Lock integration is unavailable

The validated beta integration is Niri + DankMaterialShell on Arch/CachyOS.
Check the DMS user service and session before changing anything. Other
compositors and distributions are not claimed as supported by this beta.

## App was replaced or reinstalled

A pre-beta/debug APK and the release-signed beta APK use different signing
identities. A one-time uninstall/reinstall may be required. Android app-private
state can be removed by uninstalling, so pair the computer and phone again
after reinstalling. Do not use signature-bypass tools or disable Play Protect
globally.

## Request expired

An approval request has a bounded lifetime. Let it expire, then make a new
genuine local interaction with the lock screen. The phone must be available
and the Android app must be allowed to operate in the background.

## Password fallback

Fallback to the normal PAM password path is expected when the phone is absent,
Bluetooth is unavailable, the service is down, or the request is denied or
expired. A DeskUnlock failure must not make the configured normal PAM path
unusable.

## Bug reports

Include:

- Linux distribution and version;
- desktop/session and whether it is the validated Niri + DMS path;
- DeskUnlock version;
- Android version and phone model;
- sanitized service status and relevant journal excerpts.

Never include keys, tokens, bond secrets, raw private state, hardware
addresses, signing material, or unnecessary personal paths.
