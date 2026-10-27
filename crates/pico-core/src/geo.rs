//! Place names for GPS positions, without the network.
//!
//! `data/places.bin` (made by `data/make_places.py`) holds the GeoNames towns
//! of 1000 people or more, and Natural Earth's countries, seas and oceans, and
//! continents. A position gets the nearest town within that town's reach,
//! else the country it lies in, else the sea, else the continent.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const PACKED: &[u8] = include_bytes!("../data/places.bin");

/// Where the place names come from, for Help → About.
pub const CREDITS: &str = "Place names: GeoNames (CC BY 4.0) and Natural Earth.";

/// "Tallinn, Estonia", "Monaco", "Estonia", "Baltic Sea" or "Europe"; `None` if the
/// position is in none of them.
pub fn place_name(lat: f64, lon: f64) -> Option<String> {
    // The Places view asks every frame; remember answers to the meter.
    static CACHE: OnceLock<Mutex<Answers>> = OnceLock::new();
    let key = ((lat * 1e5).round() as i32, (lon * 1e5).round() as i32);
    let cache = CACHE.get_or_init(Default::default);
    if let Some(name) = cache.lock().unwrap().get(&key) {
        return name.clone();
    }
    let name = data().lookup(lat, lon);
    cache.lock().unwrap().insert(key, name.clone());
    name
}

/// Names by position in 1e-5 degrees.
type Answers = HashMap<(i32, i32), Option<String>>;

struct Town {
    lat: f64,
    lon: f64,
    radius_km: f64,
    country: u16,
    name: String,
}

struct Shape {
    name: String,
    /// South, west, north, east.
    bounds: (f64, f64, f64, f64),
    /// (lat, lon) rings; holes and islands alike, by the even-odd rule.
    rings: Vec<Vec<(f64, f64)>>,
}

struct Data {
    country_names: Vec<String>,
    towns: Vec<Town>,
    /// Town indices by whole-degree cell, (lat + 90) * 360 + (lon + 180).
    cells: HashMap<u32, Vec<u32>>,
    countries: Vec<Shape>,
    seas: Vec<Shape>,
    continents: Vec<Shape>,
}

fn data() -> &'static Data {
    static DATA: OnceLock<Data> = OnceLock::new();
    DATA.get_or_init(|| {
        let bytes = miniz_oxide::inflate::decompress_to_vec_zlib(PACKED).expect("places.bin is zlib");
        Data::parse(&bytes).expect("places.bin is well formed")
    })
}

const KM_PER_DEGREE: f64 = 111.2;

fn cell(lat: f64, lon: f64) -> u32 {
    let row = (lat.floor() + 90.0).clamp(0.0, 179.0) as u32;
    let col = (lon.floor() + 180.0).rem_euclid(360.0) as u32;
    row * 360 + col
}

impl Data {
    fn lookup(&self, lat: f64, lon: f64) -> Option<String> {
        if let Some(town) = self.nearest_town(lat, lon) {
            let country = &self.country_names[town.country as usize];
            // Monaco, Singapore: the town is the country.
            if &town.name == country {
                return Some(town.name.clone());
            }
            return Some(format!("{}, {country}", town.name));
        }
        [&self.countries, &self.seas, &self.continents]
            .into_iter()
            .find_map(|shapes| smallest_containing(shapes, lat, lon))
            .map(|shape| shape.name.clone())
    }

    /// The town whose reach covers the point best: distance over reach, so a
    /// city wins over a village at its edge only when nearer in its terms.
    fn nearest_town(&self, lat: f64, lon: f64) -> Option<&Town> {
        const MAX_REACH_KM: f64 = 35.0;
        let cos = lat.to_radians().cos().max(0.01);
        let dlat = (MAX_REACH_KM / KM_PER_DEGREE).ceil() as i32;
        let dlon = (MAX_REACH_KM / (KM_PER_DEGREE * cos)).ceil().min(180.0) as i32;
        let mut best: Option<(f64, &Town)> = None;
        for row in -dlat..=dlat {
            for col in -dlon..=dlon {
                let Some(ids) = self.cells.get(&cell(lat + row as f64, lon + col as f64)) else { continue };
                for &id in ids {
                    let town = &self.towns[id as usize];
                    let mut dx = (town.lon - lon).abs();
                    if dx > 180.0 {
                        dx = 360.0 - dx;
                    }
                    let km = KM_PER_DEGREE * ((town.lat - lat).powi(2) + (dx * cos).powi(2)).sqrt();
                    let score = km / town.radius_km;
                    if score <= 1.0 && best.is_none_or(|(s, _)| score < s) {
                        best = Some((score, town));
                    }
                }
            }
        }
        best.map(|(_, town)| town)
    }

    fn parse(b: &[u8]) -> Option<Data> {
        let mut r = Reader(b);
        if r.take(8)? != b"PICOGEO1" {
            return None;
        }
        let country_names = (0..r.u16()?).map(|_| r.text()).collect::<Option<Vec<_>>>()?;
        let mut towns = Vec::new();
        let mut cells: HashMap<u32, Vec<u32>> = HashMap::new();
        for id in 0..r.u32()? {
            let (lat, lon) = r.point()?;
            let radius_km = r.u8()? as f64;
            let country = r.u16()?;
            let name = r.text()?;
            if country as usize >= country_names.len() {
                return None;
            }
            cells.entry(cell(lat, lon)).or_default().push(id);
            towns.push(Town { lat, lon, radius_km, country, name });
        }
        let [countries, seas, continents] = [r.shapes()?, r.shapes()?, r.shapes()?];
        Some(Data { country_names, towns, cells, countries, seas, continents })
    }
}

/// Seas lie inside oceans' bounds, so the smallest match is the most precise.
fn smallest_containing(shapes: &[Shape], lat: f64, lon: f64) -> Option<&Shape> {
    let area = |s: &Shape| (s.bounds.2 - s.bounds.0) * (s.bounds.3 - s.bounds.1);
    shapes
        .iter()
        .filter(|s| (s.bounds.0..=s.bounds.2).contains(&lat) && (s.bounds.1..=s.bounds.3).contains(&lon))
        .filter(|s| contains(&s.rings, lat, lon))
        .min_by(|a, b| area(a).total_cmp(&area(b)))
}

/// Ray casting along the latitude, by the even-odd rule over all rings.
fn contains(rings: &[Vec<(f64, f64)>], lat: f64, lon: f64) -> bool {
    let mut inside = false;
    for ring in rings {
        let Some(&last) = ring.last() else { continue };
        let mut prev = last;
        for &point in ring {
            let ((y1, x1), (y2, x2)) = (prev, point);
            if (y1 > lat) != (y2 > lat) && lon < x1 + (lat - y1) * (x2 - x1) / (y2 - y1) {
                inside = !inside;
            }
            prev = point;
        }
    }
    inside
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn point(&mut self) -> Option<(f64, f64)> {
        let mut coord = || Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?) as f64 / 1e5);
        Some((coord()?, coord()?))
    }

    fn text(&mut self) -> Option<String> {
        let n = self.u8()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }

    fn shapes(&mut self) -> Option<Vec<Shape>> {
        let mut shapes = Vec::new();
        for _ in 0..self.u16()? {
            let name = self.text()?;
            let mut rings = Vec::new();
            let mut bounds = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for _ in 0..self.u16()? {
                let ring = (0..self.u32()?).map(|_| self.point()).collect::<Option<Vec<_>>>()?;
                for &(lat, lon) in &ring {
                    bounds = (bounds.0.min(lat), bounds.1.min(lon), bounds.2.max(lat), bounds.3.max(lon));
                }
                rings.push(ring);
            }
            shapes.push(Shape { name, bounds, rings });
        }
        Some(shapes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn towns_then_countries_then_seas_then_continents() {
        assert_eq!(place_name(59.437, 24.745).as_deref(), Some("Tallinn, Estonia"));
        assert_eq!(place_name(-33.87, 151.21).as_deref(), Some("Sydney, Australia"));
        assert_eq!(place_name(43.7, 7.42).as_deref(), Some("Monaco"));
        // Deep in the Sahara, far from any town.
        assert_eq!(place_name(23.0, 12.0).as_deref(), Some("Niger"));
        assert_eq!(place_name(57.5, 19.5).as_deref(), Some("Baltic Sea"));
        assert_eq!(place_name(0.0, -140.0).as_deref(), Some("South Pacific Ocean"));
        assert_eq!(place_name(-82.0, 60.0).as_deref(), Some("Antarctica"));
    }

    #[test]
    fn across_the_date_line() {
        let fiji = place_name(-16.8, 179.99).unwrap_or_default();
        assert!(fiji.ends_with("Fiji"), "{fiji}");
    }
}
