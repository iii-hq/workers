//! `provider::xai::count_tokens` — prompt token counting with Grok's own
//! tokenizer, reached over the wire.
//!
//! xAI publishes no endpoint that meters a whole prompt; what it publishes is
//! a tokenizer (`…/tokenize-text`), which takes text and answers with the
//! tokens it becomes. So the count renders the request the way this worker
//! sends it (the provider's own wire rows and tool array), tokenizes that
//! text in one call, and adds what xAI bills on top of the text: a
//! server-side preamble per model family, a framing cost per row, a tools
//! header, image tokens from each image's dimensions, and the definitions of
//! the server-side tools on the Agent Tools path. Those constants are
//! measured against billed `prompt_tokens`, not published by xAI.
//!
//! Exposed behind `router::count_tokens`.

use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::provider_scaffold::cache::ScaffoldCache;
use llm_router::provider_scaffold::endpoint_count::{base_route_url, post_count};
use llm_router::provider_scaffold::vocabulary_count::ESTIMATOR_TOKENIZER;
use llm_router::types::events::ErrorKind;
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use llm_router::types::router::ResponseFormat;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::config_from_resolve;
use crate::configuration::ConfigCell;
use crate::errors::classify_bus_error;
use crate::request::build_headers;
use crate::state;
use crate::wire::messages::to_wire_messages;
use crate::wire::tools::functions_to_wire;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountTokensRequest {
    /// Model id the prompt targets; selects the tokenizer upstream.
    pub model: String,
    /// System prompt counted as its own framed wire row when present.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Function invocation schemas; each serialized schema counts toward the
    /// total.
    #[serde(default)]
    pub tools: Option<Vec<AgentFunction>>,
    /// Wire agent messages, the same shape `provider::xai::stream` accepts.
    /// Must be non-empty.
    pub messages: Vec<AgentMessage>,
    /// The turn's structured-output request, when it has one; its schema is
    /// billed as prompt.
    #[serde(default)]
    pub response_format: Option<ResponseFormat>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CountTokensResponse {
    pub model: String,
    /// Prompt tokens for the assembled request: Grok's tokenizer over the
    /// rendered text, xAI's measured overhead on top.
    pub tokens: u64,
    /// Always `tokenizer`: a real vocabulary produced the count, but the
    /// upstream metered the text rather than the request.
    pub estimator: String,
}

pub async fn handle(
    iii: &IIIClient,
    http: &reqwest::Client,
    cache: &ScaffoldCache,
    cell: &ConfigCell,
    req: CountTokensRequest,
) -> Result<CountTokensResponse, Error> {
    // Dumb pipe: an empty request is a caller bug, never padded into a
    // countable one with placeholder messages.
    if req.messages.is_empty() {
        return Err(Error::Handler(
            "invalid_input: messages must not be empty".into(),
        ));
    }

    let token = cache.load_token(iii, state::STATE_SCOPE).await;
    let resolved = match cache
        .resolve(
            iii,
            crate::PROVIDER_ID,
            token.as_deref(),
            Some(crate::register::CREDENTIAL_ENV_VAR),
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            if classify_bus_error(&e) == ErrorKind::AuthExpired {
                cache.invalidate();
            }
            return Err(e);
        }
    };
    let cfg = config_from_resolve(&req.model, None, &resolved)
        .map_err(|e| Error::Handler(e.to_string()))?;

    // The routing decision the stream makes: server tools ride only the
    // Agent Tools path, which a request with client functions never takes.
    let wc = { cell.read().await.clone() };
    let has_client_functions = req.tools.as_ref().is_some_and(|tools| !tools.is_empty());
    let server_tools: Vec<String> =
        if crate::stream_fn::uses_responses_agent_tools(&wc, has_client_functions) {
            wc.tool_sources
                .iter()
                .map(|t| t.as_type().to_string())
                .collect()
        } else {
            Vec::new()
        };
    let headers = build_headers(&cfg, None)
        .into_iter()
        .map(|(name, value)| (name.to_string(), value))
        .collect();
    let tokens = count(http, &cfg.api_url, headers, &cfg.model, &req, &server_tools).await?;

    Ok(CountTokensResponse {
        model: cfg.model,
        tokens,
        estimator: ESTIMATOR_TOKENIZER.into(),
    })
}

/// Tokenize the rendered prompt in one `tokenize-text` call and add the
/// overhead xAI bills on top of it.
pub async fn count(
    http: &reqwest::Client,
    api_url: &str,
    headers: Vec<(String, String)>,
    model: &str,
    req: &CountTokensRequest,
    server_tools: &[String],
) -> Result<u64, Error> {
    let prompt = prompt(model, req, server_tools);
    let text = prompt.text.join("\n");
    if text.is_empty() {
        return Ok(prompt.fixed);
    }
    let text_tokens = post_count(
        http,
        base_route_url(api_url, "tokenize-text"),
        headers,
        json!({ "model": model, "text": text }),
        // The reply is the tokens themselves, so the count is their number.
        |reply| {
            reply
                .get("token_ids")
                .and_then(Value::as_array)
                .map(|tokens| tokens.len() as u64)
        },
    )
    .await?;
    Ok(prompt.fixed + text_tokens)
}

/// The request as xAI sees it: the text to tokenize, and the tokens billed
/// on top of that text.
struct Prompt {
    text: Vec<String>,
    fixed: u64,
}

// Measured on 2026-10-27 against billed `prompt_tokens` (grok-build-0.1,
// grok-4.3, grok-4.20, grok-4.5/4.6/4.7); xAI publishes none of them.
/// Framing per wire row, the newline that joins it to the next included.
const ROW: u64 = 4;
/// Once per request that carries client tools, and per tool, past their
/// schemas.
const TOOLS_HEADER: u64 = 29;
const TOOL: u64 = 1;
/// Text around each image part, past its image tokens.
const IMAGE_WRAPPER: u64 = 31;
/// `response_format: json_schema`, past the schema itself.
const JSON_SCHEMA_HEADER: u64 = 8;
/// `response_format: json_object`.
const JSON_OBJECT: u64 = 20;
/// Once per request on the Agent Tools path, past each tool's definition.
const SERVER_TOOLS_HEADER: u64 = 367;
/// One image token per 32×32 patch, plus two around them; past this many
/// patches (~2.4M pixels) xAI scales the image down first.
const IMAGE_PATCH: u64 = 32;
const IMAGE_MAX_PATCHES: u64 = 49 * 49;
const IMAGE_MARKERS: u64 = 2;

/// The server-side preamble xAI puts in front of every prompt: billed
/// tokens for "hi" less its one text token and its row.
fn preamble(model: &str) -> u64 {
    let id = model.to_ascii_lowercase();
    if id.starts_with("grok-build") || id.starts_with("grok-4.20") {
        182
    } else if id.starts_with("grok-4.3") {
        188
    } else if id.starts_with("grok-4.5") {
        490
    } else if id.starts_with("grok-4.6") {
        632
    } else {
        // grok-4.7, and any family not measured yet: the preamble has grown
        // with each release, so the largest one known is the safe guess.
        1238
    }
}

/// Definition tokens of one server-side tool on `/v1/responses` (grok-4.3).
fn server_tool(kind: &str) -> u64 {
    match kind {
        "web_search" => 479,
        "x_search" => 978,
        "code_interpreter" => 215,
        // collections_search: not measured, xAI rejects it without
        // `vector_store_ids`, which this worker never sends.
        _ => 0,
    }
}

fn prompt(model: &str, req: &CountTokensRequest, server_tools: &[String]) -> Prompt {
    let rows = to_wire_messages(&req.messages, req.system_prompt.as_deref().unwrap_or(""));
    let mut text = Vec::new();
    let mut fixed = preamble(model) + ROW * rows.len() as u64;
    for row in &rows {
        match &row["content"] {
            Value::String(content) => text.push(content.clone()),
            Value::Array(parts) => {
                for part in parts {
                    if let Some(t) = part["text"].as_str() {
                        text.push(t.to_string());
                    }
                    if let Some(url) = part["image_url"]["url"].as_str() {
                        let data = url.split_once(',').map_or(url, |(_, data)| data);
                        fixed += image_tokens(data) + IMAGE_WRAPPER;
                    }
                }
            }
            _ => {}
        }
        for call in row["tool_calls"].as_array().into_iter().flatten() {
            text.push(call["function"]["name"].as_str().unwrap_or_default().into());
            text.push(
                call["function"]["arguments"]
                    .as_str()
                    .unwrap_or_default()
                    .into(),
            );
        }
    }
    let tools = functions_to_wire(req.tools.as_deref().unwrap_or(&[]));
    if !tools.is_empty() {
        fixed += TOOLS_HEADER + TOOL * tools.len() as u64;
        text.extend(tools.iter().map(|tool| python_json(&tool["function"])));
    }
    if server_tools.is_empty() {
        match req.response_format.as_ref().map(|rf| rf.schema.as_ref()) {
            Some(Some(schema)) => {
                fixed += JSON_SCHEMA_HEADER;
                text.push(python_json(schema));
            }
            Some(None) => fixed += JSON_OBJECT,
            None => {}
        }
    } else {
        // The Agent Tools path drops response_format (stream_fn warns).
        fixed += SERVER_TOOLS_HEADER + server_tools.iter().map(|t| server_tool(t)).sum::<u64>();
    }
    Prompt { text, fixed }
}

/// Image tokens from the PNG or JPEG header in base64 `data`; anything
/// unreadable counts as the largest image xAI bills.
fn image_tokens(data: &str) -> u64 {
    // The header sits near the start; a JPEG's metadata segments rarely
    // run past this, and one that does counts as the largest image.
    let head = &data.as_bytes()[..data.len().min(256 * 1024)];
    let patches = match dimensions(&decode_base64(head)) {
        Some((w, h)) => {
            let (w, h) = (w as u64, h as u64);
            let patches = w.div_ceil(IMAGE_PATCH) * h.div_ceil(IMAGE_PATCH);
            if patches <= IMAGE_MAX_PATCHES {
                patches
            } else {
                // Scaled to fit the patch budget, each side floored to whole
                // patches: 2048×1536 bills as 1792×1344.
                let scale = ((IMAGE_MAX_PATCHES * IMAGE_PATCH * IMAGE_PATCH) as f64
                    / (w * h) as f64)
                    .sqrt();
                let side = |n: u64| ((n as f64 * scale) as u64 / IMAGE_PATCH).max(1);
                side(w) * side(h)
            }
        }
        None => IMAGE_MAX_PATCHES,
    };
    patches + IMAGE_MARKERS
}

/// Width and height from a PNG `IHDR` or a JPEG start-of-frame marker, the
/// two formats xAI accepts.
fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let be16 = |at: usize| Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as u32);
    let be32 = |at: usize| Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some((be32(16)?, be32(20)?));
    }
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut at = 2;
    while *bytes.get(at)? == 0xFF {
        let marker = *bytes.get(at + 1)?;
        // SOF0..SOF15, less DHT (C4), JPG (C8) and DAC (CC).
        if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
            return Some((be16(at + 7)?, be16(at + 5)?));
        }
        at += 2 + be16(at + 2)? as usize;
    }
    None
}

fn decode_base64(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for &c in data {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => continue,
        };
        acc = (acc << 6 | value as u32) & 0xFF_FFFF;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

/// JSON as Python's `json.dumps` writes it, which is the form xAI bills
/// tool and response schemas in: a space after every `:` and `,`.
fn python_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let fields: Vec<String> = map
                .iter()
                .map(|(key, value)| {
                    format!("{}: {}", Value::from(key.as_str()), python_json(value))
                })
                .collect();
            format!("{{{}}}", fields.join(", "))
        }
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(python_json).collect();
            format!("[{}]", items.join(", "))
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_router::types::content::ContentBlock;
    use llm_router::types::events::StopReason;
    use llm_router::types::messages::{
        AssistantMessage, AssistantRoleTag, FunctionResultMessage, FunctionResultRoleTag,
        UserMessage, UserRoleTag,
    };

    const MODEL: &str = "grok-build-0.1";

    fn user(content: Vec<ContentBlock>) -> AgentMessage {
        AgentMessage::User(UserMessage {
            role: UserRoleTag::User,
            content,
            timestamp: 1,
        })
    }
    fn hi() -> AgentMessage {
        user(vec![ContentBlock::Text { text: "hi".into() }])
    }
    fn calls(ids: &[&str]) -> AgentMessage {
        AgentMessage::Assistant(AssistantMessage {
            role: AssistantRoleTag::Assistant,
            content: ids
                .iter()
                .map(|id| ContentBlock::FunctionCall {
                    id: id.to_string(),
                    function_id: "docs::read".into(),
                    arguments: json!({ "segment": 3 }),
                })
                .collect(),
            stop_reason: StopReason::End,
            native_stop_reason: None,
            error_message: None,
            error_kind: None,
            warnings: None,
            usage: None,
            model: MODEL.into(),
            provider: "xai".into(),
            timestamp: 2,
        })
    }
    fn result(id: &str, content: Vec<ContentBlock>, details: Value) -> AgentMessage {
        AgentMessage::FunctionResult(FunctionResultMessage {
            role: FunctionResultRoleTag::FunctionResult,
            function_call_id: id.into(),
            function_id: "docs::read".into(),
            content,
            details,
            is_error: false,
            timestamp: 3,
        })
    }
    fn text(text: &str) -> ContentBlock {
        ContentBlock::Text { text: text.into() }
    }
    fn image(data: String) -> ContentBlock {
        ContentBlock::Image {
            mime: "image/png".into(),
            data,
            attachment_id: None,
        }
    }
    fn request(tools: Vec<AgentFunction>, messages: Vec<AgentMessage>) -> CountTokensRequest {
        CountTokensRequest {
            model: MODEL.into(),
            system_prompt: None,
            tools: Some(tools),
            messages,
            response_format: None,
        }
    }
    fn reader() -> AgentFunction {
        AgentFunction {
            name: "docs::read".into(),
            description: "Read one segment.".into(),
            parameters: json!({ "type": "object", "properties": { "segment": { "type": "integer" } } }),
            label: None,
            execution_mode: None,
        }
    }

    /// Base64 of an image header that says `width`×`height`; the count reads
    /// nothing past the header.
    fn png(width: u32, height: u32) -> String {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        bytes.extend(width.to_be_bytes());
        bytes.extend(height.to_be_bytes());
        bytes.extend([8, 2, 0, 0, 0]);
        base64(&bytes)
    }
    fn jpeg(width: u16, height: u16) -> String {
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC0, 0, 11, 8];
        bytes.extend(height.to_be_bytes());
        bytes.extend(width.to_be_bytes());
        bytes.extend([1, 1, 0x11, 0]);
        base64(&bytes)
    }
    fn base64(bytes: &[u8]) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n =
                chunk.iter().fold(0u32, |acc, b| acc << 8 | *b as u32) << (8 * (3 - chunk.len()));
            for i in 0..=chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            }
            out.extend(std::iter::repeat_n('=', 3 - chunk.len()));
        }
        out
    }

    #[test]
    fn a_plain_turn_carries_the_server_preamble() {
        // Billed 187 for "hi" on grok-build-0.1, of which "hi" is one token.
        let p = prompt(MODEL, &request(vec![], vec![hi()]), &[]);
        assert_eq!(p.text, vec!["hi"]);
        assert_eq!(p.fixed + 1, 187);
    }

    #[test]
    fn the_preamble_follows_the_model_family() {
        // Billed for "hi" (one token) on each family, 2026-10-27.
        for (model, billed) in [
            ("grok-build-0.1", 187),
            ("grok-4.20-0309-reasoning", 187),
            ("grok-4.3", 193),
            ("grok-4.5", 495),
            ("grok-4.6", 637),
            ("grok-4.7", 1243),
        ] {
            let fixed = prompt(model, &request(vec![], vec![hi()]), &[]).fixed;
            assert_eq!(fixed + 1, billed, "{model}");
        }
        // A family not measured yet counts as the largest one, never under.
        let fixed = prompt("grok-5", &request(vec![], vec![hi()]), &[]).fixed;
        assert_eq!(fixed + 1, 1243);
    }

    #[test]
    fn tools_render_as_python_json_under_one_header() {
        let p = prompt(MODEL, &request(vec![reader()], vec![hi()]), &[]);
        assert_eq!(
            p.text,
            vec![
                "hi".to_string(),
                r#"{"description": "Read one segment.", "name": "docs__read", "parameters": {"properties": {"segment": {"type": "integer"}}, "type": "object"}}"#.to_string(),
            ]
        );
        let bare = prompt(MODEL, &request(vec![], vec![hi()]), &[]);
        assert_eq!(p.fixed - bare.fixed, TOOLS_HEADER + TOOL);
    }

    #[test]
    fn calls_results_denials_and_orphans_count_their_wire_text() {
        let denied = json!({ "status": "denied", "reason": "policy" });
        let p = prompt(
            MODEL,
            &request(
                vec![],
                vec![
                    hi(),
                    calls(&["c1", "c2"]),
                    result("c1", vec![text("segment three")], denied.clone()),
                ],
            ),
            &[],
        );
        assert_eq!(
            p.text,
            vec![
                "hi".to_string(),
                "docs__read".into(),
                r#"{"segment":3}"#.into(),
                "docs__read".into(),
                r#"{"segment":3}"#.into(),
                // c2 has no result: the wire answers it with a placeholder.
                "Tool call was interrupted before completing. Continue without its output.".into(),
                format!("[PERMISSION_DENIED]\n{denied}\n\nsegment three"),
            ]
        );
        // A call costs only its text; the rows past the user's are the
        // assistant's and two tool rows.
        let bare = prompt(MODEL, &request(vec![], vec![hi()]), &[]);
        assert_eq!(p.fixed - bare.fixed, 3 * ROW);
    }

    #[test]
    fn images_count_from_their_dimensions() {
        // Billed image_tokens on grok-build-0.1: 66 for 256×256, 477 for
        // 800×600, 10 for 100×50, and past ~2.4M pixels the image is scaled
        // down: 2354 for 2048×1536 and 3000×1000, 2403 for 4096×4096.
        for ((w, h), tokens) in [
            ((256, 256), 66),
            ((800, 600), 477),
            ((100, 50), 10),
            ((2048, 1536), 2354),
            ((3000, 1000), 2354),
            ((4096, 4096), 2403),
        ] {
            assert_eq!(image_tokens(&png(w, h)), tokens, "{w}x{h}");
        }
        assert_eq!(image_tokens(&jpeg(800, 600)), 477);
        // Undecodable: the most an image can bill, never under.
        assert_eq!(image_tokens("not an image"), 2403);

        let bare = prompt(MODEL, &request(vec![], vec![hi()]), &[]);
        let with_image = prompt(
            MODEL,
            &request(vec![], vec![user(vec![text("hi"), image(png(256, 256))])]),
            &[],
        );
        assert_eq!(with_image.text, vec!["hi"]);
        // Billed 284 against 187 for "hi" alone.
        assert_eq!(with_image.fixed - bare.fixed, 284 - 187);
    }

    #[test]
    fn a_tool_result_image_counts_its_hoisted_user_row() {
        let round = |content| {
            let messages = vec![hi(), calls(&["c1"]), result("c1", content, json!({}))];
            prompt(MODEL, &request(vec![], messages), &[])
        };
        let with_image = round(vec![text("segment three"), image(png(256, 256))]);
        let text_only = round(vec![text("segment three")]);
        assert_eq!(
            with_image.text.last().unwrap(),
            "[image result of tool call c1]"
        );
        assert_eq!(with_image.fixed - text_only.fixed, ROW + 66 + IMAGE_WRAPPER);
    }

    #[test]
    fn response_format_counts_its_schema_or_the_json_mode_cost() {
        let schema = json!({ "type": "object", "properties": { "answer": { "type": "string" } } });
        let mut req = request(vec![], vec![hi()]);
        req.response_format = Some(ResponseFormat {
            r#type: "json".into(),
            schema: Some(schema.clone()),
        });
        let p = prompt(MODEL, &req, &[]);
        assert_eq!(p.text[1], python_json(&schema));
        let bare = prompt(MODEL, &request(vec![], vec![hi()]), &[]);
        assert_eq!(p.fixed - bare.fixed, JSON_SCHEMA_HEADER);

        req.response_format = Some(ResponseFormat {
            r#type: "json".into(),
            schema: None,
        });
        let p = prompt(MODEL, &req, &[]);
        assert_eq!(p.text, vec!["hi"]);
        // Billed 211 against 191 for the same prompt without the mode.
        assert_eq!(p.fixed - bare.fixed, 211 - 191);
    }

    #[test]
    fn server_tools_add_their_measured_definitions() {
        // grok-4.3 on /v1/responses, "hi": 193 bare, 1039 with web_search,
        // 1538 with x_search, 2017 with both, 775 with code_interpreter.
        let req = request(vec![], vec![hi()]);
        let bare = prompt("grok-4.3", &req, &[]).fixed;
        for (tools, billed) in [
            (vec!["web_search"], 1039),
            (vec!["x_search"], 1538),
            (vec!["web_search", "x_search"], 2017),
            (vec!["code_interpreter"], 775),
        ] {
            let tools: Vec<String> = tools.into_iter().map(String::from).collect();
            let fixed = prompt("grok-4.3", &req, &tools).fixed;
            assert_eq!(fixed - bare, billed - 193, "{tools:?}");
        }
        // The Agent Tools path drops response_format, so it costs nothing.
        let mut with_format = request(vec![], vec![hi()]);
        with_format.response_format = Some(ResponseFormat {
            r#type: "json".into(),
            schema: None,
        });
        let web = vec!["web_search".to_string()];
        assert_eq!(
            prompt("grok-4.3", &with_format, &web).fixed,
            prompt("grok-4.3", &req, &web).fixed
        );
    }
}
