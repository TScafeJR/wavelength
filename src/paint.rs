//! Painting: turns a layout tree into pixels.

use std::collections::HashMap;

use crate::dom::{Document, NodeId};
use crate::font::{Face, Fonts};
use crate::image::Image;
use crate::layout::{Fragment, LayoutBox, Rect};
use crate::style::Color;

/// Everything the painter needs to draw a page.
#[derive(Debug, Clone, Copy)]
pub struct Scene<'a> {
    /// The document the layout tree was built from.
    pub doc: &'a Document,
    /// The root of the layout tree, if the page generates any boxes.
    pub root: Option<&'a LayoutBox>,
    /// The fonts used to draw text.
    pub fonts: &'a Fonts,
    /// Decoded pictures, by `img` element.
    pub images: &'a HashMap<NodeId, Image>,
}

/// Paints the part of the page that starts `scroll_y` pixels down, onto a canvas of the given size.
///
/// The canvas takes the root element's background, or the body's when the
/// root has none, or white. The element that lends its background to the
/// canvas does not paint it a second time on its own box.
pub fn paint(scene: Scene<'_>, width: u32, height: u32, scroll_y: f32) -> Image {
    let Some(root) = scene.root else {
        return Image::new(width, height, rgba(Color::WHITE));
    };
    let body = root.children.iter().find(|child| {
        scene
            .doc
            .element(child.node)
            .is_some_and(|e| e.name == "body")
    });
    let canvas_source = [Some(root), body]
        .into_iter()
        .flatten()
        .find(|candidate| !candidate.style.background.is_transparent());

    let canvas_color = canvas_source.map_or(Color::WHITE, |source| source.style.background);
    let mut painter = Painter {
        image: Image::new(width, height, [255, 255, 255, 255]),
        scene,
        offset_y: -scroll_y,
        skip_background: canvas_source.map(|source| source.node),
    };
    let everything = Rect {
        x: 0.0,
        y: scroll_y,
        width: width as f32,
        height: height as f32,
    };
    painter.fill(everything, canvas_color);
    painter.paint_box(root);
    painter.image
}

/// How a run of text is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    /// The font size in pixels.
    pub size: f32,
    /// The font face used.
    pub face: Face,
    /// The text colour.
    pub color: Color,
}

/// Draws a run of text with its baseline at `baseline`, starting at `x`.
pub fn draw_text(
    image: &mut Image,
    fonts: &Fonts,
    x: f32,
    baseline: f32,
    text: &str,
    style: TextStyle,
) {
    let (color, baseline) = (rgba(style.color), baseline.round() as i64);
    fonts.glyphs(text, style.size, style.face, |pen, glyph| {
        let left = (x + pen).round() as i64 + i64::from(glyph.left);
        let top = baseline + i64::from(glyph.top);
        for (index, &coverage) in glyph.coverage.iter().enumerate() {
            if coverage > 0 {
                let (column, row) = (index % glyph.width, index / glyph.width);
                image.blend(left + column as i64, top + row as i64, color, coverage);
            }
        }
    });
}

struct Painter<'a> {
    image: Image,
    scene: Scene<'a>,
    /// Added to every y coordinate, which is how scrolling is applied.
    offset_y: f32,
    skip_background: Option<NodeId>,
}

impl Painter<'_> {
    fn paint_box(&mut self, layout_box: &LayoutBox) {
        let (rect, style) = (layout_box.rect, &layout_box.style);
        let radius = style.used_radius(rect.width, rect.height);
        if self.skip_background != Some(layout_box.node) {
            self.fill_rounded(rect, radius, 0.0, style.fade(style.background));
        }

        let (border, colors) = (style.border(), style.used_border_color());
        let inner_height = rect.height - border.top - border.bottom;
        let widths = [border.top, border.right, border.bottom, border.left];
        let uniform = widths.iter().all(|width| *width == border.top)
            && [colors.right, colors.bottom, colors.left]
                .iter()
                .all(|color| *color == colors.top);
        if border.top <= 0.0 && uniform {
            // No border to draw.
        } else if radius > 0.0 && uniform {
            // A rounded border is a ring: the rounded box with a smaller rounded box cut out of it.
            self.fill_rounded(rect, radius, border.top, style.fade(colors.top));
        } else {
            let edge = |x, y, width, height| Rect {
                x,
                y,
                width,
                height,
            };
            let right_x = rect.x + rect.width - border.right;
            let bottom_y = rect.y + rect.height - border.bottom;
            self.fill(
                edge(rect.x, rect.y, rect.width, border.top),
                style.fade(colors.top),
            );
            self.fill(
                edge(rect.x, bottom_y, rect.width, border.bottom),
                style.fade(colors.bottom),
            );
            self.fill(
                edge(rect.x, rect.y + border.top, border.left, inner_height),
                style.fade(colors.left),
            );
            self.fill(
                edge(right_x, rect.y + border.top, border.right, inner_height),
                style.fade(colors.right),
            );
        }

        if let Some(picture) = self
            .scene
            .images
            .get(&layout_box.node)
            .filter(|_| style.visible)
        {
            let padding = style.padding;
            let content = Rect {
                x: rect.x + border.left + padding.left,
                y: rect.y + border.top + padding.top,
                width: rect.width - border.left - border.right - padding.left - padding.right,
                height: inner_height - padding.top - padding.bottom,
            };
            self.draw_image(picture, content);
        }

        for child in &layout_box.children {
            self.paint_box(child);
        }
        for fragment in &layout_box.fragments {
            self.paint_text(fragment);
        }
    }

    /// Fills a box with rounded corners, antialiasing the curves. With a
    /// `ring` width only that much of the box's edge is filled, which draws a border.
    fn fill_rounded(&mut self, rect: Rect, radius: f32, ring: f32, color: Color) {
        if radius <= 0.0 && ring <= 0.0 {
            return self.fill(rect, color);
        }
        if color.is_transparent() {
            return;
        }
        let Some((x0, y0, x1, y1)) = self.pixel_bounds(rect) else {
            return;
        };
        // Coverage is measured against the whole box, which may extend past the canvas.
        let left = rect.x.round();
        let top = (rect.y + self.offset_y).round();
        let right = (rect.x + rect.width).round();
        let bottom = (rect.y + self.offset_y + rect.height).round();
        let coverage = |x: f32, y: f32, inset: f32, radius: f32| {
            let (left, top, right, bottom) =
                (left + inset, top + inset, right - inset, bottom - inset);
            if x < left || x > right || y < top || y > bottom {
                return 0.0;
            }
            // Only points in a corner square are affected by the curve.
            let nearest_x = x.clamp(
                (left + radius).min(right - radius),
                (right - radius).max(left + radius),
            );
            let nearest_y = y.clamp(
                (top + radius).min(bottom - radius),
                (bottom - radius).max(top + radius),
            );
            let distance = ((x - nearest_x).powi(2) + (y - nearest_y).powi(2)).sqrt();
            (radius - distance + 0.5).clamp(0.0, 1.0)
        };
        let rgba = rgba(color);
        for y in y0..y1 {
            for x in x0..x1 {
                let (center_x, center_y) = (x as f32 + 0.5, y as f32 + 0.5);
                let outer = coverage(center_x, center_y, 0.0, radius);
                let amount = if ring > 0.0 {
                    outer * (1.0 - coverage(center_x, center_y, ring, (radius - ring).max(0.0)))
                } else {
                    outer
                };
                if amount > 0.0 {
                    self.image.blend(
                        i64::from(x),
                        i64::from(y),
                        rgba,
                        (amount * 255.0).round() as u8,
                    );
                }
            }
        }
    }

    fn paint_text(&mut self, fragment: &Fragment) {
        let baseline = fragment.baseline + self.offset_y;
        let visible = -fragment.size * 2.0..self.image.height() as f32 + fragment.size * 2.0;
        if !visible.contains(&baseline) {
            return;
        }
        let style = TextStyle {
            size: fragment.size,
            face: fragment.face,
            color: fragment.color,
        };
        draw_text(
            &mut self.image,
            self.scene.fonts,
            fragment.rect.x,
            baseline,
            &fragment.text,
            style,
        );
        if fragment.underline {
            let thickness = (fragment.size / 16.0).round().max(1.0);
            let line = Rect {
                x: fragment.rect.x,
                y: fragment.baseline.round() + 1.0,
                width: fragment.rect.width,
                height: thickness,
            };
            self.fill(line, fragment.color);
        }
    }

    /// The pixel bounds of a rectangle after scrolling, with each edge snapped
    /// to the nearest pixel boundary and clipped to the canvas.
    fn pixel_bounds(&self, rect: Rect) -> Option<(u32, u32, u32, u32)> {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return None;
        }
        // Float to integer casts saturate, so edges left of or above the canvas clamp to zero.
        let snap = |edge: f32, limit: u32| (edge.round() as u32).min(limit);
        let (width, height) = (self.image.width(), self.image.height());
        let (x0, x1) = (snap(rect.x, width), snap(rect.x + rect.width, width));
        let top = rect.y + self.offset_y;
        let (y0, y1) = (snap(top, height), snap(top + rect.height, height));
        (x0 < x1 && y0 < y1).then_some((x0, y0, x1, y1))
    }

    fn fill(&mut self, rect: Rect, color: Color) {
        if color.is_transparent() {
            return;
        }
        let Some((x0, y0, x1, y1)) = self.pixel_bounds(rect) else {
            return;
        };
        if color.a == 255 {
            self.image.fill(x0, y0, x1, y1, rgba(color));
            return;
        }
        for y in y0..y1 {
            for x in x0..x1 {
                self.image
                    .blend(i64::from(x), i64::from(y), rgba(color), 255);
            }
        }
    }

    /// Draws a picture scaled to fill `target`, sampling the nearest source pixel.
    fn draw_image(&mut self, picture: &Image, target: Rect) {
        let Some((x0, y0, x1, y1)) = self.pixel_bounds(target) else {
            return;
        };
        let left = target.x.round();
        let top = (target.y + self.offset_y).round();
        let scale_x = picture.width() as f32 / target.width;
        let scale_y = picture.height() as f32 / target.height;
        for y in y0..y1 {
            let source_y = ((y as f32 - top + 0.5) * scale_y) as u32;
            for x in x0..x1 {
                let source_x = ((x as f32 - left + 0.5) * scale_x) as u32;
                if let Some(pixel) = picture.pixel(source_x, source_y) {
                    self.image.blend(i64::from(x), i64::from(y), pixel, 255);
                }
            }
        }
    }
}

fn rgba(color: Color) -> [u8; 4] {
    [color.r, color.g, color.b, color.a]
}

#[cfg(test)]
mod tests {
    use crate::font::Fonts;
    use crate::image::Image;
    use crate::page::{Page, Viewport};

    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const GREEN: [u8; 4] = [0, 128, 0, 255];

    fn page(html: &str) -> Page {
        let viewport = Viewport {
            width: 100,
            height: 60,
        };
        Page::load(html, viewport)
    }

    fn render(html: &str) -> Image {
        page(html).render()
    }

    #[test]
    fn a_box_paints_its_background_and_border() {
        let image = render(
            "<div style='width: 20px; height: 10px; background: red; border: 2px solid blue'></div>",
        );
        assert_eq!(image.pixel(7, 7), Some(WHITE));
        assert_eq!(image.pixel(8, 8), Some(BLUE));
        assert_eq!(image.pixel(9, 19), Some(BLUE));
        assert_eq!(image.pixel(10, 10), Some(RED));
        assert_eq!(image.pixel(29, 19), Some(RED));
        assert_eq!(image.pixel(30, 19), Some(BLUE));
        assert_eq!(image.pixel(31, 21), Some(BLUE));
        assert_eq!(image.pixel(32, 22), Some(WHITE));
    }

    #[test]
    fn each_border_side_has_its_own_color() {
        let image = render(
            "<div style='width: 20px; height: 10px; border: 2px solid; border-color: red blue green white'></div>",
        );
        assert_eq!(image.pixel(15, 8), Some(RED));
        assert_eq!(image.pixel(31, 15), Some(BLUE));
        assert_eq!(image.pixel(15, 21), Some(GREEN));
        assert_eq!(image.pixel(8, 15), Some(WHITE));
    }

    #[test]
    fn the_body_background_covers_the_canvas() {
        let image = render("<body style='background: green'><div style='height: 5px'></div>");
        assert_eq!(image.pixel(0, 0), Some(GREEN));
        assert_eq!(image.pixel(99, 59), Some(GREEN));
    }

    #[test]
    fn the_root_background_wins_and_the_body_then_paints_its_own() {
        let image = render(
            "<html style='background: red'><body style='background: green'><div style='height: 5px'></div>",
        );
        assert_eq!(image.pixel(0, 0), Some(RED));
        assert_eq!(image.pixel(8, 8), Some(GREEN));
        assert_eq!(image.pixel(8, 13), Some(RED));
    }

    #[test]
    fn boxes_outside_the_canvas_are_clipped() {
        let image = render(
            "<div style='margin-left: -50px; width: 500px; height: 500px; background: blue'></div>",
        );
        assert_eq!(image.pixel(0, 8), Some(BLUE));
        assert_eq!(image.pixel(99, 59), Some(BLUE));
    }

    #[test]
    fn translucent_colors_blend_with_what_is_underneath() {
        let image = render("<div style='height: 10px; background: rgba(255, 0, 0, 0.5)'></div>");
        assert_eq!(image.pixel(10, 10), Some([255, 127, 127, 255]));
    }

    #[test]
    fn rounded_corners_cut_the_background_and_border() {
        let image = render(
            "<div style='width: 40px; height: 40px; background: red; border: 4px solid blue; border-radius: 50%'></div>",
        );
        // The box spans 8..56 in both directions: its corners are empty and its centre is filled.
        assert_eq!(image.pixel(9, 9), Some(WHITE));
        assert_eq!(image.pixel(54, 54), Some(WHITE));
        assert_eq!(image.pixel(32, 32), Some(RED));
        assert_eq!(image.pixel(32, 9), Some(BLUE));
        assert_eq!(image.pixel(9, 32), Some(BLUE));
        assert_eq!(image.pixel(32, 14), Some(RED));
    }

    #[test]
    fn opacity_and_visibility_fade_a_box_and_its_text() {
        let faded = render("<div style='height: 10px; background: red; opacity: 0.5'></div>");
        assert_eq!(faded.pixel(10, 10), Some([255, 127, 127, 255]));
        let hidden =
            render("<div style='height: 10px; background: red; visibility: hidden'></div>");
        assert_eq!(hidden.pixel(10, 10), Some(WHITE));
    }

    #[test]
    fn scrolling_moves_the_page_up() {
        let page = page(
            "<div style='height: 100px'></div><div style='height: 20px; background: red'></div>",
        );
        assert_eq!(page.render().pixel(10, 50), Some(WHITE));
        assert_eq!(page.render_at(70.0).pixel(10, 40), Some(RED));
        assert_eq!(page.render_at(70.0).pixel(10, 37), Some(WHITE));
    }

    #[test]
    fn text_leaves_ink_in_its_own_color() {
        if !Fonts::system().available() {
            return;
        }
        let image = render("<div style='color: red; font-size: 30px'>MMM</div>");
        let mut inked = 0;
        for y in 0..60 {
            for x in 0..100 {
                match image.pixel(x, y) {
                    Some(RED) => inked += 1,
                    Some([_, g, b, _]) => assert_eq!(g, b, "only red ink is expected"),
                    None => {}
                }
            }
        }
        assert!(inked > 100, "{inked}");
    }
}
