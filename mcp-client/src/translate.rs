//! Pure translation of MCP wire JSON into the blocks the harness shows, the
//! per-result size cap, and the texts of call failures.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

/// Cap per result, measured on the JSON-escaped size of the text. The harness
/// caps a result at 256 KiB counting `content` and `details` together, so each
/// text is counted twice.
pub const CAP_BYTES: usize = 122_880;
pub const TOOL_CUT_HINT: &str = "narrow the query, lower `limit`, or page with the cursor";

/// Drops the top-level `_`-prefixed keys the schema does not declare (the
/// engine adds `_caller_worker_id`). A non-object payload becomes `{}`.
pub fn strip_args(payload: Value, declared: &BTreeSet<String>) -> Map<String, Value> {
    match payload {
        Value::Object(map) => map
            .into_iter()
            .filter(|(k, _)| !k.starts_with('_') || declared.contains(k))
            .collect(),
        _ => Map::new(),
    }
}

fn char_cost(c: char, levels: u8) -> usize {
    if levels == 0 || (c.is_ascii() && !c.is_ascii_control() && c != '"' && c != '\\') {
        return if levels == 0 { c.len_utf8() } else { 1 };
    }
    if !c.is_ascii() {
        return c.len_utf8();
    }
    let mut s = c.to_string();
    for _ in 0..levels {
        let q = serde_json::to_string(&s).unwrap_or_default();
        s = q[1..q.len() - 1].to_string();
    }
    s.len()
}

/// Byte length of `text` after `levels` rounds of JSON string escaping,
/// without the surrounding quotes.
pub fn escaped_len(text: &str, levels: u8) -> usize {
    text.chars().map(|c| char_cost(c, levels)).sum()
}

/// Longest prefix on a char boundary whose escaped length fits `budget`.
fn cut_prefix(text: &str, budget: usize) -> &str {
    let mut used = 0;
    for (i, c) in text.char_indices() {
        used += char_cost(c, 1);
        if used > budget {
            return &text[..i];
        }
    }
    text
}

#[derive(Clone, Copy)]
pub struct BlockCtx<'a> {
    pub server: &'a str,
    pub read_registered: bool,
    pub asked_uri: Option<&'a str>,
    pub many: bool,
}

fn text_block(t: impl Into<String>) -> Value {
    json!({"type": "text", "text": t.into()})
}

fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn contents(item: &Value, ctx: &BlockCtx) -> Option<Value> {
    let uri = str_field(item, "uri")?;
    if let Some(text) = str_field(item, "text") {
        let with_line = ctx.many || ctx.asked_uri != Some(uri);
        return Some(text_block(if with_line {
            format!("[{uri}]\n{text}")
        } else {
            text.to_string()
        }));
    }
    let blob = str_field(item, "blob")?;
    let mime = str_field(item, "mimeType").unwrap_or("application/octet-stream");
    if mime.starts_with("image/") {
        return Some(json!({"type": "image", "mime": mime, "data": blob}));
    }
    Some(text_block(format!(
        "[binary resource {uri}, {mime}, {} bytes, not shown]",
        blob.len()
    )))
}

fn try_block(item: &Value, ctx: &BlockCtx) -> Option<Value> {
    match str_field(item, "type") {
        Some("text") => Some(text_block(str_field(item, "text")?)),
        Some("image") => Some(
            json!({"type": "image", "mime": str_field(item, "mimeType")?, "data": str_field(item, "data")?}),
        ),
        Some("audio") => Some(text_block(format!(
            "[audio {}, {} bytes, not shown]",
            str_field(item, "mimeType")?,
            str_field(item, "data")?.len()
        ))),
        Some("resource") => contents(item.get("resource")?, ctx),
        Some("resource_link") => {
            let mut lines = vec![format!(
                "{}: {}",
                str_field(item, "name")?,
                str_field(item, "uri")?
            )];
            if let Some(d) = str_field(item, "description") {
                lines.push(d.to_string());
            }
            if ctx.read_registered {
                lines.push(format!(
                    "read it with `mcp::{}::resources::read`",
                    ctx.server
                ));
            }
            Some(text_block(lines.join("\n")))
        }
        None => contents(item, ctx),
        _ => None,
    }
}

/// One MCP content block (or bare resource contents) as an output block.
pub fn block(item: &Value, ctx: &BlockCtx) -> Value {
    try_block(item, ctx).unwrap_or_else(|| text_block(item.to_string()))
}

pub fn cap_blocks(blocks: Vec<Value>, cut_note: &str) -> Vec<Value> {
    let mut left = CAP_BYTES;
    let mut out = Vec::new();
    for mut b in blocks {
        if b["type"] == "image" {
            let n = b["data"].as_str().map_or(0, str::len);
            if n <= left {
                left -= n;
                out.push(b);
                continue;
            }
            let mime = b["mime"].as_str().unwrap_or("");
            b = text_block(format!("[image {mime}, {n} bytes, not shown]"));
        }
        let text = b["text"].as_str().unwrap_or("");
        let n = escaped_len(text, 1);
        if n <= left {
            left -= n;
            out.push(b);
        } else {
            let cut = format!("{}\n{cut_note}", cut_prefix(text, left));
            out.push(text_block(cut));
            break;
        }
    }
    out
}

pub fn tool_result(result: &Value, ctx: &BlockCtx) -> Result<Value, String> {
    let items = result
        .get("content")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    if result["isError"] == true {
        let texts: Vec<&str> = items
            .iter()
            .filter(|i| str_field(i, "type") == Some("text"))
            .filter_map(|i| str_field(i, "text"))
            .collect();
        if texts.is_empty() {
            return Err("the tool reported an error without a message".to_string());
        }
        return Err(cut_prefix(&texts.join("\n"), CAP_BYTES).to_string());
    }
    let mut blocks: Vec<Value> = items.iter().map(|i| block(i, ctx)).collect();
    if blocks.is_empty()
        && let Some(sc) = result.get("structuredContent")
    {
        blocks.push(text_block(sc.to_string()));
    }
    Ok(json!({"content": cap_blocks(blocks, TOOL_CUT_HINT)}))
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallFailure {
    JsonRpc { code: i32, message: String },
    Timeout { secs: u64 },
    RateLimited { retry_after: Option<String> },
    InsufficientScope { scope: Option<String> },
    CredentialRejected { reference: String },
    LoginRequired,
    Unavailable { state: &'static str, for_secs: u64 },
    Transport(String),
}

pub fn failure_text(server: &str, failure: &CallFailure) -> String {
    match failure {
        CallFailure::JsonRpc {
            code: -32602,
            message,
        } => format!("invalid_argument: {message}"),
        CallFailure::JsonRpc { code, message } => format!("MCP error {code}: {message}"),
        CallFailure::Timeout { secs } => format!(
            "no answer from {server} after {secs} s. outcome unknown: check before retrying a write"
        ),
        CallFailure::RateLimited { retry_after } => format!(
            "rate limited by {server}; retry after {}",
            retry_after.as_deref().unwrap_or("an unknown time")
        ),
        CallFailure::InsufficientScope { scope: Some(s) } => {
            format!("insufficient scope: {server} requires scope `{s}`")
        }
        CallFailure::InsufficientScope { scope: None } => {
            format!("insufficient scope: {server} refused the call")
        }
        CallFailure::CredentialRejected { reference } => {
            format!("credential rejected, an operator must fix `{reference}`")
        }
        CallFailure::LoginRequired => {
            "sign-in required, an operator must sign in on the MCP servers page".to_string()
        }
        CallFailure::Unavailable { state, for_secs } => {
            format!("`{server}` MCP unavailable (`{state}`), for {for_secs} s")
        }
        CallFailure::Transport(t) => format!("{server} MCP request failed: {t}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> BlockCtx<'static> {
        BlockCtx {
            server: "s",
            read_registered: false,
            asked_uri: None,
            many: false,
        }
    }
    fn text(t: &str) -> Value {
        json!({"type": "text", "text": t})
    }

    /// Mirrors `harness/src/call.rs` result sizing: content and details both count.
    fn harness_bytes(result: &Value) -> usize {
        let content = result["content"].as_array().filter(|c| !c.is_empty());
        match content {
            Some(c) => serde_json::to_vec(&(c, result)).unwrap().len(),
            None => {
                let c = json!([{"type": "text", "text": serde_json::to_string(result).unwrap()}]);
                serde_json::to_vec(&(c, result)).unwrap().len()
            }
        }
    }

    #[test]
    fn strip_args_drops_undeclared_underscore_keys_at_the_top_level_only() {
        let declared: BTreeSet<String> = ["query", "_keep"].map(String::from).into();
        let out = strip_args(
            json!({"_caller_worker_id": "w", "_keep": 1, "query": "x", "filter": {"_x": 1}, "extra": true}),
            &declared,
        );
        assert_eq!(
            Value::Object(out),
            json!({"_keep": 1, "query": "x", "filter": {"_x": 1}, "extra": true})
        );
        for p in [json!(null), json!("text"), json!([1]), json!(7)] {
            assert!(strip_args(p, &declared).is_empty());
        }
    }

    #[test]
    fn block_translates_each_row_of_the_table() {
        let c = ctx();
        assert_eq!(
            block(
                &json!({"type": "text", "text": "hi", "annotations": {"priority": 1}}),
                &c
            ),
            text("hi")
        );
        assert_eq!(
            block(
                &json!({"type": "image", "data": "QUJD", "mimeType": "image/png"}),
                &c
            ),
            json!({"type": "image", "mime": "image/png", "data": "QUJD"})
        );
        assert_eq!(
            block(
                &json!({"type": "audio", "data": "QUJDRA==", "mimeType": "audio/wav"}),
                &c
            ),
            text("[audio audio/wav, 8 bytes, not shown]")
        );
        assert_eq!(
            block(
                &json!({"type": "resource", "resource": {"uri": "file:///a.txt", "mimeType": "text/plain", "text": "alpha"}}),
                &c
            ),
            text("[file:///a.txt]\nalpha")
        );
        assert_eq!(
            block(
                &json!({"type": "resource", "resource": {"uri": "file:///b.dat", "blob": "QUJD"}}),
                &c
            ),
            text("[binary resource file:///b.dat, application/octet-stream, 4 bytes, not shown]")
        );
        assert_eq!(
            block(
                &json!({"type": "resource", "resource": {"uri": "file:///i.png", "mimeType": "image/png", "blob": "QUJD"}}),
                &c
            ),
            json!({"type": "image", "mime": "image/png", "data": "QUJD"})
        );
        let mystery = json!({"type": "mystery", "x": 1});
        assert_eq!(block(&mystery, &c), text(&mystery.to_string()));
        let bare = json!({"type": "image", "data": "QUJD"});
        assert_eq!(block(&bare, &c), text(&bare.to_string()));
    }

    #[test]
    fn a_read_item_gets_the_uri_line_only_when_needed() {
        let c = ctx();
        let item = json!({"uri": "file:///a.txt", "mimeType": "text/plain", "text": "alpha"});
        let asked = BlockCtx {
            asked_uri: Some("file:///a.txt"),
            ..c
        };
        assert_eq!(block(&item, &asked), text("alpha"));
        assert_eq!(
            block(
                &item,
                &BlockCtx {
                    many: true,
                    ..asked
                }
            ),
            text("[file:///a.txt]\nalpha")
        );
        assert_eq!(
            block(
                &item,
                &BlockCtx {
                    asked_uri: Some("file:///other"),
                    ..c
                }
            ),
            text("[file:///a.txt]\nalpha")
        );
    }

    #[test]
    fn resource_link_names_the_read_function_only_when_it_exists() {
        let link = json!({"type": "resource_link", "uri": "file:///a.txt", "name": "a.txt", "description": "First file"});
        assert_eq!(
            block(&link, &ctx()),
            text("a.txt: file:///a.txt\nFirst file")
        );
        assert_eq!(
            block(
                &link,
                &BlockCtx {
                    read_registered: true,
                    ..ctx()
                }
            ),
            text("a.txt: file:///a.txt\nFirst file\nread it with `mcp::s::resources::read`")
        );
    }

    #[test]
    fn cap_cuts_on_a_char_boundary() {
        let long = format!("a{}", "😀".repeat(CAP_BYTES / 4));
        let out = cap_blocks(vec![text(&long), text("dropped")], TOOL_CUT_HINT);
        assert_eq!(out.len(), 1);
        let cut = out[0]["text"].as_str().unwrap();
        assert!(cut.starts_with("a😀") && cut.ends_with(TOOL_CUT_HINT));
        assert_eq!(cut.len(), CAP_BYTES - 3 + "\n".len() + TOOL_CUT_HINT.len());
    }

    #[test]
    fn cap_keeps_what_fits_and_turns_a_large_image_into_a_note() {
        let exact = "x".repeat(CAP_BYTES);
        assert_eq!(cap_blocks(vec![text(&exact)], "n"), vec![text(&exact)]);
        let img = json!({"type": "image", "mime": "image/png", "data": "A".repeat(CAP_BYTES)});
        assert_eq!(
            cap_blocks(vec![text("ok"), img, text("after")], "n"),
            vec![
                text("ok"),
                text("[image image/png, 122880 bytes, not shown]"),
                text("after")
            ]
        );
        let out = cap_blocks(
            vec![text(&"x".repeat(CAP_BYTES + 1))],
            "resource cut at 120 KiB",
        );
        assert!(
            out[0]["text"]
                .as_str()
                .unwrap()
                .ends_with("x\nresource cut at 120 KiB")
        );
    }

    #[test]
    fn tool_result_is_one_content_shape_or_an_error() {
        let c = ctx();
        assert_eq!(
            tool_result(
                &json!({"content": [text("hi")], "structuredContent": {"a": 1}}),
                &c
            ),
            Ok(json!({"content": [text("hi")]}))
        );
        assert_eq!(
            tool_result(&json!({"content": [], "structuredContent": {"a": 1}}), &c),
            Ok(json!({"content": [text("{\"a\":1}")]}))
        );
        assert_eq!(tool_result(&json!({}), &c), Ok(json!({"content": []})));
        assert_eq!(
            tool_result(
                &json!({"content": [text("boom"), {"type": "image", "data": "QUJD", "mimeType": "image/png"}, text("line 2")], "isError": true}),
                &c
            ),
            Err("boom\nline 2".to_string())
        );
        assert_eq!(
            tool_result(&json!({"content": [], "isError": true}), &c),
            Err("the tool reported an error without a message".to_string())
        );
        let e = format!("a{}", "é".repeat(CAP_BYTES));
        let err = tool_result(&json!({"content": [text(&e)], "isError": true}), &c).unwrap_err();
        assert_eq!(err.len(), CAP_BYTES - 1);
        let ok = tool_result(&json!({"content": [text(&"x".repeat(CAP_BYTES + 5))]}), &c).unwrap();
        assert!(
            ok["content"][0]["text"]
                .as_str()
                .unwrap()
                .ends_with(TOOL_CUT_HINT)
        );
    }

    #[test]
    fn failure_texts() {
        use CallFailure::*;
        let t = |f| failure_text("linear", &f);
        assert_eq!(
            t(JsonRpc {
                code: -32602,
                message: "bad".into()
            }),
            "invalid_argument: bad"
        );
        assert_eq!(
            t(JsonRpc {
                code: -32000,
                message: "boom".into()
            }),
            "MCP error -32000: boom"
        );
        assert_eq!(
            t(Timeout { secs: 30 }),
            "no answer from linear after 30 s. outcome unknown: check before retrying a write"
        );
        assert_eq!(
            t(RateLimited {
                retry_after: Some("5".into())
            }),
            "rate limited by linear; retry after 5"
        );
        assert_eq!(
            t(RateLimited { retry_after: None }),
            "rate limited by linear; retry after an unknown time"
        );
        assert_eq!(
            t(InsufficientScope {
                scope: Some("write".into())
            }),
            "insufficient scope: linear requires scope `write`"
        );
        assert_eq!(
            t(InsufficientScope { scope: None }),
            "insufficient scope: linear refused the call"
        );
        assert_eq!(
            t(CredentialRejected {
                reference: "env:K".into()
            }),
            "credential rejected, an operator must fix `env:K`"
        );
        assert_eq!(
            t(LoginRequired),
            "sign-in required, an operator must sign in on the MCP servers page"
        );
        assert_eq!(
            t(Unavailable {
                state: "unavailable",
                for_secs: 42
            }),
            "`linear` MCP unavailable (`unavailable`), for 42 s"
        );
        assert_eq!(t(Transport("eof".into())), "linear MCP request failed: eof");
    }

    #[test]
    fn caps_hold_under_the_harness_measure() {
        let raw: String = "\"\\\n".chars().cycle().take(200_000).collect();
        let r = tool_result(&json!({"content": [text(&raw)]}), &ctx()).unwrap();
        assert!(harness_bytes(&r) <= 262_144, "{}", harness_bytes(&r));
        assert!(
            r["content"][0]["text"]
                .as_str()
                .unwrap()
                .ends_with(TOOL_CUT_HINT)
        );
    }
}
