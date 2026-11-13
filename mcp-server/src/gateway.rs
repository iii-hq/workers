//! Gateway mode: a fixed set of tools that reach the whole engine, instead
//! of one MCP tool per function (hundreds on a real engine).
//!
//!   iii_call               call any function by id
//!   iii_secret_store       store a credential, get a secret:// reference
//!   iii_observe            observe a function: buffer its observations in
//!                          the session, or run a target function with each
//!   iii_observations_sse   observe a function, streaming observations on GET
//!   iii_observations_poll  read (and remove) buffered observations
//!   iii_observers          this session's observers
//!   iii_unobserve          remove one
//!   iii_status             this server, and the access control in force
//!   iii_list_agents, iii_get_agent, iii_list_skills, iii_get_skill
//!
//! What a call may reach is decided by RBAC (rbac-proxy) on the connection,
//! not here; the only local rule is that `mcp-server::` internals are refused.

use serde_json::{json, Value};

use crate::bus::Bus;
use crate::directory_bridge;
use crate::rbac_guard::RbacMode;
use crate::sessions::{Delivery, Observe, ObserveRequest, Sessions};

pub const TOOL_NAMES: [&str; 12] = [
    "iii_call",
    "iii_secret_store",
    "iii_observe",
    "iii_observations_sse",
    "iii_observations_poll",
    "iii_observers",
    "iii_unobserve",
    "iii_status",
    "iii_list_agents",
    "iii_get_agent",
    "iii_list_skills",
    "iii_get_skill",
];

const DEFAULT_MAX_CHARS: usize = 100_000;
const INTERNAL_PREFIX: &str = "mcp-server::";
/// What [`iii_secret_store`] calls: `secrets::set` writes the vault.
const SECRET_SET_FN: &str = "secrets::set";

/// What one gateway tool call runs against.
pub struct Ctx<'a, B> {
    pub bus: &'a B,
    pub sessions: &'a Sessions,
    /// The request's `Mcp-Session-Id`, when it carries a live one.
    pub session_id: Option<&'a str>,
    pub mode: &'a RbacMode,
    /// The function every observer targets (`mcp-server::observed`).
    pub sink: &'a str,
}

/// The profile every session loads first: iii-directory's built-in
/// `default` identity, written for MCP clients.
pub const DEFAULT_PROFILE: &str = "default-mcp";

const INSTRUCTIONS: &str = "Start by loading the default agent profile with iii_get_agent (no arguments, or id \"default-mcp\"; it reads directory::agents::get from iii-directory) and follow it. Everything on the running iii engine is a function, called by id with iii_call (a function id and a JSON payload) instead of the `iii call` CLI or curl; never use a function id from memory. Discover what this system can do with iii_call: directory::search_functions { capabilities: [...] } returns candidate function ids for what you need, engine::functions::list ({ prefix } or { search }) the functions registered now, engine::workers::list the workers connected now, directory::skills::index the skills relevant to this system (guidance documents, not one per worker; use it sparingly), and engine::functions::info { function_id } a function's contract, which you read before its first call. Some functions are higher-order: they call other functions for you, so their payload names function ids and payloads. fp::pipe runs a chain of calls worker-side, each result passed to the next, so a large value moves between other functions without passing through you; fp::batch runs independent calls at once. Call them with iii_call like any other function. React to the engine with iii_observe instead of polling functions: each matching invocation becomes an observation, buffered for this session and read with iii_observations_poll, or handed to a target_function_id the engine runs; iii_observations_sse streams observations on this session's GET stream instead. List observers with iii_observers and remove each one with iii_unobserve when done. This server cannot register functions: to add one, create or change a worker you control and register it with the iii-sdk. Agent profiles: iii_list_agents / iii_get_agent. Skills: iii_list_skills / iii_get_skill (also resources at iii://<id>). iii_status reports the access control in force. Never put a credential in an iii_call payload: store it with iii_secret_store and pass the secret://NAME reference it returns.";

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
        tool("iii_call", "Call any function on the iii engine by id and return its result: the way to act on the engine, instead of the `iii call` CLI. Find ids with directory::search_functions (by capability) or engine::functions::list, and read a contract with engine::functions::info before the first call. Some functions are higher-order and call other functions for you: fp::pipe chains calls worker-side, each result passed to the next; fp::batch runs independent calls at once.", json!({
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
        tool("iii_observe", "Observe a function on the iii engine: each matching invocation of function_id becomes an observation. Without target_function_id, observations are buffered in this MCP session; read and remove them with iii_observations_poll. With target_function_id, the engine runs that function with each observation instead, and nothing comes back to this session: use it to wire one function to another. Either way the observer belongs to this session: list it with iii_observers, remove it with iii_unobserve, and it is removed when the session ends. Use it instead of polling a function.", observe_schema(true)),
        tool("iii_observations_sse", "Observe a function and stream its observations: each one is sent as a notifications/iii/observation message on this session's GET stream (a GET on the MCP endpoint with your Mcp-Session-Id). Observations wait in the session while no stream is open. Same arguments as iii_observe, without target_function_id. The observer belongs to this session: remove it with iii_unobserve; it is removed when the session ends.", observe_schema(false)),
        tool("iii_observations_poll", "Read and remove the buffered observations of observers created with iii_observe (without a target), oldest first: one observer's, or all of them. Reports how many remain and how many were dropped because the session's buffer was full.", json!({
            "type": "object",
            "properties": {
                "observer_id": { "type": "string", "description": "Omit for every poll observer." },
                "max": { "type": "number", "description": "Default 50." }
            }
        })),
        tool("iii_observers", "This session's observers: function, filters, state, and buffered / received / dropped counts.", json!({ "type": "object", "properties": {} })),
        tool("iii_unobserve", "Remove one of this session's observers, discarding its unread observations.", json!({
            "type": "object", "properties": { "observer_id": { "type": "string" } }, "required": ["observer_id"]
        })),
        tool("iii_status", "This MCP server: version, session, and access_control: rbac-proxy (with the rules in force) or disabled.", json!({ "type": "object", "properties": {} })),
        tool("iii_list_agents", "List the agent profiles installed in the iii engine (id, name, description, model, counts).", json!({
            "type": "object", "properties": { "include_hidden": { "type": "boolean", "description": "Include hidden helper profiles." } }
        })),
        tool("iii_get_agent", "Fetch an agent profile: its full system prompt (resolved through extends), preloaded skills and functions. Without an id, the default profile, which every session loads first.", json!({
            "type": "object",
            "properties": { "id": { "type": "string", "description": "Profile id. Default: default." } }
        })),
        tool("iii_list_skills", "List skills installed in the iii engine. Filter with search (substring) or prefix (worker namespace).", json!({
            "type": "object", "properties": { "search": { "type": "string" }, "prefix": { "type": "string" } }
        })),
        tool("iii_get_skill", "Fetch one skill by id and return its markdown body.", id_only("Skill id, e.g. kanban")),
    ]})
}

/// The observe tools' arguments; `with_target` adds `target_function_id`.
fn observe_schema(with_target: bool) -> Value {
    let mut schema = json!({
        "type": "object",
        "properties": {
            "function_id": { "type": "string", "description": "The function to observe, e.g. `state::set`." },
            "filters": { "type": "object", "description": "Which calls match, e.g. { \"input.scope\": { \"$eq\": \"jobs\" } }. Default: every call." },
            "args": { "type": "object", "description": "The observed function's args_format (see engine::functions::info)." },
            "capture": { "type": "array", "items": { "type": "string", "enum": ["input", "output", "error", "caller", "metadata"] }, "description": "What each observation carries. Default input, output, caller (successes only); add error for failures." },
            "namespace": { "type": "string" },
            "max_observations": { "type": "number", "description": "Remove the observer after this many observations." },
            "expires_in_ms": { "type": "number", "description": "Remove the observer after this long." }
        },
        "required": ["function_id"]
    });
    if with_target {
        schema["properties"]["target_function_id"] = json!({
            "type": "string",
            "description": "A function the engine runs with each observation as its input, instead of buffering it here. Nothing comes back to this session."
        });
    }
    schema
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

async fn observe<B: Bus + Observe>(
    ctx: &Ctx<'_, B>,
    sid: &str,
    args: &Value,
    delivery: Delivery,
) -> Value {
    match serde_json::from_value::<ObserveRequest>(args.clone()) {
        Err(e) => error(format!("invalid observe arguments: {e}")),
        Ok(req) => match ctx
            .sessions
            .observe(ctx.bus, sid, req, delivery, ctx.sink)
            .await
        {
            Ok(v) => text(&v),
            Err(e) => error(e),
        },
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
            Ok(sid) => {
                let delivery = match args.get("target_function_id") {
                    None | Some(Value::Null) => Delivery::Poll,
                    Some(Value::String(t)) => Delivery::Target(t.clone()),
                    Some(_) => return Some(error("target_function_id must be a function id (a string)")),
                };
                observe(ctx, sid, args, delivery).await
            }
        },
        "iii_observations_sse" => match need_session(ctx) {
            Err(e) => e,
            Ok(_) if args.get("target_function_id").is_some_and(|t| !t.is_null()) => error(
                "iii_observations_sse streams observations to this session; to run a function with each one, use iii_observe with target_function_id",
            ),
            Ok(sid) => observe(ctx, sid, args, Delivery::Sse).await,
        },
        "iii_observations_poll" => match need_session(ctx) {
            Err(e) => e,
            Ok(sid) => {
                let max = args.get("max").and_then(Value::as_u64).unwrap_or(50) as usize;
                match ctx.sessions.poll(sid, str_arg(args, "observer_id"), max) {
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
                    v["access_control"] = json!("rbac-proxy");
                    v["policy"] = policy.clone();
                }
                RbacMode::Dangerous => v["access_control"] = json!("disabled"),
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
        "iii_get_agent" => match args.get("id") {
            Some(v) if !v.is_null() && !v.is_string() => error("id must be a profile id (a string)"),
            _ => match directory_bridge::get_agent(
                ctx.bus,
                str_arg(args, "id").unwrap_or(DEFAULT_PROFILE),
            )
            .await
            {
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
            sink: "mcp-server::observed",
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
        assert_eq!(
            TOOL_NAMES.to_vec(),
            vec![
                "iii_call",
                "iii_secret_store",
                "iii_observe",
                "iii_observations_sse",
                "iii_observations_poll",
                "iii_observers",
                "iii_unobserve",
                "iii_status",
                "iii_list_agents",
                "iii_get_agent",
                "iii_list_skills",
                "iii_get_skill",
            ]
        );
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
        assert_eq!(required("iii_observations_sse"), json!(["function_id"]));
        assert_eq!(required("iii_unobserve"), json!(["observer_id"]));
        // No id: the default profile.
        assert_eq!(required("iii_get_agent"), Value::Null);
    }

    fn description(name: &str) -> String {
        tools_list()["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap()["description"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn the_observe_description_says_where_observations_go() {
        let d = description("iii_observe");
        for needle in [
            "iii_observations_poll",
            "target_function_id",
            "nothing",
            "iii_unobserve",
            "session ends",
        ] {
            assert!(d.contains(needle), "{needle} missing from: {d}");
        }
        let props = &tools_list()["tools"][2]["inputSchema"]["properties"];
        assert!(props["target_function_id"]["description"]
            .as_str()
            .unwrap()
            .contains("runs"));
        assert!(props.get("max_observations").is_some());
        assert!(props.get("max_events").is_none());
        let sse = description("iii_observations_sse");
        assert!(sse.contains("GET") && sse.contains("stream"), "{sse}");
    }

    #[test]
    fn the_instructions_start_with_the_default_profile() {
        let i = instructions(&proxied());
        let first = i.split(". ").next().unwrap();
        assert!(
            first.contains("iii_get_agent") && first.contains("default"),
            "the first sentence: {first}"
        );
        assert!(i.contains("directory::agents::get"), "{i}");
        assert!(first.contains("default-mcp"), "{first}");
        // The profile is written for MCP: nothing to translate.
        assert!(!i.contains("agent_call"), "{i}");
        // Dangerous mode keeps the warning first, then the same opening.
        let d = instructions(&RbacMode::Dangerous);
        assert!(d.starts_with(DANGEROUS_NOTICE));
        assert!(d.contains("iii_get_agent"));
    }

    #[test]
    fn the_instructions_name_the_discovery_functions() {
        let i = instructions(&proxied());
        for f in [
            "directory::search_functions",
            "engine::functions::list",
            "engine::functions::info",
            "engine::workers::list",
            "directory::skills::index",
        ] {
            assert!(i.contains(f), "{f} missing from: {i}");
        }
        // Skills are arbitrary guidance documents, not one per worker.
        assert!(
            i.contains("sparingly") && !i.contains("per installed worker"),
            "{i}"
        );
    }

    #[test]
    fn the_instructions_explain_higher_order_functions() {
        let i = instructions(&proxied());
        for needle in ["higher-order", "fp::pipe", "fp::batch", "other functions"] {
            assert!(i.contains(needle), "{needle} missing from: {i}");
        }
        let d = description("iii_call");
        for needle in [
            "directory::search_functions",
            "engine::functions::info",
            "higher-order",
            "fp::pipe",
        ] {
            assert!(d.contains(needle), "{needle} missing from: {d}");
        }
    }

    #[test]
    fn the_get_agent_description_points_at_the_default_profile() {
        let d = description("iii_get_agent");
        assert!(d.contains("default"), "{d}");
    }

    #[tokio::test]
    async fn get_agent_without_an_id_reads_the_default_profile() {
        let e = Engine::new(FakeBus::new().on("directory::agents::get", |p| {
            Ok(json!({ "id": p["id"], "system_prompt": "You are an iii agent." }))
        }));
        let s = sessions();
        for args in [json!({}), json!({ "id": " " })] {
            let (err, v) = tool(&e, &s, None, "iii_get_agent", args).await;
            assert!(!err, "{v}");
            assert_eq!(v["id"], "default-mcp");
        }
        assert!(e
            .bus
            .calls()
            .iter()
            .all(|(f, p)| f == "directory::agents::get" && p["id"] == "default-mcp"));
    }

    #[test]
    fn no_tool_talks_about_events() {
        let text = tools_list().to_string() + &instructions(&proxied());
        assert!(!text.to_lowercase().contains("event"), "{text}");
    }

    #[test]
    fn instructions_point_at_call_and_observe_and_warn_without_rbac() {
        let i = instructions(&proxied());
        for t in [
            "iii_call",
            "iii_observe",
            "iii_observations_poll",
            "iii_observations_sse",
            "iii_unobserve",
        ] {
            assert!(i.contains(t), "{t}");
        }
        // It cannot register functions; a worker the agent controls can.
        assert!(i.contains("cannot register functions"), "{i}");
        assert!(i.contains("iii-sdk"), "{i}");
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
            sink: "mcp-server::observed",
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
    async fn secret_store_passes_the_consumers_as_given() {
        // mcp-server hands references on; it never resolves a value back to a client.
        // Who may resolve is the secrets worker's and RBAC's decision.
        let e = Engine::new(FakeBus::new().on("secrets::set", |_| {
            Ok(json!({ "ref": "secret://K", "consumers": ["billing", "mcp-server"] }))
        }));
        let (err, _) = tool(
            &e,
            &sessions(),
            None,
            "iii_secret_store",
            json!({ "name": "K", "value": SECRET, "consumers": ["billing", "mcp-server"] }),
        )
        .await;
        assert!(!err);
        assert_eq!(stored(&e)[0]["consumers"], json!(["billing", "mcp-server"]));
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
    async fn iii_call_leaves_secrets_resolve_to_rbac() {
        // rbac-proxy's allow list refuses it; mcp-server does not special-case it.
        let e = Engine::new(FakeBus::new().on("secrets::resolve", |_| {
            Err(crate::bus::BusError::new("FORBIDDEN", "not allowed"))
        }));
        let (err, v) = tool(
            &e,
            &sessions(),
            None,
            "iii_call",
            json!({ "function_id": "secrets::resolve", "payload": { "ref": "secret://K" } }),
        )
        .await;
        assert!(err && v.as_str().unwrap().contains("FORBIDDEN"), "{v}");
        assert_eq!(e.bus.calls()[0].0, "secrets::resolve");
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
        let e = Engine::new(FakeBus::new().on("mcp-server::observed", |_| Ok(json!({}))));
        let (err, v) = tool(
            &e,
            &sessions(),
            None,
            "iii_call",
            json!({ "function_id": "mcp-server::observed" }),
        )
        .await;
        assert!(err && v.as_str().unwrap().contains("mcp-server::"), "{v}");
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
            ("iii_observations_sse", json!({ "function_id": "a::b" })),
            ("iii_observations_poll", json!({})),
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
    async fn observe_poll_observers_unobserve_end_to_end() {
        let e = Engine::new(FakeBus::new());
        let s = sessions();
        let sid = s.create().unwrap();
        let (err, v) = tool(
            &e,
            &s,
            Some(&sid),
            "iii_observe",
            json!({ "function_id": "state::set", "max_observations": 2 }),
        )
        .await;
        assert!(!err, "{v}");
        assert_eq!(v["observer_id"], "obs-1");
        assert_eq!(v["delivery"], "poll");
        {
            let observed = e.observed.lock().unwrap();
            assert_eq!(observed[0].1, "mcp-server::observed");
            assert_eq!(observed[0].0.max_observations, Some(2));
        }
        s.deliver(json!({ "observer_id": "obs-1", "input": { "k": 1 } }));
        let (_, listed) = tool(&e, &s, Some(&sid), "iii_observers", json!({})).await;
        assert_eq!(listed[0]["buffered"], 1);
        let (_, got) = tool(
            &e,
            &s,
            Some(&sid),
            "iii_observations_poll",
            json!({ "observer_id": "obs-1", "max": 10 }),
        )
        .await;
        assert_eq!(got["observations"][0]["input"], json!({ "k": 1 }));
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
    async fn observe_with_a_target_runs_that_function() {
        let e = Engine::new(FakeBus::new());
        let s = sessions();
        let sid = s.create().unwrap();
        let (err, v) = tool(
            &e,
            &s,
            Some(&sid),
            "iii_observe",
            json!({ "function_id": "orders::create", "target_function_id": "mail::send" }),
        )
        .await;
        assert!(!err, "{v}");
        assert_eq!(v["delivery"], "target");
        assert_eq!(v["target_function_id"], "mail::send");
        assert_eq!(e.observed.lock().unwrap()[0].1, "mail::send");
        let (err, v) = tool(
            &e,
            &s,
            Some(&sid),
            "iii_observe",
            json!({ "function_id": "a::b", "target_function_id": 3 }),
        )
        .await;
        assert!(
            err && v.as_str().unwrap().contains("target_function_id"),
            "{v}"
        );
    }

    #[tokio::test]
    async fn observations_sse_creates_an_sse_observer() {
        let e = Engine::new(FakeBus::new());
        let s = sessions();
        let sid = s.create().unwrap();
        let (err, v) = tool(
            &e,
            &s,
            Some(&sid),
            "iii_observations_sse",
            json!({ "function_id": "state::set" }),
        )
        .await;
        assert!(!err, "{v}");
        assert_eq!(v["delivery"], "sse");
        assert_eq!(e.observed.lock().unwrap()[0].1, "mcp-server::observed");
        // A target makes no sense on a stream observer.
        let (err, v) = tool(
            &e,
            &s,
            Some(&sid),
            "iii_observations_sse",
            json!({ "function_id": "a::b", "target_function_id": "c::d" }),
        )
        .await;
        assert!(err && v.as_str().unwrap().contains("iii_observe"), "{v}");
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
        let (err, v) = tool(&e, &s, None, "iii_get_agent", json!({ "id": 3 })).await;
        assert!(err && v.as_str().unwrap().contains("id"), "{v}");
    }

    #[tokio::test]
    async fn iii_status_says_whether_rbac_is_on() {
        let e = Engine::new(FakeBus::new());
        let s = sessions();
        let (_, v) = tool(&e, &s, Some("sess-1"), "iii_status", json!({})).await;
        assert_eq!(v["access_control"], "rbac-proxy");
        assert!(v.get("rbac").is_none());
        assert_eq!(v["session_id"], "sess-1");
        assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
        assert!(v.get("warning").is_none());
        let ctx = Ctx {
            bus: &e,
            sessions: &s,
            session_id: None,
            mode: &RbacMode::Dangerous,
            sink: "mcp-server::observed",
        };
        let r = call_tool(&ctx, "iii_status", &json!({})).await.unwrap();
        let v: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(v["access_control"], "disabled");
        assert_eq!(v["warning"], DANGEROUS_NOTICE);
    }
}
