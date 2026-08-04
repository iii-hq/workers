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
use std::path::Path;

use crate::config::WorkerConfig;

/// The filesystem jail a call runs under.
///
/// The harness stamps this onto every function it dispatches, so a `path` an
/// agent supplies has to be checked against it. Without the check these
/// functions would read any document on the machine and hand back its text,
/// which is a way around the scope the session was granted. Mirrors the shape
/// the shell worker takes.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct FsScope {
    /// The session's working directory.
    pub root: String,
    /// Additional directories or files explicitly granted to this session.
    #[serde(default)]
    pub grants: Vec<String>,
}

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

    /// The filesystem jail this call runs under. Stamped by the harness on an
    /// agent's call; absent on an operator or console call, which is already
    /// user-initiated and not subject to the agent's scope.
    #[serde(default)]
    pub fs_scope: Option<FsScope>,
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
            (Some(path), None) => Self::read_file(path, self.fs_scope.as_ref(), cfg),
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

    fn read_file(
        path: &str,
        scope: Option<&FsScope>,
        cfg: &WorkerConfig,
    ) -> Result<Vec<u8>, String> {
        // Resolve before checking. A path is only inside the jail once symlinks
        // and `..` are gone, and `metadata` would follow a symlink out of it.
        let resolved = std::fs::canonicalize(path).map_err(|e| format!("{path}: {e}"))?;
        if let Some(scope) = scope {
            authorize(&resolved, scope)?;
        }
        let meta = std::fs::metadata(&resolved).map_err(|e| format!("{path}: {e}"))?;
        if !meta.is_file() {
            return Err(format!("{path}: not a file"));
        }
        check_size(meta.len(), cfg)?;
        std::fs::read(&resolved).map_err(|e| format!("{path}: {e}"))
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

/// Reject a resolved path that sits outside the session's jail.
///
/// The comparison is on canonical paths and whole path components, so a
/// sibling directory whose name merely starts with the root (`/w/project-old`
/// against a root of `/w/project`) is not treated as inside it.
fn authorize(resolved: &Path, scope: &FsScope) -> Result<(), String> {
    let allowed = std::iter::once(&scope.root).chain(scope.grants.iter());
    for entry in allowed {
        // A grant that does not resolve is a stale grant, not a reason to fail
        // the call: skip it and let the remaining ones decide.
        let Ok(base) = std::fs::canonicalize(entry) else {
            continue;
        };
        if resolved == base || resolved.starts_with(&base) {
            return Ok(());
        }
    }
    Err(format!(
        "{} is outside this session's filesystem scope",
        resolved.display()
    ))
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

    /// `true` when `text` stops short of the document. Ask again with
    /// `max_chars: 0` to take everything, or on the functions that accept one,
    /// narrow with a `pages` filter.
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
            fs_scope: None,
        };
        let err = both.load(&cfg()).expect_err("both");
        assert!(err.contains("not both"), "{err}");
    }

    #[test]
    fn decodes_inline_bytes() {
        let src = PdfSource {
            path: None,
            bytes_base64: Some(BASE64.encode(b"%PDF-1.4")),
            fs_scope: None,
        };
        assert_eq!(src.load(&cfg()).expect("decodes"), b"%PDF-1.4");
    }

    #[test]
    fn rejects_malformed_base64() {
        let src = PdfSource {
            path: None,
            bytes_base64: Some("not base64!!!".into()),
            fs_scope: None,
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
            fs_scope: None,
        };
        let err = src.load(&cfg).expect_err("over the ceiling");
        assert!(err.contains("max_input_bytes"), "{err}");
    }

    /// The harness stamps a scope on every call it dispatches. Without this
    /// check an agent could read any document on the machine and get its text
    /// back, which is a way around the scope its session was granted.
    #[test]
    fn a_path_outside_the_session_scope_is_refused() {
        let dir = tempfile::tempdir().expect("temp dir");
        let inside = dir.path().join("report.pdf");
        std::fs::write(&inside, b"%PDF-1.4").expect("write");

        let outside = tempfile::tempdir().expect("second temp dir");
        let secret = outside.path().join("payroll.pdf");
        std::fs::write(&secret, b"%PDF-1.4").expect("write");

        let scope = FsScope {
            root: dir.path().to_string_lossy().to_string(),
            grants: vec![],
        };

        let allowed = PdfSource {
            path: Some(inside.to_string_lossy().to_string()),
            bytes_base64: None,
            fs_scope: Some(scope.clone()),
        };
        assert!(allowed.load(&cfg()).is_ok(), "a path inside the root reads");

        let refused = PdfSource {
            path: Some(secret.to_string_lossy().to_string()),
            bytes_base64: None,
            fs_scope: Some(scope),
        };
        let err = refused.load(&cfg()).expect_err("outside the scope");
        assert!(
            err.contains("outside this session's filesystem scope"),
            "{err}"
        );
    }

    #[test]
    fn an_explicit_grant_widens_the_scope() {
        let root = tempfile::tempdir().expect("temp dir");
        let granted = tempfile::tempdir().expect("granted dir");
        let doc = granted.path().join("statement.pdf");
        std::fs::write(&doc, b"%PDF-1.4").expect("write");

        let source = PdfSource {
            path: Some(doc.to_string_lossy().to_string()),
            bytes_base64: None,
            fs_scope: Some(FsScope {
                root: root.path().to_string_lossy().to_string(),
                grants: vec![granted.path().to_string_lossy().to_string()],
            }),
        };
        assert!(source.load(&cfg()).is_ok(), "an explicit grant is honoured");
    }

    /// A sibling whose name merely starts with the root is not inside it.
    /// A prefix comparison on strings would let `/w/project-old` pass for a
    /// root of `/w/project`.
    #[test]
    fn a_sibling_directory_with_a_shared_prefix_is_not_inside_the_scope() {
        let parent = tempfile::tempdir().expect("temp dir");
        let root = parent.path().join("project");
        let sibling = parent.path().join("project-old");
        std::fs::create_dir_all(&root).expect("root");
        std::fs::create_dir_all(&sibling).expect("sibling");
        let doc = sibling.join("secret.pdf");
        std::fs::write(&doc, b"%PDF-1.4").expect("write");

        let source = PdfSource {
            path: Some(doc.to_string_lossy().to_string()),
            bytes_base64: None,
            fs_scope: Some(FsScope {
                root: root.to_string_lossy().to_string(),
                grants: vec![],
            }),
        };
        let err = source.load(&cfg()).expect_err("sibling is outside");
        assert!(err.contains("outside"), "{err}");
    }

    /// Inline bytes carry no path, so there is nothing to escape and the scope
    /// does not apply.
    #[test]
    fn inline_bytes_are_unaffected_by_a_scope() {
        let source = PdfSource {
            path: None,
            bytes_base64: Some(BASE64.encode(b"%PDF-1.4")),
            fs_scope: Some(FsScope {
                root: "/nowhere".to_string(),
                grants: vec![],
            }),
        };
        assert!(source.load(&cfg()).is_ok());
    }

    #[test]
    fn label_never_leaks_the_directory() {
        let src = PdfSource {
            path: Some("/home/someone/private/report.pdf".into()),
            bytes_base64: None,
            fs_scope: None,
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

    /// The password must not survive into an error string. The parser is what
    /// produces this text, so the test feeds in an error that DOES carry the
    /// password and asserts the rewrite drops it. The previous version passed a
    /// message with no password in it, so it proved nothing.
    #[test]
    fn encryption_errors_never_echo_the_password() {
        let leaky = "PDF is encrypted: bad password 'hunter2-secret'";
        let message = describe_error("classify", leaky, true, true);
        assert!(
            !message.contains("hunter2-secret"),
            "the password survived into the error: {message}"
        );
        assert!(
            message.contains("did not open it"),
            "the caller still needs to know the password was wrong: {message}"
        );
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
