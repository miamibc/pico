//! Fields that say the same thing in different kinds of metadata (EXIF, XMP,
//! IPTC, PNG text, the camera maker's notes) and should agree: the title,
//! description, creator, copyright, keywords, dates, software, rating, lens,
//! camera settings and serial numbers, and every XMP property that copies an
//! EXIF tag. Where the ones present differ, each of them is reported, with
//! what all of them say. Edited or forged files often give themselves away
//! here.

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
    (
        Compare::Text,
        &[
            ("EXIF", "Lens Model"),
            ("XMP", "Lens Model"),
            ("XMP", "Lens"),
            ("EXIF", "Canon Lens Model"),
            ("EXIF", "OLYMPUS Lens Model"),
        ],
    ),
    // The camera's own notes against the EXIF tags beside them.
    (
        Compare::Stops(0.34),
        &[
            ("EXIF", "Exposure Time"),
            ("EXIF", "Shutter Speed Value"),
            ("XMP", "Exposure Time"),
            ("EXIF", "Canon Exposure Time"),
        ],
    ),
    (Compare::Stops(0.34 / 2.0), &[("EXIF", "F Number"), ("EXIF", "Aperture Value"), ("EXIF", "Canon F-Number")]),
    (Compare::Near(0.15), &[("EXIF", "ISO"), ("EXIF", "Canon ISO"), ("EXIF", "NIKON ISO")]),
    (Compare::Near(0.03), &[("EXIF", "Focal Length"), ("EXIF", "Canon Focal Length")]),
    (Compare::Plus(0.2), &[("EXIF", "Exposure Compensation"), ("EXIF", "Canon Exposure Compensation")]),
    // Adobe copies what the maker's notes say into XMP's `aux:` properties.
    (
        Compare::Serial,
        &[
            ("EXIF", "Body Serial Number"),
            ("EXIF", "Canon Serial Number"),
            ("EXIF", "NIKON Serial Number"),
            ("EXIF", "OLYMPUS Serial Number"),
            ("XMP", "Serial Number"),
        ],
    ),
    (
        Compare::Serial,
        &[("EXIF", "Lens Serial Number"), ("EXIF", "OLYMPUS Lens Serial Number"), ("XMP", "Lens Serial Number")],
    ),
    (Compare::Text, &[("EXIF", "Camera Owner"), ("EXIF", "Canon Owner Name"), ("XMP", "Owner Name")]),
    (
        Compare::Version,
        &[("EXIF", "Canon Firmware Version"), ("EXIF", "OLYMPUS Body Firmware Version"), ("XMP", "Firmware")],
    ),
    (Compare::Model, &[("EXIF", "Model"), ("EXIF", "Canon Image Type")]),
    // The camera was switched on before the photo was taken.
    (Compare::Before, &[("EXIF", "NIKON Power Up Time"), ("EXIF", "Date Taken")]),
];

/// XMP's copies of EXIF tags carry EXIF's labels; these are compared as
/// positions elsewhere, being written differently.
const NOT_ALIKE: &[&str] = &["GPS Latitude", "GPS Longitude", "GPS Latitude Ref", "GPS Longitude Ref", "Padding"];

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
    /// Times or f-numbers within so many stops: "1/125 s", "f/8.0".
    Stops(f64),
    /// Numbers within a share of each other.
    Near(f64),
    /// Numbers within so much of each other: EV.
    Plus(f64),
    /// The same letters and digits, case and leading zeros aside: serial
    /// numbers ("0000c1c975" and "C1C975").
    Serial,
    /// The same version number: "Firmware Version 1.0.3" and "1.0.3".
    Version,
    /// One camera name inside the other, the maker's name aside:
    /// "Canon PowerShot A400" in "IMG:PowerShot A400 JPEG".
    Model,
    /// Dates in this order, the first no later than the next.
    Before,
    /// An EXIF tag and XMP's copy of it, each written its own way: "2.20"
    /// and "220", "Inches" and "Inch", "Uncalibrated" and 65535.
    Copy,
}

impl Compare {
    /// What fields of the group have to do with each other, for the note.
    fn rule(self) -> &'static str {
        match self {
            Compare::Before => "Should come in this order",
            _ => "Should match",
        }
    }

    /// Whether a value can be compared this way at all ("Auto" is no ISO).
    fn takes(self, value: &str) -> bool {
        match self {
            Compare::Stops(_) | Compare::Near(_) | Compare::Plus(_) => number(value).is_some(),
            Compare::Serial | Compare::Version => value.chars().any(|c| c.is_ascii_digit()),
            // Some cameras put their firmware where the model would be.
            Compare::Model => !value.to_lowercase().starts_with("firmware"),
            _ => true,
        }
    }
}

/// The number a value starts with: "1/125 s", "f/8.0", "+0.3 EV", "90.0 mm".
fn number(value: &str) -> Option<f64> {
    let value = value.trim().trim_start_matches("f/");
    let token: String =
        value.chars().take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '/' | '-' | '+')).collect();
    match token.split_once('/') {
        Some((n, d)) => Some(n.parse::<f64>().ok()? / d.parse::<f64>().ok().filter(|d| *d != 0.0)?),
        None => token.parse().ok(),
    }
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
            let compare = if label.starts_with("Date") { Compare::Date } else { Compare::Copy };
            groups.push((compare, vec![("EXIF", label.as_str()), ("XMP", label.as_str())]));
        }
    }
    let mut out = Vec::new();
    for (compare, group) in groups {
        let present: Vec<(&str, &str, &str)> = group
            .iter()
            .filter_map(|&(section, label)| value(section, label).map(|(_, v)| (section, label, v.as_str())))
            .filter(|&(_, _, v)| compare.takes(v))
            .collect();
        let Some(&(_, _, first)) = present.first() else { continue };
        let agree = match compare {
            Compare::Before => present.windows(2).all(|w| alike(compare, w[0].2, w[1].2)),
            _ => present.iter().all(|&(_, _, v)| alike(compare, first, v)),
        };
        if agree {
            continue;
        }
        let each: Vec<String> = present.iter().map(|(s, l, v)| format!("{s} {l}: {v}")).collect();
        let note = format!("{}:\n{}", compare.rule(), each.join("\n"));
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
        Compare::Stops(most) => match (number(a), number(b)) {
            (Some(a), Some(b)) if a > 0.0 && b > 0.0 => (a / b).log2().abs() <= most + 1e-9,
            _ => true,
        },
        Compare::Near(share) => match (number(a), number(b)) {
            (Some(a), Some(b)) => (a - b).abs() <= share * a.abs().max(b.abs()) + 1e-9,
            _ => true,
        },
        Compare::Plus(most) => match (number(a), number(b)) {
            (Some(a), Some(b)) => (a - b).abs() <= most + 1e-9,
            _ => true,
        },
        Compare::Serial => {
            let serial = |s: &str| s.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_lowercase();
            serial(a).trim_start_matches('0') == serial(b).trim_start_matches('0')
        }
        Compare::Version => {
            // The first run of digits and dots.
            let version = |s: &str| {
                let start = s.find(|c: char| c.is_ascii_digit()).unwrap_or(s.len());
                s[start..].chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect::<String>()
            };
            version(a).trim_end_matches('.') == version(b).trim_end_matches('.')
        }
        Compare::Model => {
            let name = |s: &str| {
                let s = s.to_lowercase();
                let s = s.trim_start_matches("img:").trim_end_matches(" jpeg").to_owned();
                s.split_whitespace()
                    .filter(|w| !matches!(*w, "canon" | "nikon" | "corporation"))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            let (a, b) = (name(a), name(b));
            a.contains(&b) || b.contains(&a)
        }
        Compare::Before => {
            let digits = |s: &str| s.chars().filter(char::is_ascii_digit).take(14).collect::<String>();
            let (a, b) = (digits(a), digits(b));
            let n = a.len().min(b.len());
            a[..n] <= b[..n]
        }
        Compare::Copy => {
            let loose = |s: &str| {
                let s = s.trim().to_lowercase();
                // Uncalibrated colour, as a word or as 0xFFFF, 0xFFFFFFFF.
                if s.contains("uncalibrated") || s.contains("65535") || s == "4294967295" {
                    return "uncalibrated".to_owned();
                }
                // Version numbers with or without dots: 2.20, 220, "Exif Version 2.2".
                if s.contains("version") || s.chars().all(|c| c.is_ascii_digit() || c == '.') {
                    let digits: String = s.chars().filter(char::is_ascii_digit).collect();
                    if s.contains("version") || s.contains('.') || digits.len() == 3 || digits.len() == 4 {
                        return digits.trim_end_matches('0').to_owned();
                    }
                }
                s
            };
            // Word by word, one in the plural or not: "Inches" and "Inch".
            let plural = |x: &str, y: &str| x == y || x == format!("{y}s") || x == format!("{y}es");
            let (a, b) = (loose(a), loose(b));
            let (a, b): (Vec<&str>, Vec<&str>) = (a.split_whitespace().collect(), b.split_whitespace().collect());
            a.len() == b.len() && a.iter().zip(&b).all(|(x, y)| plural(x, y) || plural(y, x))
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

    #[test]
    fn xmp_copies_of_exif_may_be_written_their_own_way() {
        let exif = rows(&[
            ("Exif Version", "2.20"),
            ("Color Space", "Uncalibrated"),
            ("Resolution Unit", "Inches"),
            ("Software", "GIMP 2.10.22"),
        ]);
        let xmp = rows(&[
            ("Exif Version", "220"),
            ("Color Space", "4294967295"),
            ("Resolution Unit", "Inch"),
            ("Creator Tool", "GIMP 2.10"),
        ]);
        let found = disagreements(&[("EXIF", &exif), ("XMP", &xmp)]);
        let flagged: Vec<(&str, &str)> = found.iter().map(|(s, l, _)| (s.as_str(), l.as_str())).collect();
        assert_eq!(flagged, [("EXIF", "Software"), ("XMP", "Creator Tool")]);
    }

    #[test]
    fn checks_the_makers_notes_against_exif() {
        let exif = rows(&[
            // Within a third of a stop, and equal: fine.
            ("Exposure Time", "1/320 s"),
            ("Canon Exposure Time", "1/354 s"),
            ("F Number", "f/3.8"),
            ("Canon F-Number", "f/3.9"),
            ("ISO", "100"),
            ("Canon ISO", "100"),
            ("NIKON ISO", "Auto"),
            ("Model", "Canon PowerShot A400"),
            ("Canon Image Type", "IMG:PowerShot A400 JPEG"),
            // Another lens, another owner, a camera switched on after the shot.
            ("Focal Length", "5.9 mm"),
            ("Canon Focal Length", "25.2 mm"),
            ("Camera Owner", "Ann"),
            ("Canon Owner Name", "Bob"),
            ("Date Taken", "2013:10:17 11:17:37"),
            ("NIKON Power Up Time", "2013:10:17 11:27:04"),
            ("Body Serial Number", "0025021"),
            ("Canon Serial Number", "25021"),
            ("Canon Firmware Version", "Firmware Version 1.0.3"),
            ("Lens Serial Number", "0000c1c975"),
        ]);
        // Adobe's copies of the notes in XMP: the same firmware and lens, but
        // another body.
        let xmp = rows(&[("Firmware", "1.0.3"), ("Lens Serial Number", "C1C975"), ("Serial Number", "25022")]);
        let found = disagreements(&[("EXIF", &exif), ("XMP", &xmp)]);
        let flagged: Vec<&str> = found.iter().map(|(_, l, _)| l.as_str()).collect();
        assert_eq!(
            flagged,
            [
                "Focal Length",
                "Canon Focal Length",
                "Body Serial Number",
                "Canon Serial Number",
                "Serial Number",
                "Camera Owner",
                "Canon Owner Name",
                "NIKON Power Up Time",
                "Date Taken"
            ]
        );
        assert!(found[7].2.starts_with("Should come in this order"), "{}", found[7].2);
    }
}
