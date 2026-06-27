//! The cascade: turns stylesheets, `style` attributes and legacy HTML
//! attributes into one computed style per node.

use std::collections::HashMap;

use crate::css::{self, Specificity};
use crate::dom::{Document, Element, NodeId};

/// The built-in stylesheet. It covers only what the engine can lay out today.
const USER_AGENT_CSS: &str = "
html, body, div, section, article, header, footer, main, nav, aside, p, h1, h2, h3, h4, h5, h6,
ul, ol, li, dl, dt, dd, form, center, blockquote, pre, hr, address, figure, figcaption, fieldset,
details, summary, thead, tbody, tfoot, caption { display: block }
head, base, link, meta, script, style, title, template, datalist, option, svg, area, param,
source, track { display: none }
body { margin: 8px }
p, dl, ul, ol, pre, figure { margin: 1em 0 }
blockquote { margin: 1em 40px }
ul, ol { padding-left: 40px }
dd { margin-left: 40px }
h1 { font-size: 2em; margin: 0.67em 0; font-weight: bold }
h2 { font-size: 1.5em; margin: 0.83em 0; font-weight: bold }
h3 { font-size: 1.17em; margin: 1em 0; font-weight: bold }
h4 { margin: 1.33em 0; font-weight: bold }
h5 { font-size: 0.83em; margin: 1.67em 0; font-weight: bold }
h6 { font-size: 0.67em; margin: 2.33em 0; font-weight: bold }
b, strong, th { font-weight: bold }
small { font-size: smaller }
big { font-size: larger }
a { color: #0000ee; text-decoration: underline }
u, ins { text-decoration: underline }
center { text-align: -webkit-center }
pre { white-space: pre }
hr { border: 1px solid #808080; margin: 8px 0 }
table { display: table }
tr { display: table-row }
td, th { display: table-cell; padding: 1px; vertical-align: middle }
th { text-align: center }
img { display: inline-block }
input, textarea, select, button { display: inline-block; font-size: 13.333px; font-weight: normal;
  color: #000; line-height: normal; text-align: left; text-decoration: none; white-space: nowrap }
input, textarea, select { background: #fff; border: 1px solid #767676; padding: 1px 2px }
button { background: #efefef; border: 1px solid #767676; padding: 1px 6px; text-align: center }
";

/// The initial value of `border-width`, which CSS calls `medium`.
const MEDIUM_BORDER: f32 = 3.0;
/// The initial `font-size`, which CSS calls `medium`.
const DEFAULT_FONT_SIZE: f32 = 16.0;

/// An sRGB colour with straight alpha.
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

    /// Parses a lowercased CSS colour: hex notation, `rgb()`, `rgba()` or a common colour name.
    pub fn parse(value: &str) -> Option<Self> {
        if let Some(hex) = value.strip_prefix('#') {
            return Self::parse_hex(hex);
        }
        let function = value
            .strip_prefix("rgba(")
            .or_else(|| value.strip_prefix("rgb("));
        if let Some(arguments) = function.and_then(|v| v.strip_suffix(')')) {
            let parts: Vec<&str> = arguments
                .split(|c: char| c == ',' || c == '/' || c.is_ascii_whitespace())
                .filter(|part| !part.is_empty())
                .collect();
            let channel = |part: &str| {
                let number = match part.strip_suffix('%') {
                    Some(percent) => percent.parse::<f32>().ok()? * 2.55,
                    None => part.parse::<f32>().ok()?,
                };
                number
                    .is_finite()
                    .then(|| number.round().clamp(0.0, 255.0) as u8)
            };
            let alpha = |part: &str| {
                let number = match part.strip_suffix('%') {
                    Some(percent) => percent.parse::<f32>().ok()? / 100.0,
                    None => part.parse::<f32>().ok()?,
                };
                number
                    .is_finite()
                    .then(|| (number.clamp(0.0, 1.0) * 255.0).round() as u8)
            };
            return match parts[..] {
                [r, g, b] => Some(Self::rgb(channel(r)?, channel(g)?, channel(b)?)),
                [r, g, b, a] => Some(Self {
                    r: channel(r)?,
                    g: channel(g)?,
                    b: channel(b)?,
                    a: alpha(a)?,
                }),
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
            "fuchsia" | "magenta" => (255, 0, 255),
            "green" => (0, 128, 0),
            "lime" => (0, 255, 0),
            "olive" => (128, 128, 0),
            "yellow" => (255, 255, 0),
            "navy" => (0, 0, 128),
            "blue" => (0, 0, 255),
            "teal" => (0, 128, 128),
            "aqua" | "cyan" => (0, 255, 255),
            "orange" => (255, 165, 0),
            "pink" => (255, 192, 203),
            "brown" => (165, 42, 42),
            "gold" => (255, 215, 0),
            "indigo" => (75, 0, 130),
            "violet" => (238, 130, 238),
            "tan" => (210, 180, 140),
            "beige" => (245, 245, 220),
            "ivory" => (255, 255, 240),
            "crimson" => (220, 20, 60),
            "coral" => (255, 127, 80),
            "tomato" => (255, 99, 71),
            "salmon" => (250, 128, 114),
            "khaki" => (240, 230, 140),
            "lightgray" | "lightgrey" => (211, 211, 211),
            "darkgray" | "darkgrey" => (169, 169, 169),
            "dimgray" | "dimgrey" => (105, 105, 105),
            "whitesmoke" => (245, 245, 245),
            "gainsboro" => (220, 220, 220),
            "lightblue" => (173, 216, 230),
            "skyblue" => (135, 206, 235),
            "steelblue" => (70, 130, 180),
            "royalblue" => (65, 105, 225),
            "darkblue" => (0, 0, 139),
            "darkgreen" => (0, 100, 0),
            "darkred" => (139, 0, 0),
            "lightyellow" => (255, 255, 224),
            "lightgreen" => (144, 238, 144),
            _ => return None,
        };
        Some(Self::rgb(r, g, b))
    }

    fn parse_hex(hex: &str) -> Option<Self> {
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let digits: Vec<u8> = hex
            .bytes()
            .filter_map(|b| (b as char).to_digit(16).map(|d| d as u8))
            .collect();
        let pair = |index: usize| digits[index] * 16 + digits[index + 1];
        match digits.len() {
            3 | 4 => Some(Self {
                r: digits[0] * 17,
                g: digits[1] * 17,
                b: digits[2] * 17,
                a: digits.get(3).map_or(255, |a| a * 17),
            }),
            6 => Some(Self::rgb(pair(0), pair(2), pair(4))),
            8 => Some(Self {
                r: pair(0),
                g: pair(2),
                b: pair(4),
                a: pair(6),
            }),
            _ => None,
        }
    }

    /// Whether painting this colour would change nothing.
    pub fn is_transparent(self) -> bool {
        self.a == 0
    }
}

/// How an element takes part in layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    /// A block-level box. Flex and grid containers are laid out as blocks for now.
    Block,
    /// An inline box, whose content flows into its parent's lines.
    Inline,
    /// A box that sits on a line but is laid out inside as a block.
    InlineBlock,
    /// A table.
    Table,
    /// A table row.
    TableRow,
    /// A table cell.
    TableCell,
    /// No box at all.
    None,
}

/// A size that is either absolute or relative to the containing block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Length {
    /// CSS pixels.
    Px(f32),
    /// A percentage of the containing block's size.
    Percent(f32),
}

/// Horizontal alignment of inline content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    /// Lines start at the left edge.
    Left,
    /// Lines are centred.
    Center,
    /// Lines end at the right edge.
    Right,
}

/// The height of a line of text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineHeight {
    /// Whatever the font recommends.
    Normal,
    /// A multiple of the font size.
    Number(f32),
    /// A fixed height in pixels.
    Px(f32),
}

/// Where a table cell's content sits when the row is taller than the content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalAlign {
    /// On the baseline, which for cells behaves like the top.
    Baseline,
    /// At the top.
    Top,
    /// In the middle.
    Middle,
    /// At the bottom.
    Bottom,
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

    fn side_mut(&mut self, side: &str) -> Option<&mut T> {
        match side {
            "top" => Some(&mut self.top),
            "right" => Some(&mut self.right),
            "bottom" => Some(&mut self.bottom),
            "left" => Some(&mut self.left),
            _ => None,
        }
    }
}

/// The computed style of one node. Lengths are in CSS pixels and `None` means `auto`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// The `display` property.
    pub display: Display,
    /// The content width.
    pub width: Option<Length>,
    /// The content height.
    pub height: Option<Length>,
    /// The largest content width allowed.
    pub max_width: Option<f32>,
    /// The margins.
    pub margin: Edges<Option<f32>>,
    /// The padding.
    pub padding: Edges<f32>,
    /// The specified `border-width`. See [`Style::border`] for the widths actually used.
    pub border_width: Edges<f32>,
    /// Whether each side's border is drawn. Every style other than `none` is drawn solid.
    pub border_solid: Edges<bool>,
    /// The `border-color` of each side, where `None` means the value of `color`.
    pub border_color: Edges<Option<Color>>,
    /// The `background-color`.
    pub background: Color,
    /// Where a table cell's content sits vertically.
    pub vertical_align: VerticalAlign,
    /// The foreground `color`. Inherited.
    pub color: Color,
    /// The `font-size` in pixels. Inherited.
    pub font_size: f32,
    /// Whether `font-weight` is bold. Inherited.
    pub bold: bool,
    /// The `line-height`. Inherited.
    pub line_height: LineHeight,
    /// The `text-align`. Inherited.
    pub text_align: TextAlign,
    /// Whether block children are centred too, as the `center` element does. Inherited.
    pub center_blocks: bool,
    /// Whether text is underlined. Inherited, which approximates how decorations propagate.
    pub underline: bool,
    /// Whether lines may only break where the markup says so. Inherited.
    pub nowrap: bool,
}

impl Style {
    fn initial(parent: Option<&Style>) -> Self {
        Self {
            display: Display::Inline,
            width: None,
            height: None,
            max_width: None,
            margin: Edges::all(Some(0.0)),
            padding: Edges::all(0.0),
            border_width: Edges::all(MEDIUM_BORDER),
            border_solid: Edges::all(false),
            border_color: Edges::all(None),
            background: Color::TRANSPARENT,
            vertical_align: VerticalAlign::Baseline,
            color: parent.map_or(Color::BLACK, |p| p.color),
            font_size: parent.map_or(DEFAULT_FONT_SIZE, |p| p.font_size),
            bold: parent.is_some_and(|p| p.bold),
            line_height: parent.map_or(LineHeight::Normal, |p| p.line_height),
            text_align: parent.map_or(TextAlign::Left, |p| p.text_align),
            center_blocks: parent.is_some_and(|p| p.center_blocks),
            underline: parent.is_some_and(|p| p.underline),
            nowrap: parent.is_some_and(|p| p.nowrap),
        }
    }

    /// The border widths used for layout and painting, which are zero on sides that are not drawn.
    pub fn border(&self) -> Edges<f32> {
        let used = |solid: bool, width: f32| if solid { width } else { 0.0 };
        Edges {
            top: used(self.border_solid.top, self.border_width.top),
            right: used(self.border_solid.right, self.border_width.right),
            bottom: used(self.border_solid.bottom, self.border_width.bottom),
            left: used(self.border_solid.left, self.border_width.left),
        }
    }

    /// The colour each side's border is painted in.
    pub fn used_border_color(&self) -> Edges<Color> {
        Edges {
            top: self.border_color.top.unwrap_or(self.color),
            right: self.border_color.right.unwrap_or(self.color),
            bottom: self.border_color.bottom.unwrap_or(self.color),
            left: self.border_color.left.unwrap_or(self.color),
        }
    }

    /// The line height in pixels, given what the font recommends for `normal`.
    pub fn used_line_height(&self, font_normal: f32) -> f32 {
        match self.line_height {
            LineHeight::Normal => font_normal,
            LineHeight::Number(factor) => factor * self.font_size,
            LineHeight::Px(height) => height,
        }
    }

    /// Applies a font size declaration, which has to happen before anything measured in `em`.
    fn apply_font_size(&mut self, name: &str, value: &str, parent_size: f32) {
        match name {
            "font-size" => set(&mut self.font_size, font_size(value, parent_size)),
            "font" => self.apply_font(value, parent_size),
            _ => {}
        }
    }

    /// The `font` shorthand: optional style and weight, a size with an optional line height, then families.
    fn apply_font(&mut self, value: &str, parent_size: f32) {
        let mut bold = false;
        for token in tokens(value) {
            let (size_part, line_part) = token.split_once('/').unwrap_or((token, ""));
            if let Some(size) = font_size(size_part, parent_size) {
                self.font_size = size;
                self.bold = bold;
                self.line_height = line_height(line_part, size).unwrap_or(LineHeight::Normal);
                return;
            }
            match font_weight(token) {
                Some(weight) => bold = weight,
                // Anything else before the size is a style or variant keyword.
                None if matches!(token, "italic" | "oblique" | "small-caps") => {}
                None => return,
            }
        }
    }

    /// Applies one declaration. Unknown properties and invalid values leave the style untouched.
    fn apply(&mut self, name: &str, value: &str) {
        let em = self.font_size;
        let non_negative = |v: &str| length(v, em).filter(|n| *n >= 0.0);
        let margin = |v: &str| {
            if v == "auto" {
                Some(None)
            } else {
                length(v, em).map(Some)
            }
        };
        let size = |v: &str| match v {
            "auto" | "fit-content" | "max-content" | "min-content" => Some(None),
            _ => match v.strip_suffix('%') {
                Some(percent) => {
                    let percent = percent.parse::<f32>().ok().filter(|p| *p >= 0.0)?;
                    Some(Some(Length::Percent(percent)))
                }
                None => non_negative(v).map(|px| Some(Length::Px(px))),
            },
        };
        match name {
            "display" => set(&mut self.display, display(value)),
            "width" => set(&mut self.width, size(value)),
            "height" => set(&mut self.height, size(value)),
            "max-width" => match value {
                "none" => self.max_width = None,
                _ => set(&mut self.max_width, non_negative(value).map(Some)),
            },
            "margin" => set(&mut self.margin, sides(value, margin)),
            "padding" => set(&mut self.padding, sides(value, non_negative)),
            "border-width" => set(
                &mut self.border_width,
                sides(value, |v| border_width(v, em)),
            ),
            "border-style" => set(&mut self.border_solid, sides(value, border_style)),
            "border-color" => {
                set(
                    &mut self.border_color,
                    sides(value, |v| Color::parse(v).map(Some)),
                );
            }
            "border" => {
                if let Some((width, solid, color)) = border_shorthand(value, em) {
                    self.border_width = Edges::all(width);
                    self.border_solid = Edges::all(solid);
                    self.border_color = Edges::all(color);
                }
            }
            "color" => set(&mut self.color, Color::parse(value)),
            "background-color" => set(&mut self.background, Color::parse(value)),
            "background" => {
                // The shorthand may also carry images and positions, which are skipped.
                let color = tokens(value).into_iter().find_map(Color::parse);
                set(
                    &mut self.background,
                    color.or((value == "none").then_some(Color::TRANSPARENT)),
                );
            }
            "font-weight" => set(&mut self.bold, font_weight(value)),
            "line-height" => set(&mut self.line_height, line_height(value, em)),
            "text-align" => match value {
                "left" | "start" | "justify" => self.set_align(TextAlign::Left, false),
                "right" | "end" => self.set_align(TextAlign::Right, false),
                "center" => self.set_align(TextAlign::Center, false),
                "-webkit-center" | "-moz-center" => self.set_align(TextAlign::Center, true),
                _ => {}
            },
            "text-decoration" | "text-decoration-line" => {
                if tokens(value).contains(&"underline") {
                    self.underline = true;
                } else if tokens(value).contains(&"none") {
                    self.underline = false;
                }
            }
            "white-space" => match value {
                "nowrap" | "pre" => self.nowrap = true,
                "normal" | "pre-wrap" | "pre-line" | "break-spaces" => self.nowrap = false,
                _ => {}
            },
            "vertical-align" => match value {
                "top" | "text-top" => self.vertical_align = VerticalAlign::Top,
                "middle" => self.vertical_align = VerticalAlign::Middle,
                "bottom" | "text-bottom" => self.vertical_align = VerticalAlign::Bottom,
                "baseline" => self.vertical_align = VerticalAlign::Baseline,
                _ => {}
            },
            _ => self.apply_side(name, value),
        }
    }

    fn set_align(&mut self, align: TextAlign, center_blocks: bool) {
        self.text_align = align;
        self.center_blocks = center_blocks;
    }

    /// Longhands that name one side, such as `margin-left` or `border-top-color`.
    fn apply_side(&mut self, name: &str, value: &str) {
        let em = self.font_size;
        if let Some(side) = name.strip_prefix("margin-") {
            let parsed = if value == "auto" {
                Some(None)
            } else {
                length(value, em).map(Some)
            };
            if let (Some(slot), Some(parsed)) = (self.margin.side_mut(side), parsed) {
                *slot = parsed;
            }
        } else if let Some(side) = name.strip_prefix("padding-") {
            let parsed = length(value, em).filter(|n| *n >= 0.0);
            if let (Some(slot), Some(parsed)) = (self.padding.side_mut(side), parsed) {
                *slot = parsed;
            }
        } else if let Some(rest) = name.strip_prefix("border-") {
            let (side, part) = rest.split_once('-').unwrap_or((rest, ""));
            match part {
                "" => {
                    if let Some((width, solid, color)) = border_shorthand(value, em) {
                        set_side(&mut self.border_width, side, Some(width));
                        set_side(&mut self.border_solid, side, Some(solid));
                        set_side(&mut self.border_color, side, Some(color));
                    }
                }
                "width" => set_side(&mut self.border_width, side, border_width(value, em)),
                "style" => set_side(&mut self.border_solid, side, border_style(value)),
                "color" => set_side(&mut self.border_color, side, Color::parse(value).map(Some)),
                _ => {}
            }
        }
    }
}

fn set<T>(slot: &mut T, parsed: Option<T>) {
    if let Some(value) = parsed {
        *slot = value;
    }
}

fn set_side<T: Copy>(edges: &mut Edges<T>, side: &str, parsed: Option<T>) {
    if let (Some(slot), Some(value)) = (edges.side_mut(side), parsed) {
        *slot = value;
    }
}

/// A length in pixels. `em` is the font size that relative units resolve against.
fn length(value: &str, em: f32) -> Option<f32> {
    const UNITS: &[(&str, f32)] = &[("px", 1.0), ("pt", 4.0 / 3.0), ("pc", 16.0), ("in", 96.0)];
    let number = if let Some(number) = value.strip_suffix("rem") {
        number.parse::<f32>().ok()? * DEFAULT_FONT_SIZE
    } else if let Some(number) = value.strip_suffix("em") {
        number.parse::<f32>().ok()? * em
    } else if let Some(number) = value
        .strip_suffix("ex")
        .or_else(|| value.strip_suffix("ch"))
    {
        number.parse::<f32>().ok()? * em * 0.5
    } else if let Some((number, factor)) = UNITS
        .iter()
        .find_map(|(unit, factor)| Some((value.strip_suffix(unit)?, factor)))
    {
        number.parse::<f32>().ok()? * factor
    } else {
        // Without a unit only zero is a valid length.
        value.parse::<f32>().ok().filter(|n| *n == 0.0)?
    };
    number.is_finite().then_some(number)
}

fn font_size(value: &str, parent: f32) -> Option<f32> {
    let size = match value {
        "xx-small" => 9.0,
        "x-small" => 10.0,
        "small" => 13.0,
        "medium" => 16.0,
        "large" => 18.0,
        "x-large" => 24.0,
        "xx-large" => 32.0,
        "smaller" => parent / 1.2,
        "larger" => parent * 1.2,
        _ => match value.strip_suffix('%') {
            Some(percent) => percent.parse::<f32>().ok()? * parent / 100.0,
            None => length(value, parent)?,
        },
    };
    (size.is_finite() && size >= 0.0).then_some(size)
}

fn font_weight(value: &str) -> Option<bool> {
    match value {
        "bold" | "bolder" => Some(true),
        "normal" | "lighter" => Some(false),
        _ => value.parse::<f32>().ok().map(|weight| weight >= 600.0),
    }
}

fn line_height(value: &str, em: f32) -> Option<LineHeight> {
    if value == "normal" {
        return Some(LineHeight::Normal);
    }
    if let Some(percent) = value.strip_suffix('%') {
        return percent
            .parse::<f32>()
            .ok()
            .map(|p| LineHeight::Px(p * em / 100.0));
    }
    match value.parse::<f32>() {
        Ok(factor) if factor >= 0.0 => Some(LineHeight::Number(factor)),
        Ok(_) => None,
        Err(_) => length(value, em).filter(|n| *n >= 0.0).map(LineHeight::Px),
    }
}

fn display(value: &str) -> Option<Display> {
    Some(match value {
        "block" | "list-item" | "flex" | "grid" | "flow-root" | "table-caption"
        | "table-row-group" | "table-header-group" | "table-footer-group" => Display::Block,
        "inline" | "contents" => Display::Inline,
        "inline-block" | "inline-flex" | "inline-grid" | "inline-table" => Display::InlineBlock,
        "table" => Display::Table,
        "table-row" => Display::TableRow,
        "table-cell" => Display::TableCell,
        "none" => Display::None,
        _ => return None,
    })
}

fn border_width(value: &str, em: f32) -> Option<f32> {
    match value {
        "thin" => Some(1.0),
        "medium" => Some(MEDIUM_BORDER),
        "thick" => Some(5.0),
        _ => length(value, em).filter(|n| *n >= 0.0),
    }
}

fn border_style(value: &str) -> Option<bool> {
    match value {
        "none" | "hidden" => Some(false),
        "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset" => {
            Some(true)
        }
        _ => None,
    }
}

/// A border shorthand: a width, a style and a colour in any order, each optional.
fn border_shorthand(value: &str, em: f32) -> Option<(f32, bool, Option<Color>)> {
    let (mut width, mut solid, mut color) = (MEDIUM_BORDER, false, None);
    for token in tokens(value) {
        if let Some(parsed) = border_width(token, em) {
            width = parsed;
        } else if let Some(parsed) = border_style(token) {
            solid = parsed;
        } else {
            color = Some(Color::parse(token)?);
        }
    }
    Some((width, solid, color))
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

/// Built-in styles that depend on attributes, which the selector engine cannot express yet.
fn element_defaults(element: &Element) -> Vec<(&'static str, String)> {
    let mut defaults = Vec::new();
    if element.attribute("hidden").is_some() {
        defaults.push(("display", "none".to_owned()));
    }
    if element.name == "input" {
        match element
            .attribute("type")
            .unwrap_or("text")
            .to_ascii_lowercase()
            .as_str()
        {
            "hidden" => defaults.push(("display", "none".to_owned())),
            "submit" | "button" | "reset" => {
                defaults.push(("background", "#efefef".to_owned()));
                defaults.push(("padding", "1px 6px".to_owned()));
                defaults.push(("text-align", "center".to_owned()));
            }
            _ => {}
        }
    }
    defaults
}

/// Styles carried by legacy presentational attributes such as `bgcolor` and `align`.
fn presentational_hints(
    doc: &Document,
    id: NodeId,
    element: &Element,
) -> Vec<(&'static str, String)> {
    let mut hints = Vec::new();
    let name = element.name.as_str();
    let dimension = |value: &str| {
        let value = value.trim();
        match value.strip_suffix('%') {
            Some(percent) => percent.parse::<f32>().ok().map(|p| format!("{p}%")),
            None => value.parse::<f32>().ok().map(|px| format!("{px}px")),
        }
    };
    let legacy_color = |value: &str| {
        let value = value.trim().to_ascii_lowercase();
        let bare_hex = matches!(value.len(), 3 | 6) && value.bytes().all(|b| b.is_ascii_hexdigit());
        if bare_hex { format!("#{value}") } else { value }
    };

    if matches!(name, "body" | "table" | "tr" | "td" | "th")
        && let Some(color) = element.attribute("bgcolor")
    {
        hints.push(("background-color", legacy_color(color)));
    }
    if name == "font"
        && let Some(color) = element.attribute("color")
    {
        hints.push(("color", legacy_color(color)));
    }
    if matches!(name, "img" | "table" | "td" | "th" | "hr") {
        for property in ["width", "height"] {
            if let Some(value) = element.attribute(property).and_then(dimension) {
                hints.push((property, value));
            }
        }
    }
    if let Some(align) = element.attribute("align") {
        let align = align.to_ascii_lowercase();
        if name == "table" {
            if align == "center" {
                hints.push(("margin-left", "auto".to_owned()));
                hints.push(("margin-right", "auto".to_owned()));
            }
        } else if name != "img" {
            match align.as_str() {
                "center" | "middle" => hints.push(("text-align", "-webkit-center".to_owned())),
                "left" | "right" => hints.push(("text-align", align)),
                _ => {}
            }
        }
    }
    if matches!(name, "td" | "th") {
        if element.attribute("nowrap").is_some() {
            hints.push(("white-space", "nowrap".to_owned()));
        }
        let row = doc.node(id).parent.and_then(|parent| doc.element(parent));
        let valign = element
            .attribute("valign")
            .or_else(|| row.and_then(|row| row.attribute("valign")));
        if let Some(valign) = valign {
            hints.push(("vertical-align", valign.to_ascii_lowercase()));
        }
    }
    hints
}

/// Computes a style for every node, indexed by node id.
///
/// Author rules come from the document's `style` elements and from `linked`,
/// which maps a `link` element to the stylesheet text loaded for it.
/// Declarations are ordered by importance, origin, specificity and source
/// order, with `style` attributes ranking above every selector.
pub fn compute(doc: &Document, linked: &HashMap<NodeId, String>) -> Vec<Style> {
    let user_agent = css::parse(USER_AGENT_CSS);
    let mut author = css::Stylesheet::default();
    for id in doc.ids() {
        if let Some(text) = linked.get(&id) {
            author.rules.extend(css::parse(text).rules);
        } else if doc.element(id).is_some_and(|e| e.name == "style") {
            author.rules.extend(css::parse(&doc.text_content(id)).rules);
        }
    }
    let inline_specificity: Specificity = (u32::MAX, 0, 0);

    let mut styles: Vec<Style> = Vec::with_capacity(doc.len());
    for id in doc.ids() {
        // Parents are always created before their children, so the parent's style exists.
        let parent = doc.node(id).parent.map(|parent| styles[parent.0]);
        let mut style = Style::initial(parent.as_ref());
        if let Some(element) = doc.element(id) {
            // Each entry is (important, origin, specificity, order, name, value).
            let mut matched: Vec<(bool, usize, Specificity, usize, &str, String)> = Vec::new();
            for (name, value) in element_defaults(element) {
                matched.push((false, 0, inline_specificity, 0, name, value));
            }
            for (name, value) in presentational_hints(doc, id, element) {
                matched.push((false, 1, (0, 0, 0), 0, name, value));
            }
            for (origin, sheet) in [&user_agent, &author].into_iter().enumerate() {
                for (order, rule) in sheet.rules.iter().enumerate() {
                    let specificity = rule
                        .selectors
                        .iter()
                        .filter(|selector| selector.matches(doc, id))
                        .map(css::Selector::specificity)
                        .max();
                    if let Some(specificity) = specificity {
                        matched.extend(rule.declarations.iter().map(|d| {
                            (
                                d.important,
                                origin,
                                specificity,
                                order + 1,
                                d.name.as_str(),
                                d.value.clone(),
                            )
                        }));
                    }
                }
            }
            let inline = element.attribute("style").map(css::parse_declarations);
            for declaration in inline.iter().flatten() {
                matched.push((
                    declaration.important,
                    1,
                    inline_specificity,
                    usize::MAX,
                    declaration.name.as_str(),
                    declaration.value.clone(),
                ));
            }
            // The sort is stable, so declarations within one rule keep their order.
            matched.sort_by_key(|&(important, origin, specificity, order, _, _)| {
                (important, origin, specificity, order)
            });

            // Font sizes first, because every `em` in the other declarations depends on them.
            let parent_size = parent.map_or(DEFAULT_FONT_SIZE, |p| p.font_size);
            for (_, _, _, _, name, value) in &matched {
                style.apply_font_size(name, value, parent_size);
            }
            for (_, _, _, _, name, value) in &matched {
                if !matches!(*name, "font-size" | "font") {
                    style.apply(name, value);
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
        let styles = compute(&doc, &HashMap::new());
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
        let translucent = Color {
            r: 32,
            g: 33,
            b: 36,
            a: 128,
        };
        assert_eq!(Color::parse("rgba(32,33,36,.5)"), Some(translucent));
        assert_eq!(Color::parse("#20212480"), Some(translucent));
        for invalid in [
            "#ff",
            "#ggg",
            "#+f0",
            "rgb(1, 2)",
            "rgb(1, 2, 3, 4, 5)",
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
        let heading = style_of("<h1>x</h1>", "h1");
        assert_eq!(heading.map(|s| (s.font_size, s.bold)), Some((32.0, true)));
        assert_eq!(heading.map(|s| s.margin.top), Some(Some(32.0 * 0.67)));
    }

    #[test]
    fn specificity_beats_order_and_inline_style_beats_both() {
        let html = "<style>#a { width: 10px; height: 10px } .b { width: 20px } \
                    div { width: 30px; padding: 1px } div { padding: 2px }</style>\
                    <div id=a class=b style='height: 40px'></div>";
        let div = style_of(html, "div");
        assert_eq!(div.map(|s| s.width), Some(Some(Length::Px(10.0))));
        assert_eq!(div.map(|s| s.height), Some(Some(Length::Px(40.0))));
        assert_eq!(div.map(|s| s.padding), Some(Edges::all(2.0)));
    }

    #[test]
    fn important_declarations_win_over_inline_styles() {
        let html = "<style>div { width: 10px !important; height: 5px }</style>\
                    <div style='width: 20px; height: 6px'></div>";
        let div = style_of(html, "div");
        assert_eq!(div.map(|s| s.width), Some(Some(Length::Px(10.0))));
        assert_eq!(div.map(|s| s.height), Some(Some(Length::Px(6.0))));
    }

    #[test]
    fn box_shorthands_expand_and_invalid_values_are_ignored() {
        let html = "<div style='margin: 1px auto 3px; padding: 5px; padding: -1px; \
                    padding-left: 2vw; width: 50%; margin-left: -4px'></div>";
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
        assert_eq!(div.map(|s| s.width), Some(Some(Length::Percent(50.0))));
    }

    #[test]
    fn relative_units_resolve_against_the_font_size() {
        let html = "<div style='padding: 2em; margin-left: 12pt; font-size: 10px'></div>";
        let div = style_of(html, "div");
        assert_eq!(div.map(|s| s.padding.left), Some(20.0));
        assert_eq!(div.map(|s| s.margin.left), Some(Some(16.0)));
    }

    #[test]
    fn the_font_shorthand_sets_size_weight_and_line_height() {
        let div = style_of(
            "<div style='font: bold 13px/27px Roboto, Arial, sans-serif'></div>",
            "div",
        );
        assert_eq!(div.map(|s| (s.font_size, s.bold)), Some((13.0, true)));
        assert_eq!(div.map(|s| s.line_height), Some(LineHeight::Px(27.0)));
    }

    #[test]
    fn borders_need_a_style_and_default_to_the_text_color() {
        let unstyled = style_of("<div style='border-width: 4px'></div>", "div");
        assert_eq!(unstyled.map(|s| s.border()), Some(Edges::all(0.0)));

        let medium = style_of("<div style='color: red; border-style: solid'></div>", "div");
        assert_eq!(medium.map(|s| s.border()), Some(Edges::all(3.0)));
        assert_eq!(
            medium.map(|s| s.used_border_color()),
            Some(Edges::all(Color::rgb(255, 0, 0)))
        );

        let shorthand = style_of(
            "<div style='border: rgb(0, 0, 255) 2px solid'></div>",
            "div",
        );
        assert_eq!(shorthand.map(|s| s.border()), Some(Edges::all(2.0)));

        let sides = style_of(
            "<div style='border: solid 1px; border-color: red blue; border-left: none; border-top-width: 5px'></div>",
            "div",
        );
        assert_eq!(
            sides.map(|s| s.border()),
            Some(Edges {
                top: 5.0,
                right: 1.0,
                bottom: 1.0,
                left: 0.0
            })
        );
        assert_eq!(
            sides.map(|s| s.used_border_color().right),
            Some(Color::rgb(0, 0, 255))
        );
    }

    #[test]
    fn text_properties_inherit() {
        let html = "<div style='color: #00f; text-align: center; font-size: 20px'><section></section></div>";
        let section = style_of(html, "section");
        assert_eq!(section.map(|s| s.color), Some(Color::rgb(0, 0, 255)));
        assert_eq!(section.map(|s| s.text_align), Some(TextAlign::Center));
        assert_eq!(section.map(|s| s.font_size), Some(20.0));
    }

    #[test]
    fn legacy_attributes_become_styles() {
        let body = style_of("<body bgcolor='fff'>", "body");
        assert_eq!(body.map(|s| s.background), Some(Color::WHITE));
        let cell = style_of(
            "<table><tr valign=top><td width='25%' align=center nowrap></td></tr></table>",
            "td",
        );
        assert_eq!(cell.map(|s| s.width), Some(Some(Length::Percent(25.0))));
        assert_eq!(
            cell.map(|s| (s.text_align, s.nowrap)),
            Some((TextAlign::Center, true))
        );
        assert_eq!(cell.map(|s| s.vertical_align), Some(VerticalAlign::Top));
        let hidden = style_of("<input type=hidden>", "input");
        assert_eq!(hidden.map(|s| s.display), Some(Display::None));
    }
}
