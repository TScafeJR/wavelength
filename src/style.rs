//! The cascade: turns stylesheets, `style` attributes and legacy HTML
//! attributes into one computed style per node.
//!
//! Declarations are sorted by importance, origin, specificity and source
//! order. Custom properties are collected first so that `var()` can be
//! substituted into the declarations that use them, then font sizes are
//! resolved so that `em` lengths have something to refer to.

use std::collections::HashMap;
use std::rc::Rc;

use crate::css::{self, Media, Specificity, split_top_level};
use crate::dom::{Document, Element, NodeId};
use crate::font::Face;

/// The built-in stylesheet. It covers only what the engine can lay out today.
const USER_AGENT_CSS: &str = "
html, body, div, section, article, header, footer, main, nav, aside, p, h1, h2, h3, h4, h5, h6,
ul, ol, dl, dt, dd, form, center, blockquote, pre, hr, address, figure, figcaption, fieldset,
details, summary, thead, tbody, tfoot, caption, hgroup, search, menu, legend { display: block }
li { display: list-item }
head, base, link, meta, script, style, title, template, datalist, option, svg, area, param,
source, track, dialog, iframe, video, audio, canvas, object, embed, [hidden] { display: none }
body { margin: 8px }
p, dl, ul, ol, menu, pre, figure { margin: 1em 0 }
blockquote { margin: 1em 40px }
ul, ol, menu { padding-left: 40px }
ul ul, ul ol, ol ul, ol ol { margin: 0 }
dd { margin-left: 40px }
h1 { font-size: 2em; margin: 0.67em 0; font-weight: bold }
h2 { font-size: 1.5em; margin: 0.83em 0; font-weight: bold }
h3 { font-size: 1.17em; margin: 1em 0; font-weight: bold }
h4 { margin: 1.33em 0; font-weight: bold }
h5 { font-size: 0.83em; margin: 1.67em 0; font-weight: bold }
h6 { font-size: 0.67em; margin: 2.33em 0; font-weight: bold }
b, strong, th { font-weight: bold }
i, em, cite, var, dfn, address { font-style: italic }
small { font-size: smaller }
big { font-size: larger }
mark { background: #ff0; color: #000 }
a:link { color: #0000ee; text-decoration: underline }
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
  font-style: normal; color: #000; line-height: normal; text-align: left; text-decoration: none;
  text-transform: none; white-space: nowrap }
input, textarea, select { background: #fff; border: 1px solid #767676; padding: 1px 2px }
button, input[type=submit i], input[type=button i], input[type=reset i] {
  background: #efefef; border: 1px solid #767676; padding: 1px 6px; text-align: center }
input[type=hidden i] { display: none }
";

/// The initial value of `border-width`, which CSS calls `medium`.
const MEDIUM_BORDER: f32 = 3.0;
/// The initial `font-size`, which CSS calls `medium`.
const DEFAULT_FONT_SIZE: f32 = 16.0;
/// How many `var()` references may be followed before giving up on a cycle.
const MAX_VARIABLE_DEPTH: u32 = 16;

/// The CSS named colours, sorted by name, as 0xRRGGBB.
const NAMED_COLORS: &[(&str, u32)] = &[
    ("aliceblue", 0xF0F8FF),
    ("antiquewhite", 0xFAEBD7),
    ("aqua", 0x00FFFF),
    ("aquamarine", 0x7FFFD4),
    ("azure", 0xF0FFFF),
    ("beige", 0xF5F5DC),
    ("bisque", 0xFFE4C4),
    ("black", 0x000000),
    ("blanchedalmond", 0xFFEBCD),
    ("blue", 0x0000FF),
    ("blueviolet", 0x8A2BE2),
    ("brown", 0xA52A2A),
    ("burlywood", 0xDEB887),
    ("cadetblue", 0x5F9EA0),
    ("chartreuse", 0x7FFF00),
    ("chocolate", 0xD2691E),
    ("coral", 0xFF7F50),
    ("cornflowerblue", 0x6495ED),
    ("cornsilk", 0xFFF8DC),
    ("crimson", 0xDC143C),
    ("cyan", 0x00FFFF),
    ("darkblue", 0x00008B),
    ("darkcyan", 0x008B8B),
    ("darkgoldenrod", 0xB8860B),
    ("darkgray", 0xA9A9A9),
    ("darkgreen", 0x006400),
    ("darkgrey", 0xA9A9A9),
    ("darkkhaki", 0xBDB76B),
    ("darkmagenta", 0x8B008B),
    ("darkolivegreen", 0x556B2F),
    ("darkorange", 0xFF8C00),
    ("darkorchid", 0x9932CC),
    ("darkred", 0x8B0000),
    ("darksalmon", 0xE9967A),
    ("darkseagreen", 0x8FBC8F),
    ("darkslateblue", 0x483D8B),
    ("darkslategray", 0x2F4F4F),
    ("darkslategrey", 0x2F4F4F),
    ("darkturquoise", 0x00CED1),
    ("darkviolet", 0x9400D3),
    ("deeppink", 0xFF1493),
    ("deepskyblue", 0x00BFFF),
    ("dimgray", 0x696969),
    ("dimgrey", 0x696969),
    ("dodgerblue", 0x1E90FF),
    ("firebrick", 0xB22222),
    ("floralwhite", 0xFFFAF0),
    ("forestgreen", 0x228B22),
    ("fuchsia", 0xFF00FF),
    ("gainsboro", 0xDCDCDC),
    ("ghostwhite", 0xF8F8FF),
    ("gold", 0xFFD700),
    ("goldenrod", 0xDAA520),
    ("gray", 0x808080),
    ("green", 0x008000),
    ("greenyellow", 0xADFF2F),
    ("grey", 0x808080),
    ("honeydew", 0xF0FFF0),
    ("hotpink", 0xFF69B4),
    ("indianred", 0xCD5C5C),
    ("indigo", 0x4B0082),
    ("ivory", 0xFFFFF0),
    ("khaki", 0xF0E68C),
    ("lavender", 0xE6E6FA),
    ("lavenderblush", 0xFFF0F5),
    ("lawngreen", 0x7CFC00),
    ("lemonchiffon", 0xFFFACD),
    ("lightblue", 0xADD8E6),
    ("lightcoral", 0xF08080),
    ("lightcyan", 0xE0FFFF),
    ("lightgoldenrodyellow", 0xFAFAD2),
    ("lightgray", 0xD3D3D3),
    ("lightgreen", 0x90EE90),
    ("lightgrey", 0xD3D3D3),
    ("lightpink", 0xFFB6C1),
    ("lightsalmon", 0xFFA07A),
    ("lightseagreen", 0x20B2AA),
    ("lightskyblue", 0x87CEFA),
    ("lightslategray", 0x778899),
    ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xB0C4DE),
    ("lightyellow", 0xFFFFE0),
    ("lime", 0x00FF00),
    ("limegreen", 0x32CD32),
    ("linen", 0xFAF0E6),
    ("magenta", 0xFF00FF),
    ("maroon", 0x800000),
    ("mediumaquamarine", 0x66CDAA),
    ("mediumblue", 0x0000CD),
    ("mediumorchid", 0xBA55D3),
    ("mediumpurple", 0x9370DB),
    ("mediumseagreen", 0x3CB371),
    ("mediumslateblue", 0x7B68EE),
    ("mediumspringgreen", 0x00FA9A),
    ("mediumturquoise", 0x48D1CC),
    ("mediumvioletred", 0xC71585),
    ("midnightblue", 0x191970),
    ("mintcream", 0xF5FFFA),
    ("mistyrose", 0xFFE4E1),
    ("moccasin", 0xFFE4B5),
    ("navajowhite", 0xFFDEAD),
    ("navy", 0x000080),
    ("oldlace", 0xFDF5E6),
    ("olive", 0x808000),
    ("olivedrab", 0x6B8E23),
    ("orange", 0xFFA500),
    ("orangered", 0xFF4500),
    ("orchid", 0xDA70D6),
    ("palegoldenrod", 0xEEE8AA),
    ("palegreen", 0x98FB98),
    ("paleturquoise", 0xAFEEEE),
    ("palevioletred", 0xDB7093),
    ("papayawhip", 0xFFEFD5),
    ("peachpuff", 0xFFDAB9),
    ("peru", 0xCD853F),
    ("pink", 0xFFC0CB),
    ("plum", 0xDDA0DD),
    ("powderblue", 0xB0E0E6),
    ("purple", 0x800080),
    ("rebeccapurple", 0x663399),
    ("red", 0xFF0000),
    ("rosybrown", 0xBC8F8F),
    ("royalblue", 0x4169E1),
    ("saddlebrown", 0x8B4513),
    ("salmon", 0xFA8072),
    ("sandybrown", 0xF4A460),
    ("seagreen", 0x2E8B57),
    ("seashell", 0xFFF5EE),
    ("sienna", 0xA0522D),
    ("silver", 0xC0C0C0),
    ("skyblue", 0x87CEEB),
    ("slateblue", 0x6A5ACD),
    ("slategray", 0x708090),
    ("slategrey", 0x708090),
    ("snow", 0xFFFAFA),
    ("springgreen", 0x00FF7F),
    ("steelblue", 0x4682B4),
    ("tan", 0xD2B48C),
    ("teal", 0x008080),
    ("thistle", 0xD8BFD8),
    ("tomato", 0xFF6347),
    ("turquoise", 0x40E0D0),
    ("violet", 0xEE82EE),
    ("wheat", 0xF5DEB3),
    ("white", 0xFFFFFF),
    ("whitesmoke", 0xF5F5F5),
    ("yellow", 0xFFFF00),
    ("yellowgreen", 0x9ACD32),
];

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

    /// Parses a lowercased CSS colour: hex notation, `rgb()`, `hsl()` or a colour name.
    pub fn parse(value: &str) -> Option<Self> {
        if let Some(hex) = value.strip_prefix('#') {
            return Self::parse_hex(hex);
        }
        if let Some((function, arguments)) = value.strip_suffix(')').and_then(|v| v.split_once('('))
        {
            let parts: Vec<&str> = arguments
                .split(|c: char| c == ',' || c == '/' || c.is_ascii_whitespace())
                .filter(|part| !part.is_empty())
                .collect();
            let number = |part: &str| part.parse::<f32>().ok().filter(|n| n.is_finite());
            let fraction = |part: &str| match part.strip_suffix('%') {
                Some(percent) => number(percent).map(|p| (p / 100.0).clamp(0.0, 1.0)),
                None => number(part).map(|n| n.clamp(0.0, 1.0)),
            };
            let alpha = match parts.get(3) {
                Some(part) => (fraction(part)? * 255.0).round() as u8,
                None => 255,
            };
            if !(3..=4).contains(&parts.len()) {
                return None;
            }
            return match function {
                "rgb" | "rgba" => {
                    let channel = |part: &str| match part.strip_suffix('%') {
                        Some(percent) => {
                            number(percent).map(|p| (p * 2.55).round().clamp(0.0, 255.0) as u8)
                        }
                        None => number(part).map(|n| n.round().clamp(0.0, 255.0) as u8),
                    };
                    Some(Self {
                        r: channel(parts[0])?,
                        g: channel(parts[1])?,
                        b: channel(parts[2])?,
                        a: alpha,
                    })
                }
                "hsl" | "hsla" => {
                    let hue = number(parts[0].trim_end_matches("deg"))?;
                    let (r, g, b) = hsl_to_rgb(hue, fraction(parts[1])?, fraction(parts[2])?);
                    Some(Self { r, g, b, a: alpha })
                }
                _ => None,
            };
        }
        if value == "transparent" {
            return Some(Self::TRANSPARENT);
        }
        let index = NAMED_COLORS
            .binary_search_by(|(name, _)| name.cmp(&value))
            .ok()?;
        let [_, r, g, b] = NAMED_COLORS[index].1.to_be_bytes();
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

fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> (u8, u8, u8) {
    let hue = hue.rem_euclid(360.0) / 30.0;
    let amplitude = saturation * lightness.min(1.0 - lightness);
    let channel = |offset: f32| {
        let k = (offset + hue) % 12.0;
        let value = lightness - amplitude * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0);
        (value * 255.0).round().clamp(0.0, 255.0) as u8
    };
    (channel(0.0), channel(8.0), channel(4.0))
}

/// How an element takes part in layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    /// A block-level box. Grid containers are laid out as blocks for now.
    Block,
    /// An inline box, whose content flows into its parent's lines.
    Inline,
    /// A box that sits on a line but is laid out inside as a block.
    InlineBlock,
    /// A flex container.
    Flex,
    /// A table.
    Table,
    /// A table row.
    TableRow,
    /// A table cell.
    TableCell,
    /// No box at all.
    None,
}

/// A size that may depend on the containing block: `percent` of it plus `px`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Length {
    /// The share of the containing block's size, where 100 is all of it.
    pub percent: f32,
    /// A fixed number of CSS pixels.
    pub px: f32,
}

impl Length {
    /// A fixed length in pixels.
    pub const fn px(px: f32) -> Self {
        Self { percent: 0.0, px }
    }

    /// A percentage of the containing block.
    pub const fn percent(percent: f32) -> Self {
        Self { percent, px: 0.0 }
    }

    /// Whether the length is the same whatever it is measured against.
    pub fn is_fixed(self) -> bool {
        self.percent == 0.0
    }

    /// The length in pixels, given the size percentages refer to.
    pub fn resolve(self, containing: f32) -> f32 {
        (self.percent * containing / 100.0 + self.px).max(0.0)
    }
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

/// A change of letter case applied to text before it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextTransform {
    /// Text is drawn as written.
    None,
    /// Every letter is made upper case.
    Uppercase,
    /// Every letter is made lower case.
    Lowercase,
    /// The first letter of each word is made upper case.
    Capitalize,
}

/// How a flex container spreads its items along the main axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Justify {
    /// Packed at the start.
    Start,
    /// Packed in the middle.
    Center,
    /// Packed at the end.
    End,
    /// Spread out, with the first and last items at the edges.
    SpaceBetween,
    /// Spread out, with half a gap at each edge.
    SpaceAround,
    /// Spread out, with equal gaps everywhere.
    SpaceEvenly,
}

/// How a flex container places its items across the main axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignItems {
    /// Items fill the line.
    Stretch,
    /// Items sit at the start.
    Start,
    /// Items are centred.
    Center,
    /// Items sit at the end.
    End,
}

/// A corner radius.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Radius {
    /// A radius in pixels.
    Px(f32),
    /// A percentage of the box's size, where 50 turns a square into a circle.
    Percent(f32),
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

/// The computed style of one node. Lengths are in CSS pixels and `None` means `auto` or `none`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// The `display` property.
    pub display: Display,
    /// Whether the box is a list item, which draws a marker beside it.
    pub list_item: bool,
    /// Whether the box is absolutely positioned. Such boxes are not laid out yet.
    pub out_of_flow: bool,
    /// The `width`.
    pub width: Option<Length>,
    /// The `height`.
    pub height: Option<Length>,
    /// The `min-width` in pixels.
    pub min_width: f32,
    /// The `min-height` in pixels.
    pub min_height: f32,
    /// The `max-width`.
    pub max_width: Option<Length>,
    /// The `max-height` in pixels.
    pub max_height: Option<f32>,
    /// Whether `width` and `height` include padding and borders (`box-sizing: border-box`).
    pub border_box: bool,
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
    /// The `border-radius`, applied to all four corners.
    pub border_radius: Radius,
    /// The `background-color`.
    pub background: Color,
    /// How opaque the box and everything in it is, already multiplied with its ancestors' opacity.
    pub opacity: f32,
    /// Where a table cell's content sits vertically.
    pub vertical_align: VerticalAlign,
    /// Whether a flex container's main axis runs down the page.
    pub flex_column: bool,
    /// Whether a flex container breaks its items onto several lines.
    pub flex_wrap: bool,
    /// The `justify-content` of a flex container.
    pub justify: Justify,
    /// The `align-items` of a flex container.
    pub align_items: AlignItems,
    /// The gaps between a flex container's lines and between the items on a line.
    pub gap: (f32, f32),
    /// The `flex-grow` of a flex item.
    pub flex_grow: f32,
    /// Whether a flex item starts from nothing (`flex-basis: 0`) rather than from its content's width.
    pub flex_basis_zero: bool,
    /// The foreground `color`. Inherited.
    pub color: Color,
    /// The `font-size` in pixels. Inherited.
    pub font_size: f32,
    /// Whether `font-weight` is bold. Inherited.
    pub bold: bool,
    /// Whether `font-style` is italic. Inherited.
    pub italic: bool,
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
    /// The `text-transform`. Inherited.
    pub text_transform: TextTransform,
    /// Whether list items draw their marker. Inherited.
    pub list_marker: bool,
    /// Whether the box is painted (`visibility`). Inherited.
    pub visible: bool,
}

impl Style {
    fn initial(parent: Option<&Style>) -> Self {
        Self {
            display: Display::Inline,
            list_item: false,
            out_of_flow: false,
            width: None,
            height: None,
            min_width: 0.0,
            min_height: 0.0,
            max_width: None,
            max_height: None,
            border_box: false,
            margin: Edges::all(Some(0.0)),
            padding: Edges::all(0.0),
            border_width: Edges::all(MEDIUM_BORDER),
            border_solid: Edges::all(false),
            border_color: Edges::all(None),
            border_radius: Radius::Px(0.0),
            background: Color::TRANSPARENT,
            opacity: parent.map_or(1.0, |p| p.opacity),
            vertical_align: VerticalAlign::Baseline,
            flex_column: false,
            flex_wrap: false,
            justify: Justify::Start,
            align_items: AlignItems::Stretch,
            gap: (0.0, 0.0),
            flex_grow: 0.0,
            flex_basis_zero: false,
            color: parent.map_or(Color::BLACK, |p| p.color),
            font_size: parent.map_or(DEFAULT_FONT_SIZE, |p| p.font_size),
            bold: parent.is_some_and(|p| p.bold),
            italic: parent.is_some_and(|p| p.italic),
            line_height: parent.map_or(LineHeight::Normal, |p| p.line_height),
            text_align: parent.map_or(TextAlign::Left, |p| p.text_align),
            center_blocks: parent.is_some_and(|p| p.center_blocks),
            underline: parent.is_some_and(|p| p.underline),
            nowrap: parent.is_some_and(|p| p.nowrap),
            text_transform: parent.map_or(TextTransform::None, |p| p.text_transform),
            list_marker: parent.is_none_or(|p| p.list_marker),
            visible: parent.is_none_or(|p| p.visible),
        }
    }

    /// The style of an anonymous box that wraps part of this box's content:
    /// the inherited properties carry over and everything else is reset.
    pub fn anonymous(&self) -> Self {
        Self {
            display: Display::Block,
            ..Self::initial(Some(self))
        }
    }

    /// The font face this style's text is drawn in.
    pub fn face(&self) -> Face {
        Face {
            bold: self.bold,
            italic: self.italic,
        }
    }

    /// The colour text is drawn in, after `opacity` and `visibility` are applied.
    pub fn text_color(&self) -> Color {
        self.fade(self.color)
    }

    /// Applies this box's `opacity` and `visibility` to one of its colours.
    pub fn fade(&self, color: Color) -> Color {
        let alpha = if self.visible {
            f32::from(color.a) * self.opacity
        } else {
            0.0
        };
        Color {
            a: alpha.round().clamp(0.0, 255.0) as u8,
            ..color
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

    /// The corner radius in pixels for a box of the given size.
    pub fn used_radius(&self, width: f32, height: f32) -> f32 {
        let radius = match self.border_radius {
            Radius::Px(px) => px,
            Radius::Percent(percent) => width.min(height) * percent / 100.0,
        };
        radius.clamp(0.0, width.min(height) / 2.0)
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
    fn apply_font_size(&mut self, name: &str, value: &str, parent: &Style, viewport: (f32, f32)) {
        let units = Units {
            em: parent.font_size,
            viewport,
        };
        match (name, value) {
            ("font-size" | "font", "inherit" | "unset") => self.copy_property(name, parent),
            ("font-size" | "font", "initial") => self.copy_property(name, &Self::initial(None)),
            ("font-size", _) => set(&mut self.font_size, units.font_size(value)),
            ("font", _) => self.apply_font(value, units),
            _ => {}
        }
    }

    /// The `font` shorthand: optional style and weight, a size with an optional line height, then families.
    fn apply_font(&mut self, value: &str, units: Units) {
        let (mut bold, mut italic) = (false, false);
        for token in tokens(value) {
            let (size_part, line_part) = token.split_once('/').unwrap_or((token, ""));
            if let Some(size) = units.font_size(size_part) {
                let own = Units { em: size, ..units };
                self.font_size = size;
                self.bold = bold;
                self.italic = italic;
                self.line_height = own.line_height(line_part).unwrap_or(LineHeight::Normal);
                return;
            }
            match token {
                "italic" | "oblique" => italic = true,
                "small-caps" => {}
                _ => match font_weight(token) {
                    Some(weight) => bold = weight,
                    None => return,
                },
            }
        }
    }

    /// Copies one property, or the group a shorthand stands for, from another style.
    fn copy_property(&mut self, name: &str, from: &Style) {
        match name {
            "color" => self.color = from.color,
            "background" | "background-color" => self.background = from.background,
            "font-size" => self.font_size = from.font_size,
            "font-weight" => self.bold = from.bold,
            "font-style" => self.italic = from.italic,
            "font" => {
                self.font_size = from.font_size;
                self.bold = from.bold;
                self.italic = from.italic;
                self.line_height = from.line_height;
            }
            "line-height" => self.line_height = from.line_height,
            "text-align" => {
                self.text_align = from.text_align;
                self.center_blocks = from.center_blocks;
            }
            "text-decoration" | "text-decoration-line" => self.underline = from.underline,
            "text-transform" => self.text_transform = from.text_transform,
            "white-space" => self.nowrap = from.nowrap,
            "visibility" => self.visible = from.visible,
            "list-style" | "list-style-type" => self.list_marker = from.list_marker,
            "display" => {
                self.display = from.display;
                self.list_item = from.list_item;
            }
            "width" => self.width = from.width,
            "height" => self.height = from.height,
            "min-width" => self.min_width = from.min_width,
            "min-height" => self.min_height = from.min_height,
            "max-width" => self.max_width = from.max_width,
            "max-height" => self.max_height = from.max_height,
            "box-sizing" => self.border_box = from.border_box,
            "margin" => self.margin = from.margin,
            "padding" => self.padding = from.padding,
            "border" => {
                self.border_width = from.border_width;
                self.border_solid = from.border_solid;
                self.border_color = from.border_color;
            }
            "border-width" => self.border_width = from.border_width,
            "border-style" => self.border_solid = from.border_solid,
            "border-color" => self.border_color = from.border_color,
            "border-radius" => self.border_radius = from.border_radius,
            "vertical-align" => self.vertical_align = from.vertical_align,
            _ => {
                let (mut margin, mut padding) = (from.margin, from.padding);
                if let Some(side) = name.strip_prefix("margin-") {
                    set_side(&mut self.margin, side, margin.side_mut(side).copied());
                } else if let Some(side) = name.strip_prefix("padding-") {
                    set_side(&mut self.padding, side, padding.side_mut(side).copied());
                }
            }
        }
    }

    /// Applies one declaration. Unknown properties and invalid values leave the style untouched.
    fn apply(&mut self, name: &str, value: &str, parent: &Style, viewport: (f32, f32)) {
        const INHERITED: &[&str] = &[
            "color",
            "font-size",
            "font-weight",
            "font-style",
            "font",
            "line-height",
            "text-align",
            "text-transform",
            "white-space",
            "visibility",
            "list-style",
            "list-style-type",
        ];
        match value {
            "inherit" => return self.copy_property(name, parent),
            "initial" => return self.copy_property(name, &Self::initial(None)),
            "unset" if INHERITED.contains(&name) => return self.copy_property(name, parent),
            "unset" => return self.copy_property(name, &Self::initial(None)),
            _ => {}
        }
        let units = Units {
            em: self.font_size,
            viewport,
        };
        let non_negative = |v: &str| units.length(v).filter(|n| *n >= 0.0);
        let margin = |v: &str| {
            if v == "auto" {
                Some(None)
            } else {
                units.length(v).map(Some)
            }
        };
        let size = |v: &str| match v {
            "auto" | "fit-content" | "max-content" | "min-content" => Some(None),
            _ => units.length_or_percent(v).map(Some),
        };
        match name {
            "display" => {
                if let Some(display) = display(value) {
                    self.display = display;
                    self.list_item = value == "list-item";
                }
            }
            "position" => match value {
                "absolute" => self.out_of_flow = true,
                // Fixed and sticky boxes are kept in the flow, which at least keeps them visible.
                "static" | "relative" | "fixed" | "sticky" => self.out_of_flow = false,
                _ => {}
            },
            "width" => set(&mut self.width, size(value)),
            "height" => set(&mut self.height, size(value)),
            "min-width" => set(
                &mut self.min_width,
                if value == "auto" {
                    Some(0.0)
                } else {
                    non_negative(value)
                },
            ),
            "min-height" => set(
                &mut self.min_height,
                if value == "auto" {
                    Some(0.0)
                } else {
                    non_negative(value)
                },
            ),
            "max-width" => set(
                &mut self.max_width,
                if value == "none" {
                    Some(None)
                } else {
                    size(value)
                },
            ),
            "max-height" => {
                set(
                    &mut self.max_height,
                    if value == "none" {
                        Some(None)
                    } else {
                        non_negative(value).map(Some)
                    },
                );
            }
            "box-sizing" => match value {
                "border-box" => self.border_box = true,
                "content-box" => self.border_box = false,
                _ => {}
            },
            "margin" => set(&mut self.margin, sides(value, margin)),
            "padding" => set(&mut self.padding, sides(value, non_negative)),
            "border-width" => set(
                &mut self.border_width,
                sides(value, |v| units.border_width(v)),
            ),
            "border-style" => set(&mut self.border_solid, sides(value, border_style)),
            "border-color" => {
                let colors = sides(value, |v| self.parse_color(v).map(Some));
                set(&mut self.border_color, colors);
            }
            "border" => {
                if let Some((width, solid, color)) = self.border_shorthand(value, units) {
                    self.border_width = Edges::all(width);
                    self.border_solid = Edges::all(solid);
                    self.border_color = Edges::all(color);
                }
            }
            "border-radius" => {
                // Corners are not rounded separately yet, so the first value stands for all four.
                let first = tokens(value).into_iter().next().unwrap_or_default();
                let radius = match first.strip_suffix('%') {
                    Some(percent) => percent
                        .parse::<f32>()
                        .ok()
                        .filter(|p| *p >= 0.0)
                        .map(Radius::Percent),
                    None => non_negative(first).map(Radius::Px),
                };
                set(&mut self.border_radius, radius);
            }
            "color" => {
                let color = self.parse_color(value);
                set(&mut self.color, color);
            }
            "background-color" => {
                let color = self.parse_color(value);
                set(&mut self.background, color);
            }
            "background" => {
                // The shorthand may also carry images and positions, which are skipped.
                let color = tokens(value)
                    .into_iter()
                    .find_map(|token| self.parse_color(token));
                set(
                    &mut self.background,
                    color.or((value == "none").then_some(Color::TRANSPARENT)),
                );
            }
            "opacity" => {
                let own = match value.strip_suffix('%') {
                    Some(percent) => percent.parse::<f32>().ok().map(|p| p / 100.0),
                    None => value.parse::<f32>().ok(),
                };
                if let Some(own) = own.filter(|o| o.is_finite()) {
                    self.opacity = parent.opacity * own.clamp(0.0, 1.0);
                }
            }
            "visibility" => match value {
                "visible" => self.visible = true,
                "hidden" | "collapse" => self.visible = false,
                _ => {}
            },
            "font-weight" => set(&mut self.bold, font_weight(value)),
            "font-style" => match value {
                "italic" | "oblique" => self.italic = true,
                "normal" => self.italic = false,
                _ => {}
            },
            "line-height" => set(&mut self.line_height, units.line_height(value)),
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
            "text-transform" => match value {
                "uppercase" => self.text_transform = TextTransform::Uppercase,
                "lowercase" => self.text_transform = TextTransform::Lowercase,
                "capitalize" => self.text_transform = TextTransform::Capitalize,
                "none" => self.text_transform = TextTransform::None,
                _ => {}
            },
            "white-space" => match value {
                "nowrap" | "pre" => self.nowrap = true,
                "normal" | "pre-wrap" | "pre-line" | "break-spaces" => self.nowrap = false,
                _ => {}
            },
            "list-style" | "list-style-type" => {
                self.list_marker = !tokens(value).contains(&"none");
            }
            "vertical-align" => match value {
                "top" | "text-top" => self.vertical_align = VerticalAlign::Top,
                "middle" => self.vertical_align = VerticalAlign::Middle,
                "bottom" | "text-bottom" => self.vertical_align = VerticalAlign::Bottom,
                "baseline" => self.vertical_align = VerticalAlign::Baseline,
                _ => {}
            },
            "flex-direction" => match value {
                "row" | "row-reverse" => self.flex_column = false,
                "column" | "column-reverse" => self.flex_column = true,
                _ => {}
            },
            "flex-wrap" => match value {
                "wrap" | "wrap-reverse" => self.flex_wrap = true,
                "nowrap" => self.flex_wrap = false,
                _ => {}
            },
            "flex-flow" => {
                for token in tokens(value) {
                    self.apply("flex-direction", token, parent, viewport);
                    self.apply("flex-wrap", token, parent, viewport);
                }
            }
            "justify-content" => set(&mut self.justify, justify(value)),
            "align-items" => set(&mut self.align_items, align_items(value)),
            "gap" | "grid-gap" => {
                let parsed: Option<Vec<f32>> =
                    tokens(value).into_iter().map(non_negative).collect();
                match parsed.as_deref() {
                    Some([both]) => self.gap = (*both, *both),
                    Some([row, column]) => self.gap = (*row, *column),
                    _ => {}
                }
            }
            "row-gap" => set(&mut self.gap.0, non_negative(value)),
            "column-gap" => set(&mut self.gap.1, non_negative(value)),
            "flex-grow" => set(
                &mut self.flex_grow,
                value.parse::<f32>().ok().filter(|g| *g >= 0.0),
            ),
            "flex-basis" => {
                self.flex_basis_zero =
                    matches!(units.quantity(value), Some(q) if q.px == 0.0 && q.percent == 0.0);
            }
            "flex" => {
                // `flex: <grow> [<shrink>] [<basis>]`, where a missing basis means zero.
                let parts = tokens(value);
                let first = parts.first().copied().unwrap_or_default();
                match first {
                    "none" | "initial" => (self.flex_grow, self.flex_basis_zero) = (0.0, false),
                    "auto" => (self.flex_grow, self.flex_basis_zero) = (1.0, false),
                    _ => {
                        if let Some(grow) = first.parse::<f32>().ok().filter(|g| *g >= 0.0) {
                            let basis = parts
                                .iter()
                                .skip(1)
                                .find(|part| part.parse::<f32>().is_err());
                            let zero = |part: &&str| matches!(units.quantity(part), Some(q) if q.px == 0.0 && q.percent == 0.0);
                            self.flex_grow = grow;
                            self.flex_basis_zero = basis.is_none_or(zero);
                        }
                    }
                }
            }
            _ => self.apply_side(name, value, units),
        }
    }

    fn set_align(&mut self, align: TextAlign, center_blocks: bool) {
        self.text_align = align;
        self.center_blocks = center_blocks;
    }

    /// A colour value, where `currentcolor` means this element's own `color`.
    fn parse_color(&self, value: &str) -> Option<Color> {
        if value == "currentcolor" {
            return Some(self.color);
        }
        Color::parse(value)
    }

    /// A border shorthand: a width, a style and a colour in any order, each optional.
    fn border_shorthand(&self, value: &str, units: Units) -> Option<(f32, bool, Option<Color>)> {
        let (mut width, mut solid, mut color) = (MEDIUM_BORDER, false, None);
        for token in tokens(value) {
            if let Some(parsed) = units.border_width(token) {
                width = parsed;
            } else if let Some(parsed) = border_style(token) {
                solid = parsed;
            } else {
                color = Some(self.parse_color(token)?);
            }
        }
        Some((width, solid, color))
    }

    /// Longhands that name one side, such as `margin-left` or `border-top-color`.
    fn apply_side(&mut self, name: &str, value: &str, units: Units) {
        if let Some(side) = name.strip_prefix("margin-") {
            let parsed = if value == "auto" {
                Some(None)
            } else {
                units.length(value).map(Some)
            };
            set_side(&mut self.margin, side, parsed);
        } else if let Some(side) = name.strip_prefix("padding-") {
            set_side(
                &mut self.padding,
                side,
                units.length(value).filter(|n| *n >= 0.0),
            );
        } else if let Some(rest) = name.strip_prefix("border-") {
            let (side, part) = rest.split_once('-').unwrap_or((rest, ""));
            match part {
                "" => {
                    if let Some((width, solid, color)) = self.border_shorthand(value, units) {
                        set_side(&mut self.border_width, side, Some(width));
                        set_side(&mut self.border_solid, side, Some(solid));
                        set_side(&mut self.border_color, side, Some(color));
                    }
                }
                "width" => set_side(&mut self.border_width, side, units.border_width(value)),
                "style" => set_side(&mut self.border_solid, side, border_style(value)),
                "color" => {
                    let color = self.parse_color(value).map(Some);
                    set_side(&mut self.border_color, side, color);
                }
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

/// What relative units resolve against.
#[derive(Debug, Clone, Copy)]
struct Units {
    /// The font size that `em` refers to.
    em: f32,
    /// The viewport's width and height.
    viewport: (f32, f32),
}

/// A value inside `calc()`: pixels plus a percentage, or a plain number.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Quantity {
    px: f32,
    percent: f32,
    /// Whether this is a unitless number rather than a length.
    number: bool,
}

impl Units {
    /// A number with a unit, as (pixels, percent). A bare zero is a length too.
    fn dimension(&self, value: &str) -> Option<Quantity> {
        let split = value
            .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+')))
            .unwrap_or(value.len());
        let (number, unit) = value.split_at(split);
        let number = number.parse::<f32>().ok().filter(|n| n.is_finite())?;
        let (width, height) = self.viewport;
        let px = match unit {
            "" => {
                return Some(Quantity {
                    px: number,
                    percent: 0.0,
                    number: true,
                });
            }
            "%" => {
                return Some(Quantity {
                    px: 0.0,
                    percent: number,
                    number: false,
                });
            }
            "px" => number,
            "pt" => number * 4.0 / 3.0,
            "pc" => number * 16.0,
            "in" => number * 96.0,
            "cm" => number * 96.0 / 2.54,
            "mm" => number * 96.0 / 25.4,
            "em" => number * self.em,
            "rem" => number * DEFAULT_FONT_SIZE,
            "ex" | "ch" => number * self.em * 0.5,
            "vw" | "dvw" | "svw" | "lvw" => number * width / 100.0,
            "vh" | "dvh" | "svh" | "lvh" => number * height / 100.0,
            "vmin" => number * width.min(height) / 100.0,
            "vmax" => number * width.max(height) / 100.0,
            _ => return None,
        };
        Some(Quantity {
            px,
            percent: 0.0,
            number: false,
        })
    }

    /// Evaluates a length that may be a plain dimension or a math function.
    fn quantity(&self, value: &str) -> Option<Quantity> {
        if !value.contains('(') {
            return self.dimension(value);
        }
        let mut parser = MathParser {
            units: self,
            text: value,
            position: 0,
        };
        let result = parser.sum()?;
        parser.skip_whitespace();
        (parser.position == value.len()).then_some(result)
    }

    /// A length in pixels. Percentages are not accepted, and without a unit only zero is.
    fn length(&self, value: &str) -> Option<f32> {
        let quantity = self.quantity(value)?;
        let valid = quantity.percent == 0.0 && (!quantity.number || quantity.px == 0.0);
        valid.then_some(quantity.px)
    }

    /// A non-negative size that may be a percentage of the containing block.
    fn length_or_percent(&self, value: &str) -> Option<Length> {
        let quantity = self.quantity(value)?;
        let valid = (!quantity.number || quantity.px == 0.0)
            && (quantity.percent != 0.0 || quantity.px >= 0.0);
        valid.then_some(Length {
            percent: quantity.percent,
            px: quantity.px,
        })
    }

    /// A font size, where `em` and percentages refer to the parent's size.
    fn font_size(&self, value: &str) -> Option<f32> {
        let size = match value {
            "xx-small" => 9.0,
            "x-small" => 10.0,
            "small" => 13.0,
            "medium" => 16.0,
            "large" => 18.0,
            "x-large" => 24.0,
            "xx-large" => 32.0,
            "xxx-large" => 48.0,
            "smaller" => self.em / 1.2,
            "larger" => self.em * 1.2,
            _ => {
                let quantity = self.quantity(value)?;
                if quantity.number && quantity.px != 0.0 {
                    return None;
                }
                quantity.px + quantity.percent * self.em / 100.0
            }
        };
        (size.is_finite() && size >= 0.0).then_some(size)
    }

    fn line_height(&self, value: &str) -> Option<LineHeight> {
        if value == "normal" {
            return Some(LineHeight::Normal);
        }
        let quantity = self.quantity(value)?;
        if quantity.px < 0.0 || quantity.percent < 0.0 {
            return None;
        }
        Some(if quantity.number {
            LineHeight::Number(quantity.px)
        } else {
            LineHeight::Px(quantity.px + quantity.percent * self.em / 100.0)
        })
    }

    fn border_width(&self, value: &str) -> Option<f32> {
        match value {
            "thin" => Some(1.0),
            "medium" => Some(MEDIUM_BORDER),
            "thick" => Some(5.0),
            _ => self.length(value).filter(|n| *n >= 0.0),
        }
    }
}

/// A recursive-descent parser for `calc()`, `min()`, `max()` and `clamp()`.
struct MathParser<'a> {
    units: &'a Units,
    text: &'a str,
    position: usize,
}

impl<'a> MathParser<'a> {
    fn rest(&self) -> &'a str {
        &self.text[self.position..]
    }

    fn skip_whitespace(&mut self) {
        self.position += self.rest().len() - self.rest().trim_start().len();
    }

    fn eat(&mut self, token: char) -> bool {
        self.skip_whitespace();
        let found = self.rest().starts_with(token);
        if found {
            self.position += token.len_utf8();
        }
        found
    }

    /// Terms joined by `+` and `-`, which CSS requires to be surrounded by spaces.
    fn sum(&mut self) -> Option<Quantity> {
        let mut total = self.product()?;
        loop {
            let before = self.position;
            self.skip_whitespace();
            let sign = match self.rest().chars().next() {
                Some('+') => 1.0,
                Some('-') => -1.0,
                _ => {
                    self.position = before;
                    return Some(total);
                }
            };
            self.position += 1;
            let term = self.product()?;
            if term.number != total.number {
                return None;
            }
            total.px += sign * term.px;
            total.percent += sign * term.percent;
        }
    }

    fn product(&mut self) -> Option<Quantity> {
        let mut result = self.factor()?;
        loop {
            if self.eat('*') {
                let other = self.factor()?;
                let (length, scale) = match (result.number, other.number) {
                    (true, _) => (other, result.px),
                    (false, true) => (result, other.px),
                    (false, false) => return None,
                };
                result = Quantity {
                    px: length.px * scale,
                    percent: length.percent * scale,
                    number: length.number,
                };
            } else if self.eat('/') {
                let divisor = self.factor()?;
                if !divisor.number || divisor.px == 0.0 {
                    return None;
                }
                result.px /= divisor.px;
                result.percent /= divisor.px;
            } else {
                return Some(result);
            }
        }
    }

    fn factor(&mut self) -> Option<Quantity> {
        self.skip_whitespace();
        let rest = self.rest();
        let name_end = rest
            .find(|c: char| !(c.is_ascii_alphabetic() || c == '-'))
            .unwrap_or(rest.len());
        if rest[name_end..].starts_with('(') && !rest[..name_end].starts_with('-')
            || rest.starts_with('(')
        {
            let name = rest[..name_end].to_owned();
            self.position += name_end + 1;
            let mut arguments = vec![self.sum()?];
            while self.eat(',') {
                arguments.push(self.sum()?);
            }
            if !self.eat(')') {
                return None;
            }
            // Comparisons between lengths and percentages cannot be decided before layout.
            let comparable = arguments
                .iter()
                .all(|a| a.percent == 0.0 && a.number == arguments[0].number);
            let pick = |choose: fn(f32, f32) -> f32| {
                comparable.then(|| Quantity {
                    px: arguments.iter().map(|a| a.px).reduce(choose).unwrap_or(0.0),
                    ..arguments[0]
                })
            };
            return match (name.as_str(), arguments.len()) {
                ("" | "calc", 1) => Some(arguments[0]),
                ("min", _) => pick(f32::min),
                ("max", _) => pick(f32::max),
                ("clamp", 3) if comparable => Some(Quantity {
                    px: arguments[1].px.min(arguments[2].px).max(arguments[0].px),
                    ..arguments[0]
                }),
                _ => None,
            };
        }
        let end = rest
            .char_indices()
            .skip(1)
            .find(|(_, c)| c.is_whitespace() || matches!(c, '*' | '/' | ',' | ')' | '('))
            .map_or(rest.len(), |(index, _)| index);
        let token = &rest[..end];
        self.position += end;
        self.units.dimension(token)
    }
}

fn font_weight(value: &str) -> Option<bool> {
    match value {
        "bold" | "bolder" => Some(true),
        "normal" | "lighter" => Some(false),
        _ => value.parse::<f32>().ok().map(|weight| weight >= 600.0),
    }
}

fn display(value: &str) -> Option<Display> {
    Some(match value {
        "block" | "list-item" | "grid" | "flow-root" | "table-caption" | "table-row-group"
        | "table-header-group" | "table-footer-group" => Display::Block,
        "inline" | "contents" => Display::Inline,
        "inline-block" | "inline-flex" | "inline-grid" | "inline-table" => Display::InlineBlock,
        "flex" => Display::Flex,
        "table" => Display::Table,
        "table-row" => Display::TableRow,
        "table-cell" => Display::TableCell,
        "none" => Display::None,
        _ => return None,
    })
}

fn justify(value: &str) -> Option<Justify> {
    Some(match value {
        "flex-start" | "start" | "left" | "normal" | "stretch" => Justify::Start,
        "center" => Justify::Center,
        "flex-end" | "end" | "right" => Justify::End,
        "space-between" => Justify::SpaceBetween,
        "space-around" => Justify::SpaceAround,
        "space-evenly" => Justify::SpaceEvenly,
        _ => return None,
    })
}

fn align_items(value: &str) -> Option<AlignItems> {
    Some(match value {
        "stretch" | "normal" => AlignItems::Stretch,
        "flex-start" | "start" | "self-start" | "baseline" => AlignItems::Start,
        "center" => AlignItems::Center,
        "flex-end" | "end" | "self-end" => AlignItems::End,
        _ => return None,
    })
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
    split_top_level(value, ' ')
        .into_iter()
        .flat_map(|part| split_top_level(part, '\t'))
        .flat_map(|part| split_top_level(part, '\n'))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .collect()
}

/// Replaces every `var()` in a value. Returns `None` if a variable is missing and has no fallback.
fn substitute(value: &str, variables: &HashMap<String, String>, depth: u32) -> Option<String> {
    let Some(start) = value.find("var(") else {
        return Some(value.to_owned());
    };
    if depth > MAX_VARIABLE_DEPTH {
        return None;
    }
    let after = &value[start + 4..];
    let close = css::closing_bracket(after, '(', ')')?;
    let arguments = &after[..close];
    let (name, fallback) = match css::find_top_level(arguments, &[',']) {
        Some(comma) => (
            arguments[..comma].trim(),
            Some(arguments[comma + 1..].trim()),
        ),
        None => (arguments.trim(), None),
    };
    let replacement = match variables.get(name) {
        Some(found) => substitute(found, variables, depth + 1),
        None => None,
    };
    let replacement = match replacement {
        Some(found) => found,
        None => substitute(fallback?, variables, depth + 1)?,
    };
    let rest = substitute(&after[close + 1..], variables, depth)?;
    Some(format!("{}{replacement}{rest}", &value[..start]))
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
/// which maps a `link` element to the stylesheet text loaded for it. The
/// viewport decides media queries and viewport units.
pub fn compute(
    doc: &Document,
    linked: &HashMap<NodeId, String>,
    viewport: (f32, f32),
) -> Vec<Style> {
    let media = Media {
        width: viewport.0,
        height: viewport.1,
    };
    let user_agent = css::parse(USER_AGENT_CSS, media);
    let mut author = css::Stylesheet::default();
    for id in doc.ids() {
        if let Some(text) = linked.get(&id) {
            author.rules.extend(css::parse(text, media).rules);
        } else if let Some(element) = doc.element(id).filter(|e| e.name == "style")
            && element
                .attribute("media")
                .is_none_or(|query| media.matches(query))
        {
            author
                .rules
                .extend(css::parse(&doc.text_content(id), media).rules);
        }
    }
    let inline_specificity: Specificity = (u32::MAX, 0, 0);
    let root_style = Style::initial(None);
    let no_variables = Rc::new(HashMap::new());

    let mut styles: Vec<Style> = Vec::with_capacity(doc.len());
    let mut variables: Vec<Rc<HashMap<String, String>>> = Vec::with_capacity(doc.len());
    for id in doc.ids() {
        // Parents are always created before their children, so the parent's style exists.
        let parent_id = doc.node(id).parent;
        let parent = parent_id.map_or(root_style, |parent| styles[parent.0]);
        let mut own_variables =
            parent_id.map_or_else(|| Rc::clone(&no_variables), |p| Rc::clone(&variables[p.0]));
        let mut style = Style::initial(parent_id.map(|_| &parent));

        if let Some(element) = doc.element(id) {
            // Each entry is (important, origin, specificity, order, name, value).
            let mut matched: Vec<(bool, usize, Specificity, usize, &str, String)> = Vec::new();
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

            // Custom properties first, since any other declaration may refer to them.
            if matched.iter().any(|entry| entry.4.starts_with("--")) {
                let mut map = (*own_variables).clone();
                for (_, _, _, _, name, value) in
                    matched.iter().filter(|entry| entry.4.starts_with("--"))
                {
                    map.insert((*name).to_owned(), value.clone());
                }
                own_variables = Rc::new(map);
            }
            let resolved: Vec<(&str, String)> = matched
                .iter()
                .filter(|entry| !entry.4.starts_with("--"))
                .filter_map(|(_, _, _, _, name, value)| {
                    let value = substitute(value, &own_variables, 0)?;
                    Some((*name, value.trim().to_ascii_lowercase()))
                })
                .collect();

            // Font sizes next, because every `em` in the other declarations depends on them.
            for (name, value) in &resolved {
                style.apply_font_size(name, value, &parent, viewport);
            }
            for (name, value) in &resolved {
                if !matches!(*name, "font-size" | "font") {
                    style.apply(name, value, &parent, viewport);
                }
            }
        }
        styles.push(style);
        variables.push(own_variables);
    }
    styles
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html;

    const VIEWPORT: (f32, f32) = (1000.0, 500.0);

    fn style_at(html: &str, tag: &str, viewport: (f32, f32)) -> Option<Style> {
        let doc = html::parse(html);
        let styles = compute(&doc, &HashMap::new(), viewport);
        doc.find_element(tag).map(|id| styles[id.0])
    }

    fn style_of(html: &str, tag: &str) -> Option<Style> {
        style_at(html, tag, VIEWPORT)
    }

    #[test]
    fn colors_parse_in_each_supported_form() {
        assert_eq!(Color::parse("#f80"), Some(Color::rgb(255, 136, 0)));
        assert_eq!(Color::parse("#0a141e"), Some(Color::rgb(10, 20, 30)));
        assert_eq!(Color::parse("rgb(1, 2,3)"), Some(Color::rgb(1, 2, 3)));
        assert_eq!(Color::parse("rgb(1 2 300)"), Some(Color::rgb(1, 2, 255)));
        assert_eq!(
            Color::parse("rgb(100%, 0%, 50%)"),
            Some(Color::rgb(255, 0, 128))
        );
        assert_eq!(Color::parse("teal"), Some(Color::rgb(0, 128, 128)));
        assert_eq!(
            Color::parse("rebeccapurple"),
            Some(Color::rgb(102, 51, 153))
        );
        assert_eq!(Color::parse("transparent"), Some(Color::TRANSPARENT));
        assert_eq!(
            Color::parse("hsl(0, 100%, 50%)"),
            Some(Color::rgb(255, 0, 0))
        );
        assert_eq!(
            Color::parse("hsl(120deg 100% 25%)"),
            Some(Color::rgb(0, 128, 0))
        );
        assert_eq!(
            Color::parse("hsl(210, 50%, 40%)"),
            Some(Color::rgb(51, 102, 153))
        );
        let translucent = Color {
            r: 32,
            g: 33,
            b: 36,
            a: 128,
        };
        assert_eq!(Color::parse("rgba(32,33,36,.5)"), Some(translucent));
        assert_eq!(Color::parse("rgb(32 33 36 / 50%)"), Some(translucent));
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
    fn named_colors_are_sorted_for_binary_search() {
        assert!(NAMED_COLORS.windows(2).all(|pair| pair[0].0 < pair[1].0));
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
        assert_eq!(
            style_of("<a href='#'>x</a>", "a").map(|s| s.underline),
            Some(true)
        );
        assert_eq!(style_of("<a>x</a>", "a").map(|s| s.underline), Some(false));
        assert_eq!(style_of("<em>x</em>", "em").map(|s| s.italic), Some(true));
        assert_eq!(
            style_of("<div hidden></div>", "div").map(|s| s.display),
            Some(Display::None)
        );
    }

    #[test]
    fn specificity_beats_order_and_inline_style_beats_both() {
        let html = "<style>#a { width: 10px; height: 10px } .b { width: 20px } \
                    div { width: 30px; padding: 1px } div { padding: 2px }</style>\
                    <div id=a class=b style='height: 40px'></div>";
        let div = style_of(html, "div");
        assert_eq!(div.map(|s| s.width), Some(Some(Length::px(10.0))));
        assert_eq!(div.map(|s| s.height), Some(Some(Length::px(40.0))));
        assert_eq!(div.map(|s| s.padding), Some(Edges::all(2.0)));
    }

    #[test]
    fn important_declarations_win_over_inline_styles() {
        let html = "<style>div { width: 10px !important; height: 5px }</style>\
                    <div style='width: 20px; height: 6px'></div>";
        let div = style_of(html, "div");
        assert_eq!(div.map(|s| s.width), Some(Some(Length::px(10.0))));
        assert_eq!(div.map(|s| s.height), Some(Some(Length::px(6.0))));
    }

    #[test]
    fn box_shorthands_expand_and_invalid_values_are_ignored() {
        let html = "<div style='margin: 1px auto 3px; padding: 5px; padding: -1px; \
                    padding-left: 2furlongs; width: 50%; margin-left: -4px'></div>";
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
        assert_eq!(div.map(|s| s.width), Some(Some(Length::percent(50.0))));
    }

    #[test]
    fn relative_and_viewport_units_resolve() {
        let html =
            "<div style='padding: 2em 10vw 5vh 1rem; margin-left: 12pt; font-size: 10px'></div>";
        let div = style_of(html, "div");
        assert_eq!(
            div.map(|s| s.padding),
            Some(Edges {
                top: 20.0,
                right: 100.0,
                bottom: 25.0,
                left: 16.0
            })
        );
        assert_eq!(div.map(|s| s.margin.left), Some(Some(16.0)));
    }

    #[test]
    fn math_functions_are_evaluated() {
        let html = "<div style='width: calc(100% - 2 * 20px); height: calc((10px + 1em) * 2); \
                    padding-left: min(5vw, 30px); padding-right: max(1px, 2px, 3px); \
                    font-size: clamp(10px, 5vw, 20px); margin-top: calc(10px + 5%)'></div>";
        let div = style_of(html, "div");
        assert_eq!(
            div.map(|s| s.width),
            Some(Some(Length {
                percent: 100.0,
                px: -40.0
            }))
        );
        assert_eq!(div.map(|s| s.font_size), Some(20.0));
        assert_eq!(div.map(|s| s.height), Some(Some(Length::px(60.0))));
        assert_eq!(
            div.map(|s| (s.padding.left, s.padding.right)),
            Some((30.0, 3.0))
        );
        // A margin cannot be a percentage yet, so that declaration is ignored.
        assert_eq!(div.map(|s| s.margin.top), Some(Some(0.0)));
        assert_eq!(
            Length {
                percent: 100.0,
                px: -40.0
            }
            .resolve(500.0),
            460.0
        );
    }

    #[test]
    fn custom_properties_inherit_and_substitute() {
        let html = "<style>:root { --brand: #00f; --gap: 12px; --loop: var(--loop) } \
                    section { --gap: 20px } \
                    div { color: var(--brand); padding: var(--gap) var(--missing, 3px); \
                          margin-left: var(--missing); width: var(--loop, 7px); \
                          background: var(--also-missing, var(--brand)) }</style>\
                    <section><div></div></section>";
        let div = style_of(html, "div");
        assert_eq!(div.map(|s| s.color), Some(Color::rgb(0, 0, 255)));
        assert_eq!(
            div.map(|s| (s.padding.top, s.padding.left)),
            Some((20.0, 3.0))
        );
        assert_eq!(div.map(|s| s.margin.left), Some(Some(0.0)));
        assert_eq!(div.map(|s| s.width), Some(Some(Length::px(7.0))));
        assert_eq!(div.map(|s| s.background), Some(Color::rgb(0, 0, 255)));
    }

    #[test]
    fn the_font_shorthand_sets_size_weight_and_line_height() {
        let div = style_of(
            "<div style='font: italic bold 13px/27px Roboto, Arial, sans-serif'></div>",
            "div",
        );
        assert_eq!(
            div.map(|s| (s.font_size, s.bold, s.italic)),
            Some((13.0, true, true))
        );
        assert_eq!(div.map(|s| s.line_height), Some(LineHeight::Px(27.0)));
        let relative = style_of(
            "<div style='font: 16px/1.6 system-ui, sans-serif'></div>",
            "div",
        );
        assert_eq!(
            relative.map(|s| s.line_height),
            Some(LineHeight::Number(1.6))
        );
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
            "<div style='color: lime; border: solid 1px; border-color: red blue currentcolor; \
             border-left: none; border-top-width: 5px'></div>",
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
        assert_eq!(
            sides.map(|s| s.used_border_color().bottom),
            Some(Color::rgb(0, 255, 0))
        );
    }

    #[test]
    fn text_properties_inherit_and_keywords_override() {
        let html = "<div style='color: #00f; text-align: center; font-size: 20px; text-transform: uppercase; \
                    border: 1px solid; opacity: 0.5'>\
                    <section style='border: inherit; opacity: 0.5'><p style='color: initial; font-size: unset'></p></section></div>";
        let section = style_of(html, "section");
        assert_eq!(section.map(|s| s.color), Some(Color::rgb(0, 0, 255)));
        assert_eq!(section.map(|s| s.text_align), Some(TextAlign::Center));
        assert_eq!(
            section.map(|s| s.text_transform),
            Some(TextTransform::Uppercase)
        );
        assert_eq!(section.map(|s| s.border()), Some(Edges::all(1.0)));
        assert_eq!(section.map(|s| s.opacity), Some(0.25));
        let paragraph = style_of(html, "p");
        assert_eq!(paragraph.map(|s| s.color), Some(Color::BLACK));
        assert_eq!(paragraph.map(|s| s.font_size), Some(20.0));
    }

    #[test]
    fn box_model_and_decoration_properties_are_read() {
        let html = "<div style='box-sizing: border-box; min-width: 10px; max-width: 80%; min-height: 2em; \
                    border-radius: 8px 4px; visibility: hidden; position: absolute'></div>";
        let div = style_of(html, "div");
        assert_eq!(
            div.map(|s| (s.border_box, s.min_width, s.min_height)),
            Some((true, 10.0, 32.0))
        );
        assert_eq!(div.map(|s| s.max_width), Some(Some(Length::percent(80.0))));
        assert_eq!(div.map(|s| s.border_radius), Some(Radius::Px(8.0)));
        assert_eq!(div.map(|s| (s.visible, s.out_of_flow)), Some((false, true)));
        let pill = style_of("<div style='border-radius: 50%'></div>", "div");
        assert_eq!(pill.map(|s| s.used_radius(40.0, 20.0)), Some(10.0));
    }

    #[test]
    fn flex_properties_are_read() {
        let html = "<div style='display: flex; flex-flow: column wrap; justify-content: space-between; \
                    align-items: center; gap: 4px 8px'><span style='flex: 2 1 0'></span></div>";
        let div = style_of(html, "div");
        assert_eq!(
            div.map(|s| (s.display, s.flex_column, s.flex_wrap)),
            Some((Display::Flex, true, true))
        );
        assert_eq!(
            div.map(|s| (s.justify, s.align_items)),
            Some((Justify::SpaceBetween, AlignItems::Center))
        );
        assert_eq!(div.map(|s| s.gap), Some((4.0, 8.0)));
        assert_eq!(
            style_of(html, "span").map(|s| (s.flex_grow, s.flex_basis_zero)),
            Some((2.0, true))
        );
        let auto = style_of("<div style='flex: 1 1 auto'></div>", "div");
        assert_eq!(
            auto.map(|s| (s.flex_grow, s.flex_basis_zero)),
            Some((1.0, false))
        );
    }

    #[test]
    fn media_queries_follow_the_viewport() {
        let html = "<style>div { width: 100px } @media (max-width: 600px) { div { width: 50px } }</style><div></div>";
        assert_eq!(
            style_at(html, "div", (1000.0, 500.0)).map(|s| s.width),
            Some(Some(Length::px(100.0)))
        );
        assert_eq!(
            style_at(html, "div", (400.0, 800.0)).map(|s| s.width),
            Some(Some(Length::px(50.0)))
        );
    }

    #[test]
    fn legacy_attributes_become_styles() {
        let body = style_of("<body bgcolor='fff'>", "body");
        assert_eq!(body.map(|s| s.background), Some(Color::WHITE));
        let cell = style_of(
            "<table><tr valign=top><td width='25%' align=center nowrap></td></tr></table>",
            "td",
        );
        assert_eq!(cell.map(|s| s.width), Some(Some(Length::percent(25.0))));
        assert_eq!(
            cell.map(|s| (s.text_align, s.nowrap)),
            Some((TextAlign::Center, true))
        );
        assert_eq!(cell.map(|s| s.vertical_align), Some(VerticalAlign::Top));
        let hidden = style_of("<input type=HIDDEN>", "input");
        assert_eq!(hidden.map(|s| s.display), Some(Display::None));
        let list = style_of("<ul style='list-style: none'><li></li></ul>", "li");
        assert_eq!(
            list.map(|s| (s.list_item, s.list_marker)),
            Some((true, false))
        );
    }
}
