//! `document::extract-assets` — the pictures inside a document.
//!
//! Markdown renders an embedded image as its alt text, which is the right
//! default for text but throws away the one thing a slide deck is often made
//! of. A deck whose content is diagrams converts to a page of titles and reads
//! as an empty document; the pictures are the content, and a model that can see
//! images can use them.
//!
//! Two ceilings apply, and both report what they dropped rather than trimming
//! silently. `max_assets` bounds how many come back at all. `max_asset_bytes`
//! bounds the payload of one: a larger asset is still listed with its type and
//! size, so a caller learns it exists and can go read the file itself, but its
//! bytes are left out rather than base64'd into a response nobody can hold.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::WorkerConfig;
use crate::format::{self, Format};
use crate::source::{describe_error, DocumentSource};

pub const ID: &str = "document::extract-assets";
pub const DESC: &str = "Pull the images and embedded objects out of a document as base64, for a \
                        deck or report whose content is pictures rather than text. Capped per \
                        response and per asset; anything left out is still listed with its type \
                        and size. Not available for PDFs — use pdf::extract-regions.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    #[serde(flatten)]
    pub source: DocumentSource,

    /// Force a format instead of detecting one.
    #[serde(default)]
    pub format: Option<Format>,

    /// Assets to return in this response. Narrows the configured ceiling; it
    /// cannot raise it.
    #[serde(default)]
    pub max_assets: Option<usize>,

    /// Return only assets whose media type starts with this, e.g. `image/`.
    /// Omit for every asset.
    #[serde(default)]
    pub media_type_prefix: Option<String>,

    /// Include the base64 payload. Set `false` to inventory a document — what
    /// it holds and how big — without moving the bytes.
    #[serde(default = "default_true")]
    pub include_bytes: bool,
}

fn default_true() -> bool {
    true
}

/// One embedded asset. `bytes_base64` is absent when the caller asked for an
/// inventory, or when this asset is over the per-asset ceiling — `omitted`
/// says which.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Asset {
    /// Position in the document's asset list, stable for a given document.
    pub index: usize,

    /// MIME type, e.g. `image/png`.
    pub media_type: String,

    /// The package part or stream it came from, for provenance.
    pub origin_part: String,

    /// Size of the payload in bytes, whether or not the payload is included.
    pub size_bytes: u64,

    /// The payload, base64-encoded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_base64: Option<String>,

    /// Why the payload is absent, when it is: `not_requested`, `too_large`
    /// (this asset alone is over the per-asset ceiling), or `budget_spent` (the
    /// response's total byte budget went on earlier assets — ask for this one
    /// on its own).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub omitted: Option<&'static str>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// The format that was parsed.
    pub format: Format,

    /// The assets, in document order, up to the effective ceiling.
    pub assets: Vec<Asset>,

    /// Assets the document holds after `media_type_prefix` is applied. Larger
    /// than `assets.len()` when the ceiling cut the response short.
    pub total_count: usize,

    /// `true` when the ceiling cut the response short.
    pub truncated: bool,

    /// Source label: the file name, or `<inline>` for an in-memory document.
    pub source: String,

    /// Wall-clock time for the extraction.
    pub elapsed_ms: u64,
}

pub fn handle(req: Request, cfg: &WorkerConfig) -> Result<Response, String> {
    let bytes = req.source.load(cfg)?;
    let started = std::time::Instant::now();

    let file_name = req.source.file_name_hint();
    let (format, _) = format::resolve_or_explain(
        req.format,
        &bytes,
        file_name.as_deref(),
        &req.source.label(),
    )?;

    // A PDF never builds a document model, so there is no asset list to walk.
    // Say where the pictures actually live rather than returning an empty list
    // that reads as "this document has none".
    if !format.has_document_model() {
        return Err(
            "a PDF carries no extractable asset list here; use pdf::extract-regions to read a \
             region of a page, or pdf::classify to find the pages that are images"
                .to_string(),
        );
    }

    let document = anydoc::to_document(&bytes, format.to_anydoc())
        .map_err(|e| describe_error("asset extraction", &e))?;

    let prefix = req.media_type_prefix.as_deref();
    let matching: Vec<(usize, &anydoc::model::Asset)> = document
        .assets
        .iter()
        .enumerate()
        .filter(|(_, asset)| prefix.is_none_or(|p| asset.media_type.starts_with(p)))
        .collect();

    let ceiling = cfg.effective_max_assets(req.max_assets);
    let total_count = matching.len();

    // Two budgets, because the per-asset one alone does not bound a response:
    // two dozen assets each just under the individual ceiling still add up to a
    // payload nobody asked for. Spending the total stops the ENCODING, not the
    // listing — every asset still comes back with its type and size, so the
    // caller sees what exists and can ask for it directly.
    let mut spent: u64 = 0;
    let assets = matching
        .into_iter()
        .take(ceiling)
        .map(|(index, asset)| {
            let size_bytes = asset.bytes.len() as u64;
            let (bytes_base64, omitted) = if !req.include_bytes {
                (None, Some("not_requested"))
            } else if size_bytes > cfg.max_asset_bytes {
                (None, Some("too_large"))
            } else if spent.saturating_add(size_bytes) > cfg.max_assets_total_bytes {
                (None, Some("budget_spent"))
            } else {
                spent = spent.saturating_add(size_bytes);
                (Some(BASE64.encode(&asset.bytes)), None)
            };
            Asset {
                index,
                media_type: asset.media_type.clone(),
                origin_part: asset.origin_part.clone(),
                size_bytes,
                bytes_base64,
                omitted,
            }
        })
        .collect::<Vec<_>>();

    Ok(Response {
        format,
        truncated: total_count > assets.len(),
        total_count,
        assets,
        source: req.source.label(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use base64::engine::general_purpose::STANDARD as BASE64;

    use super::*;

    /// A PDF has no asset list here, and an empty list would read as "this
    /// document holds no images", which is a different and wrong claim.
    #[test]
    fn a_pdf_is_refused_with_somewhere_to_go() {
        let req = Request {
            source: DocumentSource {
                bytes_base64: Some(BASE64.encode(b"%PDF-1.7\n")),
                file_name: Some("report.pdf".into()),
                ..DocumentSource::default()
            },
            format: None,
            max_assets: None,
            media_type_prefix: None,
            include_bytes: true,
        };
        let err = handle(req, &WorkerConfig::default()).expect_err("no model for a pdf");
        assert!(err.contains("pdf::extract-regions"), "{err}");
    }

    /// A CSV parses into a model with no assets at all, which is the honest
    /// empty case: success, zero assets, nothing truncated.
    #[test]
    fn a_document_with_no_assets_returns_an_empty_list() {
        let req = Request {
            source: DocumentSource {
                bytes_base64: Some(BASE64.encode(b"a,b\n1,2\n")),
                file_name: Some("rows.csv".into()),
                ..DocumentSource::default()
            },
            format: None,
            max_assets: None,
            media_type_prefix: None,
            include_bytes: true,
        };
        let response = handle(req, &WorkerConfig::default()).expect("parses");
        assert!(response.assets.is_empty());
        assert_eq!(response.total_count, 0);
        assert!(!response.truncated);
    }

    #[test]
    fn an_unrecognisable_file_says_to_name_the_format() {
        let req = Request {
            source: DocumentSource {
                bytes_base64: Some(BASE64.encode(b"\x00\x01\x02")),
                file_name: Some("mystery.bin".into()),
                ..DocumentSource::default()
            },
            format: None,
            max_assets: None,
            media_type_prefix: None,
            include_bytes: true,
        };
        let err = handle(req, &WorkerConfig::default()).expect_err("unrecognisable");
        assert!(err.contains("Pass `format`"), "{err}");
    }

    #[test]
    fn include_bytes_defaults_to_true() {
        let req: Request = serde_json::from_value(serde_json::json!({ "path": "deck.pptx" }))
            .expect("minimal request parses");
        assert!(req.include_bytes);
    }
}
