//! What a browsing session remembers: each tab's back and forward list, and
//! the pages visited since the window opened.
//!
//! Nothing here is written to disk, so a session's history ends with it.

/// One tab's back and forward list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct History {
    entries: Vec<String>,
    /// The index of the page being shown. Meaningless while `entries` is empty.
    position: usize,
}

impl History {
    /// Records a newly visited page, dropping anything that was ahead of the current one.
    pub fn visit(&mut self, url: &str) {
        if self.current() == Some(url) {
            return;
        }
        if !self.entries.is_empty() {
            self.entries.truncate(self.position + 1);
        }
        self.entries.push(url.to_owned());
        self.position = self.entries.len() - 1;
    }

    /// The page being shown.
    pub fn current(&self) -> Option<&str> {
        self.entries.get(self.position).map(String::as_str)
    }

    /// Whether there is an earlier page to return to.
    pub fn can_go_back(&self) -> bool {
        self.position > 0
    }

    /// Whether a page was left by going back and can be returned to.
    pub fn can_go_forward(&self) -> bool {
        self.position + 1 < self.entries.len()
    }

    /// Steps back and returns the page to show, or `None` at the start of the list.
    pub fn back(&mut self) -> Option<&str> {
        if !self.can_go_back() {
            return None;
        }
        self.position -= 1;
        self.current()
    }

    /// Steps forward and returns the page to show, or `None` at the end of the list.
    pub fn forward(&mut self) -> Option<&str> {
        if !self.can_go_forward() {
            return None;
        }
        self.position += 1;
        self.current()
    }
}

/// A page that was loaded during the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Visit {
    /// The address that was loaded, after redirects.
    pub url: String,
    /// The page's title, or an empty string if it had none.
    pub title: String,
}

/// Builds the history page: every visit of the session as a link, newest first.
pub fn history_page(visits: &[Visit]) -> String {
    let mut html = String::from(
        "<!doctype html><title>History</title>
<style>
  body { margin: 40px auto; max-width: 720px; color: #202124; font-size: 15px; line-height: 1.5 }
  h1 { font-size: 26px; margin: 0 0 4px }
  .muted { color: #5f6368 }
  .visit { padding: 9px 0; border-bottom: 1px solid #e3e3e3 }
  .address { color: #5f6368; font-size: 12px }
</style>
<h1>History</h1>
<p class=muted>Pages opened since this window started, newest first. Nothing is saved when you quit.</p>
",
    );
    if visits.is_empty() {
        html.push_str("<p>No pages have been visited yet.</p>");
    }
    for visit in visits.iter().rev() {
        let (url, title) = (escape(&visit.url), escape(&visit.title));
        let label = if title.is_empty() { &url } else { &title };
        html.push_str(&format!(
            "<div class=visit><a href=\"{url}\">{label}</a><div class=address>{url}</div></div>\n"
        ));
    }
    html
}

/// Escapes text for use in HTML content and double-quoted attributes.
pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_history_has_nowhere_to_go() {
        let mut history = History::default();
        assert_eq!(history.current(), None);
        assert!(!history.can_go_back() && !history.can_go_forward());
        assert_eq!(history.back(), None);
        assert_eq!(history.forward(), None);
    }

    #[test]
    fn back_and_forward_walk_the_list() {
        let mut history = History::default();
        for url in ["a", "b", "c"] {
            history.visit(url);
        }
        assert_eq!(history.current(), Some("c"));
        assert_eq!(history.back(), Some("b"));
        assert_eq!(history.back(), Some("a"));
        assert_eq!(history.back(), None);
        assert!(history.can_go_forward());
        assert_eq!(history.forward(), Some("b"));
        assert_eq!(history.forward(), Some("c"));
        assert_eq!(history.forward(), None);
    }

    #[test]
    fn visiting_after_going_back_drops_the_forward_pages() {
        let mut history = History::default();
        for url in ["a", "b", "c"] {
            history.visit(url);
        }
        history.back();
        history.back();
        history.visit("d");
        assert!(!history.can_go_forward());
        assert_eq!(history.back(), Some("a"));
        assert_eq!(history.forward(), Some("d"));
    }

    #[test]
    fn visiting_the_current_page_again_adds_nothing() {
        let mut history = History::default();
        history.visit("a");
        history.visit("a");
        assert!(!history.can_go_back());
    }

    #[test]
    fn the_history_page_lists_visits_newest_first_and_escapes_them() {
        let visits = [
            Visit {
                url: "https://example.com/?a=1&b=2".to_owned(),
                title: "First <page>".to_owned(),
            },
            Visit {
                url: "https://second.example/".to_owned(),
                title: String::new(),
            },
        ];
        let html = history_page(&visits);
        let first = html.find("First &lt;page&gt;");
        let second = html.find(">https://second.example/</a>");
        assert!(
            first.is_some() && second.is_some() && second < first,
            "{html}"
        );
        assert!(html.contains("href=\"https://example.com/?a=1&amp;b=2\""));
        assert!(history_page(&[]).contains("No pages have been visited yet."));
    }
}
