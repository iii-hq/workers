//! `database::getConnection` / `saveConnection` / `deleteConnection` — the
//! connection editor's read and write path.
//!
//! Writes are a read-modify-write of the WHOLE `database` configuration
//! value through the configuration worker, read with `raw: true` so the
//! other entries' `${VAR}` templates and every `secret://` reference are
//! written back exactly as they were. Saves are serialized inside the worker;
//! the response comes after the pools reloaded, carrying the saved
//! connection's build error when it has one (the configuration is saved
//! either way: an unreachable server is not a reason to refuse an entry).
//!
//! Literal sensitive values never leave the worker: `getConnection` returns
//! them as `"__redacted__"`, and a save keeps the stored value wherever the
//! request still holds the placeholder (see `crate::redaction`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::AppState;
use crate::config::{validate_connection_name, DatabaseConfig, WorkerConfig};
use crate::connector::Stage;
use crate::error::DbError;
use crate::handlers::query::err_to_str;
use crate::redaction;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetConnectionReq {
    /// Connection name (the key under `databases`).
    pub name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GetConnectionResp {
    pub name: String,
    /// The entry as stored: references and `${VAR}` templates intact,
    /// literal passwords / passphrases / private keys / url passwords as
    /// `"__redacted__"`.
    pub connection: Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SaveConnectionReq {
    /// The name to save under.
    pub name: String,
    /// The connection's current name when editing an existing one (equal to
    /// `name` to update in place, different to rename). Absent creates a new
    /// connection, refused when `name` is taken.
    #[serde(default)]
    pub previous_name: Option<String>,
    /// The entry (url or structured form). `"__redacted__"` keeps the stored
    /// value of that field.
    pub connection: Value,
    /// Sensitive values sent apart from `connection`, in the same shape
    /// (`password`, `url`, `tls.client_key`, `ssh.password`,
    /// `ssh.private_key`, `ssh.passphrase`) and merged over it. Traces record
    /// a call's payload with keys like `password` redacted but not
    /// `passphrase`, `client_key` or a url's password; a key named
    /// `credentials` is redacted whole, so literals sent here stay out of
    /// them. Prefer `secret://` references, which are safe anywhere.
    #[serde(default)]
    pub credentials: Option<Value>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SaveConnectionResp {
    pub name: String,
    pub saved: bool,
    /// Why the saved connection has no pool after the reload (it is saved
    /// regardless). Absent when it is usable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_stage: Option<Stage>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteConnectionReq {
    pub name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DeleteConnectionResp {
    pub deleted: bool,
}

fn config_error(message: impl Into<String>) -> String {
    err_to_str(DbError::ConfigError {
        message: message.into(),
    })
}

pub(crate) fn sanitize_serde_error(e: &serde_json::Error) -> String {
    crate::config::sanitize_serde_error(&e.to_string())
}

/// Credential paths `credentials` may carry.
const CREDENTIAL_PATHS: [&[&str]; 6] = [
    &["password"],
    &["url"],
    &["tls", "client_key"],
    &["ssh", "password"],
    &["ssh", "private_key"],
    &["ssh", "passphrase"],
];

/// Merge a `credentials` object (same shape as an entry, sensitive fields
/// only) over `entry`. Anything else in it is refused, by path, so a client
/// cannot hide a whole entry from traces by sending it there.
pub(crate) fn merge_credentials(
    entry: &mut Value,
    credentials: Option<Value>,
) -> Result<(), String> {
    let Some(credentials) = credentials else {
        return Ok(());
    };
    if credentials.is_null() {
        return Ok(());
    }
    fn walk(
        prefix: &mut Vec<String>,
        value: &Value,
        out: &mut Vec<(Vec<String>, Value)>,
    ) -> Result<(), String> {
        match value {
            Value::Object(map) => {
                for (k, v) in map {
                    prefix.push(k.clone());
                    walk(prefix, v, out)?;
                    prefix.pop();
                }
                Ok(())
            }
            leaf => {
                out.push((prefix.clone(), leaf.clone()));
                Ok(())
            }
        }
    }
    if !credentials.is_object() {
        return Err("`credentials` must be an object".into());
    }
    let mut leaves = Vec::new();
    walk(&mut Vec::new(), &credentials, &mut leaves)?;
    for (path, value) in leaves {
        let allowed = CREDENTIAL_PATHS
            .iter()
            .any(|p| p.len() == path.len() && p.iter().zip(&path).all(|(a, b)| *a == b));
        if !allowed {
            return Err(format!(
                "`credentials.{}` is not a credential field (password, url, tls.client_key, \
                 ssh.password, ssh.private_key, ssh.passphrase)",
                path.join(".")
            ));
        }
        if !value.is_string() {
            return Err(format!("`credentials.{}` must be a string", path.join(".")));
        }
        let Some(obj) = entry.as_object_mut() else {
            return Err("`connection` must be an object".into());
        };
        match path.as_slice() {
            [field] => {
                obj.insert(field.clone(), value);
            }
            [parent, field] => {
                let child = obj
                    .entry(parent.clone())
                    .or_insert_with(|| Value::Object(Map::new()));
                match child.as_object_mut() {
                    Some(child) => {
                        child.insert(field.clone(), value);
                    }
                    None => return Err(format!("`connection.{parent}` must be an object")),
                }
            }
            _ => unreachable!("credential paths are one or two levels deep"),
        }
    }
    Ok(())
}

/// Parse and validate one entry as a connection named `name`.
pub(crate) fn parse_entry(name: &str, entry: &Value) -> Result<DatabaseConfig, String> {
    if !entry.is_object() {
        return Err("`connection` must be an object".into());
    }
    let mut parsed: DatabaseConfig = serde_json::from_value(entry.clone())
        .map_err(|e| format!("connection `{name}`: {}", sanitize_serde_error(&e)))?;
    parsed.validate(name)?;
    Ok(parsed)
}

fn client(state: &AppState) -> Result<&std::sync::Arc<iii_sdk::IIIClient>, String> {
    state.client().ok_or_else(|| {
        config_error("no engine connection; the configuration worker is unreachable")
    })
}

/// The stored `databases` map (raw), or an empty one.
fn stored_databases(raw: &Value) -> Map<String, Value> {
    raw.get("databases")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

/// One stored entry as written (raw), if it exists.
pub(crate) async fn stored_entry(state: &AppState, name: &str) -> Result<Option<Value>, String> {
    let iii = client(state)?;
    let raw = crate::configuration::get_raw_value(iii)
        .await
        .map_err(|e| config_error(format!("reading the stored configuration failed: {e}")))?;
    Ok(stored_databases(&raw).get(name).cloned())
}

pub async fn get(state: &AppState, req: GetConnectionReq) -> Result<GetConnectionResp, String> {
    let Some(mut entry) = stored_entry(state, &req.name).await? else {
        let mut available: Vec<String> = state
            .config
            .read()
            .await
            .databases
            .keys()
            .cloned()
            .collect();
        available.sort();
        return Err(err_to_str(DbError::UnknownDb {
            db: req.name,
            available,
        }));
    };
    redaction::redact_entry(&mut entry);
    Ok(GetConnectionResp {
        name: req.name,
        connection: entry,
    })
}

pub async fn save(state: &AppState, req: SaveConnectionReq) -> Result<SaveConnectionResp, String> {
    validate_connection_name(&req.name).map_err(config_error)?;
    let iii = client(state)?.clone();
    let _saving = state.runtime.saves.lock().await;

    let mut raw = crate::configuration::get_raw_value(&iii)
        .await
        .map_err(|e| config_error(format!("reading the stored configuration failed: {e}")))?;
    if raw.is_null() {
        raw = WorkerConfig::default().to_json();
    }
    let mut databases = stored_databases(&raw);

    let stored = match &req.previous_name {
        Some(previous) => {
            let Some(entry) = databases.get(previous) else {
                return Err(config_error(format!(
                    "no connection named `{previous}` to update; omit previous_name to create one"
                )));
            };
            if previous != &req.name && databases.contains_key(&req.name) {
                return Err(config_error(format!(
                    "a connection named `{}` already exists; pick another name",
                    req.name
                )));
            }
            Some(entry.clone())
        }
        None => {
            if databases.contains_key(&req.name) {
                return Err(config_error(format!(
                    "a connection named `{0}` already exists; pass previous_name: \"{0}\" to \
                     update it",
                    req.name
                )));
            }
            None
        }
    };

    let mut entry = req.connection;
    if let Some(field) = redaction::literal_sensitive_fields(&entry).first() {
        return Err(config_error(super::test_connection::literal_refusal(field)));
    }
    merge_credentials(&mut entry, req.credentials).map_err(config_error)?;
    redaction::restore_entry(&mut entry, stored.as_ref()).map_err(config_error)?;
    parse_entry(&req.name, &entry).map_err(config_error)?;

    if let Some(previous) = &req.previous_name {
        if previous != &req.name {
            databases.remove(previous);
        }
    }
    databases.insert(req.name.clone(), entry);
    if let Some(obj) = raw.as_object_mut() {
        obj.insert("databases".into(), Value::Object(databases));
    } else {
        return Err(config_error("the stored configuration is not an object"));
    }

    crate::configuration::set_value(&iii, raw)
        .await
        .map_err(|e| config_error(format!("configuration::set refused the value: {e}")))?;

    let (error, error_stage) = reload(state, &iii, &req.name).await;
    Ok(SaveConnectionResp {
        name: req.name,
        saved: true,
        error,
        error_stage,
    })
}

pub async fn delete(
    state: &AppState,
    req: DeleteConnectionReq,
) -> Result<DeleteConnectionResp, String> {
    let iii = client(state)?.clone();
    let _saving = state.runtime.saves.lock().await;
    let mut raw = crate::configuration::get_raw_value(&iii)
        .await
        .map_err(|e| config_error(format!("reading the stored configuration failed: {e}")))?;
    let mut databases = stored_databases(&raw);
    if databases.remove(&req.name).is_none() {
        return Ok(DeleteConnectionResp { deleted: false });
    }
    if databases.is_empty() {
        return Err(config_error(format!(
            "`{}` is the only connection; the configuration must declare at least one",
            req.name
        )));
    }
    match raw.as_object_mut() {
        Some(obj) => {
            obj.insert("databases".into(), Value::Object(databases));
        }
        None => return Err(config_error("the stored configuration is not an object")),
    }
    crate::configuration::set_value(&iii, raw)
        .await
        .map_err(|e| config_error(format!("configuration::set refused the value: {e}")))?;
    let _ = reload(state, &iii, &req.name).await;
    Ok(DeleteConnectionResp { deleted: true })
}

/// Apply the just-written configuration now (the change observer will find
/// nothing left to do) and report `name`'s build error, if any.
async fn reload(
    state: &AppState,
    iii: &iii_sdk::IIIClient,
    name: &str,
) -> (Option<String>, Option<Stage>) {
    match crate::configuration::fetch_config(iii).await {
        Ok(cfg) => {
            let _ = crate::configuration::apply_config(state, cfg, None).await;
            match state.runtime.error(name) {
                Some(e) => (Some(e.message), Some(e.stage)),
                None => (None, None),
            }
        }
        Err(e) => (
            Some(format!(
                "saved, but the configuration could not be reloaded: {e}"
            )),
            Some(Stage::Config),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serde_errors_never_quote_values() {
        let err = serde_json::from_value::<DatabaseConfig>(
            json!({ "driver": "postgres", "port": "hunter2" }),
        )
        .unwrap_err();
        let msg = sanitize_serde_error(&err);
        assert!(!msg.contains("hunter2"), "{msg}");
    }

    #[test]
    fn credentials_merge_over_the_entry_and_nothing_else_rides_along() {
        let mut entry = json!({ "driver": "postgres", "host": "db", "user": "u", "ssh": { "host": "b", "user": "t" } });
        merge_credentials(
            &mut entry,
            Some(json!({ "password": "pw", "tls": { "client_key": "KEY" }, "ssh": { "passphrase": "pp" } })),
        )
        .unwrap();
        assert_eq!(entry["password"], "pw");
        assert_eq!(entry["tls"]["client_key"], "KEY");
        assert_eq!(entry["ssh"]["passphrase"], "pp");
        assert_eq!(entry["ssh"]["host"], "b", "siblings are kept");
        let err = merge_credentials(&mut entry, Some(json!({ "host": "evil" }))).unwrap_err();
        assert!(
            err.contains("`credentials.host` is not a credential field"),
            "{err}"
        );
        let err = merge_credentials(&mut entry, Some(json!({ "ssh": { "port": 2 } }))).unwrap_err();
        assert!(err.contains("credentials.ssh.port"), "{err}");
        assert!(merge_credentials(&mut entry, Some(json!({ "password": 5 }))).is_err());
        merge_credentials(&mut entry, None).unwrap();
    }

    #[test]
    fn the_sdk_trace_redaction_hides_the_credentials_key_whole() {
        // What the SDK records for an invocation's input (iii-helpers'
        // redaction): `credentials` is redacted as one value.
        let payload = json!({ "connection": { "driver": "postgres", "ssh": { "passphrase": "LEAK1" } },
                              "credentials": { "ssh": { "passphrase": "SAFE" }, "url": "postgres://u:SAFE@h/db" } });
        let (recorded, _) = iii_helpers::observability::redact_and_truncate(&payload, None);
        assert!(!recorded.contains("SAFE"), "{recorded}");
        // The contract field names alone are not all redacted — the reason
        // `credentials` exists.
        assert!(recorded.contains("LEAK1"), "{recorded}");
    }

    #[test]
    fn parse_entry_validates_the_structured_form() {
        assert!(parse_entry(
            "shop",
            &json!({ "driver": "postgres", "host": "db", "user": "app" })
        )
        .is_ok());
        let err = parse_entry("shop", &json!({ "driver": "postgres", "user": "app" })).unwrap_err();
        assert!(err.contains("`host` is required"), "{err}");
        let err = parse_entry(
            "shop",
            &json!({ "url": "postgres://h/db", "driver": "postgres", "host": "db", "user": "u" }),
        )
        .unwrap_err();
        assert!(err.contains("mutually exclusive"), "{err}");
    }
}
