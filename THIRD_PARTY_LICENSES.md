# Third-party license inventory

This file is a release gate. A public binary release should not be published until all shipped or statically linked components have been reviewed.

| Component | Relationship | License status | Action |
|---|---|---:|---|
| `syauth` upstream | Source basis | MIT verified | Preserve original notice and MIT text |
| Rust crates | Build/runtime dependencies | **Audit pending** | Run `cargo deny check licenses` or equivalent and record results |
| Android/Gradle dependencies | Mobile dependencies | **Audit pending** | Generate dependency/license inventory |
| DMS integration | External desktop dependency/integration | **Audit pending** | Confirm dependency license and whether anything is redistributed |
| PySide6 / Qt | External GUI dependency | **Audit pending** | Confirm redistribution obligations for release format |
| BlueZ | System dependency | External | Document as dependency; do not claim ownership |
| PAM | System dependency | External | Document as dependency; do not claim ownership |
| systemd | System dependency | External | Document as dependency; do not claim ownership |

## Rule

System dependencies that are merely required at runtime are not automatically copied into this repository. Any third-party source, static binary, icon, font, image, vendored module, or generated asset that *is* redistributed must have its license reviewed and recorded here.

The release audit script intentionally flags common vendored/binary artifacts for manual review.

## DankMaterialShell

DeskUnlock's optional DMS lock-screen bridge is built from
`AvengeMedia/DankMaterialShell`, pinned to the source revision documented in
`desktop/dms/README.md`.

DankMaterialShell is MIT licensed.

Copyright (c) 2025 Avenge Media LLC

DeskUnlock does not vendor the prebuilt DMS executable; the bridge is rebuilt
from the upstream source plus the DeskUnlock integration.
