# Translations

Pico's interface texts are translated with gettext `.po` files: the English
text in the code is the key, and each language is a file here (`ru.po`).
They are built into the program; nothing is read at run time.

- `pico.pot` — the template: every text to translate. Made by `make po`;
  do not edit.
- `<language>.po` — one language. Edit it in any `.po` editor (Poedit,
  Lokalize, Weblate) or by hand.
- `update.py` — what `make po` runs.

## Changing texts in the code

Mark interface texts in `crates/pico/src`:

- `tr!("Save")` — a text;
- `tr!("Saved {path}", path = path.display())` — with values filled in;
- `trn!("{n} photo", "{n} photos", count)` — a text that depends on a count;
- `tr_mark!("Original size")` — a text kept in a table, translated where it is
  shown with `crate::i18n::tr(text)`.

The names of effects and collage layouts in `crates/pico-core` are collected
too. Then run `make po`: new texts come into every `.po` untranslated, texts no
longer used are dropped. `make check` fails while a text has no Russian
translation.

## Adding a language

1. Copy `pico.pot` to `<code>.po` (`de.po`, `uk.po`…), fill in the header's
   `Language` and translate.
2. Add the language to `LANGUAGES` in `crates/pico/src/i18n.rs`, and its plural
   rule to `plural_form` if it is not there yet.

Pico speaks the system's language when it has it, English otherwise;
`language=ru` in `~/.config/pico/settings.txt` picks one.
