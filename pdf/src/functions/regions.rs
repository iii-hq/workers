//! `pdf::extract-regions` — read only what sits inside a box.
//!
//! This is the hybrid path. A vision model looks at a rendered page, finds the
//! invoice total or the table, and hands back a bounding box. Rather than trust
//! the model's transcription of the characters, ask the document: the real
//! text is already in the file, exact, with no chance of a misread digit.
//!
//! Two modes. `text` returns the characters inside the box. `table` runs table
//! detection over the items inside the box and returns a markdown table.
//!
//! Coordinates here are PDF points with the origin at the **top left**, which
//! is what a layout model produces. `pdf::extract-items` uses bottom-left,
//! which is the PDF convention. The two disagree deliberately, each matching
//! its own callers, and every response says which it used.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::WorkerConfig;
use crate::source::{describe_error, to_parser_page, to_wire_page, PdfSource};

pub const ID: &str = "pdf::extract-regions";
pub const DESC: &str = "Extract the real text, or a markdown table, from inside bounding boxes on \
                        given pages. Built for the hybrid path where a vision model locates a \
                        region and the exact characters come from the document rather than from a \
                        transcription. Coordinates are PDF points with a top-left origin.";

/// What to pull out of each box.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// The characters inside the box, as flat text.
    #[default]
    Text,
    /// A markdown table, when the items inside the box form one.
    Table,
}

/// Boxes to read on one page.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PageRegions {
    /// 1-indexed page number.
    pub page: u32,
    /// Boxes as `[x1, y1, x2, y2]` in PDF points, origin at the top left.
    pub boxes: Vec<[f32; 4]>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    #[serde(flatten)]
    pub source: PdfSource,

    /// One entry per page, each carrying the boxes to read on it.
    pub regions: Vec<PageRegions>,

    /// Flat text, or a markdown table.
    #[serde(default)]
    pub mode: Mode,
}

/// What one box held.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RegionResult {
    /// The text, or the markdown table in `table` mode.
    pub text: String,
    /// `true` when the extraction is not trustworthy: an empty box, a font the
    /// parser cannot decode, or text that decodes to nonsense. In `table` mode
    /// it also means no table structure was found.
    pub needs_ocr: bool,
    /// Machine-readable reason, when the cause is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ocr_reason: Option<String>,
}

/// Results for one page, parallel to that page's requested boxes.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PageResult {
    /// 1-indexed page number.
    pub page: u32,
    /// One result per requested box, in the order they were given.
    pub regions: Vec<RegionResult>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// One entry per requested page, in the order they were given.
    pub pages: Vec<PageResult>,

    /// Boxes read across every page.
    pub region_count: usize,

    /// Boxes whose result should not be trusted.
    pub regions_needing_ocr: usize,

    /// Origin convention the requested boxes were read under, always
    /// `pdf-points, top-left`. Stated on every response because a caller that
    /// assumed the other convention gets text from the wrong end of the page
    /// with no error. Note `pdf::extract-items` reports bottom-left instead.
    pub coordinate_origin: String,

    /// Source label: the file name, or `<inline>` for an in-memory document.
    pub source: String,

    /// Wall-clock time for the extraction.
    pub elapsed_ms: u64,
}

pub const COORDINATE_ORIGIN: &str = "pdf-points, top-left";

/// Boxes to read on one page, in the shape the parser takes them: a 0-indexed
/// page number and its `[x1, y1, x2, y2]` boxes.
type ParserPageRegions = (u32, Vec<[f32; 4]>);

/// Convert the wire's 1-indexed pages to the 0-indexed pages this parser entry
/// point expects, rejecting an empty request rather than doing no work quietly.
fn to_parser_regions(regions: &[PageRegions]) -> Result<Vec<ParserPageRegions>, String> {
    if regions.is_empty() {
        return Err("`regions` was empty; give at least one page and box".to_string());
    }
    regions
        .iter()
        .map(|r| {
            if r.boxes.is_empty() {
                return Err(format!("page {} was given no boxes", r.page));
            }
            for b in &r.boxes {
                if b[2] <= b[0] || b[3] <= b[1] {
                    return Err(format!(
                        "page {}: box [{}, {}, {}, {}] is empty or inverted; expected \
                         [x1, y1, x2, y2] with x2 > x1 and y2 > y1",
                        r.page, b[0], b[1], b[2], b[3]
                    ));
                }
            }
            Ok((to_parser_page(r.page)?, r.boxes.clone()))
        })
        .collect()
}

pub fn handle(req: Request, cfg: &WorkerConfig) -> Result<Response, String> {
    let page_regions = to_parser_regions(&req.regions)?;
    let bytes = req.source.load(cfg)?;
    let started = std::time::Instant::now();

    let extracted = match req.mode {
        Mode::Text => pdf_inspector::extract_text_in_regions_mem(&bytes, &page_regions),
        Mode::Table => pdf_inspector::extract_tables_in_regions_mem(&bytes, &page_regions),
    }
    .map_err(|e| describe_error("region extraction", e, false, false))?;

    let mut region_count = 0usize;
    let mut regions_needing_ocr = 0usize;
    let pages: Vec<PageResult> = extracted
        .into_iter()
        .map(|p| {
            let regions: Vec<RegionResult> = p
                .regions
                .into_iter()
                .map(|r| {
                    region_count += 1;
                    if r.needs_ocr {
                        regions_needing_ocr += 1;
                    }
                    RegionResult {
                        text: r.text,
                        needs_ocr: r.needs_ocr,
                        ocr_reason: r.ocr_reason,
                    }
                })
                .collect();
            PageResult {
                page: to_wire_page(p.page),
                regions,
            }
        })
        .collect();

    Ok(Response {
        pages,
        region_count,
        regions_needing_ocr,
        coordinate_origin: COORDINATE_ORIGIN.to_string(),
        source: req.source.label(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regions(page: u32) -> PageRegions {
        PageRegions {
            page,
            boxes: vec![[10.0, 20.0, 110.0, 60.0]],
        }
    }

    #[test]
    fn pages_are_converted_to_the_parsers_zero_indexed_numbering() {
        let converted = to_parser_regions(&[regions(1), regions(5)]).expect("valid");
        assert_eq!(converted[0].0, 0);
        assert_eq!(converted[1].0, 4);
    }

    #[test]
    fn page_zero_is_rejected_rather_than_wrapping() {
        let err = to_parser_regions(&[regions(0)]).expect_err("page 0");
        assert!(err.contains("1-indexed"), "{err}");
    }

    #[test]
    fn an_empty_request_is_a_caller_mistake() {
        let err = to_parser_regions(&[]).expect_err("no regions");
        assert!(err.contains("at least one"), "{err}");

        let err = to_parser_regions(&[PageRegions {
            page: 1,
            boxes: vec![],
        }])
        .expect_err("no boxes");
        assert!(err.contains("no boxes"), "{err}");
    }

    /// An inverted box silently returns nothing, which reads as "the document
    /// has no text there" rather than "the caller swapped two numbers".
    #[test]
    fn an_inverted_box_is_rejected() {
        let err = to_parser_regions(&[PageRegions {
            page: 1,
            boxes: vec![[110.0, 60.0, 10.0, 20.0]],
        }])
        .expect_err("inverted");
        assert!(err.contains("inverted"), "{err}");
    }

    #[test]
    fn mode_defaults_to_text() {
        assert_eq!(Mode::default(), Mode::Text);
    }

    /// The two coordinate conventions in this worker must stay distinct and
    /// explicit; collapsing them would be silently wrong, not loudly broken.
    #[test]
    fn region_and_item_origins_disagree_on_purpose() {
        assert_ne!(
            COORDINATE_ORIGIN,
            crate::functions::items::COORDINATE_ORIGIN
        );
        assert!(COORDINATE_ORIGIN.contains("top-left"));
        assert!(crate::functions::items::COORDINATE_ORIGIN.contains("bottom-left"));
    }
}
