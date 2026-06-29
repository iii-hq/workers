//! Durable bridge state in `iii-state`: thread↔session mapping, the per-thread
//! pending context buffer, and approval-button callbacks.

use iii_sdk::errors::Error;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::clients::state;
use crate::deps::{Deps, STATE_SCOPE};
use crate::types::ApprovalCallback;

/// Where a session's replies go.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub team: Option<String>,
    pub channel: String,
    pub thread_ts: String,
    /// User who addressed the bot (required to stream into a channel).
    #[serde(default)]
    pub recipient_user_id: Option<String>,
}

/// One captured-but-not-yet-acted-on message in a thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingMsg {
    pub user: Option<String>,
    pub text: String,
}

fn thread_key(team: Option<&str>, channel: &str, thread_ts: &str) -> String {
    format!("{}:{}:{}", team.unwrap_or("-"), channel, thread_ts)
}

pub async fn thread_session(
    deps: &Deps,
    team: Option<&str>,
    channel: &str,
    thread_ts: &str,
) -> Option<String> {
    let key = format!("thread:{}:session", thread_key(team, channel, thread_ts));
    state::get_string(&deps.iii, STATE_SCOPE, &key, deps.cfg().await.timeout_ms).await
}

pub async fn set_thread_session(
    deps: &Deps,
    team: Option<&str>,
    channel: &str,
    thread_ts: &str,
    session_id: &str,
    recipient_user_id: Option<&str>,
) -> Result<(), Error> {
    let timeout = deps.cfg().await.timeout_ms;
    let tk = thread_key(team, channel, thread_ts);
    state::set(
        &deps.iii,
        STATE_SCOPE,
        &format!("thread:{tk}:session"),
        json!(session_id),
        timeout,
    )
    .await?;
    let target = Target {
        team: team.map(str::to_string),
        channel: channel.to_string(),
        thread_ts: thread_ts.to_string(),
        recipient_user_id: recipient_user_id.map(str::to_string),
    };
    state::set(
        &deps.iii,
        STATE_SCOPE,
        &format!("session:{session_id}:target"),
        serde_json::to_value(&target).unwrap_or(json!({})),
        timeout,
    )
    .await?;
    Ok(())
}

pub async fn session_target(deps: &Deps, session_id: &str) -> Option<Target> {
    let v = state::get_value(
        &deps.iii,
        STATE_SCOPE,
        &format!("session:{session_id}:target"),
        deps.cfg().await.timeout_ms,
    )
    .await?;
    serde_json::from_value(v).ok()
}

pub async fn push_pending(
    deps: &Deps,
    team: Option<&str>,
    channel: &str,
    thread_ts: &str,
    msg: PendingMsg,
    max: u32,
) -> Result<(), Error> {
    let timeout = deps.cfg().await.timeout_ms;
    let key = format!("thread:{}:pending", thread_key(team, channel, thread_ts));
    let mut buf: Vec<PendingMsg> = state::get_value(&deps.iii, STATE_SCOPE, &key, timeout)
        .await
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    buf.push(msg);
    let max = max.max(1) as usize;
    if buf.len() > max {
        let drop = buf.len() - max;
        buf.drain(0..drop);
    }
    state::set(
        &deps.iii,
        STATE_SCOPE,
        &key,
        serde_json::to_value(&buf).unwrap_or(json!([])),
        timeout,
    )
    .await
}

pub async fn take_pending(
    deps: &Deps,
    team: Option<&str>,
    channel: &str,
    thread_ts: &str,
) -> Vec<PendingMsg> {
    let timeout = deps.cfg().await.timeout_ms;
    let key = format!("thread:{}:pending", thread_key(team, channel, thread_ts));
    let buf: Vec<PendingMsg> = state::get_value(&deps.iii, STATE_SCOPE, &key, timeout)
        .await
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    if !buf.is_empty() {
        let _ = state::delete(&deps.iii, STATE_SCOPE, &key, timeout).await;
    }
    buf
}

pub async fn store_approval_cb(
    deps: &Deps,
    token: &str,
    cb: &ApprovalCallback,
) -> Result<(), Error> {
    state::set(
        &deps.iii,
        STATE_SCOPE,
        &format!("cb:{token}"),
        serde_json::to_value(cb).unwrap_or(json!({})),
        deps.cfg().await.timeout_ms,
    )
    .await
}

pub async fn load_approval_cb(deps: &Deps, token: &str) -> Option<ApprovalCallback> {
    let v = state::get_value(
        &deps.iii,
        STATE_SCOPE,
        &format!("cb:{token}"),
        deps.cfg().await.timeout_ms,
    )
    .await?;
    serde_json::from_value(v).ok()
}

pub async fn delete_approval_cb(deps: &Deps, token: &str) {
    let _ = state::delete(
        &deps.iii,
        STATE_SCOPE,
        &format!("cb:{token}"),
        deps.cfg().await.timeout_ms,
    )
    .await;
}
