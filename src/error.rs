//! The crate-wide error type.

use std::fmt;

/// A specialised `Result` for this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong outside the engine's core, which itself never fails.
#[derive(Debug)]
pub enum Error {
    /// A file could not be read or written.
    Io(std::io::Error),
    /// A PNG file could not be encoded or decoded.
    Image(String),
    /// A selector passed to the page API is not supported.
    Selector(String),
    /// Chrome could not produce a reference screenshot.
    Chrome(String),
    /// No browser is installed and no earlier reference exists to fall back on.
    BrowserNotFound,
    /// A page differs from the reference by more than the allowed amount.
    Parity(String),
    /// The command line was used incorrectly.
    Usage(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Image(message) => write!(f, "image error: {message}"),
            Self::Selector(selector) => write!(f, "unsupported selector: {selector}"),
            Self::Chrome(message) => write!(f, "Chrome reference failed: {message}"),
            Self::BrowserNotFound => write!(
                f,
                "no Chrome or Chromium found for reference screenshots; set {} to its path",
                crate::chrome::CHROME_ENV
            ),
            Self::Parity(message) | Self::Usage(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
