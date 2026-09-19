# Packaging principles

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

## Arch packaging

The initial downstream implementation has validated an Arch/CachyOS package approach. Before AUR publication, the PKGBUILD should build from public tagged source rather than rely on a private local payload archive.
