//! `provider::openai-codex::count_tokens` — local prompt token estimation
//! with the tiktoken tokenizers (vocabularies embedded in the binary). The
//! Codex backend exposes no metering endpoint, so the count walks the
//! Responses `input` items and `tools` the stream sends (the same
//! `to_wire_messages` the body uses, so denied envelopes, orphan
//! placeholders, hoisted result images and de-duplicated results count as
//! sent) and adds the item framing and image cost the Responses bill charges.
//! It never runs the model, costs nothing, and needs no network. Model ids
//! stay namespaced (`codex/gpt-5.2`) end to end — encoder selection strips
//! the namespace internally, but the response echoes the id the caller sent.

use base64::Engine as _;
use iii_sdk::errors::Error;
use llm_router::provider_scaffold::tiktoken_count::{
    encoder_for, ESTIMATOR_TIKTOKEN, TOKENS_PER_MESSAGE, TOKENS_REPLY_PRIMING,
};
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::wire::messages::to_wire_messages;
use crate::wire::tools::functions_to_wire;

// Item framing on top of the encoded text, fitted to provider-openai's live
// Responses probe of 2026-10-27 (gpt-5-nano, billed usage; MOT-5558), which
// it matches exactly on every text shape including 12 tool rounds (16,467).
// The Codex backend takes a subscription credential and was not probed.
// A user or system item is TOKENS_PER_MESSAGE; an assistant text item is 2 more.
const TOKENS_PER_ASSISTANT_ITEM: u64 = TOKENS_PER_MESSAGE + 2;
// A call and its output bill 20 together. Every call goes out with exactly
// one output, so the split between the two items is not observable.
const TOKENS_PER_CALL_ITEM: u64 = 10;
const TOKENS_PER_OUTPUT_ITEM: u64 = 10;
// The block the server wraps around the tool schemas once any tool is sent.
// shortcut: schemas count as compact JSON, which the bill's own rendering
// undercuts (one small tool billed 19 against 28), so tool-heavy requests
// count high; render the server's form once a Codex probe can calibrate it.
const TOKENS_TOOL_BLOCK: u64 = 16;

// Image rates measured with OpenAI's free POST /v1/responses/input_tokens on
// 2026-10-28, with no `detail` as the wire sends it. gpt-5 and gpt-5.1 bill
// 512px tiles at high detail; their mini/nano and every later model bill
// 32px patches. The Codex backend was not probed (subscription credential).
const IMAGE_BASE_TOKENS: u64 = 70;
const IMAGE_TOKENS_PER_TILE: u64 = 140;
// The most 512px tiles a resized image can span (2048x768), for an image
// whose size cannot be read.
const IMAGE_MAX_TILES: u64 = 8;
// The most patches a model bills: gpt-5/5.1 mini and nano shrink an image to
// 4096, gpt-5.2 to 5.5 to 10000, and gpt-5.6 and gpt-6 reject one over 30000.
const IMAGE_PATCH_BUDGETS: [u64; 3] = [4096, 10000, 30000];
// Base64 that holds a png/webp/gif header and most jpeg ones (EXIF comes first);
// a multiple of 4 so it decodes alone. The whole image is the fallback.
const IMAGE_HEADER_B64: usize = 128 * 1024;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountTokensRequest {
    /// Model id the prompt targets; selects the tokenizer (o200k_base for
    /// gpt-4o/gpt-5/o-series and anything unknown-modern, cl100k_base for
    /// gpt-3.5 and non-o gpt-4 families).
    pub model: String,
    /// System prompt counted as its own wire message when present.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Function invocation schemas; each serialized schema counts toward
    /// the total.
    #[serde(default)]
    pub tools: Option<Vec<AgentFunction>>,
    /// Wire agent messages, the same shape `provider::openai-codex::stream`
    /// accepts. Must be non-empty.
    pub messages: Vec<AgentMessage>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CountTokensResponse {
    pub model: String,
    /// Estimated prompt tokens for the assembled request.
    pub tokens: u64,
    /// Always `tiktoken`: a local tokenizer produced the estimate.
    pub estimator: String,
}

pub fn handle(req: CountTokensRequest) -> Result<CountTokensResponse, Error> {
    // Dumb pipe: an empty request is a caller bug, never padded into a
    // countable one with placeholder messages.
    if req.messages.is_empty() {
        return Err(Error::Handler(
            "invalid_input: messages must not be empty".into(),
        ));
    }
    let bpe = encoder_for(&req.model);
    let count = |text: &str| bpe.encode_ordinary(text).len() as u64;
    let items = to_wire_messages(&req.messages, req.system_prompt.as_deref().unwrap_or(""));
    let tools = req.tools.as_deref().unwrap_or(&[]);
    let mut tokens = TOKENS_REPLY_PRIMING
        + items
            .iter()
            .map(|item| item_tokens(&req.model, item, &count))
            .sum::<u64>();
    if !tools.is_empty() {
        tokens += TOKENS_TOOL_BLOCK
            + functions_to_wire(tools)
                .iter()
                .map(|tool| count(&tool.to_string()))
                .sum::<u64>();
    }
    Ok(CountTokensResponse {
        model: req.model,
        tokens,
        estimator: ESTIMATOR_TIKTOKEN.into(),
    })
}

/// One Responses `input` item: its framing plus its encoded text and images.
fn item_tokens(model: &str, item: &Value, count: &impl Fn(&str) -> u64) -> u64 {
    let text = |value: &Value| count(value.as_str().unwrap_or(""));
    match item["type"].as_str() {
        Some("function_call") => {
            TOKENS_PER_CALL_ITEM + text(&item["name"]) + text(&item["arguments"])
        }
        Some("function_call_output") => TOKENS_PER_OUTPUT_ITEM + text(&item["output"]),
        _ => {
            let frame = if item["role"] == "assistant" {
                TOKENS_PER_ASSISTANT_ITEM
            } else {
                TOKENS_PER_MESSAGE
            };
            let parts = item["content"].as_array().map(Vec::as_slice).unwrap_or(&[]);
            frame
                + parts
                    .iter()
                    .map(|part| match part["type"].as_str() {
                        Some("input_image") => {
                            image_tokens(model, part["image_url"].as_str().unwrap_or(""))
                        }
                        _ => text(&part["text"]),
                    })
                    .sum::<u64>()
        }
    }
}

/// An `input_image` data URI's cost from the dimensions in its header.
fn image_tokens(model: &str, image_url: &str) -> u64 {
    let size = image_url.split_once(";base64,").and_then(|(_, data)| {
        let read = |b64: &str| {
            let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
            crate::image::dimensions(&bytes)
        };
        read(data.get(..IMAGE_HEADER_B64).unwrap_or(data)).or_else(|| read(data))
    });
    image_cost(model, size)
}

/// What `model` bills for an image of `size`; an unreadable image costs the
/// most its family bills.
fn image_cost(model: &str, size: Option<(u32, u32)>) -> u64 {
    let bare = model.rsplit('/').next().unwrap_or(model);
    let minor = crate::reasoning::gpt5_minor(bare);
    let early = bare == "gpt-5" || bare.starts_with("gpt-5-") || minor == Some(1);
    let small = bare.contains("-mini") || bare.contains("-nano");
    if early && !small {
        let tiles = size.map_or(IMAGE_MAX_TILES, |(width, height)| tiles(width, height));
        return IMAGE_BASE_TOKENS + IMAGE_TOKENS_PER_TILE * tiles;
    }
    let budget = match minor {
        _ if early => IMAGE_PATCH_BUDGETS[0],
        Some(2..=5) => IMAGE_PATCH_BUDGETS[1],
        _ => IMAGE_PATCH_BUDGETS[2],
    };
    // shortcut: an image over the budget counts the whole budget, where the
    // server shrinks it to a little less (and gpt-5-nano first fits it in
    // 2048x2048), so it counts high; the gpt-5.4 mini/nano budget of 2500
    // counts as 10000 for the same reason.
    let patches = size.map_or(budget, |(width, height)| {
        let side = |px: u32| u64::from(px.max(1).div_ceil(32));
        (side(width) * side(height)).min(budget)
    });
    if early && bare.contains("-nano") {
        (3 * patches).div_ceil(2)
    } else {
        6 * patches / 5 + 1
    }
}

/// OpenAI's high-detail resize: fit within 2048x2048, then shrink the short
/// side to 768, then count 512px tiles. Images are never scaled up.
fn tiles(width: u32, height: u32) -> u64 {
    let (w, h) = (f64::from(width.max(1)), f64::from(height.max(1)));
    let fit = (2048.0 / w.max(h)).min(1.0);
    let short = (768.0 / (w.min(h) * fit)).min(1.0);
    // Resized images have whole pixels; rounding keeps float noise from
    // tipping an exact 512 multiple into one more tile.
    let side = |px: f64| (px * fit * short).round() / 512.0;
    (side(w).ceil() * side(h).ceil()) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::messages::ORPHAN_TOOL_PLACEHOLDER;
    use llm_router::types::content::ContentBlock;
    use llm_router::types::events::StopReason;
    use llm_router::types::messages::{
        AssistantMessage, AssistantRoleTag, FunctionResultMessage, FunctionResultRoleTag,
        UserMessage, UserRoleTag,
    };
    use serde_json::{json, Value};

    fn user_blocks(content: Vec<ContentBlock>) -> AgentMessage {
        AgentMessage::User(UserMessage {
            role: UserRoleTag::User,
            content,
            timestamp: 1,
        })
    }

    fn user(text: &str) -> AgentMessage {
        user_blocks(vec![ContentBlock::Text { text: text.into() }])
    }

    fn assistant(content: Vec<ContentBlock>) -> AgentMessage {
        AgentMessage::Assistant(AssistantMessage {
            role: AssistantRoleTag::Assistant,
            content,
            stop_reason: StopReason::End,
            native_stop_reason: None,
            error_message: None,
            error_kind: None,
            warnings: None,
            usage: None,
            model: "m".into(),
            provider: "openai-codex".into(),
            timestamp: 2,
        })
    }

    fn call(id: &str) -> ContentBlock {
        ContentBlock::FunctionCall {
            id: id.into(),
            function_id: "docs::read".into(),
            arguments: json!({ "segment": 3 }),
        }
    }

    fn result_with(id: &str, content: Vec<ContentBlock>, details: Value) -> AgentMessage {
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

    fn result(id: &str, text: &str) -> AgentMessage {
        result_with(
            id,
            vec![ContentBlock::Text { text: text.into() }],
            json!({}),
        )
    }

    fn tool(name: &str) -> AgentFunction {
        AgentFunction {
            name: name.into(),
            description: format!("Run {name} on the workspace."),
            parameters: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
            label: None,
            execution_mode: None,
        }
    }

    fn docs_tool() -> AgentFunction {
        AgentFunction {
            name: "docs::read".into(),
            description: "Read one segment of the ladder by its index.".into(),
            parameters: json!({"type": "object", "properties": {"segment": {"type": "integer"}}, "required": ["segment"]}),
            label: None,
            execution_mode: None,
        }
    }

    fn png(width: u32, height: u32) -> ContentBlock {
        let mut bytes = Vec::new();
        image::DynamicImage::new_rgb8(width, height)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        ContentBlock::Image {
            mime: "image/png".into(),
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
            attachment_id: None,
        }
    }

    fn request(model: &str, messages: Vec<AgentMessage>) -> CountTokensRequest {
        CountTokensRequest {
            model: model.into(),
            system_prompt: None,
            tools: None,
            messages,
        }
    }

    fn count(system: &str, tools: Vec<AgentFunction>, messages: Vec<AgentMessage>) -> u64 {
        handle(CountTokensRequest {
            model: "codex/gpt-5.5".into(),
            system_prompt: Some(system.into()),
            tools: Some(tools),
            messages,
        })
        .unwrap()
        .tokens
    }

    fn text_tokens(text: &str) -> u64 {
        encoder_for("gpt-5.5").encode_ordinary(text).len() as u64
    }

    #[test]
    fn empty_messages_are_rejected() {
        assert!(handle(request("codex/gpt-5.2", vec![])).is_err());
    }

    #[test]
    fn handle_strips_the_codex_namespace_for_encoding_but_echoes_the_model_id() {
        let namespaced = handle(request("codex/gpt-5.2", vec![user("hi")])).unwrap();
        let bare = handle(request("gpt-5.2", vec![user("hi")])).unwrap();
        assert_eq!(namespaced.tokens, bare.tokens);
        assert_eq!(namespaced.model, "codex/gpt-5.2");
        assert_eq!(namespaced.estimator, "tiktoken");
    }

    // The billed numbers below are provider-openai's live Responses probe of
    // 2026-10-27 (gpt-5-nano, same tokenizer and item shapes; MOT-5558). The
    // Codex backend takes a subscription credential, so it was not probed.
    const SYSTEM: &str = "You are a careful reader. Keep identifiers verbatim.";

    #[test]
    fn text_items_match_the_responses_bill() {
        let hi = || user("hi");
        let ok = assistant(vec![ContentBlock::Text {
            text: "ok fine".into(),
        }]);
        assert_eq!(count("", vec![], vec![hi()]), 7);
        assert_eq!(count(SYSTEM, vec![], vec![hi()]), 22);
        assert_eq!(count("", vec![], vec![hi(), hi()]), 12);
        assert_eq!(count("", vec![], vec![hi(), ok, hi()]), 20);
    }

    #[test]
    fn call_rounds_differ_by_what_the_responses_bill_charged() {
        let round = |turn: Vec<ContentBlock>, results: Vec<AgentMessage>| {
            let mut messages = vec![user("hi"), assistant(turn)];
            messages.extend(results);
            count("", vec![docs_tool()], messages)
        };
        let one = round(vec![call("c1")], vec![result("c1", "segment three")]);
        let two = round(
            vec![call("c1"), call("c2")],
            vec![result("c1", "segment three"), result("c2", "segment three")],
        );
        let text_call = round(
            vec![
                ContentBlock::Text {
                    text: "reading".into(),
                },
                call("c1"),
            ],
            vec![result("c1", "segment three")],
        );
        let wide = round(
            vec![ContentBlock::FunctionCall {
                id: "w1".into(),
                function_id: "docs::read".into(),
                arguments: json!({"query": "ledger entries", "kind": "text", "paths": ["src", "docs", "tests"], "limit": 5, "options": {"case_sensitive": true, "whole_word": false}}),
            }],
            vec![result("w1", "segment three")],
        );
        // billed: 1 call 78, 2 calls 108, text+call 85, wide args 108.
        assert!(one >= 78, "1 call round counted {one}, billed 78");
        assert_eq!(two - one, 108 - 78);
        assert_eq!(text_call - one, 85 - 78);
        assert_eq!(wide - one, 108 - 78);
    }

    #[test]
    fn tool_requests_are_never_under_the_responses_bill() {
        let hi = || vec![user("hi")];
        let big = AgentFunction {
            name: "workspace::search".into(),
            description: "Search the workspace for files, symbols and text. Returns ranked matches with their paths, line numbers and a short excerpt of the surrounding code, so the caller can decide what to open next.".into(),
            parameters: json!({"type": "object", "properties": {
                "query": {"type": "string", "description": "What to look for, as plain words or a regular expression."},
                "kind": {"type": "string", "enum": ["file", "symbol", "text"], "description": "Which index to search."},
                "paths": {"type": "array", "items": {"type": "string"}, "description": "Directories to limit the search to."},
                "limit": {"type": "integer", "minimum": 1, "maximum": 200, "description": "How many matches to return."},
                "options": {"type": "object", "properties": {"case_sensitive": {"type": "boolean"}, "whole_word": {"type": "boolean"}}}
            }, "required": ["query"]}),
            label: None,
            execution_mode: None,
        };
        let shapes = [
            (vec![tool("alpha")], 42),
            (vec![tool("alpha"), tool("beta")], 61),
            (vec![tool("alpha"), tool("beta"), tool("gamma")], 80),
            (vec![big], 155),
        ];
        for (tools, billed) in shapes {
            let counted = count("", tools, hi());
            assert!(counted >= billed, "counted {counted}, billed {billed}");
        }
    }

    #[test]
    fn tiles_follow_the_published_high_detail_examples() {
        assert_eq!(tiles(1024, 1024), 4);
        assert_eq!(tiles(2048, 4096), 6);
        assert_eq!(tiles(1920, 1080), 6);
        assert_eq!(tiles(256, 256), 1);
        assert_eq!(tiles(4000, 800), 4);
        assert_eq!(tiles(0, 0), 1);
    }

    // input_tokens of "hi" plus one PNG minus "hi" alone, from OpenAI's free
    // POST /v1/responses/input_tokens on 2026-10-28 (no `detail`, as the
    // wire sends it; the endpoint matched the bill on every MOT-5558 shape).
    #[test]
    fn a_user_image_costs_what_its_model_family_bills() {
        let image_cost = |model: &str, image: ContentBlock| {
            let text = handle(request(model, vec![user("hi")])).unwrap().tokens;
            let messages = vec![user_blocks(vec![
                ContentBlock::Text { text: "hi".into() },
                image,
            ])];
            handle(request(model, messages)).unwrap().tokens - text
        };
        let measured = [
            ("codex/gpt-5", (1024, 1024), 630),
            ("codex/gpt-5-nano", (256, 256), 96),
            ("codex/gpt-5-nano", (1024, 1024), 1536),
            ("codex/gpt-5-mini", (600, 400), 297),
            ("codex/gpt-5.5", (256, 256), 77),
            ("codex/gpt-5.5", (1024, 1024), 1229),
        ];
        for (model, (width, height), billed) in measured {
            assert_eq!(
                image_cost(model, png(width, height)),
                billed,
                "{model} {width}x{height}"
            );
        }
    }

    #[test]
    fn an_image_is_sized_from_its_header_without_decoding_the_rest() {
        let ContentBlock::Image { data, .. } = png(600, 400) else {
            unreachable!()
        };
        let mut bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap();
        bytes.resize(bytes.len() + 1_000_000, 0);
        // A tail no decoder would accept: only the header may be read.
        let data = base64::engine::general_purpose::STANDARD.encode(bytes) + "!";
        let url = format!("data:image/png;base64,{data}");
        assert_eq!(image_tokens("codex/gpt-5.5", &url), 297);
    }

    #[test]
    fn every_input_format_openai_accepts_is_sized() {
        use image::ImageFormat::{Gif, Jpeg, Png, WebP};
        for format in [Png, Jpeg, WebP, Gif] {
            let mut bytes = Vec::new();
            image::DynamicImage::new_rgb8(256, 256)
                .write_to(&mut std::io::Cursor::new(&mut bytes), format)
                .unwrap();
            let data = base64::engine::general_purpose::STANDARD.encode(bytes);
            let url = format!("data:{};base64,{data}", format.to_mime_type());
            // 8x8 patches at gpt-5.5's rate, not the 12001 of an unread image.
            assert_eq!(image_tokens("codex/gpt-5.5", &url), 77, "{format:?}");
        }
    }

    #[test]
    fn image_cost_follows_the_measured_rates_and_never_counts_under_them() {
        // Same endpoint as above, on sizes too large to encode in a test.
        let exact = [
            ("gpt-5.1", (1920, 1080), 910),
            ("gpt-5-mini", (1920, 1080), 2449),
            ("gpt-5-mini", (4096, 4096), 4916),
            ("gpt-5-nano", (2048, 2048), 6144),
            ("gpt-5.2", (1920, 1080), 2449),
            ("codex/gpt-5.3-codex", (1024, 1024), 1229),
            ("gpt-5.5", (4000, 800), 3751),
            ("gpt-5.5", (4096, 4096), 12001),
            ("gpt-5.6-sol", (5000, 5000), 29579),
            ("gpt-6-sol", (4096, 4096), 19661),
        ];
        for (model, size, billed) in exact {
            assert_eq!(image_cost(model, Some(size)), billed, "{model} {size:?}");
        }
        // Shrunk below the budget by the server: counted high.
        let over = [
            ("gpt-5-nano", (4000, 800), 1248),
            ("gpt-5.4-mini", (4096, 4096), 3001),
        ];
        for (model, size, billed) in over {
            assert!(image_cost(model, Some(size)) >= billed, "{model} {size:?}");
        }
    }

    #[test]
    fn an_unreadable_image_costs_the_most_its_family_bills() {
        let with_image = |model: &str, data: &str| {
            let image = ContentBlock::Image {
                mime: "image/png".into(),
                data: data.into(),
                attachment_id: None,
            };
            let messages = vec![user_blocks(vec![image])];
            let text = handle(request(model, vec![user_blocks(vec![])])).unwrap();
            handle(request(model, messages)).unwrap().tokens - text.tokens
        };
        assert_eq!(with_image("codex/gpt-5", "AAAA"), 70 + 8 * 140);
        assert_eq!(with_image("codex/gpt-5-mini", "AAAA"), 6 * 4096 / 5 + 1);
        assert_eq!(with_image("codex/gpt-5.5", "AAAA"), 6 * 10000 / 5 + 1);
        assert_eq!(
            with_image("codex/gpt-6-sol", "not base64 é"),
            6 * 30000 / 5 + 1
        );
    }

    #[test]
    fn a_result_image_counts_its_hoisted_user_item() {
        let round = |content: Vec<ContentBlock>| {
            count(
                "",
                vec![],
                vec![
                    assistant(vec![call("t1")]),
                    result_with("t1", content, json!({})),
                ],
            )
        };
        let ok = || ContentBlock::Text { text: "ok".into() };
        let with_image = round(vec![ok(), png(600, 400)]);
        let without = round(vec![ok()]);
        let hoisted_item = TOKENS_PER_MESSAGE + text_tokens("[image result of tool call t1]") + 297;
        assert_eq!(with_image - without, hoisted_item);
    }

    #[test]
    fn a_denied_result_counts_its_envelope() {
        let details = json!({ "status": "denied", "reason": "policy forbids shell" });
        let round = |details: Value| {
            count(
                "",
                vec![],
                vec![
                    assistant(vec![call("t1")]),
                    result_with(
                        "t1",
                        vec![ContentBlock::Text {
                            text: "not run".into(),
                        }],
                        details,
                    ),
                ],
            )
        };
        let envelope = format!("[PERMISSION_DENIED]\n{details}\n\nnot run");
        assert_eq!(
            round(details) - round(json!({})),
            text_tokens(&envelope) - text_tokens("not run")
        );
    }

    #[test]
    fn an_orphan_call_counts_its_placeholder_output() {
        let orphan = count("", vec![], vec![user("hi"), assistant(vec![call("t1")])]);
        let answered = count(
            "",
            vec![],
            vec![
                user("hi"),
                assistant(vec![call("t1")]),
                result("t1", ORPHAN_TOOL_PLACEHOLDER),
            ],
        );
        assert_eq!(orphan, answered);
    }

    #[test]
    fn a_duplicated_result_counts_once_as_the_wire_sends_it() {
        let replayed = count(
            "",
            vec![],
            vec![
                assistant(vec![call("t1")]),
                result("t1", "first"),
                result("t1", "second"),
            ],
        );
        let once = count(
            "",
            vec![],
            vec![assistant(vec![call("t1")]), result("t1", "second")],
        );
        assert_eq!(replayed, once);
    }
}
