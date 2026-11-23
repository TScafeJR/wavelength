//! The public entry point: load a page, then render it, query it or interact with it.
//!
//! This is the surface an agent integration is meant to build on. Everything
//! an agent needs to perceive a page goes through [`Page`]: a screenshot from
//! [`Page::render`], element lookup from [`Page::select`], on-screen geometry
//! from [`Page::bounds`] and readable content from [`Page::text`]. Acting on a
//! page goes through it too: [`Page::link_at`], [`Page::set_value`] and
//! [`Page::submit_url`].

use std::collections::HashMap;

use crate::dom::{Document, NodeId};
use crate::error::{Error, Result};
use crate::font::Fonts;
use crate::image::Image;
use crate::layout::{self, LayoutBox, Rect};
use crate::style::{self, Style};
use crate::{css, html, net, paint, url};

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
    /// The URL relative references resolve against, when the page has one.
    base_url: Option<String>,
    images: HashMap<NodeId, Image>,
    /// The text of each linked stylesheet, kept so styles can be recomputed when the viewport changes.
    stylesheets: HashMap<NodeId, String>,
    /// Text typed into form fields.
    values: HashMap<NodeId, String>,
    focused: Option<NodeId>,
}

impl Page {
    /// Parses `html` and lays it out for `viewport`, without loading any resources.
    pub fn load(html: &str, viewport: Viewport) -> Self {
        Self::build(
            html::parse(html),
            None,
            viewport,
            HashMap::new(),
            HashMap::new(),
        )
    }

    /// Like [`Page::load`], but also loads the stylesheets and images the page refers to.
    ///
    /// `base_url` is the address of the page, and `loader` is asked for the
    /// bytes behind each absolute URL. Resources it cannot provide are skipped.
    pub fn open(
        html: &str,
        base_url: &str,
        viewport: Viewport,
        loader: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
    ) -> Self {
        let document = html::parse(html);
        let mut stylesheets = HashMap::new();
        let mut images = HashMap::new();
        for (id, is_stylesheet, resource) in resource_references(&document, base_url) {
            let Some(bytes) = loader(&resource) else {
                continue;
            };
            if is_stylesheet {
                stylesheets.insert(id, String::from_utf8_lossy(&bytes).into_owned());
            } else if let Ok(picture) = Image::decode(&bytes) {
                images.insert(id, picture);
            }
        }
        Self::build(
            document,
            Some(base_url.to_owned()),
            viewport,
            stylesheets,
            images,
        )
    }

    /// Fetches the page at `target` and everything it refers to.
    ///
    /// `target` is an `http`, `https` or `file` URL. Resources are fetched in parallel.
    pub fn fetch(target: &str, viewport: Viewport) -> Result<Self> {
        let response = net::fetch(target)?;
        let html = response.text();
        let fetched = net::fetch_all(&Self::resource_urls(&html, &response.url));
        let mut loader = |resource: &str| fetched.get(resource).cloned().flatten();
        Ok(Self::open(&html, &response.url, viewport, &mut loader))
    }

    /// The absolute URLs of the stylesheets and images a page refers to, without duplicates.
    ///
    /// Fetching these ahead of [`Page::open`] lets a caller load them in parallel.
    pub fn resource_urls(html: &str, base_url: &str) -> Vec<String> {
        let mut urls: Vec<String> = Vec::new();
        for (_, _, resource) in resource_references(&html::parse(html), base_url) {
            if !urls.contains(&resource) {
                urls.push(resource);
            }
        }
        urls
    }

    fn build(
        document: Document,
        base_url: Option<String>,
        viewport: Viewport,
        stylesheets: HashMap<NodeId, String>,
        images: HashMap<NodeId, Image>,
    ) -> Self {
        let size = (viewport.width as f32, viewport.height as f32);
        let styles = style::compute(&document, &stylesheets, size);
        let mut page = Self {
            document,
            styles,
            stylesheets,
            layout: None,
            viewport,
            base_url,
            images,
            values: HashMap::new(),
            focused: None,
        };
        page.relayout();
        page
    }

    fn relayout(&mut self) {
        let image_sizes = self
            .images
            .iter()
            .map(|(&node, picture)| (node, (picture.width(), picture.height())))
            .collect();
        let context = layout::Context {
            doc: &self.document,
            styles: &self.styles,
            fonts: Fonts::system(),
            image_sizes: &image_sizes,
            values: &self.values,
            focused: self.focused,
        };
        self.layout = layout::layout(context, self.viewport.width as f32);
    }

    /// The document tree.
    pub fn document(&self) -> &Document {
        &self.document
    }

    /// The viewport the page was laid out for.
    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// Lays the page out again for a new viewport.
    pub fn resize(&mut self, viewport: Viewport) {
        if viewport != self.viewport {
            self.viewport = viewport;
            // Media queries and viewport units depend on the size, so styles are computed again.
            let size = (viewport.width as f32, viewport.height as f32);
            self.styles = style::compute(&self.document, &self.stylesheets, size);
            self.relayout();
        }
    }

    /// The address of the page, if it was loaded from one.
    pub fn url(&self) -> Option<&str> {
        self.base_url.as_deref()
    }

    /// The contents of the `title` element, if there is one with any text.
    pub fn title(&self) -> Option<String> {
        let title = self.document.find_element("title")?;
        let text = self.document.text_content(title).trim().to_owned();
        (!text.is_empty()).then_some(text)
    }

    /// The computed style of a node.
    pub fn style(&self, node: NodeId) -> &Style {
        &self.styles[node.0]
    }

    /// The root of the layout tree, or `None` if the page generates no boxes.
    pub fn layout(&self) -> Option<&LayoutBox> {
        self.layout.as_ref()
    }

    /// The height of the whole page, which may be more or less than the viewport's.
    pub fn content_height(&self) -> f32 {
        fn bottom(layout_box: &LayoutBox) -> f32 {
            let own = layout_box.rect.y + layout_box.rect.height;
            layout_box.children.iter().map(bottom).fold(own, f32::max)
        }
        self.layout.as_ref().map_or(0.0, bottom)
    }

    /// Paints the top of the page, as much as fits in the viewport.
    pub fn render(&self) -> Image {
        self.render_at(0.0)
    }

    /// Paints the viewport after scrolling down by `scroll_y` pixels.
    pub fn render_at(&self, scroll_y: f32) -> Image {
        let scene = paint::Scene {
            doc: &self.document,
            root: self.layout.as_ref(),
            fonts: Fonts::system(),
            images: &self.images,
        };
        paint::paint(scene, self.viewport.width, self.viewport.height, scroll_y)
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

    /// The node drawn at a point in page coordinates.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<NodeId> {
        self.layout.as_ref()?.hit_test(x, y)
    }

    /// The nearest ancestor of `node`, or the node itself, that is an element named `name`.
    fn enclosing(&self, node: NodeId, name: &str) -> Option<NodeId> {
        let mut current = Some(node);
        while let Some(id) = current {
            if self
                .document
                .element(id)
                .is_some_and(|element| element.name == name)
            {
                return Some(id);
            }
            current = self.document.node(id).parent;
        }
        None
    }

    /// Where the link at a point leads, as an absolute URL when the page has an address.
    pub fn link_at(&self, x: f32, y: f32) -> Option<String> {
        let link = self.enclosing(self.hit_test(x, y)?, "a")?;
        let reference = self.document.element(link)?.attribute("href")?;
        match &self.base_url {
            Some(base) => url::resolve(base, reference),
            None => Some(reference.to_owned()),
        }
    }

    /// The form control at a point, if there is one.
    pub fn control_at(&self, x: f32, y: f32) -> Option<NodeId> {
        self.enclosing(self.hit_test(x, y)?, "input")
    }

    /// Whether the control accepts typed text, as opposed to being a button or a checkbox.
    pub fn is_text_field(&self, control: NodeId) -> bool {
        self.document.element(control).is_some_and(|element| {
            let kind = element
                .attribute("type")
                .unwrap_or("text")
                .to_ascii_lowercase();
            element.name == "input"
                && !matches!(
                    kind.as_str(),
                    "submit"
                        | "button"
                        | "reset"
                        | "checkbox"
                        | "radio"
                        | "hidden"
                        | "image"
                        | "file"
                )
        })
    }

    /// The form field that has keyboard focus.
    pub fn focused(&self) -> Option<NodeId> {
        self.focused
    }

    /// Moves keyboard focus to a form field, or away from all of them.
    pub fn focus(&mut self, field: Option<NodeId>) {
        if field != self.focused {
            self.focused = field;
            self.relayout();
        }
    }

    /// The current text of a form field: what was typed, or else its `value` attribute.
    pub fn value(&self, field: NodeId) -> String {
        let attribute = || self.document.element(field)?.attribute("value");
        match self.values.get(&field) {
            Some(typed) => typed.clone(),
            None => attribute().unwrap_or_default().to_owned(),
        }
    }

    /// Replaces the text of a form field.
    pub fn set_value(&mut self, field: NodeId, value: String) {
        self.values.insert(field, value);
        self.relayout();
    }

    /// The URL that submitting the form around `control` would load.
    ///
    /// Only forms that submit with GET are supported. `control` is the field
    /// that had focus or the button that was pressed.
    pub fn submit_url(&self, control: NodeId) -> Option<String> {
        let form = self.enclosing(control, "form")?;
        let form_element = self.document.element(form)?;
        if form_element
            .attribute("method")
            .is_some_and(|m| m.eq_ignore_ascii_case("post"))
        {
            return None;
        }

        let mut fields = Vec::new();
        for id in self.document.ids() {
            let Some(element) = self.document.element(id) else {
                continue;
            };
            let name = element.attribute("name").unwrap_or_default();
            if element.name != "input"
                || name.is_empty()
                || self.enclosing(id, "form") != Some(form)
            {
                continue;
            }
            let kind = element
                .attribute("type")
                .unwrap_or("text")
                .to_ascii_lowercase();
            let included = match kind.as_str() {
                "submit" => id == control,
                "checkbox" | "radio" => element.attribute("checked").is_some(),
                "button" | "reset" | "image" | "file" => false,
                _ => true,
            };
            if included {
                fields.push((name.to_owned(), self.value(id)));
            }
        }

        let action = form_element.attribute("action").unwrap_or_default();
        let target = match &self.base_url {
            Some(base) => url::resolve(base, action)?,
            None => action.to_owned(),
        };
        let target = target.split(['?', '#']).next().unwrap_or_default();
        Some(format!("{target}?{}", url::encode_form(&fields)))
    }
}

/// The stylesheets and images a document refers to: the element, whether it
/// is a stylesheet, and the absolute URL.
fn resource_references(document: &Document, base_url: &str) -> Vec<(NodeId, bool, String)> {
    let mut references = Vec::new();
    for id in document.ids() {
        let Some(element) = document.element(id) else {
            continue;
        };
        let (attribute, is_stylesheet) = match element.name.as_str() {
            "link" => {
                let relation = element.attribute("rel").unwrap_or_default();
                let is_stylesheet = relation
                    .split_ascii_whitespace()
                    .any(|token| token.eq_ignore_ascii_case("stylesheet"));
                if !is_stylesheet {
                    continue;
                }
                ("href", true)
            }
            "img" => ("src", false),
            _ => continue,
        };
        let reference = element
            .attribute(attribute)
            .filter(|value| !value.is_empty());
        if let Some(resource) = reference.and_then(|reference| url::resolve(base_url, reference)) {
            references.push((id, is_stylesheet, resource));
        }
    }
    references
}

#[cfg(test)]
mod tests {
    use super::*;

    const HTML: &str = "<title> Shown in the tab </title>\
        <div class='card' style='height: 10px'>one</div>\
        <div class='card wide' style='height: 20px'>two</div><span class='card'>three</span>";

    #[test]
    fn select_returns_matches_in_document_order() -> Result<()> {
        let page = Page::load(HTML, Viewport::default());
        let cards = page.select(".card")?;
        assert_eq!(cards.len(), 3);
        assert_eq!(page.select("div.wide, span")?, cards[1..]);
        assert_eq!(page.select("body > span")?, cards[2..]);
        assert!(matches!(
            page.select("div >> span"),
            Err(Error::Selector(_))
        ));
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
        assert_eq!(
            page.hit_test(20.0, 20.0)
                .map(|node| page.enclosing(node, "div")),
            Some(Some(cards[1]))
        );
        Ok(())
    }

    #[test]
    fn text_and_title_come_from_the_document() {
        let page = Page::load(HTML, Viewport::default());
        assert_eq!(page.text(), "onetwothree");
        assert_eq!(page.title().as_deref(), Some("Shown in the tab"));
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

    #[test]
    fn resources_are_requested_by_absolute_url() {
        let html = "<link rel='stylesheet' href='/site.css'><link rel=icon href=i.ico>\
                    <img src='pics/logo.png'><div></div>";
        let mut requested = Vec::new();
        let mut loader = |resource: &str| {
            requested.push(resource.to_owned());
            resource
                .ends_with(".css")
                .then(|| b"div { height: 33px }".to_vec())
        };
        let page = Page::open(
            html,
            "https://example.com/a/index.html",
            Viewport::default(),
            &mut loader,
        );
        assert_eq!(
            requested,
            [
                "https://example.com/site.css",
                "https://example.com/a/pics/logo.png"
            ]
        );
        let div = page
            .document()
            .find_element("div")
            .and_then(|div| page.bounds(div));
        assert_eq!(div.map(|rect| rect.height), Some(33.0));
    }

    #[test]
    fn links_and_forms_produce_absolute_urls() -> Result<()> {
        let html = "<a href='/about' style='display: block; height: 20px'></a>\
            <form action='/search?old=1'>\
            <input type=hidden name=hl value=en><input name=q size=10>\
            <input type=submit name=go value=Search><input type=submit name=lucky value=Lucky>\
            <input type=checkbox name=safe><input name='' value=skipped></form>";
        let mut page = Page::open(
            html,
            "https://example.com/",
            Viewport::default(),
            &mut |_| None,
        );
        assert_eq!(
            page.link_at(20.0, 15.0).as_deref(),
            Some("https://example.com/about")
        );
        assert_eq!(page.link_at(20.0, 500.0), None);

        let inputs = page.select("input")?;
        let (query, go) = (inputs[1], inputs[2]);
        assert!(page.is_text_field(query) && !page.is_text_field(go));
        page.focus(Some(query));
        page.set_value(query, "rust lang".to_owned());
        assert_eq!(page.value(query), "rust lang");
        assert_eq!(
            page.submit_url(query).as_deref(),
            Some("https://example.com/search?hl=en&q=rust+lang")
        );
        assert_eq!(
            page.submit_url(go).as_deref(),
            Some("https://example.com/search?hl=en&q=rust+lang&go=Search")
        );
        Ok(())
    }
}
