//! `pdf::extract-items` — where the text sits, not just what it says.
//!
//! Each item is one run of characters with its box on the page, its font, and
//! the styling the parser recovered. Underline and strikeout are geometric
//! findings: PDF has no flag for either, so they come from vector lines drawn
//! near the baseline.
//!
//! Coordinates are PDF points with the origin at the **bottom left** of the
//! page, which is the PDF convention and the opposite of every layout model's
//! output. The region functions use top-left instead, because that is what
//! their callers produce. Both are stated on the schema, because getting this
//! wrong is silent: the text comes back, it is just from the wrong end of the
//! page.

use std::collections::HashSet;

use pdf_inspector::types::ItemType;
use pdf_inspector::TextItem;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::WorkerConfig;
use crate::source::{describe_error, PdfSource};

pub const ID: &str = "pdf::extract-items";
pub const DESC: &str = "Extract positioned text items: the box, font, size and styling of every \
                        run of characters on a page. Coordinates are PDF points with a \
                        bottom-left origin. Use this for layout-aware reading; use \
                        pdf::to-markdown to just read the document.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    #[serde(flatten)]
    pub source: PdfSource,

    /// 1-indexed pages to read. Omit for the whole document.
    #[serde(default)]
    pub pages: Option<Vec<u32>>,

    /// Items to return before truncating. Omit for the configured default; `0`
    /// returns every item, which on a dense document is a very large response.
    #[serde(default)]
    pub max_items: Option<usize>,
}

/// What one item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    /// Ordinary text.
    Text,
    /// An image placeholder. The box is real; no pixels are decoded.
    Image,
    /// Text carrying a hyperlink; the target is in `link`.
    Link,
    /// A filled-in form field value.
    FormField,
}

/// One positioned run of characters.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Item {
    /// The characters.
    pub text: String,
    /// 1-indexed page number.
    pub page: u32,
    /// Left edge, PDF points from the left of the page.
    pub x: f32,
    /// Baseline, PDF points from the **bottom** of the page.
    pub y: f32,
    /// Width in PDF points.
    pub width: f32,
    /// Height in PDF points, approximated from the font size.
    pub height: f32,
    /// Font name as the document names it.
    pub font: String,
    /// Font size in points.
    pub font_size: f32,
    pub bold: bool,
    pub italic: bool,
    /// Recovered from vector lines near the baseline, not from a flag.
    pub underline: bool,
    /// Recovered from vector lines through the text, not from a flag.
    pub strikeout: bool,
    pub kind: ItemKind,
    /// Link target, for a `link` item.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// Marked-content id tying this item to the document's tagged structure
    /// tree, when the document has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcid: Option<i64>,
}

impl From<TextItem> for Item {
    fn from(item: TextItem) -> Self {
        let (kind, link) = match &item.item_type {
            ItemType::Text => (ItemKind::Text, None),
            ItemType::Image => (ItemKind::Image, None),
            ItemType::Link(url) => (ItemKind::Link, Some(url.clone())),
            ItemType::FormField => (ItemKind::FormField, None),
        };
        Self {
            text: item.text,
            page: item.page,
            x: item.x,
            y: item.y,
            width: item.width,
            height: item.height,
            font: item.font,
            font_size: item.font_size,
            bold: item.is_bold,
            italic: item.is_italic,
            underline: item.is_underline,
            strikeout: item.is_strikeout,
            kind,
            link,
            mcid: item.mcid,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// The items, in document order, capped per `max_items`.
    pub items: Vec<Item>,

    /// Items returned.
    pub count: usize,

    /// Items the document holds for the requested pages. Equal to `count` when
    /// nothing was dropped.
    pub total_count: usize,

    /// `true` when `items` stops short. Narrow `pages`, or pass `max_items: 0`.
    pub truncated: bool,

    /// Origin convention for `x` and `y`, always `pdf-points, bottom-left`.
    /// Stated on every response because a caller that assumed the other
    /// convention reads the wrong end of the page with no error. Note
    /// `pdf::extract-regions` takes boxes with a top-left origin instead.
    pub coordinate_origin: String,

    /// Source label: the file name, or `<inline>` for an in-memory document.
    pub source: String,

    /// Wall-clock time for the extraction.
    pub elapsed_ms: u64,
}

pub const COORDINATE_ORIGIN: &str = "pdf-points, bottom-left";

/// Validate a requested page list into the filter this entry point expects.
///
/// This entry point already counts pages from one, so the numbers pass through
/// unchanged. Page 0 is still rejected: as a filter value it would silently
/// match nothing and return an empty document.
pub fn page_filter(pages: Option<&[u32]>) -> Result<Option<HashSet<u32>>, String> {
    match pages {
        None => Ok(None),
        Some([]) => Err("`pages` was empty; omit it to read the whole document".to_string()),
        Some(pages) if pages.contains(&0) => {
            Err("page numbers are 1-indexed; 0 is not a page".to_string())
        }
        Some(pages) => Ok(Some(pages.iter().copied().collect())),
    }
}

pub fn handle(req: Request, cfg: &WorkerConfig) -> Result<Response, String> {
    let filter = page_filter(req.pages.as_deref())?;
    let bytes = req.source.load(cfg)?;
    let started = std::time::Instant::now();

    let raw =
        pdf_inspector::extractor::extract_text_with_positions_mem_pages(&bytes, filter.as_ref())
            .map_err(|e| describe_error("item extraction", e, false, false))?;

    let total_count = raw.len();
    let max_items = req.max_items.unwrap_or(cfg.max_items);
    let truncated = max_items > 0 && total_count > max_items;
    let items: Vec<Item> = if truncated {
        raw.into_iter().take(max_items).map(Item::from).collect()
    } else {
        raw.into_iter().map(Item::from).collect()
    };

    Ok(Response {
        count: items.len(),
        items,
        total_count,
        truncated,
        coordinate_origin: COORDINATE_ORIGIN.to_string(),
        source: req.source.label(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(item_type: ItemType) -> TextItem {
        TextItem {
            text: "hello".to_string(),
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
            font: "Helvetica".to_string(),
            font_size: 12.0,
            page: 1,
            is_bold: true,
            is_italic: false,
            is_underline: true,
            is_strikeout: false,
            item_type,
            mcid: Some(7),
        }
    }

    #[test]
    fn item_kinds_map_across() {
        assert_eq!(Item::from(sample(ItemType::Text)).kind, ItemKind::Text);
        assert_eq!(Item::from(sample(ItemType::Image)).kind, ItemKind::Image);
        assert_eq!(
            Item::from(sample(ItemType::FormField)).kind,
            ItemKind::FormField
        );
    }

    #[test]
    fn a_link_carries_its_target() {
        let item = Item::from(sample(ItemType::Link("https://example.com".to_string())));
        assert_eq!(item.kind, ItemKind::Link);
        assert_eq!(item.link.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn geometry_and_styling_survive_the_conversion() {
        let item = Item::from(sample(ItemType::Text));
        assert_eq!(
            (item.x, item.y, item.width, item.height),
            (1.0, 2.0, 3.0, 4.0)
        );
        assert!(item.bold);
        assert!(item.underline);
        assert!(!item.strikeout);
        assert_eq!(item.mcid, Some(7));
        assert_eq!(item.page, 1);
    }

    #[test]
    fn page_zero_is_rejected_rather_than_matching_nothing() {
        let err = page_filter(Some(&[1, 0])).expect_err("page 0");
        assert!(err.contains("1-indexed"), "{err}");
    }

    #[test]
    fn an_empty_page_list_is_a_caller_mistake() {
        let err = page_filter(Some(&[])).expect_err("empty list");
        assert!(err.contains("omit it"), "{err}");
    }

    #[test]
    fn pages_pass_through_unconverted() {
        let filter = page_filter(Some(&[1, 3])).expect("valid").expect("some");
        assert!(filter.contains(&1) && filter.contains(&3));
        assert_eq!(filter.len(), 2);
        assert_eq!(page_filter(None), Ok(None));
    }
}
