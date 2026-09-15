pub mod source;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use source::{ConversationSummary, ExternalMessage, Source, Transcript};

#[derive(Deserialize, JsonSchema)]
pub struct DiscoverRequest {
    pub source: Source,
    pub query: Option<String>,
    pub cursor: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct Selection {
    pub source: Source,
    pub id: String,
}

#[derive(Serialize, JsonSchema)]
pub struct Preview {
    pub conversation: ConversationSummary,
    pub messages: Vec<ExternalMessage>,
    pub warnings: Vec<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct ImportResult {
    pub session_id: String,
    pub imported_messages: usize,
    pub total_messages: usize,
}

fn handler_error(error: impl std::fmt::Display) -> Error {
    Error::Handler(error.to_string())
}

pub fn register(iii: &Arc<IIIClient>) {
    iii.register_function(
        "console::conversations::discover",
        RegisterFunction::new_async(|req: DiscoverRequest| async move {
            tokio::task::spawn_blocking(move || {
                source::discover(req.source, req.query.as_deref(), req.cursor.as_deref())
            })
            .await
            .map_err(handler_error)?
            .map_err(handler_error)
        })
        .description("Find selectable Codex or Claude Code histories on the ADE host.")
        .metadata(json!({"internal":true})),
    );

    iii.register_function("console::conversations::preview", RegisterFunction::new_async(|req: Selection| async move {
        let mut transcript = tokio::task::spawn_blocking(move || source::read(req.source, &req.id))
            .await.map_err(handler_error)?.map_err(handler_error)?;
        if transcript.messages.len() > 50 {
            transcript.warnings.push("Preview shows the last 50 text messages; import includes the full text history.".into());
            transcript.messages = transcript.messages.split_off(transcript.messages.len() - 50);
        }
        Ok::<_, Error>(Preview { conversation: transcript.conversation, messages: transcript.messages, warnings: transcript.warnings })
    }).description("Preview text from a selected local conversation without importing or running it.")
        .metadata(json!({"internal":true})));

    let client = iii.clone();
    iii.register_function(
        "console::conversations::import",
        RegisterFunction::new_async(move |req: Selection| {
            let client = client.clone();
            async move {
                let transcript =
                    tokio::task::spawn_blocking(move || source::read(req.source, &req.id))
                        .await
                        .map_err(handler_error)?
                        .map_err(handler_error)?;
                import_transcript(&client, transcript)
                    .await
                    .map_err(handler_error)
            }
        })
        .description("Create a new editable ADE conversation from a local text history.")
        .metadata(json!({"internal":true})),
    );
}

async fn rpc(iii: &IIIClient, function_id: &str, payload: Value) -> Result<Value> {
    iii.trigger(TriggerRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(30_000),
    })
    .await
    .map_err(Into::into)
}

async fn import_transcript(iii: &IIIClient, transcript: Transcript) -> Result<ImportResult> {
    let source = &transcript.conversation;
    let messages: Vec<_> = transcript.messages.iter().map(|message| {
        let mut body = json!({"role":message.role,"content":[{"type":"text","text":message.text}],"timestamp":message.timestamp});
        if message.role == "assistant" {
            body["model"] = json!(message.model.as_deref().unwrap_or(""));
            body["provider"] = json!(message.provider.as_deref().unwrap_or(""));
            body["agent"] = json!(source.source.as_str());
            body["stop_reason"] = json!(match message.native_stop_reason.as_deref() {
                Some("max_tokens" | "length") => "length",
                Some("tool_use" | "function_call") => "function_call",
                Some("aborted") => "aborted",
                Some("error") => "error",
                _ => "end",
            });
            body["native_stop_reason"] = json!(message.native_stop_reason);
        }
        body
    }).collect();
    let count = messages.len();
    let created = rpc(
        iii,
        "session::create",
        json!({
            "title": source.title,
            "metadata": {
                "surface": "console",
                "external_source": source.source,
                "external_session_id": source.id,
                "source_cwd": source.cwd,
                "imported_at": SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as i64,
            },
        }),
    )
    .await?;
    let session_id = created["session_id"]
        .as_str()
        .context("Created session ID is missing")?
        .to_owned();
    if let Err(error) = rpc(
        iii,
        "session::append-many",
        json!({
            "session_id": session_id,
            "messages": messages,
        }),
    )
    .await
    {
        rpc(iii, "session::delete", json!({"session_id": session_id}))
            .await
            .with_context(|| {
                format!("Import failed ({error}); could not remove incomplete session {session_id}")
            })?;
        return Err(error);
    }
    Ok(ImportResult {
        session_id,
        imported_messages: count,
        total_messages: count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_accept_engine_caller_metadata() {
        let request: DiscoverRequest = serde_json::from_value(json!({
            "source":"codex", "_caller_worker_id":"console-client"
        }))
        .unwrap();
        assert_eq!(request.source, Source::Codex);
        let selection: Selection = serde_json::from_value(json!({
            "source":"claude-code", "id":"external-id", "_caller_worker_id":"console-client"
        }))
        .unwrap();
        assert_eq!(selection.id, "external-id");
    }

    #[tokio::test]
    #[ignore = "requires ADE_IMPORT_TEST_ENGINE_URL pointing at an isolated iii engine"]
    async fn imports_create_independent_native_sessions_and_remove_failed_copies() {
        use std::collections::HashMap;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Mutex;
        use std::time::Duration;

        let url = std::env::var("ADE_IMPORT_TEST_ENGINE_URL").expect("isolated engine URL");
        let iii = iii_sdk::register_worker(
            &url,
            iii_sdk::InitOptions {
                namespace: Some(format!("native-import-test-{}", uuid::Uuid::new_v4())),
                ..Default::default()
            },
        );
        let sessions = Arc::new(Mutex::new(HashMap::<String, Value>::new()));
        let fail = Arc::new(AtomicBool::new(false));
        let stored = sessions.clone();
        iii.register_function(
            "session::create",
            RegisterFunction::new(move |request: Value| {
                let id = uuid::Uuid::new_v4().to_string();
                stored.lock().unwrap().insert(id.clone(), request);
                Ok::<_, Error>(json!({"session_id": id}))
            }),
        );
        let stored = sessions.clone();
        let fail_append = fail.clone();
        iii.register_function(
            "session::append-many",
            RegisterFunction::new(move |request: Value| {
                if fail_append.load(Ordering::SeqCst) {
                    return Err(Error::Handler("append failed".into()));
                }
                stored
                    .lock()
                    .unwrap()
                    .get_mut(request["session_id"].as_str().unwrap())
                    .unwrap()["messages"] = request["messages"].clone();
                Ok(json!({}))
            }),
        );
        let stored = sessions.clone();
        iii.register_function(
            "session::delete",
            RegisterFunction::new(move |request: Value| {
                stored
                    .lock()
                    .unwrap()
                    .remove(request["session_id"].as_str().unwrap());
                Ok::<_, Error>(json!({"deleted":true}))
            }),
        );
        iii.wait_until_registered(Duration::from_secs(5))
            .await
            .unwrap();
        let history = || Transcript {
            conversation: ConversationSummary {
                id: "external-session".into(),
                source: Source::Codex,
                title: "Imported context".into(),
                cwd: Some("/external/project".into()),
                created_at: 1,
                updated_at: 2,
            },
            messages: vec![ExternalMessage {
                id: "original-message".into(),
                role: "user".into(),
                text: "context".into(),
                timestamp: 1,
                model: None,
                provider: None,
                native_stop_reason: None,
            }],
            warnings: vec![],
        };
        let first = import_transcript(&iii, history()).await.unwrap();
        let second = import_transcript(&iii, history()).await.unwrap();
        assert_ne!(first.session_id, second.session_id);
        assert_eq!(second.imported_messages, 1);
        {
            let stored = sessions.lock().unwrap();
            let meta = &stored[&first.session_id]["metadata"];
            assert_eq!(meta["external_source"], "codex");
            assert_eq!(meta["external_session_id"], "external-session");
            assert_eq!(meta["source_cwd"], "/external/project");
            assert!(meta["imported_at"].is_i64());
            assert!(meta.get("read_only").is_none());
            assert!(meta.get("fs_scope").is_none());
            assert!(meta.get("model").is_none());
            assert_eq!(
                stored[&first.session_id]["messages"],
                json!([{
                    "role":"user", "timestamp":1, "content":[{"type":"text","text":"context"}]
                }])
            );
        }
        fail.store(true, Ordering::SeqCst);
        assert!(import_transcript(&iii, history()).await.is_err());
        assert_eq!(sessions.lock().unwrap().len(), 2);
        iii.shutdown_async().await;
    }
}
