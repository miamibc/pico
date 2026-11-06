//! Picture collages (Picasa's Create > Picture Collage): several photos laid
//! out on one page and saved as a new picture.

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use image::{DynamicImage, GenericImageView, Rgba, RgbaImage, imageops};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Photos fitted edge to edge in rows that fill the page.
    Mosaic,
    /// A mosaic around one large photo in the middle.
    FrameMosaic,
    /// Equal cells in rows and columns.
    Grid,
    /// Thumbnails with file names under a header.
    ContactSheet,
    /// All photos laid over each other.
    MultipleExposure,
}

impl Layout {
    pub const ALL: [(Layout, &'static str); 5] = [
        (Layout::Mosaic, "Mosaic"),
        (Layout::FrameMosaic, "Frame Mosaic"),
        (Layout::Grid, "Grid"),
        (Layout::ContactSheet, "Contact Sheet"),
        (Layout::MultipleExposure, "Multiple Exposure"),
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Border {
    None,
    White,
    /// A white frame with a wide bottom strip for the caption.
    InstantCamera,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Background {
    /// RGB color.
    Color(u32),
    /// One of the collage's photos, filling the page.
    Photo(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CollageOptions {
    pub layout: Layout,
    /// Page width in pixels; the height follows from `aspect`.
    pub width: u32,
    /// Page width / height.
    pub aspect: f32,
    pub background: Background,
    pub border: Border,
    pub shadows: bool,
    pub captions: bool,
    /// Gap between pictures as a fraction of the page width (0..=0.1).
    pub spacing: f32,
    /// Seed for "Shuffle Pictures".
    pub seed: u64,
    /// The photo in the middle of a Frame Mosaic.
    pub center: usize,
    /// Header of a Contact Sheet.
    pub title: String,
}

impl Default for CollageOptions {
    fn default() -> Self {
        Self {
            layout: Layout::Mosaic,
            width: 2400,
            aspect: 1.5,
            background: Background::Color(0xffffff),
            border: Border::White,
            shadows: true,
            captions: false,
            spacing: 0.01,
            seed: 1,
            center: 0,
            title: String::new(),
        }
    }
}

/// One photo, already rendered with its edits.
pub struct Item {
    pub image: DynamicImage,
    pub name: String,
    pub caption: Option<String>,
}

/// Where one picture goes: center, size, turn, and whether it is cropped to
/// fill its box (`true`) or fitted inside it.
struct Placement {
    item: usize,
    cx: f32,
    cy: f32,
    w: f32,
    h: f32,
    degrees: f32,
    fill: bool,
    caption_below: bool,
}

pub fn page_size(options: &CollageOptions) -> (u32, u32) {
    let w = options.width.max(64);
    (w, ((w as f32 / options.aspect.clamp(0.2, 5.0)).round() as u32).max(64))
}

/// Renders the collage. Items are used in the order given ("Shuffle Pictures"
/// reorders them before calling this).
pub fn render(items: &[&Item], options: &CollageOptions) -> RgbaImage {
    let (w, h) = page_size(options);
    let mut page = RgbaImage::from_pixel(
        w,
        h,
        Rgba(rgb(match options.background {
            Background::Color(c) => c,
            Background::Photo(_) => 0xffffff,
        })),
    );
    if let Background::Photo(i) = options.background
        && let Some(item) = items.get(i)
    {
        let cover = cover(&item.image, w, h);
        imageops::overlay(&mut page, &cover, 0, 0);
    }
    if items.is_empty() {
        return page;
    }
    if options.layout == Layout::MultipleExposure {
        multiple_exposure(&mut page, items);
        return page;
    }
    let font = FontRef::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT).expect("bundled font");
    let mut top = 0.0;
    if options.layout == Layout::ContactSheet {
        top = header(&mut page, &font, items.len(), &options.title);
    }
    let placements = match options.layout {
        Layout::Mosaic => mosaic(items, 0..items.len(), (0.0, 0.0, w as f32, h as f32), gap(options, w)),
        Layout::FrameMosaic => {
            frame_mosaic(items, options.center.min(items.len() - 1), w as f32, h as f32, gap(options, w))
        }
        Layout::Grid => grid(items.len(), (0.0, 0.0, w as f32, h as f32), gap(options, w), true, false),
        Layout::ContactSheet => {
            grid(items.len(), (0.0, top, w as f32, h as f32 - top), gap(options, w).max(w as f32 * 0.01), false, true)
        }
        Layout::MultipleExposure => unreachable!("handled above"),
    };
    let border = if options.layout == Layout::ContactSheet { Border::None } else { options.border };
    // Shadows fall into the gaps between pictures; with no room for them they
    // would only smear the neighbours, so they are skipped.
    let room = gap(options, w) / 2.0;
    let shadow = (options.shadows && options.layout != Layout::ContactSheet && room >= 2.0).then_some(room);
    for p in &placements {
        let item = &items[p.item];
        let caption = if p.caption_below || (options.captions && border == Border::InstantCamera) {
            Some(item.caption.clone().unwrap_or_else(|| item.name.clone()))
        } else {
            None
        };
        draw_picture(&mut page, &font, item, p, border, shadow, caption);
    }
    page
}

fn gap(options: &CollageOptions, width: u32) -> f32 {
    options.spacing.clamp(0.0, 0.1) * width as f32
}

/// `n` equal cells filling the box, as square as the page allows.
fn grid(n: usize, (x, y, w, h): (f32, f32, f32, f32), gap: f32, fill: bool, captions: bool) -> Vec<Placement> {
    let cols = ((n as f32 * w / h).sqrt().ceil() as usize).clamp(1, n);
    let rows = n.div_ceil(cols);
    let cell_w = (w - gap * (cols as f32 + 1.0)) / cols as f32;
    let cell_h = (h - gap * (rows as f32 + 1.0)) / rows as f32;
    (0..n)
        .map(|i| {
            let (c, r) = (i % cols, i / cols);
            Placement {
                item: i,
                cx: x + gap + c as f32 * (cell_w + gap) + cell_w / 2.0,
                cy: y + gap + r as f32 * (cell_h + gap) + cell_h / 2.0,
                w: cell_w,
                h: cell_h,
                degrees: 0.0,
                fill,
                caption_below: captions,
            }
        })
        .collect()
}

/// Justified rows: photos keep their shape and each row spans the box; the
/// number of rows that best fills the height wins, then rows are stretched to
/// fit exactly (cropping each photo a little).
fn mosaic(
    items: &[&Item],
    range: std::ops::Range<usize>,
    (x, y, w, h): (f32, f32, f32, f32),
    gap: f32,
) -> Vec<Placement> {
    let order: Vec<usize> = range.collect();
    if order.is_empty() {
        return Vec::new();
    }
    let aspect = |i: usize| {
        let (iw, ih) = items[i].image.dimensions();
        iw as f32 / ih.max(1) as f32
    };
    let total: f32 = order.iter().map(|&i| aspect(i)).sum();
    let mut best: Option<(f32, Vec<Vec<usize>>)> = None;
    for rows in 1..=order.len() {
        // Greedy split into rows of about equal summed aspect.
        let target = total / rows as f32;
        let mut split: Vec<Vec<usize>> = vec![Vec::new()];
        let mut acc = 0.0;
        for &i in &order {
            if acc >= target * 0.999 && split.len() < rows {
                split.push(Vec::new());
                acc = 0.0;
            }
            split.last_mut().expect("a row").push(i);
            acc += aspect(i);
        }
        let height: f32 = split
            .iter()
            .map(|row| (w - gap * (row.len() as f32 + 1.0)) / row.iter().map(|&i| aspect(i)).sum::<f32>())
            .sum::<f32>()
            + gap * (split.len() as f32 + 1.0);
        let error = (height - h).abs() / h;
        if best.as_ref().is_none_or(|(e, _)| error < *e) {
            best = Some((error, split));
        }
    }
    let rows = best.expect("at least one layout").1;
    let natural: Vec<f32> = rows
        .iter()
        .map(|row| (w - gap * (row.len() as f32 + 1.0)) / row.iter().map(|&i| aspect(i)).sum::<f32>())
        .collect();
    let stretch = (h - gap * (rows.len() as f32 + 1.0)) / natural.iter().sum::<f32>();
    let mut placements = Vec::new();
    let mut cursor_y = y + gap;
    for (row, row_h) in rows.iter().zip(&natural) {
        let row_h = row_h * stretch;
        let mut cursor_x = x + gap;
        let width_left = w - gap * (row.len() as f32 + 1.0);
        let sum: f32 = row.iter().map(|&i| aspect(i)).sum();
        for &i in row {
            let cell_w = width_left * aspect(i) / sum;
            placements.push(Placement {
                item: i,
                cx: cursor_x + cell_w / 2.0,
                cy: cursor_y + row_h / 2.0,
                w: cell_w,
                h: row_h,
                degrees: 0.0,
                fill: true,
                caption_below: false,
            });
            cursor_x += cell_w + gap;
        }
        cursor_y += row_h + gap;
    }
    placements
}

/// The center photo fills the middle block of a k x k grid; the others go round
/// it in the ring of cells, repeating when there are fewer photos than cells,
/// so the frame is always complete.
fn frame_mosaic(items: &[&Item], center: usize, w: f32, h: f32, gap: f32) -> Vec<Placement> {
    let mut others: Vec<usize> = (0..items.len()).filter(|&i| i != center).collect();
    if others.is_empty() {
        others.push(center);
    }
    // A k x k grid has 4(k-1) cells around its middle block; at least 4 so the
    // middle photo is clearly bigger than the frame ones.
    let mut k = 4;
    while 4 * (k - 1) < others.len() {
        k += 1;
    }
    let (cell_w, cell_h) = ((w - gap * (k as f32 + 1.0)) / k as f32, (h - gap * (k as f32 + 1.0)) / k as f32);
    let cell = |c: usize, r: usize| {
        (gap + c as f32 * (cell_w + gap) + cell_w / 2.0, gap + r as f32 * (cell_h + gap) + cell_h / 2.0)
    };
    let ring: Vec<(usize, usize)> = (0..k)
        .map(|c| (c, 0))
        .chain((1..k).map(|r| (k - 1, r)))
        .chain((0..k - 1).rev().map(|c| (c, k - 1)))
        .chain((1..k - 1).rev().map(|r| (0, r)))
        .collect();
    let mut placements: Vec<Placement> = ring
        .iter()
        .enumerate()
        .map(|(n, &(c, r))| {
            let (cx, cy) = cell(c, r);
            let item = others[n % others.len()];
            Placement { item, cx, cy, w: cell_w, h: cell_h, degrees: 0.0, fill: true, caption_below: false }
        })
        .collect();
    let inner = (k - 2) as f32;
    placements.push(Placement {
        item: center,
        cx: w / 2.0,
        cy: h / 2.0,
        w: inner * cell_w + (inner - 1.0) * gap,
        h: inner * cell_h + (inner - 1.0) * gap,
        degrees: 0.0,
        fill: true,
        caption_below: false,
    });
    placements
}

fn multiple_exposure(page: &mut RgbaImage, items: &[&Item]) {
    let (w, h) = page.dimensions();
    // Running average: the k-th photo weighs 1/k, so all end up equal.
    let mut acc = vec![[0.0f32; 3]; (w * h) as usize];
    for item in items {
        let layer = cover(&item.image, w, h);
        for (a, p) in acc.iter_mut().zip(layer.pixels()) {
            for (sum, value) in a.iter_mut().zip(p.0) {
                *sum += value as f32;
            }
        }
    }
    let n = items.len() as f32;
    for (p, a) in page.pixels_mut().zip(&acc) {
        p.0 = [(a[0] / n) as u8, (a[1] / n) as u8, (a[2] / n) as u8, 255];
    }
}

/// Contact sheet header; returns its height.
fn header(page: &mut RgbaImage, font: &FontRef, count: usize, title: &str) -> f32 {
    let w = page.width() as f32;
    let size = (w * 0.03).max(14.0);
    let title = if title.trim().is_empty() { "Contact Sheet" } else { title.trim() };
    draw_text(page, font, title, w * 0.02, size * 0.6, size, [40, 40, 40]);
    let sub = format!("{count} pictures");
    draw_text(page, font, &sub, w * 0.02, size * 1.8, size * 0.55, [110, 110, 110]);
    size * 2.8
}

fn draw_picture(
    page: &mut RgbaImage,
    font: &FontRef,
    item: &Item,
    p: &Placement,
    border: Border,
    // Largest shadow offset that still fits the gap, if shadows are on.
    shadow: Option<f32>,
    caption: Option<String>,
) {
    let text_room = if p.caption_below { (p.h * 0.14).max(12.0) } else { 0.0 };
    let (box_w, box_h) = (p.w.max(1.0), (p.h - text_room).max(1.0));
    // Room for the frame inside the box.
    let (pad_side, pad_bottom) = match border {
        Border::None => (0.0, 0.0),
        Border::White => (0.04, 0.04),
        Border::InstantCamera => (0.06, 0.22),
    };
    let inner_w = box_w / (1.0 + 2.0 * pad_side);
    let inner_h = box_h / (1.0 + pad_side + pad_bottom);
    let photo = if p.fill {
        cover(&item.image, inner_w.round().max(1.0) as u32, inner_h.round().max(1.0) as u32)
    } else {
        contain(&item.image, inner_w.round().max(1.0) as u32, inner_h.round().max(1.0) as u32)
    };
    let (pw, ph) = photo.dimensions();
    let side = (pw.max(ph) as f32 * pad_side).round() as u32;
    let bottom = (pw.max(ph) as f32 * pad_bottom).round() as u32;
    let mut card = if border == Border::None {
        photo
    } else {
        let mut card = RgbaImage::from_pixel(pw + 2 * side, ph + side + bottom, Rgba([250, 250, 248, 255]));
        imageops::overlay(&mut card, &photo, side as i64, side as i64);
        if border == Border::InstantCamera
            && let Some(text) = &caption
        {
            let size = (bottom as f32 * 0.32).max(10.0);
            let width = text_width(font, text, size);
            let x = (card.width() as f32 - width) / 2.0;
            draw_text(&mut card, font, text, x, (ph + side) as f32 + (bottom as f32 - size) / 2.0, size, [50, 50, 50]);
        }
        card
    };
    if p.degrees.abs() > 0.01 {
        card = rotate(&card, p.degrees);
    }
    let x = (p.cx - card.width() as f32 / 2.0).round() as i64;
    let y = (p.cy - (card.height() as f32 + text_room) / 2.0).round() as i64;
    if let Some(room) = shadow {
        let offset = (card.width().max(card.height()) as f32 * 0.012).clamp(2.0, room) as i64;
        // A soft shadow needs no detail: blur a quarter-size mask, then scale up.
        let (sw, sh) = ((card.width() / 4).max(1), (card.height() / 4).max(1));
        let small = imageops::resize(&card, sw, sh, imageops::FilterType::Triangle);
        let mask = RgbaImage::from_fn(sw, sh, |cx, cy| Rgba([0, 0, 0, small.get_pixel(cx, cy).0[3] / 3]));
        // Pad so the blur can spread past the card's outline.
        let pad = (offset as u32 / 2).max(1);
        let mut padded = RgbaImage::new(sw + 2 * pad, sh + 2 * pad);
        imageops::overlay(&mut padded, &mask, pad as i64, pad as i64);
        let blurred = imageops::blur(&padded, (offset as f32 * 0.25).max(0.6));
        let (shade_w, shade_h) = (card.width() + 8 * pad, card.height() + 8 * pad);
        let shade = imageops::resize(&blurred, shade_w, shade_h, imageops::FilterType::Triangle);
        imageops::overlay(page, &shade, x + offset - 4 * pad as i64, y + offset - 4 * pad as i64);
    }
    imageops::overlay(page, &card, x, y);
    if p.caption_below
        && let Some(text) = caption
    {
        let size = (text_room * 0.6).max(9.0);
        let shown = ellipsize(font, &text, size, p.w * 0.95);
        let width = text_width(font, &shown, size);
        draw_text(
            page,
            font,
            &shown,
            p.cx - width / 2.0,
            y as f32 + card.height() as f32 + text_room * 0.15,
            size,
            [60, 60, 60],
        );
    }
}

/// Scales and center-crops to exactly `w` x `h`.
pub fn cover(img: &DynamicImage, w: u32, h: u32) -> RgbaImage {
    let (iw, ih) = img.dimensions();
    let scale = (w as f32 / iw as f32).max(h as f32 / ih as f32);
    let (sw, sh) = (((iw as f32 * scale).ceil() as u32).max(w), ((ih as f32 * scale).ceil() as u32).max(h));
    let scaled = img.resize_exact(sw, sh, imageops::FilterType::Triangle).to_rgba8();
    imageops::crop_imm(&scaled, (sw - w) / 2, (sh - h) / 2, w, h).to_image()
}

/// Scales to fit inside `w` x `h`, keeping the shape.
fn contain(img: &DynamicImage, w: u32, h: u32) -> RgbaImage {
    img.resize(w, h, imageops::FilterType::Triangle).to_rgba8()
}

/// Rotates around the center onto a transparent canvas big enough for the result.
fn rotate(img: &RgbaImage, degrees: f32) -> RgbaImage {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let (w, h) = (img.width() as f32, img.height() as f32);
    let (nw, nh) = ((w * cos.abs() + h * sin.abs()).ceil(), (w * sin.abs() + h * cos.abs()).ceil());
    let (cx, cy, ncx, ncy) = (w / 2.0, h / 2.0, nw / 2.0, nh / 2.0);
    RgbaImage::from_fn(nw as u32, nh as u32, |x, y| {
        let (dx, dy) = (x as f32 + 0.5 - ncx, y as f32 + 0.5 - ncy);
        let (sx, sy) = (cos * dx + sin * dy + cx, -sin * dx + cos * dy + cy);
        if sx < -0.5 || sy < -0.5 || sx > w - 0.5 || sy > h - 0.5 {
            return Rgba([0, 0, 0, 0]);
        }
        // Soften the edge by fading the outermost half pixel.
        let edge = (sx + 0.5).min(sy + 0.5).min(w - 0.5 - sx).min(h - 0.5 - sy).clamp(0.0, 1.0);
        let mut p = *img.get_pixel(sx.clamp(0.0, w - 1.0) as u32, sy.clamp(0.0, h - 1.0) as u32);
        p.0[3] = (p.0[3] as f32 * edge) as u8;
        p
    })
}

fn draw_text(img: &mut RgbaImage, font: &FontRef, text: &str, x: f32, top: f32, size: f32, color: [u8; 3]) {
    let scaled = font.as_scaled(PxScale::from(size));
    let mut caret = x;
    let baseline = top + scaled.ascent();
    let mut previous = None;
    for ch in text.chars() {
        let id = scaled.glyph_id(ch);
        if let Some(prev) = previous {
            caret += scaled.kern(prev, id);
        }
        let glyph = id.with_scale_and_position(size, ab_glyph::point(caret, baseline));
        caret += scaled.h_advance(id);
        previous = Some(id);
        let Some(outline) = font.outline_glyph(glyph) else { continue };
        let bounds = outline.px_bounds();
        outline.draw(|gx, gy, coverage| {
            let (px, py) = (bounds.min.x as i64 + gx as i64, bounds.min.y as i64 + gy as i64);
            if px < 0 || py < 0 || px >= img.width() as i64 || py >= img.height() as i64 {
                return;
            }
            let p = img.get_pixel_mut(px as u32, py as u32);
            for (channel, target) in p.0.iter_mut().zip(color) {
                *channel = (*channel as f32 + (target as f32 - *channel as f32) * coverage) as u8;
            }
        });
    }
}

fn text_width(font: &FontRef, text: &str, size: f32) -> f32 {
    let scaled = font.as_scaled(PxScale::from(size));
    text.chars().map(|c| scaled.h_advance(scaled.glyph_id(c))).sum()
}

fn ellipsize(font: &FontRef, text: &str, size: f32, max: f32) -> String {
    if text_width(font, text, size) <= max {
        return text.to_owned();
    }
    let mut out: String = text.to_owned();
    while !out.is_empty() && text_width(font, &format!("{out}…"), size) > max {
        out.pop();
    }
    format!("{out}…")
}

fn rgb(color: u32) -> [u8; 4] {
    [(color >> 16) as u8, (color >> 8) as u8, color as u8, 255]
}

/// Deterministic shuffling for "Shuffle Pictures".
pub fn shuffled<T: Clone>(items: &[T], seed: u64) -> Vec<T> {
    let mut rng = Rng(seed.wrapping_mul(0xbf58_476d_1ce4_e5b9) | 1);
    let mut out = items.to_vec();
    for i in (1..out.len()).rev() {
        out.swap(i, rng.below(i + 1));
    }
    out
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs(items: &[Item]) -> Vec<&Item> {
        items.iter().collect()
    }

    fn items(n: usize) -> Vec<Item> {
        (0..n)
            .map(|i| {
                let (w, h) = if i % 2 == 0 { (300, 200) } else { (200, 300) };
                let color = [(i * 60 % 256) as u8, 120, (255 - i * 40 % 256) as u8, 255];
                Item {
                    image: DynamicImage::ImageRgba8(RgbaImage::from_pixel(w, h, Rgba(color))),
                    name: format!("photo{i}.jpg"),
                    caption: None,
                }
            })
            .collect()
    }

    #[test]
    fn every_layout_renders_the_page_size() {
        let items = items(5);
        for (layout, name) in Layout::ALL {
            let options =
                CollageOptions { layout, width: 600, aspect: 1.5, title: "Trip".into(), ..Default::default() };
            let page = render(&refs(&items), &options);
            assert_eq!(page.dimensions(), (600, 400), "{name}");
            // Something besides the background got drawn.
            let white = page.pixels().filter(|p| p.0 == [255, 255, 255, 255]).count();
            assert!(white < 600 * 400 * 9 / 10, "{name} drew almost nothing");
        }
    }

    #[test]
    fn mosaic_fills_the_page_edge_to_edge() {
        let options = CollageOptions { layout: Layout::Mosaic, width: 600, spacing: 0.0, ..Default::default() };
        let page = render(&refs(&items(7)), &options);
        assert!(page.pixels().all(|p| p.0 != [255, 255, 255, 255]), "no background shows through");
    }

    #[test]
    fn grid_keeps_gaps_with_background() {
        let options = CollageOptions {
            layout: Layout::Grid,
            width: 600,
            spacing: 0.05,
            background: Background::Color(0x000000),
            border: Border::None,
            shadows: false,
            ..Default::default()
        };
        let page = render(&refs(&items(4)), &options);
        assert_eq!(page.get_pixel(5, 5).0, [0, 0, 0, 255], "the margin is background");
        assert_ne!(page.get_pixel(150, 100).0, [0, 0, 0, 255], "a cell is a photo");
    }

    #[test]
    fn shuffle_is_a_permutation() {
        let mut order = shuffled(&[0, 1, 2, 3, 4], 7);
        assert_ne!(order, [0, 1, 2, 3, 4]);
        order.sort_unstable();
        assert_eq!(order, [0, 1, 2, 3, 4]);
    }

    #[test]
    fn contact_sheet_prints_text() {
        let options = CollageOptions { layout: Layout::ContactSheet, width: 800, ..Default::default() };
        let page = render(&refs(&items(3)), &options);
        // The header text darkens some pixels in the top band.
        let dark = (0..page.width())
            .flat_map(|x| (0..40).map(move |y| (x, y)))
            .filter(|&(x, y)| page.get_pixel(x, y).0[0] < 100)
            .count();
        assert!(dark > 50, "header drawn: {dark}");
    }
}
