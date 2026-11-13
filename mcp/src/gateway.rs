//! Gateway mode: a fixed set of tools that reach the whole engine, instead
//! of one MCP tool per function (hundreds on a real engine).
//!
//!   iii_call       call any function by id
//!   iii_observe    observe a function; events buffer in the MCP session
//!   iii_events     read (and remove) buffered events
//!   iii_observers  this session's observers
//!   iii_unobserve  remove one
//!   iii_status     what this server is, and whether RBAC is on
//!   iii_list_agents, iii_get_agent, iii_list_skills, iii_get_skill
//!
//! What a call may reach is decided by RBAC (rbac-proxy) on the connection,
//! not here; the only local rule is that `mcp::` internals are refused.

use serde_json::{json, Value};

use crate::bus::Bus;
use crate::directory_bridge;
use crate::rbac_guard::RbacMode;
use crate::sessions::{Observe, ObserveRequest, Sessions};

pub const TOOL_NAMES: [&str; 11] = [
    "iii_call",
    "iii_secret_store",
    "iii_observe",
    "iii_events",
    "iii_observers",
    "iii_unobserve",
    "iii_status",
    "iii_list_agents",
    "iii_get_agent",
    "iii_list_skills",
    "iii_get_skill",
];

const DEFAULT_MAX_CHARS: usize = 100_000;
const INTERNAL_PREFIX: &str = "mcp::";
/// The one `secrets` function that answers with a value. `iii_call` refuses
/// it: this bridge hands references on, never values back to a client.
const RESOLVE_FN: &str = "secrets::resolve";
/// What [`iii_secret_store`] calls: `secrets::set` writes the vault.
const SECRET_SET_FN: &str = "secrets::set";
/// This worker's name. It never resolves a secret, so it is never a consumer.
const SELF_NAME: &str = "mcp";

/// What one gateway tool call runs against.
pub struct Ctx<'a, B> {
    pub bus: &'a B,
    pub sessions: &'a Sessions,
    /// The request's `Mcp-Session-Id`, when it carries a live one.
    pub session_id: Option<&'a str>,
    pub mode: &'a RbacMode,
    /// The function every observer targets (`mcp::observed`).
    pub sink: &'a str,
}

const INSTRUCTIONS: &str = "The running iii engine. Everything on it is a function, called by id. Act on the engine with iii_call (any function id, with a JSON payload) instead of the `iii call` CLI or curl; discover ids with iii_call on engine::functions::list or directory::search_functions, and read a contract with engine::functions::info before its first call. React to the engine with iii_observe instead of polling: it buffers every matching invocation of a function for this session; read them with iii_events, list them with iii_observers, and remove each one with iii_unobserve when done. Profiles written for the iii harness say agent_call and harness::observe: here those are iii_call and iii_observe. Agent profiles: iii_list_agents / iii_get_agent. Skills: iii_list_skills / iii_get_skill (also resources at iii://<id>). iii_status says whether RBAC is on. Never put a credential in an iii_call payload: store it with iii_secret_store and pass the secret://NAME reference it returns.";

/// The server instructions MCP clients show the model.
pub fn instructions(mode: &RbacMode) -> String {
    match mode.client_notice() {
        Some(notice) => format!("{notice} {INSTRUCTIONS}"),
        None => INSTRUCTIONS.to_string(),
    }
}

fn tool(name: &str, description: &str, schema: Value) -> Value {
    json!({ "name": name, "description": description, "inputSchema": schema })
}

pub fn tools_list() -> Value {
    let id_only = |what: &str| json!({ "type": "object", "properties": { "id": { "type": "string", "description": what } }, "required": ["id"] });
    json!({ "tools": [
        tool("iii_call", "Call any function on the iii engine by id and return its result. The way to act on the engine: use it instead of the `iii call` CLI. Find ids with engine::functions::list or directory::search_functions, and read a contract with engine::functions::info before the first call.", json!({
            "type": "object",
            "properties": {
                "function_id": { "type": "string", "description": "e.g. `engine::workers::list`" },
                "payload": { "type": "object", "description": "The function arguments (a JSON object). Default {}." },
                "timeout_ms": { "type": "number" },
                "void": { "type": "boolean", "description": "Fire and forget: return without waiting for a result." },
                "max_chars": { "type": "number", "description": "Cap on the returned JSON (default 100000)." }
            },
            "required": ["function_id"]
        })),
        tool("iii_secret_store", "Store a credential (API key, token, password) in the iii secrets vault and get back its secret://NAME reference. Pass that reference wherever the credential is needed instead of the value: only the workers named in consumers can resolve it. Use it whenever you hold a credential; never put one in an iii_call payload. Storing an existing name rotates it.", json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "pattern": "^[A-Za-z_][A-Za-z0-9_.-]{0,127}$", "description": "Secret name, e.g. STRIPE_API_KEY. The reference is secret://<name>." },
                "value": { "type": "string", "minLength": 1, "description": "The credential, stored as given." },
                "consumers": { "type": "array", "items": { "type": "string" }, "minItems": 1, "description": "Worker names allowed to resolve it, e.g. [\"llm-router\"]." },
                "description": { "type": "string", "description": "What it is for." }
            },
            "required": ["name", "value", "consumers"]
        })),
        tool("iii_observe", "Observe a function on the iii engine: every matching invocation of it is buffered for this MCP session. Returns an observer_id; read what happened with iii_events, remove it with iii_unobserve. Use it instead of polling.", json!({
            "type": "object",
            "properties": {
                "function_id": { "type": "string", "description": "The function to observe, e.g. `state::set`." },
                "filters": { "type": "object", "description": "Which calls match, e.g. { \"input.scope\": { \"$eq\": \"jobs\" } }. Default: every call." },
                "args": { "type": "object", "description": "The observed function's args_format (see engine::functions::info)." },
                "capture": { "type": "array", "items": { "type": "string", "enum": ["input", "output", "error", "caller", "metadata"] }, "description": "What each event carries. Default input, output, caller (successes only); add error for failures." },
                "namespace": { "type": "string" },
                "max_events": { "type": "number", "description": "Remove the observer after this many matches." },
                "expires_in_ms": { "type": "number", "description": "Remove the observer after this long." }
            },
            "required": ["function_id"]
        })),
        tool("iii_events", "Read (and remove) the buffered events of one observer, or of all of this session's, oldest first. Reports how many remain and how many were dropped because the buffer was full.", json!({
            "type": "object",
            "properties": {
                "observer_id": { "type": "string", "description": "Omit for every observer." },
                "max": { "type": "number", "description": "Default 50." }
            }
        })),
        tool("iii_observers", "This session's observers: function, filters, state, and buffered / received / dropped counts.", json!({ "type": "object", "properties": {} })),
        tool("iii_unobserve", "Remove an observer created with iii_observe, discarding its unread events.", json!({
            "type": "object", "properties": { "observer_id": { "type": "string" } }, "required": ["observer_id"]
        })),
        tool("iii_status", "This MCP server: version, session, and whether RBAC (rbac-proxy) governs what it may call.", json!({ "type": "object", "properties": {} })),
        tool("iii_list_agents", "List the agent profiles installed in the iii engine (id, name, description, model, counts).", json!({
            "type": "object", "properties": { "include_hidden": { "type": "boolean", "description": "Include hidden helper profiles." } }
        })),
        tool("iii_get_agent", "Fetch one agent profile: its full system prompt (resolved through extends), preloaded skills and functions.", id_only("Profile id")),
        tool("iii_list_skills", "List skills installed in the iii engine. Filter with search (substring) or prefix (worker namespace).", json!({
            "type": "object", "properties": { "search": { "type": "string" }, "prefix": { "type": "string" } }
        })),
        tool("iii_get_skill", "Fetch one skill by id and return its markdown body.", id_only("Skill id, e.g. kanban")),
    ]})
}

fn text(v: &Value) -> Value {
    let body = match v {
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    };
    json!({ "content": [{ "type": "text", "text": body }] })
}

fn error(message: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": message.into() }], "isError": true })
}

fn str_arg<'v>(args: &'v Value, key: &str) -> Option<&'v str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn need_session<'a, B>(ctx: &Ctx<'a, B>) -> Result<&'a str, Value> {
    ctx.session_id.ok_or_else(|| {
        error("this tool needs an MCP session: initialize first and send the Mcp-Session-Id header the server returned")
    })
}

async fn iii_call<B: Bus>(ctx: &Ctx<'_, B>, args: &Value) -> Value {
    let Some(function_id) = str_arg(args, "function_id") else {
        return error("function_id is required");
    };
    if function_id == RESOLVE_FN {
        return error(format!("`{RESOLVE_FN}` returns a secret's value, and this bridge never hands one back. Pass the secret://NAME reference to the function that needs it; that worker resolves it."));
    }
    if function_id.starts_with(INTERNAL_PREFIX) {
        return error(format!("`{function_id}`: `{INTERNAL_PREFIX}` functions are this bridge's internals and cannot be called through it"));
    }
    let payload = match args.get("payload") {
        None | Some(Value::Null) => json!({}),
        Some(p) if p.is_object() => p.clone(),
        Some(_) => return error("payload must be a JSON object"),
    };
    if args.get("void").and_then(Value::as_bool) == Some(true) {
        return match ctx.bus.call_void(function_id, payload).await {
            Ok(()) => text(&json!({ "result": null, "void": true })),
            Err(e) => error(e.to_string()),
        };
    }
    let timeout = args.get("timeout_ms").and_then(Value::as_u64);
    match ctx.bus.call(function_id, payload, timeout).await {
        Ok(result) => {
            let max = args
                .get("max_chars")
                .and_then(Value::as_u64)
                .filter(|m| *m > 0)
                .map(|m| m as usize)
                .unwrap_or(DEFAULT_MAX_CHARS);
            let serialized = serde_json::to_string(&result).unwrap_or_default();
            let total = serialized.chars().count();
            if total <= max {
                text(&json!({ "result": result }))
            } else {
                let cut: String = serialized.chars().take(max).collect();
                text(&json!({ "result": cut, "truncated": true, "total_chars": total }))
            }
        }
        Err(e) => error(e.to_string()),
    }
}

fn valid_secret_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && name.len() <= 128
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Store a credential with `secrets::set` and answer with its reference only.
/// No message built here, or passed on from `secrets`, carries the value.
async fn iii_secret_store<B: Bus>(ctx: &Ctx<'_, B>, args: &Value) -> Value {
    let Some(name) = args.get("name").and_then(Value::as_str) else {
        return error("name is required");
    };
    if !valid_secret_name(name) {
        return error("name must match ^[A-Za-z_][A-Za-z0-9_.-]{0,127}$");
    }
    if name == "III_SECRETS_KEY" {
        return error("name III_SECRETS_KEY is reserved");
    }
    let value = match args.get("value") {
        Some(Value::String(v)) if !v.is_empty() => v.clone(),
        _ => return error("value is required and must be a non-empty string"),
    };
    let consumers: Option<Vec<String>> = args
        .get("consumers")
        .and_then(Value::as_array)
        .filter(|list| !list.is_empty())
        .and_then(|list| {
            list.iter()
                .map(|c| {
                    c.as_str()
                        .map(str::trim)
                        .filter(|c| !c.is_empty())
                        .map(String::from)
                })
                .collect()
        });
    let Some(consumers) = consumers else {
        return error("consumers must list at least one worker name that may resolve the secret");
    };
    if consumers.iter().any(|c| c == SELF_NAME) {
        return error("consumers cannot include mcp: this bridge passes references on and never resolves a value");
    }
    let mut payload = json!({ "name": name, "value": value, "consumers": consumers });
    match args.get("description") {
        None | Some(Value::Null) => {}
        Some(Value::String(d)) => payload["description"] = Value::String(d.clone()),
        Some(_) => return error("description must be a string"),
    }
    match ctx.bus.call(SECRET_SET_FN, payload, None).await {
        Ok(stored) => {
            let mut out = serde_json::Map::new();
            for key in ["ref", "hint", "fingerprint", "consumers"] {
                if let Some(v) = stored.get(key) {
                    out.insert(key.to_string(), v.clone());
                }
            }
            text(&Value::Object(out))
        }
        Err(e) if e.is_missing_function() => error(
            "the secrets worker is not running: add it with compose::add worker=secrets, then try again",
        ),
        Err(e) => error(e.to_string()),
    }
}

/// Run a gateway tool; `None` when `name` is not one of [`TOOL_NAMES`].
pub async fn call_tool<B: Bus + Observe>(
    ctx: &Ctx<'_, B>,
    name: &str,
    args: &Value,
) -> Option<Value> {
    let result = match name {
        "iii_call" => iii_call(ctx, args).await,
        "iii_secret_store" => iii_secret_store(ctx, args).await,
        "iii_observe" => match need_session(ctx) {
            Err(e) => e,
            Ok(sid) => match serde_json::from_value::<ObserveRequest>(args.clone()) {
                Err(e) => error(format!("invalid iii_observe arguments: {e}")),
                Ok(req) => match ctx.sessions.observe(ctx.bus, sid, req, ctx.sink).await {
                    Ok(v) => text(&v),
                    Err(e) => error(e),
                },
            },
        },
        "iii_events" => match need_session(ctx) {
            Err(e) => e,
            Ok(sid) => {
                let max = args.get("max").and_then(Value::as_u64).unwrap_or(50) as usize;
                match ctx.sessions.events(sid, str_arg(args, "observer_id"), max) {
                    Ok(v) => text(&v),
                    Err(e) => error(e),
                }
            }
        },
        "iii_observers" => match need_session(ctx) {
            Err(e) => e,
            Ok(sid) => match ctx.sessions.list(sid) {
                Ok(v) => text(&Value::Array(v)),
                Err(e) => error(e),
            },
        },
        "iii_unobserve" => match need_session(ctx) {
            Err(e) => e,
            Ok(sid) => match str_arg(args, "observer_id") {
                None => error("observer_id is required"),
                Some(oid) => match ctx.sessions.unobserve(sid, oid).await {
                    Ok(v) => text(&v),
                    Err(e) => error(e),
                },
            },
        },
        "iii_status" => {
            let mut v = json!({
                "server": env!("CARGO_PKG_NAME"),
                "version": env!("CARGO_PKG_VERSION"),
                "session_id": ctx.session_id,
            });
            match ctx.mode {
                RbacMode::Proxied { policy } => {
                    v["rbac"] = json!("rbac-proxy");
                    v["policy"] = policy.clone();
                }
                RbacMode::Dangerous => v["rbac"] = json!("disabled"),
            }
            if let Some(notice) = ctx.mode.client_notice() {
                v["warning"] = json!(notice);
            }
            text(&v)
        }
        "iii_list_agents" => {
            let hidden = args.get("include_hidden").and_then(Value::as_bool) == Some(true);
            match directory_bridge::list_agents(ctx.bus, hidden).await {
                Ok(v) => text(&v),
                Err(e) => error(e.to_string()),
            }
        }
        "iii_get_agent" => match str_arg(args, "id") {
            None => error("id is required"),
            Some(id) => match directory_bridge::get_agent(ctx.bus, id).await {
                Ok(v) => text(&v),
                Err(e) => error(e.to_string()),
            },
        },
        "iii_list_skills" => match directory_bridge::list_skills(
            ctx.bus,
            str_arg(args, "search"),
            str_arg(args, "prefix"),
        )
        .await
        {
            Ok(v) => text(&v),
            Err(e) => error(e.to_string()),
        },
        "iii_get_skill" => match str_arg(args, "id") {
            None => error("id is required"),
            Some(id) => match directory_bridge::get_skill_body(ctx.bus, id).await {
                Ok(body) => text(&Value::String(body)),
                Err(e) => error(e.to_string()),
            },
        },
        _ => return None,
    };
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::fake::FakeBus;
    use crate::rbac_guard::{RbacMode, DANGEROUS_NOTICE};
    use crate::sessions::{SessionLimits, Sessions};
    use crate::testkit::Engine;
    use serde_json::json;

    fn proxied() -> RbacMode {
        RbacMode::Proxied {
            policy: json!({ "auth_function": true }),
        }
    }

    fn sessions() -> Sessions {
        Sessions::new(SessionLimits::default())
    }

    async fn tool(
        e: &Engine,
        s: &Sessions,
        sid: Option<&str>,
        name: &str,
        args: Value,
    ) -> (bool, Value) {
        let ctx = Ctx {
            bus: e,
            sessions: s,
            session_id: sid,
            mode: &proxied(),
            sink: "mcp::observed",
        };
        let r = call_tool(&ctx, name, &args).await.expect("a gateway tool");
        let text = r["content"][0]["text"].as_str().unwrap().to_string();
        let is_error = r.get("isError").and_then(Value::as_bool) == Some(true);
        (
            is_error,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        )
    }

    #[test]
    fn the_tool_list_is_the_fixed_gateway_set() {
        let list = tools_list();
        let names: Vec<&str> = list["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, TOOL_NAMES.to_vec());
        let required = |n: &str| {
            list["tools"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["name"] == n)
                .unwrap()["inputSchema"]["required"]
                .clone()
        };
        assert_eq!(required("iii_call"), json!(["function_id"]));
        assert_eq!(required("iii_observe"), json!(["function_id"]));
        assert_eq!(required("iii_unobserve"), json!(["observer_id"]));
        assert_eq!(required("iii_get_agent"), json!(["id"]));
    }

    #[test]
    fn instructions_point_at_call_and_observe_and_warn_without_rbac() {
        let i = instructions(&proxied());
        for t in ["iii_call", "iii_observe", "iii_events", "iii_unobserve"] {
            assert!(i.contains(t), "{t}");
        }
        assert!(!i.contains("WITHOUT RBAC"));
        assert!(instructions(&RbacMode::Dangerous).starts_with(DANGEROUS_NOTICE));
    }

    #[tokio::test]
    async fn unknown_tool_names_are_not_gateway_tools() {
        let e = Engine::new(FakeBus::new());
        let s = sessions();
        let ctx = Ctx {
            bus: &e,
            sessions: &s,
            session_id: None,
            mode: &proxied(),
            sink: "mcp::observed",
        };
        assert!(call_tool(&ctx, "myworker__echo", &json!({}))
            .await
            .is_none());
    }

    const SECRET: &str = "sk-live-0123456789abcdef";

    fn stored(e: &Engine) -> Vec<Value> {
        e.bus
            .calls()
            .into_iter()
            .filter(|(f, _)| f == "secrets::set")
            .map(|(_, p)| p)
            .collect()
    }

    #[test]
    fn secret_store_is_a_gateway_tool_needing_name_value_and_consumers() {
        let list = tools_list();
        let t = list["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "iii_secret_store")
            .expect("iii_secret_store is listed")
            .clone();
        assert_eq!(
            t["inputSchema"]["required"],
            json!(["name", "value", "consumers"])
        );
        let d = t["description"].as_str().unwrap();
        assert!(d.contains("secret://"), "{d}");
    }

    #[tokio::test]
    async fn secret_store_writes_the_vault_and_returns_only_the_reference() {
        let e = Engine::new(FakeBus::new().on("secrets::set", |_| {
            // Whatever secrets answers, a value must not come back out.
            Ok(json!({
                "name": "STRIPE_KEY", "ref": "secret://STRIPE_KEY", "store": "vault",
                "hint": "sk-liv…cdef", "fingerprint": "3f9a0c1d2e4b5a67",
                "consumers": ["billing"], "value": SECRET,
                "created_at": "t", "updated_at": "t"
            }))
        }));
        let (err, v) = tool(
            &e,
            &sessions(),
            None,
            "iii_secret_store",
            json!({ "name": "STRIPE_KEY", "value": SECRET, "consumers": ["billing"],
                    "description": "Stripe live key" }),
        )
        .await;
        assert!(!err, "{v}");
        assert_eq!(
            v,
            json!({ "ref": "secret://STRIPE_KEY", "hint": "sk-liv…cdef",
                    "fingerprint": "3f9a0c1d2e4b5a67", "consumers": ["billing"] })
        );
        assert_eq!(
            stored(&e),
            vec![
                json!({ "name": "STRIPE_KEY", "value": SECRET, "consumers": ["billing"],
                         "description": "Stripe live key" })
            ]
        );
    }

    #[tokio::test]
    async fn secret_store_passes_the_value_byte_for_byte() {
        let e = Engine::new(FakeBus::new().on("secrets::set", |_| {
            Ok(json!({ "ref": "secret://K", "hint": "h", "consumers": ["w"] }))
        }));
        let raw = "  line1\nline2  ";
        let (err, _) = tool(
            &e,
            &sessions(),
            None,
            "iii_secret_store",
            json!({ "name": "K", "value": raw, "consumers": ["w"] }),
        )
        .await;
        assert!(!err);
        assert_eq!(stored(&e)[0]["value"], raw);
        assert!(stored(&e)[0].get("description").is_none());
    }

    #[tokio::test]
    async fn secret_store_refuses_bad_arguments_without_echoing_the_value() {
        let e = Engine::new(FakeBus::new());
        let s = sessions();
        for (args, mentions) in [
            (json!({ "value": SECRET, "consumers": ["w"] }), "name"),
            (
                json!({ "name": "1BAD", "value": SECRET, "consumers": ["w"] }),
                "name",
            ),
            (
                json!({ "name": "has space", "value": SECRET, "consumers": ["w"] }),
                "name",
            ),
            (
                json!({ "name": "III_SECRETS_KEY", "value": SECRET, "consumers": ["w"] }),
                "reserved",
            ),
            (json!({ "name": "K", "consumers": ["w"] }), "value"),
            (
                json!({ "name": "K", "value": "", "consumers": ["w"] }),
                "value",
            ),
            (
                json!({ "name": "K", "value": 42, "consumers": ["w"] }),
                "value",
            ),
            (json!({ "name": "K", "value": SECRET }), "consumers"),
            (
                json!({ "name": "K", "value": SECRET, "consumers": [] }),
                "consumers",
            ),
            (
                json!({ "name": "K", "value": SECRET, "consumers": ["w", 3] }),
                "consumers",
            ),
            (
                json!({ "name": "K", "value": SECRET, "consumers": [" "] }),
                "consumers",
            ),
            (
                json!({ "name": "K", "value": SECRET, "consumers": ["w"], "description": 1 }),
                "description",
            ),
        ] {
            let (err, v) = tool(&e, &s, None, "iii_secret_store", args.clone()).await;
            let msg = v.to_string();
            assert!(err, "{args}");
            assert!(msg.contains(mentions), "{args} -> {msg}");
            assert!(!msg.contains(SECRET), "value echoed: {msg}");
        }
        assert!(e.bus.calls().is_empty());
    }

    #[tokio::test]
    async fn secret_store_never_makes_mcp_a_consumer() {
        // mcp hands references on; it never resolves a value back to a client.
        let e = Engine::new(FakeBus::new());
        let (err, v) = tool(
            &e,
            &sessions(),
            None,
            "iii_secret_store",
            json!({ "name": "K", "value": SECRET, "consumers": ["billing", "mcp"] }),
        )
        .await;
        assert!(err);
        assert!(v.as_str().unwrap().contains("mcp"), "{v}");
        assert!(e.bus.calls().is_empty());
    }

    #[tokio::test]
    async fn secret_store_reports_a_secrets_error_without_the_value() {
        let e = Engine::new(FakeBus::new().on("secrets::set", |_| {
            Err(crate::bus::BusError::new("VAULT_ERROR", "vault is locked"))
        }));
        let (err, v) = tool(
            &e,
            &sessions(),
            None,
            "iii_secret_store",
            json!({ "name": "K", "value": SECRET, "consumers": ["w"] }),
        )
        .await;
        assert!(err);
        let msg = v.as_str().unwrap();
        assert!(
            msg.contains("VAULT_ERROR") && !msg.contains(SECRET),
            "{msg}"
        );
    }

    #[tokio::test]
    async fn secret_store_explains_a_missing_secrets_worker() {
        let e = Engine::new(FakeBus::new());
        let (err, v) = tool(
            &e,
            &sessions(),
            None,
            "iii_secret_store",
            json!({ "name": "K", "value": SECRET, "consumers": ["w"] }),
        )
        .await;
        assert!(err);
        let msg = v.as_str().unwrap();
        assert!(
            msg.contains("secrets") && msg.contains("compose::add"),
            "{msg}"
        );
        assert!(!msg.contains(SECRET));
    }

    #[tokio::test]
    async fn iii_call_cannot_resolve_a_secret_value() {
        let e = Engine::new(FakeBus::new());
        for id in ["secrets::resolve", " secrets::resolve "] {
            let (err, v) = tool(
                &e,
                &sessions(),
                None,
                "iii_call",
                json!({ "function_id": id, "payload": { "ref": "secret://K" } }),
            )
            .await;
            assert!(err);
            assert!(v.as_str().unwrap().contains("secret://"), "{v}");
        }
        assert!(e.bus.calls().is_empty());
    }

    #[tokio::test]
    async fn iii_call_returns_the_result_and_defaults_the_payload() {
        let e = Engine::new(FakeBus::new().on("engine::workers::list", |p| {
            assert_eq!(p, json!({}));
            Ok(json!({ "workers": 3 }))
        }));
        let (err, v) = tool(
            &e,
            &sessions(),
            None,
            "iii_call",
            json!({ "function_id": "engine::workers::list" }),
        )
        .await;
        assert!(!err);
        assert_eq!(v, json!({ "result": { "workers": 3 } }));
    }

    #[tokio::test]
    async fn iii_call_reports_engine_errors_with_their_code() {
        let e = Engine::new(FakeBus::new());
        let (err, v) = tool(
            &e,
            &sessions(),
            None,
            "iii_call",
            json!({ "function_id": "nope::x" }),
        )
        .await;
        assert!(err);
        assert!(v.as_str().unwrap().contains("function_not_found"), "{v}");
    }

    #[tokio::test]
    async fn iii_call_validates_its_arguments() {
        let e = Engine::new(FakeBus::new());
        let s = sessions();
        let (err, v) = tool(&e, &s, None, "iii_call", json!({ "function_id": " " })).await;
        assert!(err && v.as_str().unwrap().contains("function_id"));
        let (err, v) = tool(
            &e,
            &s,
            None,
            "iii_call",
            json!({ "function_id": "a::b", "payload": [1] }),
        )
        .await;
        assert!(err && v.as_str().unwrap().contains("payload"));
        assert!(e.bus.calls().is_empty());
    }

    #[tokio::test]
    async fn iii_call_refuses_the_bridges_own_internals() {
        let e = Engine::new(FakeBus::new().on("mcp::observed", |_| Ok(json!({}))));
        let (err, v) = tool(
            &e,
            &sessions(),
            None,
            "iii_call",
            json!({ "function_id": "mcp::observed" }),
        )
        .await;
        assert!(err && v.as_str().unwrap().contains("mcp::"), "{v}");
        assert!(e.bus.calls().is_empty());
    }

    #[tokio::test]
    async fn iii_call_truncates_a_large_result_and_says_so() {
        let e =
            Engine::new(FakeBus::new().on("big::one", |_| Ok(json!({ "x": "y".repeat(5000) }))));
        let (_, v) = tool(
            &e,
            &sessions(),
            None,
            "iii_call",
            json!({ "function_id": "big::one", "max_chars": 100 }),
        )
        .await;
        assert_eq!(v["truncated"], true);
        assert_eq!(v["result"].as_str().unwrap().chars().count(), 100);
        assert!(v["total_chars"].as_u64().unwrap() > 5000);
    }

    #[tokio::test]
    async fn iii_call_fires_and_forgets_with_void() {
        let e = Engine::new(FakeBus::new().on("a::b", |_| Ok(json!("ignored"))));
        let (err, v) = tool(
            &e,
            &sessions(),
            None,
            "iii_call",
            json!({ "function_id": "a::b", "void": true }),
        )
        .await;
        assert!(!err);
        assert_eq!(v, json!({ "result": null, "void": true }));
        assert_eq!(e.bus.calls()[0].0, "void:a::b");
    }

    #[tokio::test]
    async fn observe_tools_need_an_mcp_session() {
        let e = Engine::new(FakeBus::new());
        let s = sessions();
        for (name, args) in [
            ("iii_observe", json!({ "function_id": "a::b" })),
            ("iii_events", json!({})),
            ("iii_observers", json!({})),
            ("iii_unobserve", json!({ "observer_id": "x" })),
        ] {
            let (err, v) = tool(&e, &s, None, name, args).await;
            assert!(err, "{name}");
            assert!(
                v.as_str().unwrap().contains("Mcp-Session-Id"),
                "{name}: {v}"
            );
        }
    }

    #[tokio::test]
    async fn observe_events_observers_unobserve_end_to_end() {
        let e = Engine::new(FakeBus::new());
        let s = sessions();
        let sid = s.create();
        let (err, v) = tool(
            &e,
            &s,
            Some(&sid),
            "iii_observe",
            json!({ "function_id": "state::set" }),
        )
        .await;
        assert!(!err, "{v}");
        assert_eq!(v["observer_id"], "obs-1");
        s.deliver(json!({ "observer_id": "obs-1", "input": { "k": 1 } }));
        let (_, listed) = tool(&e, &s, Some(&sid), "iii_observers", json!({})).await;
        assert_eq!(listed[0]["buffered"], 1);
        let (_, ev) = tool(
            &e,
            &s,
            Some(&sid),
            "iii_events",
            json!({ "observer_id": "obs-1", "max": 10 }),
        )
        .await;
        assert_eq!(ev["events"][0]["input"], json!({ "k": 1 }));
        let (_, gone) = tool(
            &e,
            &s,
            Some(&sid),
            "iii_unobserve",
            json!({ "observer_id": "obs-1" }),
        )
        .await;
        assert_eq!(gone["removed"], true);
    }

    #[tokio::test]
    async fn profile_and_skill_tools_read_the_directory() {
        let e = Engine::new(
            FakeBus::new()
                .on("directory::agents::list", |_| {
                    Ok(json!({ "agents": [{ "id": "writer" }, { "id": "h", "hidden": true }] }))
                })
                .on("directory::agents::get", |p| {
                    Ok(json!({ "id": p["id"], "system_prompt": "Be." }))
                })
                .on("directory::skills::list", |_| {
                    Ok(json!({ "skills": [{ "id": "kanban" }] }))
                })
                .on("directory::skills::get", |_| {
                    Ok(json!({ "body": "# kanban" }))
                }),
        );
        let s = sessions();
        assert_eq!(
            tool(&e, &s, None, "iii_list_agents", json!({}))
                .await
                .1
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            tool(
                &e,
                &s,
                None,
                "iii_list_agents",
                json!({ "include_hidden": true })
            )
            .await
            .1
            .as_array()
            .unwrap()
            .len(),
            2
        );
        assert_eq!(
            tool(&e, &s, None, "iii_get_agent", json!({ "id": "writer" }))
                .await
                .1["system_prompt"],
            "Be."
        );
        assert_eq!(
            tool(&e, &s, None, "iii_list_skills", json!({ "search": "kan" }))
                .await
                .1[0]["id"],
            "kanban"
        );
        assert_eq!(
            tool(&e, &s, None, "iii_get_skill", json!({ "id": "kanban" }))
                .await
                .1,
            json!("# kanban")
        );
        let (err, v) = tool(&e, &s, None, "iii_get_agent", json!({})).await;
        assert!(err && v.as_str().unwrap().contains("id"));
    }

    #[tokio::test]
    async fn iii_status_says_whether_rbac_is_on() {
        let e = Engine::new(FakeBus::new());
        let s = sessions();
        let (_, v) = tool(&e, &s, Some("sess-1"), "iii_status", json!({})).await;
        assert_eq!(v["rbac"], "rbac-proxy");
        assert_eq!(v["session_id"], "sess-1");
        assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
        assert!(v.get("warning").is_none());
        let ctx = Ctx {
            bus: &e,
            sessions: &s,
            session_id: None,
            mode: &RbacMode::Dangerous,
            sink: "mcp::observed",
        };
        let r = call_tool(&ctx, "iii_status", &json!({})).await.unwrap();
        let v: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(v["rbac"], "disabled");
        assert_eq!(v["warning"], DANGEROUS_NOTICE);
    }
}
