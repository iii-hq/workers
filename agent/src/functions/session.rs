use std::future::Future;
use std::pin::Pin;

use iii_sdk::{IIIError, III};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::state;

pub fn build_create_handler(
    iii: III,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |_payload: Value| {
        let iii = iii.clone();

        Box::pin(async move {
            let session_id = Uuid::new_v4().to_string();
            let now = chrono::Utc::now().to_rfc3339();

            let session_data = json!({
                "created_at": now,
                "messages": []
            });

            state::state_set(&iii, "agent:sessions", &session_id, &session_data)
                .await
                .map_err(|e| IIIError::Handler(format!("failed to create session: {}", e)))?;

            Ok(json!({
                "session_id": session_id,
                "created_at": now
            }))
        })
    }
}

pub fn build_history_handler(
    iii: III,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |payload: Value| {
        let iii = iii.clone();

        Box::pin(async move {
            let session_id = payload
                .get("session_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| IIIError::Handler("missing 'session_id' field".to_string()))?
                .to_string();

            let result = state::state_get(&iii, "agent:sessions", &session_id).await;

            match result {
                Ok(val) => Ok(json!({
                    "session_id": session_id,
                    "history": val.get("value").cloned().unwrap_or(json!(null))
                })),
                Err(_) => Ok(json!({
                    "session_id": session_id,
                    "history": null,
                    "error": "session not found"
                })),
            }
        })
    }
}

pub fn build_cleanup_handler(
    iii: III,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |_payload: Value| {
        let iii = iii.clone();

        Box::pin(async move {
            let sessions = state::state_list(&iii, "agent:sessions").await;

            let mut cleaned = 0u64;

            if let Ok(list) = sessions {
                if let Some(keys) = list.get("keys").and_then(|v| v.as_array()) {
                    let now = chrono::Utc::now();

                    for key_val in keys {
                        if let Some(key) = key_val.as_str() {
                            if let Ok(session) =
                                state::state_get(&iii, "agent:sessions", key).await
                            {
                                let should_delete = session
                                    .get("value")
                                    .and_then(|v| v.get("created_at"))
                                    .and_then(|v| v.as_str())
                                    .and_then(|ts| {
                                        chrono::DateTime::parse_from_rfc3339(ts).ok()
                                    })
                                    .map(|created| {
                                        let age = now
                                            .signed_duration_since(created.with_timezone(&chrono::Utc));
                                        age.num_hours() > 24
                                    })
                                    .unwrap_or(false);

                                if should_delete {
                                    let _ = state::state_delete(
                                        &iii,
                                        "agent:sessions",
                                        key,
                                    )
                                    .await;
                                    cleaned += 1;
                                }
                            }
                        }
                    }
                }
            }

            Ok(json!({
                "cleaned_sessions": cleaned
            }))
        })
    }
}
