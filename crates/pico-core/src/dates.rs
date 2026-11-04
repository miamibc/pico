//! Dates in the EXIF layout `YYYY:MM:DD HH:MM:SS`, which sorts as text.
//!
//! Sorting mixes dates from different clocks: a camera writes its local time,
//! a video and the file system keep UTC. Everything is brought to the local
//! time of this computer (with its daylight saving on that date), so a video
//! sits among the photos taken around it.

use chrono::{DateTime, FixedOffset, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use std::time::SystemTime;

const LAYOUT: &str = "%Y:%m:%d %H:%M:%S";

fn parse(text: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(text.get(..19)?, LAYOUT).ok()
}

/// A UTC date (a video's recording date) in this computer's local time.
pub fn utc_to_local(text: &str) -> Option<String> {
    let utc = parse(text)?.and_utc();
    Some(utc.with_timezone(&Local).format(LAYOUT).to_string())
}

/// A camera's local date written with its offset from UTC (EXIF
/// `OffsetTimeOriginal`, `+03:00`) in this computer's local time.
pub fn with_offset_to_local(text: &str, offset: &str) -> Option<String> {
    let offset = parse_offset(offset)?;
    let at = offset.from_local_datetime(&parse(text)?).single()?;
    Some(at.with_timezone(&Local).format(LAYOUT).to_string())
}

/// `+03:00` or `-05:30` as an offset; cameras with no zone set write blanks.
fn parse_offset(text: &str) -> Option<FixedOffset> {
    let text = text.trim();
    let sign = match text.get(..1)? {
        "+" => 1,
        "-" => -1,
        _ => return None,
    };
    let (hours, minutes) = text[1..].split_once(':')?;
    let seconds = hours.parse::<i32>().ok()? * 3600 + minutes.parse::<i32>().ok()? * 60;
    FixedOffset::east_opt(sign * seconds)
}

/// A file system time in this computer's local time.
pub fn system_to_local(time: SystemTime) -> String {
    DateTime::<Utc>::from(time).with_timezone(&Local).format(LAYOUT).to_string()
}

/// The earlier of a file's creation and modification times, ignoring ones
/// before 1980 (unset or broken clocks). Copying a file usually makes its
/// creation time "now" but keeps the modification time; editing it later
/// does the opposite. The earlier one is the closer guess either way.
pub fn earliest_file_time(created: Option<SystemTime>, modified: Option<SystemTime>) -> Option<SystemTime> {
    let floor = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(315_532_800); // 1980-01-01
    [created, modified].into_iter().flatten().filter(|t| *t >= floor).min()
}

/// The date and time phones and messengers write into file names:
/// `IMG_20220421_153012`, `PXL_20220421_153012345`, `VID-20220421-WA0001`,
/// `Screenshot_2022-04-21-15-30-12`, `signal-2022-04-21-153012`. A name
/// with only a date gives midnight, so such files keep their names' order.
pub fn from_file_name(name: &str) -> Option<String> {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let groups: Vec<&str> = stem.split(|c: char| !c.is_ascii_digit()).filter(|g| !g.is_empty()).collect();
    let number = |s: &str| s.parse::<u32>().ok();
    for start in 0..groups.len() {
        let (date, used) = match groups[start..] {
            [ymd, ..] if ymd.len() == 8 => (date_of(number(&ymd[..4])?, number(&ymd[4..6])?, number(&ymd[6..])?), 1),
            [y, m, d, ..] if (y.len(), m.len(), d.len()) == (4, 2, 2) => {
                (date_of(number(y)?, number(m)?, number(d)?), 3)
            }
            _ => continue,
        };
        let Some(date) = date else { continue };
        let time = match groups[start + used..] {
            [hms, ..] if hms.len() >= 6 => time_of(&hms[..2], &hms[2..4], &hms[4..6]),
            [h, m, s, ..] if (h.len(), m.len(), s.len()) == (2, 2, 2) => time_of(h, m, s),
            _ => None,
        };
        let time = time.unwrap_or_default();
        return Some(date.and_time(time).format(LAYOUT).to_string());
    }
    None
}

fn date_of(year: u32, month: u32, day: u32) -> Option<NaiveDate> {
    if !(1990..=2099).contains(&year) {
        return None;
    }
    NaiveDate::from_ymd_opt(year as i32, month, day)
}

fn time_of(h: &str, m: &str, s: &str) -> Option<chrono::NaiveTime> {
    chrono::NaiveTime::from_hms_opt(h.parse().ok()?, m.parse().ok()?, s.parse().ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_in_file_names() {
        let cases = [
            ("IMG_20220421_153012.jpg", Some("2022:04:21 15:30:12")),
            ("PXL_20220421_153012345.MP.jpg", Some("2022:04:21 15:30:12")),
            ("VID_20220421_153012.mp4", Some("2022:04:21 15:30:12")),
            ("IMG-20220421-WA0001.jpg", Some("2022:04:21 00:00:00")),
            ("Screenshot_2022-04-21-15-30-12.png", Some("2022:04:21 15:30:12")),
            ("signal-2022-04-21-153012.jpg", Some("2022:04:21 15:30:12")),
            ("2022-04-21 Мины на кладбище-H8FUn6scJwk.mp4", Some("2022:04:21 00:00:00")),
            ("20220421_153012.jpg", Some("2022:04:21 15:30:12")),
            // Counters and camera numbering are not dates.
            ("IMG_2418.JPG", None),
            ("DSC01234.ARW", None),
            ("12345678.jpg", None),
            ("photo_20221345.jpg", None),
            ("holiday.png", None),
        ];
        for (name, expected) in cases {
            assert_eq!(from_file_name(name).as_deref(), expected, "{name}");
        }
    }

    #[test]
    fn offsets_and_utc_meet_in_local_time() {
        // The same moment: 15:30 in Tallinn summer time is 12:30 UTC.
        let from_camera = with_offset_to_local("2022:06:21 15:30:12", "+03:00").unwrap();
        let from_video = utc_to_local("2022:06:21 12:30:12").unwrap();
        assert_eq!(from_camera, from_video);
        assert_eq!(with_offset_to_local("2022:06:21 15:30:12", "   :  "), None);
        assert_eq!(utc_to_local("0000:00:00 00:00:00"), None);
    }

    #[test]
    fn earliest_file_time_skips_broken_clocks() {
        let day = std::time::Duration::from_secs(86_400);
        let old = SystemTime::UNIX_EPOCH + day * 20_000;
        let new = old + day;
        assert_eq!(earliest_file_time(Some(new), Some(old)), Some(old));
        assert_eq!(earliest_file_time(Some(SystemTime::UNIX_EPOCH), Some(new)), Some(new));
        assert_eq!(earliest_file_time(None, None), None);
    }
}
