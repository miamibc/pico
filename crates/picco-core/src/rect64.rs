//! Picasa's `rect64` rectangles, used by crops and face regions.
//!
//! A rectangle is a 64-bit number written in hex without leading zeros. From the
//! most significant end it holds four 16-bit fields: left, top, right, bottom,
//! each a fraction of the image size scaled to 0..=65535.

use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rect64 {
    pub left: u16,
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
}

impl Rect64 {
    pub const FULL: Self = Self { left: 0, top: 0, right: u16::MAX, bottom: u16::MAX };

    pub fn to_bits(self) -> u64 {
        (self.left as u64) << 48 | (self.top as u64) << 32 | (self.right as u64) << 16 | self.bottom as u64
    }

    pub fn from_bits(bits: u64) -> Self {
        Self { left: (bits >> 48) as u16, top: (bits >> 32) as u16, right: (bits >> 16) as u16, bottom: bits as u16 }
    }

    /// Returns `[left, top, right, bottom]` as fractions of the image size.
    pub fn to_unit(self) -> [f32; 4] {
        let f = |v: u16| v as f32 / u16::MAX as f32;
        [f(self.left), f(self.top), f(self.right), f(self.bottom)]
    }

    pub fn from_unit([left, top, right, bottom]: [f32; 4]) -> Self {
        let q = |v: f32| (v.clamp(0.0, 1.0) * u16::MAX as f32).round() as u16;
        Self { left: q(left), top: q(top), right: q(right), bottom: q(bottom) }
    }
}

impl FromStr for Rect64 {
    type Err = std::num::ParseIntError;

    /// Accepts both the bare hex form (`crop64=1,<hex>`) and `rect64(<hex>)`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex = s.trim();
        let hex = hex.strip_prefix("rect64(").and_then(|h| h.strip_suffix(')')).unwrap_or(hex);
        u64::from_str_radix(hex, 16).map(Self::from_bits)
    }
}

impl fmt::Display for Rect64 {
    /// Bare hex, as written after `crop64=1,`. Wrap in `rect64(...)` where needed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:x}", self.to_bits())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_value_without_leading_zeros() {
        let r: Rect64 = "cbc3ffff".parse().unwrap();
        assert_eq!(r, Rect64 { left: 0, top: 0, right: 0xcbc3, bottom: 0xffff });
        assert_eq!(r.to_string(), "cbc3ffff");
    }

    #[test]
    fn parses_wrapped_form() {
        let r: Rect64 = "rect64(b042c64ffff9a6e)".parse().unwrap();
        assert_eq!(r, Rect64 { left: 0x0b04, top: 0x2c64, right: 0xffff, bottom: 0x9a6e });
    }
}
