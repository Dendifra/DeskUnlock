# GitHub setup checklist

Suggested repository:

```text
Dendifra/DeskUnlock
```

Suggested description:

> Desktop-focused smartphone authentication and proximity unlock for Linux — an independent fork of syauth.

Suggested topics:

```text
linux
authentication
pam
bluetooth
ble
android
biometrics
security
phone-as-key
wayland
arch-linux
cachyos
```

## Repository settings

Enable:

- Issues;
- Pull Requests;
- Discussions;
- GitHub Actions after CI is reviewed;
- Private Vulnerability Reporting;
- Dependabot/security alerts where applicable.

Recommended default branch:

```text
main
```

Recommended merge policy:

- squash merge allowed;
- require CI before merge once CI is stable;
- avoid force pushes to `main`.

## First public release

Do not publish a binary release until:

- `PUBLICATION_AUDIT.md` is green;
- `THIRD_PARTY_LICENSES.md` is complete;
- clean-machine install works;
- reboot works;
- password fallback works;
- no private identifiers are found;
- source corresponding to the binary is public.

Suggested first tag:

```text
v0.1.0-beta.1
```
