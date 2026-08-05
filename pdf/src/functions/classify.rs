//! `pdf::classify` — the routing call.
//!
//! Answers one question in tens of milliseconds: does this document hold real
//! text, or is it pictures of text? Everything expensive downstream hangs off
//! that answer, so it samples content streams rather than extracting anything.
//!
//! The per-page verdict matters as much as the document-level one. A report
//! with a scanned cover and two hundred text pages is not a scanned document,
//! and treating it as one sends the whole thing to an OCR service for the sake
//! of one page.

use pdf_inspector::{DetectionConfig, PdfType, ScanStrategy};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::WorkerConfig;
use crate::source::{describe_error, PdfSource};

pub const ID: &str = "pdf::classify";
pub const DESC: &str = "Classify a PDF as text-based, scanned, image-based or mixed, and report \
                        which pages need OCR and why. Samples content streams rather than \
                        extracting text, so it answers in tens of milliseconds. Call this before \
                        any other pdf function.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    #[serde(flatten)]
    pub source: PdfSource,

    /// Password for an encrypted document. Never logged or echoed back.
    #[serde(default)]
    pub password: Option<String>,

    /// Pages sampled for the verdict, overriding the configured default. `0`
    /// scans every page, which is slower but settles a borderline mixed
    /// document.
    #[serde(default)]
    pub sample_pages: Option<usize>,
}

/// What a document is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DocumentType {
    /// Real text throughout. Extract locally.
    TextBased,
    /// Pictures of pages. Every page needs OCR.
    Scanned,
    /// Images with little or no text layer.
    ImageBased,
    /// Some pages carry text, others do not. Read `pages_needing_ocr`.
    Mixed,
}

impl From<PdfType> for DocumentType {
    fn from(value: PdfType) -> Self {
        match value {
            PdfType::TextBased => Self::TextBased,
            PdfType::Scanned => Self::Scanned,
            PdfType::ImageBased => Self::ImageBased,
            PdfType::Mixed => Self::Mixed,
        }
    }
}

/// Why one page cannot be read without OCR.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PageOcrReason {
    /// 1-indexed page number.
    pub page: u32,
    /// Machine-readable reasons: `scanned` (a raster page), `no_text` (nothing
    /// extractable and nothing to OCR), `vector_text` (characters drawn as
    /// outlines rather than text) or `suspected_garbled_text` (a text layer
    /// that decodes to nonsense).
    pub reasons: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// The document-level verdict.
    pub document_type: DocumentType,

    /// How much to trust the verdict, from 0.0 to 1.0.
    pub confidence: f32,

    /// Pages in the document.
    pub page_count: u32,

    /// Pages actually inspected. Lower than `page_count` when sampling, so a
    /// verdict from a sample can be told apart from one that read everything.
    /// Absent for an encrypted document, which takes a decryption path that
    /// does not report the counters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages_sampled: Option<u32>,

    /// Inspected pages that carry text operators. Absent for an encrypted
    /// document, for the same reason as `pages_sampled`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages_with_text: Option<u32>,

    /// 1-indexed pages that cannot be read without OCR. Empty for a clean
    /// text-based document.
    pub pages_needing_ocr: Vec<u32>,

    /// Per-page explanation for `pages_needing_ocr`.
    pub ocr_reasons: Vec<PageOcrReason>,

    /// `true` when the images carry meaning the text layer does not, so OCR
    /// adds something even on a text-based document. Absent for an encrypted
    /// document, for the same reason as `pages_sampled`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ocr_recommended: Option<bool>,

    /// Document title from the PDF metadata, when it has one.
    pub title: Option<String>,

    /// `true` when font encodings decoded badly. Only known on the encrypted
    /// path, which extracts far enough to notice; absent otherwise, where
    /// `suspected_garbled_text` in `ocr_reasons` carries the same signal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_encoding_issues: Option<bool>,

    /// Source label: the file name, or `<inline>` for an in-memory document.
    pub source: String,

    /// Wall-clock time for the classification.
    pub elapsed_ms: u64,
}

/// Build the detection config for one call: worker defaults, with the per-call
/// sample override applied.
pub fn detection_config(cfg: &WorkerConfig, sample_pages: Option<usize>) -> DetectionConfig {
    let sample = sample_pages.unwrap_or(cfg.classify_sample_pages);
    DetectionConfig {
        // A zero sample means "look at everything". `Full` says that precisely;
        // `Sample(0)` would be a request to look at nothing.
        strategy: if sample == 0 {
            ScanStrategy::Full
        } else {
            // Saturate rather than cast: a `sample` above u32::MAX would wrap,
            // and a wrap to 0 means Sample(0), which samples nothing at all.
            ScanStrategy::Sample(u32::try_from(sample).unwrap_or(u32::MAX))
        },
        min_text_ops_per_page: cfg.min_text_ops_per_page as u32,
        text_page_ratio_threshold: cfg.text_page_ratio_threshold,
    }
}

pub fn handle(req: Request, cfg: &WorkerConfig) -> Result<Response, String> {
    let bytes = req.source.load(cfg)?;
    let started = std::time::Instant::now();
    let detection = detection_config(cfg, req.sample_pages);

    // Two paths, because the parser's fast detection entry point takes no
    // password and simply errors on an encrypted document. The decrypting path
    // runs the same detection under the same config, it just returns fewer
    // counters, so those fields go absent rather than being invented.
    let mut response = match &req.password {
        Some(_) => classify_encrypted(&bytes, detection, req.password.clone())?,
        None => classify_plain(&bytes, detection)?,
    };

    response.source = req.source.label();
    response.elapsed_ms = started.elapsed().as_millis() as u64;
    Ok(response)
}

fn classify_plain(bytes: &[u8], detection: DetectionConfig) -> Result<Response, String> {
    let result = pdf_inspector::detect_pdf_type_mem_with_config(bytes, detection)
        .map_err(|e| describe_error("classify", e, false, true))?;

    // Both `pages_needing_ocr` and the reason map are 1-indexed here, unlike
    // the parser's lightweight classification entry point, which counts from
    // zero. Covered by test.
    let ocr_reasons = result
        .ocr_reasons_by_page
        .into_iter()
        .map(|(page, reasons)| PageOcrReason { page, reasons })
        .collect();

    Ok(Response {
        document_type: result.pdf_type.into(),
        confidence: result.confidence,
        page_count: result.page_count,
        pages_sampled: Some(result.pages_sampled),
        pages_with_text: Some(result.pages_with_text),
        pages_needing_ocr: result.pages_needing_ocr,
        ocr_reasons,
        ocr_recommended: Some(result.ocr_recommended),
        title: result.title,
        has_encoding_issues: None,
        source: String::new(),
        elapsed_ms: 0,
    })
}

fn classify_encrypted(
    bytes: &[u8],
    detection: DetectionConfig,
    password: Option<String>,
) -> Result<Response, String> {
    let result = pdf_inspector::process_pdf_mem_with_options(
        bytes,
        pdf_inspector::PdfOptions {
            mode: pdf_inspector::ProcessMode::DetectOnly,
            detection,
            markdown: Default::default(),
            page_filter: None,
            password,
        },
    )
    .map_err(|e| describe_error("classify", e, true, true))?;

    let ocr_reasons = result
        .ocr_reasons_by_page
        .into_iter()
        .map(|r| PageOcrReason {
            page: r.page,
            reasons: r.reasons,
        })
        .collect();

    Ok(Response {
        document_type: result.pdf_type.into(),
        confidence: result.confidence,
        page_count: result.page_count,
        pages_sampled: None,
        pages_with_text: None,
        pages_needing_ocr: result.pages_needing_ocr,
        ocr_reasons,
        ocr_recommended: None,
        title: result.title,
        has_encoding_issues: Some(result.has_encoding_issues),
        source: String::new(),
        elapsed_ms: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_sample_pages_means_scan_everything() {
        let cfg = WorkerConfig::default();
        assert!(matches!(
            detection_config(&cfg, Some(0)).strategy,
            ScanStrategy::Full
        ));
    }

    #[test]
    fn sample_override_beats_the_configured_default() {
        let cfg = WorkerConfig {
            classify_sample_pages: 8,
            ..WorkerConfig::default()
        };
        assert!(matches!(
            detection_config(&cfg, Some(3)).strategy,
            ScanStrategy::Sample(3)
        ));
        assert!(matches!(
            detection_config(&cfg, None).strategy,
            ScanStrategy::Sample(8)
        ));
    }

    /// The parser's own default is `Sample(8)`, not the early-exit strategy its
    /// documentation advertises. This worker's default must track the code, and
    /// a dependency bump that changes it must break here rather than quietly
    /// reclassifying every report with an image cover.
    #[test]
    fn worker_default_matches_the_parsers_actual_default() {
        let parser_default = DetectionConfig::default();
        assert!(matches!(parser_default.strategy, ScanStrategy::Sample(8)));

        let cfg = WorkerConfig::default();
        let ours = detection_config(&cfg, None);
        assert!(matches!(ours.strategy, ScanStrategy::Sample(8)));
        assert_eq!(
            ours.min_text_ops_per_page,
            parser_default.min_text_ops_per_page
        );
        assert!(
            (ours.text_page_ratio_threshold - parser_default.text_page_ratio_threshold).abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn document_type_covers_every_parser_variant() {
        assert_eq!(
            DocumentType::from(PdfType::TextBased),
            DocumentType::TextBased
        );
        assert_eq!(DocumentType::from(PdfType::Scanned), DocumentType::Scanned);
        assert_eq!(
            DocumentType::from(PdfType::ImageBased),
            DocumentType::ImageBased
        );
        assert_eq!(DocumentType::from(PdfType::Mixed), DocumentType::Mixed);
    }

    #[test]
    fn document_type_serializes_in_snake_case() {
        let json = serde_json::to_string(&DocumentType::TextBased).expect("serializes");
        assert_eq!(json, "\"text_based\"");
    }
}
