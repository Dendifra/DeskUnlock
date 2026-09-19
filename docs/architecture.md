# Architecture

## Components

DeskUnlock is split into layers so desktop integration can evolve without changing the authentication core.

### Authentication core

The upstream-derived core provides:

- pairing/bond records;
- BLE/GATT transport;
- challenge-response;
- signature verification;
- presence daemon;
- PAM module;
- CLI functionality.

### Desktop integration layer

The downstream project adds operational integration:

- settings GUI;
- master ON/OFF control;
- first-run setup;
- pairing workflow;
- proximity lock watcher;
- reconciliation;
- health checks;
- systemd user units;
- lock-screen bridge.

The original tested environment uses DMS for lock-screen integration. This should become an explicit adapter rather than an assumption in the portable project.

## State separation

Program files belong to the package manager.

Persistent private state does not.

Target conceptual separation:

```text
/usr/bin/...                 packaged executables
/usr/lib/...                 daemon, PAM module, helpers
/usr/lib/systemd/user/...    packaged user units

/var/lib/deskunlock/...      persistent pairing / cryptographic state
/var/log/deskunlock/...      persistent audit/log state if used
/run/user/<uid>/...          runtime authentication socket
/run/deskunlock/...          runtime markers if required
```

The exact rename/migration of existing `syauth` state paths is a compatibility decision. Do not blindly rename state directories until migration behavior is specified and tested.

## Boot/session flow

```text
user session starts
   │
   ├─> bootstrap
   │
   ├─> presence daemon
   ├─> proximity watcher
   ├─> reconcile path/service
   ├─> health timer
   └─> lock-screen integration
```

The presence daemon registers with BlueZ, exposes the authentication socket, and handles the paired phone transport.

## Authentication flow

```text
PAM authentication request
        │
        ▼
DeskUnlock PAM module
        │
        ▼
user runtime socket
        │
        ▼
presence daemon
        │ BLE challenge
        ▼
paired phone
        │ biometric confirmation + signature
        ▼
presence daemon verifies response
        │
        ▼
PAM success

If DeskUnlock is unavailable:
PAM returns unavailable/failure as designed -> normal configured fallback continues.
```

## Compatibility rule

Product branding and package names may become `DeskUnlock` without immediately changing every wire-level identifier inherited from `syauth`. Protocol/state renames must be deliberate migrations, not search-and-replace operations.
