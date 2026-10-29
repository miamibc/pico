//! Translations of the interface, gettext style: the English text is the
//! key, and each language is a `.po` file in `crates/pico/po`, built into the
//! program. `make po` collects the texts from the code into `pico.pot` and
//! the `.po` files (see `po/README.md`).
//!
//! Texts are marked with [`tr!`] (and [`trn!`] for counts); texts kept
//! elsewhere, such as tables, with [`tr_mark!`] and translated where shown
//! with [`tr`].

use std::collections::HashMap;
use std::sync::RwLock;

/// A text in the language in use; with `name = value` pairs, filled in.
macro_rules! tr {
    ($text:literal) => {
        $crate::i18n::tr($text)
    };
    ($text:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::fill($crate::i18n::tr($text), &[$((stringify!($name), &$value as &dyn std::fmt::Display)),+])
    };
}

/// The form of a text for the count `n`, with `{n}` and any other `name =
/// value` pairs filled in.
macro_rules! trn {
    ($one:literal, $other:literal, $n:expr $(, $name:ident = $value:expr)* $(,)?) => {{
        let n: usize = $n;
        $crate::i18n::fill($crate::i18n::trn($one, $other, n), &[("n", &n as &dyn std::fmt::Display) $(, (stringify!($name), &$value as &dyn std::fmt::Display))*])
    }};
}

/// Marks a text for translation where it is kept (a table); it is
/// translated where shown, with [`tr`].
macro_rules! tr_mark {
    ($text:literal) => {
        $text
    };
}

/// The languages Pico speaks: code, name in that language, its `.po` file.
pub const LANGUAGES: &[(&str, &str, &str)] = &[("en", "English", ""), ("ru", "Русский", include_str!("../po/ru.po"))];

/// One language's translations.
#[derive(Default)]
struct Catalog {
    code: String,
    /// msgid → msgstr, or the plural forms by msgid (the singular).
    texts: HashMap<String, Vec<String>>,
}

static CATALOG: RwLock<Option<&'static Catalog>> = RwLock::new(None);

/// The code of the system's language among [`LANGUAGES`]; English when
/// Pico does not speak it.
pub fn system_language() -> &'static str {
    let base = base_language(&sys_locale::get_locale().unwrap_or_default());
    LANGUAGES.iter().find(|l| l.0 == base).map_or("en", |l| l.0)
}

/// `ru` for `ru`, `ru_RU.UTF-8` or `ru-RU`.
fn base_language(code: &str) -> String {
    code.split(['_', '-', '.']).next().unwrap_or_default().to_lowercase()
}

/// Speaks the language `code` (`ru`, `ru_RU.UTF-8`…); `None`, or one Pico
/// does not know, picks the system's; English when that is unknown too.
pub fn set_language(code: Option<&str>) {
    let base = match code {
        Some(code) => base_language(code),
        None => system_language().to_owned(),
    };
    let po = LANGUAGES.iter().find(|l| l.0 == base).map_or("", |l| l.2);
    // Each language is read once or so in a run; it stays for the program's life.
    let catalog: &'static Catalog = Box::leak(Box::new(Catalog { code: base, texts: parse_po(po) }));
    *CATALOG.write().unwrap_or_else(|e| e.into_inner()) = Some(catalog);
}

fn catalog() -> Option<&'static Catalog> {
    *CATALOG.read().unwrap_or_else(|e| e.into_inner())
}

/// `text` in the language in use; itself where there is no translation.
pub fn tr(text: &str) -> &str {
    match catalog().and_then(|c| c.texts.get(text)).and_then(|forms| forms.first()) {
        Some(translated) if !translated.is_empty() => translated,
        _ => text,
    }
}

/// The form for `n` of `one`/`other` in the language in use.
pub fn trn<'a>(one: &'a str, other: &'a str, n: usize) -> &'a str {
    let Some(catalog) = catalog() else { return if n == 1 { one } else { other } };
    match catalog.texts.get(one).and_then(|forms| forms.get(plural_form(&catalog.code, n))) {
        Some(translated) if !translated.is_empty() => translated,
        _ if n == 1 => one,
        _ => other,
    }
}

/// Puts the values in place of `{name}` in `text`.
pub fn fill(text: &str, values: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut text = text.to_owned();
    for (name, value) in values {
        text = text.replace(&format!("{{{name}}}"), &value.to_string());
    }
    text
}

/// "May 2018" for a `YYYY:MM` key.
pub fn month_label(key: &str) -> String {
    const MONTHS: [&str; 12] = [
        tr_mark!("January {year}"),
        tr_mark!("February {year}"),
        tr_mark!("March {year}"),
        tr_mark!("April {year}"),
        tr_mark!("May {year}"),
        tr_mark!("June {year}"),
        tr_mark!("July {year}"),
        tr_mark!("August {year}"),
        tr_mark!("September {year}"),
        tr_mark!("October {year}"),
        tr_mark!("November {year}"),
        tr_mark!("December {year}"),
    ];
    fill(tr(MONTHS[month_index(key)]), &[("year", &key.get(..4).unwrap_or_default())])
}

/// "14 March 2021, 09:05" for a `YYYY:MM:DD HH:MM:SS` date; months take
/// another form after a day in some languages.
pub fn date_label(date: &str) -> String {
    const DAYS: [&str; 12] = [
        tr_mark!("{day} January {year}"),
        tr_mark!("{day} February {year}"),
        tr_mark!("{day} March {year}"),
        tr_mark!("{day} April {year}"),
        tr_mark!("{day} May {year}"),
        tr_mark!("{day} June {year}"),
        tr_mark!("{day} July {year}"),
        tr_mark!("{day} August {year}"),
        tr_mark!("{day} September {year}"),
        tr_mark!("{day} October {year}"),
        tr_mark!("{day} November {year}"),
        tr_mark!("{day} December {year}"),
    ];
    let Some(day) = date.get(8..10).and_then(|d| d.parse::<u32>().ok()) else { return date.to_owned() };
    let year = date.get(..4).unwrap_or_default();
    let label = fill(tr(DAYS[month_index(date)]), &[("day", &day), ("year", &year)]);
    match date.get(11..16).filter(|t| *t != "00:00") {
        Some(time) => format!("{label}, {time}"),
        None => label,
    }
}

/// The month of a `YYYY:MM…` date, from 0.
fn month_index(date: &str) -> usize {
    date.get(5..7).and_then(|m| m.parse::<usize>().ok()).unwrap_or(1).clamp(1, 12) - 1
}

/// Which plural form a count takes, as in the languages' `Plural-Forms`.
fn plural_form(language: &str, n: usize) -> usize {
    let (ten, hundred) = (n % 10, n % 100);
    match language {
        // One, few, many: 1 фото, 2 фото, 5 фото; 21, 22, 25 alike.
        "ru" | "uk" | "be" | "sr" | "hr" | "bs" => {
            if ten == 1 && hundred != 11 {
                0
            } else if (2..=4).contains(&ten) && !(12..=14).contains(&hundred) {
                1
            } else {
                2
            }
        }
        "pl" => {
            if n == 1 {
                0
            } else if (2..=4).contains(&ten) && !(12..=14).contains(&hundred) {
                1
            } else {
                2
            }
        }
        "cs" | "sk" => match n {
            1 => 0,
            2..=4 => 1,
            _ => 2,
        },
        "fr" | "pt_BR" => usize::from(n > 1),
        "ja" | "zh" | "ko" | "vi" | "th" | "id" => 0,
        _ => usize::from(n != 1),
    }
}

/// The translations in a `.po` file; untranslated and fuzzy ones are left out.
fn parse_po(text: &str) -> HashMap<String, Vec<String>> {
    let mut texts = HashMap::new();
    let mut id = String::new();
    let mut forms: Vec<String> = Vec::new();
    let mut fuzzy = false;
    // A `#, fuzzy` flag comes before the msgid of the text it is about.
    let mut fuzzy_next = false;
    // Which string continuation lines add to: the msgid, a msgstr, or none.
    enum Field {
        None,
        Id,
        Plural,
        Str(usize),
    }
    let mut field = Field::None;
    let mut done = |id: &mut String, forms: &mut Vec<String>, fuzzy: &mut bool| {
        if !id.is_empty() && !*fuzzy && forms.iter().any(|f| !f.is_empty()) {
            texts.insert(std::mem::take(id), std::mem::take(forms));
        }
        id.clear();
        forms.clear();
        *fuzzy = false;
    };
    for line in text.lines().map(str::trim) {
        if line.starts_with("#,") && line.contains("fuzzy") {
            fuzzy_next = true;
        } else if line.starts_with("msgid_plural ") {
            field = Field::Plural;
        } else if let Some(rest) = line.strip_prefix("msgid ") {
            done(&mut id, &mut forms, &mut fuzzy);
            fuzzy = std::mem::take(&mut fuzzy_next);
            id = unquote(rest);
            field = Field::Id;
        } else if let Some(rest) = line.strip_prefix("msgstr") {
            let (index, value) = match rest.strip_prefix('[') {
                Some(indexed) => {
                    let (n, value) = indexed.split_once(']').unwrap_or(("0", ""));
                    (n.parse().unwrap_or(0), value)
                }
                None => (0, rest),
            };
            if forms.len() <= index {
                forms.resize(index + 1, String::new());
            }
            forms[index] = unquote(value.trim());
            field = Field::Str(index);
        } else if line.starts_with('"') {
            match field {
                Field::Id => id.push_str(&unquote(line)),
                Field::Str(i) => forms[i].push_str(&unquote(line)),
                Field::Plural | Field::None => {}
            }
        } else if line.is_empty() {
            field = Field::None;
        }
    }
    done(&mut id, &mut forms, &mut fuzzy);
    texts
}

/// The text of a quoted `.po` string.
fn unquote(quoted: &str) -> String {
    let inner = quoted.trim().strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or_default();
    let mut text = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            text.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => text.push('\n'),
            Some('t') => text.push('\t'),
            Some(other) => text.push(other),
            None => {}
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    const PO: &str = r#"
msgid ""
msgstr ""
"Plural-Forms: nplurals=3; plural=(n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2);\n"

#: src/app.rs
msgid "Add Folder to Pico…"
msgstr "Добавить папку в Pico…"

msgid "{n} photo"
msgid_plural "{n} photos"
msgstr[0] "{n} фотография"
msgstr[1] "{n} фотографии"
msgstr[2] "{n} фотографий"

#, fuzzy
msgid "Hide"
msgstr "Скрыть"

msgid "Two "
"lines"
msgstr "Две\n"
"строки"

msgid "Untranslated"
msgstr ""
"#;

    #[test]
    fn reads_po_files() {
        let texts = parse_po(PO);
        assert_eq!(texts["Add Folder to Pico…"], ["Добавить папку в Pico…"]);
        assert_eq!(texts["{n} photo"], ["{n} фотография", "{n} фотографии", "{n} фотографий"]);
        assert_eq!(texts["Two lines"], ["Две\nстроки"]);
        assert!(!texts.contains_key("Hide"), "fuzzy");
        assert!(!texts.contains_key("Untranslated"));
        assert!(!texts.contains_key(""), "the header");
    }

    #[test]
    fn russian_plurals() {
        let forms: Vec<usize> = [1, 2, 5, 11, 12, 21, 22, 25, 111, 0].iter().map(|&n| plural_form("ru", n)).collect();
        assert_eq!(forms, [0, 1, 2, 2, 2, 0, 1, 2, 2, 2]);
        assert_eq!(plural_form("en", 1), 0);
        assert_eq!(plural_form("en", 0), 1);
    }

    #[test]
    fn fills_in_values() {
        assert_eq!(fill("{n} of {total}", &[("n", &3), ("total", &"10")]), "3 of 10");
    }

    #[test]
    fn russian_is_complete() {
        // Every text in the template has its Russian translation.
        let template =
            include_str!("../po/pico.pot").replace("msgstr \"\"", "msgstr \"x\"").replace("] \"\"", "] \"x\"");
        let template = parse_po(&template);
        let russian = parse_po(LANGUAGES[1].2);
        let missing: Vec<&String> = template.keys().filter(|id| !russian.contains_key(*id)).collect();
        assert!(missing.is_empty(), "not translated: {missing:?}");
    }
}
