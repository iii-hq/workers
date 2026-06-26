//! `slack::files::*`. Upload uses the current 3-step external flow
//! (`files.upload` was sunset 2025-11-12).

use std::sync::Arc;

use base64::Engine;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::clients::slack;
use crate::deps::Deps;
use crate::slack_method;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct InfoReq {
    pub file: String,
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct ListReq {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

slack_method!(
    info,
    "slack::files::info",
    "files.info",
    "Get metadata for a file.",
    InfoReq
);
slack_method!(
    list,
    "slack::files::list",
    "files.list",
    "List files visible to the bot.",
    ListReq
);

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UploadReq {
    pub filename: String,
    /// UTF-8 text content. Provide this OR `content_base64`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Base64-encoded binary content. Provide this OR `content`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_base64: Option<String>,
    /// Channel id to share the file into.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_ts: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UploadResponse {
    pub ok: bool,
    pub file_id: String,
}

async fn upload(deps: &Deps, req: UploadReq) -> Result<UploadResponse, Error> {
    let bytes: Vec<u8> = match (req.content, req.content_base64) {
        (Some(text), None) => text.into_bytes(),
        (None, Some(b64)) => base64::engine::general_purpose::STANDARD
            .decode(b64.as_bytes())
            .map_err(|e| Error::Handler(format!("files.upload: invalid content_base64: {e}")))?,
        (Some(_), Some(_)) => {
            return Err(Error::Handler(
                "files.upload: provide content OR content_base64, not both".into(),
            ))
        }
        (None, None) => {
            return Err(Error::Handler(
                "files.upload: content or content_base64 is required".into(),
            ))
        }
    };

    // 1. reserve an upload URL
    let reserve = slack::call(
        deps,
        "files.getUploadURLExternal",
        json!({ "filename": req.filename, "length": bytes.len() }),
    )
    .await?;
    let upload_url = reserve
        .get("upload_url")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Handler("files.upload: missing upload_url".into()))?;
    let file_id = reserve
        .get("file_id")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Handler("files.upload: missing file_id".into()))?
        .to_string();

    // 2. PUT the bytes to the reserved URL
    let put = deps
        .http
        .post(upload_url)
        .body(bytes)
        .send()
        .await
        .map_err(|e| Error::Handler(format!("files.upload PUT: {}", e.without_url())))?;
    if !put.status().is_success() {
        return Err(Error::Handler(format!(
            "files.upload: upload returned HTTP {}",
            put.status()
        )));
    }

    // 3. finalize
    let mut file_entry = Map::new();
    file_entry.insert("id".into(), json!(file_id));
    if let Some(t) = &req.title {
        file_entry.insert("title".into(), json!(t));
    }
    let mut complete = Map::new();
    complete.insert("files".into(), json!([Value::Object(file_entry)]));
    if let Some(c) = req.channel_id {
        complete.insert("channel_id".into(), json!(c));
    }
    if let Some(c) = req.initial_comment {
        complete.insert("initial_comment".into(), json!(c));
    }
    if let Some(t) = req.thread_ts {
        complete.insert("thread_ts".into(), json!(t));
    }
    slack::call(
        deps,
        "files.completeUploadExternal",
        Value::Object(complete),
    )
    .await?;

    Ok(UploadResponse { ok: true, file_id })
}

pub fn register(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    info(iii, deps);
    list(iii, deps);
    super::register(
        iii,
        deps,
        "slack::files::upload",
        "Upload a file (reserve URL, upload bytes, finalize) and optionally share it to a channel.",
        |d, req: UploadReq| async move { upload(&d, req).await },
    );
}
