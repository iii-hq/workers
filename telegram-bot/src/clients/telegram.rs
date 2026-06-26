use std::time::Duration;

use iii_sdk::errors::Error;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::deps::Deps;
use crate::types::TelegramUpdate;

const API_BASE: &str = "https://api.telegram.org/bot";

pub async fn send_message(
    deps: &Deps,
    chat_id: i64,
    text: &str,
    reply_markup: Option<Value>,
    parse_mode: Option<&str>,
) -> Result<i64, Error> {
    let mut body = json!({
        "chat_id": chat_id,
        "text": text,
    });
    if let Some(markup) = reply_markup {
        body["reply_markup"] = markup;
    }
    if let Some(mode) = parse_mode {
        body["parse_mode"] = json!(mode);
    }
    let message_id = api_call(deps, "sendMessage", body).await?;
    message_id
        .get("message_id")
        .and_then(Value::as_i64)
        .ok_or_else(|| Error::Handler("telegram sendMessage: missing message_id".into()))
}

pub async fn edit_message_text(
    deps: &Deps,
    chat_id: i64,
    message_id: i64,
    text: &str,
    parse_mode: Option<&str>,
) -> Result<(), Error> {
    let mut body = json!({
        "chat_id": chat_id,
        "message_id": message_id,
        "text": text,
    });
    if let Some(mode) = parse_mode {
        body["parse_mode"] = json!(mode);
    }
    api_call(deps, "editMessageText", body).await?;
    Ok(())
}

pub async fn edit_message_reply_markup(
    deps: &Deps,
    chat_id: i64,
    message_id: i64,
) -> Result<(), Error> {
    let body = json!({
        "chat_id": chat_id,
        "message_id": message_id,
        "reply_markup": json!({ "inline_keyboard": [] }),
    });
    api_call(deps, "editMessageReplyMarkup", body).await?;
    Ok(())
}

pub async fn send_message_draft(
    deps: &Deps,
    chat_id: i64,
    draft_id: i32,
    text: &str,
    message_thread_id: Option<i64>,
) -> Result<(), Error> {
    let mut body = json!({
        "chat_id": chat_id,
        "draft_id": draft_id,
        "text": text,
    });
    if let Some(thread) = message_thread_id {
        body["message_thread_id"] = json!(thread);
    }
    api_call(deps, "sendMessageDraft", body).await?;
    Ok(())
}

pub async fn send_rich_message_draft(
    deps: &Deps,
    chat_id: i64,
    draft_id: i32,
    rich_message: &Value,
    message_thread_id: Option<i64>,
) -> Result<(), Error> {
    let mut body = json!({
        "chat_id": chat_id,
        "draft_id": draft_id,
        "rich_message": rich_message,
    });
    if let Some(thread) = message_thread_id {
        body["message_thread_id"] = json!(thread);
    }
    api_call(deps, "sendRichMessageDraft", body).await?;
    Ok(())
}

pub async fn send_rich_message(
    deps: &Deps,
    chat_id: i64,
    rich_message: &Value,
    message_thread_id: Option<i64>,
) -> Result<i64, Error> {
    let mut body = json!({
        "chat_id": chat_id,
        "rich_message": rich_message,
    });
    if let Some(thread) = message_thread_id {
        body["message_thread_id"] = json!(thread);
    }
    let result = api_call(deps, "sendRichMessage", body).await?;
    result
        .get("message_id")
        .and_then(Value::as_i64)
        .ok_or_else(|| Error::Handler("telegram sendRichMessage: missing message_id".into()))
}

pub fn rich_thinking_draft(thinking_text: &str) -> Value {
    let content = if thinking_text.is_empty() {
        "Thinking…"
    } else {
        thinking_text
    };
    let escaped = escape_rich_html(content);
    json!({
        "html": format!("<tg-thinking>{escaped}</tg-thinking>")
    })
}

fn escape_rich_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

pub async fn answer_callback_query(
    deps: &Deps,
    callback_id: &str,
    text: Option<&str>,
) -> Result<(), Error> {
    let mut body = json!({ "callback_query_id": callback_id });
    if let Some(t) = text {
        body["text"] = json!(t);
    }
    api_call(deps, "answerCallbackQuery", body).await?;
    Ok(())
}

pub async fn set_my_commands(deps: &Deps) -> Result<(), Error> {
    let commands = json!([
        { "command": "start", "description": "Start or pick a model" },
        { "command": "stop", "description": "Stop the current turn" },
        { "command": "model", "description": "Change model" },
        { "command": "help", "description": "Show available commands" },
        { "command": "thinking", "description": "Set reasoning depth" },
        { "command": "verbosity", "description": "Set transcript verbosity" },
        { "command": "settings", "description": "Show current bot settings" },
    ]);
    api_call(deps, "setMyCommands", json!({ "commands": commands })).await?;
    Ok(())
}

pub async fn set_webhook(deps: &Deps, url: &str, secret_token: Option<&str>) -> Result<(), Error> {
    let mut body = json!({ "url": url });
    if let Some(secret) = secret_token {
        body["secret_token"] = json!(secret);
    }
    api_call(deps, "setWebhook", body).await?;
    Ok(())
}

pub async fn delete_webhook(deps: &Deps) -> Result<(), Error> {
    api_call(deps, "deleteWebhook", json!({})).await?;
    Ok(())
}

pub async fn get_file(deps: &Deps, file_id: &str) -> Result<Value, Error> {
    api_call(deps, "getFile", json!({ "file_id": file_id })).await
}

pub async fn get_updates(
    deps: &Deps,
    offset: i64,
    timeout_secs: u64,
) -> Result<Vec<TelegramUpdate>, Error> {
    get_updates_with_cancel(deps, offset, timeout_secs, None).await
}

pub async fn get_updates_with_cancel(
    deps: &Deps,
    offset: i64,
    timeout_secs: u64,
    cancel: Option<&CancellationToken>,
) -> Result<Vec<TelegramUpdate>, Error> {
    let body = json!({
        "offset": offset,
        "timeout": timeout_secs,
        "allowed_updates": ["message", "callback_query"],
    });
    let timeout = Duration::from_secs(timeout_secs.saturating_add(15));
    let result = if let Some(cancel) = cancel {
        let mut call = Box::pin(api_call_with_timeout(deps, "getUpdates", body, timeout));
        tokio::select! {
            _ = cancel.cancelled() => {
                return Err(Error::Handler("getUpdates cancelled".into()));
            }
            result = &mut call => result?,
        }
    } else {
        api_call_with_timeout(deps, "getUpdates", body, timeout).await?
    };
    let updates = result
        .as_array()
        .ok_or_else(|| Error::Handler("telegram getUpdates: expected array".into()))?;
    let mut out = Vec::with_capacity(updates.len());
    for item in updates {
        let update: TelegramUpdate = serde_json::from_value(item.clone())
            .map_err(|e| Error::Handler(format!("telegram getUpdates parse: {e}")))?;
        out.push(update);
    }
    Ok(out)
}

async fn api_call(deps: &Deps, method: &str, body: Value) -> Result<Value, Error> {
    api_call_with_timeout(deps, method, body, Duration::from_secs(30)).await
}

async fn api_call_with_timeout(
    deps: &Deps,
    method: &str,
    body: Value,
    timeout: Duration,
) -> Result<Value, Error> {
    let cfg = deps.cfg().await;
    let token = cfg.bot_token.clone();
    let url = format!("{API_BASE}{token}/{method}");
    let resp = deps
        .runtime
        .http
        .post(&url)
        .timeout(timeout)
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Handler(format!("telegram http: {}", e.without_url())))?;
    let payload: Value = resp
        .json()
        .await
        .map_err(|e| Error::Handler(format!("telegram json: {e}")))?;
    if payload.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(Error::Handler(format!(
            "telegram {method} failed: {payload}"
        )));
    }
    Ok(payload.get("result").cloned().unwrap_or(Value::Null))
}

pub fn inline_keyboard(rows: Vec<Vec<(String, String)>>) -> Value {
    let keyboard: Vec<Vec<Value>> = rows
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|(text, data)| json!({ "text": text, "callback_data": data }))
                .collect()
        })
        .collect();
    json!({ "inline_keyboard": keyboard })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rich_thinking_draft_uses_input_rich_message_html() {
        let draft = rich_thinking_draft("");
        let html = draft.get("html").and_then(Value::as_str).unwrap();
        assert!(html.contains("<tg-thinking>"));
        assert!(html.contains("Thinking…"));
        assert!(html.contains("</tg-thinking>"));
        assert!(draft.get("blocks").is_none());
    }

    #[test]
    fn rich_thinking_draft_escapes_html_in_content() {
        let draft = rich_thinking_draft("a < b & c");
        let html = draft.get("html").and_then(Value::as_str).unwrap();
        assert!(html.contains("a &lt; b &amp; c"));
    }
}
