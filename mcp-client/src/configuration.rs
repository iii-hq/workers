//! The worker's configuration entry: registered with the configuration
//! worker at boot, read raw, and reloaded on every change.

use std::future::Future;
use std::sync::Arc;

use iii_config_client as config_client;
use iii_sdk::errors::Error;
use iii_sdk::{CallRequest, IIIClient};
use serde_json::{Value, json};

use crate::config::RawConfig;
use crate::server::Manager;

pub const DEFAULT_CONFIG_ID: &str = "mcp-client";
pub const CONFIG_FN_ID: &str = "mcp-client::on-config-change";

/// `III_CONFIG_NAME` when a supervisor set it, else the built-in name, so two
/// instances can own two entries.
pub fn config_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        std::env::var("III_CONFIG_NAME")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_CONFIG_ID.to_string())
    })
    .as_str()
}

/// Loose on purpose: a pasted entry with unknown fields must store, and
/// `config::validate` names what is wrong with it.
pub fn entry_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "mcpServers": { "type": "object", "additionalProperties": { "type": "object" } }
        }
    })
}

pub fn default_value() -> Value {
    json!({ "mcpServers": {} })
}

fn spec() -> config_client::EntrySpec {
    config_client::EntrySpec {
        id: config_id(),
        form_id: DEFAULT_CONFIG_ID,
        name: "mcp-client",
        description: "MCP servers this worker connects to. Edit them on the MCP client page.",
        schema: entry_schema(),
        default_value: default_value(),
    }
}

/// Declares the entry and seeds the empty map only when nothing is stored.
pub async fn register_config(iii: &IIIClient) -> Result<(), String> {
    config_client::ensure(iii, &spec(), None).await
}

/// Raw: a stored `${VAR}` must reach validation unexpanded, so the
/// fingerprint covers what the page validated.
pub fn raw_get_request(id: &str) -> Value {
    json!({ "id": id, "raw": true })
}

fn config_from_get(result: Result<Value, Error>) -> Result<RawConfig, String> {
    match result {
        Ok(reply) => Ok(RawConfig::from_value(&reply["value"])),
        // Case-sensitive: a missing configuration worker answers lower-case
        // `function_not_found`, which is a failure, not an empty entry.
        Err(Error::Remote { code, .. }) if code == "NOT_FOUND" => Ok(RawConfig(Default::default())),
        Err(error) => Err(error.to_string()),
    }
}

pub async fn fetch_config_via<F, Fut>(call: F) -> Result<RawConfig, String>
where
    F: FnOnce(&'static str, Value) -> Fut,
    Fut: Future<Output = Result<Value, Error>>,
{
    config_from_get(call("configuration::get", raw_get_request(config_id())).await)
}

/// Never `config_client::fetch`: that read expands `${VAR}`.
pub async fn fetch_config(iii: &IIIClient) -> Result<RawConfig, String> {
    fetch_config_via(|function_id, payload| {
        iii.call(
            CallRequest {
                function_id: function_id.into(),
                payload,
                action: None,
                timeout_ms: Some(5_000),
            }
            .namespace("default"),
        )
    })
    .await
}

/// Registers `CONFIG_FN_ID` and observes the entry's updates with it. Run the
/// returned reload once after boot to close the fetch-to-observe gap.
pub fn register_config_observer(
    iii: &Arc<IIIClient>,
    manager: Arc<Manager>,
) -> config_client::Reload {
    let engine = iii.clone();
    config_client::on_change(
        iii,
        config_id(),
        CONFIG_FN_ID,
        "Internal: reload the MCP server list from the configuration entry on change.",
        move || {
            let (engine, manager) = (engine.clone(), manager.clone());
            async move {
                match fetch_config(&engine).await {
                    Ok(raw) => manager.apply_config(raw).await,
                    Err(error) => {
                        tracing::error!(%error, "config-change: keeping the previous servers")
                    }
                }
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn remote(code: &str) -> iii_sdk::errors::Error {
        iii_sdk::errors::Error::Remote {
            code: code.into(),
            message: "m".into(),
            stacktrace: None,
        }
    }

    #[test]
    fn fetch_is_raw_so_dollar_brace_reaches_validation() {
        assert_eq!(raw_get_request("x"), json!({ "id": "x", "raw": true }));
        let raw = config_from_get(Ok(
            json!({ "value": { "mcpServers": { "s": { "url": "https://x/${T}/mcp" } } } }),
        ))
        .unwrap();
        let checked = crate::config::check_entry("s", &raw.0["s"], true);
        assert!(
            checked.entry.is_none() && checked.errors.iter().any(|e| e.message.contains("env://")),
            "{:?}",
            checked.errors
        );
    }

    #[test]
    fn nothing_stored_is_the_empty_map_and_a_missing_worker_is_an_error() {
        assert!(
            config_from_get(Err(remote("NOT_FOUND")))
                .unwrap()
                .0
                .is_empty()
        );
        assert!(
            config_from_get(Ok(json!({ "value": null })))
                .unwrap()
                .0
                .is_empty()
        );
        assert!(config_from_get(Err(remote("function_not_found"))).is_err());
        assert!(config_from_get(Err(iii_sdk::errors::Error::Timeout)).is_err());
    }

    #[test]
    fn the_entry_schema_is_loose_and_the_default_is_the_empty_map() {
        assert_eq!(
            entry_schema(),
            json!({
                "type": "object",
                "properties": {
                    "mcpServers": { "type": "object", "additionalProperties": { "type": "object" } }
                }
            })
        );
        assert_eq!(default_value(), json!({ "mcpServers": {} }));
    }

    #[tokio::test]
    async fn fetch_config_sends_the_raw_read() {
        let seen = Mutex::new(None);
        let raw = fetch_config_via(|id, payload| {
            *seen.lock().unwrap() = Some((id, payload));
            async { Ok(json!({ "value": { "mcpServers": { "a": {} } } })) }
        })
        .await
        .unwrap();
        assert_eq!(
            seen.into_inner().unwrap(),
            Some((
                "configuration::get",
                json!({ "id": config_id(), "raw": true })
            ))
        );
        assert_eq!(raw.0.keys().collect::<Vec<_>>(), ["a"]);
    }
}
