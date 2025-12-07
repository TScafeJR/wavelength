//! The parity report: renders pages with the engine and with Chrome, then
//! writes both images, a diff overlay and an HTML page that shows them side by side.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::image::Image;
use crate::page::{Page, Viewport};
use crate::{chrome, diff};

/// How one page compared against the reference.
#[derive(Debug, Clone)]
pub struct Entry {
    /// The page's file name without its extension, which also names its images.
    pub name: String,
    /// How many pixels differ.
    pub differing: u64,
    /// How many pixels were compared.
    pub total: u64,
    /// The share of differing pixels, from 0 to 100.
    pub percent: f64,
}

/// Compares every page against Chrome and writes the report into `out_dir`.
///
/// For each page this writes `<name>.chrome.png`, `<name>.engine.png` and
/// `<name>.diff.png`, plus one `index.html` for the whole run. The reference
/// is taken fresh when a browser is available. Without one, a reference left
/// by an earlier run is reused, so the report still works on machines that
/// have no browser. The engine renders at the reference's size.
pub fn run(pages: &[PathBuf], out_dir: &Path, viewport: Viewport) -> Result<Vec<Entry>> {
    fs::create_dir_all(out_dir)?;
    let browser = chrome::find();
    let mut names = HashSet::new();
    let mut entries = Vec::new();

    for page_path in pages {
        let name = page_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| {
                Error::Usage(format!("{}: not a usable file name", page_path.display()))
            })?
            .to_owned();
        if !names.insert(name.clone()) {
            return Err(Error::Usage(format!(
                "two pages are named {name}; report files would overwrite each other"
            )));
        }

        let reference_path = out_dir.join(format!("{name}.chrome.png"));
        match &browser {
            Some(browser) => {
                chrome::screenshot(browser, page_path, viewport, &reference_path, out_dir)?;
            }
            None if reference_path.is_file() => {}
            None => return Err(Error::BrowserNotFound),
        }
        let reference = Image::load(&reference_path)?;

        let html = fs::read_to_string(page_path)?;
        let reference_viewport = Viewport {
            width: reference.width(),
            height: reference.height(),
        };
        let render = Page::load(&html, reference_viewport).render();
        render.save(&out_dir.join(format!("{name}.engine.png")))?;

        let diff = diff::compare(&render, &reference);
        diff.overlay
            .save(&out_dir.join(format!("{name}.diff.png")))?;
        entries.push(Entry {
            percent: diff.percent(),
            differing: diff.differing,
            total: diff.total,
            name,
        });
    }

    fs::write(
        out_dir.join("index.html"),
        index_html(&entries, browser.is_some()),
    )?;
    Ok(entries)
}

fn index_html(entries: &[Entry], fresh_references: bool) -> String {
    let exact = entries.iter().filter(|entry| entry.differing == 0).count();
    let source = if fresh_references {
        "References were captured from Chrome during this run."
    } else {
        "No browser was found, so references from an earlier run were reused."
    };
    let favicon = icon_data_uri();
    let mut html = format!(
        "<!doctype html>\n<html lang=\"en\">\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>Parity report · wavelength</title>\n\
         <link rel=\"icon\" href=\"{favicon}\">\n<style>{STYLE}</style>\n\
         <h1>{ICON}Parity report</h1>\n\
         <p>{exact} of {} pages match Chrome exactly. {source}</p>\n",
        entries.len()
    );
    for entry in entries {
        let name = escape(&entry.name);
        let status = if entry.differing == 0 {
            "exact"
        } else {
            "differs"
        };
        html.push_str(&format!(
            "<section>\n<h2>{name} <span class=\"{status}\">{} of {} pixels differ ({:.3}%)</span></h2>\n\
             <div class=\"renders\">\n",
            entry.differing, entry.total, entry.percent
        ));
        for (label, kind) in [("Engine", "engine"), ("Chrome", "chrome"), ("Diff", "diff")] {
            html.push_str(&format!(
                "<figure><figcaption>{label}</figcaption>\
                 <a href=\"{name}.{kind}.png\"><img src=\"{name}.{kind}.png\" alt=\"{label} render of {name}\"></a></figure>\n"
            ));
        }
        html.push_str("</div>\n</section>\n");
    }
    html.push_str("</html>\n");
    html
}

const STYLE: &str = "
:root { color-scheme: light dark; font: 15px/1.5 system-ui, sans-serif; }
body { margin: 2rem auto; max-width: 1400px; padding: 0 1.5rem; }
h1 { font-size: 1.4rem; display: flex; align-items: center; gap: 0.6rem; }
h1 svg { width: 1.8rem; height: 1.8rem; }
h2 { font-size: 1.05rem; margin: 2rem 0 0.75rem; }
h2 span { font-weight: normal; margin-left: 0.5rem; }
.exact { color: #2e7d32; }
.differs { color: #c62828; }
.renders { display: grid; grid-template-columns: repeat(3, 1fr); gap: 1rem; }
figure { margin: 0; min-width: 0; }
figcaption { font-size: 0.85rem; opacity: 0.7; margin-bottom: 0.25rem; }
img { display: block; width: 100%; border: 1px solid #8884; }
";

/// The project icon, drawn inline in the report header and reused as its favicon.
const ICON: &str = include_str!("../assets/icon.svg");

/// Encodes the icon as a `data:` URL so the report stays a single self-contained page.
fn icon_data_uri() -> String {
    let mut uri = String::from("data:image/svg+xml,");
    for c in ICON.chars() {
        match c {
            '#' => uri.push_str("%23"),
            '<' => uri.push_str("%3C"),
            '>' => uri.push_str("%3E"),
            '"' => uri.push('\''),
            '\n' => {}
            _ => uri.push(c),
        }
    }
    uri
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_index_lists_every_entry_and_escapes_names() {
        let entries = [
            Entry {
                name: "blocks".into(),
                differing: 0,
                total: 100,
                percent: 0.0,
            },
            Entry {
                name: "a<b".into(),
                differing: 5,
                total: 100,
                percent: 5.0,
            },
        ];
        let html = index_html(&entries, true);
        assert!(html.contains("1 of 2 pages match Chrome exactly."));
        assert!(html.contains("blocks.engine.png"));
        assert!(html.contains("a&lt;b <span class=\"differs\">5 of 100 pixels differ (5.000%)"));
        assert!(!html.contains("a<b"));
    }
}
