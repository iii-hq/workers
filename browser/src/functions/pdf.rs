//! `browser::pdf` — print the session's page to a PDF (what the browser's
//! Print → Save as PDF does), returned as bytes for the caller to save or
//! attach.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Upper bound on a returned PDF: the bytes travel base64 over the bus and
/// into the caller's context, so an unbounded print of a huge page is a
/// refusal, not a surprise.
pub const MAX_PDF_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PdfInput {
    pub session_id: String,
    /// Landscape orientation. Default portrait.
    #[serde(default)]
    pub landscape: Option<bool>,
    /// Print background colours and images. Default true (what the page
    /// looks like, not what a printer would save ink on).
    #[serde(default)]
    pub print_background: Option<bool>,
    /// Page scale, 0.1–2. Default 1.
    #[serde(default)]
    pub scale: Option<f64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PdfOutput {
    pub ok: bool,
    /// The PDF, base64.
    pub data: String,
    pub size_bytes: u64,
    /// Suggested file name, from the page title.
    pub file_name: String,
    pub url: String,
}

/// `<title>.pdf`, safe for a file system; falls back to the host.
pub fn file_name(title: &str, url: &str) -> String {
    let base = if title.trim().is_empty() {
        url.trim_start_matches("https://")
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap_or("page")
            .to_string()
    } else {
        title.to_string()
    };
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' || c == '.' {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let cleaned = cleaned.chars().take(80).collect::<String>();
    format!(
        "{}.pdf",
        if cleaned.is_empty() {
            "page".to_string()
        } else {
            cleaned
        }
    )
}

#[cfg(test)]
mod tests {
    use super::file_name;

    #[test]
    fn names_after_the_title_or_the_host() {
        assert_eq!(
            file_name("Example Domain", "https://example.com/"),
            "Example Domain.pdf"
        );
        assert_eq!(
            file_name("  ", "https://example.com/a/b"),
            "example.com.pdf"
        );
        assert_eq!(file_name("a/b:c*d", "x"), "a b c d.pdf");
        assert_eq!(file_name("v1.2 notes", "x"), "v1.2 notes.pdf");
    }
}
