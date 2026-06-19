//! Chat ↔ session KV helpers.

use iii_sdk::IIIError;
use serde_json::json;

use crate::clients::state;
use crate::deps::{Deps, STATE_SCOPE};
use crate::types::ChatFsm;

pub async fn chat_session(deps: &Deps, chat_id: i64) -> Option<String> {
    let key = format!("chat:{chat_id}:session");
    state::get_string(&deps.iii, STATE_SCOPE, &key, deps.cfg().await.timeout_ms).await
}

pub async fn set_chat_session(deps: &Deps, chat_id: i64, session_id: &str) -> Result<(), IIIError> {
    let timeout = deps.cfg().await.timeout_ms;
    let chat_key = format!("chat:{chat_id}:session");
    state::set(
        &deps.iii,
        STATE_SCOPE,
        &chat_key,
        json!(session_id),
        Some(timeout),
    )
    .await?;
    if let Err(e) = state::set(
        &deps.iii,
        STATE_SCOPE,
        &format!("session:{session_id}:chat"),
        json!(chat_id),
        Some(timeout),
    )
    .await
    {
        let _ = state::delete(&deps.iii, STATE_SCOPE, &chat_key, Some(timeout)).await;
        return Err(e);
    }
    Ok(())
}

pub async fn chat_id_for_session(deps: &Deps, session_id: &str) -> Option<i64> {
    state::get_i64(
        &deps.iii,
        STATE_SCOPE,
        &format!("session:{session_id}:chat"),
        deps.cfg().await.timeout_ms,
    )
    .await
}

pub async fn entry_message_id(deps: &Deps, session_id: &str, entry_id: &str) -> Option<i64> {
    state::get_i64(
        &deps.iii,
        STATE_SCOPE,
        &format!("entry:{session_id}:{entry_id}:msg"),
        deps.cfg().await.timeout_ms,
    )
    .await
}

pub async fn set_entry_message_id(
    deps: &Deps,
    session_id: &str,
    entry_id: &str,
    message_id: i64,
) -> Result<(), IIIError> {
    state::set(
        &deps.iii,
        STATE_SCOPE,
        &format!("entry:{session_id}:{entry_id}:msg"),
        json!(message_id),
        Some(deps.cfg().await.timeout_ms),
    )
    .await?;
    Ok(())
}

pub async fn entry_chunk_message_id(
    deps: &Deps,
    session_id: &str,
    entry_id: &str,
    chunk_idx: u32,
) -> Option<i64> {
    state::get_i64(
        &deps.iii,
        STATE_SCOPE,
        &format!("entry:{session_id}:{entry_id}:chunk:{chunk_idx}:msg"),
        deps.cfg().await.timeout_ms,
    )
    .await
}

pub async fn set_entry_chunk_message_id(
    deps: &Deps,
    session_id: &str,
    entry_id: &str,
    chunk_idx: u32,
    message_id: i64,
) -> Result<(), IIIError> {
    state::set(
        &deps.iii,
        STATE_SCOPE,
        &format!("entry:{session_id}:{entry_id}:chunk:{chunk_idx}:msg"),
        json!(message_id),
        Some(deps.cfg().await.timeout_ms),
    )
    .await?;
    Ok(())
}

/// Per-entry ordering key (append time in ms). Survives reconstruction so
/// finalize can post entries in append order even across a restart.
pub async fn entry_order(deps: &Deps, session_id: &str, entry_id: &str) -> Option<i64> {
    state::get_i64(
        &deps.iii,
        STATE_SCOPE,
        &format!("entry:{session_id}:{entry_id}:order"),
        deps.cfg().await.timeout_ms,
    )
    .await
}

pub async fn set_entry_order(
    deps: &Deps,
    session_id: &str,
    entry_id: &str,
    order_key: i64,
) -> Result<(), IIIError> {
    state::set(
        &deps.iii,
        STATE_SCOPE,
        &format!("entry:{session_id}:{entry_id}:order"),
        json!(order_key),
        Some(deps.cfg().await.timeout_ms),
    )
    .await?;
    Ok(())
}

pub async fn is_entry_finalized(deps: &Deps, session_id: &str, entry_id: &str) -> bool {
    state::get(
        &deps.iii,
        STATE_SCOPE,
        &format!("entry:{session_id}:{entry_id}:finalized"),
        Some(deps.cfg().await.timeout_ms),
    )
    .await
    .ok()
    .and_then(|v| v.as_bool())
    .unwrap_or(false)
}

pub async fn set_entry_finalized(
    deps: &Deps,
    session_id: &str,
    entry_id: &str,
) -> Result<(), IIIError> {
    state::set(
        &deps.iii,
        STATE_SCOPE,
        &format!("entry:{session_id}:{entry_id}:finalized"),
        json!(true),
        Some(deps.cfg().await.timeout_ms),
    )
    .await?;
    Ok(())
}

pub async fn approval_message_id(
    deps: &Deps,
    session_id: &str,
    function_call_id: &str,
) -> Option<i64> {
    state::get_i64(
        &deps.iii,
        STATE_SCOPE,
        &format!("approval:{session_id}:{function_call_id}:msg"),
        deps.cfg().await.timeout_ms,
    )
    .await
}

pub async fn set_approval_message_id(
    deps: &Deps,
    session_id: &str,
    function_call_id: &str,
    message_id: i64,
) -> Result<(), IIIError> {
    state::set(
        &deps.iii,
        STATE_SCOPE,
        &format!("approval:{session_id}:{function_call_id}:msg"),
        json!(message_id),
        Some(deps.cfg().await.timeout_ms),
    )
    .await?;
    Ok(())
}

pub async fn get_fsm(deps: &Deps, chat_id: i64) -> ChatFsm {
    let key = format!("chat:{chat_id}:fsm");
    match state::get_string(&deps.iii, STATE_SCOPE, &key, deps.cfg().await.timeout_ms).await {
        Some(s) => ChatFsm::parse(&s),
        None => ChatFsm::Idle,
    }
}

pub async fn set_fsm(deps: &Deps, chat_id: i64, fsm: ChatFsm) -> Result<(), IIIError> {
    state::set(
        &deps.iii,
        STATE_SCOPE,
        &format!("chat:{chat_id}:fsm"),
        json!(fsm.as_str()),
        Some(deps.cfg().await.timeout_ms),
    )
    .await?;
    Ok(())
}

pub async fn clear_chat_session(deps: &Deps, chat_id: i64) -> Result<Option<String>, IIIError> {
    let timeout = deps.cfg().await.timeout_ms;
    let old = chat_session(deps, chat_id).await;
    if let Some(ref sid) = old {
        state::delete(
            &deps.iii,
            STATE_SCOPE,
            &format!("session:{sid}:chat"),
            Some(timeout),
        )
        .await?;
    }
    state::delete(
        &deps.iii,
        STATE_SCOPE,
        &format!("chat:{chat_id}:session"),
        Some(timeout),
    )
    .await?;
    Ok(old)
}

pub async fn clear_chat_model(deps: &Deps, chat_id: i64) -> Result<(), IIIError> {
    let timeout = deps.cfg().await.timeout_ms;
    state::delete(
        &deps.iii,
        STATE_SCOPE,
        &format!("chat:{chat_id}:model"),
        Some(timeout),
    )
    .await?;
    Ok(())
}

pub async fn set_chat_model(
    deps: &Deps,
    chat_id: i64,
    model: &crate::config::ModelRef,
) -> Result<(), IIIError> {
    state::set(
        &deps.iii,
        STATE_SCOPE,
        &format!("chat:{chat_id}:model"),
        serde_json::to_value(model).unwrap_or(json!({})),
        Some(deps.cfg().await.timeout_ms),
    )
    .await?;
    Ok(())
}

pub async fn chat_model(deps: &Deps, chat_id: i64) -> Option<crate::config::ModelRef> {
    let v = state::get(
        &deps.iii,
        STATE_SCOPE,
        &format!("chat:{chat_id}:model"),
        Some(deps.cfg().await.timeout_ms),
    )
    .await
    .ok()?;
    serde_json::from_value(v).ok()
}

pub async fn store_approval_callback(
    deps: &Deps,
    token: &str,
    data: &crate::types::ApprovalCallbackData,
) -> Result<(), IIIError> {
    state::set(
        &deps.iii,
        STATE_SCOPE,
        &format!("cb:{token}"),
        serde_json::to_value(data).unwrap_or(json!({})),
        Some(deps.cfg().await.timeout_ms),
    )
    .await?;
    Ok(())
}

pub async fn load_approval_callback(
    deps: &Deps,
    token: &str,
) -> Option<crate::types::ApprovalCallbackData> {
    let v = state::get(
        &deps.iii,
        STATE_SCOPE,
        &format!("cb:{token}"),
        Some(deps.cfg().await.timeout_ms),
    )
    .await
    .ok()?;
    serde_json::from_value(v).ok()
}

pub async fn delete_approval_callback(deps: &Deps, token: &str) {
    let _ = state::delete(
        &deps.iii,
        STATE_SCOPE,
        &format!("cb:{token}"),
        Some(deps.cfg().await.timeout_ms),
    )
    .await;
}

pub async fn thinking_message_id(deps: &Deps, session_id: &str, entry_id: &str) -> Option<i64> {
    state::get_i64(
        &deps.iii,
        STATE_SCOPE,
        &format!("entry:{session_id}:{entry_id}:thinking_msg"),
        deps.cfg().await.timeout_ms,
    )
    .await
}

pub async fn set_thinking_message_id(
    deps: &Deps,
    session_id: &str,
    entry_id: &str,
    message_id: i64,
) -> Result<(), IIIError> {
    state::set(
        &deps.iii,
        STATE_SCOPE,
        &format!("entry:{session_id}:{entry_id}:thinking_msg"),
        json!(message_id),
        Some(deps.cfg().await.timeout_ms),
    )
    .await?;
    Ok(())
}

pub async fn chat_verbosity_override(
    deps: &Deps,
    chat_id: i64,
) -> Option<crate::config::Verbosity> {
    let v = state::get(
        &deps.iii,
        STATE_SCOPE,
        &format!("chat:{chat_id}:verbosity"),
        Some(deps.cfg().await.timeout_ms),
    )
    .await
    .ok()?;
    serde_json::from_value(v).ok()
}

pub async fn set_chat_verbosity(
    deps: &Deps,
    chat_id: i64,
    verbosity: crate::config::Verbosity,
) -> Result<(), IIIError> {
    state::set(
        &deps.iii,
        STATE_SCOPE,
        &format!("chat:{chat_id}:verbosity"),
        json!(verbosity),
        Some(deps.cfg().await.timeout_ms),
    )
    .await?;
    Ok(())
}

pub async fn chat_thinking_level_override(
    deps: &Deps,
    chat_id: i64,
) -> Option<crate::config::ThinkingLevel> {
    let v = state::get(
        &deps.iii,
        STATE_SCOPE,
        &format!("chat:{chat_id}:thinking_level"),
        Some(deps.cfg().await.timeout_ms),
    )
    .await
    .ok()?;
    serde_json::from_value(v).ok()
}

pub async fn set_chat_thinking_level(
    deps: &Deps,
    chat_id: i64,
    level: Option<crate::config::ThinkingLevel>,
) -> Result<(), IIIError> {
    let key = format!("chat:{chat_id}:thinking_level");
    let timeout = deps.cfg().await.timeout_ms;
    match level {
        Some(l) => {
            state::set(&deps.iii, STATE_SCOPE, &key, json!(l), Some(timeout)).await?;
        }
        None => {
            let _ = state::delete(&deps.iii, STATE_SCOPE, &key, Some(timeout)).await;
        }
    }
    Ok(())
}
