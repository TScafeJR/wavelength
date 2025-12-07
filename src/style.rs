//! The cascade: turns stylesheets and `style` attributes into one computed style per node.

use crate::css::{self, Declaration, Specificity, Stylesheet};
use crate::dom::Document;

/// The built-in stylesheet. It covers only what the engine can lay out today.
const USER_AGENT_CSS: &str = "
html, body, div, section, article, header, footer, main, nav, aside { display: block }
head, base, link, meta, script, style, title { display: none }
body { margin: 8px }
";

/// The initial value of `border-width`, which CSS calls `medium`.
const MEDIUM_BORDER: f32 = 3.0;

/// An sRGB colour. Alpha is either fully opaque or fully transparent for now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha, where 255 is opaque.
    pub a: u8,
}

impl Color {
    /// Opaque black.
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    /// Opaque white.
    pub const WHITE: Self = Self::rgb(255, 255, 255);
    /// Fully transparent.
    pub const TRANSPARENT: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };

    /// An opaque colour.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Parses a lowercased CSS colour: `#rgb`, `#rrggbb`, `rgb(r, g, b)` or a basic colour name.
    pub fn parse(value: &str) -> Option<Self> {
        if let Some(hex) = value.strip_prefix('#') {
            if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let channel = |range: std::ops::Range<usize>| u8::from_str_radix(&hex[range], 16).ok();
            return match hex.len() {
                3 => Some(Self::rgb(
                    channel(0..1)? * 17,
                    channel(1..2)? * 17,
                    channel(2..3)? * 17,
                )),
                6 => Some(Self::rgb(channel(0..2)?, channel(2..4)?, channel(4..6)?)),
                _ => None,
            };
        }
        if let Some(arguments) = value.strip_prefix("rgb(").and_then(|v| v.strip_suffix(')')) {
            let channels: Vec<u8> = arguments
                .split(|c: char| c == ',' || c.is_ascii_whitespace())
                .filter(|part| !part.is_empty())
                .map(|part| part.parse::<u32>().ok().map(|n| n.min(255) as u8))
                .collect::<Option<_>>()?;
            return match channels[..] {
                [r, g, b] => Some(Self::rgb(r, g, b)),
                _ => None,
            };
        }
        let (r, g, b) = match value {
            "transparent" => return Some(Self::TRANSPARENT),
            "black" => (0, 0, 0),
            "silver" => (192, 192, 192),
            "gray" | "grey" => (128, 128, 128),
            "white" => (255, 255, 255),
            "maroon" => (128, 0, 0),
            "red" => (255, 0, 0),
            "purple" => (128, 0, 128),
            "fuchsia" => (255, 0, 255),
            "green" => (0, 128, 0),
            "lime" => (0, 255, 0),
            "olive" => (128, 128, 0),
            "yellow" => (255, 255, 0),
            "navy" => (0, 0, 128),
            "blue" => (0, 0, 255),
            "teal" => (0, 128, 128),
            "aqua" => (0, 255, 255),
            "orange" => (255, 165, 0),
            _ => return None,
        };
        Some(Self::rgb(r, g, b))
    }

    /// Whether painting this colour would change nothing.
    pub fn is_transparent(self) -> bool {
        self.a == 0
    }
}

/// How an element takes part in layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    /// A block-level box.
    Block,
    /// An inline box. Inline layout is not implemented, so these are not rendered yet.
    Inline,
    /// No box at all.
    None,
}

/// One value per side of a box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Edges<T> {
    /// The top side.
    pub top: T,
    /// The right side.
    pub right: T,
    /// The bottom side.
    pub bottom: T,
    /// The left side.
    pub left: T,
}

impl<T: Copy> Edges<T> {
    /// The same value on every side.
    pub const fn all(value: T) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }
}

/// The computed style of one node. Lengths are in CSS pixels and `None` means `auto`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// The `display` property.
    pub display: Display,
    /// The content width.
    pub width: Option<f32>,
    /// The content height.
    pub height: Option<f32>,
    /// The margins.
    pub margin: Edges<Option<f32>>,
    /// The padding.
    pub padding: Edges<f32>,
    /// The specified `border-width`. See [`Style::border`] for the widths actually used.
    pub border_width: Edges<f32>,
    /// Whether `border-style` is `solid`. The only other supported value is `none`.
    pub border_solid: bool,
    /// The `border-color`, where `None` means the value of `color`.
    pub border_color: Option<Color>,
    /// The `background-color`.
    pub background: Color,
    /// The inherited foreground `color`.
    pub color: Color,
}

impl Style {
    fn initial(inherited_color: Color) -> Self {
        Self {
            display: Display::Inline,
            width: None,
            height: None,
            margin: Edges::all(Some(0.0)),
            padding: Edges::all(0.0),
            border_width: Edges::all(MEDIUM_BORDER),
            border_solid: false,
            border_color: None,
            background: Color::TRANSPARENT,
            color: inherited_color,
        }
    }

    /// The border widths used for layout and painting, which are zero unless the border is drawn.
    pub fn border(&self) -> Edges<f32> {
        if self.border_solid {
            self.border_width
        } else {
            Edges::all(0.0)
        }
    }

    /// The colour the border is painted in.
    pub fn used_border_color(&self) -> Color {
        self.border_color.unwrap_or(self.color)
    }

    /// Applies one declaration. Unknown properties and invalid values leave the style untouched.
    fn apply(&mut self, name: &str, value: &str) {
        match name {
            "display" => match value {
                "block" => self.display = Display::Block,
                "inline" => self.display = Display::Inline,
                "none" => self.display = Display::None,
                _ => {}
            },
            "width" => set(&mut self.width, size(value)),
            "height" => set(&mut self.height, size(value)),
            "margin" => set(&mut self.margin, sides(value, margin)),
            "margin-top" => set(&mut self.margin.top, margin(value)),
            "margin-right" => set(&mut self.margin.right, margin(value)),
            "margin-bottom" => set(&mut self.margin.bottom, margin(value)),
            "margin-left" => set(&mut self.margin.left, margin(value)),
            "padding" => set(&mut self.padding, sides(value, non_negative)),
            "padding-top" => set(&mut self.padding.top, non_negative(value)),
            "padding-right" => set(&mut self.padding.right, non_negative(value)),
            "padding-bottom" => set(&mut self.padding.bottom, non_negative(value)),
            "padding-left" => set(&mut self.padding.left, non_negative(value)),
            "border-width" => set(&mut self.border_width, sides(value, border_width)),
            "border-style" => set(&mut self.border_solid, border_style(value)),
            "border-color" => set(&mut self.border_color, Color::parse(value).map(Some)),
            "border" => self.apply_border(value),
            "color" => set(&mut self.color, Color::parse(value)),
            "background" | "background-color" => set(&mut self.background, Color::parse(value)),
            _ => {}
        }
    }

    /// The `border` shorthand: a width, a style and a colour in any order, each optional.
    fn apply_border(&mut self, value: &str) {
        let (mut width, mut solid, mut color) = (MEDIUM_BORDER, false, None);
        for token in tokens(value) {
            if let Some(parsed) = border_width(token) {
                width = parsed;
            } else if let Some(parsed) = border_style(token) {
                solid = parsed;
            } else if let Some(parsed) = Color::parse(token) {
                color = Some(parsed);
            } else {
                return;
            }
        }
        self.border_width = Edges::all(width);
        self.border_solid = solid;
        self.border_color = color;
    }
}

fn set<T>(slot: &mut T, parsed: Option<T>) {
    if let Some(value) = parsed {
        *slot = value;
    }
}

/// A length in pixels. Only `px` and a bare zero are understood so far.
fn length(value: &str) -> Option<f32> {
    let number = match value.strip_suffix("px") {
        Some(number) => number.parse::<f32>().ok()?,
        None => value.parse::<f32>().ok().filter(|n| *n == 0.0)?,
    };
    number.is_finite().then_some(number)
}

fn non_negative(value: &str) -> Option<f32> {
    length(value).filter(|n| *n >= 0.0)
}

fn size(value: &str) -> Option<Option<f32>> {
    if value == "auto" {
        Some(None)
    } else {
        non_negative(value).map(Some)
    }
}

fn margin(value: &str) -> Option<Option<f32>> {
    if value == "auto" {
        Some(None)
    } else {
        length(value).map(Some)
    }
}

fn border_width(value: &str) -> Option<f32> {
    match value {
        "thin" => Some(1.0),
        "medium" => Some(MEDIUM_BORDER),
        "thick" => Some(5.0),
        _ => non_negative(value),
    }
}

fn border_style(value: &str) -> Option<bool> {
    match value {
        "solid" => Some(true),
        "none" => Some(false),
        _ => None,
    }
}

/// Expands the one to four values of a box shorthand such as `margin`.
fn sides<T: Copy>(value: &str, parse: impl Fn(&str) -> Option<T>) -> Option<Edges<T>> {
    let values: Vec<T> = tokens(value)
        .into_iter()
        .map(parse)
        .collect::<Option<_>>()?;
    let (top, right, bottom, left) = match values[..] {
        [all] => (all, all, all, all),
        [vertical, horizontal] => (vertical, horizontal, vertical, horizontal),
        [top, horizontal, bottom] => (top, horizontal, bottom, horizontal),
        [top, right, bottom, left] => (top, right, bottom, left),
        _ => return None,
    };
    Some(Edges {
        top,
        right,
        bottom,
        left,
    })
}

/// Splits a value on whitespace, keeping functions such as `rgb(1, 2, 3)` whole.
fn tokens(value: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let (mut depth, mut start) = (0usize, None);
    for (index, c) in value.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if c.is_ascii_whitespace() && depth == 0 {
            if let Some(from) = start.take() {
                tokens.push(&value[from..index]);
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(from) = start {
        tokens.push(&value[from..]);
    }
    tokens
}

/// Computes a style for every node, indexed by node id.
///
/// Author rules come from the document's `style` elements. Declarations are
/// ordered by origin, then specificity, then source order, and `style`
/// attributes are applied last.
pub fn compute(doc: &Document) -> Vec<Style> {
    let user_agent = css::parse(USER_AGENT_CSS);
    let mut author = Stylesheet::default();
    for id in doc.ids() {
        if doc.element(id).is_some_and(|e| e.name == "style") {
            author.rules.extend(css::parse(&doc.text_content(id)).rules);
        }
    }

    let mut styles: Vec<Style> = Vec::with_capacity(doc.len());
    for id in doc.ids() {
        // Parents are always created before their children, so the parent's style exists.
        let inherited_color = doc
            .node(id)
            .parent
            .map_or(Color::BLACK, |parent| styles[parent.0].color);
        let mut style = Style::initial(inherited_color);
        if let Some(element) = doc.element(id) {
            let mut matched: Vec<(usize, Specificity, usize, &Declaration)> = Vec::new();
            for (origin, sheet) in [&user_agent, &author].into_iter().enumerate() {
                for (order, rule) in sheet.rules.iter().enumerate() {
                    let specificity = rule
                        .selectors
                        .iter()
                        .filter(|selector| selector.matches(doc, id))
                        .map(css::Selector::specificity)
                        .max();
                    if let Some(specificity) = specificity {
                        let declarations = rule.declarations.iter();
                        matched.extend(declarations.map(|d| (origin, specificity, order, d)));
                    }
                }
            }
            // The sort is stable, so declarations within one rule keep their order.
            matched.sort_by_key(|&(origin, specificity, order, _)| (origin, specificity, order));
            for (_, _, _, declaration) in matched {
                style.apply(&declaration.name, &declaration.value);
            }
            if let Some(inline) = element.attribute("style") {
                for declaration in css::parse_declarations(inline) {
                    style.apply(&declaration.name, &declaration.value);
                }
            }
        }
        styles.push(style);
    }
    styles
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html;

    fn style_of(html: &str, tag: &str) -> Option<Style> {
        let doc = html::parse(html);
        let styles = compute(&doc);
        doc.find_element(tag).map(|id| styles[id.0])
    }

    #[test]
    fn colors_parse_in_each_supported_form() {
        assert_eq!(Color::parse("#f80"), Some(Color::rgb(255, 136, 0)));
        assert_eq!(Color::parse("#0a141e"), Some(Color::rgb(10, 20, 30)));
        assert_eq!(Color::parse("rgb(1, 2,3)"), Some(Color::rgb(1, 2, 3)));
        assert_eq!(Color::parse("rgb(1 2 300)"), Some(Color::rgb(1, 2, 255)));
        assert_eq!(Color::parse("teal"), Some(Color::rgb(0, 128, 128)));
        assert_eq!(Color::parse("transparent"), Some(Color::TRANSPARENT));
        for invalid in [
            "#ff",
            "#ggg",
            "#+f0",
            "rgb(1, 2)",
            "rgb(1, 2, 3, 4)",
            "nope",
            "#é",
        ] {
            assert_eq!(Color::parse(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn the_user_agent_sheet_applies() {
        let body = style_of("<div></div>", "body");
        assert_eq!(body.map(|s| s.display), Some(Display::Block));
        assert_eq!(body.map(|s| s.margin), Some(Edges::all(Some(8.0))));
        assert_eq!(
            style_of("<span></span>", "span").map(|s| s.display),
            Some(Display::Inline)
        );
        assert_eq!(
            style_of("<title>x</title>", "title").map(|s| s.display),
            Some(Display::None)
        );
    }

    #[test]
    fn specificity_beats_order_and_inline_style_beats_both() {
        let html = "<style>#a { width: 10px; height: 10px } .b { width: 20px } \
                    div { width: 30px; padding: 1px } div { padding: 2px }</style>\
                    <div id=a class=b style='height: 40px'></div>";
        let div = style_of(html, "div");
        assert_eq!(div.map(|s| s.width), Some(Some(10.0)));
        assert_eq!(div.map(|s| s.height), Some(Some(40.0)));
        assert_eq!(div.map(|s| s.padding), Some(Edges::all(2.0)));
    }

    #[test]
    fn box_shorthands_expand_and_invalid_values_are_ignored() {
        let html = "<div style='margin: 1px auto 3px; padding: 5px; padding: -1px; \
                    padding-left: 2em; width: 50%; margin-left: -4px'></div>";
        let div = style_of(html, "div");
        assert_eq!(
            div.map(|s| s.margin),
            Some(Edges {
                top: Some(1.0),
                right: None,
                bottom: Some(3.0),
                left: Some(-4.0)
            })
        );
        assert_eq!(div.map(|s| s.padding), Some(Edges::all(5.0)));
        assert_eq!(div.map(|s| s.width), Some(None));
    }

    #[test]
    fn borders_need_a_style_and_default_to_the_text_color() {
        let unstyled = style_of("<div style='border-width: 4px'></div>", "div");
        assert_eq!(unstyled.map(|s| s.border()), Some(Edges::all(0.0)));

        let medium = style_of("<div style='color: red; border-style: solid'></div>", "div");
        assert_eq!(medium.map(|s| s.border()), Some(Edges::all(3.0)));
        assert_eq!(
            medium.map(|s| s.used_border_color()),
            Some(Color::rgb(255, 0, 0))
        );

        let shorthand = style_of(
            "<div style='border: rgb(0, 0, 255) 2px solid'></div>",
            "div",
        );
        assert_eq!(shorthand.map(|s| s.border()), Some(Edges::all(2.0)));
        assert_eq!(
            shorthand.map(|s| s.used_border_color()),
            Some(Color::rgb(0, 0, 255))
        );
    }

    #[test]
    fn color_inherits() {
        let html = "<div style='color: #00f'><section></section></div>";
        assert_eq!(
            style_of(html, "section").map(|s| s.color),
            Some(Color::rgb(0, 0, 255))
        );
    }
}
