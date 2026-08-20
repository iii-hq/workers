//! Harness pre-trigger hook that stamps authoritative session context.

use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const STAMP_SESSION_ID: &str = "a2ui::stamp-session";

/// Trusted pre-trigger envelope supplied by the Harness hook runner.
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

pub fn bind(iii: &IIIClient) -> Result<(), iii_sdk::errors::Error> {
    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "harness::hook::pre-trigger".into(),
        function_id: STAMP_SESSION_ID.into(),
        config: json!({
            "functions": [
                "a2ui::generate",
                "a2ui::surface::apply",
                "a2ui::surface::get",
                "a2ui::surface::list",
                "a2ui::surface::delete",
                "a2ui::surface::patch",
                "a2ui::surface::export"
                ,"a2ui::surface::history"
                ,"a2ui::surface::undo"
                ,"a2ui::surface::duplicate"
                ,"a2ui::surface::pin"
                ,"a2ui::surface::import"
                ,"a2ui::surface::export-code"
                ,"a2ui::binding::set"
                ,"a2ui::binding::delete"
                ,"a2ui::binding::apply"
                ,"a2ui::template::save"
                ,"a2ui::template::list"
                ,"a2ui::template::get"
                ,"a2ui::template::apply"
                ,"a2ui::template::delete"
            ],
            "timeout_ms": 5_000,
            "on_error": "fail_closed"
        }),
        metadata: None,
    })?;
    Ok(())
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
}
