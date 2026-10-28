//! Fields that say the same thing in different kinds of metadata (EXIF, XMP,
//! IPTC, PNG text) and should agree: the title, description, creator,
//! copyright, keywords, dates, software, rating, lens, and every XMP
//! property that copies an EXIF tag. Where the ones present differ, each of
//! them is reported, with what all of them say.

/// Groups of `(section, label)` that hold the same fact, and how to compare.
const GROUPS: &[(Compare, &[(&str, &str)])] = &[
    (Compare::Text, &[("EXIF", "Title"), ("EXIF", "Image Title"), ("XMP", "Title"), ("IPTC", "Title")]),
    (
        Compare::Text,
        &[("EXIF", "Description"), ("XMP", "Description"), ("IPTC", "Caption/Abstract"), ("PNG", "Description")],
    ),
    (
        Compare::Text,
        &[
            ("EXIF", "Artist"),
            ("EXIF", "Author"),
            ("EXIF", "Photographer"),
            ("XMP", "Creator"),
            ("IPTC", "By-line"),
            ("PNG", "Author"),
        ],
    ),
    (Compare::Text, &[("EXIF", "Copyright"), ("XMP", "Rights"), ("IPTC", "Copyright Notice"), ("PNG", "Copyright")]),
    (Compare::Words, &[("EXIF", "Keywords"), ("XMP", "Subject"), ("IPTC", "Keywords")]),
    (
        Compare::Date,
        &[("EXIF", "Date Taken"), ("XMP", "Date Taken"), ("XMP", "Date Created"), ("IPTC", "Date Created")],
    ),
    (Compare::Date, &[("EXIF", "Date Digitized"), ("XMP", "Create Date"), ("XMP", "Date Digitized")]),
    (Compare::Date, &[("EXIF", "Date Modified"), ("XMP", "Modify Date"), ("PNG", "Modified")]),
    (
        Compare::Text,
        &[("EXIF", "Software"), ("XMP", "Creator Tool"), ("PNG", "Software"), ("IPTC", "Originating Program")],
    ),
    (Compare::Number, &[("EXIF", "Rating"), ("XMP", "Rating")]),
    (Compare::Text, &[("EXIF", "Lens Model"), ("XMP", "Lens Model"), ("XMP", "Lens")]),
];

/// XMP's copies of EXIF tags carry EXIF's labels; these are compared as
/// positions elsewhere, being written differently.
const NOT_ALIKE: &[&str] = &["GPS Latitude", "GPS Longitude", "GPS Latitude Ref", "GPS Longitude Ref"];

#[derive(Clone, Copy)]
enum Compare {
    /// Same words, spacing and case aside.
    Text,
    /// The same set of words, in any order: keywords.
    Words,
    /// The same moment to the second, as far as both tell.
    Date,
    /// The same number: a rating, written as stars or not.
    Number,
}

/// Every `(section, label)` that disagrees with another field holding the
/// same fact, and a note listing what each of them says.
pub fn disagreements(sections: &[(&str, &[(String, String)])]) -> Vec<(String, String, String)> {
    let value = |section: &str, label: &str| {
        sections.iter().filter(|(s, _)| *s == section).flat_map(|(_, rows)| rows.iter()).find(|(l, _)| l == label)
    };
    let mut groups: Vec<(Compare, Vec<(&str, &str)>)> = GROUPS.iter().map(|(c, g)| (*c, g.to_vec())).collect();
    // Every XMP property with an EXIF tag's label copies that tag.
    let exif_rows = sections.iter().filter(|(s, _)| *s == "EXIF").flat_map(|(_, rows)| rows.iter());
    for (label, _) in exif_rows {
        let grouped = groups.iter().any(|(_, g)| g.contains(&("EXIF", label.as_str())));
        if !grouped && !NOT_ALIKE.contains(&label.as_str()) && value("XMP", label).is_some() {
            let compare = if label.starts_with("Date") { Compare::Date } else { Compare::Text };
            groups.push((compare, vec![("EXIF", label.as_str()), ("XMP", label.as_str())]));
        }
    }
    let mut out = Vec::new();
    for (compare, group) in groups {
        let present: Vec<(&str, &str, &str)> = group
            .iter()
            .filter_map(|&(section, label)| value(section, label).map(|(_, v)| (section, label, v.as_str())))
            .collect();
        let Some(&(_, _, first)) = present.first() else { continue };
        if present.iter().all(|&(_, _, v)| alike(compare, first, v)) {
            continue;
        }
        let each: Vec<String> = present.iter().map(|(s, l, v)| format!("{s} {l}: {v}")).collect();
        let note = format!("Should match:\n{}", each.join("\n"));
        out.extend(present.iter().map(|(s, l, _)| (s.to_string(), l.to_string(), note.clone())));
    }
    out
}

fn alike(compare: Compare, a: &str, b: &str) -> bool {
    match compare {
        Compare::Text => {
            let words = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
            words(a) == words(b)
        }
        Compare::Words => {
            let set = |s: &str| {
                let mut words: Vec<String> =
                    s.split([',', ';']).map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty()).collect();
                words.sort();
                words.dedup();
                words
            };
            set(a) == set(b)
        }
        Compare::Date => {
            let digits = |s: &str| s.chars().filter(char::is_ascii_digit).take(14).collect::<String>();
            let (a, b) = (digits(a), digits(b));
            let n = a.len().min(b.len());
            a[..n] == b[..n]
        }
        Compare::Number => {
            // "★★★☆☆ (3)" or "3".
            let number = |s: &str| {
                let s = s.rsplit('(').next().unwrap_or(s);
                s.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect::<String>().parse::<f64>().ok()
            };
            number(a) == number(b)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(l, v)| (l.to_string(), v.to_string())).collect()
    }

    #[test]
    fn finds_fields_that_disagree_across_kinds() {
        let exif = rows(&[
            ("Artist", "Ann"),
            ("Copyright", "Ann"),
            ("Date Taken", "2018:02:09 19:17:10"),
            ("Exposure Time", "1/125 s"),
            ("Rating", "★★★☆☆ (3)"),
        ]);
        let xmp = rows(&[
            ("Creator", "Bob"),
            ("Rights", " ann "),
            ("Date Taken", "2018-02-09T19:17:10.57+02:00"),
            ("Exposure Time", "1/60 s"),
            ("Rating", "3"),
            ("Subject", "sea, Sun"),
        ]);
        let iptc = rows(&[("By-line", "Ann"), ("Keywords", "sun, sea")]);
        let found = disagreements(&[("EXIF", &exif), ("XMP", &xmp), ("IPTC", &iptc)]);
        let flagged: Vec<(&str, &str)> = found.iter().map(|(s, l, _)| (s.as_str(), l.as_str())).collect();
        assert_eq!(
            flagged,
            [
                ("EXIF", "Artist"),
                ("XMP", "Creator"),
                ("IPTC", "By-line"),
                ("EXIF", "Exposure Time"),
                ("XMP", "Exposure Time")
            ]
        );
        assert!(found[0].2.contains("XMP Creator: Bob"), "{}", found[0].2);
    }
}
