# Roadmap

Everything here is not implemented yet. The milestones are in the order they will be built, and each one has a bar that can be checked on a machine with no internet access. The end goal is that a saved news page rendered by this engine and overlaid on Chrome's screenshot barely differs by a pixel.

The first milestone, already done, is described in the README: a simplified HTML parser, basic selectors and cascade, block layout, solid backgrounds and borders, and the parity report.

## 1. An HTML parser that passes html5lib

The current parser handles well-formed markup and nothing more. It needs to be replaced by the tokenizer and tree construction algorithm from the WHATWG specification.

- The full tokenizer state machine, including every named character reference
- Tree construction with insertion modes, implied end tags, the adoption agency algorithm and foster parenting
- Comment and doctype nodes in the DOM, and quirks mode detection
- Foreign content (SVG and MathML), `<template>`, and fragment parsing
- Character encoding detection, since input is assumed to be UTF-8 today

Done when the html5lib tokenizer and tree-construction test suites pass, with a runner in this repository that reports any skipped test.

## 2. Text

Nothing draws text yet, and no real page can match without it. This is likely the hardest milestone, because glyph outlines have to be rasterised the way Chrome's text stack does it for pixels to line up.

- Font loading (TrueType and OpenType), `font-family` matching and fallback
- Shaping, at least kerning and ligatures for Latin text
- Glyph rasterisation with antialiasing that matches Chrome's on the same fonts
- `font-size`, `font-weight`, `font-style`, `line-height`, `text-align`, `text-decoration`, `white-space`
- Font-relative units (`em`, `rem`, `ex`, `ch`)

Done when a set of text-only fixtures using bundled, openly licensed fonts stays within an agreed pixel tolerance of Chrome.

## 3. The full cascade

- Remaining selectors: child, sibling and attribute selectors, pseudo-classes and pseudo-elements
- `!important`, `inherit`, `initial`, `unset`, and inheritance for every inherited property
- Custom properties and `var()`, `calc()`, percentages and the remaining length units
- At-rules: `@media`, `@import`, `@supports`, `@font-face`
- External stylesheets through `<link>`, and loading resources relative to the page
- A user-agent stylesheet that covers every HTML element, including headings, lists and form controls
- A real CSS tokenizer in place of the current string splitting, with correct error recovery

## 4. Block, inline and float layout

- Margin collapsing between siblings, between parents and children, and through empty boxes
- `box-sizing`, `min-width`, `max-width`, `min-height`, `max-height`, percentage sizes
- Per-side border widths, styles and colours, and `border-radius`
- Inline formatting contexts: line boxes, inline boxes, `inline-block`, `vertical-align`, anonymous block boxes
- Floats and `clear`
- Positioning (`relative`, `absolute`, `fixed`, `sticky`) and `z-index` stacking contexts
- `overflow`, clipping and scrolling, with rendering of pages taller than the viewport
- Replaced elements: `<img>` with PNG and JPEG decoding, and intrinsic sizes
- Sub-pixel layout in 1/64 pixel units, the way Chrome does it, so fractional lengths snap to the same device pixels

## 5. Flexbox, grid and tables

- Flexbox, including wrapping, alignment and `gap`
- Grid, including named areas, `fr` units, auto-placement and `minmax()`
- Table layout, both automatic and fixed, with borders collapsed and separated

## 6. Painting

- Alpha blending for `rgba()`, `hsl()` and `opacity`, plus the full set of named colours
- Background images, gradients, `background-position`, `background-size` and `background-repeat`
- `box-shadow`, `outline`, dashed and dotted borders, mitred border corners
- Transforms and filters
- A device pixel ratio other than 1

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

## 8. Agent integration

`Page` already exposes screenshots, selector queries, element bounds and text. The layers below build on that API and do not change the engine.

- An accessibility-style outline of the page (roles, names, bounds) as structured data for agents
- Hit testing, so a coordinate can be mapped back to an element
- Interaction: clicks, typing, focus, form state and navigation between local pages
- A JavaScript engine and DOM bindings, which most interactive pages need
- An MCP server over stdio that exposes rendering, querying and interaction as tools
- A stable, versioned public API, with the pipeline modules made private behind it

## 9. Speed

- Benchmarks for parse, style, layout and paint on the page corpus, tracked across commits
- Selector matching with rule hashing and a bloom filter for ancestors
- Incremental style and layout after DOM changes, where today every load starts from nothing
- Parallel style and layout, and SIMD rasterisation

## Engineering

- Fuzzing for the HTML and CSS parsers
- `cargo deny` and `cargo audit` in CI
- A workspace split (engine, harness, CLI) once the crate is large enough to need it
- Prebuilt release binaries
- Windows support, which is untested
