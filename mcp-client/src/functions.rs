//! The operator functions (`mcp-client::*`), their wire types, and the one
//! table of their registration flags.

use std::collections::BTreeMap;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::{Effects, Issue, check_entry};
use crate::host::ServerEvent;
use crate::server::{Manager, ServersList};

pub const SERVERS_LIST: &str = "mcp-client::servers::list";
pub const CONFIG_VALIDATE: &str = "mcp-client::config::validate";
pub const ON_SERVERS_CHANGED: &str = "mcp-client::on-servers-changed";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flags {
    pub internal: bool,
    pub observable: bool,
    pub private: bool,
}

const OPERATOR: Flags = Flags {
    internal: true,
    observable: true,
    private: false,
};

/// One row per function `register_all` registers. A function whose payloads
/// carry a credential is `observable: false`; an event source is `private`.
const FLAGS: [(&str, Flags); 3] = [
    (SERVERS_LIST, OPERATOR),
    (CONFIG_VALIDATE, OPERATOR),
    (
        ON_SERVERS_CHANGED,
        Flags {
            private: true,
            ..OPERATOR
        },
    ),
];

/// Panics for an id with no row, so nothing registers without one.
pub fn flags(id: &str) -> Flags {
    FLAGS
        .iter()
        .find(|(row, _)| *row == id)
        .map(|(_, flags)| *flags)
        .unwrap_or_else(|| panic!("no registration flags for {id}"))
}

/// The only place the internal tag, `observable` and `private` are set.
fn with_flags(id: &str, f: RegisterFunction) -> RegisterFunction {
    let flags = flags(id);
    let f = if flags.internal {
        f.metadata(json!({ "internal": true }))
    } else {
        f
    };
    let f = f.observable(flags.observable);
    if flags.private { f.private() } else { f }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EmptyRequest {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ValidateRequest {
    /// The candidate map, keyed by server name, as the entry would store it.
    #[serde(rename = "mcpServers")]
    pub mcp_servers: BTreeMap<String, Value>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ValidateResponse {
    pub servers: BTreeMap<String, ServerValidation>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ServerValidation {
    pub ok: bool,
    pub errors: Vec<Issue>,
    pub warnings: Vec<Issue>,
    /// What an approval of this entry would cover.
    pub fingerprint: String,
    pub effects: Effects,
}

pub fn validate_map(request: &ValidateRequest, stdio_enabled: bool) -> ValidateResponse {
    let servers = request
        .mcp_servers
        .iter()
        .map(|(name, raw)| {
            let checked = check_entry(name, raw, stdio_enabled);
            let validation = ServerValidation {
                ok: checked.errors.is_empty(),
                errors: checked.errors,
                warnings: checked.warnings,
                fingerprint: checked.fingerprint,
                effects: checked.effects,
            };
            (name.clone(), validation)
        })
        .collect();
    ValidateResponse { servers }
}

fn schema_value<T: JsonSchema>() -> Value {
    let root = schemars::r#gen::SchemaGenerator::default().into_root_schema_for::<T>();
    let mut v = serde_json::to_value(root).expect("schema serializes");
    if let Some(obj) = v.as_object_mut() {
        obj.remove("$schema");
    }
    v
}

/// Id, request schema and response schema of every function `register_all`
/// registers, from the types its handlers take and return.
pub fn catalog() -> Vec<(&'static str, Value, Value)> {
    vec![
        (
            SERVERS_LIST,
            schema_value::<EmptyRequest>(),
            schema_value::<ServersList>(),
        ),
        (
            CONFIG_VALIDATE,
            schema_value::<ValidateRequest>(),
            schema_value::<ValidateResponse>(),
        ),
        (
            ON_SERVERS_CHANGED,
            schema_value::<ServerEvent>(),
            schema_value::<ServerEvent>(),
        ),
    ]
}

fn register(iii: &IIIClient, id: &'static str, description: &str, f: RegisterFunction) {
    iii.register_function(id, with_flags(id, f.description(description)));
}

pub fn register_all(iii: &Arc<IIIClient>, manager: &Arc<Manager>) {
    let m = manager.clone();
    register(
        iii,
        SERVERS_LIST,
        "Every configured MCP server: its state, the step it is at or stopped at, its error and the functions it registered. Never returns a token.",
        RegisterFunction::new_async(move |_: EmptyRequest| {
            let m = m.clone();
            async move { Ok::<ServersList, Error>(m.list().await) }
        }),
    );
    let stdio_enabled = manager.stdio_enabled();
    register(
        iii,
        CONFIG_VALIDATE,
        "Check a candidate mcpServers map without changing anything: errors and warnings per server, the fingerprint an approval would cover, and what approving would run or send.",
        RegisterFunction::new_async(move |request: ValidateRequest| async move {
            Ok::<ValidateResponse, Error>(validate_map(&request, stdio_enabled))
        }),
    );
    register(
        iii,
        ON_SERVERS_CHANGED,
        "Private: called by this worker when a server's state, tools, process, sign-in or stderr changes. Observe it to follow the MCP servers page.",
        RegisterFunction::new_async(
            |event: ServerEvent| async move { Ok::<ServerEvent, Error>(event) },
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_reports_each_server_on_its_own() {
        let request: ValidateRequest = serde_json::from_value(json!({
            "_caller_worker_id": "w1",
            "mcpServers": {
                "linear": { "type": "http", "url": "https://mcp.linear.app/mcp",
                    "auth": { "type": "bearer", "token": "secret://LINEAR_API_KEY" } },
                "bad": { "url": "https://x.example/mcp",
                    "auth": { "type": "bearer", "token": "lin_api_literal" } }
            }
        }))
        .unwrap();
        let response = serde_json::to_value(validate_map(&request, true)).unwrap();
        let linear = &response["servers"]["linear"];
        assert_eq!(linear["ok"], true, "{linear}");
        assert_eq!(linear["errors"], json!([]));
        assert_eq!(linear["fingerprint"].as_str().unwrap().len(), 64);
        assert_eq!(
            linear["effects"]["sends"],
            json!([{ "ref": "secret://LINEAR_API_KEY", "host": "mcp.linear.app" }])
        );
        let bad = &response["servers"]["bad"];
        assert_eq!(bad["ok"], false, "{bad}");
        assert_eq!(bad["errors"][0]["path"], "/mcpServers/bad/auth/token");
    }

    #[test]
    fn request_types_accept_the_key_the_engine_adds() {
        assert!(
            serde_json::from_value::<EmptyRequest>(json!({ "_caller_worker_id": "w1" })).is_ok()
        );
        assert!(
            serde_json::from_value::<ValidateRequest>(
                json!({ "_caller_worker_id": "w1", "mcpServers": {} })
            )
            .is_ok()
        );
    }
}
