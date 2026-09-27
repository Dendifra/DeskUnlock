# DeskUnlock roadmap

## 0.1.0 — first public beta

Release gates:

- [ ] complete portability audit;
- [ ] complete third-party license inventory;
- [ ] remove personal paths, usernames, MAC addresses and test secrets;
- [ ] decide which internal `syauth` protocol identifiers remain for compatibility;
- [ ] apply DeskUnlock product/package/GUI branding;
- [ ] make desktop-specific integration optional or clearly scoped;
- [ ] build package from source rather than opaque local artifacts;
- [ ] clean install in a fresh Arch/CachyOS VM or machine;
- [ ] upgrade test;
- [ ] uninstall test preserving private state;
- [ ] reboot test;
- [ ] password fallback test;
- [ ] phone absent / Bluetooth unavailable test;
- [ ] document Android app build/install path;
- [ ] enable GitHub security reporting;
- [ ] publish source before or together with binary release.

## 0.2.x

- broader Arch testing;
- KDE/GNOME integration work;
- Debian/Fedora packaging prototypes;
- additional Android devices;
- automated package CI;
- better diagnostics;
- translation framework.

## 1.0

Target only after the authentication model, package/state format, upgrade path, and public interfaces are considered stable.
