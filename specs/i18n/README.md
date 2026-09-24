# i18n work notes

## State

| Surface | State |
|---|---|
| Android app | **bilingual** — 56 keys in `values/` (English source, default) and `values-it/`, read through `stringResource`/`getString` |
| Settings GUI | **seam ready** — `resolve_language()` fixes the order (operator choice, then locale, then English), the selector sits under the theme button, `write_gui_config` merges instead of rewriting, and `main()` installs the catalog before the first label is built |
| Settings GUI copy | **not done** — see below |

## `gui-strings.tsv`

Every distinct user-visible literal in `desktop/bin/syauth-settings`, extracted with Python's
AST (not a regex: the nested and interpolated cases are where a regex lies). `line<TAB>text`.

Counts: 179 distinct literals over 161 lines. That number includes docstrings and
technical tokens the screen never shows (`NEAR`, `ABSENT`, `Powered`, `Mai`, colour
strings), so the copy that actually needs translating is a subset — but the list is
the honest starting inventory, not an estimate.

## Why the copy is not done yet

The source is Italian today. Making English the source means **two** lists of the
same size: the 179 rewritten in English in the file, and the Italian preserved in
the catalog. Doing part of it produces an English screen with Italian holes, which
is worse than the current coherent Italian screen — so it goes in one pass or not
at all.

## The pass, in order

1. Rewrite the copy in `desktop/bin/syauth-settings` in English and wrap every call
   site in `_()`.
2. `xgettext --language=Python --keyword=_ -o deskunlock.pot desktop/bin/syauth-settings`,
   then `msginit` for `it` and fill the `msgstr`s with the Italian from this inventory.
3. `msgfmt` to `deskunlock.mo` into `packaging/locale/<lang>/LC_MESSAGES/`.
4. `PKGBUILD`: install `usr/share/locale/<lang>/LC_MESSAGES/deskunlock.mo`, and build
   the `.mo` **before** `makepkg`, not during.
5. A test that every `msgid` in the source has a non-empty `msgstr` in the `it`
   catalog: a missing translation must fail the suite, never reach the screen.
