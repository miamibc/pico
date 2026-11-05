# Contributing to Picco

Thanks for wanting to help. Bug reports, ideas, translations and code are all
welcome.

## Before you start

For anything bigger than a small fix, open an issue or a discussion first, so
we can agree on the approach before you spend time on it.

## Building and checking

You need Rust 1.95 or newer. See [Building and running](README.md#building-and-running)
in the README; in short:

```sh
cargo run --release -p picco -- ~/Pictures
make check    # formatting, clippy and all tests: run it before a pull request
```

The interface is tested without a window (`crates/picco/src/tests.rs`, with
`egui_kittest`); screenshots of the tests land in `target/ui-shots/`. A change
in behaviour comes with a test.

## Things to keep in mind

- **Picasa's files are sacred.** Picco reads and writes `.picasa.ini` so that
  Picasa 3 can still open the library. Never drop keys Picco doesn't know, and
  change only what the user changed.
- **Texts are translated.** Interface texts go through `tr!`. After adding or
  changing them, run `make po` and fill in `crates/picco/po/ru.po` (or leave
  it for a Russian speaker to do).
- **Few dependencies.** Ask before adding a crate; after changing
  dependencies, run `make licenses` to update the list in Help → About.
- **Commit messages** are one line saying what changed, in plain words.

## Translations

The texts are in `crates/picco/po/`. A new language is a copy of `picco.pot`
translated, plus one line in `crates/picco/src/i18n.rs`; the steps are in
[`crates/picco/po/README.md`](crates/picco/po/README.md#adding-a-language).

By contributing you agree that your work is licensed under the
[Apache License 2.0](LICENSE), like the rest of Picco.
