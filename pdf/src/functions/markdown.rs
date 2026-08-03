//! `pdf::to-markdown` — a text-based document as markdown that keeps its shape.
//!
//! Headings, lists, links and tables survive; the parser reconstructs them from
//! font sizes, geometry and ruled lines rather than from any structure the file
//! promises to have.
//!
//! The size cap is the other half of the job. A long report runs to hundreds of
//! thousands of characters, and handing that to a model wastes the context it
//! needed for the answer. Responses are capped by default and say so; a caller
//! that genuinely wants the whole document passes `max_chars: 0`, which is what
//! a worker-to-worker pipeline does when the document is going to storage
//! rather than to a model.

use std::collections::HashSet;

use pdf_inspector::{MarkdownOptions, MarkdownProfile, PdfOptions, ProcessMode};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::WorkerConfig;
use crate::functions::classify::{detection_config, DocumentType, PageOcrReason};
use crate::source::{describe_error, to_parser_pages, Body, PdfSource};

pub const ID: &str = "pdf::to-markdown";
pub const DESC: &str = "Convert a text-based PDF to markdown, preserving headings, lists, links \
                        and tables. Returns nothing for a scanned document — call pdf::classify \
                        first. Responses are capped; pass max_chars 0 to take the whole document, \
                        or pages to take a slice of it.";

/// How faithful the markdown should be to the source characters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    /// Preserve the source text as written.
    #[default]
    Fidelity,
    /// Prefer shorter output, collapsing runs like the dot leaders in a table
    /// of contents. Not character-faithful to the source.
    Compact,
}

impl From<Profile> for MarkdownProfile {
    fn from(value: Profile) -> Self {
        match value {
            Profile::Fidelity => MarkdownProfile::Fidelity,
            Profile::Compact => MarkdownProfile::Compact,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    #[serde(flatten)]
    pub source: PdfSource,

    /// Password for an encrypted document. Never logged or echoed back.
    #[serde(default)]
    pub password: Option<String>,

    /// 1-indexed pages to convert. Omit for the whole document. A page filter
    /// is the cheap way to read a long report: take the pages you need rather
    /// than the whole thing truncated.
    #[serde(default)]
    pub pages: Option<Vec<u32>>,

    /// Characters to return before truncating. Omit for the configured
    /// default; `0` returns the whole document.
    #[serde(default)]
    pub max_chars: Option<usize>,

    /// Source fidelity versus token efficiency.
    #[serde(default)]
    pub profile: Profile,

    /// Include `[Image: …]` placeholders. Off by default: nothing here decodes
    /// pixels, so a placeholder adds noise without adding information.
    #[serde(default)]
    pub include_images: bool,

    /// Strip repeated running headers and footers.
    #[serde(default = "default_true")]
    pub strip_headers_footers: bool,

    /// Return markdown per page as well as the joined document. Useful when a
    /// caller wants to route some pages to OCR and keep the rest.
    #[serde(default)]
    pub per_page: bool,
}

fn default_true() -> bool {
    true
}

/// One page of markdown, with its own OCR verdict.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PageResult {
    /// 1-indexed page number.
    pub page: u32,
    /// Markdown for this page.
    pub markdown: String,
    /// `true` when this page's text is not trustworthy and OCR would do better.
    pub needs_ocr: bool,
    /// Machine-readable reason, when the cause is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ocr_reason: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// The document-level verdict, so a caller that skipped `pdf::classify`
    /// still learns it got nothing because the document is a scan.
    pub document_type: DocumentType,

    /// The markdown, capped per `max_chars`.
    pub body: Body,

    /// Pages in the document.
    pub page_count: u32,

    /// Pages actually converted. Equal to `page_count` unless `pages` was set.
    pub pages_converted: u32,

    /// Per-page markdown, when `per_page` was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<Vec<PageResult>>,

    /// 1-indexed pages holding a detected table.
    pub pages_with_tables: Vec<u32>,

    /// 1-indexed pages laid out in multiple columns.
    pub pages_with_columns: Vec<u32>,

    /// 1-indexed pages that need OCR.
    pub pages_needing_ocr: Vec<u32>,

    /// Per-page explanation for `pages_needing_ocr`.
    pub ocr_reasons: Vec<PageOcrReason>,

    /// `true` when font encodings decoded badly. The markdown, if any, is not
    /// to be trusted.
    pub has_encoding_issues: bool,

    /// Source label: the file name, or `<inline>` for an in-memory document.
    pub source: String,

    /// Wall-clock time for the conversion.
    pub elapsed_ms: u64,
}

fn markdown_options(req: &Request) -> MarkdownOptions {
    MarkdownOptions {
        profile: req.profile.into(),
        include_images: req.include_images,
        strip_headers_footers: req.strip_headers_footers,
        ..MarkdownOptions::default()
    }
}

pub fn handle(req: Request, cfg: &WorkerConfig) -> Result<Response, String> {
    let bytes = req.source.load(cfg)?;
    let started = std::time::Instant::now();

    let page_filter: Option<HashSet<u32>> = match &req.pages {
        Some(pages) if pages.is_empty() => {
            return Err("`pages` was empty; omit it to convert the whole document".to_string())
        }
        // The whole-document options take 1-indexed pages, unlike the per-page
        // entry point below. Both are covered by tests.
        Some(pages) => {
            for &page in pages {
                if page == 0 {
                    return Err("page numbers are 1-indexed; 0 is not a page".to_string());
                }
            }
            Some(pages.iter().copied().collect())
        }
        None => None,
    };

    let options = PdfOptions {
        mode: ProcessMode::Full,
        detection: detection_config(cfg, None),
        markdown: markdown_options(&req),
        page_filter: page_filter.clone(),
        password: req.password.clone(),
    };

    let result = pdf_inspector::process_pdf_mem_with_options(&bytes, options)
        .map_err(|e| describe_error("markdown conversion", e, req.password.is_some(), true))?;

    let max_chars = cfg.effective_max_chars(req.max_chars);
    let body = Body::new(
        result.markdown.unwrap_or_default(),
        max_chars,
        cfg.preview_chars,
    );

    let pages_converted = page_filter
        .as_ref()
        .map(|f| f.len() as u32)
        .unwrap_or(result.page_count);

    let per_page = if req.per_page {
        Some(extract_per_page(&bytes, req.pages.as_deref())?)
    } else {
        None
    };

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
        body,
        page_count: result.page_count,
        pages_converted,
        pages: per_page,
        pages_with_tables: result.layout.pages_with_tables,
        pages_with_columns: result.layout.pages_with_columns,
        pages_needing_ocr: result.pages_needing_ocr,
        ocr_reasons,
        has_encoding_issues: result.has_encoding_issues,
        source: req.source.label(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// Per-page markdown. The parser's per-page entry point counts pages from zero
/// on the way in and on the way out, so both directions are converted here.
fn extract_per_page(bytes: &[u8], pages: Option<&[u32]>) -> Result<Vec<PageResult>, String> {
    let parser_pages = match pages {
        Some(pages) => Some(to_parser_pages(pages)?),
        None => None,
    };
    let extracted = pdf_inspector::extract_pages_markdown_mem(bytes, parser_pages.as_deref())
        .map_err(|e| describe_error("per-page extraction", e, false, false))?;

    Ok(extracted
        .pages
        .into_iter()
        .map(|p| PageResult {
            page: crate::source::to_wire_page(p.page),
            markdown: p.markdown,
            needs_ocr: p.needs_ocr,
            ocr_reason: p.ocr_reason,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_maps_to_the_parser() {
        assert_eq!(
            MarkdownProfile::from(Profile::Fidelity),
            MarkdownProfile::Fidelity
        );
        assert_eq!(
            MarkdownProfile::from(Profile::Compact),
            MarkdownProfile::Compact
        );
    }

    #[test]
    fn profile_defaults_to_fidelity() {
        assert_eq!(Profile::default(), Profile::Fidelity);
    }

    #[test]
    fn images_are_excluded_by_default() {
        let req: Request = serde_json::from_value(serde_json::json!({ "path": "x.pdf" }))
            .expect("minimal request parses");
        assert!(!req.include_images);
        assert!(req.strip_headers_footers);
        assert!(!markdown_options(&req).include_images);
    }

    #[test]
    fn a_source_field_is_still_required_after_flattening() {
        let req: Request = serde_json::from_value(serde_json::json!({}))
            .expect("shape parses; validation is late");
        let err = req
            .source
            .load(&WorkerConfig::default())
            .expect_err("no source");
        assert!(err.contains("provide a `path`"), "{err}");
    }
}
