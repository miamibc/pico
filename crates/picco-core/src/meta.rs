//! Typed view of the per-photo section of `.picasa.ini`.

use crate::filters::FilterStack;
use crate::ini::Section;
use crate::rect64::Rect64;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PhotoMeta {
    pub star: bool,
    pub caption: Option<String>,
    pub keywords: Vec<String>,
    /// Quarter turns clockwise, from `rotate=rotate(n)`.
    pub rotate: u8,
    pub filters: FilterStack,
    pub faces: Vec<Face>,
    /// Tokens of Picasa albums this photo belongs to (`[.album:<token>]`).
    pub albums: Vec<String>,
    /// Hidden from the library unless hidden photos are shown (`hidden=yes`).
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Face {
    pub rect: Rect64,
    /// Key into the `[Contacts2]` section.
    pub contact_id: String,
}

impl PhotoMeta {
    pub fn from_section(section: &Section) -> Self {
        let list = |key: &str| -> Vec<String> {
            section
                .get(key)
                .map(|v| v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect())
                .unwrap_or_default()
        };
        Self {
            star: section.get("star") == Some("yes"),
            caption: section.get("caption").filter(|c| !c.is_empty()).map(str::to_owned),
            keywords: list("keywords"),
            rotate: section.get("rotate").and_then(parse_rotate).unwrap_or(0),
            filters: section.get("filters").map(FilterStack::parse).unwrap_or_default(),
            faces: section.get("faces").map(parse_faces).unwrap_or_default(),
            albums: list("albums"),
            hidden: section.get("hidden") == Some("yes"),
        }
    }

    /// Where a trimmed video starts (`moviestart` in the edit stack).
    pub fn video_start(&self) -> Option<std::time::Duration> {
        self.filters.0.iter().rev().find(|f| f.id == "moviestart").and_then(|f| f.time())
    }

    /// Writes back the known fields that differ from what the section says,
    /// leaving every other key, and the way it was written, alone.
    pub fn write_to(&self, section: &mut Section) {
        let old = Self::from_section(section);
        let mut put = |changed: bool, key: &str, value: Option<String>| match value {
            _ if !changed => {}
            Some(v) => section.set(key, v),
            None => {
                section.remove(key);
            }
        };
        put(self.star != old.star, "star", self.star.then(|| "yes".to_owned()));
        put(self.caption != old.caption, "caption", self.caption.clone());
        put(self.keywords != old.keywords, "keywords", (!self.keywords.is_empty()).then(|| self.keywords.join(",")));
        let rotate = (!self.rotate.is_multiple_of(4)).then(|| format!("rotate({})", self.rotate % 4));
        put(self.rotate != old.rotate, "rotate", rotate);
        let filters_changed = self.filters != old.filters;
        put(filters_changed, "filters", (!self.filters.is_empty()).then(|| self.filters.to_string()));
        // Picasa mirrors the crop in effect into its own key for older readers.
        put(filters_changed, "crop", self.filters.crop().map(|rect| format!("rect64({rect})")));
        let faces = (!self.faces.is_empty()).then(|| {
            self.faces.iter().map(|f| format!("rect64({}),{}", f.rect, f.contact_id)).collect::<Vec<_>>().join(";")
        });
        put(self.faces != old.faces, "faces", faces);
        put(self.albums != old.albums, "albums", (!self.albums.is_empty()).then(|| self.albums.join(",")));
        put(self.hidden != old.hidden, "hidden", self.hidden.then(|| "yes".to_owned()));
    }
}

fn parse_rotate(value: &str) -> Option<u8> {
    value.strip_prefix("rotate(")?.strip_suffix(')')?.parse::<u8>().ok().map(|r| r % 4)
}

fn parse_faces(value: &str) -> Vec<Face> {
    value
        .split(';')
        .filter_map(|entry| {
            let (rect, contact) = entry.split_once(',')?;
            Some(Face { rect: rect.parse().ok()?, contact_id: contact.trim().to_owned() })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ini::PicasaIni;

    #[test]
    fn reads_and_writes_known_fields() {
        let mut ini = PicasaIni::parse(
            "[a.jpg]\r\nbackuphash=60720\r\nstar=yes\r\nrotate=rotate(3)\r\nkeywords=sea,sun\r\n\
faces=rect64(3f845bcb59418507),8e62398ebda8c1a5;rect64(9eb15e89b6b584c1),d04a8b6e1d5d7b12\r\n",
        );
        let section = ini.section_mut("a.jpg");
        let mut meta = PhotoMeta::from_section(section);
        assert!(meta.star);
        assert_eq!(meta.rotate, 3);
        assert_eq!(meta.keywords, ["sea", "sun"]);
        assert_eq!(meta.faces.len(), 2);
        assert_eq!(meta.faces[1].contact_id, "d04a8b6e1d5d7b12");

        meta.star = false;
        meta.filters = crate::filters::FilterStack::parse("crop64=1,7fffffff;bw=1;");
        meta.write_to(section);
        assert_eq!(section.get("crop"), Some("rect64(7fffffff)"));
        assert_eq!(section.get("star"), None);
        assert_eq!(section.get("backuphash"), Some("60720"));
        assert_eq!(PhotoMeta::from_section(section).faces, meta.faces);
    }

    #[test]
    fn rewrites_only_what_changed() {
        // Written by Picasa or another tool, each in its own way.
        let text = "[a.jpg]\r\nkeywords=sea, sun\r\nrotate=rotate(0)\r\ncaption=\r\n\
filters=crop64=1,a4f0a44f56ef546;HDR=1,6.362573,1.771930,19.298244;\r\n";
        let mut ini = PicasaIni::parse(text);
        let section = ini.section_mut("a.jpg");
        let mut meta = PhotoMeta::from_section(section);
        meta.write_to(section);
        assert_eq!(ini.to_text(), text, "nothing changed, nothing rewritten");

        // A star: only the star is added, no crop mirror, no tidied keywords.
        let section = ini.section_mut("a.jpg");
        meta.star = true;
        meta.write_to(section);
        assert_eq!(ini.to_text(), format!("{text}star=yes\r\n"));

        // New edits: the stack is written, with the crop mirror.
        let section = ini.section_mut("a.jpg");
        meta.filters.0.push(crate::filters::Filter::new("bw", vec!["1".into()]));
        meta.write_to(section);
        assert!(section.get("filters").is_some_and(|f| f.ends_with("19.298244;bw=1;")));
        assert_eq!(section.get("crop"), Some("rect64(a4f0a44f56ef546)"));
        assert_eq!(section.get("keywords"), Some("sea, sun"));
    }
}
