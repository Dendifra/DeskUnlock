# Linux desktop integration

This directory contains the desktop-oriented integration layer developed by the
DeskUnlock fork on top of the `syauth` core.

It includes the currently tested command wrappers, health checks, proximity
logic, first-run/setup helpers, systemd user units, desktop launcher, PAM
package hook, and optional DMS integration used by the Arch/CachyOS package.

## Compatibility naming

Some files and commands still use the `syauth` name intentionally.

DeskUnlock is a fork of `syauth`, and renaming protocol identifiers, state
paths, PAM ABI/module names, UUIDs, Android identifiers, or other compatibility
surfaces without a migration plan could break existing pairings or upgrades.

Public-facing DeskUnlock naming will be migrated separately from compatibility
identifiers.
