//! `document::detect` — what is this file, before anything tries to read it.
//!
//! Cheap on purpose: the signature lives in the first bytes of the file, so
//! this answers in microseconds where a conversion takes milliseconds. It is
//! what a caller holding a mixed bag of attachments runs first, to decide
//! whether a file is a document at all and which worker should read it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::WorkerConfig;
use crate::format::{self, DetectedFrom, Family, Format};
use crate::source::DocumentSource;

pub const ID: &str = "document::detect";
pub const DESC: &str = "Identify a document's format from its bytes (falling back to the file \
                        name for CSV, which carries no signature), and report which family it \
                        belongs to and whether this worker can convert it. Microseconds, and no \
                        conversion.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    #[serde(flatten)]
    pub source: DocumentSource,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// The format, or `null` when nothing recognised it. A null means the file
    /// is not one of the formats this worker reads — an image, an archive, a
    /// plain text file — not that it is broken.
    pub format: Option<Format>,

    /// What the document is: prose, a spreadsheet, a presentation, a book, a
    /// PDF. Absent when the format is unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<Family>,

    /// How the format was arrived at. `extension` is the weaker claim: the
    /// content matched nothing known, and only the file name suggested this.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detected_from: Option<DetectedFrom>,

    /// `true` when `document::to-markdown` can convert this file.
    pub convertible: bool,

    /// `true` when the format carries embedded assets `document::extract-assets`
    /// can pull out. A PDF converts straight to markdown and has none here.
    pub has_assets: bool,

    /// Size of the document in bytes.
    pub size_bytes: u64,

    /// Source label: the file name, or `<inline>` for an in-memory document
    /// that arrived without one.
    pub source: String,

    /// Wall-clock time for the detection.
    pub elapsed_ms: u64,
}

pub fn handle(req: Request, cfg: &WorkerConfig) -> Result<Response, String> {
    let bytes = req.source.load(cfg)?;
    let started = std::time::Instant::now();

    let resolved = format::resolve(None, &bytes, req.source.file_name_hint().as_deref());

    Ok(Response {
        format: resolved.map(|(format, _)| format),
        family: resolved.map(|(format, _)| format.family()),
        detected_from: resolved.map(|(_, how)| how),
        convertible: resolved.is_some(),
        has_assets: resolved.is_some_and(|(format, _)| format.has_document_model()),
        size_bytes: bytes.len() as u64,
        source: req.source.label(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine as _;

    use super::*;

    fn detect(bytes: &[u8], file_name: Option<&str>) -> Response {
        let req = Request {
            source: DocumentSource {
                bytes_base64: Some(BASE64.encode(bytes)),
                file_name: file_name.map(str::to_string),
                ..DocumentSource::default()
            },
        };
        handle(req, &WorkerConfig::default()).expect("detection never fails on readable bytes")
    }

    #[test]
    fn a_pdf_is_recognised_from_its_header() {
        let response = detect(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n", Some("report.pdf"));
        assert_eq!(response.format, Some(Format::Pdf));
        assert_eq!(response.family, Some(Family::Pdf));
        assert_eq!(response.detected_from, Some(DetectedFrom::Content));
        assert!(response.convertible);
        // A PDF has no document model, so it has no assets to extract here.
        assert!(!response.has_assets);
    }

    #[test]
    fn a_csv_is_recognised_from_its_name() {
        let response = detect(b"name,total\nrohit,3\n", Some("rows.csv"));
        assert_eq!(response.format, Some(Format::Csv));
        assert_eq!(response.family, Some(Family::Spreadsheet));
        assert_eq!(response.detected_from, Some(DetectedFrom::Extension));
        assert!(response.convertible);
    }

    /// An unrecognised file is an answer, not a failure: the caller asked what
    /// this is, and "not a document I read" is the answer.
    #[test]
    fn an_unknown_file_reports_itself_as_unconvertible() {
        let response = detect(b"\x89PNG\r\n\x1a\n", Some("shot.png"));
        assert_eq!(response.format, None);
        assert!(response.family.is_none());
        assert!(!response.convertible);
        assert!(!response.has_assets);
        assert_eq!(response.size_bytes, 8);
    }

    #[test]
    fn a_missing_source_is_refused() {
        let req = Request {
            source: DocumentSource::default(),
        };
        let err = handle(req, &WorkerConfig::default()).expect_err("no source");
        assert!(err.contains("provide a `path`"), "{err}");
    }
}
