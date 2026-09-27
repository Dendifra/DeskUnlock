# Publication plan

## Phase 1 — staging and audit

1. Copy the current source into a separate `DeskUnlock` staging tree.
2. Preserve the original working installation and current package.
3. Run privacy/identity scan.
4. Run license inventory.
5. Record desktop-specific assumptions.
6. Decide compatibility boundaries for internal `syauth` protocol/state names.

## Phase 2 — branding

Rename public-facing elements first:

- product/UI name -> DeskUnlock;
- package -> `deskunlock`;
- desktop file -> `deskunlock.desktop`;
- public docs/repository -> DeskUnlock.

Do **not** blindly rename:

- wire protocol UUIDs/identifiers;
- existing state paths;
- Android compatibility identifiers;
- database formats;
- PAM ABI-sensitive names;

until migration/backward compatibility is specified.

## Phase 3 — portability

- remove fixed UID/username assumptions;
- parameterize desktop integration;
- make DMS integration optional or clearly packaged as an adapter;
- clean first-run provisioning;
- source-built packaging;
- test on a clean Arch/CachyOS machine.

## Phase 4 — public repository

Add:

- README;
- LICENSE;
- NOTICE;
- SECURITY;
- CONTRIBUTING;
- third-party license inventory;
- architecture/security/install/troubleshooting docs;
- issue/PR templates.

## Phase 5 — release

- tag `v0.1.0-beta.1`;
- CI from source;
- attach package artifact;
- publish checksums;
- invite testing and contributions;
- track compatibility reports.
