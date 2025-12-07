//! The public entry point: load a page, then render it or ask questions about it.
//!
//! This is the surface an agent integration is meant to build on. Everything
//! an agent needs to perceive a page goes through [`Page`]: a screenshot from
//! [`Page::render`], element lookup from [`Page::select`], on-screen geometry
//! from [`Page::bounds`] and readable content from [`Page::text`].

use crate::css;
use crate::dom::{Document, NodeId};
use crate::error::{Error, Result};
use crate::image::Image;
use crate::layout::{self, LayoutBox, Rect};
use crate::style::{self, Style};
use crate::{html, paint};

/// The size of the window a page is laid out for, in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    /// The width.
    pub width: u32,
    /// The height.
    pub height: u32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            width: 800,
            height: 600,
        }
    }
}

/// A parsed, styled and laid-out document.
#[derive(Debug, Clone)]
pub struct Page {
    document: Document,
    styles: Vec<Style>,
    layout: Option<LayoutBox>,
    viewport: Viewport,
}

impl Page {
    /// Parses `html` and lays it out for `viewport`. Loading never fails.
    pub fn load(html: &str, viewport: Viewport) -> Self {
        let document = html::parse(html);
        let styles = style::compute(&document);
        let layout = layout::layout(&document, &styles, viewport.width as f32);
        Self {
            document,
            styles,
            layout,
            viewport,
        }
    }

    /// The document tree.
    pub fn document(&self) -> &Document {
        &self.document
    }

    /// The viewport the page was laid out for.
    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// The computed style of a node.
    pub fn style(&self, node: NodeId) -> &Style {
        &self.styles[node.0]
    }

    /// The root of the layout tree, or `None` if the page generates no boxes.
    pub fn layout(&self) -> Option<&LayoutBox> {
        self.layout.as_ref()
    }

    /// Paints the part of the page that is visible in the viewport.
    pub fn render(&self) -> Image {
        paint::paint(
            &self.document,
            self.layout.as_ref(),
            self.viewport.width,
            self.viewport.height,
        )
    }

    /// The elements matching a selector list, in document order.
    pub fn select(&self, selector: &str) -> Result<Vec<NodeId>> {
        let selectors = css::parse_selector_list(selector)
            .ok_or_else(|| Error::Selector(selector.to_owned()))?;
        Ok(self
            .document
            .ids()
            .filter(|&id| selectors.iter().any(|s| s.matches(&self.document, id)))
            .collect())
    }

    /// The border box of a node in page coordinates, or `None` if it is not rendered.
    pub fn bounds(&self, node: NodeId) -> Option<Rect> {
        Some(self.layout.as_ref()?.find(node)?.rect)
    }

    /// The text of the document body.
    pub fn text(&self) -> String {
        self.document
            .find_element("body")
            .map(|body| self.document.text_content(body))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HTML: &str = "<title>Hidden</title>\
        <div class='card' style='height: 10px'>one</div>\
        <div class='card wide' style='height: 20px'>two</div><span class='card'>three</span>";

    #[test]
    fn select_returns_matches_in_document_order() -> Result<()> {
        let page = Page::load(HTML, Viewport::default());
        let cards = page.select(".card")?;
        assert_eq!(cards.len(), 3);
        assert_eq!(page.select("div.wide, span")?, cards[1..]);
        assert!(matches!(page.select("div > span"), Err(Error::Selector(_))));
        Ok(())
    }

    #[test]
    fn bounds_report_where_an_element_is_drawn() -> Result<()> {
        let page = Page::load(HTML, Viewport::default());
        let cards = page.select(".card")?;
        let second = page.bounds(cards[1]);
        assert_eq!(
            second.map(|r| (r.x, r.y, r.width, r.height)),
            Some((8.0, 18.0, 784.0, 20.0))
        );
        assert_eq!(page.bounds(cards[2]), None);
        Ok(())
    }

    #[test]
    fn text_covers_the_body_only() {
        let page = Page::load(HTML, Viewport::default());
        assert_eq!(page.text(), "onetwothree");
    }

    #[test]
    fn render_matches_the_viewport() {
        let viewport = Viewport {
            width: 40,
            height: 30,
        };
        let image = Page::load(HTML, viewport).render();
        assert_eq!((image.width(), image.height()), (40, 30));
    }
}
