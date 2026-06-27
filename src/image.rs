//! An RGBA bitmap that can be read from and written to PNG files.

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Cursor, Seek};
use std::path::Path;

use crate::error::{Error, Result};

/// An image with 8 bits per channel, stored row by row as RGBA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Image {
    /// Creates an image filled with one colour.
    pub fn new(width: u32, height: u32, rgba: [u8; 4]) -> Self {
        Self {
            width,
            height,
            pixels: rgba.repeat(width as usize * height as usize),
        }
    }

    /// The width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The pixel at (`x`, `y`), or `None` outside the image.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let start = (y as usize * self.width as usize + x as usize) * 4;
        self.pixels[start..start + 4].try_into().ok()
    }

    /// Fills the pixels with `x0 <= x < x1` and `y0 <= y < y1`, clipped to the image.
    pub fn fill(&mut self, x0: u32, y0: u32, x1: u32, y1: u32, rgba: [u8; 4]) {
        let (x0, x1) = (x0.min(self.width) as usize, x1.min(self.width) as usize);
        if x0 >= x1 {
            return;
        }
        for y in y0.min(self.height) as usize..y1.min(self.height) as usize {
            let row = y * self.width as usize * 4;
            for pixel in self.pixels[row + x0 * 4..row + x1 * 4].chunks_exact_mut(4) {
                pixel.copy_from_slice(&rgba);
            }
        }
    }

    /// Writes the image as a PNG file.
    pub fn save(&self, path: &Path) -> Result<()> {
        let file = BufWriter::new(File::create(path)?);
        let mut encoder = png::Encoder::new(file, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(image_error(path))?;
        writer
            .write_image_data(&self.pixels)
            .map_err(image_error(path))?;
        writer.finish().map_err(image_error(path))
    }

    /// Reads a PNG file, converting it to 8-bit RGBA.
    pub fn load(path: &Path) -> Result<Self> {
        Self::read(
            BufReader::new(File::open(path)?),
            &path.display().to_string(),
        )
    }

    /// Decodes PNG data held in memory. Other image formats are not supported yet.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Self::read(Cursor::new(bytes), "image data")
    }

    fn read(source: impl BufRead + Seek, name: &str) -> Result<Self> {
        let failed = |error: &dyn std::fmt::Display| Error::Image(format!("{name}: {error}"));
        let mut decoder = png::Decoder::new(source);
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info().map_err(|e| failed(&e))?;
        let size = reader
            .output_buffer_size()
            .ok_or_else(|| failed(&"image is too large"))?;
        let mut buffer = vec![0; size];
        let info = reader.next_frame(&mut buffer).map_err(|e| failed(&e))?;
        buffer.truncate(info.buffer_size());
        let pixels = match info.color_type {
            png::ColorType::Rgba => buffer,
            png::ColorType::Rgb => buffer
                .chunks_exact(3)
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
            png::ColorType::Grayscale => buffer.iter().flat_map(|&g| [g, g, g, 255]).collect(),
            png::ColorType::GrayscaleAlpha => buffer
                .chunks_exact(2)
                .flat_map(|p| [p[0], p[0], p[0], p[1]])
                .collect(),
            png::ColorType::Indexed => return Err(failed(&"palette was not expanded")),
        };
        Ok(Self {
            width: info.width,
            height: info.height,
            pixels,
        })
    }

    /// The raw pixels, row by row, four bytes each in RGBA order.
    pub fn data(&self) -> &[u8] {
        &self.pixels
    }

    /// Draws one pixel over what is already there. `coverage` scales the colour's own alpha.
    ///
    /// Coordinates outside the image are ignored. The image itself stays opaque.
    pub fn blend(&mut self, x: i64, y: i64, rgba: [u8; 4], coverage: u8) {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return;
        }
        let alpha = u32::from(rgba[3]) * u32::from(coverage);
        if alpha == 0 {
            return;
        }
        let start = (y as usize * self.width as usize + x as usize) * 4;
        for (target, &source) in self.pixels[start..start + 3].iter_mut().zip(&rgba[..3]) {
            let (source, current) = (u32::from(source), u32::from(*target));
            let mixed =
                (source * alpha + current * (255 * 255 - alpha) + 255 * 255 / 2) / (255 * 255);
            *target = mixed as u8;
        }
        self.pixels[start + 3] = 255;
    }
}

fn image_error<E: std::fmt::Display>(path: &Path) -> impl Fn(E) -> Error {
    move |error| Error::Image(format!("{}: {error}", path.display()))
}
