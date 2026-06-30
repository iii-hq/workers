//! Session registry on engine state. Scope `grok_sessions`, key = iii
//! session_id. Maps iii sessions to Grok thread ids so a run with the same
//! session_id resumes the underlying Grok thread (persisted by the CLI under
//! ~/.grok/sessions). Trigger errors propagate (fail-fast); swallowing them
//! would silently start a fresh conversation instead of resuming.

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::json;

use crate::wire::SessionRecord;

const SCOPE: &str = "grok_sessions";
const TIMEOUT_MS: u64 = 5_000;

pub async fn load_session(
    iii: &IIIClient,
    session_id: &str,
) -> anyhow::Result<Option<SessionRecord>> {
    let v = iii
        .trigger(TriggerRequest {
            function_id: "state::get".to_string(),
            payload: json!({ "scope": SCOPE, "key": session_id }),
            action: None,
            timeout_ms: Some(TIMEOUT_MS),
        })
        .await
        .map_err(|e| anyhow::anyhow!("state::get failed: {e}"))?;
    // The engine returns the stored value directly (no envelope); null when absent.
    if v.is_null() {
        return Ok(None);
    }
    // A decode failure means a corrupt or version-drifted record we wrote — a
    // real problem, not a missing session. Surface it instead of silently
    // starting a fresh conversation.
    let rec = serde_json::from_value::<SessionRecord>(v)
        .map_err(|e| anyhow::anyhow!("corrupt session record for {session_id}: {e}"))?;
    Ok(Some(rec))
}

pub async fn save_session(iii: &IIIClient, record: &SessionRecord) -> anyhow::Result<()> {
    iii.trigger(TriggerRequest {
        function_id: "state::set".to_string(),
        payload: json!({ "scope": SCOPE, "key": record.session_id, "value": record }),
        action: None,
        timeout_ms: Some(TIMEOUT_MS),
    })
    .await
    .map_err(|e| anyhow::anyhow!("state::set failed: {e}"))?;
    Ok(())
}

pub async fn list_sessions(iii: &IIIClient) -> anyhow::Result<Vec<SessionRecord>> {
    let v = iii
        .trigger(TriggerRequest {
            function_id: "state::list".to_string(),
            payload: json!({ "scope": SCOPE }),
            action: None,
            timeout_ms: Some(TIMEOUT_MS),
        })
        .await
        .map_err(|e| anyhow::anyhow!("state::list failed: {e}"))?;
    let arr = match v.as_array() {
        Some(a) => a,
        None => return Ok(vec![]),
    };
    Ok(arr
        .iter()
        .filter_map(|item| serde_json::from_value::<SessionRecord>(item.clone()).ok())
        .collect())
}

/// Best-effort flip to `error` so a failed background run never leaves a
/// record stuck in `working`. Swallows its own failure.
pub async fn mark_error(iii: &IIIClient, session_id: &str) {
    match load_session(iii, session_id).await {
        Ok(Some(mut rec)) if matches!(rec.status, crate::wire::Status::Working) => {
            rec.status = crate::wire::Status::Error;
            rec.updated_at_ms = crate::wire::now_ms();
            if let Err(e) = save_session(iii, &rec).await {
                tracing::error!(session_id, error = %e, "failed to mark session error");
            }
        }
        Ok(_) => {}
        Err(e) => tracing::error!(session_id, error = %e, "mark_error load failed"),
    }
}
