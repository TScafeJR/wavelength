//! Reference screenshots from a locally installed Chrome or Chromium.
//!
//! Chrome is only ever used as the yardstick. The engine does not depend on it.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::{Error, Result};
use crate::page::Viewport;

/// Set this environment variable to the browser binary to use for references.
pub const CHROME_ENV: &str = "WAVELENGTH_CHROME";

const MAC_APPLICATIONS: &[&str] = &[
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
];
const PATH_NAMES: &[&str] = &[
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
];

/// Finds a browser binary: the [`CHROME_ENV`] override first, then the usual install locations.
pub fn find() -> Option<PathBuf> {
    if let Some(path) = env::var_os(CHROME_ENV) {
        return Some(PathBuf::from(path));
    }
    let applications = MAC_APPLICATIONS.iter().map(PathBuf::from);
    let search_path = env::var_os("PATH").unwrap_or_default();
    let on_path = env::split_paths(&search_path)
        .flat_map(|dir| PATH_NAMES.iter().map(move |name| dir.join(name)));
    applications.chain(on_path).find(|path| path.is_file())
}

/// Screenshots `page` with headless Chrome and writes the PNG to `output`.
///
/// The browser runs with a throwaway profile in `scratch`, which is removed afterwards.
pub fn screenshot(
    chrome: &Path,
    page: &Path,
    viewport: Viewport,
    output: &Path,
    scratch: &Path,
) -> Result<()> {
    let page = fs::canonicalize(page)?;
    let profile = scratch.join(".chrome-profile");
    // Chrome resolves the screenshot path against its own working directory.
    let output = std::path::absolute(output)?;
    let _ = fs::remove_file(&output);

    let result = Command::new(chrome)
        .arg("--headless")
        .arg("--hide-scrollbars")
        .arg("--force-device-scale-factor=1")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-background-networking")
        .arg("--password-store=basic")
        .arg("--use-mock-keychain")
        .arg(format!(
            "--window-size={},{}",
            viewport.width, viewport.height
        ))
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(format!("--screenshot={}", output.display()))
        .arg(file_url(&page))
        .stdin(Stdio::null())
        .output();
    let _ = fs::remove_dir_all(&profile);

    let result = result.map_err(|e| Error::Chrome(format!("{}: {e}", chrome.display())))?;
    if !output.is_file() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let last_line = stderr.lines().last().unwrap_or("no output");
        return Err(Error::Chrome(format!(
            "no screenshot was written for {} ({}): {last_line}",
            page.display(),
            result.status
        )));
    }
    Ok(())
}

/// Builds a `file://` URL, percent-encoding everything outside the unreserved set.
fn file_url(path: &Path) -> String {
    let mut url = String::from("file://");
    for &byte in path.as_os_str().as_encoded_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            url.push(byte as char);
        } else {
            url.push_str(&format!("%{byte:02X}"));
        }
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_urls_are_percent_encoded() {
        assert_eq!(
            file_url(Path::new("/tmp/a b/é#1.html")),
            "file:///tmp/a%20b/%C3%A9%231.html"
        );
    }
}
