//! The operator functions (`mcp-client::*`), their wire types, and the one
//! table of their registration flags.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::{Capture, IIIClient, Observed, RegisterFunction, Source};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::{Effects, Issue, check_entry};
use crate::host::{EventKind, ServerEvent};
use crate::server::{LoginStart, Logs, Manager, ServersList, State};

pub const SERVERS_LIST: &str = "mcp-client::servers::list";
pub const SERVERS_APPROVE: &str = "mcp-client::servers::approve";
pub const SERVERS_RECONNECT: &str = "mcp-client::servers::reconnect";
pub const PROCESS_LOGS: &str = "mcp-client::process::logs";
pub const CONFIG_VALIDATE: &str = "mcp-client::config::validate";
pub const ON_SERVERS_CHANGED: &str = "mcp-client::on-servers-changed";
pub const ON_SECRET_CHANGE: &str = "mcp-client::on-secret-change";
pub const LOGIN_START: &str = "mcp-client::login::start";
pub const LOGIN_COMPLETE: &str = "mcp-client::login::complete";
pub const LOGIN_CANCEL: &str = "mcp-client::login::cancel";
pub const LOGIN_SIGN_OUT: &str = "mcp-client::login::sign-out";

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
const FLAGS: [(&str, Flags); 11] = [
    (SERVERS_LIST, OPERATOR),
    (SERVERS_APPROVE, OPERATOR),
    (SERVERS_RECONNECT, OPERATOR),
    (
        PROCESS_LOGS,
        Flags {
            observable: false,
            ..OPERATOR
        },
    ),
    (CONFIG_VALIDATE, OPERATOR),
    (ON_SECRET_CHANGE, OPERATOR),
    (
        ON_SERVERS_CHANGED,
        Flags {
            private: true,
            ..OPERATOR
        },
    ),
    (
        LOGIN_START,
        Flags {
            observable: false,
            ..OPERATOR
        },
    ),
    (
        LOGIN_COMPLETE,
        Flags {
            observable: false,
            ..OPERATOR
        },
    ),
    (LOGIN_CANCEL, OPERATOR),
    (LOGIN_SIGN_OUT, OPERATOR),
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
pub struct ServerRequest {
    pub server: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ApproveRequest {
    pub server: String,
    /// The fingerprint `config::validate` gave for the entry; without it, the
    /// entry as loaded.
    pub fingerprint: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ApproveResponse {
    pub approved: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StateResponse {
    pub state: State,
}

/// No `Debug`: the callback URL carries the code and `state`.
#[derive(Deserialize, JsonSchema)]
pub struct LoginCompleteRequest {
    /// The whole address the browser ended on after the sign-in.
    pub callback_url: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct LoginCompleteResponse {
    /// The server the sign-in was started for.
    pub server: String,
    pub state: State,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct EmptyResponse {}

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

/// A `secrets::on-changed` event: names a reference, never its value.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SecretChange {
    pub name: Option<String>,
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub action: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Ack {
    pub ok: bool,
}

/// `secrets::on-changed`, the event as the envelope's output. No filters:
/// the set of references changes with the config, so `event_matches` picks.
pub fn secret_change_source() -> Source {
    Source::function("secrets::on-changed").capture([Capture::Output])
}

type Sink = Arc<dyn Fn(ServerEvent) + Send + Sync>;

/// One delivery per `(server, kind)` per window: the first push at once,
/// and one more when the window ends if pushes arrived inside it.
pub struct Coalescer {
    sink: Sink,
    /// The open windows, each marked when a push arrived inside it.
    open: Arc<Mutex<HashMap<(String, EventKind), bool>>>,
}

impl Coalescer {
    pub fn new(sink: Sink) -> Arc<Coalescer> {
        Arc::new(Coalescer {
            sink,
            open: Arc::default(),
        })
    }

    pub fn push(&self, event: ServerEvent) {
        let key = (event.server.clone(), event.kind);
        match self.open.lock().unwrap().entry(key.clone()) {
            Entry::Occupied(mut window) => {
                window.insert(true);
                return;
            }
            Entry::Vacant(window) => {
                window.insert(false);
            }
        }
        (self.sink)(event);
        let (open, sink) = (self.open.clone(), self.sink.clone());
        let window = Duration::from_millis(match key.1 {
            EventKind::Stderr => 1_000,
            _ => 100,
        });
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(window).await;
                {
                    let mut open = open.lock().unwrap();
                    // Clears the mark of a window that goes on.
                    if open.get_mut(&key).map(std::mem::take) != Some(true) {
                        open.remove(&key);
                        return;
                    }
                }
                let (server, kind) = key.clone();
                sink(ServerEvent { server, kind });
            }
        });
    }
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
            SERVERS_APPROVE,
            schema_value::<ApproveRequest>(),
            schema_value::<ApproveResponse>(),
        ),
        (
            SERVERS_RECONNECT,
            schema_value::<ServerRequest>(),
            schema_value::<StateResponse>(),
        ),
        (
            PROCESS_LOGS,
            schema_value::<ServerRequest>(),
            schema_value::<Logs>(),
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
        (
            ON_SECRET_CHANGE,
            schema_value::<Observed<Value, SecretChange>>(),
            schema_value::<Ack>(),
        ),
        (
            LOGIN_START,
            schema_value::<ServerRequest>(),
            schema_value::<LoginStart>(),
        ),
        (
            LOGIN_COMPLETE,
            schema_value::<LoginCompleteRequest>(),
            schema_value::<LoginCompleteResponse>(),
        ),
        (
            LOGIN_CANCEL,
            schema_value::<ServerRequest>(),
            schema_value::<EmptyResponse>(),
        ),
        (
            LOGIN_SIGN_OUT,
            schema_value::<ServerRequest>(),
            schema_value::<StateResponse>(),
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
    let m = manager.clone();
    register(
        iii,
        SERVERS_APPROVE,
        "Approve an MCP server entry: the given fingerprint (from mcp-client::config::validate), else the entry as loaded. Only an approved entry runs.",
        RegisterFunction::new_async(move |request: ApproveRequest| {
            let m = m.clone();
            async move {
                m.approve(&request.server, request.fingerprint.as_deref())
                    .await
                    .map_err(Error::Handler)?;
                Ok::<ApproveResponse, Error>(ApproveResponse { approved: true })
            }
        }),
    );
    let m = manager.clone();
    register(
        iii,
        SERVERS_RECONNECT,
        "Connect an MCP server again now, with a fresh backoff. A disabled, unapproved or invalid entry is returned as it is.",
        RegisterFunction::new_async(move |request: ServerRequest| {
            let m = m.clone();
            async move {
                let state = m.reconnect(&request.server).await.map_err(Error::Handler)?;
                Ok::<StateResponse, Error>(StateResponse { state })
            }
        }),
    );
    let m = manager.clone();
    register(
        iii,
        PROCESS_LOGS,
        "The last 500 stderr lines of a stdio MCP server, with its secrets redacted, and how many older lines were dropped. An http server has none.",
        RegisterFunction::new_async(move |request: ServerRequest| {
            let m = m.clone();
            async move { m.logs(&request.server).await.map_err(Error::Handler) }
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
    let m = manager.clone();
    register(
        iii,
        ON_SECRET_CHANGE,
        "Internal: reconnect the MCP servers that send a secret:// or env:// value that was set, rotated or deleted.",
        RegisterFunction::new_async(move |observed: Observed<Value, SecretChange>| {
            let m = m.clone();
            async move {
                let Some(change) = observed.output else {
                    return Ok::<Ack, Error>(Ack { ok: false });
                };
                m.on_secret_change(&change).await;
                Ok(Ack { ok: true })
            }
        }),
    );
    let m = manager.clone();
    register(
        iii,
        LOGIN_START,
        "Start the sign-in of an MCP server that asks for one: the authorize URL to open in a browser, the redirect URI and when the sign-in expires. Starting again replaces the pending one.",
        RegisterFunction::new_async(move |request: ServerRequest| {
            let m = m.clone();
            async move { m.login_start(&request.server).await.map_err(Error::Handler) }
        }),
    );
    let m = manager.clone();
    register(
        iii,
        LOGIN_COMPLETE,
        "Finish a sign-in with the whole address the browser ended on; the server is found from that address, then connected.",
        RegisterFunction::new_async(move |request: LoginCompleteRequest| {
            let m = m.clone();
            async move {
                let (server, state) = m
                    .login_complete(&request.callback_url)
                    .await
                    .map_err(Error::Handler)?;
                Ok::<LoginCompleteResponse, Error>(LoginCompleteResponse { server, state })
            }
        }),
    );
    let m = manager.clone();
    register(
        iii,
        LOGIN_CANCEL,
        "Drop the pending sign-in of an MCP server; nothing pending is not an error.",
        RegisterFunction::new_async(move |request: ServerRequest| {
            let m = m.clone();
            async move {
                m.login_cancel(&request.server)
                    .await
                    .map_err(Error::Handler)?;
                Ok::<EmptyResponse, Error>(EmptyResponse {})
            }
        }),
    );
    let m = manager.clone();
    register(
        iii,
        LOGIN_SIGN_OUT,
        "Sign an MCP server out: its session ends and is cleared from the vault; its functions stay registered and answer that a sign-in is required.",
        RegisterFunction::new_async(move |request: ServerRequest| {
            let m = m.clone();
            async move {
                let state = m.sign_out(&request.server).await.map_err(Error::Handler)?;
                Ok::<StateResponse, Error>(StateResponse { state })
            }
        }),
    );
    // Pending while the secrets worker is absent; lives until disposed.
    iii.observe(secret_change_source(), ON_SECRET_CHANGE);
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
    fn every_flagged_function_is_in_the_catalog() {
        // `mcp-client::on-config-change` is in neither: config-client registers it.
        let mut flagged: Vec<&str> = FLAGS.iter().map(|(id, _)| *id).collect();
        let mut listed: Vec<&str> = catalog().into_iter().map(|(id, ..)| id).collect();
        flagged.sort();
        listed.sort();
        assert_eq!(flagged, listed);
    }

    #[tokio::test(start_paused = true)]
    async fn coalescer_delivers_the_first_and_the_last_event_of_a_window() {
        use tokio::time::sleep;

        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let coalescer =
            Coalescer::new(Arc::new(move |e: ServerEvent| sink.lock().unwrap().push(e)));
        let count = || seen.lock().unwrap().len();
        let event = |server: &str, kind| ServerEvent {
            server: server.into(),
            kind,
        };
        for _ in 0..50 {
            coalescer.push(event("a", EventKind::State));
        }
        coalescer.push(event("a", EventKind::Tools));
        coalescer.push(event("b", EventKind::State));
        tokio::task::yield_now().await;
        assert_eq!(count(), 3, "one per (server, kind), at once");
        sleep(Duration::from_millis(99)).await;
        assert_eq!(count(), 3);
        sleep(Duration::from_millis(2)).await;
        assert_eq!(
            count(),
            4,
            "the 49 folded pushes are one delivery when the window ends"
        );
        assert_eq!(seen.lock().unwrap()[3], event("a", EventKind::State));
        sleep(Duration::from_millis(300)).await;
        assert_eq!(count(), 4, "nothing pending, nothing sent");
        coalescer.push(event("a", EventKind::Stderr));
        coalescer.push(event("a", EventKind::Stderr));
        tokio::task::yield_now().await;
        assert_eq!(count(), 5);
        sleep(Duration::from_millis(999)).await;
        assert_eq!(count(), 5, "stderr has a one-second window");
        sleep(Duration::from_millis(2)).await;
        assert_eq!(count(), 6);
    }

    #[test]
    fn secret_change_is_read_from_the_observed_output() {
        // `fingerprint` and `updated_at` are fields of the event this worker does not read.
        let observed: iii_sdk::Observed<Value, SecretChange> = serde_json::from_value(json!({
            "observer_id": "obs-1", "activation_id": "act-1", "function_id": "secrets::on-changed",
            "invocation_id": "inv-1", "status": "success", "started_at_ms": 1, "duration_ms": 0.5,
            "output": {"name": "LINEAR_API_KEY", "ref": "secret://LINEAR_API_KEY", "action": "rotated",
                       "fingerprint": "9f2c", "updated_at": "2026-10-29T12:00:00Z"}
        }))
        .unwrap();
        let change = observed.output.unwrap();
        assert_eq!(change.name.as_deref(), Some("LINEAR_API_KEY"));
        assert_eq!(change.reference.as_deref(), Some("secret://LINEAR_API_KEY"));
        assert!(catalog().iter().any(|(id, ..)| *id == ON_SECRET_CHANGE));
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
        let approve: ApproveRequest =
            serde_json::from_value(json!({ "_caller_worker_id": "w1", "server": "linear" }))
                .unwrap();
        assert_eq!(
            (approve.server.as_str(), approve.fingerprint),
            ("linear", None)
        );
        assert!(
            serde_json::from_value::<ServerRequest>(
                json!({ "_caller_worker_id": "w1", "server": "linear" })
            )
            .is_ok()
        );
        assert!(
            serde_json::from_value::<LoginCompleteRequest>(
                json!({ "_caller_worker_id": "w1", "callback_url": "http://127.0.0.1/cb" })
            )
            .is_ok()
        );
    }
}
