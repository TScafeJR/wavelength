//! TrueType fonts: parsing, text measurement and glyph rasterisation.
//!
//! Only fonts with `glyf` outlines are supported, which covers the `.ttf`
//! files shipped with macOS and most Linux distributions. There is no shaping
//! or kerning yet: each character maps to one glyph and advances by its width.
//! One sans-serif family, in regular and bold, stands in for every font a
//! page asks for.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// Set this environment variable to a `.ttf` file to use it for regular text.
pub const FONT_ENV: &str = "WAVELENGTH_FONT";
/// Set this environment variable to a `.ttf` file to use it for bold text.
pub const BOLD_FONT_ENV: &str = "WAVELENGTH_FONT_BOLD";

const REGULAR_CANDIDATES: &[&str] = &[
    "/System/Library/Fonts/Supplemental/Arial.ttf",
    "/Library/Fonts/Arial.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/liberation-sans/LiberationSans-Regular.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
];
const BOLD_CANDIDATES: &[&str] = &[
    "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
    "/Library/Fonts/Arial Bold.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
    "/usr/share/fonts/liberation-sans/LiberationSans-Bold.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans-Bold.ttf",
];

/// Nested composite glyphs deeper than this are treated as empty.
const MAX_COMPOSITE_DEPTH: u32 = 8;

/// A point of a glyph outline, in font units with the y axis pointing up.
#[derive(Debug, Clone, Copy, PartialEq)]
struct OutlinePoint {
    x: f32,
    y: f32,
    on_curve: bool,
}

/// A parsed TrueType font.
#[derive(Debug)]
pub struct Font {
    data: Vec<u8>,
    glyf: usize,
    loca: usize,
    hmtx: usize,
    cmap: Cmap,
    units_per_em: f32,
    long_loca: bool,
    glyph_count: u32,
    advance_count: u32,
    ascent: f32,
    descent: f32,
    line_gap: f32,
}

#[derive(Debug, Clone, Copy)]
enum Cmap {
    /// Format 4: segments covering the Basic Multilingual Plane.
    Segments(usize),
    /// Format 12: groups covering all of Unicode.
    Groups(usize),
}

/// Vertical metrics of a font at one size, in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// Distance from the baseline to the top of the tallest glyphs.
    pub ascent: f32,
    /// Distance from the baseline to the bottom of the lowest glyphs, as a positive number.
    pub descent: f32,
    /// Extra space the font asks for between lines.
    pub line_gap: f32,
}

/// A rasterised glyph: one coverage byte per pixel.
#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    /// Bitmap width in pixels.
    pub width: usize,
    /// Bitmap height in pixels.
    pub height: usize,
    /// Where the bitmap's left edge sits relative to the pen position.
    pub left: i32,
    /// Where the bitmap's top edge sits relative to the baseline; negative is above it.
    pub top: i32,
    /// Coverage values, row by row, from 0 (empty) to 255 (fully covered).
    pub coverage: Vec<u8>,
}

impl Font {
    /// Parses a TrueType file or the first font of a TrueType collection.
    ///
    /// Returns `None` when the data is not a font this engine can use.
    pub fn parse(data: Vec<u8>) -> Option<Self> {
        let base = if data.get(..4)? == b"ttcf" {
            be_u32(&data, 12)? as usize
        } else {
            0
        };
        let table_count = be_u16(&data, base + 4)? as usize;
        let table = |tag: &[u8; 4]| {
            (0..table_count)
                .map(|index| base + 12 + index * 16)
                .find(|&record| data.get(record..record + 4) == Some(tag))
                .and_then(|record| be_u32(&data, record + 8))
                .map(|offset| offset as usize)
        };
        let (head, maxp, hhea) = (table(b"head")?, table(b"maxp")?, table(b"hhea")?);
        let (glyf, loca, hmtx) = (table(b"glyf")?, table(b"loca")?, table(b"hmtx")?);
        let cmap = find_cmap(&data, table(b"cmap")?)?;

        let units_per_em = f32::from(be_u16(&data, head + 18)?);
        if units_per_em == 0.0 {
            return None;
        }
        Some(Self {
            glyf,
            loca,
            hmtx,
            cmap,
            units_per_em,
            long_loca: be_i16(&data, head + 50)? != 0,
            glyph_count: u32::from(be_u16(&data, maxp + 4)?),
            advance_count: u32::from(be_u16(&data, hhea + 34)?),
            ascent: f32::from(be_i16(&data, hhea + 4)?),
            descent: -f32::from(be_i16(&data, hhea + 6)?),
            line_gap: f32::from(be_i16(&data, hhea + 8)?),
            data,
        })
    }

    /// The font's vertical metrics at `size` pixels.
    pub fn metrics(&self, size: f32) -> Metrics {
        let scale = size / self.units_per_em;
        Metrics {
            ascent: self.ascent * scale,
            descent: self.descent * scale,
            line_gap: self.line_gap * scale,
        }
    }

    /// The glyph used for a character, or glyph 0 when the font has none.
    pub fn glyph_id(&self, character: char) -> u32 {
        let code = character as u32;
        let id = match self.cmap {
            Cmap::Segments(table) => self.segment_lookup(table, code),
            Cmap::Groups(table) => self.group_lookup(table, code),
        };
        id.filter(|&id| id < self.glyph_count).unwrap_or(0)
    }

    fn segment_lookup(&self, table: usize, code: u32) -> Option<u32> {
        if code > 0xFFFF {
            return None;
        }
        let doubled = be_u16(&self.data, table + 6)? as usize;
        let (ends, starts) = (table + 14, table + 16 + doubled);
        let (deltas, offsets) = (starts + doubled, starts + doubled * 2);
        for segment in (0..doubled).step_by(2) {
            if u32::from(be_u16(&self.data, ends + segment)?) < code {
                continue;
            }
            let start = u32::from(be_u16(&self.data, starts + segment)?);
            if start > code {
                return None;
            }
            let delta = u32::from(be_u16(&self.data, deltas + segment)?);
            let range_offset = be_u16(&self.data, offsets + segment)? as usize;
            if range_offset == 0 {
                return Some((code + delta) & 0xFFFF);
            }
            let address = offsets + segment + range_offset + 2 * (code - start) as usize;
            let glyph = u32::from(be_u16(&self.data, address)?);
            return (glyph != 0).then_some((glyph + delta) & 0xFFFF);
        }
        None
    }

    fn group_lookup(&self, table: usize, code: u32) -> Option<u32> {
        let count = be_u32(&self.data, table + 12)? as usize;
        let (mut low, mut high) = (0, count);
        while low < high {
            let middle = (low + high) / 2;
            let group = table + 16 + middle * 12;
            let (start, end) = (be_u32(&self.data, group)?, be_u32(&self.data, group + 4)?);
            if code < start {
                high = middle;
            } else if code > end {
                low = middle + 1;
            } else {
                return Some(be_u32(&self.data, group + 8)? + (code - start));
            }
        }
        None
    }

    /// How far the pen moves after a glyph, in pixels at `size`.
    pub fn advance(&self, glyph: u32, size: f32) -> f32 {
        let index = glyph.min(self.advance_count.saturating_sub(1)) as usize;
        let units = be_u16(&self.data, self.hmtx + index * 4).unwrap_or(0);
        f32::from(units) * size / self.units_per_em
    }

    /// The width of a run of text in pixels at `size`.
    pub fn measure(&self, text: &str, size: f32) -> f32 {
        text.chars()
            .map(|character| self.advance(self.glyph_id(character), size))
            .sum()
    }

    /// The byte range of a glyph's outline in the `glyf` table, or `None` if it has no outline.
    fn glyph_range(&self, glyph: u32) -> Option<(usize, usize)> {
        let glyph = glyph as usize;
        let offset = |index: usize| {
            if self.long_loca {
                be_u32(&self.data, self.loca + index * 4).map(|o| o as usize)
            } else {
                be_u16(&self.data, self.loca + index * 2).map(|o| o as usize * 2)
            }
        };
        let (start, end) = (offset(glyph)?, offset(glyph + 1)?);
        (end > start).then_some((self.glyf + start, self.glyf + end))
    }

    /// Reads a glyph's contours in font units.
    fn outline(&self, glyph: u32, depth: u32) -> Vec<Vec<OutlinePoint>> {
        self.read_outline(glyph, depth).unwrap_or_default()
    }

    fn read_outline(&self, glyph: u32, depth: u32) -> Option<Vec<Vec<OutlinePoint>>> {
        let (start, _) = self.glyph_range(glyph)?;
        let contour_count = be_i16(&self.data, start)?;
        if contour_count < 0 {
            return (depth < MAX_COMPOSITE_DEPTH).then(|| self.composite(start + 10, depth))?;
        }
        let contour_count = contour_count as usize;
        let mut cursor = start + 10;
        let mut ends = Vec::with_capacity(contour_count);
        for _ in 0..contour_count {
            ends.push(be_u16(&self.data, cursor)? as usize);
            cursor += 2;
        }
        let point_count = ends.last().map_or(0, |last| last + 1);
        cursor += 2 + be_u16(&self.data, cursor)? as usize; // Skip the hinting instructions.

        let mut flags = Vec::with_capacity(point_count);
        while flags.len() < point_count {
            let flag = *self.data.get(cursor)?;
            cursor += 1;
            let repeats = if flag & 8 != 0 {
                cursor += 1;
                *self.data.get(cursor - 1)? as usize
            } else {
                0
            };
            flags.extend(std::iter::repeat_n(flag, repeats + 1));
        }
        flags.truncate(point_count);

        // Coordinates are stored as deltas: first every x, then every y.
        let mut read_axis = |short_bit: u8, same_bit: u8| {
            let mut value = 0i32;
            let mut values = Vec::with_capacity(point_count);
            for &flag in &flags {
                if flag & short_bit != 0 {
                    let magnitude = i32::from(*self.data.get(cursor)?);
                    cursor += 1;
                    value += if flag & same_bit != 0 {
                        magnitude
                    } else {
                        -magnitude
                    };
                } else if flag & same_bit == 0 {
                    value += i32::from(be_i16(&self.data, cursor)?);
                    cursor += 2;
                }
                values.push(value as f32);
            }
            Some(values)
        };
        let xs = read_axis(2, 16)?;
        let ys = read_axis(4, 32)?;

        let mut contours = Vec::with_capacity(contour_count);
        let mut first = 0;
        for end in ends {
            if end < first || end >= point_count {
                return None;
            }
            contours.push(
                (first..=end)
                    .map(|i| OutlinePoint {
                        x: xs[i],
                        y: ys[i],
                        on_curve: flags[i] & 1 != 0,
                    })
                    .collect(),
            );
            first = end + 1;
        }
        Some(contours)
    }

    /// Reads a composite glyph, which is assembled from other glyphs.
    fn composite(&self, mut cursor: usize, depth: u32) -> Option<Vec<Vec<OutlinePoint>>> {
        let mut contours = Vec::new();
        loop {
            let flags = be_u16(&self.data, cursor)?;
            let component = u32::from(be_u16(&self.data, cursor + 2)?);
            cursor += 4;
            let (dx, dy) = if flags & 1 != 0 {
                cursor += 4;
                (
                    f32::from(be_i16(&self.data, cursor - 4)?),
                    f32::from(be_i16(&self.data, cursor - 2)?),
                )
            } else {
                cursor += 2;
                (
                    f32::from(*self.data.get(cursor - 2)? as i8),
                    f32::from(*self.data.get(cursor - 1)? as i8),
                )
            };
            // Arguments that are point numbers rather than offsets are not supported.
            let (dx, dy) = if flags & 2 != 0 { (dx, dy) } else { (0.0, 0.0) };

            let fixed = |cursor: &mut usize| {
                *cursor += 2;
                be_i16(&self.data, *cursor - 2).map(|v| f32::from(v) / 16384.0)
            };
            let (a, b, c, d) = if flags & 0x08 != 0 {
                let scale = fixed(&mut cursor)?;
                (scale, 0.0, 0.0, scale)
            } else if flags & 0x40 != 0 {
                (fixed(&mut cursor)?, 0.0, 0.0, fixed(&mut cursor)?)
            } else if flags & 0x80 != 0 {
                (
                    fixed(&mut cursor)?,
                    fixed(&mut cursor)?,
                    fixed(&mut cursor)?,
                    fixed(&mut cursor)?,
                )
            } else {
                (1.0, 0.0, 0.0, 1.0)
            };

            for mut contour in self.outline(component, depth + 1) {
                for point in &mut contour {
                    let (x, y) = (point.x, point.y);
                    point.x = a * x + c * y + dx;
                    point.y = b * x + d * y + dy;
                }
                contours.push(contour);
            }
            if flags & 0x20 == 0 {
                return Some(contours);
            }
        }
    }

    /// Rasterises a glyph at `size` pixels. Glyphs without an outline, such as the space, are empty.
    pub fn rasterize(&self, glyph: u32, size: f32) -> Glyph {
        let scale = size / self.units_per_em;
        let contours = self.outline(glyph, 0);
        let points = || contours.iter().flatten();
        let bound = |pick: fn(&OutlinePoint) -> f32, fold: fn(f32, f32) -> f32, start: f32| {
            points().map(pick).fold(start, fold)
        };
        let min_x = bound(|p| p.x, f32::min, f32::INFINITY) * scale;
        let max_x = bound(|p| p.x, f32::max, f32::NEG_INFINITY) * scale;
        let min_y = bound(|p| p.y, f32::min, f32::INFINITY) * scale;
        let max_y = bound(|p| p.y, f32::max, f32::NEG_INFINITY) * scale;
        if !(min_x.is_finite() && max_x.is_finite() && min_y.is_finite() && max_y.is_finite()) {
            return Glyph {
                width: 0,
                height: 0,
                left: 0,
                top: 0,
                coverage: Vec::new(),
            };
        }

        // One pixel of margin on every side keeps the rasteriser's writes inside the bitmap.
        let left = min_x.floor() as i32 - 1;
        let top = (-max_y).floor() as i32 - 1;
        let width = ((max_x.ceil() as i32 - left) + 2).clamp(1, 4096) as usize;
        let height = (((-min_y).ceil() as i32 - top) + 2).clamp(1, 4096) as usize;
        let mut raster = Rasterizer::new(width, height);
        let to_pixels =
            |point: &OutlinePoint| (point.x * scale - left as f32, -point.y * scale - top as f32);
        for contour in &contours {
            raster.contour(contour, to_pixels);
        }
        Glyph {
            width,
            height,
            left,
            top,
            coverage: raster.finish(),
        }
    }
}

fn find_cmap(data: &[u8], cmap: usize) -> Option<Cmap> {
    let count = be_u16(data, cmap + 2)? as usize;
    let mut segments = None;
    for index in 0..count {
        let subtable = cmap + be_u32(data, cmap + 4 + index * 8 + 4)? as usize;
        match be_u16(data, subtable)? {
            12 => return Some(Cmap::Groups(subtable)),
            4 => segments = Some(Cmap::Segments(subtable)),
            _ => {}
        }
    }
    segments
}

fn be_u16(data: &[u8], offset: usize) -> Option<u16> {
    let bytes = data.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn be_i16(data: &[u8], offset: usize) -> Option<i16> {
    be_u16(data, offset).map(|value| value as i16)
}

fn be_u32(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = data.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Turns outlines into coverage by accumulating the signed area each edge sweeps out.
struct Rasterizer {
    width: usize,
    height: usize,
    area: Vec<f32>,
}

impl Rasterizer {
    fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            area: vec![0.0; width * height + 4],
        }
    }

    fn add(&mut self, index: usize, value: f32) {
        if let Some(cell) = self.area.get_mut(index) {
            *cell += value;
        }
    }

    /// Adds one closed contour, expanding the implied points between consecutive curve controls.
    fn contour(
        &mut self,
        points: &[OutlinePoint],
        to_pixels: impl Fn(&OutlinePoint) -> (f32, f32),
    ) {
        let Some((first, last)) = points.first().zip(points.last()) else {
            return;
        };
        let middle = |a: (f32, f32), b: (f32, f32)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
        // Start from a point that is on the curve, inventing one if the contour has none.
        let (start, range) = if first.on_curve {
            (to_pixels(first), 1..points.len())
        } else if last.on_curve {
            (to_pixels(last), 0..points.len() - 1)
        } else {
            (middle(to_pixels(first), to_pixels(last)), 0..points.len())
        };

        let mut current = start;
        let mut control: Option<(f32, f32)> = None;
        for point in &points[range] {
            let position = to_pixels(point);
            match (point.on_curve, control) {
                (true, Some(c)) => {
                    self.curve(current, c, position);
                    current = position;
                    control = None;
                }
                (true, None) => {
                    self.line(current, position);
                    current = position;
                }
                (false, Some(c)) => {
                    let implied = middle(c, position);
                    self.curve(current, c, implied);
                    current = implied;
                    control = Some(position);
                }
                (false, None) => control = Some(position),
            }
        }
        match control {
            Some(c) => self.curve(current, c, start),
            None => self.line(current, start),
        }
    }

    /// Flattens a quadratic Bézier curve into line segments.
    fn curve(&mut self, from: (f32, f32), control: (f32, f32), to: (f32, f32)) {
        let deviation_x = from.0 - 2.0 * control.0 + to.0;
        let deviation_y = from.1 - 2.0 * control.1 + to.1;
        let deviation = deviation_x * deviation_x + deviation_y * deviation_y;
        let steps = (1.0 + (3.0 * deviation).sqrt().sqrt().floor()).min(32.0) as usize;
        let mut previous = from;
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            let u = 1.0 - t;
            let next = (
                u * u * from.0 + 2.0 * u * t * control.0 + t * t * to.0,
                u * u * from.1 + 2.0 * u * t * control.1 + t * t * to.1,
            );
            self.line(previous, next);
            previous = next;
        }
    }

    /// Accumulates the area to the right of one edge, row by row.
    fn line(&mut self, from: (f32, f32), to: (f32, f32)) {
        if from.1 == to.1 {
            return;
        }
        let (direction, top, bottom) = if from.1 < to.1 {
            (1.0, from, to)
        } else {
            (-1.0, to, from)
        };
        let slope = (bottom.0 - top.0) / (bottom.1 - top.1);
        let mut x = top.0;
        if top.1 < 0.0 {
            x -= top.1 * slope;
        }
        let first_row = top.1.max(0.0) as usize;
        let last_row = (bottom.1.ceil().max(0.0) as usize).min(self.height);
        for row in first_row..last_row {
            let row_start = row * self.width;
            let dy = ((row + 1) as f32).min(bottom.1) - (row as f32).max(top.1);
            let next_x = x + slope * dy;
            let d = dy * direction;
            let (x0, x1) = if x < next_x { (x, next_x) } else { (next_x, x) };
            let x0_floor = x0.floor();
            let x0_cell = x0_floor.max(0.0) as usize;
            let x1_ceil = x1.ceil();
            let x1_cell = x1_ceil.max(0.0) as usize;
            if x1_cell <= x0_cell + 1 {
                let middle = 0.5 * (x + next_x) - x0_floor;
                self.add(row_start + x0_cell, d - d * middle);
                self.add(row_start + x0_cell + 1, d * middle);
            } else {
                let inverse = (x1 - x0).recip();
                let x0_fraction = x0 - x0_floor;
                let a0 = 0.5 * inverse * (1.0 - x0_fraction) * (1.0 - x0_fraction);
                let x1_fraction = x1 - x1_ceil + 1.0;
                let last = 0.5 * inverse * x1_fraction * x1_fraction;
                self.add(row_start + x0_cell, d * a0);
                if x1_cell == x0_cell + 2 {
                    self.add(row_start + x0_cell + 1, d * (1.0 - a0 - last));
                } else {
                    let a1 = inverse * (1.5 - x0_fraction);
                    self.add(row_start + x0_cell + 1, d * (a1 - a0));
                    for cell in x0_cell + 2..x1_cell - 1 {
                        self.add(row_start + cell, d * inverse);
                    }
                    let a2 = a1 + (x1_cell - x0_cell - 3) as f32 * inverse;
                    self.add(row_start + x1_cell - 1, d * (1.0 - a2 - last));
                }
                self.add(row_start + x1_cell, d * last);
            }
            x = next_x;
        }
    }

    /// Integrates the accumulated areas along each row into coverage values.
    fn finish(self) -> Vec<u8> {
        let mut coverage = Vec::with_capacity(self.width * self.height);
        for row in self.area.chunks_exact(self.width).take(self.height) {
            let mut sum = 0.0;
            for cell in row {
                sum += cell;
                coverage.push((sum.abs().min(1.0) * 255.0).round() as u8);
            }
        }
        coverage
    }
}

/// The regular and bold faces used for all text, with a cache of rasterised glyphs.
#[derive(Debug, Default)]
pub struct Fonts {
    regular: Option<Font>,
    bold: Option<Font>,
    cache: Mutex<HashMap<(bool, u32, u32), Arc<Glyph>>>,
}

impl Fonts {
    /// The fonts found on this machine, loaded on first use.
    pub fn system() -> &'static Fonts {
        static FONTS: OnceLock<Fonts> = OnceLock::new();
        FONTS.get_or_init(|| {
            let load = |variable: &str, candidates: &[&str]| {
                let overridden = std::env::var(variable).ok();
                overridden
                    .iter()
                    .map(String::as_str)
                    .chain(candidates.iter().copied())
                    .find_map(|path| Font::parse(std::fs::read(path).ok()?))
            };
            Fonts {
                regular: load(FONT_ENV, REGULAR_CANDIDATES),
                bold: load(BOLD_FONT_ENV, BOLD_CANDIDATES),
                cache: Mutex::default(),
            }
        })
    }

    /// Whether any font was found. Without one, text takes no space and is not drawn.
    pub fn available(&self) -> bool {
        self.regular.is_some()
    }

    fn face(&self, bold: bool) -> Option<&Font> {
        if bold {
            self.bold.as_ref().or(self.regular.as_ref())
        } else {
            self.regular.as_ref()
        }
    }

    /// The vertical metrics of the chosen face at `size` pixels.
    pub fn metrics(&self, size: f32, bold: bool) -> Metrics {
        self.face(bold).map_or(
            Metrics {
                ascent: 0.0,
                descent: 0.0,
                line_gap: 0.0,
            },
            |font| font.metrics(size),
        )
    }

    /// The width of `text` in pixels.
    pub fn measure(&self, text: &str, size: f32, bold: bool) -> f32 {
        self.face(bold).map_or(0.0, |font| font.measure(text, size))
    }

    /// Calls `draw` with each glyph of `text` and the pen position it belongs at.
    pub fn glyphs(&self, text: &str, size: f32, bold: bool, mut draw: impl FnMut(f32, &Glyph)) {
        let Some(font) = self.face(bold) else {
            return;
        };
        // Sizes are cached in quarter pixels, which is finer than the eye can tell apart.
        let size_key = (size * 4.0).round() as u32;
        let mut pen = 0.0;
        for character in text.chars() {
            let id = font.glyph_id(character);
            let cached = self
                .cache
                .lock()
                .ok()
                .and_then(|cache| cache.get(&(bold, id, size_key)).cloned());
            let glyph = cached.unwrap_or_else(|| {
                let glyph = Arc::new(font.rasterize(id, size));
                if let Ok(mut cache) = self.cache.lock() {
                    cache.insert((bold, id, size_key), Arc::clone(&glyph));
                }
                glyph
            });
            draw(pen, &glyph);
            pen += font.advance(id, size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(x: f32, y: f32) -> OutlinePoint {
        OutlinePoint {
            x,
            y,
            on_curve: true,
        }
    }

    #[test]
    fn a_square_contour_fills_exactly_its_pixels() {
        let mut raster = Rasterizer::new(6, 6);
        let square = [on(1.0, 1.0), on(4.0, 1.0), on(4.0, 4.0), on(1.0, 4.0)];
        raster.contour(&square, |p| (p.x, p.y));
        let coverage = raster.finish();
        for y in 0..6 {
            for x in 0..6 {
                let inside = (1..4).contains(&x) && (1..4).contains(&y);
                assert_eq!(
                    coverage[y * 6 + x],
                    if inside { 255 } else { 0 },
                    "({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn edges_between_pixels_give_partial_coverage() {
        let mut raster = Rasterizer::new(4, 3);
        let half = [on(1.5, 1.0), on(3.0, 1.0), on(3.0, 2.0), on(1.5, 2.0)];
        raster.contour(&half, |p| (p.x, p.y));
        let coverage = raster.finish();
        assert_eq!(&coverage[4..8], [0, 128, 255, 0]);
    }

    #[test]
    fn curves_enclose_roughly_the_expected_area() {
        // A circle of radius 10 drawn with four off-curve controls and implied on-curve points.
        let off = |x, y| OutlinePoint {
            x,
            y,
            on_curve: false,
        };
        let circle = [
            off(2.0, 2.0),
            off(22.0, 2.0),
            off(22.0, 22.0),
            off(2.0, 22.0),
        ];
        let mut raster = Rasterizer::new(24, 24);
        raster.contour(&circle, |p| (p.x, p.y));
        let area: f32 = raster.finish().iter().map(|&c| f32::from(c) / 255.0).sum();
        // Quadratic curves through these controls enclose a shape a little larger than a circle.
        assert!((300.0..345.0).contains(&area), "{area}");
    }

    #[test]
    fn system_fonts_measure_and_rasterise_text_when_present() {
        let fonts = Fonts::system();
        if !fonts.available() {
            return; // No usable font on this machine.
        }
        let narrow = fonts.measure("i", 16.0, false);
        let wide = fonts.measure("W", 16.0, false);
        assert!(narrow > 0.0 && wide > narrow * 2.0);
        assert_eq!(fonts.measure("iW", 16.0, false), narrow + wide);
        assert!(fonts.metrics(16.0, false).ascent > 10.0);

        let mut inked = 0;
        fonts.glyphs("W ", 16.0, false, |_, glyph| {
            inked += glyph.coverage.iter().filter(|&&c| c > 128).count();
        });
        assert!(inked > 20, "{inked}");
    }

    #[test]
    fn garbage_is_not_a_font() {
        assert!(Font::parse(Vec::new()).is_none());
        assert!(Font::parse(vec![0; 64]).is_none());
        assert!(Font::parse(b"ttcf".to_vec()).is_none());
    }
}
