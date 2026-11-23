//! The browser window: a tab strip and a toolbar on top of a page rendered by the engine.
//!
//! This is deliberately small. Loading is synchronous, so the window does not
//! respond while a page is being fetched, and there are no bookmarks or text
//! selection yet. History lasts as long as the window is open.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};

use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};

use crate::error::{Error, Result};
use crate::font::{Face, Fonts};
use crate::image::Image;
use crate::page::{Page, Viewport};
use crate::paint::{self, TextStyle};
use crate::session::{self, History, Visit};
use crate::style::Color;
use crate::{net, url};

/// Set this environment variable to a file path to save the first frame there and exit.
/// It exists so the window can be checked without a person watching it.
pub const SNAPSHOT_ENV: &str = "WAVELENGTH_SNAPSHOT";

const TAB_STRIP_HEIGHT: u32 = 32;
const TOOLBAR_HEIGHT: u32 = 44;
/// Everything above the page: the tab strip and the toolbar.
const CHROME_HEIGHT: u32 = TAB_STRIP_HEIGHT + TOOLBAR_HEIGHT;
const MAX_TAB_WIDTH: f32 = 220.0;
const TAB_CLOSE_WIDTH: f32 = 26.0;
const NEW_TAB_WIDTH: f32 = 36.0;
const NAV_BUTTON_WIDTH: f32 = 34.0;
const HISTORY_BUTTON_WIDTH: f32 = 76.0;
const ADDRESS_SIZE: f32 = 14.0;
const TAB_TITLE_SIZE: f32 = 12.0;
const LINE_SCROLL: f32 = 48.0;
const WHEEL_SCROLL: f32 = 6.0;
const START_URL: &str = "wavelength://start";
const HISTORY_URL: &str = "wavelength://history";

const STRIP_COLOR: [u8; 4] = [222, 225, 230, 255];
const TOOLBAR_COLOR: [u8; 4] = [241, 243, 244, 255];
const DIVIDER_COLOR: [u8; 4] = [196, 199, 197, 255];
const FIELD_COLOR: [u8; 4] = [255, 255, 255, 255];
const FOCUS_BORDER: [u8; 4] = [11, 87, 208, 255];
const TEXT_COLOR: Color = Color::rgb(32, 33, 36);
const MUTED_COLOR: Color = Color::rgb(140, 143, 148);

const START_PAGE: &str = "<!doctype html><title>New tab</title>
<style>
  :root { --ink: #1f2330; --muted: #6a7080; --line: #e2e5ec; }
  body { margin: 0; background: #f3f4f8; color: var(--ink); font: 15px/1.5 sans-serif; }
  main { max-width: 680px; margin: 0 auto; padding: 64px 20px; }
  h1 { font-size: 32px; margin: 0 0 4px; }
  p { margin: 0; }
  .muted { color: var(--muted); }
  .links { display: flex; gap: 12px; margin: 26px 0; }
  .links a { flex: 1; background: #fff; border: 1px solid var(--line); border-radius: 12px;
             padding: 14px 16px; color: var(--ink); text-decoration: none; font-weight: 700; }
  .links span { display: block; color: var(--muted); font-weight: 400; font-size: 12px; }
  .keys { background: #fff; border: 1px solid var(--line); border-radius: 12px; padding: 6px 18px;
          margin-bottom: 22px; }
  .keys div { display: flex; justify-content: space-between; padding: 8px 0; }
  .keys div + div { border-top: 1px solid var(--line); }
  .keys span:last-child { color: var(--muted); }
  @media (max-width: 620px) { .links { flex-direction: column; } }
</style>
<main>
<h1>Wavelength</h1>
<p class=muted>A browser engine written from scratch in Rust. Type an address above and press Enter.</p>
<div class=links>
  <a href='https://www.google.com/'>Google<span>google.com</span></a>
  <a href='https://news.ycombinator.com/'>Hacker News<span>news.ycombinator.com</span></a>
  <a href='https://en.wikipedia.org/wiki/Web_browser'>Wikipedia<span>en.wikipedia.org</span></a>
  <a href='wavelength://history'>History<span>this session</span></a>
</div>
<div class=keys>
  <div><span>Open and close tabs</span><span>Cmd+T, Cmd+W</span></div>
  <div><span>Back and forward</span><span>Cmd+[, Cmd+]</span></div>
  <div><span>Address bar, reload, history</span><span>Cmd+L, Cmd+R, Cmd+Y</span></div>
  <div><span>Open a link in a new tab</span><span>Cmd+click</span></div>
</div>
<p class=muted>Pages are drawn by the engine itself. It does not run JavaScript, so sites that
depend on it will look plain or incomplete.</p>
</main>";

/// Forwards typed characters from the window to the main loop.
struct Typed(Sender<char>);

impl minifb::InputCallback for Typed {
    fn add_char(&mut self, code: u32) {
        // Control characters and the private-use codes macOS sends for arrow keys are not text.
        let printable = code >= 0x20 && code != 0x7F && !(0xF700..=0xF8FF).contains(&code);
        if let Some(character) = char::from_u32(code).filter(|_| printable) {
            let _ = self.0.send(character);
        }
    }
}

/// One open page with its own scroll position and back and forward list.
struct Tab {
    page: Page,
    /// What the address bar shows, which differs from the page's URL while it is being edited.
    address: String,
    title: String,
    scroll: f32,
    history: History,
}

struct Browser {
    window: Window,
    typed: Receiver<char>,
    size: (usize, usize),
    /// Never empty while the window is open: closing the last tab ends the loop.
    tabs: Vec<Tab>,
    active: usize,
    /// Every page loaded in this session, oldest first, shown on the history page.
    visits: Vec<Visit>,
    editing_address: bool,
    /// Whether the next typed character replaces the whole address, as after selecting it all.
    replace_address: bool,
    /// Bytes already fetched, so resizing or going back does not fetch them again.
    cache: HashMap<String, Option<Vec<u8>>>,
    mouse_was_down: bool,
    frame: Vec<u32>,
    dirty: bool,
}

/// Opens the browser window with one tab per address, or a start page, and runs until it is closed.
pub fn run(addresses: &[String]) -> Result<()> {
    let window_error = |error: minifb::Error| Error::Window(error.to_string());
    let size = (1100, 760);
    let options = WindowOptions {
        resize: true,
        ..WindowOptions::default()
    };
    let mut window = Window::new("Wavelength", size.0, size.1, options).map_err(window_error)?;
    window.set_target_fps(60);
    let (sender, typed) = mpsc::channel();
    window.set_input_callback(Box::new(Typed(sender)));

    let mut browser = Browser {
        window,
        typed,
        size,
        tabs: Vec::new(),
        active: 0,
        visits: Vec::new(),
        editing_address: false,
        replace_address: false,
        cache: HashMap::new(),
        mouse_was_down: false,
        frame: Vec::new(),
        dirty: true,
    };
    if addresses.is_empty() {
        browser.open_tab(START_URL);
    }
    for address in addresses {
        browser.open_tab(&url::from_input(address));
    }

    let snapshot = std::env::var_os(SNAPSHOT_ENV);
    while browser.window.is_open() && !browser.tabs.is_empty() {
        browser.handle_resize();
        browser.handle_keyboard();
        browser.handle_mouse();
        if browser.tabs.is_empty() {
            break;
        }
        if browser.dirty {
            browser.redraw();
        }
        let (width, height) = browser.size;
        browser
            .window
            .update_with_buffer(&browser.frame, width, height)
            .map_err(window_error)?;
        if let Some(path) = &snapshot {
            return browser.save_frame(std::path::Path::new(path));
        }
    }
    Ok(())
}

fn page_viewport(size: (usize, usize)) -> Viewport {
    Viewport {
        width: (size.0 as u32).max(1),
        height: (size.1 as u32).saturating_sub(CHROME_HEIGHT).max(1),
    }
}

/// Shortens `text` until it fits in `room` pixels, marking the cut with an ellipsis.
fn fit(fonts: &Fonts, text: &str, size: f32, room: f32) -> String {
    if fonts.measure(text, size, false) <= room {
        return text.to_owned();
    }
    let mut shortened = text.to_owned();
    while !shortened.is_empty() && fonts.measure(&format!("{shortened}…"), size, false) > room {
        shortened.pop();
    }
    format!("{shortened}…")
}

impl Browser {
    fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    /// The width of one tab in the strip, which shrinks as more tabs are opened.
    fn tab_width(&self) -> f32 {
        let room = (self.size.0 as f32 - NEW_TAB_WIDTH).max(0.0);
        (room / self.tabs.len().max(1) as f32).min(MAX_TAB_WIDTH)
    }

    /// Opens a new tab at the end of the strip, switches to it and loads `target`.
    fn open_tab(&mut self, target: &str) {
        self.tabs.push(Tab {
            page: Page::load("", page_viewport(self.size)),
            address: String::new(),
            title: String::new(),
            scroll: 0.0,
            history: History::default(),
        });
        self.active = self.tabs.len() - 1;
        self.navigate(target, true);
    }

    /// Closes a tab. Closing the last one leaves no tabs, which ends the session.
    fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        if index < self.active || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        if !self.tabs.is_empty() {
            self.switch_tab(self.active);
        }
    }

    fn switch_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.active = index;
        self.editing_address = false;
        // The window may have been resized while this tab was in the background.
        let viewport = page_viewport(self.size);
        self.tab_mut().page.resize(viewport);
        self.scroll_by(0.0);
        self.update_title();
        self.dirty = true;
    }

    fn update_title(&mut self) {
        let title = self.tab().title.clone();
        let separator = if title.is_empty() { "" } else { " · " };
        self.window
            .set_title(&format!("{title}{separator}Wavelength"));
    }

    /// Fetches and lays out `target`, returning the page and the address to show for it.
    fn load(&mut self, target: &str) -> (Page, String) {
        let viewport = page_viewport(self.size);
        // Work out what to show first: the markup, the address it resolves links against,
        // and the address to display.
        let mut visited = false;
        let (html, base, shown) = match target {
            START_URL => (START_PAGE.to_owned(), START_URL.to_owned(), String::new()),
            HISTORY_URL => (
                session::history_page(&self.visits),
                HISTORY_URL.to_owned(),
                HISTORY_URL.to_owned(),
            ),
            _ => match net::fetch(target) {
                Ok(response) => {
                    visited = true;
                    (response.text(), response.url.clone(), response.url)
                }
                Err(error) => {
                    let html = format!(
                        "<title>Not loaded</title><body style='margin: 60px auto; max-width: 560px'>\
                         <h1 style='font-size: 24px'>This page could not be loaded</h1><p>{}</p>",
                        session::escape(&error.to_string())
                    );
                    (html, target.to_owned(), target.to_owned())
                }
            },
        };

        // Fetch whatever the page needs that is not cached yet, several at a time.
        let missing: Vec<String> = Page::resource_urls(&html, &base)
            .into_iter()
            .filter(|resource| !self.cache.contains_key(resource))
            .collect();
        self.cache.extend(net::fetch_all(&missing));
        let cache = &self.cache;
        let mut loader = |resource: &str| cache.get(resource).cloned().flatten();
        let page = Page::open(&html, &base, viewport, &mut loader);
        if visited {
            self.visits.push(Visit {
                url: shown.clone(),
                title: page.title().unwrap_or_default(),
            });
        }
        (page, shown)
    }

    /// Loads `target` in the current tab. `record` adds it to the tab's back and forward list.
    fn navigate(&mut self, target: &str, record: bool) {
        self.tab_mut().address = "Loading…".to_owned();
        self.editing_address = false;
        self.redraw();
        let (width, height) = self.size;
        let _ = self.window.update_with_buffer(&self.frame, width, height);

        let (page, shown_url) = self.load(target);
        let tab = self.tab_mut();
        if record {
            tab.history.visit(target);
        }
        tab.title = page.title().unwrap_or_else(|| shown_url.clone());
        tab.page = page;
        tab.address = shown_url;
        tab.scroll = 0.0;
        self.update_title();
        self.dirty = true;
    }

    fn go_back(&mut self) {
        if let Some(previous) = self.tab_mut().history.back().map(str::to_owned) {
            self.navigate(&previous, false);
        }
    }

    fn go_forward(&mut self) {
        if let Some(next) = self.tab_mut().history.forward().map(str::to_owned) {
            self.navigate(&next, false);
        }
    }

    fn reload(&mut self) {
        if let Some(current) = self.tab().history.current().map(str::to_owned) {
            self.cache.clear();
            self.navigate(&current, false);
        }
    }

    fn handle_resize(&mut self) {
        let size = self.window.get_size();
        if size != self.size && size.0 > 0 && size.1 > 0 {
            self.size = size;
            self.tab_mut().page.resize(page_viewport(size));
            self.scroll_by(0.0);
            self.dirty = true;
        }
    }

    fn scroll_by(&mut self, delta: f32) {
        let view_height = page_viewport(self.size).height as f32;
        let tab = self.tab_mut();
        let limit = (tab.page.content_height() - view_height).max(0.0);
        let target = (tab.scroll + delta).clamp(0.0, limit);
        if target != tab.scroll {
            tab.scroll = target;
            self.dirty = true;
        }
    }

    /// Applies an edit to whichever text has keyboard focus.
    fn edit_text(&mut self, edit: impl FnOnce(&mut String)) {
        let (editing, replace) = (self.editing_address, self.replace_address);
        let tab = self.tab_mut();
        if editing {
            if replace {
                tab.address.clear();
            }
            edit(&mut tab.address);
            self.replace_address = false;
            self.dirty = true;
        } else if let Some(field) = tab.page.focused() {
            let mut value = tab.page.value(field);
            edit(&mut value);
            tab.page.set_value(field, value);
            self.dirty = true;
        }
    }

    fn focus_address(&mut self) {
        self.tab_mut().page.focus(None);
        self.editing_address = true;
        self.replace_address = true;
        self.dirty = true;
    }

    fn handle_keyboard(&mut self) {
        let command = [
            Key::LeftSuper,
            Key::RightSuper,
            Key::LeftCtrl,
            Key::RightCtrl,
        ]
        .into_iter()
        .any(|key| self.window.is_key_down(key));
        let typed: Vec<char> = self.typed.try_iter().collect();
        if !command {
            for character in typed {
                self.edit_text(|text| text.push(character));
            }
        }

        let page_height = page_viewport(self.size).height as f32;
        for key in self.window.get_keys_pressed(KeyRepeat::Yes) {
            if self.tabs.is_empty() {
                return;
            }
            let typing = self.editing_address || self.tab().page.focused().is_some();
            match key {
                Key::T if command => {
                    self.open_tab(START_URL);
                    self.focus_address();
                }
                Key::W if command => self.close_tab(self.active),
                Key::Tab if command => self.switch_tab((self.active + 1) % self.tabs.len()),
                Key::L if command => self.focus_address(),
                Key::R if command => self.reload(),
                Key::Y if command => self.navigate(HISTORY_URL, true),
                Key::LeftBracket if command => self.go_back(),
                Key::RightBracket if command => self.go_forward(),
                Key::Key1
                | Key::Key2
                | Key::Key3
                | Key::Key4
                | Key::Key5
                | Key::Key6
                | Key::Key7
                | Key::Key8
                | Key::Key9
                    if command =>
                {
                    self.switch_tab(key as usize - Key::Key1 as usize);
                }
                Key::Backspace => self.edit_text(|text| {
                    text.pop();
                }),
                Key::Enter | Key::NumPadEnter => self.submit(),
                Key::Escape => {
                    self.editing_address = false;
                    let tab = self.tab_mut();
                    let shown = tab.page.url().filter(|url| *url != START_URL);
                    tab.address = shown.unwrap_or_default().to_owned();
                    tab.page.focus(None);
                    self.dirty = true;
                }
                Key::Down => self.scroll_by(LINE_SCROLL),
                Key::Up => self.scroll_by(-LINE_SCROLL),
                Key::PageDown => self.scroll_by(page_height * 0.9),
                Key::PageUp => self.scroll_by(-page_height * 0.9),
                Key::Space if !typing => self.scroll_by(page_height * 0.9),
                Key::Home if !typing => self.scroll_by(f32::NEG_INFINITY),
                Key::End if !typing => self.scroll_by(f32::INFINITY),
                _ => {}
            }
        }
    }

    /// Enter was pressed: load the typed address, or submit the form around the focused field.
    fn submit(&mut self) {
        if self.editing_address {
            let target = url::from_input(&self.tab().address);
            self.navigate(&target, true);
        } else {
            let page = &self.tab().page;
            if let Some(target) = page.focused().and_then(|field| page.submit_url(field)) {
                self.navigate(&target, true);
            }
        }
    }

    fn handle_mouse(&mut self) {
        if let Some((_, vertical)) = self.window.get_scroll_wheel() {
            self.scroll_by(-vertical * WHEEL_SCROLL);
        }
        let down = self.window.get_mouse_down(MouseButton::Left);
        let pressed = down && !self.mouse_was_down;
        self.mouse_was_down = down;
        let Some((x, y)) = self
            .window
            .get_mouse_pos(MouseMode::Discard)
            .filter(|_| pressed)
        else {
            return;
        };

        if y < TAB_STRIP_HEIGHT as f32 {
            self.click_tab_strip(x);
        } else if y < CHROME_HEIGHT as f32 {
            self.click_toolbar(x);
        } else {
            self.click_page(x, y - CHROME_HEIGHT as f32);
        }
    }

    fn click_tab_strip(&mut self, x: f32) {
        let tab_width = self.tab_width();
        let index = (x / tab_width.max(1.0)) as usize;
        if index >= self.tabs.len() {
            if x < self.tabs.len() as f32 * tab_width + NEW_TAB_WIDTH {
                self.open_tab(START_URL);
                self.focus_address();
            }
        } else if x >= (index + 1) as f32 * tab_width - TAB_CLOSE_WIDTH {
            self.close_tab(index);
        } else {
            self.switch_tab(index);
        }
    }

    fn click_toolbar(&mut self, x: f32) {
        if x < NAV_BUTTON_WIDTH {
            self.go_back();
        } else if x < NAV_BUTTON_WIDTH * 2.0 {
            self.go_forward();
        } else if x >= self.size.0 as f32 - HISTORY_BUTTON_WIDTH {
            self.navigate(HISTORY_URL, true);
        } else {
            self.focus_address();
        }
    }

    /// A click on the page, in viewport coordinates.
    fn click_page(&mut self, x: f32, y: f32) {
        let new_tab =
            self.window.is_key_down(Key::LeftSuper) || self.window.is_key_down(Key::RightSuper);
        self.editing_address = false;
        self.dirty = true;
        let tab = self.tab_mut();
        let page_y = y + tab.scroll;
        if let Some(control) = tab.page.control_at(x, page_y) {
            if tab.page.is_text_field(control) {
                tab.page.focus(Some(control));
            } else if let Some(target) = tab.page.submit_url(control) {
                self.navigate(&target, true);
            }
        } else if let Some(target) = tab.page.link_at(x, page_y) {
            tab.page.focus(None);
            if new_tab {
                self.open_tab(&target);
            } else {
                self.navigate(&target, true);
            }
        } else {
            tab.page.focus(None);
        }
    }

    /// Draws the tab strip, the toolbar and the visible part of the page into the frame buffer.
    fn redraw(&mut self) {
        let (width, height) = (self.size.0 as u32, self.size.1 as u32);
        let mut chrome = Image::new(width, CHROME_HEIGHT.min(height), TOOLBAR_COLOR);
        self.draw_tab_strip(&mut chrome);
        self.draw_toolbar(&mut chrome);

        let tab = self.tab();
        let page = tab.page.render_at(tab.scroll);
        self.frame.clear();
        self.frame.reserve(self.size.0 * self.size.1);
        for image in [&chrome, &page] {
            let pixels = image.data().chunks_exact(4);
            self.frame
                .extend(pixels.map(|p| u32::from_be_bytes([0, p[0], p[1], p[2]])));
        }
        // A window shorter than the chrome leaves the buffer the wrong length without this.
        self.frame.resize(self.size.0 * self.size.1, 0x00FF_FFFF);
        self.dirty = false;
    }

    fn draw_tab_strip(&self, chrome: &mut Image) {
        let fonts = Fonts::system();
        let width = self.size.0 as u32;
        chrome.fill(0, 0, width, TAB_STRIP_HEIGHT, STRIP_COLOR);
        let tab_width = self.tab_width();
        let text = |color| TextStyle {
            size: TAB_TITLE_SIZE,
            face: Face::REGULAR,
            color,
        };
        for (index, tab) in self.tabs.iter().enumerate() {
            let left = index as f32 * tab_width;
            let right = left + tab_width;
            let is_active = index == self.active;
            if is_active {
                chrome.fill(
                    left as u32,
                    4,
                    right as u32,
                    TAB_STRIP_HEIGHT,
                    TOOLBAR_COLOR,
                );
            } else {
                chrome.fill(
                    right as u32 - 1,
                    9,
                    right as u32,
                    TAB_STRIP_HEIGHT - 7,
                    DIVIDER_COLOR,
                );
            }
            let title = if tab.title.is_empty() {
                "New tab"
            } else {
                &tab.title
            };
            let room = (tab_width - TAB_CLOSE_WIDTH - 14.0).max(0.0);
            let color = if is_active {
                TEXT_COLOR
            } else {
                Color::rgb(70, 73, 78)
            };
            let label = fit(fonts, title, TAB_TITLE_SIZE, room);
            paint::draw_text(chrome, fonts, left + 12.0, 22.0, &label, text(color));
            paint::draw_text(
                chrome,
                fonts,
                right - TAB_CLOSE_WIDTH + 8.0,
                22.0,
                "x",
                text(MUTED_COLOR),
            );
        }
        let plus = TextStyle {
            size: 18.0,
            face: Face::REGULAR,
            color: TEXT_COLOR,
        };
        let plus_x = self.tabs.len() as f32 * tab_width + 12.0;
        paint::draw_text(chrome, fonts, plus_x, 24.0, "+", plus);
    }

    fn draw_toolbar(&self, chrome: &mut Image) {
        let fonts = Fonts::system();
        let width = self.size.0 as u32;
        let top = TAB_STRIP_HEIGHT;
        let tab = self.tab();
        let baseline = top as f32 + 27.0;

        let arrow = |enabled: bool| TextStyle {
            size: 20.0,
            face: Face::REGULAR,
            color: if enabled { TEXT_COLOR } else { MUTED_COLOR },
        };
        paint::draw_text(
            chrome,
            fonts,
            11.0,
            baseline + 2.0,
            "<",
            arrow(tab.history.can_go_back()),
        );
        let forward_x = NAV_BUTTON_WIDTH + 11.0;
        paint::draw_text(
            chrome,
            fonts,
            forward_x,
            baseline + 2.0,
            ">",
            arrow(tab.history.can_go_forward()),
        );

        let label = TextStyle {
            size: 13.0,
            face: Face::REGULAR,
            color: TEXT_COLOR,
        };
        let history_x = width as f32 - HISTORY_BUTTON_WIDTH + 14.0;
        paint::draw_text(chrome, fonts, history_x, baseline, "History", label);

        let field_left = (NAV_BUTTON_WIDTH * 2.0) as u32 + 4;
        let field_right = width.saturating_sub(HISTORY_BUTTON_WIDTH as u32);
        let border = if self.editing_address {
            FOCUS_BORDER
        } else {
            DIVIDER_COLOR
        };
        chrome.fill(field_left, top + 7, field_right, top + 37, border);
        chrome.fill(
            field_left + 1,
            top + 8,
            field_right.saturating_sub(1),
            top + 36,
            FIELD_COLOR,
        );

        let placeholder = tab.address.is_empty() && !self.editing_address;
        let mut shown = if placeholder {
            "Type an address and press Enter".to_owned()
        } else {
            tab.address.clone()
        };
        let room = (field_right.saturating_sub(field_left) as f32 - 24.0).max(0.0);
        if self.editing_address {
            // Keep the end of a long address visible while typing.
            while fonts.measure(&shown, ADDRESS_SIZE, false) > room && !shown.is_empty() {
                shown.remove(0);
            }
            shown.push('|');
        } else {
            shown = fit(fonts, &shown, ADDRESS_SIZE, room);
        }
        let style = TextStyle {
            size: ADDRESS_SIZE,
            face: Face::REGULAR,
            color: if placeholder { MUTED_COLOR } else { TEXT_COLOR },
        };
        paint::draw_text(
            chrome,
            fonts,
            field_left as f32 + 10.0,
            baseline,
            &shown,
            style,
        );
    }

    fn save_frame(&self, path: &std::path::Path) -> Result<()> {
        let (width, height) = (self.size.0 as u32, self.size.1 as u32);
        let mut image = Image::new(width, height, [0, 0, 0, 255]);
        for (index, pixel) in self.frame.iter().enumerate() {
            let [_, r, g, b] = pixel.to_be_bytes();
            let (x, y) = ((index % self.size.0) as u32, (index / self.size.0) as u32);
            image.fill(x, y, x + 1, y + 1, [r, g, b, 255]);
        }
        image.save(path)
    }
}
