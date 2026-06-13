//! Compaction-lease storage over the engine's `state::*` built-ins.
//!
//! `swap` uses `state::update` — the only atomic read-modify-write iii
//! exposes — so exactly one concurrent acquirer observes the prior
//! value (see `core::lease` for the acquire/release protocol).

use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::{TriggerRequest, III};
use serde_json::{json, Value};

use crate::ports::{LeaseRecord, LeaseStore};

const STATE_TIMEOUT_MS: u64 = 5_000;

pub struct IiiLeaseStore {
    iii: Arc<III>,
}

impl IiiLeaseStore {
    pub fn new(iii: Arc<III>) -> Self {
        Self { iii }
    }

    async fn call(&self, function_id: &str, payload: Value) -> Result<Value, String> {
        self.iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload,
                action: None,
                timeout_ms: Some(STATE_TIMEOUT_MS),
            })
            .await
            .map_err(|e| format!("{function_id} failed: {e}"))
    }
}

/// A stored value that isn't a `{nonce, ts}` record reads as free —
/// same posture as the harness lease (`readLeaseTimestampSecs` returns
/// 0 for foreign shapes).
fn parse_record(v: Value) -> Option<LeaseRecord> {
    serde_json::from_value(v).ok()
}

#[async_trait]
impl LeaseStore for IiiLeaseStore {
    async fn get(&self, scope: &str, key: &str) -> Result<Option<LeaseRecord>, String> {
        let v = self
            .call("state::get", json!({ "scope": scope, "key": key }))
            .await?;
        if v.is_null() {
            return Ok(None);
        }
        Ok(parse_record(v))
    }

    async fn set(&self, scope: &str, key: &str, value: Option<LeaseRecord>) -> Result<(), String> {
        match value {
            Some(rec) => {
                self.call(
                    "state::set",
                    json!({ "scope": scope, "key": key, "value": rec }),
                )
                .await?;
            }
            None => {
                self.call("state::delete", json!({ "scope": scope, "key": key }))
                    .await?;
            }
        }
        Ok(())
    }

    async fn swap(
        &self,
        scope: &str,
        key: &str,
        value: LeaseRecord,
    ) -> Result<Option<LeaseRecord>, String> {
        let resp = self
            .call(
                "state::update",
                json!({
                    "scope": scope,
                    "key": key,
                    "ops": [{ "type": "set", "path": "", "value": value }],
                }),
            )
            .await?;
        // A null envelope means the atomic write failed — the caller
        // must never treat that as a win.
        if resp.is_null() {
            return Err("state::update returned a null envelope".to_string());
        }
        let old = resp.get("old_value").cloned().unwrap_or(Value::Null);
        if old.is_null() {
            return Ok(None);
        }
        Ok(parse_record(old))
    }
}
