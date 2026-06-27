//! Fetching pages and their resources.
//!
//! HTTP and HTTPS go through the system's `curl` program, which keeps TLS and
//! the rest of the network stack out of this crate. Requests carry a generic
//! user agent and no cookies, and nothing is stored between them.

use std::process::{Command, Stdio};

use crate::error::{Error, Result};
use crate::url;

/// The user agent sent with every request. It names the engine and nothing about the machine.
pub const USER_AGENT: &str = concat!("wavelength/", env!("CARGO_PKG_VERSION"));

const TIMEOUT_SECONDS: &str = "30";
const MAX_BYTES: &str = "33554432";

/// A fetched resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// The URL the content came from, after any redirects.
    pub url: String,
    /// The `Content-Type` the server sent, or a guess for local files.
    pub content_type: String,
    /// The raw bytes.
    pub body: Vec<u8>,
}

impl Response {
    /// The body as text, decoded according to the declared character set.
    ///
    /// UTF-8 and Latin-1 are understood. Anything else is read as UTF-8 if it
    /// is valid and as Latin-1 otherwise.
    pub fn text(&self) -> String {
        let content_type = self.content_type.to_ascii_lowercase();
        let charset = content_type.split_once("charset=").map(|(_, charset)| {
            charset.trim_matches(|c: char| c == '"' || c.is_ascii_whitespace())
        });
        let latin1 = || self.body.iter().map(|&byte| byte as char).collect();
        match charset {
            Some("utf-8" | "utf8") => String::from_utf8_lossy(&self.body).into_owned(),
            Some("iso-8859-1" | "latin1" | "windows-1252" | "us-ascii") => latin1(),
            _ => String::from_utf8(self.body.clone()).unwrap_or_else(|_| latin1()),
        }
    }
}

/// Fetches an `http`, `https` or `file` URL.
pub fn fetch(target: &str) -> Result<Response> {
    if let Some(path) = target.strip_prefix("file://") {
        let path = url::percent_decode(path.split(['?', '#']).next().unwrap_or_default());
        let content_type = match path.rsplit('.').next() {
            Some("png") => "image/png",
            Some("css") => "text/css",
            _ => "text/html; charset=utf-8",
        };
        return Ok(Response {
            url: target.to_owned(),
            content_type: content_type.to_owned(),
            body: std::fs::read(&path).map_err(|e| Error::Network(format!("{path}: {e}")))?,
        });
    }
    if !(target.starts_with("http://") || target.starts_with("https://")) {
        return Err(Error::Network(format!(
            "{target}: only http, https and file URLs can be loaded"
        )));
    }

    let output = Command::new("curl")
        .args(["--silent", "--show-error", "--location", "--compressed"])
        .args([
            "--max-time",
            TIMEOUT_SECONDS,
            "--max-redirs",
            "10",
            "--max-filesize",
            MAX_BYTES,
        ])
        .args(["--proto", "=http,https", "--proto-redir", "=http,https"])
        .args(["--user-agent", USER_AGENT])
        // The body goes to stdout and the facts about the response to stderr.
        .args([
            "--write-out",
            "%{stderr}%{url_effective}\n%{content_type}\n",
        ])
        .arg("--")
        .arg(target)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| Error::Network(format!("could not run curl: {e}")))?;
    let report = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        let reason = report
            .lines()
            .find(|line| line.starts_with("curl:"))
            .unwrap_or("request failed");
        return Err(Error::Network(format!("{target}: {reason}")));
    }
    let mut lines = report.lines().rev();
    let content_type = lines.next().unwrap_or_default().to_owned();
    let final_url = lines
        .next()
        .filter(|line| !line.is_empty())
        .unwrap_or(target);
    Ok(Response {
        url: final_url.to_owned(),
        content_type,
        body: output.stdout,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(content_type: &str, body: &[u8]) -> Response {
        Response {
            url: String::new(),
            content_type: content_type.to_owned(),
            body: body.to_vec(),
        }
    }

    #[test]
    fn text_follows_the_declared_charset() {
        assert_eq!(
            response("text/html; charset=UTF-8", "é".as_bytes()).text(),
            "é"
        );
        assert_eq!(
            response("text/html; charset=ISO-8859-1", &[0xE9]).text(),
            "é"
        );
        assert_eq!(response("text/html", &[0xE9]).text(), "é");
        assert_eq!(response("text/html", "é".as_bytes()).text(), "é");
    }

    #[test]
    fn local_files_load_and_other_schemes_are_refused() -> Result<()> {
        let manifest = concat!("file://", env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
        assert!(fetch(manifest)?.text().contains("[package]"));
        assert!(matches!(
            fetch("ftp://example.com/x"),
            Err(Error::Network(_))
        ));
        assert!(matches!(
            fetch("file:///no/such/file"),
            Err(Error::Network(_))
        ));
        Ok(())
    }
}
