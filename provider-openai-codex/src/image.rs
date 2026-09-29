//! `provider::openai-codex::image::generate` — text-to-image on the ChatGPT
//! subscription backend.
//!
//! The Codex backend has no `/images/generations` endpoint, so the picture is
//! produced the way the Codex CLI does it: a Responses call on a Codex chat
//! model (`host_model`, default gpt-5.5) carries the hosted
//! `image_generation` tool pinned to the requested image model, `tool_choice`
//! forces exactly that tool, and the `image_generation_call` output item's
//! `result` is the base64 picture. The backend is stream-only, so the SSE
//! stream is collected here rather than relayed.
//!
//! The picture is persisted at full resolution under
//! `data/provider-openai-codex/images/` (`details.path`) for other processes
//! to pick up. The result carries no image bytes by default: base64 in a
//! function result only inflates the conversation context (the model sees
//! it once, then context-manager drops it). The chat preview is drawn by
//! this worker's injected console renderer (ui/), which fetches a downscaled
//! JPEG through `provider::openai-codex::image::read`; an agent that must
//! look at the picture itself calls that same function, or asks for
//! `inline: "preview"` (≤512px JPEG, fits the harness's 256 KiB result cap)
//! or `inline: "full"`. Agent-callable on purpose: images have no router
//! front door.

use crate::config::{build_backend_config, CodexBackendConfig, CodexConfig};
use crate::errors::{classify, classify_bus_error};
use crate::request::build_headers;
use crate::session::AuthManager;
use crate::state;
use base64::Engine as _;
use futures::StreamExt;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::provider_scaffold::cache::ScaffoldCache;
use llm_router::provider_scaffold::sse_transport::{append_utf8_chunk, normalize_crlf};
use llm_router::types::events::ErrorKind;
use llm_router::types::router::{CredentialSource, ProviderResolveResponse};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The image models this surface accepts (pinned on the `image_generation` tool).
pub const IMAGE_MODELS: [&str; 2] = ["gpt-image-2.5-sunburst", "gpt-image-2.5-flare"];
/// Codex chat model that hosts the tool call when the caller names none.
pub const DEFAULT_HOST_MODEL: &str = "gpt-5.5";
/// Project-relative folder where generated images are persisted; resolved
/// against `III_COMPOSE_DIR` (or the process cwd outside Compose).
pub const IMAGE_DATA_DIR: &str = "data/provider-openai-codex/images";
/// Inline preview budget: raw JPEG bytes. Base64 (×1.33) counted twice by the
/// harness stays under its 256 KiB cap with room for the text and details.
pub const PREVIEW_MAX_BYTES: usize = 80 * 1024;
/// Longest edge of the inline preview, in pixels.
pub const PREVIEW_MAX_EDGE: u32 = 512;
/// The host model reasons briefly, then the image renders: slow but bounded.
const IMAGE_TIMEOUT_SECS: u64 = 240;
const MAX_PROMPT_CHARS: usize = 32_000;
const MAX_STEM_CHARS: usize = 48;
const DEFAULT_STEM: &str = "image";
const DEFAULT_FORMAT: &str = "png";
/// Keeps the host model from answering in prose instead of calling the tool.
const RELAY_INSTRUCTIONS: &str = "You are an image generation relay. Call the image_generation tool \
     exactly once, passing the user's message verbatim as the prompt. Do not reply with text, do not \
     ask questions, do not describe the image.";

const SIZES: [&str; 4] = ["1024x1024", "1536x1024", "1024x1536", "auto"];
const QUALITIES: [&str; 4] = ["low", "medium", "high", "auto"];
const FORMATS: [&str; 3] = ["png", "jpeg", "webp"];
const BACKGROUNDS: [&str; 3] = ["transparent", "opaque", "auto"];
const INLINE_MODES: [&str; 3] = ["preview", "full", "none"];

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImageGenerateRequest {
    /// Image model: `gpt-image-2.5-sunburst` or `gpt-image-2.5-flare`.
    pub model: String,
    /// What to draw (up to 32k characters).
    pub prompt: String,
    /// `1024x1024`, `1536x1024` (landscape), `1024x1536` (portrait), or
    /// `auto`; the model default when omitted.
    #[serde(default)]
    pub size: Option<String>,
    /// `low`, `medium`, `high`, or `auto`; the model default when omitted.
    #[serde(default)]
    pub quality: Option<String>,
    /// `png` (default), `jpeg`, or `webp`: the format of the file saved under
    /// data/ (and of the inline picture when `inline` is `full`).
    #[serde(default)]
    pub output_format: Option<String>,
    /// `transparent`, `opaque`, or `auto`; transparency needs png or webp.
    #[serde(default)]
    pub background: Option<String>,
    /// File stem for the persisted image (letters, digits, `.`, `_`, `-`;
    /// anything else becomes `-`). A timestamp and a random suffix are always
    /// appended so files never collide. Defaults to `image`.
    #[serde(default)]
    pub file_name: Option<String>,
    /// Codex chat model that hosts the `image_generation` tool call
    /// (default `gpt-5.5`; the router's `codex/` prefix is accepted).
    #[serde(default)]
    pub host_model: Option<String>,
    /// Whether the result embeds image bytes: `none` (default — the file is
    /// saved, `details.path` names it, and the console fetches its own
    /// preview), `preview` (a JPEG at most 512px / ~80 KB, for an agent that
    /// must look at the picture; fits the harness result cap), or `full`
    /// (the original bytes — over a megabyte for a 1024px PNG).
    #[serde(default)]
    pub inline: Option<String>,
}

/// One block of a viewable response: an image block plus a text line — the
/// shape the harness renders inline (same as `browser::screenshot`).
#[derive(Debug, Serialize, JsonSchema)]
pub struct ContentBlock {
    pub r#type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Token usage of the hosting Responses call (absent when the backend omits it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ImageUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
}

/// What the inline image block actually holds when it is a preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct PreviewDetails {
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ImageDetails {
    /// Image model that produced the picture.
    pub model: String,
    /// Codex chat model that hosted the tool call.
    pub host_model: String,
    /// Absolute path of the full-resolution file under `data/provider-openai-codex/images/`.
    pub path: String,
    /// Mime of the saved file: `image/png`, `image/jpeg`, or `image/webp`.
    pub mime: String,
    /// Saved file size in bytes.
    pub bytes: u64,
    /// Requested size (`auto` when left to the model).
    pub size: String,
    /// Requested quality (`auto` when left to the model).
    pub quality: String,
    /// Pixel width of the saved file, when decodable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Pixel height of the saved file, when decodable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// `preview`, `full`, or `none`: what the inline image block carries.
    pub inline: String,
    /// Present when `inline` is `preview` and the preview could be built.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<PreviewDetails>,
    /// The prompt as the model actually rendered it, when upstream reports one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revised_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ImageUsage>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ImageGenerateResponse {
    /// `[image block, text line]` (`[text line]` with `inline: none`) —
    /// render `content` and the picture shows.
    pub content: Vec<ContentBlock>,
    pub details: ImageDetails,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineMode {
    Preview,
    Full,
    None,
}

impl InlineMode {
    fn as_str(self) -> &'static str {
        match self {
            InlineMode::Preview => "preview",
            InlineMode::Full => "full",
            InlineMode::None => "none",
        }
    }
}

/// The request after validation: every enum normalized, defaults applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageParams {
    pub model: String,
    pub host_model: String,
    pub prompt: String,
    pub size: Option<String>,
    pub quality: Option<String>,
    pub output_format: String,
    pub background: Option<String>,
    pub file_stem: String,
    pub inline: InlineMode,
}

/// One decoded image from upstream plus the metadata worth surfacing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedImage {
    pub bytes: Vec<u8>,
    pub revised_prompt: Option<String>,
    pub usage: Option<ImageUsage>,
}

/// A downscaled JPEG of the generated image that fits the inline budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

fn one_of(field: &str, value: Option<String>, allowed: &[&str]) -> Result<Option<String>, String> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let v = raw.trim().to_ascii_lowercase();
    if v.is_empty() {
        return Ok(None);
    }
    if allowed.contains(&v.as_str()) {
        Ok(Some(v))
    } else {
        Err(format!(
            "{field} must be one of {}; got {raw:?}",
            allowed.join(", ")
        ))
    }
}

/// Keep a caller-supplied file stem filesystem-safe and short.
pub fn sanitize_stem(name: Option<&str>) -> String {
    let cleaned: String = name
        .unwrap_or("")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches(|c| matches!(c, '.' | '-' | '_'));
    if trimmed.is_empty() {
        return DEFAULT_STEM.to_string();
    }
    trimmed.chars().take(MAX_STEM_CHARS).collect()
}

/// Validate and normalize a request; the error names the offending field
/// and the accepted values.
pub fn validate(req: ImageGenerateRequest) -> Result<ImageParams, String> {
    let model = req.model.trim().to_string();
    if !IMAGE_MODELS.contains(&model.as_str()) {
        return Err(format!(
            "model must be one of {}; got {:?}",
            IMAGE_MODELS.join(", "),
            req.model
        ));
    }
    let prompt = req.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err("prompt must not be empty".into());
    }
    if prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(format!("prompt exceeds {MAX_PROMPT_CHARS} characters"));
    }
    let host_model = req
        .host_model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(|m| m.strip_prefix("codex/").unwrap_or(m).to_string())
        .unwrap_or_else(|| DEFAULT_HOST_MODEL.to_string());
    if IMAGE_MODELS.contains(&host_model.as_str()) {
        return Err(format!(
            "host_model must be a Codex chat model (e.g. {DEFAULT_HOST_MODEL}), not an image model"
        ));
    }
    let size = one_of("size", req.size, &SIZES)?;
    let quality = one_of("quality", req.quality, &QUALITIES)?;
    let output_format = one_of("output_format", req.output_format, &FORMATS)?
        .unwrap_or_else(|| DEFAULT_FORMAT.into());
    let background = one_of("background", req.background, &BACKGROUNDS)?;
    if background.as_deref() == Some("transparent") && output_format == "jpeg" {
        return Err("background=transparent requires output_format png or webp".into());
    }
    let inline = match one_of("inline", req.inline, &INLINE_MODES)?.as_deref() {
        Some("full") => InlineMode::Full,
        Some("preview") => InlineMode::Preview,
        _ => InlineMode::None,
    };
    Ok(ImageParams {
        model,
        host_model,
        prompt,
        size,
        quality,
        output_format,
        background,
        file_stem: sanitize_stem(req.file_name.as_deref()),
        inline,
    })
}

/// Responses body: the host model, a relay system item, the prompt as the
/// user item, and the `image_generation` tool pinned to the image model and
/// forced through `tool_choice`. Stream-only and stateless like the chat
/// path; no `max_output_tokens` (the backend rejects it).
pub fn build_body(p: &ImageParams) -> Value {
    let mut tool = json!({
        "type": "image_generation",
        "model": p.model,
        "output_format": p.output_format,
    });
    if let Some(size) = &p.size {
        tool["size"] = json!(size);
    }
    if let Some(quality) = &p.quality {
        tool["quality"] = json!(quality);
    }
    if let Some(background) = &p.background {
        tool["background"] = json!(background);
    }
    json!({
        "model": p.host_model,
        "input": [
            {
                "role": "system",
                "content": [{ "type": "input_text", "text": RELAY_INSTRUCTIONS }],
            },
            {
                "role": "user",
                "content": [{ "type": "input_text", "text": p.prompt }],
            },
        ],
        "stream": true,
        "store": false,
        "tools": [tool],
        "tool_choice": { "type": "image_generation" },
    })
}

pub fn mime_for(output_format: &str) -> &'static str {
    match output_format {
        "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "image/png",
    }
}

fn parse_usage(v: Option<&Value>) -> Option<ImageUsage> {
    let u = v?.as_object()?;
    let get = |k: &str| u.get(k).and_then(Value::as_u64);
    let usage = ImageUsage {
        input_tokens: get("input_tokens"),
        output_tokens: get("output_tokens"),
        total_tokens: get("total_tokens"),
    };
    if usage.input_tokens.is_none() && usage.output_tokens.is_none() && usage.total_tokens.is_none()
    {
        None
    } else {
        Some(usage)
    }
}

/// Folds the Responses SSE stream into the one thing we want: the
/// `image_generation_call` result. Text the host model emits instead of
/// calling the tool is kept for the error message.
#[derive(Debug, Default)]
pub struct ImageStreamCollector {
    pub result_b64: Option<String>,
    pub revised_prompt: Option<String>,
    pub usage: Option<ImageUsage>,
    pub text: String,
    pub error: Option<String>,
    pub terminal: bool,
}

impl ImageStreamCollector {
    fn take_image_item(&mut self, item: &Value) {
        if item.get("type").and_then(Value::as_str) != Some("image_generation_call") {
            return;
        }
        if let Some(result) = item
            .get("result")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            self.result_b64 = Some(result.to_string());
        }
        if let Some(revised) = item.get("revised_prompt").and_then(Value::as_str) {
            self.revised_prompt = Some(revised.to_string());
        }
    }

    /// One SSE block (`event:`/`data:` lines up to the blank line).
    pub fn feed_block(&mut self, block: &str) {
        let Some(data) = block
            .lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .map(str::trim)
            .next_back()
        else {
            return;
        };
        if data == "[DONE]" {
            self.terminal = true;
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            return;
        };
        self.feed_event(&v);
    }

    pub fn feed_event(&mut self, v: &Value) {
        match v.get("type").and_then(Value::as_str).unwrap_or("") {
            "response.output_item.done" => {
                if let Some(item) = v.get("item") {
                    self.take_image_item(item);
                }
            }
            "response.output_text.delta" => {
                if let Some(delta) = v.get("delta").and_then(Value::as_str) {
                    self.text.push_str(delta);
                }
            }
            "response.completed" => {
                if let Some(items) = v.pointer("/response/output").and_then(Value::as_array) {
                    for item in items {
                        self.take_image_item(item);
                    }
                }
                self.usage = parse_usage(v.pointer("/response/usage"));
                self.terminal = true;
            }
            "response.incomplete" => {
                let reason = v
                    .pointer("/response/incomplete_details/reason")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                self.error = Some(format!("response incomplete: {reason}"));
                self.terminal = true;
            }
            "response.failed" | "error" => {
                let message = v
                    .pointer("/response/error/message")
                    .or_else(|| v.pointer("/error/message"))
                    .or_else(|| v.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("upstream reported failure");
                self.error = Some(message.to_string());
                self.terminal = true;
            }
            _ => {}
        }
    }

    /// Decode what was collected into an image, or explain why there is none.
    pub fn finish(self) -> Result<GeneratedImage, String> {
        if let Some(err) = self.error {
            return Err(err);
        }
        let Some(b64) = self.result_b64 else {
            let excerpt: String = self.text.trim().chars().take(300).collect();
            return Err(if excerpt.is_empty() {
                "stream ended without an image_generation_call result".into()
            } else {
                format!("host model answered in text instead of generating an image: {excerpt}")
            });
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .map_err(|e| format!("image_generation_call result is not valid base64: {e}"))?;
        if bytes.is_empty() {
            return Err("decoded image is empty".into());
        }
        Ok(GeneratedImage {
            bytes,
            revised_prompt: self.revised_prompt,
            usage: self.usage,
        })
    }
}

/// Pixel dimensions of the saved image (png/jpeg/webp header read).
pub fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    image::load_from_memory(bytes).ok().map(|img| {
        use image::GenericImageView as _;
        img.dimensions()
    })
}

/// Transparent pixels composited over white: a JPEG has no alpha, and black
/// (what a plain RGB conversion gives) makes a transparent-background render
/// unreadable in the chat.
fn flatten_over_white(img: &image::DynamicImage) -> image::RgbImage {
    if !img.color().has_alpha() {
        return img.to_rgb8();
    }
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut out = image::RgbImage::new(w, h);
    for (x, y, px) in rgba.enumerate_pixels() {
        let a = px[3] as u32;
        let blend = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
        out.put_pixel(x, y, image::Rgb([blend(px[0]), blend(px[1]), blend(px[2])]));
    }
    out
}

fn encode_jpeg(rgb: &image::RgbImage, quality: u8) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality)
        .encode_image(rgb)
        .map_err(|e| format!("jpeg encode: {e}"))?;
    Ok(buf)
}

/// Downscale to at most `PREVIEW_MAX_EDGE` and encode as JPEG, stepping
/// quality then size down until the result fits `PREVIEW_MAX_BYTES`. Bounded
/// (at most 12 encodes of a ≤512px image); the smallest attempt is returned
/// when nothing fits.
pub fn make_preview(full: &[u8]) -> Result<Preview, String> {
    let img = image::load_from_memory(full).map_err(|e| format!("decode: {e}"))?;
    let mut last: Option<Preview> = None;
    let longest = img.width().max(img.height());
    for edge in [
        PREVIEW_MAX_EDGE,
        PREVIEW_MAX_EDGE * 3 / 4,
        PREVIEW_MAX_EDGE / 2,
    ] {
        // `thumbnail` fits the bounds in both directions (it upscales too);
        // only call it when the image is actually larger than the edge.
        let downscaled = longest > edge;
        let rgb = if downscaled {
            flatten_over_white(&img.thumbnail(edge, edge))
        } else {
            flatten_over_white(&img)
        };
        for quality in [80u8, 65, 50, 35] {
            let bytes = encode_jpeg(&rgb, quality)?;
            let preview = Preview {
                bytes,
                width: rgb.width(),
                height: rgb.height(),
            };
            if preview.bytes.len() <= PREVIEW_MAX_BYTES {
                return Ok(preview);
            }
            last = Some(preview);
        }
        if !downscaled {
            break; // smaller edges would re-encode the same pixels
        }
    }
    last.ok_or_else(|| "no preview attempt produced output".to_string())
}

/// Unique file name: `<stem>-<unix_ms>-<8 hex>.<ext>`; sorts chronologically.
pub fn file_name(stem: &str, ext: &str) -> String {
    let ms = crate::now_ms();
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    format!("{stem}-{ms}-{}.{ext}", &suffix[..8])
}

/// Write the image under `IMAGE_DATA_DIR` and return its absolute path.
pub async fn persist(bytes: &[u8], stem: &str, ext: &str) -> Result<PathBuf, String> {
    persist_in(
        &iii_worker_paths::project_path(IMAGE_DATA_DIR),
        bytes,
        stem,
        ext,
    )
    .await
}

pub async fn persist_in(
    dir: &Path,
    bytes: &[u8],
    stem: &str,
    ext: &str,
) -> Result<PathBuf, String> {
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| format!("create {}: {e}", dir.display()))?;
    let path = dir.join(file_name(stem, ext));
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    // Absolute so the path is meaningful to whoever the agent pipes it to.
    Ok(tokio::fs::canonicalize(&path).await.unwrap_or(path))
}

fn image_block(mime: &str, bytes: &[u8]) -> ContentBlock {
    ContentBlock {
        r#type: "image".into(),
        mime: Some(mime.into()),
        data: Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
        text: None,
    }
}

fn text_block(text: String) -> ContentBlock {
    ContentBlock {
        r#type: "text".into(),
        mime: None,
        data: None,
        text: Some(text),
    }
}

/// The viewable envelope: the inline image block (per `inline`), then a text
/// line naming the saved file. A preview that cannot be built degrades to the
/// text line with the reason — the file on disk is the deliverable.
pub fn build_response(p: &ImageParams, img: GeneratedImage, path: &Path) -> ImageGenerateResponse {
    let mime = mime_for(&p.output_format);
    let (width, height) = match dimensions(&img.bytes) {
        Some((w, h)) => (Some(w), Some(h)),
        None => (None, None),
    };
    let bytes = img.bytes.len() as u64;
    let size = p.size.clone().unwrap_or_else(|| "auto".into());
    let quality = p.quality.clone().unwrap_or_else(|| "auto".into());
    let path_str = path.display().to_string();
    let dims = match (width, height) {
        (Some(w), Some(h)) => format!("{w}x{h}"),
        _ => size.clone(),
    };
    let mut line = format!(
        "Image generated by {} via Codex ({dims}, {bytes} bytes, {mime}) saved to {path_str}",
        p.model
    );

    let mut content = Vec::with_capacity(2);
    let mut preview_details = None;
    match p.inline {
        InlineMode::Full => content.push(image_block(mime, &img.bytes)),
        InlineMode::None => line.push_str(&format!(
            "; view with {} {{ path }}",
            crate::surface::IMAGE_READ_ID
        )),
        InlineMode::Preview => match make_preview(&img.bytes) {
            Ok(preview) => {
                line.push_str(&format!(
                    "; inline preview {}x{} jpeg",
                    preview.width, preview.height
                ));
                preview_details = Some(PreviewDetails {
                    mime: "image/jpeg".into(),
                    width: preview.width,
                    height: preview.height,
                    bytes: preview.bytes.len() as u64,
                });
                content.push(image_block("image/jpeg", &preview.bytes));
            }
            Err(reason) => line.push_str(&format!("; inline preview unavailable ({reason})")),
        },
    }
    content.push(text_block(line));

    ImageGenerateResponse {
        content,
        details: ImageDetails {
            model: p.model.clone(),
            host_model: p.host_model.clone(),
            path: path_str,
            mime: mime.into(),
            bytes,
            size,
            quality,
            width,
            height,
            inline: p.inline.as_str().into(),
            preview: preview_details,
            revised_prompt: img.revised_prompt,
            usage: img.usage,
        },
    }
}

fn default_resolve() -> ProviderResolveResponse {
    ProviderResolveResponse {
        configured: false,
        source: CredentialSource::None,
        credential: None,
        api_url: None,
        max_tokens: None,
    }
}

fn headers_for(backend: &CodexBackendConfig, host_model: &str) -> Vec<(&'static str, String)> {
    build_headers(&CodexConfig {
        access_token: backend.access_token.clone(),
        account_id: backend.account_id.clone(),
        model: host_model.to_string(),
        max_tokens: 0,
        api_url: backend.api_url.clone(),
    })
}

async fn send(
    http: &reqwest::Client,
    backend: &CodexBackendConfig,
    host_model: &str,
    body: &Value,
) -> Result<reqwest::Response, Error> {
    let mut req = http
        .post(&backend.api_url)
        .timeout(std::time::Duration::from_secs(IMAGE_TIMEOUT_SECS));
    for (name, value) in headers_for(backend, host_model) {
        req = req.header(name, value);
    }
    req.json(body)
        .send()
        .await
        .map_err(|e| Error::Handler(format!("provider/upstream: {e}")))
}

/// Read the SSE body to its end (or the first terminal event).
async fn collect_stream(resp: reqwest::Response) -> Result<GeneratedImage, String> {
    let mut collector = ImageStreamCollector::default();
    let mut stream = resp.bytes_stream();
    let mut text = String::new();
    let mut byte_buf = Vec::new();
    'read: while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("stream read failed: {e}"))?;
        append_utf8_chunk(&mut byte_buf, &mut text, &chunk);
        normalize_crlf(&mut text);
        while let Some(idx) = text.find("\n\n") {
            let block: String = text.drain(..idx + 2).collect();
            collector.feed_block(&block);
            if collector.terminal {
                break 'read;
            }
        }
    }
    if !collector.terminal && !text.trim().is_empty() {
        collector.feed_block(&text);
    }
    collector.finish()
}

pub async fn handle(
    iii: &IIIClient,
    http: &reqwest::Client,
    cache: &ScaffoldCache,
    auth: &Arc<AuthManager>,
    req: ImageGenerateRequest,
) -> Result<ImageGenerateResponse, Error> {
    let params = validate(req).map_err(|m| Error::Handler(format!("invalid_input: {m}")))?;

    // Router resolve only contributes api_url/max_tokens here; the credential
    // is the provider session. A missing router degrades to defaults.
    let token = cache.load_token(iii, state::STATE_SCOPE).await;
    let resolved = match cache
        .resolve(iii, crate::PROVIDER_ID, token.as_deref(), None)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            if classify_bus_error(&e) == ErrorKind::AuthExpired {
                cache.invalidate();
            }
            default_resolve()
        }
    };
    let credential = auth.resolve(None).await.map_err(|e| {
        Error::Handler(format!(
            "provider/auth [{:?}]: {}",
            e.error_kind(),
            e.message
        ))
    })?;
    let backend = build_backend_config(&resolved, credential.as_ref().map(|c| &c.value))
        .map_err(|e| Error::Handler(format!("provider/not_configured: {e}")))?;

    let body = build_body(&params);
    let mut resp = send(http, &backend, &params.host_model, &body).await?;

    // One rotation retry before any body is read, mirroring the chat path:
    // a 401 on a token the session can renew is not the caller's problem.
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        if let Ok(Some(fresh)) = auth.resolve(Some(&backend.access_token)).await {
            if let Some(token) = fresh.value.get("access_token").and_then(Value::as_str) {
                let renewed = CodexBackendConfig {
                    access_token: token.to_string(),
                    account_id: fresh
                        .value
                        .get("account_id")
                        .and_then(Value::as_str)
                        .unwrap_or(&backend.account_id)
                        .to_string(),
                    api_url: backend.api_url.clone(),
                };
                resp = send(http, &renewed, &params.host_model, &body).await?;
            }
        }
    }

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        let kind = classify(Some(status.as_u16()), &text);
        let excerpt: String = text.chars().take(400).collect();
        return Err(Error::Handler(format!(
            "provider/upstream_status: {status} [{kind:?}]: {excerpt}"
        )));
    }

    let image = collect_stream(resp)
        .await
        .map_err(|m| Error::Handler(format!("provider/bad_response: {m}")))?;
    let path = persist(&image.bytes, &params.file_stem, &params.output_format)
        .await
        .map_err(|m| Error::Handler(format!("provider/persist: {m}")))?;
    Ok(build_response(&params, image, &path))
}

/// Read a generated image back. Callers pass the `details.path` a generate
/// call returned (or just its file name); anything outside `IMAGE_DATA_DIR`
/// is refused, symlinks included, so this never becomes a general file
/// reader. The console renderer uses it for the chat preview; an agent uses
/// it when it needs to look at a picture it generated without `inline`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImageReadRequest {
    /// Absolute path returned by `image::generate`, or the bare file name
    /// inside `data/provider-openai-codex/images/`.
    pub path: String,
    /// `preview` (default; a JPEG at most 512px / ~80 KB that fits the
    /// harness result cap) or `full` (the saved bytes as they are).
    #[serde(default)]
    pub variant: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ImageReadDetails {
    /// Absolute path of the file that was read.
    pub path: String,
    /// Mime of the saved file.
    pub mime: String,
    /// Saved file size in bytes.
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// `preview` or `full`: what the image block carries.
    pub variant: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<PreviewDetails>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ImageReadResponse {
    /// `[image block, text line]` — the same viewable envelope as generate.
    pub content: Vec<ContentBlock>,
    pub details: ImageReadDetails,
}

const READ_VARIANTS: [&str; 2] = ["preview", "full"];

pub fn mime_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => "image/png",
    }
}

/// Resolve a caller path to an existing file inside `dir`. Both sides are
/// canonicalized, so `..` segments and symlinks pointing elsewhere fail the
/// prefix check instead of escaping.
pub fn resolve_image_path(dir: &Path, requested: &str) -> Result<PathBuf, String> {
    let requested = requested.trim();
    if requested.is_empty() {
        return Err("path must not be empty".into());
    }
    let dir = std::fs::canonicalize(dir)
        .map_err(|_| format!("no generated images yet ({} does not exist)", dir.display()))?;
    let candidate = {
        let p = Path::new(requested);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            dir.join(p)
        }
    };
    let resolved = std::fs::canonicalize(&candidate)
        .map_err(|_| format!("{requested:?} does not exist under {}", dir.display()))?;
    if !resolved.starts_with(&dir) {
        return Err(format!("{requested:?} is outside {}", dir.display()));
    }
    if !resolved.is_file() {
        return Err(format!("{requested:?} is not a file"));
    }
    Ok(resolved)
}

pub async fn read(req: ImageReadRequest) -> Result<ImageReadResponse, Error> {
    read_in(&iii_worker_paths::project_path(IMAGE_DATA_DIR), req).await
}

pub async fn read_in(dir: &Path, req: ImageReadRequest) -> Result<ImageReadResponse, Error> {
    let variant = one_of("variant", req.variant, &READ_VARIANTS)
        .map_err(|m| Error::Handler(format!("invalid_input: {m}")))?
        .unwrap_or_else(|| "preview".to_string());
    let path = resolve_image_path(dir, &req.path)
        .map_err(|m| Error::Handler(format!("invalid_input: {m}")))?;
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| Error::Handler(format!("provider/read: {}: {e}", path.display())))?;
    let mime = mime_for_path(&path);
    let (width, height) = match dimensions(&bytes) {
        Some((w, h)) => (Some(w), Some(h)),
        None => (None, None),
    };
    let dims = match (width, height) {
        (Some(w), Some(h)) => format!("{w}x{h}"),
        _ => "unknown size".to_string(),
    };
    let path_str = path.display().to_string();
    let mut line = format!("{path_str} ({dims}, {} bytes, {mime})", bytes.len());
    let (block, preview_details) = if variant == "full" {
        (image_block(mime, &bytes), None)
    } else {
        let preview =
            make_preview(&bytes).map_err(|m| Error::Handler(format!("provider/bad_image: {m}")))?;
        line.push_str(&format!(
            "; preview {}x{} jpeg",
            preview.width, preview.height
        ));
        let details = PreviewDetails {
            mime: "image/jpeg".into(),
            width: preview.width,
            height: preview.height,
            bytes: preview.bytes.len() as u64,
        };
        (image_block("image/jpeg", &preview.bytes), Some(details))
    };
    Ok(ImageReadResponse {
        content: vec![block, text_block(line)],
        details: ImageReadDetails {
            path: path_str,
            mime: mime.into(),
            bytes: bytes.len() as u64,
            width,
            height,
            variant,
            preview: preview_details,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(model: &str) -> ImageGenerateRequest {
        ImageGenerateRequest {
            model: model.into(),
            prompt: "a lighthouse at dusk".into(),
            size: None,
            quality: None,
            output_format: None,
            background: None,
            file_name: None,
            host_model: None,
            inline: None,
        }
    }

    /// A real PNG (noisy gradient so JPEG has to work for its budget), with
    /// an alpha channel to exercise the white flattening.
    fn sample_png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(w, h, |x, y| {
            let r = (x * 255 / w.max(1)) as u8;
            let g = (y * 255 / h.max(1)) as u8;
            let b = ((x ^ y) & 0xFF) as u8;
            let a = if x < w / 2 { 0 } else { 255 };
            image::Rgba([r, g, b, a])
        });
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        buf
    }

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn accepts_exactly_the_two_image_models_and_defaults_the_host() {
        for m in IMAGE_MODELS {
            let p = validate(req(m)).unwrap();
            assert_eq!(p.model, m);
            assert_eq!(p.host_model, DEFAULT_HOST_MODEL);
            assert_eq!(p.inline, InlineMode::None);
        }
        let err = validate(req("dall-e-3")).unwrap_err();
        assert!(
            err.contains("gpt-image-2.5-sunburst") && err.contains("gpt-image-2.5-flare"),
            "{err}"
        );
    }

    #[test]
    fn host_model_strips_router_prefix_and_rejects_image_models() {
        let mut r = req(IMAGE_MODELS[0]);
        r.host_model = Some(" codex/gpt-5.6-luna ".into());
        assert_eq!(validate(r).unwrap().host_model, "gpt-5.6-luna");
        let mut r = req(IMAGE_MODELS[0]);
        r.host_model = Some(IMAGE_MODELS[1].into());
        assert!(validate(r).unwrap_err().contains("host_model"));
    }

    #[test]
    fn rejects_bad_enums_empty_prompt_and_transparent_jpeg() {
        let mut r = req(IMAGE_MODELS[0]);
        r.quality = Some("ultra".into());
        assert!(validate(r)
            .unwrap_err()
            .starts_with("quality must be one of"));
        let mut r = req(IMAGE_MODELS[0]);
        r.prompt = "".into();
        assert_eq!(validate(r).unwrap_err(), "prompt must not be empty");
        let mut r = req(IMAGE_MODELS[0]);
        r.output_format = Some("JPEG".into());
        r.background = Some("Transparent".into());
        assert!(validate(r).unwrap_err().contains("transparent"));
        let mut r = req(IMAGE_MODELS[0]);
        r.inline = Some("thumb".into());
        assert!(validate(r)
            .unwrap_err()
            .starts_with("inline must be one of"));
    }

    #[test]
    fn body_pins_the_tool_and_forces_it() {
        let mut r = req(IMAGE_MODELS[1]);
        r.size = Some("1536x1024".into());
        r.quality = Some("high".into());
        r.file_name = Some("cover art".into());
        r.inline = Some("none".into());
        let p = validate(r).unwrap();
        assert_eq!(p.file_stem, "cover-art");
        let body = build_body(&p);
        assert_eq!(body["model"], DEFAULT_HOST_MODEL);
        assert_eq!(body["stream"], true);
        assert_eq!(body["store"], false);
        assert!(
            body.get("max_output_tokens").is_none(),
            "backend rejects it"
        );
        assert!(body.get("inline").is_none(), "inline is local, never sent");
        assert_eq!(body["input"][0]["role"], "system");
        assert_eq!(body["input"][1]["role"], "user");
        assert_eq!(
            body["input"][1]["content"][0]["text"],
            "a lighthouse at dusk"
        );
        let tool = &body["tools"][0];
        assert_eq!(tool["type"], "image_generation");
        assert_eq!(tool["model"], "gpt-image-2.5-flare");
        assert_eq!(tool["output_format"], "png");
        assert_eq!(tool["size"], "1536x1024");
        assert_eq!(tool["quality"], "high");
        assert!(tool.get("background").is_none());
        assert_eq!(body["tool_choice"]["type"], "image_generation");
    }

    #[test]
    fn collector_takes_the_image_from_output_item_done_and_usage_from_completed() {
        let png = sample_png(8, 8);
        let mut c = ImageStreamCollector::default();
        c.feed_block("event: response.created\ndata: {\"type\":\"response.created\"}\n\n");
        c.feed_block(
            "event: response.image_generation_call.generating\ndata: {\"type\":\"response.image_generation_call.generating\"}\n\n",
        );
        let done = json!({
            "type": "response.output_item.done",
            "item": { "type": "image_generation_call", "status": "completed",
                      "result": b64(&png), "revised_prompt": "a tall lighthouse" }
        });
        c.feed_block(&format!("data: {done}\n\n"));
        assert!(!c.terminal);
        let completed = json!({
            "type": "response.completed",
            "response": { "output": [{ "type": "image_generation_call", "result": null }],
                          "usage": { "input_tokens": 40, "output_tokens": 1200, "total_tokens": 1240 } }
        });
        c.feed_block(&format!("data: {completed}\n\n"));
        assert!(c.terminal);
        let img = c.finish().unwrap();
        assert_eq!(img.bytes, png);
        assert_eq!(img.revised_prompt.as_deref(), Some("a tall lighthouse"));
        assert_eq!(img.usage.unwrap().total_tokens, Some(1240));
    }

    #[test]
    fn collector_reads_a_result_only_present_in_completed() {
        let png = sample_png(8, 8);
        let mut c = ImageStreamCollector::default();
        let completed = json!({
            "type": "response.completed",
            "response": { "output": [
                { "type": "reasoning", "summary": [] },
                { "type": "image_generation_call", "result": b64(&png) }
            ] }
        });
        c.feed_block(&format!("data: {completed}\n\n"));
        assert_eq!(c.finish().unwrap().bytes, png);
    }

    #[test]
    fn collector_explains_text_answers_failures_and_done_without_image() {
        let mut c = ImageStreamCollector::default();
        c.feed_event(&json!({ "type": "response.output_text.delta", "delta": "I cannot " }));
        c.feed_event(&json!({ "type": "response.output_text.delta", "delta": "draw that." }));
        c.feed_event(&json!({ "type": "response.completed", "response": { "output": [] } }));
        let err = c.finish().unwrap_err();
        assert!(err.contains("I cannot draw that."), "{err}");

        let mut c = ImageStreamCollector::default();
        c.feed_event(&json!({
            "type": "response.failed",
            "response": { "error": { "message": "image_generation is not enabled" } }
        }));
        assert!(c.terminal);
        assert_eq!(c.finish().unwrap_err(), "image_generation is not enabled");

        let mut c = ImageStreamCollector::default();
        c.feed_event(&json!({ "type": "error", "message": "boom" }));
        assert_eq!(c.finish().unwrap_err(), "boom");

        let mut c = ImageStreamCollector::default();
        c.feed_block("data: [DONE]\n\n");
        assert!(c.terminal);
        assert!(c
            .finish()
            .unwrap_err()
            .contains("without an image_generation_call"));

        let mut c = ImageStreamCollector::default();
        c.feed_event(&json!({
            "type": "response.output_item.done",
            "item": { "type": "image_generation_call", "result": "@@@" }
        }));
        assert!(c.finish().unwrap_err().contains("base64"));
    }

    #[test]
    fn headers_carry_the_codex_identity() {
        let backend = CodexBackendConfig {
            access_token: "at".into(),
            account_id: "acc-1".into(),
            api_url: crate::config::DEFAULT_API_URL.into(),
        };
        let h = headers_for(&backend, "gpt-5.5");
        assert!(h.contains(&("authorization", "Bearer at".to_string())));
        assert!(h.contains(&("chatgpt-account-id", "acc-1".to_string())));
        assert!(h.contains(&("accept", "text/event-stream".to_string())));
        assert!(h.iter().any(|(k, _)| *k == "version"));
    }

    #[test]
    fn preview_fits_the_harness_budget_and_keeps_aspect() {
        let png = sample_png(1024, 1536);
        assert!(png.len() > PREVIEW_MAX_BYTES);
        assert_eq!(dimensions(&png), Some((1024, 1536)));
        let preview = make_preview(&png).unwrap();
        assert!(
            preview.bytes.len() <= PREVIEW_MAX_BYTES,
            "{}",
            preview.bytes.len()
        );
        assert_eq!((preview.width, preview.height), (341, 512));
        assert!(preview.bytes.len() * 4 / 3 * 2 + 4096 < 262_144);
        let decoded = image::load_from_memory(&preview.bytes).unwrap().to_rgb8();
        let px = decoded.get_pixel(4, 4);
        assert!(px[0] > 200 && px[1] > 200 && px[2] > 200, "{px:?}");
        let small = make_preview(&sample_png(64, 32)).unwrap();
        assert_eq!((small.width, small.height), (64, 32));
        assert!(make_preview(b"not an image")
            .unwrap_err()
            .starts_with("decode"));
    }

    #[tokio::test]
    async fn response_is_a_viewable_envelope_and_the_file_exists() {
        let dir = std::env::temp_dir().join(format!(
            "provider-openai-codex-image-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let mut p = validate(req(IMAGE_MODELS[0])).unwrap();
        p.inline = InlineMode::Preview;
        let png = sample_png(1024, 1024);
        let path = persist_in(&dir, &png, &p.file_stem, &p.output_format)
            .await
            .unwrap();
        assert!(path.is_absolute());
        assert_eq!(std::fs::read(&path).unwrap(), png);
        let generated = || GeneratedImage {
            bytes: png.clone(),
            revised_prompt: None,
            usage: None,
        };

        let resp = build_response(&p, generated(), &path);
        assert_eq!(resp.content.len(), 2);
        assert_eq!(resp.content[0].r#type, "image");
        assert_eq!(resp.content[0].mime.as_deref(), Some("image/jpeg"));
        assert_eq!(resp.content[1].r#type, "text");
        let line = resp.content[1].text.as_deref().unwrap();
        assert!(
            line.contains("via Codex") && line.contains("1024x1024"),
            "{line}"
        );
        assert!(line.contains("inline preview 512x512 jpeg"), "{line}");
        assert_eq!(resp.details.host_model, DEFAULT_HOST_MODEL);
        assert_eq!(resp.details.path, path.display().to_string());
        assert_eq!(resp.details.mime, "image/png");
        assert_eq!(
            (resp.details.width, resp.details.height),
            (Some(1024), Some(1024))
        );
        assert_eq!(resp.details.inline, "preview");
        assert_eq!(resp.details.preview.as_ref().unwrap().width, 512);
        let v = serde_json::to_value(&resp).unwrap();
        assert!(serde_json::to_vec(&v).unwrap().len() * 2 < 262_144);
        assert!(v["content"][0].get("text").is_none());
        assert!(v["details"].get("usage").is_none());

        let mut full = p.clone();
        full.inline = InlineMode::Full;
        let resp = build_response(&full, generated(), &path);
        assert_eq!(resp.content[0].mime.as_deref(), Some("image/png"));
        assert_eq!(resp.content[0].data.as_deref(), Some(b64(&png).as_str()));
        assert!(resp.details.preview.is_none());

        let mut none = validate(req(IMAGE_MODELS[0])).unwrap();
        none.inline = InlineMode::None;
        let resp = build_response(&none, generated(), &path);
        assert_eq!(resp.content.len(), 1);
        assert_eq!(resp.content[0].r#type, "text");
        let line = resp.content[0].text.as_deref().unwrap();
        assert!(
            line.contains("saved to") && line.contains("image::read"),
            "{line}"
        );
        assert_eq!(resp.details.inline, "none");
        assert!(serde_json::to_vec(&resp).unwrap().len() < 4096);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn image_paths_stay_inside_the_data_dir() {
        let root = std::env::temp_dir().join(format!(
            "provider-openai-codex-image-paths-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let dir = root.join("images");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ok.png"), sample_png(4, 4)).unwrap();
        std::fs::write(root.join("secret.txt"), b"nope").unwrap();
        let canon = std::fs::canonicalize(&dir).unwrap();

        assert_eq!(
            resolve_image_path(&dir, "ok.png").unwrap(),
            canon.join("ok.png")
        );
        let abs = canon.join("ok.png").display().to_string();
        assert_eq!(
            resolve_image_path(&dir, &abs).unwrap(),
            canon.join("ok.png")
        );
        assert!(resolve_image_path(&dir, "../secret.txt")
            .unwrap_err()
            .contains("outside"));
        let secret = root.join("secret.txt").display().to_string();
        assert!(resolve_image_path(&dir, &secret)
            .unwrap_err()
            .contains("outside"));
        assert!(resolve_image_path(&dir, "missing.png")
            .unwrap_err()
            .contains("does not exist"));
        assert!(resolve_image_path(&dir, "  ")
            .unwrap_err()
            .contains("empty"));
        assert!(resolve_image_path(&dir, ".")
            .unwrap_err()
            .contains("not a file"));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("secret.txt"), dir.join("link.png")).unwrap();
            assert!(resolve_image_path(&dir, "link.png")
                .unwrap_err()
                .contains("outside"));
        }
        assert!(resolve_image_path(&root.join("never"), "x.png")
            .unwrap_err()
            .contains("no generated images yet"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn read_returns_a_preview_by_default_and_the_bytes_on_request() {
        let dir = std::env::temp_dir().join(format!(
            "provider-openai-codex-image-read-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let png = sample_png(1024, 768);
        let path = persist_in(&dir, &png, "hero", "png").await.unwrap();

        let preview = read_in(
            &dir,
            ImageReadRequest {
                path: path.display().to_string(),
                variant: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(preview.content.len(), 2);
        assert_eq!(preview.content[0].mime.as_deref(), Some("image/jpeg"));
        let inline_len = preview.content[0].data.as_deref().unwrap().len();
        assert!(inline_len <= PREVIEW_MAX_BYTES * 4 / 3 + 4, "{inline_len}");
        assert_eq!(preview.details.variant, "preview");
        assert_eq!(preview.details.mime, "image/png");
        assert_eq!(
            (preview.details.width, preview.details.height),
            (Some(1024), Some(768))
        );
        assert_eq!(preview.details.preview.as_ref().unwrap().width, 512);
        let line = preview.content[1].text.as_deref().unwrap();
        assert!(line.contains("preview 512x384 jpeg"), "{line}");

        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        let full = read_in(
            &dir,
            ImageReadRequest {
                path: name,
                variant: Some("FULL".into()),
            },
        )
        .await
        .unwrap();
        assert_eq!(full.content[0].mime.as_deref(), Some("image/png"));
        assert_eq!(full.content[0].data.as_deref(), Some(b64(&png).as_str()));
        assert_eq!(full.details.variant, "full");
        assert!(full.details.preview.is_none());

        let bad = read_in(
            &dir,
            ImageReadRequest {
                path: "hero.png".into(),
                variant: Some("thumb".into()),
            },
        )
        .await
        .unwrap_err();
        assert!(bad.to_string().contains("variant must be one of"), "{bad}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn undecodable_bytes_degrade_to_the_text_line() {
        let mut p = validate(req(IMAGE_MODELS[0])).unwrap();
        p.inline = InlineMode::Preview;
        let resp = build_response(
            &p,
            GeneratedImage {
                bytes: b"definitely not an image".to_vec(),
                revised_prompt: None,
                usage: None,
            },
            Path::new("/tmp/x.png"),
        );
        assert_eq!(resp.content.len(), 1);
        assert!(resp.content[0]
            .text
            .as_deref()
            .unwrap()
            .contains("inline preview unavailable"));
        assert!(resp.details.preview.is_none());
    }
}
