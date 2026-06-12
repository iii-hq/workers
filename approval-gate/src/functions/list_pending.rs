//! `approval::list_pending` — the pending inbox. Filters apply
//! worker-side over the live scope (cheap — the scope only ever holds
//! live records). Ordered by `pending_at` ascending with a stable
//! tie-break so the opaque cursor paginates deterministically.

use base64::Engine as _;

use super::Deps;
use crate::error::ApprovalError;
use crate::pending;
use crate::types::{
    metadata_matches, ListPendingRequest, ListPendingResponse, PendingApprovalRecord,
};

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 500;

type CursorKey = (i64, String, String);

fn sort_key(record: &PendingApprovalRecord) -> CursorKey {
    (
        record.pending_at,
        record.session_id.clone(),
        record.function_call_id.clone(),
    )
}

fn encode_cursor(key: &CursorKey) -> String {
    let raw = serde_json::to_vec(&serde_json::json!([key.0, key.1, key.2])).unwrap_or_default();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
}

fn decode_cursor(cursor: &str) -> Result<CursorKey, ApprovalError> {
    let invalid = || ApprovalError::InvalidPayload("invalid cursor".to_string());
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| invalid())?;
    let value: serde_json::Value = serde_json::from_slice(&raw).map_err(|_| invalid())?;
    let parts = value
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or_else(invalid)?;
    Ok((
        parts[0].as_i64().ok_or_else(invalid)?,
        parts[1].as_str().ok_or_else(invalid)?.to_string(),
        parts[2].as_str().ok_or_else(invalid)?.to_string(),
    ))
}

pub async fn handle(
    deps: &Deps,
    req: ListPendingRequest,
) -> Result<ListPendingResponse, ApprovalError> {
    let after = req.cursor.as_deref().map(decode_cursor).transpose()?;
    let limit = req.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

    let mut records = pending::list_all(deps.bus.as_ref(), deps.cfg.state_timeout_ms)
        .await
        .map_err(|e| ApprovalError::StateUnavailable(format!("pending list failed: {e}")))?;

    records.retain(|record| {
        if let Some(want_sid) = &req.session_id {
            if want_sid != &record.session_id {
                return false;
            }
        }
        if let Some(want) = &req.metadata {
            if !metadata_matches(want, record.session_metadata.as_ref()) {
                return false;
            }
        }
        true
    });
    records.sort_by_key(sort_key);

    let upper: Vec<PendingApprovalRecord> = match after {
        Some(cursor_key) => records
            .into_iter()
            .filter(|r| sort_key(r) > cursor_key)
            .collect(),
        None => records,
    };

    let has_more = upper.len() > limit;
    let page: Vec<PendingApprovalRecord> = upper.into_iter().take(limit).collect();
    let next_cursor = if has_more {
        page.last().map(|last| encode_cursor(&sort_key(last)))
    } else {
        None
    };

    Ok(ListPendingResponse {
        pending: page,
        next_cursor,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::config::WorkerConfig;
    use crate::events::RecordingSink;
    use crate::gate_config::shared_defaults;
    use crate::pending::PENDING_SCOPE;
    use crate::testkit::{FakeBus, MemoryState};

    fn fixture() -> (Arc<Deps>, MemoryState) {
        let bus = Arc::new(FakeBus::new());
        let state = bus.with_memory_state();
        let deps = Arc::new(Deps {
            bus,
            sink: Arc::new(RecordingSink::new()),
            defaults: shared_defaults(),
            cfg: Arc::new(WorkerConfig::default()),
        });
        (deps, state)
    }

    fn seed(state: &MemoryState, sid: &str, cid: &str, pending_at: i64, owner: Option<&str>) {
        let mut record = json!({
            "session_id": sid,
            "turn_id": "t_1",
            "function_call_id": cid,
            "function_id": "shell::run",
            "arguments_excerpt": {},
            "pending_at": pending_at,
            "expires_at": pending_at + 1000,
            "depth": 0,
        });
        if let Some(owner) = owner {
            record["session_metadata"] = json!({ "owner": owner });
        }
        state.seed(PENDING_SCOPE, &format!("{sid}/{cid}"), record);
    }

    #[tokio::test]
    async fn orders_by_pending_at_ascending() {
        let (deps, state) = fixture();
        seed(&state, "s_1", "c_2", 300, None);
        seed(&state, "s_1", "c_1", 100, None);
        seed(&state, "s_2", "c_3", 200, None);

        let res = handle(&deps, ListPendingRequest::default()).await.unwrap();
        let order: Vec<i64> = res.pending.iter().map(|r| r.pending_at).collect();
        assert_eq!(order, vec![100, 200, 300]);
        assert!(res.next_cursor.is_none());
    }

    #[tokio::test]
    async fn filters_by_session_and_metadata() {
        let (deps, state) = fixture();
        seed(&state, "s_1", "c_1", 100, Some("u_1"));
        seed(&state, "s_2", "c_2", 200, Some("u_2"));

        let by_session = handle(
            &deps,
            ListPendingRequest {
                session_id: Some("s_2".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(by_session.pending.len(), 1);
        assert_eq!(by_session.pending[0].session_id, "s_2");

        let by_meta = handle(
            &deps,
            ListPendingRequest {
                metadata: Some(serde_json::from_value(json!({ "owner": "u_1" })).unwrap()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(by_meta.pending.len(), 1);
        assert_eq!(by_meta.pending[0].session_id, "s_1");
    }

    #[tokio::test]
    async fn paginates_with_an_opaque_cursor() {
        let (deps, state) = fixture();
        for i in 0..5 {
            seed(&state, "s_1", &format!("c_{i}"), 100 + i, None);
        }

        let first = handle(
            &deps,
            ListPendingRequest {
                limit: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(first.pending.len(), 2);
        let cursor = first.next_cursor.clone().unwrap();

        let second = handle(
            &deps,
            ListPendingRequest {
                limit: Some(2),
                cursor: Some(cursor),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(second.pending.len(), 2);
        assert!(second.pending[0].pending_at > first.pending[1].pending_at);

        let third = handle(
            &deps,
            ListPendingRequest {
                limit: Some(2),
                cursor: second.next_cursor.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(third.pending.len(), 1);
        assert!(third.next_cursor.is_none());
    }

    #[tokio::test]
    async fn rejects_malformed_cursors() {
        let (deps, _state) = fixture();
        let err = handle(
            &deps,
            ListPendingRequest {
                cursor: Some("not base64 json!!!".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), "approval/invalid_payload");
    }
}
