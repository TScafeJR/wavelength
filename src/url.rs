//! The small amount of URL handling a browser cannot do without: resolving
//! relative references, tidying what was typed into the address bar and
//! encoding form data.

/// Whether the text starts with a URL scheme such as `https:`.
fn has_scheme(text: &str) -> bool {
    let Some((scheme, _)) = text.split_once(':') else {
        return false;
    };
    scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Resolves a reference found in a document against that document's URL.
///
/// Returns `None` when the base is not a hierarchical URL of the form `scheme://…`.
pub fn resolve(base: &str, reference: &str) -> Option<String> {
    let reference = reference.trim();
    if has_scheme(reference) {
        return Some(reference.to_owned());
    }
    let (scheme, rest) = base.split_once("://")?;
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, path_and_more) = rest.split_at(authority_end);
    let without_fragment = path_and_more.split('#').next().unwrap_or_default();
    let base_path = without_fragment.split('?').next().unwrap_or_default();
    let origin = format!("{scheme}://{authority}");

    Some(if reference.is_empty() {
        format!("{origin}{without_fragment}")
    } else if let Some(rest) = reference.strip_prefix("//") {
        format!("{scheme}://{rest}")
    } else if reference.starts_with('/') {
        format!("{origin}{}", remove_dot_segments(reference))
    } else if reference.starts_with('#') {
        format!("{origin}{without_fragment}{reference}")
    } else if reference.starts_with('?') {
        format!(
            "{origin}{}{reference}",
            if base_path.is_empty() { "/" } else { base_path }
        )
    } else {
        let directory = base_path
            .rfind('/')
            .map_or("/", |slash| &base_path[..=slash]);
        format!(
            "{origin}{}",
            remove_dot_segments(&format!("{directory}{reference}"))
        )
    })
}

/// Collapses `.` and `..` in the path part of a reference, leaving any query or fragment alone.
fn remove_dot_segments(path_and_more: &str) -> String {
    let split = path_and_more
        .find(['?', '#'])
        .unwrap_or(path_and_more.len());
    let (path, tail) = path_and_more.split_at(split);
    let mut segments: Vec<&str> = Vec::new();
    let mut ends_in_directory = false;
    for segment in path.split('/') {
        ends_in_directory = matches!(segment, "." | "..");
        match segment {
            "." => {}
            ".." => {
                // The first segment is the empty one before the leading slash, which must stay.
                if segments.len() > 1 {
                    segments.pop();
                }
            }
            other => segments.push(other),
        }
    }
    if ends_in_directory {
        segments.push("");
    }
    format!("{}{tail}", segments.join("/"))
}

/// Turns what was typed into the address bar into a URL.
///
/// Existing files become `file://` URLs and anything without a scheme is assumed to be a web address.
pub fn from_input(text: &str) -> String {
    let text = text.trim();
    if has_scheme(text) && text.contains("://") {
        return text.to_owned();
    }
    if let Ok(path) = std::fs::canonicalize(text) {
        return format!(
            "file://{}",
            percent_encode(&path.to_string_lossy(), b"/-._~")
        );
    }
    format!("https://{text}")
}

/// Encodes form fields as `application/x-www-form-urlencoded`.
pub fn encode_form(fields: &[(String, String)]) -> String {
    let encode = |text: &str| percent_encode(text, b"-._*").replace("%20", "+");
    fields
        .iter()
        .map(|(name, value)| format!("{}={}", encode(name), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn percent_encode(text: &str, keep: &[u8]) -> String {
    let mut out = String::with_capacity(text.len());
    for &byte in text.as_bytes() {
        if byte.is_ascii_alphanumeric() || keep.contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Decodes `%XX` escapes. Invalid escapes are left as written.
pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = (bytes[index] == b'%')
            .then(|| text.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                out.push(byte);
                index += 3;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_resolve_against_the_base() {
        let base = "https://example.com/a/b/page.html?x=1#top";
        let resolved = |reference| resolve(base, reference);
        assert_eq!(
            resolved("https://other.org/z").as_deref(),
            Some("https://other.org/z")
        );
        assert_eq!(
            resolved("//cdn.example.com/i.png").as_deref(),
            Some("https://cdn.example.com/i.png")
        );
        assert_eq!(
            resolved("/root.css").as_deref(),
            Some("https://example.com/root.css")
        );
        assert_eq!(
            resolved("next.html").as_deref(),
            Some("https://example.com/a/b/next.html")
        );
        assert_eq!(
            resolved("../up/x.png?v=2").as_deref(),
            Some("https://example.com/a/up/x.png?v=2")
        );
        assert_eq!(
            resolved("../../../../x").as_deref(),
            Some("https://example.com/x")
        );
        assert_eq!(resolved("./").as_deref(), Some("https://example.com/a/b/"));
        assert_eq!(
            resolved("?y=2").as_deref(),
            Some("https://example.com/a/b/page.html?y=2")
        );
        assert_eq!(
            resolved("#end").as_deref(),
            Some("https://example.com/a/b/page.html?x=1#end")
        );
        assert_eq!(
            resolved("").as_deref(),
            Some("https://example.com/a/b/page.html?x=1")
        );
        assert_eq!(
            resolved("mailto:someone@example.com").as_deref(),
            Some("mailto:someone@example.com")
        );
    }

    #[test]
    fn a_bare_host_base_still_resolves() {
        assert_eq!(
            resolve("https://example.com", "a.png").as_deref(),
            Some("https://example.com/a.png")
        );
        assert_eq!(
            resolve("https://example.com", "?q=1").as_deref(),
            Some("https://example.com/?q=1")
        );
        assert_eq!(resolve("not a url", "a.png"), None);
    }

    #[test]
    fn address_bar_text_becomes_a_url() {
        assert_eq!(from_input(" google.com "), "https://google.com");
        assert_eq!(
            from_input("http://localhost:8000/x"),
            "http://localhost:8000/x"
        );
        assert!(from_input("Cargo.toml").starts_with("file:///"));
        assert!(from_input("Cargo.toml").ends_with("/Cargo.toml"));
    }

    #[test]
    fn form_fields_are_encoded() {
        let fields = [
            ("q".to_owned(), "rust & c++ é".to_owned()),
            ("hl".to_owned(), "en".to_owned()),
        ];
        assert_eq!(encode_form(&fields), "q=rust+%26+c%2B%2B+%C3%A9&hl=en");
    }

    #[test]
    fn percent_escapes_decode() {
        assert_eq!(percent_decode("/a%20b/%C3%A9%zz%2"), "/a b/é%zz%2");
    }
}
