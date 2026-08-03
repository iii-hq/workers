//! `pdf::extract-text` — the plain-text reading, with no attempt at structure.
//!
//! Cheaper than markdown and the right call when the caller is going to search
//! or embed the result rather than read it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::WorkerConfig;
use crate::source::{describe_error, Body, PdfSource};

pub const ID: &str = "pdf::extract-text";
pub const DESC: &str = "Extract a PDF as plain text, with no structure recovery. Cheaper than \
                        pdf::to-markdown and the right call when the text will be searched or \
                        embedded rather than read.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    #[serde(flatten)]
    pub source: PdfSource,

    /// Characters to return before truncating. Omit for the configured
    /// default; `0` returns the whole document.
    #[serde(default)]
    pub max_chars: Option<usize>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// The text, capped per `max_chars`.
    pub body: Body,

    /// Source label: the file name, or `<inline>` for an in-memory document.
    pub source: String,

    /// Wall-clock time for the extraction.
    pub elapsed_ms: u64,
}

pub fn handle(req: Request, cfg: &WorkerConfig) -> Result<Response, String> {
    let bytes = req.source.load(cfg)?;
    let started = std::time::Instant::now();
    let text = pdf_inspector::extractor::extract_text_mem(&bytes)
        .map_err(|e| describe_error("text extraction", e, false, false))?;

    Ok(Response {
        body: Body::new(
            text,
            cfg.effective_max_chars(req.max_chars),
            cfg.preview_chars,
        ),
        source: req.source.label(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_document_that_is_not_a_pdf() {
        use base64::engine::general_purpose::STANDARD as BASE64;
        use base64::Engine as _;

        let req = Request {
            source: PdfSource {
                path: None,
                bytes_base64: Some(BASE64.encode(b"this is not a pdf")),
                fs_scope: None,
            },
            max_chars: None,
        };
        let err = handle(req, &WorkerConfig::default()).expect_err("not a pdf");
        assert!(err.contains("extract"), "{err}");
    }
}
