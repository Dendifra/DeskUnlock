# Installation

## Status

DeskUnlock is currently being prepared for its first public beta.

Do not present the current downstream package as universally portable until the clean-machine audit passes.

## First target: Arch / CachyOS

The intended package name is:

```text
deskunlock
```

The package should own application code, helpers, PAM module, systemd user units, desktop launcher, and packaging hooks.

Persistent pairing and cryptographic state must remain outside the package payload.

## Public release workflow target

For an initial GitHub release:

```text
source tag
   ↓
CI build + tests
   ↓
Arch package artifact
   ↓
GitHub Release
```

A later AUR package can build from a tagged source release.

## Clean-machine test checklist

Test on a fresh environment:

1. install required dependencies;
2. install DeskUnlock package;
3. open the GUI;
4. first-run state provisioning;
5. pair one phone;
6. confirm biometric authentication;
7. confirm password fallback;
8. reboot;
9. confirm automatic startup;
10. upgrade package;
11. uninstall package;
12. verify persistent private state is not unexpectedly deleted.

## Proximity Lock

La GUI DeskUnlock espone Proximity Lock con i profili **Vicino**, **Bilanciato** e **Ampio**. Il profilo Bilanciato è predefinito. Il baseline RSSI viene appreso localmente dopo più campioni e viene invalidato quando cambia il telefono associato.

Lo stato normale mostra solo `Vicino`, `Intermedio`, `Lontano` o `Assente`. Per la diagnostica avanzata, eseguire:

```bash
syauth-proximity diagnostics
```

I campioni RSSI e lo stato operativo restano in `XDG_RUNTIME_DIR`; la configurazione persistente contiene solo profilo, abilitazione, versione algoritmo e baseline numerico. Pairing, cambio e dissociazione resettano il baseline senza salvare identificatori del telefono. Per richiedere un nuovo apprendimento locale:

```bash
syauth-proximity reset
```

## Other distributions

Debian, Fedora, openSUSE and other distributions are contribution targets. They are not yet claimed as supported.
