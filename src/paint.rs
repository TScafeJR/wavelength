//! Painting: turns a layout tree into pixels.

use crate::dom::Document;
use crate::image::Image;
use crate::layout::{LayoutBox, Rect};
use crate::style::Color;

/// Paints the layout tree onto a canvas of the given size.
///
/// The canvas takes the root element's background, or the body's when the
/// root has none, or white. The element that lends its background to the
/// canvas does not paint it a second time on its own box.
pub fn paint(doc: &Document, root: Option<&LayoutBox>, width: u32, height: u32) -> Image {
    let Some(root) = root else {
        return Image::new(width, height, rgba(Color::WHITE));
    };
    let body = root
        .children
        .iter()
        .find(|child| doc.element(child.node).is_some_and(|e| e.name == "body"));
    let canvas_source = [Some(root), body]
        .into_iter()
        .flatten()
        .find(|candidate| !candidate.style.background.is_transparent());

    let canvas_color = canvas_source.map_or(Color::WHITE, |source| source.style.background);
    let mut image = Image::new(width, height, rgba(canvas_color));
    paint_box(&mut image, root, canvas_source.map(|source| source.node));
    image
}

fn paint_box(
    image: &mut Image,
    layout_box: &LayoutBox,
    skip_background: Option<crate::dom::NodeId>,
) {
    let (rect, style) = (layout_box.rect, &layout_box.style);
    if skip_background != Some(layout_box.node) {
        fill(image, rect, style.background);
    }

    let (border, color) = (style.border(), style.used_border_color());
    let inner_height = rect.height - border.top - border.bottom;
    let edge = |x, y, width, height| Rect {
        x,
        y,
        width,
        height,
    };
    for side in [
        edge(rect.x, rect.y, rect.width, border.top),
        edge(
            rect.x,
            rect.y + rect.height - border.bottom,
            rect.width,
            border.bottom,
        ),
        edge(rect.x, rect.y + border.top, border.left, inner_height),
        edge(
            rect.x + rect.width - border.right,
            rect.y + border.top,
            border.right,
            inner_height,
        ),
    ] {
        fill(image, side, color);
    }

    for child in &layout_box.children {
        paint_box(image, child, skip_background);
    }
}

/// Fills a rectangle after snapping each edge to the nearest pixel boundary.
fn fill(image: &mut Image, rect: Rect, color: Color) {
    if color.is_transparent() || rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    // Float to integer casts saturate, so negative edges clamp to zero.
    let snap = |edge: f32| edge.round() as u32;
    image.fill(
        snap(rect.x),
        snap(rect.y),
        snap(rect.x + rect.width),
        snap(rect.y + rect.height),
        rgba(color),
    );
}

fn rgba(color: Color) -> [u8; 4] {
    [color.r, color.g, color.b, color.a]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{html, layout, style};

    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const GREEN: [u8; 4] = [0, 128, 0, 255];

    fn render(html: &str) -> Image {
        let doc = html::parse(html);
        let styles = style::compute(&doc);
        let root = layout::layout(&doc, &styles, 100.0);
        paint(&doc, root.as_ref(), 100, 60)
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
}
