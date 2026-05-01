//! Register `auth::rbac::*` functions on the iii bus. Mirrors the function
//! surface in roster/workers/auth/src/worker.ts.

use std::sync::Arc;

use chrono::Utc;
use iii_sdk::{FunctionRef, IIIError, RegisterFunctionMessage, Value, III};
use serde_json::json;

use crate::hmac::{generate_token, hash_token, load_secret, timing_safe_hex_equal};
use crate::roles::{assert_role, role_satisfies, Role};
use crate::store::{
    key_key, key_lookup_key, role_key, state_delete, state_get, state_list, state_set,
    workspace_key, ApiKey, RoleGrant, Workspace,
};

const LAST_USED_WRITE_INTERVAL_MS: i64 = 5 * 60 * 1000;
const FN_PREFIX: &str = "auth::rbac";

pub struct AuthRbacConfig {
    pub secret: String,
}

impl AuthRbacConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            secret: load_secret()?,
        })
    }
}

pub struct AuthRbacFunctionRefs {
    pub workspace_create: FunctionRef,
    pub workspace_get: FunctionRef,
    pub key_create: FunctionRef,
    pub key_list: FunctionRef,
    pub key_revoke: FunctionRef,
    pub verify: FunctionRef,
    pub role_grant: FunctionRef,
    pub role_check: FunctionRef,
    pub role_list: FunctionRef,
}

impl AuthRbacFunctionRefs {
    pub fn unregister_all(self) {
        for r in [
            self.workspace_create,
            self.workspace_get,
            self.key_create,
            self.key_list,
            self.key_revoke,
            self.verify,
            self.role_grant,
            self.role_check,
            self.role_list,
        ] {
            r.unregister();
        }
    }
}

pub async fn register_with_iii(
    iii: &III,
    cfg: AuthRbacConfig,
) -> anyhow::Result<AuthRbacFunctionRefs> {
    let secret = Arc::new(cfg.secret);

    let workspace_create = {
        let iii_for = iii.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id(format!("{FN_PREFIX}::workspace_create"))
                .with_description(
                    "Create a workspace and grant the owner role to its creator.".into(),
                ),
            move |payload: Value| {
                let iii = iii_for.clone();
                async move {
                    let name = required_str(&payload, "name")?;
                    let owner_id = required_str(&payload, "owner_id")?;
                    let id = uuid::Uuid::new_v4().to_string();
                    let now_ms = Utc::now().timestamp_millis();
                    let ws = Workspace {
                        id: id.clone(),
                        name,
                        owner_id: owner_id.clone(),
                        created_at: now_ms,
                    };
                    state_set(
                        &iii,
                        &workspace_key(&id),
                        &serde_json::to_value(&ws).unwrap(),
                    )
                    .await
                    .map_err(|e| IIIError::Handler(e.to_string()))?;
                    let grant = RoleGrant {
                        workspace_id: id.clone(),
                        user_id: owner_id.clone(),
                        role: Role::Owner,
                        granted_at: now_ms,
                    };
                    if let Err(e) = state_set(
                        &iii,
                        &role_key(&id, &owner_id),
                        &serde_json::to_value(&grant).unwrap(),
                    )
                    .await
                    {
                        let _ = state_delete(&iii, &workspace_key(&id)).await;
                        return Err(IIIError::Handler(format!("owner grant failed: {e}")));
                    }
                    Ok(json!({ "workspace_id": id }))
                }
            },
        ))
    };

    let workspace_get = {
        let iii_for = iii.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id(format!("{FN_PREFIX}::workspace_get"))
                .with_description("Fetch the public-facing workspace metadata.".into()),
            move |payload: Value| {
                let iii = iii_for.clone();
                async move {
                    let workspace_id = required_str(&payload, "workspace_id")?;
                    let ws: Option<Workspace> = state_get(&iii, &workspace_key(&workspace_id)).await
                        .map_err(|e| IIIError::Handler(e.to_string()))?;
                    Ok(json!({ "workspace": ws.map(|w| json!({ "id": w.id, "name": w.name, "created_at": w.created_at })) }))
                }
            },
        ))
    };

    let key_create = {
        let iii_for = iii.clone();
        let secret = secret.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id(format!("{FN_PREFIX}::key_create")).with_description(
                "Mint an HMAC-hashed API key bound to a workspace + role.".into(),
            ),
            move |payload: Value| {
                let iii = iii_for.clone();
                let secret = secret.clone();
                async move {
                    let workspace_id = required_str(&payload, "workspace_id")?;
                    let role_str = required_str(&payload, "role")?;
                    let role = assert_role(&role_str).map_err(IIIError::Handler)?;
                    let description = payload
                        .get("description")
                        .and_then(Value::as_str)
                        .map(String::from);
                    let created_by = payload
                        .get("created_by")
                        .and_then(Value::as_str)
                        .map(String::from);

                    let ws: Option<Workspace> = state_get(&iii, &workspace_key(&workspace_id))
                        .await
                        .map_err(|e| IIIError::Handler(e.to_string()))?;
                    if ws.is_none() {
                        return Err(IIIError::Handler(format!(
                            "workspace not found: {workspace_id}"
                        )));
                    }

                    let id = uuid::Uuid::new_v4().to_string();
                    let token = generate_token(&workspace_id);
                    let hash = hash_token(&secret, &token);
                    let record = ApiKey {
                        id: id.clone(),
                        workspace_id: workspace_id.clone(),
                        role,
                        hash: hash.clone(),
                        description,
                        created_by,
                        created_at: Utc::now().timestamp_millis(),
                        last_used_at: None,
                        revoked_at: None,
                    };
                    state_set(&iii, &key_key(&id), &serde_json::to_value(&record).unwrap())
                        .await
                        .map_err(|e| IIIError::Handler(e.to_string()))?;
                    if let Err(e) =
                        state_set(&iii, &key_lookup_key(&hash), &Value::String(id.clone())).await
                    {
                        let _ = state_delete(&iii, &key_key(&id)).await;
                        return Err(IIIError::Handler(format!("lookup write failed: {e}")));
                    }
                    Ok(json!({ "key_id": id, "token": token }))
                }
            },
        ))
    };

    let key_list = {
        let iii_for = iii.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id(format!("{FN_PREFIX}::key_list")).with_description(
                "List all keys in a workspace, ordered by created_at desc.".into(),
            ),
            move |payload: Value| {
                let iii = iii_for.clone();
                async move {
                    let workspace_id = required_str(&payload, "workspace_id")?;
                    let entries = state_list(&iii, "key:")
                        .await
                        .map_err(|e| IIIError::Handler(e.to_string()))?;
                    let mut keys: Vec<Value> = entries
                        .into_iter()
                        .filter_map(|v| serde_json::from_value::<ApiKey>(v).ok())
                        .filter(|k| k.workspace_id == workspace_id)
                        .map(|k| {
                            json!({
                                "key_id": k.id,
                                "role": k.role,
                                "description": k.description,
                                "created_at": k.created_at,
                                "last_used_at": k.last_used_at,
                                "revoked_at": k.revoked_at,
                            })
                        })
                        .collect();
                    keys.sort_by_key(|v| -v.get("created_at").and_then(Value::as_i64).unwrap_or(0));
                    Ok(json!({ "keys": keys }))
                }
            },
        ))
    };

    let key_revoke = {
        let iii_for = iii.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id(format!("{FN_PREFIX}::key_revoke"))
                .with_description("Mark a key as revoked. Idempotent.".into()),
            move |payload: Value| {
                let iii = iii_for.clone();
                async move {
                    let key_id = required_str(&payload, "key_id")?;
                    let mut k: ApiKey = state_get(&iii, &key_key(&key_id))
                        .await
                        .map_err(|e| IIIError::Handler(e.to_string()))?
                        .ok_or_else(|| IIIError::Handler(format!("key not found: {key_id}")))?;
                    if k.revoked_at.is_some() {
                        return Ok(json!({ "ok": true }));
                    }
                    k.revoked_at = Some(Utc::now().timestamp_millis());
                    state_set(&iii, &key_key(&k.id), &serde_json::to_value(&k).unwrap())
                        .await
                        .map_err(|e| IIIError::Handler(e.to_string()))?;
                    Ok(json!({ "ok": true }))
                }
            },
        ))
    };

    let verify = {
        let iii_for = iii.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id(format!("{FN_PREFIX}::verify"))
                .with_description("Validate a token, optionally requiring workspace + role.".into()),
            move |payload: Value| {
                let iii = iii_for.clone();
                let secret = secret.clone();
                async move {
                    let token = match payload.get("token").and_then(Value::as_str) {
                        Some(t) if !t.is_empty() => t.to_string(),
                        _ => return Ok(json!({ "valid": false, "reason": "missing token" })),
                    };
                    let incoming = hash_token(&secret, &token);
                    let key_id_value: Option<String> = state_get(&iii, &key_lookup_key(&incoming)).await
                        .map_err(|e| IIIError::Handler(e.to_string()))?;
                    let Some(key_id) = key_id_value else {
                        return Ok(json!({ "valid": false, "reason": "unknown token" }));
                    };
                    let record: Option<ApiKey> = state_get(&iii, &key_key(&key_id)).await
                        .map_err(|e| IIIError::Handler(e.to_string()))?;
                    let Some(mut record) = record else {
                        return Ok(json!({ "valid": false, "reason": "unknown token" }));
                    };
                    if !timing_safe_hex_equal(&incoming, &record.hash) {
                        return Ok(json!({ "valid": false, "reason": "unknown token" }));
                    }
                    if record.revoked_at.is_some() {
                        return Ok(json!({ "valid": false, "reason": "revoked" }));
                    }
                    if let Some(ws) = payload.get("workspace_id").and_then(Value::as_str) {
                        if record.workspace_id != ws {
                            return Ok(json!({ "valid": false, "reason": "workspace mismatch" }));
                        }
                    }
                    if let Some(req) = payload.get("required_role").and_then(Value::as_str) {
                        match assert_role(req) {
                            Err(_) => return Ok(json!({ "valid": false, "reason": format!("invalid required_role: {req}") })),
                            Ok(req_role) => {
                                if !role_satisfies(record.role, req_role) {
                                    return Ok(json!({ "valid": false, "reason": "insufficient role" }));
                                }
                            }
                        }
                    }
                    let now_ms = Utc::now().timestamp_millis();
                    let fresh = record
                        .last_used_at
                        .is_some_and(|t| now_ms - t < LAST_USED_WRITE_INTERVAL_MS);
                    if !fresh {
                        record.last_used_at = Some(now_ms);
                        let updated = serde_json::to_value(&record).unwrap();
                        let key = key_key(&record.id);
                        let iii_for_bg = iii.clone();
                        tokio::spawn(async move {
                            if let Err(e) = state_set(&iii_for_bg, &key, &updated).await {
                                tracing::warn!(error = %e, "last_used_at update failed");
                            }
                        });
                    }
                    Ok(json!({
                        "valid": true,
                        "key_id": record.id,
                        "workspace_id": record.workspace_id,
                        "role": record.role,
                    }))
                }
            },
        ))
    };

    let role_grant = {
        let iii_for = iii.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id(format!("{FN_PREFIX}::role_grant")).with_description(
                "Grant a workspace role to a user. Refuses to demote the workspace owner.".into(),
            ),
            move |payload: Value| {
                let iii = iii_for.clone();
                async move {
                    let workspace_id = required_str(&payload, "workspace_id")?;
                    let user_id = required_str(&payload, "user_id")?;
                    let role_str = required_str(&payload, "role")?;
                    let role = assert_role(&role_str).map_err(IIIError::Handler)?;
                    let ws: Option<Workspace> = state_get(&iii, &workspace_key(&workspace_id))
                        .await
                        .map_err(|e| IIIError::Handler(e.to_string()))?;
                    let Some(ws) = ws else {
                        return Err(IIIError::Handler(format!(
                            "workspace not found: {workspace_id}"
                        )));
                    };
                    if ws.owner_id == user_id && role != Role::Owner {
                        return Err(IIIError::Handler(format!(
                            "cannot demote workspace owner {user_id} via role_grant; \
                             use an explicit ownership transfer flow"
                        )));
                    }
                    let grant = RoleGrant {
                        workspace_id: workspace_id.clone(),
                        user_id: user_id.clone(),
                        role,
                        granted_at: Utc::now().timestamp_millis(),
                    };
                    state_set(
                        &iii,
                        &role_key(&workspace_id, &user_id),
                        &serde_json::to_value(&grant).unwrap(),
                    )
                    .await
                    .map_err(|e| IIIError::Handler(e.to_string()))?;
                    Ok(json!({ "ok": true }))
                }
            },
        ))
    };

    let role_check = {
        let iii_for = iii.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id(format!("{FN_PREFIX}::role_check")).with_description(
                "Check whether a user holds at least the required role in a workspace.".into(),
            ),
            move |payload: Value| {
                let iii = iii_for.clone();
                async move {
                    let workspace_id = required_str(&payload, "workspace_id")?;
                    let user_id = required_str(&payload, "user_id")?;
                    let required = assert_role(&required_str(&payload, "required_role")?)
                        .map_err(IIIError::Handler)?;
                    let grant: Option<RoleGrant> =
                        state_get(&iii, &role_key(&workspace_id, &user_id))
                            .await
                            .map_err(|e| IIIError::Handler(e.to_string()))?;
                    let allowed = grant.is_some_and(|g| role_satisfies(g.role, required));
                    Ok(json!({ "allowed": allowed }))
                }
            },
        ))
    };

    let role_list = {
        let iii_for = iii.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id(format!("{FN_PREFIX}::role_list"))
                .with_description("List all role grants in a workspace.".into()),
            move |payload: Value| {
                let iii = iii_for.clone();
                async move {
                    let workspace_id = required_str(&payload, "workspace_id")?;
                    let entries = state_list(&iii, &format!("role:{workspace_id}:")).await
                        .map_err(|e| IIIError::Handler(e.to_string()))?;
                    let mut grants: Vec<Value> = entries
                        .into_iter()
                        .filter_map(|v| serde_json::from_value::<RoleGrant>(v).ok())
                        .filter(|g| g.workspace_id == workspace_id)
                        .map(|g| json!({ "user_id": g.user_id, "role": g.role, "granted_at": g.granted_at }))
                        .collect();
                    grants.sort_by_key(|v| v.get("granted_at").and_then(Value::as_i64).unwrap_or(0));
                    Ok(json!({ "grants": grants }))
                }
            },
        ))
    };

    Ok(AuthRbacFunctionRefs {
        workspace_create,
        workspace_get,
        key_create,
        key_list,
        key_revoke,
        verify,
        role_grant,
        role_check,
        role_list,
    })
}

fn required_str(payload: &Value, field: &str) -> Result<String, IIIError> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| IIIError::Handler(format!("missing required field: {field}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_str_extracts_field() {
        assert_eq!(required_str(&json!({"a": "1"}), "a").unwrap(), "1");
    }

    #[test]
    fn required_str_errors_on_missing() {
        assert!(required_str(&json!({}), "a").is_err());
    }
}
