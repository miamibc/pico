//! The edit stack stored in a photo's `filters=` key.
//!
//! Format: `id=<params>;id=<params>;...`. Most edits start their parameters
//! with an enabled flag (`1`), followed by comma-separated values whose order is
//! defined per filter (see `docs/research/picasa-filter-layout.tsv`). Video trim
//! points are the exception: `moviestart=<hex>` holds only a time.
//!
//! Parameters are kept as text so that stacks round-trip exactly, including
//! filters Pico does not implement yet.

use crate::rect64::Rect64;
use std::fmt;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct Filter {
    pub id: String,
    /// Raw comma-separated parameters, enabled flag included where present.
    pub params: Vec<String>,
}

impl Filter {
    pub fn new(id: impl Into<String>, params: Vec<String>) -> Self {
        Self { id: id.into(), params }
    }

    /// An enabled edit with the given values, formatted like Picasa writes them.
    pub fn effect(id: impl Into<String>, values: impl IntoIterator<Item = Value>) -> Self {
        let params = std::iter::once("1".to_owned()).chain(values.into_iter().map(|v| v.to_string())).collect();
        Self::new(id, params)
    }

    /// Filters that store a bare value instead of an enabled flag.
    fn is_time(&self) -> bool {
        matches!(self.id.as_str(), "moviestart" | "movieend")
    }

    pub fn enabled(&self) -> bool {
        self.is_time() || self.params.first().is_none_or(|flag| flag != "0")
    }

    /// Values after the enabled flag. Missing values mean "use the default".
    pub fn values(&self) -> &[String] {
        if self.is_time() { &self.params } else { self.params.get(1..).unwrap_or(&[]) }
    }

    pub fn float(&self, index: usize) -> Option<f32> {
        self.values().get(index)?.parse().ok()
    }

    /// Colors are stored as `%08x`; Picasa packs them as ARGB.
    pub fn color(&self, index: usize) -> Option<u32> {
        u32::from_str_radix(self.values().get(index)?, 16).ok()
    }

    pub fn crop(&self) -> Option<Rect64> {
        (self.id == "crop64").then(|| self.values().first()?.parse().ok())?
    }

    /// Trim point of `moviestart`/`movieend`; stored in 100 ns units.
    pub fn time(&self) -> Option<Duration> {
        if !self.is_time() {
            return None;
        }
        let ticks = u64::from_str_radix(self.params.first()?, 16).ok()?;
        Some(Duration::from_nanos(ticks.saturating_mul(100)))
    }

    pub fn trim(id: &str, at: Duration) -> Self {
        let ticks = (at.as_nanos() / 100) as u64;
        Self::new(id, vec![format!("{ticks:x}")])
    }
}

/// One parameter value of an edit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    /// Written as `%f` (six decimals).
    Float(f32),
    /// ARGB, written as `%08x`.
    Color(u32),
    /// Written as `%d` (e.g. a quarter-turn count).
    Int(i32),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Float(v) => write!(f, "{v:.6}"),
            Value::Color(c) => write!(f, "{c:08x}"),
            Value::Int(i) => write!(f, "{i}"),
        }
    }
}

impl fmt::Display for Filter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}={};", self.id, self.params.join(","))
    }
}

/// Ordered edits; the same filter may appear several times as history.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FilterStack(pub Vec<Filter>);

impl FilterStack {
    pub fn parse(text: &str) -> Self {
        let filters = text
            .split(';')
            .filter(|part| !part.trim().is_empty())
            .map(|part| match part.split_once('=') {
                Some((id, params)) => Filter::new(id.trim(), params.split(',').map(str::to_owned).collect()),
                None => Filter::new(part.trim(), Vec::new()),
            })
            .collect();
        Self(filters)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The crop in effect: the last enabled `crop64`.
    pub fn crop(&self) -> Option<Rect64> {
        self.0.iter().rev().filter(|f| f.enabled()).find_map(Filter::crop)
    }
}

impl fmt::Display for FilterStack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|filter| filter.fmt(f))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_real_stack() {
        let text = "unsharp=1;crop64=1,a5c0050fb46f104;\
finetune2=1,0.561404,0.148772,0.188070,ff686761,-0.450292;sat=1,0.005848;";
        let stack = FilterStack::parse(text);
        assert_eq!(stack.to_string(), text);

        let finetune = &stack.0[2];
        assert_eq!(finetune.float(0), Some(0.561404));
        assert_eq!(finetune.color(3), Some(0xff686761));
        assert_eq!(finetune.float(4), Some(-0.450292));
        assert!(stack.0[0].values().is_empty());
    }

    #[test]
    fn builds_edits_in_picasa_format() {
        use Value::{Color, Float};
        let tuning =
            Filter::effect("finetune2", [Float(0.5), Float(0.0), Float(0.19), Color(0xfff7e7e6), Float(-0.45)]);
        assert_eq!(tuning.to_string(), "finetune2=1,0.500000,0.000000,0.190000,fff7e7e6,-0.450000;");
        assert_eq!(Filter::effect("bw", []).to_string(), "bw=1;");
    }

    #[test]
    fn last_crop_wins() {
        let stack = FilterStack::parse("crop64=1,52dffffbae6;tilt=1,0.691700,0.000000;crop64=1,e2c0c2dffff9797;");
        assert_eq!(stack.crop(), "e2c0c2dffff9797".parse().ok());
    }

    #[test]
    fn video_trim_points() {
        let start = Filter::trim("moviestart", Duration::from_secs(5));
        assert_eq!(start.to_string(), "moviestart=2faf080;");
        let parsed = FilterStack::parse(&start.to_string());
        assert_eq!(parsed.0[0].time(), Some(Duration::from_secs(5)));
    }
}
