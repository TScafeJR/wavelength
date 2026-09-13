<p align="center"><img src="assets/logo.svg" alt="wavelength" width="420"></p>

# wavelength

A browser engine written from scratch in Rust, with one measure of progress: render a page, overlay the result on Chrome's screenshot of the same page, and count the pixels that differ. The long-term goal is an engine small and fast enough to embed in AI agents, which is why everything it knows about a page is already exposed through a small library API.

The engine parses HTML and CSS, runs the cascade, lays out blocks, text, inline-blocks, form controls and simple tables, and paints them with its own TrueType rasteriser. It can load pages over HTTP and HTTPS, and on macOS it comes with a window that has an address bar, so you can type `google.com` and see the page drawn by the engine. It does not run JavaScript and it knows only part of CSS, so most modern sites look plain or incomplete. [ROADMAP.md](ROADMAP.md) lists what is missing and the order it will be built in.

![A spectrum of coloured bars rendered by the engine](assets/spectrum.png)

The image above is `fixtures/spectrum.html` as painted by the engine. Chrome produces the same picture, pixel for pixel. Exact parity currently holds for the three fixture pages, which use only boxes, backgrounds and borders. Text is drawn but does not match Chrome's glyph rendering yet.

## Quick start

You need a Rust toolchain at version 1.88 or newer. Loading web pages uses the `curl` program that ships with macOS and most Linux systems, and Chrome or Chromium is only needed for the parity comparison.

```sh
make browse URL=google.com    # open the browser window (macOS)
make report                   # compare the fixtures against Chrome and open the report
make render PAGE=https://example.com OUT=example.png
```

Without `make`, the same things are:

```sh
cargo run --release --features window -- browse google.com
cargo run --release -- compare fixtures/*.html
cargo run --release -- render https://example.com --out example.png --width 1000 --height 700
```

### The browser window

`browse` opens a window with a tab strip and a toolbar above the page, and opens one tab for each address you pass it. Type an address and press Enter, click links to follow them, click a text field to type into it and press Enter to submit its form. The mouse wheel, arrow keys, Page Up and Page Down scroll.

Each tab keeps its own back and forward list, which the `<` and `>` buttons or Cmd+[ and Cmd+] walk through. The History button, or Cmd+Y, lists every page opened since the window started, newest first, and each entry is a link back to that page. History lives in memory only and is gone when the window closes.

Cmd+T opens a tab, Cmd+W closes the current one, Cmd+1 to Cmd+9 and Ctrl+Tab switch between them, and holding Cmd while clicking a link opens it in a new tab. Cmd+L focuses the address bar and Cmd+R reloads.

Loading is synchronous, so the window does not respond while a page is being fetched. Requests send a generic user agent and no cookies, and nothing is kept between pages. Many sites, Google among them, answer an unknown browser with a simpler page, which is the version the engine can draw. Google's search results need JavaScript, so searching from its home page leads to a page that says so.

### The parity report

`compare` renders each page with the engine and with headless Chrome, then writes both images, a diff overlay and an `index.html` that shows the three side by side with the number of differing pixels. Pass `--max-diff 0` to make the command fail when any page is not an exact match, which is how the test suite uses it. If Chrome is installed somewhere unusual, set `WAVELENGTH_CHROME` to the path of its binary. When no browser is found, `compare` reuses the reference screenshots left in the output directory by an earlier run.

## Using it as a library

`Page` wraps the whole pipeline, and it is the surface an agent integration is meant to build on. It gives an agent a screenshot, a way to find elements, the position of each element on screen and the page's text, and it lets the agent follow links and fill in forms.

```rust
use wavelength::{Page, Viewport};

let mut page = Page::fetch("https://www.google.com", Viewport::default())?;
let image = page.render();                          // RGBA pixels, savable as PNG
let text = page.text();                             // the text of the body
let fields = page.select("input")?;                 // elements matching a selector
let bounds = page.bounds(fields[0]);                // where an element is drawn
let link = page.link_at(60.0, 20.0);                // where a click would lead
page.set_value(fields[0], "rust".to_owned());       // type into a form field
let next = page.submit_url(fields[0]);              // the URL the form would load
```

`Page::load` does the same for an HTML string without touching the network. An MCP server and other agent-facing layers are planned on top of this API and are tracked in the roadmap.

## How it works

The pipeline is a straight line, and each stage is one module in `src/`.

| Stage | Module | What it does today |
|---|---|---|
| Fetch | `net.rs`, `url.rs` | HTTP, HTTPS and local files, URL resolution, form encoding |
| Parse HTML | `html.rs`, `dom.rs` | Tags, attributes, text, raw-text elements, implied `html`, `head` and `body`, implied end tags for list items, cells and paragraphs |
| Parse CSS | `css.rs` | Style rules with type, class, id and descendant selectors, specificity, `!important` |
| Cascade | `style.rs` | User-agent sheet, linked and embedded stylesheets, `style` attributes, legacy attributes such as `bgcolor`, inheritance for text properties |
| Fonts | `font.rs` | TrueType parsing, text measurement, antialiased glyph rasterisation |
| Layout | `layout.rs` | Block flow with sibling margin collapsing, line breaking, inline-blocks, images, form controls, simplified tables |
| Paint | `paint.rs`, `image.rs` | Backgrounds, per-side borders, text, PNG images, alpha blending |
| Window | `browse.rs`, `session.rs` | Tabs, address bar, back and forward, a history page, scrolling, links, form fields (optional `window` feature) |
| Verify | `chrome.rs`, `diff.rs`, `report.rs` | Chrome reference screenshots, pixel diff, HTML report |

One sans-serif family stands in for every font a page asks for: Arial on macOS, Liberation Sans or DejaVu Sans on Linux. Set `WAVELENGTH_FONT` and `WAVELENGTH_FONT_BOLD` to `.ttf` files to use others.

## Development

A `Makefile` wraps the common tasks, and `make help` lists them. `make check` runs everything below in one go, and `make report` runs the comparison and opens the result.

On macOS, `make build` also assembles `Wavelength.app` and copies it into `/Applications`. Opening the app starts the browser window. The installed copy is only replaced when the build has changed. When it is, macOS may say your terminal was prevented from modifying apps, and allowing the terminal under System Settings, Privacy & Security, App Management stops that. Use `make binary` to build without installing, `make build APP_DIR=~/Applications` to install elsewhere, and `make uninstall` to remove it.

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test                      # unit tests and the doc example
cargo test -- --include-ignored # also checks the fixtures against a local Chrome
```

Add `--features window` to include the browser window in any of these. The crate forbids `unsafe` code and treats missing documentation as a warning. Parsing, layout and painting never return errors, since a browser has to do something sensible with any input, and the error type only covers files, images, the network and the command line.

### Dependencies

The engine has no dependencies. The crate has two: `png` reads and writes image files, and `minifb` opens the window when the optional `window` feature is on. Versions are pinned exactly in `Cargo.toml` and `Cargo.lock` is committed. Please keep it that way and prefer writing a small amount of code over adding a crate.

## License

[MIT](LICENSE)
