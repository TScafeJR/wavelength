//! A small HTML parser.
//!
//! This handles well-formed markup and the most common shortcuts: the `html`,
//! `head` and `body` elements always exist, void elements never take children,
//! and `script`, `style`, `title` and `textarea` hold raw text. It does not yet
//! follow the WHATWG tree construction algorithm, so it will disagree with
//! browsers on malformed input. Passing html5lib is the first roadmap item.

use crate::dom::{Document, Element, NodeData, NodeId};

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];
/// Elements whose content is text up to the matching end tag, taken literally.
const RAW_TEXT: &[&str] = &["script", "style"];
/// Like [`RAW_TEXT`], except that character references are decoded.
const ESCAPABLE_RAW_TEXT: &[&str] = &["textarea", "title"];
/// Elements that go into `head` when they appear before any body content.
const HEAD_ONLY: &[&str] = &["base", "link", "meta", "script", "style", "title"];

/// Elements whose start tag ends an open paragraph.
const CLOSES_PARAGRAPH: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "center",
    "dd",
    "details",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "li",
    "main",
    "menu",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "ul",
];

/// Parses a document. Parsing never fails; unrecognised input is skipped.
pub fn parse(input: &str) -> Document {
    let mut doc = Document::new();
    let html = doc.append(doc.root(), element("html", Vec::new()));
    let head = doc.append(html, element("head", Vec::new()));
    let body = doc.append(html, element("body", Vec::new()));
    let mut parser = Parser {
        input,
        pos: 0,
        doc,
        html,
        head,
        body,
        open: Vec::new(),
        in_body: false,
    };
    parser.run();
    parser.doc
}

fn element(name: &str, attributes: Vec<(String, String)>) -> NodeData {
    NodeData::Element(Element {
        name: name.to_owned(),
        attributes,
    })
}

fn is_tag_name_end(c: char) -> bool {
    c.is_ascii_whitespace() || c == '/' || c == '>'
}

struct Parser<'a> {
    input: &'a str,
    pos: usize,
    doc: Document,
    html: NodeId,
    head: NodeId,
    body: NodeId,
    /// Open elements inside `body`, innermost last.
    open: Vec<NodeId>,
    /// Set once body content has started, after which nothing goes to `head`.
    in_body: bool,
}

impl<'a> Parser<'a> {
    fn run(&mut self) {
        while self.pos < self.input.len() {
            let rest = self.rest();
            let letter_at = |index: usize| {
                rest.as_bytes()
                    .get(index)
                    .is_some_and(u8::is_ascii_alphabetic)
            };
            if rest.starts_with("<!--") {
                self.skip_past("-->");
            } else if rest.starts_with("</") && letter_at(2) {
                self.pos += 2;
                self.end_tag();
            } else if rest.starts_with('<') && letter_at(1) {
                self.pos += 1;
                self.start_tag();
            } else if ["<!", "<?", "</"].iter().any(|p| rest.starts_with(p)) {
                self.skip_past(">");
            } else {
                // Text runs to the next `<`. A leading `<` that opened no tag is text too.
                let end = rest
                    .char_indices()
                    .skip(1)
                    .find(|&(_, c)| c == '<')
                    .map_or(rest.len(), |(index, _)| index);
                self.pos += end;
                self.text(&rest[..end]);
            }
        }
    }

    fn rest(&self) -> &'a str {
        &self.input[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn take_while(&mut self, keep: impl Fn(char) -> bool) -> &'a str {
        let rest = self.rest();
        let end = rest.find(|c| !keep(c)).unwrap_or(rest.len());
        self.pos += end;
        &rest[..end]
    }

    fn skip_past(&mut self, needle: &str) {
        self.pos = match self.rest().find(needle) {
            Some(index) => self.pos + index + needle.len(),
            None => self.input.len(),
        };
    }

    /// The element new content is inserted into.
    fn current(&self) -> NodeId {
        self.open.last().copied().unwrap_or(self.body)
    }

    fn start_tag(&mut self) {
        let name = self
            .take_while(|c| !is_tag_name_end(c))
            .to_ascii_lowercase();
        let attributes = self.attributes();
        match name.as_str() {
            "html" => self.merge_attributes(self.html, attributes),
            "head" => self.merge_attributes(self.head, attributes),
            "body" => {
                self.merge_attributes(self.body, attributes);
                self.in_body = true;
            }
            _ => self.insert_element(&name, attributes),
        }
    }

    fn insert_element(&mut self, name: &str, attributes: Vec<(String, String)>) {
        let parent = if !self.in_body && HEAD_ONLY.contains(&name) {
            self.head
        } else {
            self.in_body = true;
            self.close_implied(name);
            self.current()
        };
        let id = self.doc.append(parent, element(name, attributes));
        let escapable = ESCAPABLE_RAW_TEXT.contains(&name);
        if escapable || RAW_TEXT.contains(&name) {
            let raw = self.raw_text(name);
            let text = if escapable {
                decode_entities(raw)
            } else {
                raw.to_owned()
            };
            if !text.is_empty() {
                self.doc.append(id, NodeData::Text(text));
            }
        } else if !VOID.contains(&name) {
            self.open.push(id);
        }
    }

    /// Ends the elements a new start tag closes implicitly: the previous list
    /// item, table cell or row, and any open paragraph before a block.
    fn close_implied(&mut self, name: &str) {
        let (targets, boundaries): (&[&str], &[&str]) = match name {
            "li" => (&["li"], &["ul", "ol", "table"]),
            "dt" | "dd" => (&["dt", "dd"], &["dl", "table"]),
            "tr" => (&["tr"], &["table"]),
            "td" | "th" => (&["td", "th"], &["tr", "table"]),
            "option" => (&["option"], &["select"]),
            _ => (&[], &[]),
        };
        self.close_nearest(targets, boundaries);
        if CLOSES_PARAGRAPH.contains(&name) {
            self.close_nearest(&["p"], &["table", "td", "th", "button", "caption"]);
        }
    }

    /// Closes the innermost open element named in `targets`, unless one of `boundaries` is met first.
    fn close_nearest(&mut self, targets: &[&str], boundaries: &[&str]) {
        for index in (0..self.open.len()).rev() {
            let Some(element) = self.doc.element(self.open[index]) else {
                continue;
            };
            if targets.contains(&element.name.as_str()) {
                self.open.truncate(index);
                return;
            }
            if boundaries.contains(&element.name.as_str()) {
                return;
            }
        }
    }

    /// Adds attributes from a repeated `html`, `head` or `body` tag without overriding existing ones.
    fn merge_attributes(&mut self, id: NodeId, attributes: Vec<(String, String)>) {
        if let NodeData::Element(element) = &mut self.doc.node_mut(id).data {
            for (name, value) in attributes {
                if element.attribute(&name).is_none() {
                    element.attributes.push((name, value));
                }
            }
        }
    }

    /// Reads attributes up to and including the closing `>` of a start tag.
    fn attributes(&mut self) -> Vec<(String, String)> {
        let mut attributes: Vec<(String, String)> = Vec::new();
        loop {
            self.take_while(|c| c.is_ascii_whitespace() || c == '/');
            match self.peek() {
                None => break,
                Some('>') => {
                    self.pos += 1;
                    break;
                }
                Some(_) => {}
            }
            let name = self
                .take_while(|c| !is_tag_name_end(c) && c != '=')
                .to_ascii_lowercase();
            if name.is_empty() {
                self.pos += 1; // A stray `=`.
                continue;
            }
            self.take_while(|c| c.is_ascii_whitespace());
            let value = if self.peek() == Some('=') {
                self.pos += 1;
                self.take_while(|c| c.is_ascii_whitespace());
                self.attribute_value()
            } else {
                String::new()
            };
            if !attributes.iter().any(|(existing, _)| *existing == name) {
                attributes.push((name, value));
            }
        }
        attributes
    }

    fn attribute_value(&mut self) -> String {
        let raw = match self.peek() {
            Some(quote @ ('"' | '\'')) => {
                self.pos += 1;
                let value = self.take_while(|c| c != quote);
                self.pos = (self.pos + 1).min(self.input.len());
                value
            }
            _ => self.take_while(|c| !c.is_ascii_whitespace() && c != '>'),
        };
        decode_entities(raw)
    }

    /// Consumes everything up to the end tag for `name`, and the end tag itself.
    fn raw_text(&mut self, name: &str) -> &'a str {
        let rest = self.rest();
        let end = rest
            .match_indices("</")
            .map(|(index, _)| index)
            .find(|index| {
                rest[index + 2..]
                    .get(..name.len())
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
            });
        match end {
            Some(end) => {
                self.pos += end;
                self.skip_past(">");
                &rest[..end]
            }
            None => {
                self.pos = self.input.len();
                rest
            }
        }
    }

    fn end_tag(&mut self) {
        let name = self
            .take_while(|c| !is_tag_name_end(c))
            .to_ascii_lowercase();
        self.skip_past(">");
        let doc = &self.doc;
        let matching = self
            .open
            .iter()
            .rposition(|&id| doc.element(id).is_some_and(|element| element.name == name));
        if let Some(index) = matching {
            self.open.truncate(index);
        }
    }

    fn text(&mut self, raw: &str) {
        if !self.in_body {
            if raw.trim_ascii().is_empty() {
                return;
            }
            self.in_body = true;
        }
        let text = decode_entities(raw);
        let parent = self.current();
        let last = self.doc.node(parent).children.last().copied();
        if let Some(last) = last
            && let NodeData::Text(existing) = &mut self.doc.node_mut(last).data
        {
            existing.push_str(&text);
            return;
        }
        self.doc.append(parent, NodeData::Text(text));
    }
}

/// Decodes the handful of character references this parser knows, leaving the rest as written.
fn decode_entities(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(ampersand) = rest.find('&') {
        out.push_str(&rest[..ampersand]);
        let after = &rest[ampersand + 1..];
        let decoded = after
            .find(';')
            .filter(|&end| end <= 10)
            .and_then(|end| Some((entity(&after[..end])?, end + 1)));
        match decoded {
            Some((character, consumed)) => {
                out.push(character);
                rest = &after[consumed..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        "copy" => Some('\u{a9}'),
        "reg" => Some('\u{ae}'),
        "trade" => Some('\u{2122}'),
        "middot" => Some('\u{b7}'),
        "bull" => Some('\u{2022}'),
        "hellip" => Some('\u{2026}'),
        "ndash" => Some('\u{2013}'),
        "mdash" => Some('\u{2014}'),
        "lsquo" => Some('\u{2018}'),
        "rsquo" => Some('\u{2019}'),
        "ldquo" => Some('\u{201c}'),
        "rdquo" => Some('\u{201d}'),
        "laquo" => Some('\u{ab}'),
        "raquo" => Some('\u{bb}'),
        "times" => Some('\u{d7}'),
        "euro" => Some('\u{20ac}'),
        "pound" => Some('\u{a3}'),
        _ => {
            let code = name.strip_prefix('#')?;
            let value = match code.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => code.parse().ok()?,
            };
            char::from_u32(value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialises the tree as nested tag names, which keeps the assertions readable.
    fn outline(doc: &Document, id: NodeId) -> String {
        let node = doc.node(id);
        let children: String = node.children.iter().map(|&c| outline(doc, c)).collect();
        match &node.data {
            NodeData::Document => children,
            NodeData::Text(text) => format!("'{text}'"),
            NodeData::Element(element) => format!("<{}>{children}</>", element.name),
        }
    }

    fn parsed(input: &str) -> String {
        let doc = parse(input);
        outline(&doc, doc.root())
    }

    #[test]
    fn html_head_and_body_always_exist() {
        assert_eq!(parsed(""), "<html><head></><body></></>");
        assert_eq!(
            parsed("<div>hi</div>"),
            "<html><head></><body><div>'hi'</></></>"
        );
    }

    #[test]
    fn metadata_goes_to_head_until_body_content_starts() {
        assert_eq!(
            parsed(
                "<!doctype html>\n<title>T</title>\n<style>a{}</style>\n<div></div><style>b{}</style>"
            ),
            "<html><head><title>'T'</><style>'a{}'</></><body><div></><style>'b{}'</></></>"
        );
    }

    #[test]
    fn explicit_structure_tags_merge_into_the_implied_ones() {
        let doc = parse("<html lang=en><head></head><body class='a b'><p>x</p></body></html>");
        assert_eq!(
            outline(&doc, doc.root()),
            "<html><head></><body><p>'x'</></></>"
        );
        let body = doc.find_element("body").and_then(|id| doc.element(id));
        assert_eq!(
            body.map(|b| b.classes().collect::<Vec<_>>()),
            Some(vec!["a", "b"])
        );
    }

    #[test]
    fn void_elements_take_no_children_and_end_tags_close_through() {
        assert_eq!(
            parsed("<div><br><span><b>x</div>y"),
            "<html><head></><body><div><br></><span><b>'x'</></></>'y'</></>"
        );
    }

    #[test]
    fn attributes_accept_every_quoting_style() {
        let doc = parse(r#"<div id=a CLASS="b c" data-x='1 &amp; 2' hidden id=z></div>"#);
        let div = doc.find_element("div").and_then(|id| doc.element(id));
        assert_eq!(
            div.map(|d| d.attributes.clone()),
            Some(vec![
                ("id".to_owned(), "a".to_owned()),
                ("class".to_owned(), "b c".to_owned()),
                ("data-x".to_owned(), "1 & 2".to_owned()),
                ("hidden".to_owned(), String::new()),
            ])
        );
    }

    #[test]
    fn raw_text_is_not_parsed_as_markup() {
        assert_eq!(
            parsed("<body><script>if (a < b) { x = '<div>'; }</SCRIPT><div></div>"),
            "<html><head></><body><script>'if (a < b) { x = '<div>'; }'</><div></></></>"
        );
    }

    #[test]
    fn comments_are_dropped_and_entities_decoded() {
        assert_eq!(
            parsed("<p>a<!-- <b> -->&lt;b&#33;&#x21;&unknown; é</p>"),
            "<html><head></><body><p>'a<b!!&unknown; é'</></></>"
        );
    }

    #[test]
    fn list_items_cells_and_paragraphs_close_implicitly() {
        assert_eq!(
            parsed("<ul><li>a<li>b<ul><li>c</ul><li>d</ul>"),
            "<html><head></><body><ul><li>'a'</><li>'b'<ul><li>'c'</></></><li>'d'</></></></>"
        );
        assert_eq!(
            parsed("<dl><dt>t<dd>d<dt>u</dl><p>one<p>two<div>three</div>"),
            "<html><head></><body><dl><dt>'t'</><dd>'d'</><dt>'u'</></><p>'one'</><p>'two'</><div>'three'</></></>"
        );
        assert_eq!(
            parsed("<table><tr><td>a<td>b<tr><td><p>c</table>"),
            "<html><head></><body><table><tr><td>'a'</><td>'b'</></><tr><td><p>'c'</></></></></></>"
        );
    }

    #[test]
    fn truncated_input_does_not_panic() {
        for input in [
            "<",
            "<div",
            "<div a=",
            "<div a='x",
            "</",
            "<!--",
            "<style>a",
            "&",
            "é<",
        ] {
            parse(input);
        }
    }
}
