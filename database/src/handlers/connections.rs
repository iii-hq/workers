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

/// serde's messages quote offending values (`invalid type: string "…"`),
/// and a misplaced value may be a password: drop every quoted span.
pub(crate) fn sanitize_serde_error(e: &serde_json::Error) -> String {
    let text = e.to_string();
    let mut out = String::with_capacity(text.len());
    let mut in_quote = false;
    for c in text.chars() {
        if c == '"' {
            if !in_quote {
                out.push_str("\"…");
            } else {
                out.push('"');
            }
            in_quote = !in_quote;
        } else if !in_quote {
            out.push(c);
        }
    }
    out
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
