# Fork relationship

DeskUnlock is an independent fork of the MIT-licensed
[`syauth`](https://github.com/dmytrogajewski/syauth) project.

## Upstream

- Project: `syauth`
- Upstream repository: https://github.com/dmytrogajewski/syauth
- License: MIT
- Original copyright: `Copyright (c) 2026 syauth contributors`

## DeskUnlock focus

DeskUnlock keeps the phone-as-key authentication foundation while focusing on
a complete Linux desktop experience:

- desktop-oriented settings and onboarding;
- packaging and upgrade safety;
- proximity-aware lock behavior;
- systemd user integration;
- health diagnostics;
- single-device UX;
- normal PAM password fallback;
- broader distro/desktop portability.

DeskUnlock is not presented as an official upstream release.

## Compatibility policy

Public branding can use the DeskUnlock name while internal protocol, state,
UUID, ABI, or compatibility identifiers inherited from `syauth` may remain
unchanged when renaming them would break compatibility.

Any such retained identifiers should be documented rather than changed only
for cosmetic consistency.
