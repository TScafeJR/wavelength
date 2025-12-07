//! A browser engine written from scratch, measured against Chrome pixel by pixel.
//!
//! The pipeline is a straight line, and each stage is its own module:
//!
//! 1. [`html`] parses markup into a [`dom::Document`].
//! 2. [`css`] parses stylesheets and matches selectors.
//! 3. [`style`] runs the cascade and computes a style for every node.
//! 4. [`layout`] turns styled elements into positioned boxes.
//! 5. [`paint`] draws those boxes into an [`image::Image`].
//!
//! [`page::Page`] wraps the whole pipeline behind one type. The [`chrome`],
//! [`diff`] and [`report`] modules exist to check the result against a real
//! browser and are not part of the engine itself.
//!
//! ```
//! use wavelength::{Page, Viewport};
//!
//! let page = Page::load(
//!     "<div id='box' style='width: 100px; height: 50px; background: teal'></div>",
//!     Viewport { width: 320, height: 240 },
//! );
//! let boxes = page.select("#box").unwrap();
//! let bounds = page.bounds(boxes[0]).unwrap();
//! assert_eq!((bounds.x, bounds.y, bounds.width, bounds.height), (8.0, 8.0, 100.0, 50.0));
//! assert_eq!(page.render().pixel(8, 8), Some([0, 128, 128, 255]));
//! ```

pub mod chrome;
pub mod css;
pub mod diff;
pub mod dom;
pub mod error;
pub mod html;
pub mod image;
pub mod layout;
pub mod page;
pub mod paint;
pub mod report;
pub mod style;

pub use error::{Error, Result};
pub use page::{Page, Viewport};
