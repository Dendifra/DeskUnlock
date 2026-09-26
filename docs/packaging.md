# Packaging principles

## Public beta package

The validated beta package target is Arch Linux/CachyOS:

```bash
sha256sum deskunlock-0.1.0-18-x86_64.pkg.tar.zst
sudo pacman -U deskunlock-0.1.0-18-x86_64.pkg.tar.zst
```

Use normal package-manager dependency resolution. Do not use `--nodeps`,
`--overwrite`, or force options. The future release package includes legal
notices under `/usr/share/licenses/deskunlock/` and does not contain private
pairing state or signing material.

Source builds use the repository Makefile and normal pinned project tooling.
They are distinct from installing the signed public beta artifacts.

## Package owns code, not private state

The package manager should own:

- executables;
- daemon/helper binaries;
- PAM module;
- systemd units;
- desktop launcher;
- hooks needed to keep package-owned integration correct.

The package must not contain:

- private keys;
- bond databases copied from a developer machine;
- device MAC addresses;
- peer identifiers;
- user-specific logs;
- developer home paths;
- `.env` files or tokens.

## Upgrade

An upgrade should replace program files while preserving compatible persistent user state.

## Uninstall

Removing the package should remove package-owned application files.

Destructive deletion of private pairing/cryptographic state should require a separate explicit purge action, if the project ever provides one.

## Build reproducibility/privacy

Release builds should avoid embedding developer home paths in binaries or `.BUILDINFO`.

Before release, scan both payload and final package for:

- personal paths;
- usernames;
- hardware addresses;
- tokens/secrets;
- build-directory leakage.

## PAM integration on Arch install and upgrade

`deskunlock.install` invokes `/usr/lib/syauth/syauth-pam-sync install` from
`post_install` and `post_upgrade` when `plasmalogin` exists. The helper is
idempotent, preserves the `system-login` password fallback, and orders the
Plasma greeter after `bluetooth.service`. No `post_remove` callback strips PAM
configuration during package renames.

## Arch packaging

The initial downstream implementation has validated an Arch/CachyOS package approach. Before AUR publication, the PKGBUILD should build from public tagged source rather than rely on a private local payload archive.
