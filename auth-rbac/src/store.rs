//! State-backed CRUD for workspaces, API keys, and role grants.
//! Direct port of roster/workers/auth/src/store.ts.

use iii_sdk::{TriggerRequest, Value, III};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::roles::Role;

pub const SCOPE: &str = "auth";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub owner_id: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: String,
    pub workspace_id: String,
    pub role: Role,
    pub hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleGrant {
    pub workspace_id: String,
    pub user_id: String,
    pub role: Role,
    pub granted_at: i64,
}

pub fn workspace_key(id: &str) -> String {
    format!("workspace:{id}")
}

pub fn key_key(id: &str) -> String {
    format!("key:{id}")
}

/// Lookup keyed on the **full** hmac hash, not a prefix, so two keys that
/// share a prefix can't overwrite each other's lookup entry.
pub fn key_lookup_key(full_hash: &str) -> String {
    format!("key_lookup:{full_hash}")
}

pub fn role_key(workspace_id: &str, user_id: &str) -> String {
    format!("role:{workspace_id}:{user_id}")
}

pub async fn state_set(iii: &III, key: &str, value: &Value) -> anyhow::Result<()> {
    iii.trigger(TriggerRequest {
        function_id: "state::set".into(),
        payload: json!({ "scope": SCOPE, "key": key, "value": value }),
        action: None,
        timeout_ms: None,
    })
    .await?;
    Ok(())
}

pub async fn state_get<T: serde::de::DeserializeOwned>(
    iii: &III,
    key: &str,
) -> anyhow::Result<Option<T>> {
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "state::get".into(),
            payload: json!({ "scope": SCOPE, "key": key }),
            action: None,
            timeout_ms: None,
        })
        .await?;
    if resp.is_null() {
        return Ok(None);
    }
    let value = resp.get("value").cloned().unwrap_or(resp);
    if value.is_null() {
        return Ok(None);
    }
    Ok(serde_json::from_value(value)?)
}

pub async fn state_list(iii: &III, prefix: &str) -> anyhow::Result<Vec<Value>> {
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "state::list".into(),
            payload: json!({ "scope": SCOPE, "prefix": prefix }),
            action: None,
            timeout_ms: None,
        })
        .await?;
    Ok(resp.as_array().cloned().unwrap_or_default())
}

pub async fn state_delete(iii: &III, key: &str) -> anyhow::Result<()> {
    iii.trigger(TriggerRequest {
        function_id: "state::delete".into(),
        payload: json!({ "scope": SCOPE, "key": key }),
        action: None,
        timeout_ms: None,
    })
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_key_format() {
        assert_eq!(workspace_key("w-1"), "workspace:w-1");
    }

    #[test]
    fn role_key_includes_both_ids() {
        assert_eq!(role_key("ws-1", "u-2"), "role:ws-1:u-2");
    }

    #[test]
    fn key_lookup_key_uses_full_hash() {
        let h = "a".repeat(64);
        assert_eq!(key_lookup_key(&h), format!("key_lookup:{h}"));
    }
}
