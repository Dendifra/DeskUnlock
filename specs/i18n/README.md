# i18n work notes

## The rule (operator, 2026-09-24)

Translate **only what the operator sees on screen**. Docstrings, comments and
internal identifiers are not copy and are never translated. Tokens that read the
same in both languages stay as they are — `auto`, `login`, `DeskUnlock`,
`Bluetooth`, `Proximity`, `Keystore`, `PAM`, `systemd`.

`auto` is the sharp edge of this rule: it is a value the code compares, so it
must **never** be wrapped in `_()`. A translated `auto` would break the language
setting silently, with no error and no crash.

## State

| Surface | State |
|---|---|
| Android app | **bilingual** — 56 keys in `values/` (English source, default) and `values-it/` |
| Settings GUI — plumbing | **done** — `resolve_language()`, the selector under the theme button, `write_gui_config` merging instead of rewriting, `main()` installing the catalog before the first label is built |
| Settings GUI — copy | **in progress** |

Copy numbers, measured with Python's AST (not a regex — see below):

- **145** visible strings
- **51** wrapped in `_()` (arguments of display calls, which a script can take safely)
- **10** that must **never** be wrapped, because the code compares them
- **16** clearly displayed and still to wrap
- **68** that need a human call: `REVIEW` in the worklist

`gui-strings.tsv` is the worklist: `line`, `state`, `text`. The states are
`wrapped`, `TODO` (wrap it), `DO-NOT-WRAP` (a compared value) and `REVIEW`.

## Why "by hand" is a judgement, not typing

The ten `DO-NOT-WRAP` entries are the whole argument. `if "Syauth: ON" in output:`
matches text printed by `syauth-control`; wrapping that literal in `_()` means a
catalog can rename it, the comparison stops matching, and the master switch reads
**OFF forever** — no error, no crash, no test failure. A script cannot tell that
`"Syauth: ON"` is a parse target while `"Servizio non disponibile"` two hundred
lines below is copy. That distinction is the work.

The same split appears inside one construct: in `"NEAR": "Vicino"` the **key** is a
state code the daemon emits and must stay, while the **value** is what the operator
reads and must be translated.


## No catalog ships until the 94 are done

With no catalog, `_()` is the identity and the GUI reads exactly as it does today.
Installing one now would translate 51 strings and leave 94 in Italian **on the
same screen** — worse than the current coherent Italian GUI. That is why
`packaging/locale/` holds only the generated `.pot` and a README saying so.

## The pass, in order

1. Wrap the 94 in `_()`.
2. Re-run the `xgettext` line in `packaging/locale/README.md`.
3. `msginit` for `it`; fill the `msgstr`s with the Italian from `gui-strings.tsv`.
4. `msgfmt` to `deskunlock.mo` under `packaging/locale/<lang>/LC_MESSAGES/`.
5. `PKGBUILD` installs `usr/share/locale/<lang>/LC_MESSAGES/deskunlock.mo`, built
   **before** `makepkg`.
6. A test asserting every `msgid` has a non-empty `msgstr`: a missing translation
   must fail the suite, never reach the screen.
