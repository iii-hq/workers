//! `document::to-markdown` — any office document as markdown that keeps its
//! shape.
//!
//! One serializer sits behind every format, so a `.doc` from 2003 and a `.pptx`
//! from yesterday come out with the same heading, table and list conventions.
//! That sameness is the point: a caller reading a mixed bag of attachments
//! writes one parser, not fourteen.
//!
//! The size cap is the other half of the job. A long report runs to hundreds of
//! thousands of characters, and handing that to a model wastes the context it
//! needed for the answer. Responses are capped by default and say so; a caller
//! that genuinely wants the whole document passes `max_chars: 0`, which is what
//! a worker-to-worker pipeline does when the document is going to storage
//! rather than to a model.
//!
//! PDFs convert here too, because the converter reads text-based ones already.
//! When the `pdf` worker is installed it is the better route for them: it
//! classifies scanned versus text-based, and reports which pages need OCR
//! rather than returning an empty document.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::WorkerConfig;
use crate::format::{self, DetectedFrom, Family, Format};
use crate::source::{describe_error, Body, DocumentSource};

pub const ID: &str = "document::to-markdown";
pub const DESC: &str = "Convert a Word, PowerPoint, Excel, OpenDocument, RTF, EPUB, CSV or PDF \
                        document to markdown, preserving headings, lists, links and tables. The \
                        format is detected from the bytes. Responses are capped; pass max_chars 0 \
                        to take the whole document. For a PDF prefer pdf::classify first, which \
                        reports which pages need OCR.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    #[serde(flatten)]
    pub source: DocumentSource,

    /// Force a format instead of detecting one. Only needed when the content
    /// carries no signature and the file name is absent or wrong.
    #[serde(default)]
    pub format: Option<Format>,

    /// Characters to return before truncating. Omit for the configured
    /// default; `0` returns the whole document.
    #[serde(default)]
    pub max_chars: Option<usize>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// The format that was converted.
    pub format: Format,

    /// What the document is: prose, a spreadsheet, a presentation, a book, a
    /// PDF.
    pub family: Family,

    /// How the format was arrived at.
    pub detected_from: DetectedFrom,

    /// The markdown, capped per `max_chars`.
    pub body: Body,

    /// Embedded images and objects the document carries. Their bytes are not
    /// here — call `document::extract-assets` for those — but the count says
    /// whether a deck's content is pictures rather than text, which markdown
    /// alone would not reveal.
    pub asset_count: usize,

    /// Source label: the file name, or `<inline>` for an in-memory document.
    pub source: String,

    /// Wall-clock time for the conversion.
    pub elapsed_ms: u64,
}

pub fn handle(req: Request, cfg: &WorkerConfig) -> Result<Response, String> {
    let bytes = req.source.load(cfg)?;
    let started = std::time::Instant::now();

    let file_name = req.source.file_name_hint();
    let (format, detected_from) = format::resolve_or_explain(
        req.format,
        &bytes,
        file_name.as_deref(),
        &req.source.label(),
    )?;

    let markdown = anydoc::to_markdown_bytes(&bytes, format.to_anydoc())
        .map_err(|e| describe_error("markdown conversion", &e))?;

    // The asset count comes from the document model, and reaching it costs a
    // SECOND parse: the converter builds the model internally, renders it, and
    // drops it, and its only other public entry point is the parse itself. That
    // is the price of telling a caller a deck's content is pictures rather than
    // letting it read as an empty document, so it is only paid for formats that
    // can actually carry an asset. A failure here is not a reason to throw away
    // markdown that converted fine.
    let asset_count = if format.carries_assets() {
        anydoc::to_document(&bytes, format.to_anydoc())
            .map(|doc| doc.assets.len())
            .unwrap_or(0)
    } else {
        0
    };

    let max_chars = cfg.effective_max_chars(req.max_chars);
    let body = Body::new(markdown, max_chars, cfg.preview_chars);

    Ok(Response {
        format,
        family: format.family(),
        detected_from,
        body,
        asset_count,
        source: req.source.label(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine as _;

    use super::*;

    fn convert(bytes: &[u8], file_name: Option<&str>, max_chars: Option<usize>) -> Response {
        let req = Request {
            source: DocumentSource {
                bytes_base64: Some(BASE64.encode(bytes)),
                file_name: file_name.map(str::to_string),
                ..DocumentSource::default()
            },
            format: None,
            max_chars,
        };
        handle(req, &WorkerConfig::default()).expect("converts")
    }

    /// CSV is the one format with no signature, so it exercises the whole
    /// detection fallback as well as the conversion.
    #[test]
    fn a_csv_converts_to_a_markdown_table() {
        let response = convert(b"name,total\nrohit,3\nsam,4\n", Some("rows.csv"), None);
        assert_eq!(response.format, Format::Csv);
        assert_eq!(response.family, Family::Spreadsheet);
        assert_eq!(response.detected_from, DetectedFrom::Extension);
        assert!(
            response.body.text.contains("name"),
            "{}",
            response.body.text
        );
        assert!(
            response.body.text.contains("rohit"),
            "{}",
            response.body.text
        );
        assert!(!response.body.truncated);
    }

    /// The cap is what keeps a long document from eating the context the
    /// question needed, and a truncated body has to say so.
    #[test]
    fn the_body_reports_truncation() {
        let response = convert(b"name,total\nrohit,3\nsam,4\n", Some("rows.csv"), Some(8));
        assert!(response.body.truncated);
        assert_eq!(response.body.chars, 8);
        assert!(response.body.total_chars > 8);
        assert!(response.body.preview.is_some());
    }

    #[test]
    fn an_unrecognisable_file_says_to_name_the_format() {
        let req = Request {
            source: DocumentSource {
                bytes_base64: Some(BASE64.encode(b"\x00\x01\x02\x03")),
                file_name: Some("mystery.bin".into()),
                ..DocumentSource::default()
            },
            format: None,
            max_chars: None,
        };
        let err = handle(req, &WorkerConfig::default()).expect_err("unrecognisable");
        assert!(err.contains("Pass `format`"), "{err}");
        assert!(err.contains("mystery.bin"), "{err}");
    }

    /// A forced format skips detection, which is the escape hatch for a file
    /// whose name and content both lie.
    #[test]
    fn an_explicit_format_overrides_detection() {
        let req = Request {
            source: DocumentSource {
                bytes_base64: Some(BASE64.encode(b"a,b\n1,2\n")),
                file_name: Some("data.txt".into()),
                ..DocumentSource::default()
            },
            format: Some(Format::Csv),
            max_chars: None,
        };
        let response = handle(req, &WorkerConfig::default()).expect("converts as csv");
        assert_eq!(response.detected_from, DetectedFrom::Requested);
        assert!(response.body.text.contains('1'));
    }

    /// A conversion failure has to arrive with the next move attached — an
    /// agent that reads only "unsupported input" retries the same call.
    #[test]
    fn a_conversion_failure_carries_advice() {
        let req = Request {
            source: DocumentSource {
                // A ZIP magic number with nothing inside it: recognised as a
                // package, unusable as a document.
                bytes_base64: Some(BASE64.encode(b"PK\x03\x04")),
                file_name: Some("broken.docx".into()),
                ..DocumentSource::default()
            },
            format: Some(Format::Docx),
            max_chars: None,
        };
        let err = handle(req, &WorkerConfig::default()).expect_err("cannot convert");
        assert!(err.contains("markdown conversion failed"), "{err}");
    }
}
