//! A CSS parser for style rules, and selector matching.
//!
//! Supported selectors are type, universal, class and id selectors, combined
//! into compounds and joined by the descendant combinator. A rule that uses
//! anything else is dropped whole, as the specification requires for
//! selectors a browser does not understand. At-rules are skipped.

use crate::dom::{Document, NodeId};

/// Selector specificity as (ids, classes, type names). Tuples compare in that order.
pub type Specificity = (u32, u32, u32);

/// A parsed stylesheet.
#[derive(Debug, Clone, Default)]
pub struct Stylesheet {
    /// Style rules in source order.
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

/// A `name: value` pair. The value is kept as lowercased text and interpreted by the style pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// The property name, lowercased.
    pub name: String,
    /// The value, trimmed and lowercased, without any `!important` marker.
    pub value: String,
    /// Whether the declaration was marked `!important`.
    pub important: bool,
}

/// A complex selector: compounds joined by descendant combinators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    /// Outermost ancestor first, subject last. Never empty.
    compounds: Vec<Compound>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Compound {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
}

/// Parses a stylesheet. Rules that cannot be understood are skipped.
pub fn parse(source: &str) -> Stylesheet {
    let source = strip_comments(source);
    let mut rules = Vec::new();
    let mut rest = source.as_str();
    while let Some(stop) = rest.find(['{', ';']) {
        if rest.as_bytes()[stop] == b';' {
            // A statement at-rule such as `@import`, or stray text.
            rest = &rest[stop + 1..];
            continue;
        }
        let prelude = rest[..stop].trim();
        let block_end = matching_brace(&rest[stop..]).map_or(rest.len(), |end| stop + end);
        let block = &rest[stop + 1..block_end];
        rest = rest.get(block_end + 1..).unwrap_or_default();
        if prelude.starts_with('@') {
            continue;
        }
        if let Some(selectors) = parse_selector_list(prelude) {
            rules.push(Rule {
                selectors,
                declarations: parse_declarations(block),
            });
        }
    }
    Stylesheet { rules }
}

/// Parses a comma-separated selector list. Returns `None` if any selector in it is unsupported.
pub fn parse_selector_list(text: &str) -> Option<Vec<Selector>> {
    text.split(',').map(Selector::parse).collect()
}

/// Parses the inside of a declaration block, or a `style` attribute.
pub fn parse_declarations(block: &str) -> Vec<Declaration> {
    block
        .split(';')
        .filter_map(|declaration| {
            let (name, value) = declaration.split_once(':')?;
            let (name, value) = (name.trim(), value.trim().to_ascii_lowercase());
            let (value, important) = match value.rsplit_once('!') {
                Some((rest, flag)) if flag.trim() == "important" => {
                    (rest.trim_end().to_owned(), true)
                }
                _ => (value, false),
            };
            (!name.is_empty() && !value.is_empty()).then(|| Declaration {
                name: name.to_ascii_lowercase(),
                value,
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

/// Given text that starts with `{`, returns the index of the `}` that closes it.
fn matching_brace(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (index, byte) in text.bytes().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn is_identifier(text: &str) -> bool {
    let valid = |c: char| c.is_alphanumeric() || c == '-' || c == '_';
    !text.is_empty() && text.chars().all(valid) && !text.starts_with(|c: char| c.is_ascii_digit())
}

impl Selector {
    /// Parses one complex selector. Returns `None` for syntax this engine does not support.
    pub fn parse(text: &str) -> Option<Self> {
        let compounds: Vec<Compound> = text
            .split_ascii_whitespace()
            .map(Compound::parse)
            .collect::<Option<_>>()?;
        (!compounds.is_empty()).then_some(Self { compounds })
    }

    /// The selector's specificity.
    pub fn specificity(&self) -> Specificity {
        self.compounds
            .iter()
            .fold((0, 0, 0), |(a, b, c), compound| {
                (
                    a + u32::from(compound.id.is_some()),
                    b + compound.classes.len() as u32,
                    c + u32::from(compound.tag.is_some()),
                )
            })
    }

    /// Whether the selector matches the given node.
    pub fn matches(&self, doc: &Document, node: NodeId) -> bool {
        let mut compounds = self.compounds.iter().rev();
        if !compounds
            .next()
            .is_some_and(|subject| subject.matches(doc, node))
        {
            return false;
        }
        // With only descendant combinators, taking the nearest matching ancestor
        // for each remaining compound never rules out a valid match.
        let mut ancestor = doc.node(node).parent;
        for compound in compounds {
            loop {
                let Some(id) = ancestor else { return false };
                ancestor = doc.node(id).parent;
                if compound.matches(doc, id) {
                    break;
                }
            }
        }
        true
    }
}

impl Compound {
    fn parse(text: &str) -> Option<Self> {
        let mut compound = Self::default();
        let first_marker = text.find(['#', '.']).unwrap_or(text.len());
        let tag = &text[..first_marker];
        if !tag.is_empty() && tag != "*" {
            if !is_identifier(tag) {
                return None;
            }
            compound.tag = Some(tag.to_ascii_lowercase());
        }
        let mut rest = &text[first_marker..];
        while !rest.is_empty() {
            let end = rest[1..].find(['#', '.']).map_or(rest.len(), |i| i + 1);
            let name = &rest[1..end];
            if !is_identifier(name) {
                return None;
            }
            if rest.starts_with('#') {
                if compound.id.as_deref().is_some_and(|id| id != name) {
                    return None; // Two different ids can never match one element.
                }
                compound.id = Some(name.to_owned());
            } else {
                compound.classes.push(name.to_owned());
            }
            rest = &rest[end..];
        }
        Some(compound)
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html;

    #[test]
    fn rules_and_declarations_are_parsed() {
        let sheet =
            parse("/* note */ div, .a { Color: RED; margin : 0 auto ;; } p{width:1px ! important}");
        assert_eq!(sheet.rules.len(), 2);
        assert_eq!(sheet.rules[0].selectors.len(), 2);
        assert_eq!(
            sheet.rules[0].declarations,
            [
                Declaration {
                    name: "color".into(),
                    value: "red".into(),
                    important: false,
                },
                Declaration {
                    name: "margin".into(),
                    value: "0 auto".into(),
                    important: false,
                },
            ]
        );
    }

    #[test]
    fn at_rules_and_unsupported_selectors_are_skipped() {
        let sheet = parse(
            "@import 'x.css'; @media print { div { width: 1px } } a:hover { width: 2px } \
             a > b { width: 3px } div { width: 4px }",
        );
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].declarations[0].value, "4px");
    }

    #[test]
    fn an_unclosed_block_runs_to_the_end() {
        let sheet = parse("div { width: 1px");
        assert_eq!(sheet.rules[0].declarations[0].value, "1px");
    }

    #[test]
    fn specificity_counts_ids_classes_and_types() {
        let specificity = |text| Selector::parse(text).map(|s| s.specificity());
        assert_eq!(specificity("*"), Some((0, 0, 0)));
        assert_eq!(specificity("div"), Some((0, 0, 1)));
        assert_eq!(specificity("div.a.b"), Some((0, 2, 1)));
        assert_eq!(specificity("#x .a div"), Some((1, 1, 1)));
        assert_eq!(specificity("div..a"), None);
        assert_eq!(specificity(""), None);
    }

    #[test]
    fn selectors_match_through_ancestors() {
        let doc = html::parse(
            "<div class='outer'><section><div id='target' class='inner box'></div></section></div>",
        );
        let target = doc.ids().find(|&id| {
            doc.element(id)
                .is_some_and(|e| e.attribute("id") == Some("target"))
        });
        let matches = |text| {
            Selector::parse(text)
                .zip(target)
                .is_some_and(|(selector, node)| selector.matches(&doc, node))
        };
        assert!(matches("div"));
        assert!(matches("#target.box.inner"));
        assert!(matches(".outer div"));
        assert!(matches("body .outer section .box"));
        assert!(matches("div div"));
        assert!(!matches("section"));
        assert!(!matches(".inner .box"));
        assert!(!matches("section .outer .box"));
    }
}
