//! A CSS parser for style rules, media queries and selectors, with selector matching.
//!
//! Selectors cover type, universal, class, id and attribute selectors, the
//! four combinators, and the structural pseudo-classes. Pseudo-classes that
//! depend on interaction, such as `:hover`, and all pseudo-elements parse but
//! never match, so the rest of a rule that mentions them still applies.

use crate::dom::{Document, NodeId};

/// Selector specificity as (ids, classes, type names). Tuples compare in that order.
pub type Specificity = (u32, u32, u32);

/// The device a stylesheet is being read for, which decides its media queries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Media {
    /// The viewport width in CSS pixels.
    pub width: f32,
    /// The viewport height in CSS pixels.
    pub height: f32,
}

/// A parsed stylesheet.
#[derive(Debug, Clone, Default)]
pub struct Stylesheet {
    /// Style rules in source order, including those inside media queries that apply.
    pub rules: Vec<Rule>,
}

/// One style rule: a selector list and its declaration block.
#[derive(Debug, Clone)]
pub struct Rule {
    /// The rule applies to an element if any of these match.
    pub selectors: Vec<Selector>,
    /// Declarations in source order.
    pub declarations: Vec<Declaration>,
}

/// A `name: value` pair. The value is kept as written and interpreted by the style pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// The property name, lowercased unless it is a custom property.
    pub name: String,
    /// The value, trimmed, without any `!important` marker.
    pub value: String,
    /// Whether the declaration was marked `!important`.
    pub important: bool,
}

/// A complex selector: compounds joined by combinators.
#[derive(Debug, Clone, PartialEq)]
pub struct Selector {
    /// Leftmost compound first, subject last. Never empty.
    compounds: Vec<Compound>,
    /// `combinators[i]` joins `compounds[i]` to `compounds[i + 1]`.
    combinators: Vec<Combinator>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Combinator {
    Descendant,
    Child,
    NextSibling,
    LaterSibling,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Compound {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    attributes: Vec<AttributeTest>,
    pseudos: Vec<Pseudo>,
}

#[derive(Debug, Clone, PartialEq)]
struct AttributeTest {
    name: String,
    operator: AttributeOperator,
    value: String,
    ignore_case: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttributeOperator {
    Exists,
    Equals,
    Includes,
    DashMatch,
    Prefix,
    Suffix,
    Substring,
}

#[derive(Debug, Clone, PartialEq)]
enum Pseudo {
    Root,
    Empty,
    Link,
    Checked,
    Disabled,
    Enabled,
    /// `:nth-child(an + b)` and its relatives: the position is counted from
    /// the end when `from_end` is set, and among same-named siblings when `of_type` is.
    Nth {
        step: i32,
        offset: i32,
        from_end: bool,
        of_type: bool,
    },
    /// `:only-child` and `:only-of-type`.
    Only {
        of_type: bool,
    },
    /// `:not()`.
    Not(Vec<Selector>),
    /// `:is()` and `:where()`. The flag says whether it counts towards specificity.
    Is(Vec<Selector>, bool),
    /// Something real browsers understand that never matches here, such as `:hover` or `::before`.
    Never,
}

/// Parses a stylesheet. Rules that cannot be understood are skipped.
pub fn parse(source: &str, media: Media) -> Stylesheet {
    let mut rules = Vec::new();
    parse_rules(&strip_comments(source), media, &mut rules);
    Stylesheet { rules }
}

fn parse_rules(source: &str, media: Media, rules: &mut Vec<Rule>) {
    let mut rest = source;
    while let Some(stop) = find_top_level(rest, &['{', ';']) {
        if rest.as_bytes()[stop] == b';' {
            // A statement at-rule such as `@import`, or stray text.
            rest = &rest[stop + 1..];
            continue;
        }
        let prelude = rest[..stop].trim();
        let block_end = matching_brace(&rest[stop..]).map_or(rest.len(), |end| stop + end);
        let block = &rest[stop + 1..block_end];
        rest = rest.get(block_end + 1..).unwrap_or_default();

        if let Some(at_rule) = prelude.strip_prefix('@') {
            let (name, condition) = at_rule
                .split_once(|c: char| c.is_whitespace())
                .unwrap_or((at_rule, ""));
            match name.to_ascii_lowercase().as_str() {
                "media" if media.matches(condition) => parse_rules(block, media, rules),
                // Cascade layers are flattened: their rules apply in source order.
                "layer" => parse_rules(block, media, rules),
                _ => {}
            }
        } else if let Some(selectors) = parse_selector_list(prelude) {
            rules.push(Rule {
                selectors,
                declarations: parse_declarations(block),
            });
        }
    }
}

/// Parses a comma-separated selector list. Returns `None` if any selector in it is invalid.
pub fn parse_selector_list(text: &str) -> Option<Vec<Selector>> {
    split_top_level(text, ',')
        .into_iter()
        .map(Selector::parse)
        .collect()
}

/// Parses the inside of a declaration block, or a `style` attribute.
pub fn parse_declarations(block: &str) -> Vec<Declaration> {
    split_top_level(block, ';')
        .into_iter()
        .filter_map(|declaration| {
            let colon = find_top_level(declaration, &[':'])?;
            let (name, value) = (declaration[..colon].trim(), declaration[colon + 1..].trim());
            let (value, important) = match value.rsplit_once('!') {
                Some((rest, flag)) if flag.trim().eq_ignore_ascii_case("important") => {
                    (rest.trim_end(), true)
                }
                _ => (value, false),
            };
            let valid_name =
                !name.is_empty() && !name.contains(|c: char| c.is_whitespace() || c == '{');
            (valid_name && !value.is_empty()).then(|| Declaration {
                name: if name.starts_with("--") {
                    name.to_owned()
                } else {
                    name.to_ascii_lowercase()
                },
                value: value.to_owned(),
                important,
            })
        })
        .collect()
}

fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        out.push(' ');
        rest = rest[start + 2..]
            .find("*/")
            .map_or("", |end| &rest[start + 2 + end + 2..]);
    }
    out.push_str(rest);
    out
}

/// Walks `text`, calling `visit` with each character that is not inside a
/// string, and the bracket depth at that point. Stops when `visit` returns true.
fn scan(text: &str, mut visit: impl FnMut(usize, char, usize) -> bool) {
    let (mut depth, mut quote, mut escaped) = (0usize, None, false);
    for (index, character) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, character) {
            (_, '\\') => escaped = true,
            (Some(open), _) => {
                if character == open {
                    quote = None;
                }
            }
            (None, '"' | '\'') => quote = Some(character),
            (None, _) => {
                if matches!(character, ')' | ']' | '}') {
                    depth = depth.saturating_sub(1);
                }
                if visit(index, character, depth) {
                    return;
                }
                if matches!(character, '(' | '[' | '{') {
                    depth += 1;
                }
            }
        }
    }
}

/// The first position of any of `targets` that is outside strings and brackets.
pub(crate) fn find_top_level(text: &str, targets: &[char]) -> Option<usize> {
    let mut found = None;
    scan(text, |index, character, depth| {
        if depth == 0 && targets.contains(&character) {
            found = Some(index);
        }
        found.is_some()
    });
    found
}

/// Splits on a delimiter wherever it appears outside strings and brackets.
pub(crate) fn split_top_level(text: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    scan(text, |index, character, depth| {
        if depth == 0 && character == delimiter {
            parts.push(&text[start..index]);
            start = index + character.len_utf8();
        }
        false
    });
    parts.push(&text[start..]);
    parts
}

/// Given the text after an opening bracket, returns the index of the bracket
/// that closes it, skipping over nested pairs and strings.
pub(crate) fn closing_bracket(text: &str, open: char, close: char) -> Option<usize> {
    let (mut depth, mut quote, mut escaped) = (0usize, None, false);
    for (index, character) in text.char_indices() {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if let Some(opened) = quote {
            if character == opened {
                quote = None;
            }
        } else if matches!(character, '"' | '\'') {
            quote = Some(character);
        } else if character == open {
            depth += 1;
        } else if character == close {
            if depth == 0 {
                return Some(index);
            }
            depth -= 1;
        }
    }
    None
}

/// Given text that starts with `{`, returns the index of the `}` that closes it.
fn matching_brace(text: &str) -> Option<usize> {
    let mut found = None;
    scan(text, |index, character, depth| {
        if character == '}' && depth == 0 {
            found = Some(index);
        }
        found.is_some()
    });
    found
}

impl Media {
    /// Whether a media query list applies. Unknown features count as not matching.
    pub fn matches(&self, query_list: &str) -> bool {
        let query_list = query_list.to_ascii_lowercase();
        split_top_level(&query_list, ',')
            .into_iter()
            .any(|query| self.matches_query(query.trim()))
    }

    fn matches_query(&self, query: &str) -> bool {
        let (negated, query) = match query.strip_prefix("not ") {
            Some(rest) => (true, rest.trim()),
            None => (false, query.strip_prefix("only ").unwrap_or(query).trim()),
        };
        let all = query.split(" and ").all(|part| {
            let part = part.trim();
            match part
                .strip_prefix('(')
                .and_then(|inner| inner.strip_suffix(')'))
            {
                Some(feature) => self.matches_feature(feature.trim()),
                None => matches!(part, "all" | "screen" | ""),
            }
        });
        all != negated
    }

    fn matches_feature(&self, feature: &str) -> bool {
        let pixels = |value: &str| {
            let value = value.trim();
            let scaled = |suffix: &str, factor: f32| {
                Some(value.strip_suffix(suffix)?.trim().parse::<f32>().ok()? * factor)
            };
            scaled("px", 1.0)
                .or_else(|| scaled("rem", 16.0))
                .or_else(|| scaled("em", 16.0))
        };
        // Range syntax, such as `width >= 600px`.
        type Comparison = fn(f32, f32) -> bool;
        let comparisons: [(&str, Comparison); 4] = [
            (">=", |a, b| a >= b),
            ("<=", |a, b| a <= b),
            (">", |a, b| a > b),
            ("<", |a, b| a < b),
        ];
        for (operator, compare) in comparisons {
            if let Some((name, value)) = feature.split_once(operator) {
                let actual = match name.trim() {
                    "width" => self.width,
                    "height" => self.height,
                    _ => return false,
                };
                return pixels(value).is_some_and(|limit| compare(actual, limit));
            }
        }
        let (name, value) = feature.split_once(':').unwrap_or((feature, ""));
        let (name, value) = (name.trim(), value.trim());
        match name {
            "min-width" => pixels(value).is_some_and(|limit| self.width >= limit),
            "max-width" => pixels(value).is_some_and(|limit| self.width <= limit),
            "min-height" => pixels(value).is_some_and(|limit| self.height >= limit),
            "max-height" => pixels(value).is_some_and(|limit| self.height <= limit),
            "orientation" => (value == "landscape") == (self.width >= self.height),
            "prefers-color-scheme" => value == "light",
            "prefers-reduced-motion" | "prefers-contrast" => value == "no-preference",
            "hover" | "any-hover" => value == "hover",
            "pointer" | "any-pointer" => value == "fine",
            "color" | "width" | "height" => value.is_empty(),
            _ => false,
        }
    }
}

fn is_identifier_character(character: char) -> bool {
    character.is_alphanumeric() || matches!(character, '-' | '_') || !character.is_ascii()
}

/// A cursor over selector text.
struct SelectorParser<'a> {
    text: &'a str,
    position: usize,
}

impl<'a> SelectorParser<'a> {
    fn peek(&self) -> Option<char> {
        self.text[self.position..].chars().next()
    }

    fn bump(&mut self) {
        self.position += self.peek().map_or(0, char::len_utf8);
    }

    /// Skips whitespace and reports whether there was any.
    fn skip_whitespace(&mut self) -> bool {
        let start = self.position;
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
        self.position > start
    }

    /// Reads an identifier, resolving backslash escapes such as the one in `.md\:flex`.
    fn identifier(&mut self) -> Option<String> {
        let mut name = String::new();
        while let Some(character) = self.peek() {
            if character == '\\' {
                self.bump();
                name.push(self.peek()?);
            } else if is_identifier_character(character) {
                name.push(character);
            } else {
                break;
            }
            self.bump();
        }
        (!name.is_empty()).then_some(name)
    }

    /// Reads the text inside the bracket that was just opened, up to its matching close.
    fn bracketed(&mut self, close: char) -> Option<&'a str> {
        let rest = &self.text[self.position..];
        let open = if close == ')' { '(' } else { '[' };
        let end = closing_bracket(rest, open, close)?;
        self.position += end + close.len_utf8();
        Some(&rest[..end])
    }

    fn complex(&mut self) -> Option<Selector> {
        let (mut compounds, mut combinators) = (Vec::new(), Vec::new());
        self.skip_whitespace();
        loop {
            compounds.push(self.compound()?);
            let spaced = self.skip_whitespace();
            let combinator = match self.peek() {
                None => break,
                Some('>') => Combinator::Child,
                Some('+') => Combinator::NextSibling,
                Some('~') => Combinator::LaterSibling,
                Some(_) if spaced => {
                    combinators.push(Combinator::Descendant);
                    continue;
                }
                Some(_) => return None,
            };
            self.bump();
            self.skip_whitespace();
            combinators.push(combinator);
        }
        Some(Selector {
            compounds,
            combinators,
        })
    }

    fn compound(&mut self) -> Option<Compound> {
        let mut compound = Compound::default();
        let start = self.position;
        loop {
            match self.peek() {
                Some('*') if self.position == start => self.bump(),
                Some('#') => {
                    self.bump();
                    let id = self.identifier()?;
                    if compound.id.as_ref().is_some_and(|existing| *existing != id) {
                        compound.pseudos.push(Pseudo::Never); // Two ids can never match one element.
                    }
                    compound.id = Some(id);
                }
                Some('.') => {
                    self.bump();
                    compound.classes.push(self.identifier()?);
                }
                Some('[') => {
                    self.bump();
                    compound
                        .attributes
                        .push(parse_attribute(self.bracketed(']')?)?);
                }
                Some(':') => {
                    self.bump();
                    compound.pseudos.push(self.pseudo()?);
                }
                Some(character) if self.position == start && is_identifier_character(character) => {
                    compound.tag = Some(self.identifier()?.to_ascii_lowercase());
                }
                _ => break,
            }
        }
        (self.position > start).then_some(compound)
    }

    fn pseudo(&mut self) -> Option<Pseudo> {
        let is_element = self.peek() == Some(':');
        if is_element {
            self.bump();
        }
        let name = self.identifier()?.to_ascii_lowercase();
        let argument = if self.peek() == Some('(') {
            self.bump();
            Some(self.bracketed(')')?)
        } else {
            None
        };
        if is_element {
            return Some(Pseudo::Never);
        }
        let nth = |from_end, of_type| {
            let (step, offset) = parse_nth(argument?)?;
            Some(Pseudo::Nth {
                step,
                offset,
                from_end,
                of_type,
            })
        };
        let edge = |from_end, of_type| Pseudo::Nth {
            step: 0,
            offset: 1,
            from_end,
            of_type,
        };
        Some(match name.as_str() {
            "root" => Pseudo::Root,
            "empty" => Pseudo::Empty,
            "link" | "any-link" => Pseudo::Link,
            "checked" => Pseudo::Checked,
            "disabled" => Pseudo::Disabled,
            "enabled" => Pseudo::Enabled,
            "first-child" => edge(false, false),
            "last-child" => edge(true, false),
            "first-of-type" => edge(false, true),
            "last-of-type" => edge(true, true),
            "only-child" => Pseudo::Only { of_type: false },
            "only-of-type" => Pseudo::Only { of_type: true },
            "nth-child" => nth(false, false)?,
            "nth-last-child" => nth(true, false)?,
            "nth-of-type" => nth(false, true)?,
            "nth-last-of-type" => nth(true, true)?,
            "not" => Pseudo::Not(parse_selector_list(argument?)?),
            "is" | "matches" | "-webkit-any" => Pseudo::Is(parse_selector_list(argument?)?, true),
            "where" => Pseudo::Is(parse_selector_list(argument?)?, false),
            // A vendor pseudo-class Chrome does not know makes the whole rule invalid there too.
            name if name.starts_with('-') && !name.starts_with("-webkit-") => return None,
            _ => Pseudo::Never,
        })
    }
}

/// Parses the inside of an attribute selector, such as `type="text" i`.
fn parse_attribute(inner: &str) -> Option<AttributeTest> {
    let inner = inner.trim();
    let name_end = inner
        .find(|c: char| !is_identifier_character(c))
        .unwrap_or(inner.len());
    let (name, rest) = (
        inner[..name_end].to_ascii_lowercase(),
        inner[name_end..].trim_start(),
    );
    if name.is_empty() {
        return None;
    }
    if rest.is_empty() {
        return Some(AttributeTest {
            name,
            operator: AttributeOperator::Exists,
            value: String::new(),
            ignore_case: false,
        });
    }
    let (operator, rest) = [
        ("~=", AttributeOperator::Includes),
        ("|=", AttributeOperator::DashMatch),
        ("^=", AttributeOperator::Prefix),
        ("$=", AttributeOperator::Suffix),
        ("*=", AttributeOperator::Substring),
        ("=", AttributeOperator::Equals),
    ]
    .into_iter()
    .find_map(|(symbol, operator)| Some((operator, rest.strip_prefix(symbol)?.trim_start())))?;

    let (value, flags) = match rest.chars().next()? {
        quote @ ('"' | '\'') => {
            let close = rest[1..].find(quote)? + 1;
            (&rest[1..close], rest[close + 1..].trim())
        }
        _ => rest.split_once(char::is_whitespace).unwrap_or((rest, "")),
    };
    Some(AttributeTest {
        name,
        operator,
        value: value.to_owned(),
        ignore_case: flags.trim().eq_ignore_ascii_case("i"),
    })
}

/// Parses the `an + b` argument of `:nth-child()` into (step, offset).
fn parse_nth(argument: &str) -> Option<(i32, i32)> {
    let text: String = argument.to_ascii_lowercase().split_whitespace().collect();
    match text.as_str() {
        "odd" => return Some((2, 1)),
        "even" => return Some((2, 0)),
        _ => {}
    }
    let Some((step, offset)) = text.split_once('n') else {
        return Some((0, text.parse().ok()?));
    };
    let step = match step {
        "" | "+" => 1,
        "-" => -1,
        _ => step.parse().ok()?,
    };
    let offset = if offset.is_empty() {
        0
    } else {
        offset.parse().ok()?
    };
    Some((step, offset))
}

/// The element children of a node's parent, which are the node's siblings including itself.
fn element_siblings(doc: &Document, node: NodeId) -> impl Iterator<Item = NodeId> + '_ {
    let parent = doc.node(node).parent;
    let children = parent.map_or(&[][..], |parent| &doc.node(parent).children[..]);
    children
        .iter()
        .copied()
        .filter(|&child| doc.element(child).is_some())
}

impl Selector {
    /// Parses one complex selector. Returns `None` if it is not valid.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let mut parser = SelectorParser { text, position: 0 };
        let selector = parser.complex()?;
        (parser.position == text.len()).then_some(selector)
    }

    /// The selector's specificity.
    pub fn specificity(&self) -> Specificity {
        let add = |(a, b, c): Specificity, (x, y, z): Specificity| (a + x, b + y, c + z);
        self.compounds
            .iter()
            .map(Compound::specificity)
            .fold((0, 0, 0), add)
    }

    /// Whether the selector matches the given node.
    pub fn matches(&self, doc: &Document, node: NodeId) -> bool {
        self.matches_from(doc, node, self.compounds.len() - 1)
    }

    /// Whether compound `index` matches `node` and everything to its left matches around it.
    fn matches_from(&self, doc: &Document, node: NodeId, index: usize) -> bool {
        if !self.compounds[index].matches(doc, node) {
            return false;
        }
        if index == 0 {
            return true;
        }
        let parent = doc.node(node).parent;
        match self.combinators[index - 1] {
            Combinator::Child => {
                parent.is_some_and(|parent| self.matches_from(doc, parent, index - 1))
            }
            Combinator::Descendant => {
                let mut ancestor = parent;
                while let Some(id) = ancestor {
                    if self.matches_from(doc, id, index - 1) {
                        return true;
                    }
                    ancestor = doc.node(id).parent;
                }
                false
            }
            Combinator::NextSibling => {
                let previous = element_siblings(doc, node)
                    .take_while(|&sibling| sibling != node)
                    .last();
                previous.is_some_and(|previous| self.matches_from(doc, previous, index - 1))
            }
            Combinator::LaterSibling => element_siblings(doc, node)
                .take_while(|&sibling| sibling != node)
                .any(|sibling| self.matches_from(doc, sibling, index - 1)),
        }
    }
}

impl Compound {
    fn specificity(&self) -> Specificity {
        let mut specificity = (
            u32::from(self.id.is_some()),
            (self.classes.len() + self.attributes.len()) as u32,
            u32::from(self.tag.is_some()),
        );
        for pseudo in &self.pseudos {
            match pseudo {
                Pseudo::Not(list) | Pseudo::Is(list, true) => {
                    let (a, b, c) = list
                        .iter()
                        .map(Selector::specificity)
                        .max()
                        .unwrap_or_default();
                    specificity = (specificity.0 + a, specificity.1 + b, specificity.2 + c);
                }
                Pseudo::Is(_, false) => {}
                _ => specificity.1 += 1,
            }
        }
        specificity
    }

    fn matches(&self, doc: &Document, node: NodeId) -> bool {
        let Some(element) = doc.element(node) else {
            return false;
        };
        self.tag.as_ref().is_none_or(|tag| *tag == element.name)
            && self
                .id
                .as_ref()
                .is_none_or(|id| element.attribute("id") == Some(id))
            && self
                .classes
                .iter()
                .all(|class| element.classes().any(|c| c == class))
            && self
                .attributes
                .iter()
                .all(|test| test.matches(element.attribute(&test.name)))
            && self.pseudos.iter().all(|pseudo| pseudo.matches(doc, node))
    }
}

impl AttributeTest {
    fn matches(&self, actual: Option<&str>) -> bool {
        let Some(actual) = actual else {
            return false;
        };
        let (actual, expected) = if self.ignore_case {
            (actual.to_ascii_lowercase(), self.value.to_ascii_lowercase())
        } else {
            (actual.to_owned(), self.value.clone())
        };
        let expected = expected.as_str();
        match self.operator {
            AttributeOperator::Exists => true,
            AttributeOperator::Equals => actual == expected,
            AttributeOperator::Includes => {
                actual.split_ascii_whitespace().any(|word| word == expected)
            }
            AttributeOperator::DashMatch => {
                actual == expected
                    || actual
                        .strip_prefix(expected)
                        .is_some_and(|rest| rest.starts_with('-'))
            }
            AttributeOperator::Prefix => !expected.is_empty() && actual.starts_with(expected),
            AttributeOperator::Suffix => !expected.is_empty() && actual.ends_with(expected),
            AttributeOperator::Substring => !expected.is_empty() && actual.contains(expected),
        }
    }
}

impl Pseudo {
    fn matches(&self, doc: &Document, node: NodeId) -> bool {
        let Some(element) = doc.element(node) else {
            return false;
        };
        let same_type = |sibling: NodeId| {
            doc.element(sibling)
                .is_some_and(|other| other.name == element.name)
        };
        match self {
            Self::Never => false,
            Self::Root => doc.node(node).parent == Some(doc.root()),
            Self::Empty => doc.node(node).children.is_empty(),
            Self::Link => {
                matches!(element.name.as_str(), "a" | "area") && element.attribute("href").is_some()
            }
            Self::Checked => {
                element.attribute("checked").is_some() || element.attribute("selected").is_some()
            }
            Self::Disabled => element.attribute("disabled").is_some(),
            Self::Enabled => {
                matches!(
                    element.name.as_str(),
                    "input" | "button" | "select" | "textarea"
                ) && element.attribute("disabled").is_none()
            }
            Self::Only { of_type } => {
                let alike =
                    element_siblings(doc, node).filter(|&sibling| !of_type || same_type(sibling));
                alike.count() == 1
            }
            Self::Nth {
                step,
                offset,
                from_end,
                of_type,
            } => {
                let siblings: Vec<NodeId> = element_siblings(doc, node)
                    .filter(|&sibling| !of_type || same_type(sibling))
                    .collect();
                let Some(index) = siblings.iter().position(|&sibling| sibling == node) else {
                    return false;
                };
                let position = if *from_end {
                    siblings.len() - index
                } else {
                    index + 1
                } as i32;
                // The position must equal step * n + offset for some n >= 0.
                let distance = position - offset;
                if *step == 0 {
                    distance == 0
                } else {
                    distance % step == 0 && distance / step >= 0
                }
            }
            Self::Not(list) => !list.iter().any(|selector| selector.matches(doc, node)),
            Self::Is(list, _) => list.iter().any(|selector| selector.matches(doc, node)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html;

    const DESKTOP: Media = Media {
        width: 1000.0,
        height: 700.0,
    };

    fn declaration(name: &str, value: &str, important: bool) -> Declaration {
        Declaration {
            name: name.to_owned(),
            value: value.to_owned(),
            important,
        }
    }

    #[test]
    fn rules_and_declarations_are_parsed() {
        let css = "/* note */ div, .a { Color: RED; margin : 0 auto ;; } p{width:1px ! important}";
        let sheet = parse(css, DESKTOP);
        assert_eq!(sheet.rules.len(), 2);
        assert_eq!(sheet.rules[0].selectors.len(), 2);
        assert_eq!(
            sheet.rules[0].declarations,
            [
                declaration("color", "RED", false),
                declaration("margin", "0 auto", false)
            ]
        );
        assert_eq!(
            sheet.rules[1].declarations,
            [declaration("width", "1px", true)]
        );
    }

    #[test]
    fn values_may_hold_delimiters_inside_strings_and_functions() {
        let declarations = parse_declarations(
            "background: url(data:image/png;base64,AA==) no-repeat; content: \"a;b}\"; \
             --Main-Color: #FFF; color: var(--Main-Color)",
        );
        assert_eq!(
            declarations,
            [
                declaration(
                    "background",
                    "url(data:image/png;base64,AA==) no-repeat",
                    false
                ),
                declaration("content", "\"a;b}\"", false),
                declaration("--Main-Color", "#FFF", false),
                declaration("color", "var(--Main-Color)", false),
            ]
        );
        let sheet = parse(
            "a::after { content: \"}\"; color: red } b { color: blue }",
            DESKTOP,
        );
        assert_eq!(sheet.rules.len(), 2);
    }

    #[test]
    fn media_queries_include_or_skip_their_rules() {
        let css = "@import 'x.css'; @media (min-width: 600px) { a { width: 1px } } \
                   @media screen and (max-width: 500px) { a { width: 2px } } \
                   @media print { a { width: 3px } } @media not print { a { width: 4px } } \
                   @media (prefers-color-scheme: dark) { a { width: 5px } } \
                   @media (width >= 900px) and (orientation: landscape) { a { width: 6px } } \
                   @font-face { font-family: x } @layer base { a { width: 7px } } a { width: 8px }";
        let widths = |media| -> Vec<String> {
            let rules = parse(css, media).rules;
            rules
                .iter()
                .map(|rule| rule.declarations[0].value.clone())
                .collect()
        };
        assert_eq!(widths(DESKTOP), ["1px", "4px", "6px", "7px", "8px"]);
        let phone = Media {
            width: 400.0,
            height: 800.0,
        };
        assert_eq!(widths(phone), ["2px", "4px", "7px", "8px"]);
    }

    #[test]
    fn invalid_selectors_drop_the_rule_and_an_unclosed_block_runs_to_the_end() {
        let css = "a > > b { width: 1px } a:-moz-thing, b { width: 2px } div..x { width: 3px } div { width: 4px";
        let sheet = parse(css, DESKTOP);
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].declarations[0].value, "4px");
    }

    #[test]
    fn specificity_counts_ids_classes_and_types() {
        let specificity = |text| Selector::parse(text).map(|s| s.specificity());
        assert_eq!(specificity("*"), Some((0, 0, 0)));
        assert_eq!(specificity("div"), Some((0, 0, 1)));
        assert_eq!(specificity("div.a.b"), Some((0, 2, 1)));
        assert_eq!(specificity("#x .a > div + p"), Some((1, 1, 2)));
        assert_eq!(specificity("a[href]:hover::before"), Some((0, 3, 1)));
        assert_eq!(specificity("li:not(.a, #b):first-child"), Some((1, 1, 1)));
        assert_eq!(specificity(":where(#a) :is(.b, p)"), Some((0, 1, 0)));
        assert_eq!(specificity(""), None);
        assert_eq!(parse_selector_list("a,"), None);
    }

    fn matcher(markup: &str) -> impl Fn(&str, &str) -> bool {
        let doc = html::parse(markup);
        move |selector: &str, id: &str| {
            let has_id = |n: &NodeId| {
                doc.element(*n)
                    .is_some_and(|e| e.attribute("id") == Some(id))
            };
            let node = doc.ids().find(has_id);
            Selector::parse(selector)
                .zip(node)
                .is_some_and(|(selector, node)| selector.matches(&doc, node))
        }
    }

    #[test]
    fn combinators_relate_elements() {
        let matches = matcher(
            "<div class='outer'><section id='s'><div id='target' class='inner box'></div></section>\
             <p id='p1'></p><p id='p2'></p><span id='sp'></span></div>",
        );
        assert!(matches("div", "target"));
        assert!(matches("#target.box.inner", "target"));
        assert!(matches(".outer div", "target"));
        assert!(matches("body .outer section .box", "target"));
        assert!(matches("section > .box", "target"));
        assert!(!matches(".outer > .box", "target"));
        assert!(matches(".outer > section > div", "target"));
        assert!(!matches(".inner .box", "target"));
        assert!(matches("section + p", "p1"));
        assert!(!matches("section + p", "p2"));
        assert!(matches("section ~ p", "p2"));
        assert!(matches("p + p + span", "sp"));
        assert!(matches("  div   div  ", "target"));
    }

    #[test]
    fn attribute_selectors_compare_values() {
        let matches = matcher(
            "<a id='a' href='https://example.com/x.PDF' lang='en-GB' class='btn big' data-x></a>",
        );
        assert!(matches("[data-x]", "a"));
        assert!(matches("a[href^='https://']", "a"));
        assert!(matches("a[href$='.pdf' i]", "a"));
        assert!(!matches("a[href$='.pdf']", "a"));
        assert!(matches("[href*=example]", "a"));
        assert!(matches("[class~=big]", "a"));
        assert!(!matches("[class=big]", "a"));
        assert!(matches("[lang|=en]", "a"));
        assert!(!matches("[title]", "a"));
    }

    #[test]
    fn structural_pseudo_classes_count_siblings() {
        let matches = matcher(
            "<ul><li id='one'></li><li id='two' class='x'></li><li id='three'></li><p id='p'></p></ul>\
             <a id='link' href='#'></a><a id='plain'></a><input id='off' disabled>",
        );
        assert!(matches("li:first-child", "one"));
        assert!(!matches("li:first-child", "two"));
        assert!(matches("li:last-of-type", "three"));
        assert!(!matches("li:last-child", "three"));
        assert!(matches("li:nth-child(2)", "two"));
        assert!(matches("li:nth-child(odd)", "three"));
        assert!(matches("li:nth-child(2n+1)", "one"));
        assert!(matches("li:nth-child(-n+2)", "two"));
        assert!(!matches("li:nth-child(-n+2)", "three"));
        assert!(matches("li:nth-last-child(2)", "three"));
        assert!(matches("p:only-of-type", "p"));
        assert!(matches("li:not(.x)", "one"));
        assert!(!matches("li:not(.x)", "two"));
        assert!(matches(":is(li, p).x", "two"));
        assert!(matches("a:link", "link"));
        assert!(!matches("a:link", "plain"));
        assert!(matches("input:disabled", "off"));
        assert!(matches("li:empty", "one"));
        assert!(matches("html:root li", "one"));
        assert!(matches("li:not(:nth-child(2)):not(.y)", "one"));
        assert!(!matches("li:not(:nth-child(2))", "two"));
    }

    #[test]
    fn interactive_and_pseudo_element_selectors_parse_but_never_match() {
        let matches = matcher("<a id='a' href='#'></a>");
        for selector in [
            "a:hover",
            "a::before",
            "a:focus-visible",
            "a:has(b)",
            "a::-webkit-scrollbar",
        ] {
            assert!(Selector::parse(selector).is_some(), "{selector}");
            assert!(!matches(selector, "a"), "{selector}");
        }
        // The rest of a selector list still applies.
        let list = parse_selector_list("a:hover, a").map(|list| list.len());
        assert_eq!(list, Some(2));
    }

    #[test]
    fn escaped_class_names_are_understood() {
        let matches = matcher("<div id='d' class='md:flex w-1/2'></div>");
        assert!(matches(".md\\:flex", "d"));
        assert!(matches(".w-1\\/2", "d"));
    }
}
