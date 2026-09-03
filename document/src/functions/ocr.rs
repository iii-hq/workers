//! `document::ocr` — read a document nothing else here can read.
//!
//! Every other function in this worker walks a file's own structure. A scan has
//! none: it is pictures of text, and the characters exist only in the pixels.
//! This is the fallback branch of the same question the rest of the surface
//! answers, so it lives on the same worker rather than making a caller learn a
//! second one.
//!
//! Three inputs, one answer. An image goes straight to the model. A PDF is
//! rendered a page at a time by the `browser` worker, the only thing in the
//! fleet that turns a page into pixels. An office document whose text came back
//! empty has its embedded images pulled out and read the same way.
//!
//! Two rules shape the whole function, and both are about money. Nothing runs
//! implicitly: the attachment path reports a scan and names this function, and
//! an agent or a person decides to spend. And nothing is rendered before the
//! model is checked for vision, because a model that cannot see fails on the
//! first page after paying to produce it.

use std::sync::Arc;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::bus::{describe_bus_failure, Bus};
use crate::config::WorkerConfig;
use crate::format::{self, Format};
use crate::source::{Body, DocumentSource};

pub const ID: &str = "document::ocr";
pub const DESC: &str = "Transcribe a scanned document with no readable text: a scanned PDF, a \
                        photographed page, a picture-only deck. Renders the pages and reads them \
                        with a vision model. Costs money per page: pass `pages` (pdf::classify) to \
                        narrow it.";

/// The prompt every page is read with.
///
/// Transcription, not description: a model told to "describe this image" writes
/// prose about a document, and the caller wanted the document. The instruction
/// to say nothing else is what keeps "Here is the text of the page:" out of the
/// markdown that ends up in someone's context.
const TRANSCRIBE_PROMPT: &str = "Transcribe every word of text in this image, in reading order, \
     as markdown. Preserve headings, lists and tables. Do not describe the image, do not \
     summarise, and do not add commentary: output only the transcription. If the image holds no \
     legible text at all, output nothing.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    #[serde(flatten)]
    pub source: DocumentSource,

    /// 1-indexed PDF pages to transcribe; omit for every page up to the
    /// configured ceiling. Pass the scan pages `pdf::classify` reports.
    #[serde(default)]
    pub pages: Option<Vec<u32>>,

    /// Vision model to read with. Omit for the configured default. The model is
    /// checked for vision support before anything is rendered.
    #[serde(default)]
    pub model: Option<String>,

    /// Characters to return before truncating. Omit for the configured
    /// default; `0` returns everything transcribed.
    #[serde(default)]
    pub max_chars: Option<usize>,
}

/// What one page turned into.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PageText {
    /// 1-indexed page number, or the asset's index for an office document.
    pub page: u32,
    /// The transcription. Empty when the page held no legible text.
    pub text: String,
    pub chars: usize,
    /// `true` when this page came from the cache rather than the model.
    pub cached: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// How the pixels were obtained: `image`, `pdf-render` or `document-assets`.
    pub via: String,

    /// The joined transcription, capped per `max_chars`.
    pub body: Body,

    /// Per-page transcriptions, in order.
    pub pages: Vec<PageText>,

    /// Pages actually read by the model this call. Excludes cache hits, so this
    /// is what was paid for.
    pub pages_transcribed: usize,

    /// Pages served from the cache, costing nothing.
    pub pages_cached: usize,

    /// The model that read them.
    pub model: String,

    /// Source label: the file name, or `<inline>` for an in-memory document.
    pub source: String,

    /// Wall-clock time, rendering included.
    pub elapsed_ms: u64,
}

pub async fn handle(
    req: Request,
    cfg: Arc<WorkerConfig>,
    bus: Arc<dyn Bus>,
) -> Result<Response, String> {
    let bytes = req.source.load(&cfg)?;
    let started = std::time::Instant::now();
    let label = req.source.label();

    let model = req
        .model
        .clone()
        .or_else(|| cfg.ocr_model.clone())
        .ok_or_else(|| {
            "no vision model chosen: pass `model`, or set `ocr_model` in this worker's \
             configuration. `router::models::list` reports which models support vision."
                .to_string()
        })?;

    // Before rendering anything. A model without vision fails on the first page
    // AFTER the render has been paid for, and the error it returns says nothing
    // about why.
    ensure_vision(bus.as_ref(), &model, cfg.ocr_timeout_ms).await?;

    let route = route_for(&bytes, req.source.file_name_hint().as_deref(), &label)?;
    let images = match &route {
        RouteKind::Image(mime) => vec![PageImage {
            page: 1,
            mime: mime.clone(),
            data: BASE64.encode(&bytes),
        }],
        RouteKind::Pdf => render_pdf(bus.as_ref(), &req, &cfg, &label).await?,
        RouteKind::Assets(format) => asset_images(&bytes, *format, &cfg)?,
    };

    if images.is_empty() {
        return Err(format!(
            "{label} gave nothing to transcribe: no page was rendered and it carries no embedded \
             image"
        ));
    }

    let mut pages: Vec<PageText> = Vec::with_capacity(images.len());
    let mut transcribed = 0usize;
    let mut cached = 0usize;
    for image in images {
        let key = cache_key(&image.data, image.page, &model);
        if let Some(hit) = cache_get(bus.as_ref(), &key, &cfg).await {
            cached += 1;
            pages.push(PageText {
                page: image.page,
                chars: hit.chars().count(),
                text: hit,
                cached: true,
            });
            continue;
        }
        let text = transcribe(bus.as_ref(), &model, &image, &cfg).await?;
        transcribed += 1;
        cache_put(bus.as_ref(), &key, &text, &cfg).await;
        pages.push(PageText {
            page: image.page,
            chars: text.chars().count(),
            text,
            cached: false,
        });
    }

    let joined = pages
        .iter()
        .filter(|p| !p.text.trim().is_empty())
        .map(|p| p.text.trim())
        .collect::<Vec<_>>()
        .join("\n\n");
    let max_chars = cfg.effective_max_chars(req.max_chars);

    Ok(Response {
        via: match route {
            RouteKind::Image(_) => "image",
            RouteKind::Pdf => "pdf-render",
            RouteKind::Assets(_) => "document-assets",
        }
        .to_string(),
        body: Body::new(joined, max_chars, cfg.preview_chars),
        pages,
        pages_transcribed: transcribed,
        pages_cached: cached,
        model,
        source: label,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// One page's pixels on the way to the model.
struct PageImage {
    page: u32,
    mime: String,
    data: String,
}

/// Which of the three shapes this document is.
pub fn route_for(bytes: &[u8], file_name: Option<&str>, label: &str) -> Result<RouteKind, String> {
    if let Some(mime) = image_mime(bytes, file_name) {
        return Ok(RouteKind::Image(mime));
    }
    match format::resolve(None, bytes, file_name) {
        Some((Format::Pdf, _)) => Ok(RouteKind::Pdf),
        Some((format, _)) => Ok(RouteKind::Assets(format)),
        None => Err(format!(
            "{label} is neither an image nor a document this worker reads, so there is nothing to \
             transcribe"
        )),
    }
}

/// Where the pixels come from for this document.
#[derive(Debug, PartialEq, Eq)]
pub enum RouteKind {
    /// The file IS the image.
    Image(String),
    /// A PDF, rendered page by page through the browser worker.
    Pdf,
    /// An office document with no readable text; its embedded images are the
    /// content.
    Assets(Format),
}

/// The image formats a vision model reads, recognised from the bytes.
///
/// Signatures rather than the file name: an image pasted into a composer often
/// arrives named `image.png` whatever it actually is, and a wrong `mime` on the
/// wire is a provider error rather than a transcription.
pub fn image_mime(bytes: &[u8], file_name: Option<&str>) -> Option<String> {
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    };
    if let Some(mime) = mime {
        return Some(mime.to_string());
    }
    // A signature-less fallback for a caller that names the file: nothing here
    // depends on it, but a `.jpg` whose header was stripped by a pipeline is a
    // real thing to hit.
    let ext = file_name
        .and_then(|name| std::path::Path::new(name).extension())
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())?;
    match ext.as_str() {
        "png" => Some("image/png".to_string()),
        "jpg" | "jpeg" => Some("image/jpeg".to_string()),
        "gif" => Some("image/gif".to_string()),
        "webp" => Some("image/webp".to_string()),
        _ => None,
    }
}

/// Refuse a model that cannot see, before anything is rendered.
///
/// The catalog is asked for its vision models rather than asked about this one:
/// `router::models::supports` needs the owning provider as well as the id, and
/// a caller naming a model rarely knows which provider serves it. The filtered
/// list answers without that.
///
/// An id the catalog does not carry at all is NOT refused. The router fails
/// open on unknown models for the same reason: a model this worker has never
/// heard of is far more likely to be newer than the catalog than to be blind,
/// and refusing it would make the function unusable on a rig that is ahead.
async fn ensure_vision(bus: &dyn Bus, model: &str, timeout_ms: u64) -> Result<(), String> {
    let seeing = bus
        .trigger(
            "router::models::list",
            json!({ "capability": "vision" }),
            timeout_ms,
        )
        .await
        .map_err(|e| describe_bus_failure("router::models::list", &e))?;

    if catalog_has(&seeing, model) {
        return Ok(());
    }

    // Not among the models that see. Either it cannot, or the catalog does not
    // know it — and those deserve different answers.
    let everything = bus
        .trigger("router::models::list", json!({}), timeout_ms)
        .await
        .map_err(|e| describe_bus_failure("router::models::list", &e))?;

    if catalog_has(&everything, model) {
        Err(format!(
            "{model} cannot read images, so it cannot transcribe anything. Pick a model whose \
             `supports_vision` is true in router::models::list."
        ))
    } else {
        Ok(())
    }
}

/// Whether a `router::models::list` answer carries this model.
///
/// A model id reaches this worker in more than one shape: bare
/// (`claude-haiku-4-5`), or carrying the provider the console composes onto it
/// (`anthropic::claude-haiku-4-5`). Compare on the bare half of both sides.
pub fn catalog_has(answer: &Value, model: &str) -> bool {
    let wanted = bare_model_id(model);
    answer
        .get("models")
        .and_then(Value::as_array)
        .is_some_and(|models| {
            models
                .iter()
                .filter_map(|m| m.get("id").and_then(Value::as_str))
                .any(|id| bare_model_id(id) == wanted)
        })
}

fn bare_model_id(model: &str) -> &str {
    model.rsplit("::").next().unwrap_or(model)
}

/// Render a PDF through the browser worker, one capture per page.
///
/// Chromium is the only component in the fleet that rasterizes a page, and its
/// PDF viewer takes the page number in the fragment. The session is started and
/// stopped here rather than left open: a browser session is a whole Chrome
/// process, and holding one for the length of a transcription costs more than
/// re-navigating.
async fn render_pdf(
    bus: &dyn Bus,
    req: &Request,
    cfg: &WorkerConfig,
    label: &str,
) -> Result<Vec<PageImage>, String> {
    let path = req.source.path.as_deref().ok_or_else(|| {
        "rendering a PDF needs it on disk: pass `path` rather than `bytes_base64`, because the \
         browser opens the file by URL"
            .to_string()
    })?;

    let pages = match &req.pages {
        Some(pages) if pages.is_empty() => {
            return Err("`pages` was empty; omit it to read the whole document".to_string())
        }
        Some(pages) => {
            if pages.contains(&0) {
                return Err("page numbers are 1-indexed; 0 is not a page".to_string());
            }
            pages.clone()
        }
        None => (1..=cfg.max_ocr_pages as u32).collect(),
    };
    let pages: Vec<u32> = pages.into_iter().take(cfg.max_ocr_pages).collect();

    let started = bus
        .trigger("browser::sessions::start", json!({}), cfg.ocr_timeout_ms)
        .await
        .map_err(|e| describe_bus_failure("browser::sessions::start", &e))?;
    let session_id = started
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "browser::sessions::start returned no session_id".to_string())?
        .to_string();

    let rendered = render_pages(bus, &session_id, path, &pages, cfg).await;

    // Always stop the session, including on the failure path: a leaked Chrome
    // process outlives the call that made it and counts against `max_sessions`.
    let _ = bus
        .trigger(
            "browser::sessions::stop",
            json!({ "session_id": session_id }),
            cfg.ocr_timeout_ms,
        )
        .await;

    let rendered = rendered?;
    if rendered.is_empty() {
        return Err(format!("{label}: no page could be rendered"));
    }
    Ok(rendered)
}

async fn render_pages(
    bus: &dyn Bus,
    session_id: &str,
    path: &str,
    pages: &[u32],
    cfg: &WorkerConfig,
) -> Result<Vec<PageImage>, String> {
    let mut out = Vec::new();
    for &page in pages {
        let url = format!("file://{path}#page={page}");
        bus.trigger(
            "browser::navigate",
            json!({ "session_id": session_id, "url": url }),
            cfg.ocr_timeout_ms,
        )
        .await
        .map_err(|e| describe_navigate_failure(&e))?;

        // `navigate` returns on the load event, which for a PDF fires when the
        // viewer has loaded rather than when it has drawn the page. Capturing
        // on that signal photographs an empty viewer, and a model reading it
        // reports a blank image — which is what the first live run produced.
        if cfg.ocr_render_settle_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(cfg.ocr_render_settle_ms)).await;
        }

        let shot = bus
            .trigger(
                "browser::screenshot",
                json!({ "session_id": session_id, "full_page": false }),
                cfg.ocr_timeout_ms,
            )
            .await
            .map_err(|e| describe_bus_failure("browser::screenshot", &e))?;

        // A page past the end of the document renders as the last page again
        // rather than failing, so a run with no explicit `pages` stops at the
        // first repeat instead of transcribing the same page to the ceiling.
        let Some((mime, data)) = image_from_screenshot(&shot) else {
            break;
        };
        if out.last().is_some_and(|last: &PageImage| last.data == data) {
            out.pop();
            break;
        }
        out.push(PageImage { page, mime, data });
    }
    Ok(out)
}

/// The one browser refusal worth translating.
///
/// A local PDF is opened over `file://`, and the browser worker ships with an
/// allowlist of `http` and `https` only. Its own error names the scheme but not
/// the setting, and "scheme `file` is not allowed" sends a reader looking
/// through this worker's configuration, where the answer is not.
fn describe_navigate_failure(err: &str) -> String {
    if err.contains("scheme") && err.contains("file") {
        return "the browser worker refuses `file://` URLs, so a local PDF cannot be rendered. Add \
                `file` to its allowed schemes: Console global Settings, Browser, Behavior, Allowed \
                URL schemes (or `allowed_schemes` in its configuration). It hot-applies."
            .to_string();
    }
    describe_bus_failure("browser::navigate", err)
}

/// Pull the image block out of a `browser::screenshot` response.
pub fn image_from_screenshot(value: &Value) -> Option<(String, String)> {
    let blocks = value.get("content")?.as_array()?;
    for block in blocks {
        let data = block.get("data").and_then(Value::as_str);
        let mime = block.get("mime").and_then(Value::as_str);
        if let (Some(mime), Some(data)) = (mime, data) {
            if !data.is_empty() {
                return Some((mime.to_string(), data.to_string()));
            }
        }
    }
    None
}

/// The embedded images of a document whose text came back empty.
fn asset_images(
    bytes: &[u8],
    format: Format,
    cfg: &WorkerConfig,
) -> Result<Vec<PageImage>, String> {
    let document = anydoc::to_document(bytes, format.to_anydoc())
        .map_err(|e| crate::source::describe_error("reading embedded images", &e))?;

    Ok(document
        .assets
        .iter()
        .filter(|asset| asset.media_type.starts_with("image/"))
        .take(cfg.max_ocr_pages)
        .enumerate()
        .map(|(index, asset)| PageImage {
            page: index as u32 + 1,
            mime: asset.media_type.clone(),
            data: BASE64.encode(&asset.bytes),
        })
        .collect())
}

/// One page, read by the model.
async fn transcribe(
    bus: &dyn Bus,
    model: &str,
    image: &PageImage,
    cfg: &WorkerConfig,
) -> Result<String, String> {
    let answer = bus
        .trigger(
            "router::complete",
            json!({
                "model": model,
                "messages": [{
                    "role": "user",
                    "content": [
                        { "type": "text", "text": TRANSCRIBE_PROMPT },
                        { "type": "image", "mime": image.mime, "data": image.data },
                    ],
                    "timestamp": 0,
                }],
            }),
            cfg.ocr_timeout_ms,
        )
        .await
        .map_err(|e| describe_bus_failure("router::complete", &e))?;

    Ok(text_of(&answer))
}

/// The text blocks of a `router::complete` answer, joined.
pub fn text_of(answer: &Value) -> String {
    let Some(content) = answer
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
    else {
        return String::new();
    };
    content
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("")
        .trim()
        .to_string()
}

/// Cache key for one page: the PIXELS that were read, and the model that read
/// them.
///
/// Keying on the rendered image rather than on the source document is what
/// makes the cache self-correcting. A render that came out blank hashes
/// differently from the same page rendered properly, so fixing the renderer
/// invalidates every bad entry it produced instead of serving them forever —
/// which is exactly what happened the first time this ran against a real PDF.
/// It also means the same page arriving inside two different documents is
/// transcribed once.
///
/// The trade is that a hit no longer skips the render, only the model call.
/// Rendering is a second of local Chromium; the model call is the money.
pub fn cache_key(image_data: &str, page: u32, model: &str) -> String {
    format!("{}/{page}/{model}", content_hash(image_data.as_bytes()))
}

/// FNV-1a over the document bytes.
///
/// Not a cryptographic hash and does not need to be: this keys a cache of the
/// worker's own transcriptions, where a collision costs a wrong page of text
/// and nothing else. Hand-rolled to keep the dependency list at one crate.
fn content_hash(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

async fn cache_get(bus: &dyn Bus, key: &str, cfg: &WorkerConfig) -> Option<String> {
    if !cfg.ocr_cache {
        return None;
    }
    let answer = bus
        .trigger(
            "state::get",
            json!({ "scope": OCR_SCOPE, "key": key }),
            cfg.ocr_timeout_ms,
        )
        .await
        .ok()?;
    // `state::get` answers with the VALUE, so a stored string arrives as one.
    answer
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            answer
                .get("value")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .filter(|text| !text.is_empty())
}

async fn cache_put(bus: &dyn Bus, key: &str, text: &str, cfg: &WorkerConfig) {
    if !cfg.ocr_cache || text.trim().is_empty() {
        return;
    }
    // Best effort: a cache that cannot be written is slower, not broken, and a
    // rig with no `state` worker still transcribes.
    let _ = bus
        .trigger(
            "state::set",
            json!({ "scope": OCR_SCOPE, "key": key, "value": text }),
            cfg.ocr_timeout_ms,
        )
        .await;
}

/// State scope for the transcription cache. Only page TEXT lives here; the
/// rendered images never leave the call that made them.
const OCR_SCOPE: &str = "document-ocr";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::test_bus::RecordedBus;
    use base64::engine::general_purpose::STANDARD as B64;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x01";

    fn cfg() -> Arc<WorkerConfig> {
        Arc::new(WorkerConfig {
            ocr_model: Some("test-vision".into()),
            ..WorkerConfig::default()
        })
    }

    fn request(bytes: &[u8], name: &str) -> Request {
        Request {
            source: DocumentSource {
                bytes_base64: Some(B64.encode(bytes)),
                file_name: Some(name.to_string()),
                ..DocumentSource::default()
            },
            pages: None,
            model: None,
            max_chars: None,
        }
    }

    fn vision_ok(bus: RecordedBus) -> RecordedBus {
        bus.on(
            "router::models::list",
            json!({ "models": [{ "id": "test-vision", "supports_vision": true }] }),
        )
    }

    fn transcription(text: &str) -> Value {
        json!({
            "message": { "role": "assistant", "content": [{ "type": "text", "text": text }] },
            "model": "test-vision",
            "provider": "test",
        })
    }

    #[tokio::test]
    async fn an_image_goes_straight_to_the_model_with_no_browser() {
        let bus = Arc::new(
            vision_ok(RecordedBus::new())
                .on("router::complete", transcription("INVOICE 42"))
                .on("state::set", json!({ "ok": true })),
        );
        let response = handle(request(PNG, "receipt.png"), cfg(), bus.clone())
            .await
            .expect("transcribes");

        assert_eq!(response.via, "image");
        assert_eq!(response.body.text, "INVOICE 42");
        assert_eq!(response.pages_transcribed, 1);
        assert!(
            !bus.called().iter().any(|id| id.starts_with("browser::")),
            "an image needs no rendering: {:?}",
            bus.called()
        );
    }

    /// The image travels as an image block, not as prose about one.
    #[tokio::test]
    async fn the_page_reaches_the_model_as_pixels() {
        let bus = Arc::new(
            vision_ok(RecordedBus::new())
                .on("router::complete", transcription("text"))
                .on("state::set", json!({ "ok": true })),
        );
        handle(request(PNG, "page.png"), cfg(), bus.clone())
            .await
            .expect("transcribes");

        let payload = &bus.payloads("router::complete")[0];
        let content = payload["messages"][0]["content"]
            .as_array()
            .expect("content");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["mime"], "image/png");
        assert!(content[1]["data"].as_str().is_some_and(|d| !d.is_empty()));
    }

    /// Checking the model comes FIRST. A model that cannot see fails on the
    /// first page otherwise, after the render has already been paid for.
    #[tokio::test]
    async fn a_model_without_vision_is_refused_before_anything_renders() {
        // Present in the catalog, absent from the models that see.
        let bus = Arc::new(
            RecordedBus::new()
                .on("router::models::list", json!({ "models": [] }))
                .on(
                    "router::models::list",
                    json!({ "models": [{ "id": "test-vision" }] }),
                ),
        );
        let err = handle(request(PNG, "page.png"), cfg(), bus.clone())
            .await
            .expect_err("refused");

        assert!(err.contains("cannot read images"), "{err}");
        assert!(
            !bus.called().iter().any(|id| id == "router::complete"),
            "nothing may be read: {:?}",
            bus.called()
        );
    }

    /// A model the catalog has never heard of is likelier to be newer than the
    /// catalog than to be blind, so it is read rather than refused — the same
    /// stance the router itself takes on unknown models.
    #[tokio::test]
    async fn a_model_the_catalog_does_not_know_still_transcribes() {
        let bus = Arc::new(
            RecordedBus::new()
                .on("router::models::list", json!({ "models": [] }))
                .on("router::complete", transcription("readable"))
                .on("state::set", json!({ "ok": true })),
        );
        let response = handle(request(PNG, "page.png"), cfg(), bus)
            .await
            .expect("transcribes");
        assert_eq!(response.body.text, "readable");
    }

    #[tokio::test]
    async fn a_missing_model_says_where_to_find_one() {
        let bare = Arc::new(WorkerConfig::default());
        let bus = Arc::new(RecordedBus::new());
        let err = handle(request(PNG, "page.png"), bare, bus)
            .await
            .expect_err("no model");
        assert!(err.contains("router::models::list"), "{err}");
    }

    /// The cache is keyed by content, so the same page never gets paid for
    /// twice, and a hit must not reach the model at all.
    #[tokio::test]
    async fn a_cached_page_costs_nothing() {
        let bus =
            Arc::new(vision_ok(RecordedBus::new()).on("state::get", json!("cached transcription")));
        let response = handle(request(PNG, "page.png"), cfg(), bus.clone())
            .await
            .expect("serves from cache");

        assert_eq!(response.pages_cached, 1);
        assert_eq!(response.pages_transcribed, 0);
        assert_eq!(response.body.text, "cached transcription");
        assert!(
            !bus.called().iter().any(|id| id == "router::complete"),
            "a cache hit must not call the model: {:?}",
            bus.called()
        );
    }

    /// The rendered pixels are never stored: only the text is.
    #[tokio::test]
    async fn only_the_text_is_cached() {
        let bus = Arc::new(
            vision_ok(RecordedBus::new())
                .on("router::complete", transcription("page one"))
                .on("state::set", json!({ "ok": true })),
        );
        handle(request(PNG, "page.png"), cfg(), bus.clone())
            .await
            .expect("transcribes");

        let stored = &bus.payloads("state::set")[0];
        assert_eq!(stored["value"], "page one");
        let serialized = stored.to_string();
        assert!(
            !serialized.contains(&B64.encode(PNG)),
            "the page image must not reach the cache"
        );
    }

    #[tokio::test]
    async fn a_pdf_without_a_path_says_why() {
        let bus = Arc::new(vision_ok(RecordedBus::new()));
        let err = handle(request(b"%PDF-1.7\n", "scan.pdf"), cfg(), bus)
            .await
            .expect_err("needs a path");
        assert!(err.contains("`path`"), "{err}");
    }

    #[tokio::test]
    async fn a_missing_browser_worker_says_what_to_install() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("scan.pdf");
        std::fs::write(&path, b"%PDF-1.7\n").expect("write");

        let bus = Arc::new(vision_ok(RecordedBus::new()));
        let req = Request {
            source: DocumentSource {
                path: Some(path.to_string_lossy().to_string()),
                ..DocumentSource::default()
            },
            pages: Some(vec![1]),
            model: None,
            max_chars: None,
        };
        let err = handle(req, cfg(), bus).await.expect_err("no browser");
        assert!(
            err.contains("iii trigger compose::add worker=browser"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_pdf_page_is_rendered_then_read() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("scan.pdf");
        std::fs::write(&path, b"%PDF-1.7\n").expect("write");

        let bus = Arc::new(
            vision_ok(RecordedBus::new())
                .on("browser::sessions::start", json!({ "session_id": "s-1" }))
                .on("browser::navigate", json!({ "ok": true }))
                .on(
                    "browser::screenshot",
                    json!({ "content": [{ "type": "image", "mime": "image/jpeg", "data": "AAAA" }] }),
                )
                .on("router::complete", transcription("PAGE ONE"))
                .on("state::set", json!({ "ok": true }))
                .on("browser::sessions::stop", json!({ "ok": true })),
        );

        let req = Request {
            source: DocumentSource {
                path: Some(path.to_string_lossy().to_string()),
                ..DocumentSource::default()
            },
            pages: Some(vec![1]),
            model: None,
            max_chars: None,
        };
        let response = handle(req, cfg(), bus.clone()).await.expect("transcribes");

        assert_eq!(response.via, "pdf-render");
        assert_eq!(response.body.text, "PAGE ONE");
        let order = bus.called();
        let navigate = order.iter().position(|id| id == "browser::navigate");
        let complete = order.iter().position(|id| id == "router::complete");
        assert!(navigate < complete, "render before read: {order:?}");
        assert!(
            order.contains(&"browser::sessions::stop".to_string()),
            "the session must be stopped: {order:?}"
        );
        // The page number rides in the URL fragment, which is how Chrome's PDF
        // viewer is told which page to show.
        let url = bus.payloads("browser::navigate")[0]["url"]
            .as_str()
            .expect("url")
            .to_string();
        assert!(url.starts_with("file://"), "{url}");
        assert!(url.ends_with("#page=1"), "{url}");
    }

    /// A Chrome session is a whole browser process; leaking one on the failure
    /// path counts against `max_sessions` until the worker restarts.
    #[tokio::test]
    async fn the_browser_session_is_stopped_even_when_a_page_fails() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("scan.pdf");
        std::fs::write(&path, b"%PDF-1.7\n").expect("write");

        let bus = Arc::new(
            vision_ok(RecordedBus::new())
                .on("browser::sessions::start", json!({ "session_id": "s-1" }))
                .failing("browser::navigate", "target closed")
                .on("browser::sessions::stop", json!({ "ok": true })),
        );
        let req = Request {
            source: DocumentSource {
                path: Some(path.to_string_lossy().to_string()),
                ..DocumentSource::default()
            },
            pages: Some(vec![1]),
            model: None,
            max_chars: None,
        };
        let err = handle(req, cfg(), bus.clone())
            .await
            .expect_err("render fails");

        assert!(err.contains("target closed"), "{err}");
        assert!(
            bus.called()
                .contains(&"browser::sessions::stop".to_string()),
            "{:?}",
            bus.called()
        );
    }

    #[test]
    fn a_blocked_file_url_names_the_setting_to_change() {
        // The browser's own message names the scheme but not the setting, and
        // the setting lives on a different worker than the one being read.
        let described = describe_navigate_failure("scheme `file` is not allowed");
        assert!(described.contains("allowed_schemes"), "{described}");
        assert!(describe_navigate_failure("target crashed").contains("target crashed"));
    }

    #[test]
    fn images_are_recognised_by_signature_first() {
        assert_eq!(image_mime(PNG, None).as_deref(), Some("image/png"));
        assert_eq!(
            image_mime(&[0xFF, 0xD8, 0xFF, 0x00], None).as_deref(),
            Some("image/jpeg")
        );
        // A stripped header falls back to the name.
        assert_eq!(
            image_mime(b"\x00\x00", Some("photo.JPG")).as_deref(),
            Some("image/jpeg")
        );
        assert_eq!(image_mime(b"%PDF-1.7", Some("scan.pdf")), None);
    }

    #[test]
    fn routing_picks_the_cheapest_path_that_works() {
        assert_eq!(
            route_for(PNG, Some("page.png"), "page.png").expect("image"),
            RouteKind::Image("image/png".to_string())
        );
        assert_eq!(
            route_for(b"%PDF-1.7\n", Some("scan.pdf"), "scan.pdf").expect("pdf"),
            RouteKind::Pdf
        );
        let err = route_for(b"\x00\x01\x02", Some("mystery.bin"), "mystery.bin")
            .expect_err("nothing to do");
        assert!(err.contains("nothing to transcribe"), "{err}");
    }

    /// Keying on the pixels is what makes a fixed renderer invalidate the bad
    /// entries it produced, rather than serving them forever.
    #[test]
    fn the_cache_key_follows_the_pixels() {
        let good = cache_key("rendered-page-bytes", 1, "m");
        assert_eq!(good, cache_key("rendered-page-bytes", 1, "m"));
        assert_ne!(good, cache_key("blank-page-bytes", 1, "m"));
        assert_ne!(good, cache_key("rendered-page-bytes", 2, "m"));
        // A different model is a different answer, not a fresher one.
        assert_ne!(good, cache_key("rendered-page-bytes", 1, "better-model"));
    }

    #[test]
    fn a_transcription_is_read_out_of_the_router_envelope() {
        assert_eq!(text_of(&transcription("hello")), "hello");
        assert_eq!(text_of(&json!({})), "");
    }
}
