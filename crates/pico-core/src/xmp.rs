//! The XMP packet many photos carry next to EXIF: where editors keep the
//! description, the creator, the rights, the rating. Read for the Properties
//! panel, as `(label, value)` pairs.

/// The properties of the XMP packet near the start of the file, in order;
/// empty when there is none.
pub fn read(path: &std::path::Path) -> Vec<(String, String)> {
    use std::io::Read;
    let mut head = Vec::new();
    // Editors put XMP before the image data; 512 KiB covers it.
    let Ok(file) = std::fs::File::open(path) else { return Vec::new() };
    if file.take(512 * 1024).read_to_end(&mut head).is_err() {
        return Vec::new();
    }
    if let Some(xml) = packet(&head) {
        return properties(xml);
    }
    // PNG may hold it compressed, or as ImageMagick's hex "Raw profile".
    let text = crate::exif::png_chunks(&head)
        .filter(|(kind, _)| matches!(kind, b"tEXt" | b"zTXt" | b"iTXt"))
        .filter_map(|(kind, data)| crate::metadata::png_text(&kind, data))
        .find_map(|(key, text)| match key.as_str() {
            "XML:com.adobe.xmp" => Some(text.into_bytes()),
            "Raw profile type xmp" => crate::metadata::raw_profile(&text),
            _ => None,
        });
    text.as_deref().and_then(packet).map(properties).unwrap_or_default()
}

/// The position XMP gives (`exif:GPSLatitude` "59,23.6102N" or
/// "59,23,36.61N"), in degrees, south and west negative.
pub fn gps(properties: &[(String, String)]) -> Option<(f64, f64)> {
    let value = |label: &str| properties.iter().find(|(l, _)| l == label).map(|(_, v)| v.as_str());
    let coordinate = |text: &str| {
        let text = text.trim();
        let sign = if text.ends_with(['S', 'W']) { -1.0 } else { 1.0 };
        let parts: Vec<f64> = text
            .trim_end_matches(['N', 'S', 'E', 'W'])
            .split(',')
            .map(|p| p.trim().parse().ok())
            .collect::<Option<_>>()?;
        let degrees = parts.iter().zip([1.0, 60.0, 3600.0]).map(|(v, per)| v / per).sum::<f64>();
        Some(sign * degrees)
    };
    let (lat, lon) = (coordinate(value("GPS Latitude")?)?, coordinate(value("GPS Longitude")?)?);
    (lat.abs() <= 90.0 && lon.abs() <= 180.0).then_some((lat, lon))
}

fn packet(bytes: &[u8]) -> Option<&str> {
    let start = find(bytes, b"<x:xmpmeta").or_else(|| find(bytes, b"<rdf:RDF"))?;
    let rest = &bytes[start..];
    let end = find(rest, b"</x:xmpmeta>").map(|e| e + 12).or_else(|| find(rest, b"</rdf:RDF>").map(|e| e + 10))?;
    std::str::from_utf8(&rest[..end]).ok()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Every property: plain values, lists (`rdf:li`, joined) and attributes.
fn properties(xml: &str) -> Vec<(String, String)> {
    // By full name, so properties of different namespaces stay apart.
    let mut found: Vec<(String, String, String)> = Vec::new();
    let mut add = |name: &str, value: &str| {
        let mut value = unescape(value.trim());
        if value.is_empty() {
            return;
        }
        // EXIF's own properties, in EXIF's words: 1 → "Manual".
        if let Some((prefix, local)) = name.split_once(':')
            && matches!(prefix, "exif" | "exifEX" | "tiff")
            && let Some(shown) = crate::exif::readable(local, &value)
        {
            value = shown;
        }
        // EXIF's own properties keep EXIF's names: "GPS Latitude".
        let exif_name = name
            .split_once(':')
            .filter(|(prefix, _)| matches!(*prefix, "exif" | "exifEX" | "tiff"))
            .and_then(|(_, local)| crate::exif_tags::tag_named(local))
            .and_then(crate::exif_tags::standard_name);
        let label = exif_name.map_or_else(|| label(name), str::to_owned);
        match found.iter_mut().find(|(n, _, _)| n == name) {
            Some((_, _, v)) => {
                v.push_str(", ");
                v.push_str(&value);
            }
            None => found.push((name.to_owned(), label, value)),
        }
    };
    let mut stack: Vec<&str> = Vec::new();
    let mut rest = xml;
    while let Some(open) = rest.find('<') {
        let text = &rest[..open];
        // Text belongs to the innermost property; a list item to its list's.
        if !text.trim().is_empty()
            && let Some(name) = stack.iter().rev().copied().find(|n| !n.starts_with("rdf:"))
        {
            add(name, text);
        }
        let Some(close) = rest[open..].find('>') else { break };
        let tag = &rest[open + 1..open + close];
        rest = &rest[open + close + 1..];
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            if let Some(at) = stack.iter().rposition(|n| *n == name.trim()) {
                stack.truncate(at);
            }
            continue;
        }
        let closed = tag.ends_with('/');
        let tag = tag.trim_end_matches('/');
        let (name, attributes) = tag.split_once(char::is_whitespace).unwrap_or((tag, ""));
        for (key, value) in attributes_of(attributes) {
            if !key.starts_with("xmlns") && !key.starts_with("rdf:") && !key.starts_with("xml:") {
                add(key, value);
            }
        }
        if !closed {
            stack.push(name);
        }
    }
    // The wrapper's attributes are not properties.
    found.retain(|(name, _, _)| !name.starts_with("x:"));
    // Different properties with the same words say whose they are:
    // "Description (dc)", "Description (exif)".
    let labels: Vec<String> = found.iter().map(|(_, label, _)| label.clone()).collect();
    found
        .into_iter()
        .map(|(name, label, value)| {
            if labels.iter().filter(|l| **l == label).count() > 1 {
                let prefix = name.split_once(':').map_or("", |(p, _)| p);
                (format!("{label} ({prefix})"), value)
            } else {
                (label, value)
            }
        })
        .collect()
}

fn attributes_of(text: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(eq) = rest.find('=') {
        let key = rest[..eq].trim();
        let after = rest[eq + 1..].trim_start();
        let Some(quote) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else { break };
        let Some(end) = after[1..].find(quote) else { break };
        out.push((key, &after[1..1 + end]));
        rest = &after[end + 2..];
    }
    out
}

/// `dc:creator` → "Creator", `xmp:CreatorTool` → "Creator Tool".
fn label(name: &str) -> String {
    let local = name.rsplit(':').next().unwrap_or(name);
    // IPTC's contact details have terse names.
    let known = match local {
        "CiEmailWork" => "Email",
        "CiTelWork" => "Phone",
        "CiUrlWork" => "Website",
        "CiAdrExtadr" => "Address",
        "CiAdrCity" => "City",
        "CiAdrRegion" => "Region",
        "CiAdrPcode" => "Postal Code",
        "CiAdrCtry" => "Country",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_owned();
    }
    // A capital starts a word, unless it continues an abbreviation; the last
    // capital of an abbreviation starts the next word ("GPSLatitude").
    let chars: Vec<char> = local.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if i == 0 {
            out.extend(c.to_uppercase());
            continue;
        }
        let after_lower = chars[i - 1].is_lowercase() || chars[i - 1].is_ascii_digit();
        let ends_abbreviation = chars[i - 1].is_uppercase() && chars.get(i + 1).is_some_and(|n| n.is_lowercase());
        if c.is_uppercase() && (after_lower || ends_abbreviation) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_properties_lists_and_attributes() {
        let xml = r#"<?xpacket begin=""?><x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="XMP Core"><rdf:RDF xmlns:rdf="r">
            <rdf:Description rdf:about="" xmlns:xmp="x" xmp:Rating="4"><xmp:CreatorTool>Pico &amp; Co</xmp:CreatorTool></rdf:Description>
            <rdf:Description rdf:about="" xmlns:dc="d"><dc:creator><rdf:Seq><rdf:li>Ann</rdf:li><rdf:li>Bob</rdf:li></rdf:Seq></dc:creator>
            <dc:description><rdf:Alt><rdf:li xml:lang="x-default">Sunset over the bay</rdf:li></rdf:Alt></dc:description></rdf:Description>
            </rdf:RDF></x:xmpmeta>"#;
        let found = properties(packet(xml.as_bytes()).unwrap());
        let get = |l: &str| found.iter().find(|(k, _)| k == l).map(|(_, v)| v.as_str());
        assert_eq!(get("Rating"), Some("4"));
        assert_eq!(get("Creator Tool"), Some("Pico & Co"));
        assert_eq!(get("Creator"), Some("Ann, Bob"));
        assert_eq!(get("Description"), Some("Sunset over the bay"));
        assert_eq!(found.len(), 4, "{found:?}");
    }

    #[test]
    fn properties_of_namespaces_stay_apart() {
        let xml = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="r"><rdf:Description
            xmlns:dc="d" xmlns:photoshop="p" photoshop:Headline="Big day"><dc:title><rdf:Alt><rdf:li>Day</rdf:li></rdf:Alt>
            </dc:title><photoshop:Title>Other</photoshop:Title></rdf:Description></rdf:RDF></x:xmpmeta>"#;
        let found = properties(packet(xml.as_bytes()).unwrap());
        let labels: Vec<&str> = found.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["Headline", "Title (dc)", "Title (photoshop)"]);
    }

    #[test]
    fn labels_split_words_and_abbreviations() {
        assert_eq!(label("xmp:CreatorTool"), "Creator Tool");
        assert_eq!(label("photoshop:ICCProfile"), "ICC Profile");
        assert_eq!(label("dc:creator"), "Creator");
        assert_eq!(label("xmpMM:DocumentID"), "Document ID");
    }

    #[test]
    fn reads_xmp_positions() {
        let props = |lat: &str, lon: &str| {
            vec![("GPS Latitude".to_owned(), lat.to_owned()), ("GPS Longitude".to_owned(), lon.to_owned())]
        };
        let (lat, lon) = gps(&props("59,23.61N", "24,39.97E")).unwrap();
        assert!((lat - 59.3935).abs() < 1e-4 && (lon - 24.66617).abs() < 1e-4, "{lat} {lon}");
        let (lat, lon) = gps(&props("33,52,12S", "151,12,36E")).unwrap();
        assert!((lat + 33.87).abs() < 1e-4 && (lon - 151.21).abs() < 1e-4, "{lat} {lon}");
    }

    #[test]
    fn exif_properties_read_like_exif() {
        let xml = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="r"><rdf:Description xmlns:exif="e"
            exif:ExposureProgram="1" exif:ExposureTime="1/125"><exif:MeteringMode>5</exif:MeteringMode>
            </rdf:Description></rdf:RDF></x:xmpmeta>"#;
        let found = properties(packet(xml.as_bytes()).unwrap());
        let get = |l: &str| found.iter().find(|(k, _)| k == l).map(|(_, v)| v.as_str());
        assert_eq!(get("Exposure Program"), Some("Manual"));
        assert_eq!(get("Exposure Time"), Some("1/125 s"));
        assert_eq!(get("Metering Mode"), Some("Pattern"));
    }
}
