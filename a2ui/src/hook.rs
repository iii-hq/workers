//! Harness pre-call hook that stamps authoritative session context: the
//! worker observes `harness::hook::on-pre-call` with `a2ui::stamp-session`,
//! and the harness calls it before each A2UI function runs.

use iii_sdk::{IIIClient, ObserverHandle, RegisterFunction, Source};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const STAMP_SESSION_ID: &str = "a2ui::stamp-session";
/// The harness hook point run before a called function.
pub const PRE_CALL_HOOK_FN: &str = "harness::hook::on-pre-call";

/// The functions whose calls the hook stamps.
pub const STAMPED_FUNCTIONS: &[&str] = &[
    "a2ui::generate",
    "a2ui::surface::apply",
    "a2ui::surface::get",
    "a2ui::surface::list",
    "a2ui::surface::delete",
    "a2ui::surface::patch",
    "a2ui::surface::export",
    "a2ui::surface::history",
    "a2ui::surface::undo",
    "a2ui::surface::duplicate",
    "a2ui::surface::pin",
    "a2ui::surface::import",
    "a2ui::surface::export-code",
    "a2ui::binding::set",
    "a2ui::binding::delete",
    "a2ui::binding::apply",
    "a2ui::binding::refresh",
    "a2ui::template::save",
    "a2ui::template::list",
    "a2ui::template::get",
    "a2ui::template::apply",
    "a2ui::template::delete",
];

/// Trusted pre-call hook input supplied by the Harness hook runner.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct StampSessionEvent {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub call: Option<StampCall>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StampCall {
    #[serde(default)]
    pub arguments: Value,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct HookMutations {
    pub arguments: Value,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StampSessionResponse {
    pub decision: String,
    pub mutations: HookMutations,
}

pub fn decide(event: StampSessionEvent) -> Option<StampSessionResponse> {
    let session_id = event.session_id.filter(|value| !value.is_empty())?;
    let mut arguments = event.call.map(|call| call.arguments).unwrap_or(Value::Null);
    let object = arguments.as_object_mut()?;
    object.insert("session_id".into(), json!(session_id));
    Some(StampSessionResponse {
        decision: "continue".into(),
        mutations: HookMutations { arguments },
    })
}

pub fn register(iii: &IIIClient) {
    iii.register_function(
        STAMP_SESSION_ID,
        RegisterFunction::new_async(|event: StampSessionEvent| async move {
            Ok::<Option<StampSessionResponse>, iii_sdk::errors::Error>(decide(event))
        })
        .description(
            "Internal: stamp the authoritative Harness session onto A2UI calls before dispatch.",
        ),
    );
}

/// The args of the stamp observer: the A2UI functions it applies to, a 5 s
/// budget, and `fail_closed`, so an A2UI call never runs unstamped.
pub fn observer_args() -> Value {
    json!({
        "functions": STAMPED_FUNCTIONS,
        "timeout_ms": 5_000,
        "on_error": "fail_closed",
    })
}

/// Observe `harness::hook::on-pre-call` with `a2ui::stamp-session`. The
/// observer stays `pending` while the harness is not running and lives as
/// long as the worker's connection.
pub fn observe(iii: &IIIClient) -> ObserverHandle {
    iii.observe(
        Source::function(PRE_CALL_HOOK_FN).args(observer_args()),
        STAMP_SESSION_ID,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overwrites_spoofed_context() {
        let event: StampSessionEvent = serde_json::from_value(json!({
            "session_id": "real",
            "call": {"arguments": {"session_id": "victim", "description": "x"}}
        }))
        .unwrap();
        let output = decide(event).unwrap();
        assert_eq!(output.mutations.arguments["session_id"], "real");
    }

    #[test]
    fn observer_args_cover_every_session_scoped_function_and_fail_closed() {
        let args = observer_args();
        assert_eq!(args["on_error"], "fail_closed");
        assert_eq!(args["timeout_ms"], 5_000);
        let functions: Vec<&str> = args["functions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap())
            .collect();
        assert_eq!(functions, STAMPED_FUNCTIONS);
        assert!(functions.contains(&"a2ui::binding::refresh"));
    }
}
