# GUI catalogs

`deskunlock.pot` is generated from `desktop/bin/syauth-settings` with:

```
xgettext --language=Python --keyword=_ --from-code=UTF-8 --add-comments \
  -o packaging/locale/deskunlock.pot desktop/bin/syauth-settings
```

## Do not install a catalog yet

55 of the 179 user-visible literals are wrapped in `_()` today: those reached
through a display call (`setText`, `QLabel`, `QPushButton`, `section_title`, …),
which is the set a script can wrap without risking a value the code compares
(`_("auto")` translated would silently break the language setting).

The other 124 sit in ternaries, in variables assigned to labels, in f-strings and
in dialog bodies. **Installing an `it` catalog now would translate those 55 and
leave 124 in the source language, inside one screen** — worse than the current
coherent Italian GUI. With no catalog present, `_()` is the identity and the GUI
reads exactly as before, which is why this state is safe to ship and the next one
is not.

Order: wrap the remaining 124 by hand, re-run `xgettext`, then add the catalogs.
