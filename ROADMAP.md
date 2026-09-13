# Roadmap

Everything here is not implemented yet. The milestones are roughly in the order they will be built, and the parity milestones each have a bar that can be checked on a machine with no internet access. The end goal is that a saved news page rendered by this engine and overlaid on Chrome's screenshot barely differs by a pixel.

What already works is described in the README: a simplified HTML parser, basic selectors and cascade, block and inline layout with text, simple tables and form controls, loading over HTTP and HTTPS, a browser window on macOS with tabs and session history, and the parity report. Each of those is a first version, and the sections below say what each still lacks.

## 1. An HTML parser that passes html5lib

The current parser handles well-formed markup and a few common shortcuts. It needs to be replaced by the tokenizer and tree construction algorithm from the WHATWG specification.

- The full tokenizer state machine, including every named character reference
- Tree construction with insertion modes, the adoption agency algorithm and foster parenting, plus implied `tbody` elements
- Comment and doctype nodes in the DOM, and quirks mode detection
- Foreign content (SVG and MathML), `<template>`, and fragment parsing
- Character encoding detection from `<meta>` and byte order marks, and encodings beyond UTF-8 and Latin-1

Done when the html5lib tokenizer and tree-construction test suites pass, with a runner in this repository that reports any skipped test.

## 2. Text that matches Chrome

Text is drawn with the engine's own rasteriser, but its pixels do not match Chrome's, and one sans-serif family stands in for every font.

- `font-family` matching and fallback, italic faces, and web fonts through `@font-face`
- OpenType fonts with CFF outlines, and TrueType hinting where Chrome applies it
- Shaping: kerning and ligatures for Latin text, then complex scripts and bidirectional text
- Antialiasing, gamma and sub-pixel glyph positioning that match Chrome's on the same fonts
- `font-style`, `letter-spacing`, `word-spacing`, `text-transform`, `text-indent`, `text-shadow`
- Preserved whitespace and newlines for `pre`, breaking inside long words, hyphenation, `text-overflow`
- List markers for `ul` and `ol`

Done when a set of text-only fixtures using bundled, openly licensed fonts stays within an agreed pixel tolerance of Chrome.

## 3. The full cascade

- Remaining selectors: child, sibling and attribute selectors, pseudo-classes and pseudo-elements
- `inherit`, `initial`, `unset`, and inheritance for every inherited property
- Custom properties and `var()`, `calc()`, viewport units (`vw`, `vh`), and percentages for margins and padding
- At-rules: `@media`, `@import`, `@supports`, `@font-face`
- Colour functions such as `hsl()` and `light-dark()`, `currentcolor`, and the full set of named colours
- A user-agent stylesheet that covers every HTML element exactly as Chrome's does
- A real CSS tokenizer in place of the current string splitting, with correct error recovery

## 4. Block, inline and float layout

- Margin collapsing between parents and children and through empty boxes
- `box-sizing`, `min-width`, `min-height`, `max-height`, percentage heights
- Inline boxes with their own backgrounds, borders, padding and margins, and `vertical-align` within a line
- Floats and `clear`
- Positioning (`relative`, `absolute`, `fixed`, `sticky`) and `z-index` stacking contexts
- `overflow` and clipping, including text that overflows form fields
- `border-radius`, dashed and dotted borders, mitred border corners
- Replaced elements: JPEG, GIF, WebP and SVG images, `alt` text, `<select>` and `<textarea>` contents, checked states
- Sub-pixel layout in 1/64 pixel units, the way Chrome does it, so fractional lengths snap to the same device pixels

## 5. Flexbox, grid and tables

Flex and grid containers are laid out as plain blocks today, and tables use a simplified algorithm.

- Flexbox, including wrapping, alignment and `gap`
- Grid, including named areas, `fr` units, auto-placement and `minmax()`
- Table layout as specified: `colspan`, `rowspan`, `cellpadding`, captions, collapsed borders, fixed layout, baseline alignment
- Caching of intrinsic sizes, since nested shrink-to-fit boxes are measured again at every level

## 6. Painting

- Background images, gradients, `background-position`, `background-size` and `background-repeat`
- `opacity`, `box-shadow`, `outline`, transforms and filters
- Smooth image scaling in place of nearest-neighbour sampling
- A device pixel ratio other than 1, so the window is sharp on high-density displays

## 7. Parity at scale

Three fixtures prove very little. The comparison harness needs to grow with the engine.

- A reftest runner for the CSS tests from web-platform-tests, working from a local copy
- A corpus of saved real pages with their resources, for whole-page comparison
- A richer report: an overlay slider and blink toggle, per-region diff statistics, and a history of results across commits
- A tolerance mode for antialiased edges alongside the exact mode
- Reference screenshots through Chrome's DevTools protocol, so the viewport, fonts and timing are controlled exactly
- A timeout around the Chrome process, and an option to pass it extra flags such as `--no-sandbox` where the environment needs it
- The parity test running in CI, which today only runs the tests that need no browser

Done when 5,000 web-platform CSS reftests pass and 100 saved pages render within 2% pixel difference of Chrome with the same fonts.

## 8. JavaScript

Nothing on a page is executed, which is the main reason modern sites look incomplete. Google's search results, for example, cannot be shown.

- A JavaScript engine: parser, interpreter, garbage collector and the standard built-ins
- DOM and CSSOM bindings, events and timers
- Script loading, including `async`, `defer` and modules
- `fetch`, `XMLHttpRequest`, storage and the other web APIs pages rely on
- Restyle and relayout after the DOM changes

## 9. Networking

Pages load through the system's `curl` program, one request at a time, with no state kept between them.

- A native HTTP client, so the engine does not depend on an external program
- Loading that does not block the window, with resources fetched in parallel and a progress indicator
- Cookies, with a per-site jar and controls over what is kept
- An HTTP cache that honours cache headers
- Form submission with POST, file uploads, `<meta http-equiv="refresh">` and `<base href>`
- Content blocking for trackers, and per-site permissions

## 10. The browser window

- History that survives a restart, with search and a way to clear it, and bookmarks
- Tab features beyond the basics: reordering, reopening a closed tab, scrolling the strip when it is full, and loading in background tabs
- Text selection, copy and paste, and a movable caret in text fields
- Find in page, zoom, and a context menu
- Hover and focus styles, and a pointer cursor over links
- Window support on Linux and Windows, which is untested
- A signed macOS app, so installing it does not trigger system warnings

## 11. Agent integration

`Page` already exposes screenshots, selector queries, element bounds, text, hit testing, links and form fields. The layers below build on that API and do not change the engine.

- An accessibility-style outline of the page (roles, names, bounds) as structured data for agents
- An MCP server over stdio that exposes loading, rendering, querying and interaction as tools
- Policies for what an agent may load, such as allowed hosts and a block on private network addresses
- A stable, versioned public API, with the pipeline modules made private behind it

## 12. Speed

- Benchmarks for parse, style, layout and paint on the page corpus, tracked across commits
- Selector matching with rule hashing and a bloom filter for ancestors
- Incremental style and layout after changes, where today every keystroke in a form lays the page out again
- Painting only what changed while scrolling
- Parallel style and layout, and SIMD rasterisation

## Engineering

- Tests for the window, which is only checked by hand and by a single start-up snapshot today
- Fuzzing for the HTML, CSS and font parsers
- `cargo deny` and `cargo audit` in CI
- A workspace split (engine, harness, CLI, window) once the crate is large enough to need it
- Prebuilt release binaries
