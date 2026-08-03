//! How a PDF reaches a handler, and the conventions every handler shares.
//!
//! Two shapes, one of them required: a filesystem `path`, or `bytes_base64`
//! for a document that only exists in memory. Both land as one owned buffer,
//! because the parser wants a slice and every function in this worker reads the
//! whole file anyway.
//!
//! Page numbers are the other shared convention. The parser is internally
//! inconsistent about them: some results count pages from one, some from zero,
//! and its own page filters disagree with each other. Every number crossing
//! this worker's wire is 1-indexed, and the conversions live here so a caller
//! never has to know which side of the boundary a number came from.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::WorkerConfig;

/// Where the PDF comes from. Exactly one of the two fields must be set.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct PdfSource {
    /// Filesystem path to the PDF. Mutually exclusive with `bytes_base64`.
    #[serde(default)]
    pub path: Option<String>,

    /// Base64-encoded PDF bytes, for a document with no path. Mutually
    /// exclusive with `path`.
    #[serde(default)]
    pub bytes_base64: Option<String>,
}

impl PdfSource {
    /// Read the document into memory, enforcing the configured size ceiling
    /// before anything is parsed.
    pub fn load(&self, cfg: &WorkerConfig) -> Result<Vec<u8>, String> {
        match (&self.path, &self.bytes_base64) {
            (Some(_), Some(_)) => {
                Err("provide either `path` or `bytes_base64`, not both".to_string())
            }
            (None, None) => Err("provide a `path` or `bytes_base64`".to_string()),
            (Some(path), None) => Self::read_file(path, cfg),
            (None, Some(encoded)) => Self::decode(encoded, cfg),
        }
    }

    /// A short label for logs and responses: the file name, or a note that the
    /// document arrived inline. Never the full path, which may be sensitive.
    pub fn label(&self) -> String {
        match (&self.path, &self.bytes_base64) {
            (Some(path), _) => std::path::Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.clone()),
            _ => "<inline>".to_string(),
        }
    }

    fn read_file(path: &str, cfg: &WorkerConfig) -> Result<Vec<u8>, String> {
        let meta = std::fs::metadata(path).map_err(|e| format!("{path}: {e}"))?;
        if !meta.is_file() {
            return Err(format!("{path}: not a file"));
        }
        check_size(meta.len(), cfg)?;
        std::fs::read(path).map_err(|e| format!("{path}: {e}"))
    }

    fn decode(encoded: &str, cfg: &WorkerConfig) -> Result<Vec<u8>, String> {
        // Reject on the encoded length first: decoding a huge blob to find out
        // it is too large defeats the ceiling.
        check_size((encoded.len() as u64 / 4) * 3, cfg)?;
        let bytes = BASE64
            .decode(encoded.as_bytes())
            .map_err(|e| format!("bytes_base64 is not valid base64: {e}"))?;
        check_size(bytes.len() as u64, cfg)?;
        Ok(bytes)
    }
}

fn check_size(bytes: u64, cfg: &WorkerConfig) -> Result<(), String> {
    if cfg.max_input_bytes > 0 && bytes > cfg.max_input_bytes {
        return Err(format!(
            "document is {bytes} bytes, over the configured max_input_bytes of {}",
            cfg.max_input_bytes
        ));
    }
    Ok(())
}

/// A body that may have been shortened to fit one response, and the numbers a
/// caller needs to decide what to do about it.
///
/// The cap is what keeps a long document from flooding a model's context. A
/// caller that genuinely wants the whole thing asks for `max_chars: 0`, which
/// is the shape a worker-to-worker pipeline uses to move a document without it
/// passing through anyone's context.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Body {
    /// The content, shortened to the effective character cap.
    pub text: String,

    /// Characters returned in `text`.
    pub chars: usize,

    /// Characters the document actually holds. Equal to `chars` when nothing
    /// was dropped.
    pub total_chars: usize,

    /// `true` when `text` stops short of the document. Ask again with a page
    /// filter, or with `max_chars: 0` to take everything.
    pub truncated: bool,

    /// Leading characters of the content. Present only when the body was
    /// truncated, so a caller can see the shape of what it did not get without
    /// re-reading the start of `text`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

impl Body {
    /// Build a response body, applying `max_chars` (`0` means uncapped) on a
    /// character boundary.
    pub fn new(full: String, max_chars: usize, preview_chars: usize) -> Self {
        let total_chars = full.chars().count();
        if max_chars == 0 || total_chars <= max_chars {
            return Self {
                chars: total_chars,
                total_chars,
                text: full,
                truncated: false,
                preview: None,
            };
        }
        let text: String = full.chars().take(max_chars).collect();
        let preview: String = full.chars().take(preview_chars).collect();
        Self {
            chars: text.chars().count(),
            total_chars,
            text,
            truncated: true,
            preview: Some(preview),
        }
    }
}

/// Turn a parser error into something the caller can act on.
///
/// "PDF is encrypted" is the parser's answer to three different situations, and
/// the caller's next move differs in each: supply a password, supply a
/// different password, or stop asking this function. `password_supported` says
/// whether the calling function has a password parameter at all, because three
/// of the five do not and telling someone to pass one there wastes their turn.
pub fn describe_error(
    what: &str,
    err: impl std::fmt::Display,
    supplied: bool,
    password_supported: bool,
) -> String {
    let text = err.to_string();
    if !text.to_lowercase().contains("encrypt") {
        return format!("{what} failed: {text}");
    }
    let advice = match (supplied, password_supported) {
        (true, _) => "the supplied password did not open it",
        (false, true) => "pass the document's password",
        (false, false) => {
            "this function cannot decrypt; use pdf::classify or pdf::to-markdown, which take a password"
        }
    };
    format!("{what} failed: the document is encrypted and {advice}")
}

/// Convert a 0-indexed page number from the parser to the 1-indexed number this
/// worker puts on the wire.
pub fn to_wire_page(zero_indexed: u32) -> u32 {
    zero_indexed + 1
}

/// Convert a 1-indexed page number from the wire to the 0-indexed number the
/// parser's per-page extraction expects.
pub fn to_parser_page(one_indexed: u32) -> Result<u32, String> {
    one_indexed
        .checked_sub(1)
        .ok_or_else(|| "page numbers are 1-indexed; 0 is not a page".to_string())
}

/// Convert a whole 1-indexed page list for the parser, rejecting `0` rather
/// than silently wrapping it to the last page.
pub fn to_parser_pages(pages: &[u32]) -> Result<Vec<u32>, String> {
    pages.iter().copied().map(to_parser_page).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> WorkerConfig {
        WorkerConfig::default()
    }

    #[test]
    fn requires_exactly_one_input() {
        let err = PdfSource::default().load(&cfg()).expect_err("neither");
        assert!(err.contains("provide a `path`"), "{err}");

        let both = PdfSource {
            path: Some("a.pdf".into()),
            bytes_base64: Some("AAAA".into()),
        };
        let err = both.load(&cfg()).expect_err("both");
        assert!(err.contains("not both"), "{err}");
    }

    #[test]
    fn decodes_inline_bytes() {
        let src = PdfSource {
            path: None,
            bytes_base64: Some(BASE64.encode(b"%PDF-1.4")),
        };
        assert_eq!(src.load(&cfg()).expect("decodes"), b"%PDF-1.4");
    }

    #[test]
    fn rejects_malformed_base64() {
        let src = PdfSource {
            path: None,
            bytes_base64: Some("not base64!!!".into()),
        };
        let err = src.load(&cfg()).expect_err("malformed");
        assert!(err.contains("not valid base64"), "{err}");
    }

    #[test]
    fn enforces_the_size_ceiling_before_decoding() {
        let cfg = WorkerConfig {
            max_input_bytes: 4,
            ..WorkerConfig::default()
        };
        let src = PdfSource {
            path: None,
            bytes_base64: Some(BASE64.encode(vec![0u8; 1024])),
        };
        let err = src.load(&cfg).expect_err("over the ceiling");
        assert!(err.contains("max_input_bytes"), "{err}");
    }

    #[test]
    fn label_never_leaks_the_directory() {
        let src = PdfSource {
            path: Some("/home/someone/private/report.pdf".into()),
            bytes_base64: None,
        };
        assert_eq!(src.label(), "report.pdf");
        assert_eq!(PdfSource::default().label(), "<inline>");
    }

    #[test]
    fn body_reports_what_it_dropped() {
        let body = Body::new("abcdefghij".to_string(), 4, 2);
        assert_eq!(body.text, "abcd");
        assert_eq!(body.chars, 4);
        assert_eq!(body.total_chars, 10);
        assert!(body.truncated);
        assert_eq!(body.preview.as_deref(), Some("ab"));
    }

    #[test]
    fn body_uncapped_when_max_chars_is_zero() {
        let body = Body::new("abcdefghij".to_string(), 0, 2);
        assert_eq!(body.text, "abcdefghij");
        assert!(!body.truncated);
        assert!(body.preview.is_none());
    }

    /// Truncation must not split a multi-byte character.
    #[test]
    fn body_truncates_on_character_boundaries() {
        let body = Body::new("日本語のテキスト".to_string(), 3, 2);
        assert_eq!(body.text, "日本語");
        assert_eq!(body.total_chars, 8);
    }

    /// Three situations behind one parser message, three different next moves.
    #[test]
    fn encryption_errors_say_what_to_do_next() {
        let no_password = describe_error("classify", "PDF is encrypted", false, true);
        assert!(
            no_password.contains("pass the document's password"),
            "{no_password}"
        );

        let wrong_password = describe_error("classify", "PDF is encrypted", true, true);
        assert!(
            wrong_password.contains("did not open it"),
            "{wrong_password}"
        );

        let unsupported = describe_error("extract", "PDF is encrypted", false, false);
        assert!(unsupported.contains("pdf::to-markdown"), "{unsupported}");
    }

    /// The password itself must never reach an error string.
    #[test]
    fn encryption_errors_never_echo_the_password() {
        let message = describe_error("classify", "PDF is encrypted", true, true);
        assert!(!message.contains("secret"), "{message}");
    }

    #[test]
    fn other_errors_pass_through_unchanged() {
        let message = describe_error("extract", "not a PDF file", false, true);
        assert_eq!(message, "extract failed: not a PDF file");
    }

    #[test]
    fn page_conversions_round_trip() {
        assert_eq!(to_wire_page(0), 1);
        assert_eq!(to_parser_page(1), Ok(0));
        assert_eq!(to_parser_pages(&[1, 3, 5]), Ok(vec![0, 2, 4]));
    }

    /// Page 0 is a caller mistake, and `0 - 1` on a u32 would wrap to the last
    /// page of a very different document.
    #[test]
    fn page_zero_is_rejected() {
        assert!(to_parser_page(0).is_err());
        assert!(to_parser_pages(&[1, 0]).is_err());
    }
}
