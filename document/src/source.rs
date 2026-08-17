//! How a document reaches a handler, and the conventions every handler shares.
//!
//! Two shapes, one of them required: a filesystem `path`, or `bytes_base64`
//! for a document that only exists in memory — an attachment in a chat
//! composer never touches the disk. Both land as one owned buffer, because the
//! converter wants a slice and every function here reads the whole file.
//!
//! Inline bytes carry a third field that a path does not need: `file_name`.
//! CSV has no signature of its own, so without a name a spreadsheet export is
//! unrecognisable, and the converter would refuse a file it can read perfectly
//! well.

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
/// the shell and pdf workers take.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct FsScope {
    /// The session's working directory.
    pub root: String,
    /// Additional directories or files explicitly granted to this session.
    #[serde(default)]
    pub grants: Vec<String>,
}

/// Where the document comes from. Exactly one of `path` and `bytes_base64`
/// must be set.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct DocumentSource {
    /// Filesystem path to the document. Mutually exclusive with
    /// `bytes_base64`.
    #[serde(default)]
    pub path: Option<String>,

    /// Base64-encoded document bytes, for a document with no path — an
    /// attachment held in memory. Mutually exclusive with `path`.
    #[serde(default)]
    pub bytes_base64: Option<String>,

    /// Original file name for inline bytes, used only to recognise a format
    /// the content cannot name. A `.csv` needs this; nothing else does.
    /// Ignored when `path` is set, which carries its own name.
    #[serde(default)]
    pub file_name: Option<String>,

    /// The filesystem jail this call runs under. Stamped by the harness on an
    /// agent's call; absent on an operator or console call, which is already
    /// user-initiated and not subject to the agent's scope.
    #[serde(default)]
    pub fs_scope: Option<FsScope>,
}

impl DocumentSource {
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
        match self.file_name_hint() {
            Some(name) => name,
            None => "<inline>".to_string(),
        }
    }

    /// The name format detection may fall back on: the path's file name, or
    /// the `file_name` supplied alongside inline bytes.
    pub fn file_name_hint(&self) -> Option<String> {
        if let Some(path) = &self.path {
            return Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .or_else(|| Some(path.clone()));
        }
        self.file_name.clone()
    }

    fn read_file(
        path: &str,
        scope: Option<&FsScope>,
        cfg: &WorkerConfig,
    ) -> Result<Vec<u8>, String> {
        use std::io::Read as _;

        // Resolve before checking. A path is only inside the jail once symlinks
        // and `..` are gone, and a check that follows a symlink checks the wrong
        // file.
        let resolved = std::fs::canonicalize(path).map_err(|e| format!("{path}: {e}"))?;
        if let Some(scope) = scope {
            authorize(&resolved, scope)?;
        }

        // Everything after this point works on ONE open handle. Checking the
        // path, then checking it again for size, then opening it a third time to
        // read leaves two windows: a file swapped for a symlink between the
        // authorization and the read discloses a file outside the scope, and one
        // that grows between the size check and the read walks past
        // `max_input_bytes`. The handle is the same file for all three.
        let file = std::fs::File::open(&resolved).map_err(|e| format!("{path}: {e}"))?;
        let meta = file.metadata().map_err(|e| format!("{path}: {e}"))?;
        if !meta.is_file() {
            return Err(format!("{path}: not a file"));
        }
        check_size(meta.len(), cfg)?;

        // Bounded regardless of what the metadata claimed: `take` is what makes
        // the ceiling hold for a file being appended to right now, and for the
        // special files whose reported length is a fiction.
        let ceiling = if cfg.max_input_bytes > 0 {
            cfg.max_input_bytes
        } else {
            u64::MAX
        };
        let mut bytes = Vec::with_capacity(meta.len().min(1 << 20) as usize);
        let read = file
            .take(ceiling.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|e| format!("{path}: {e}"))?;
        check_size(read as u64, cfg)?;
        Ok(bytes)
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
/// passing through anyone's context. Same shape the pdf worker returns, so a
/// caller handling both reads one field set.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Body {
    /// The markdown, shortened to the effective character cap.
    pub text: String,

    /// Characters returned in `text`.
    pub chars: usize,

    /// Characters the document actually holds. Equal to `chars` when nothing
    /// was dropped.
    pub total_chars: usize,

    /// `true` when `text` stops short of the document. Ask again with
    /// `max_chars: 0` to take everything.
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

/// Turn a converter error into something the caller can act on.
///
/// The converter's variants each imply a different next move, and its own
/// `Display` text does not say what that move is. An agent that reads
/// "unsupported input" with no advice tends to retry the same call.
pub fn describe_error(what: &str, err: &anydoc::ConvertError) -> String {
    let advice = match err {
        anydoc::ConvertError::Encrypted => {
            "the document is encrypted; nothing here can open it, so ask for an unlocked copy"
        }
        anydoc::ConvertError::Unsupported(_) => {
            "this format cannot be converted; for a scanned PDF call pdf::classify, which reports \
             which pages need OCR"
        }
        anydoc::ConvertError::Malformed { .. } => {
            "the file is structurally unusable; it is likely truncated or not the format it claims"
        }
        anydoc::ConvertError::ResourceLimit { .. } => {
            "the document crossed a fixed safety limit while parsing; it cannot be read here"
        }
        anydoc::ConvertError::MissingPart { .. } => {
            "a part the format requires is absent; the file is incomplete"
        }
        anydoc::ConvertError::Io(_) => "the file could not be read",
        // The converter marks its error enum non-exhaustive, so a new variant
        // must not stop this from compiling. Nothing useful to advise yet.
        _ => "the document could not be converted",
    };
    format!("{what} failed: {err} — {advice}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> WorkerConfig {
        WorkerConfig::default()
    }

    #[test]
    fn requires_exactly_one_input() {
        let err = DocumentSource::default().load(&cfg()).expect_err("neither");
        assert!(err.contains("provide a `path`"), "{err}");

        let both = DocumentSource {
            path: Some("a.docx".into()),
            bytes_base64: Some("AAAA".into()),
            ..DocumentSource::default()
        };
        let err = both.load(&cfg()).expect_err("both");
        assert!(err.contains("not both"), "{err}");
    }

    #[test]
    fn decodes_inline_bytes() {
        let src = DocumentSource {
            bytes_base64: Some(BASE64.encode(b"a,b\n1,2\n")),
            ..DocumentSource::default()
        };
        assert_eq!(src.load(&cfg()).expect("decodes"), b"a,b\n1,2\n");
    }

    #[test]
    fn rejects_malformed_base64() {
        let src = DocumentSource {
            bytes_base64: Some("not base64!!!".into()),
            ..DocumentSource::default()
        };
        let err = src.load(&cfg()).expect_err("malformed");
        assert!(err.contains("not valid base64"), "{err}");
    }

    /// The ceiling has to hold against the file itself, not only against what
    /// its metadata claimed a moment earlier.
    #[test]
    fn a_file_over_the_ceiling_is_refused_on_the_bytes_read() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("big.csv");
        std::fs::write(&path, vec![b'a'; 4096]).expect("write");

        let cfg = WorkerConfig {
            max_input_bytes: 128,
            ..WorkerConfig::default()
        };
        let source = DocumentSource {
            path: Some(path.to_string_lossy().to_string()),
            ..DocumentSource::default()
        };
        let err = source.load(&cfg).expect_err("over the ceiling");
        assert!(err.contains("max_input_bytes"), "{err}");
    }

    #[test]
    fn a_file_inside_the_ceiling_reads_whole() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("rows.csv");
        std::fs::write(&path, b"a,b\n1,2\n").expect("write");

        let source = DocumentSource {
            path: Some(path.to_string_lossy().to_string()),
            ..DocumentSource::default()
        };
        assert_eq!(source.load(&cfg()).expect("reads"), b"a,b\n1,2\n");
    }

    #[test]
    fn enforces_the_size_ceiling_before_decoding() {
        let cfg = WorkerConfig {
            max_input_bytes: 4,
            ..WorkerConfig::default()
        };
        let src = DocumentSource {
            bytes_base64: Some(BASE64.encode(vec![0u8; 1024])),
            ..DocumentSource::default()
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
        let inside = dir.path().join("report.docx");
        std::fs::write(&inside, b"PK\x03\x04").expect("write");

        let outside = tempfile::tempdir().expect("second temp dir");
        let secret = outside.path().join("payroll.xlsx");
        std::fs::write(&secret, b"PK\x03\x04").expect("write");

        let scope = FsScope {
            root: dir.path().to_string_lossy().to_string(),
            grants: vec![],
        };

        let allowed = DocumentSource {
            path: Some(inside.to_string_lossy().to_string()),
            fs_scope: Some(scope.clone()),
            ..DocumentSource::default()
        };
        assert!(allowed.load(&cfg()).is_ok(), "a path inside the root reads");

        let refused = DocumentSource {
            path: Some(secret.to_string_lossy().to_string()),
            fs_scope: Some(scope),
            ..DocumentSource::default()
        };
        let err = refused.load(&cfg()).expect_err("outside the scope");
        assert!(
            err.contains("outside this session's filesystem scope"),
            "{err}"
        );
    }

    /// A sibling whose name merely starts with the root is not inside it. A
    /// prefix comparison on strings would let `/w/project-old` pass for a root
    /// of `/w/project`.
    #[test]
    fn a_sibling_directory_with_a_shared_prefix_is_not_inside_the_scope() {
        let parent = tempfile::tempdir().expect("temp dir");
        let root = parent.path().join("project");
        let sibling = parent.path().join("project-old");
        std::fs::create_dir_all(&root).expect("root");
        std::fs::create_dir_all(&sibling).expect("sibling");
        let doc = sibling.join("secret.docx");
        std::fs::write(&doc, b"PK\x03\x04").expect("write");

        let source = DocumentSource {
            path: Some(doc.to_string_lossy().to_string()),
            fs_scope: Some(FsScope {
                root: root.to_string_lossy().to_string(),
                grants: vec![],
            }),
            ..DocumentSource::default()
        };
        let err = source.load(&cfg()).expect_err("sibling is outside");
        assert!(err.contains("outside"), "{err}");
    }

    #[test]
    fn label_never_leaks_the_directory() {
        let src = DocumentSource {
            path: Some("/home/someone/private/report.docx".into()),
            ..DocumentSource::default()
        };
        assert_eq!(src.label(), "report.docx");
        assert_eq!(DocumentSource::default().label(), "<inline>");
    }

    /// Inline bytes are the composer's path, and the name is the only way a
    /// CSV is ever recognised.
    #[test]
    fn inline_bytes_keep_their_name_for_detection() {
        let src = DocumentSource {
            bytes_base64: Some(BASE64.encode(b"a,b\n")),
            file_name: Some("rows.csv".into()),
            ..DocumentSource::default()
        };
        assert_eq!(src.file_name_hint().as_deref(), Some("rows.csv"));
        assert_eq!(src.label(), "rows.csv");
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

    /// Each failure implies a different next move, and the converter's own
    /// message never says what it is.
    #[test]
    fn errors_say_what_to_do_next() {
        let encrypted = describe_error("conversion", &anydoc::ConvertError::Encrypted);
        assert!(encrypted.contains("unlocked copy"), "{encrypted}");

        let unsupported =
            describe_error("conversion", &anydoc::ConvertError::Unsupported("x".into()));
        assert!(unsupported.contains("pdf::classify"), "{unsupported}");

        let malformed = describe_error(
            "conversion",
            &anydoc::ConvertError::Malformed {
                part: Some("word/document.xml".into()),
                detail: "unexpected end".into(),
            },
        );
        assert!(malformed.contains("truncated"), "{malformed}");
    }

    #[test]
    fn a_new_converter_variant_still_gets_advice() {
        // The converter's error enum is `#[non_exhaustive]`, so the catch-all
        // arm is what a future variant lands on. It still has to name the
        // document and read as an answer.
        let described = describe_error(
            "conversion",
            &anydoc::ConvertError::Io(std::io::Error::other("disk went away")),
        );
        assert!(described.contains("conversion failed"), "{described}");
        assert!(described.contains("could not be read"), "{described}");
    }
}
