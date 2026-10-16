# BUG-20261016: DMS binary embeds an upstream home path

## Summary
The Arch package must not embed the upstream developer's home-directory path in `dms-syauth`.

## Reproduction
- Method: inspect the built `usr/bin/dms-syauth` strings.
- Evidence: the README command containing `DankMaterialShellGit/translations` is present in the ELF; two generic greeter examples are present separately.

## Expected Behavior
The packaged binary contains no identifying home path from upstream documentation; generic examples remain unchanged.

## Actual Behavior
The embedded DMS shell tree includes a README line with a non-placeholder home component.

## Root Cause Analysis
`make sync-shell` copies `quickshell/` into `core/internal/shellembed/dist`; `//go:embed all:dist` embeds that tree into `dms-syauth`. `-trimpath` does not remove source document contents.

## Fix
Sanitize exactly one anchored README path in the temporary upstream checkout before compilation, failing closed on zero or multiple matches. Keep the greeter examples untouched.

## Traceability
- Regression test: `desktop/tests/test_dms_privacy_paths.py`
- Fixed in: `desktop/dms/build-dms-syauth.sh`, `desktop/dms/sanitize_dms_translation_readme.py`
