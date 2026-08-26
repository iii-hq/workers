//! Agent-proposed working-directory changes for Console Harness sessions.
//!
//! The model may discover and call `console::working-directory::propose`
//! after it creates or clones a project elsewhere. The function validates the
//! directory through Shell and returns a session-stamped proposal. It never
//! mutates session metadata itself: the injected Console renderer asks the
//! operator to accept the proposal, and the browser applies that choice.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const PROPOSE_ID: &str = "console::working-directory::propose";
pub const STAMP_SESSION_ID: &str = "console::working-directory::stamp-session";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProposeWorkingDirectoryRequest {
    /// Existing directory the user asked to continue working in.
    pub path: String,
    /// Short explanation shown with the confirmation.
    #[serde(default)]
    pub reason: Option<String>,
    /// Authoritative Harness context. In-turn calls are stamped before dispatch.
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ProposeWorkingDirectoryResponse {
    pub session_id: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub requires_confirmation: bool,
}

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

#[derive(Debug, Serialize, JsonSchema, PartialEq)]
pub struct HookMutations {
    pub arguments: Value,
}

#[derive(Debug, Serialize, JsonSchema, PartialEq)]
pub struct StampSessionResponse {
    pub decision: String,
    pub mutations: HookMutations,
}

#[derive(Debug, Deserialize)]
struct WorkspaceValidation {
    path: String,
}

fn handler_error(message: impl Into<String>) -> Error {
    Error::Handler(message.into())
}

fn proposal(
    request: ProposeWorkingDirectoryRequest,
    canonical_path: String,
) -> Result<ProposeWorkingDirectoryResponse, Error> {
    let session_id = request
        .session_id
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            handler_error("working-directory proposal is missing Harness session context")
        })?;
    let reason = request
        .reason
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    Ok(ProposeWorkingDirectoryResponse {
        session_id,
        path: canonical_path,
        reason,
        requires_confirmation: true,
    })
}

async fn propose(
    iii: &IIIClient,
    request: ProposeWorkingDirectoryRequest,
) -> Result<ProposeWorkingDirectoryResponse, Error> {
    let requested_path = request.path.trim();
    if requested_path.is_empty() {
        return Err(handler_error("working-directory path must not be empty"));
    }
    let validated = iii
        .trigger(TriggerRequest {
            function_id: "shell::workspace::validate".into(),
            payload: json!({ "path": requested_path }),
            action: None,
            timeout_ms: Some(10_000),
        })
        .await
        .map_err(|error| {
            handler_error(format!(
                "could not validate proposed working directory '{requested_path}': {error}"
            ))
        })?;
    let validated: WorkspaceValidation = serde_json::from_value(validated).map_err(|error| {
        handler_error(format!(
            "shell returned an invalid working-directory validation response: {error}"
        ))
    })?;
    proposal(request, validated.path)
}

fn stamp_session(event: StampSessionEvent) -> Option<StampSessionResponse> {
    let session_id = event.session_id.filter(|value| !value.is_empty())?;
    let mut arguments = event.call.map(|call| call.arguments).unwrap_or(Value::Null);
    let object = arguments.as_object_mut()?;
    object.insert("session_id".into(), json!(session_id));
    Some(StampSessionResponse {
        decision: "continue".into(),
        mutations: HookMutations { arguments },
    })
}

pub fn register(iii: &Arc<IIIClient>) {
    let client = iii.clone();
    iii.register_function(
        PROPOSE_ID,
        RegisterFunction::new_async(move |request: ProposeWorkingDirectoryRequest| {
            let client = client.clone();
            async move { propose(&client, request).await }
        })
        .description(
            "Propose changing the current Harness session's working directory after creating or \
             cloning a project elsewhere. The path is validated, then Console asks the user to \
             confirm before the chat and paired Shell switch together. Call this when the user \
             explicitly asked to continue in the new directory; never use it merely to inspect a file.",
        ),
    );

    iii.register_function(
        STAMP_SESSION_ID,
        RegisterFunction::new_async(|event: StampSessionEvent| async move {
            Ok::<Option<StampSessionResponse>, Error>(stamp_session(event))
        })
        .description(
            "Internal: stamp the authoritative Harness session onto Console working-directory proposals.",
        )
        .metadata(json!({ "internal": true })),
    );
}

pub fn bind(iii: &IIIClient) -> Result<(), Error> {
    iii.register_trigger(RegisterTriggerInput::new(
        "harness::hook::pre-trigger",
        STAMP_SESSION_ID,
        json!({
            "functions": [PROPOSE_ID],
            "timeout_ms": 5_000,
            "on_error": "fail_closed"
        }),
    ))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamped_session_overwrites_spoofed_context() {
        let event: StampSessionEvent = serde_json::from_value(json!({
            "session_id": "real-session",
            "call": {
                "arguments": {
                    "path": "/tmp/project",
                    "session_id": "other-session"
                }
            }
        }))
        .unwrap();

        let stamped = stamp_session(event).unwrap();
        assert_eq!(stamped.mutations.arguments["session_id"], "real-session");
        assert_eq!(stamped.mutations.arguments["path"], "/tmp/project");
    }

    #[test]
    fn proposal_keeps_canonical_path_and_normalizes_reason() {
        let response = proposal(
            ProposeWorkingDirectoryRequest {
                path: "/tmp/link".into(),
                reason: Some("  cloned the requested repository  ".into()),
                session_id: Some("session-1".into()),
            },
            "/private/tmp/project".into(),
        )
        .unwrap();

        assert_eq!(
            response,
            ProposeWorkingDirectoryResponse {
                session_id: "session-1".into(),
                path: "/private/tmp/project".into(),
                reason: Some("cloned the requested repository".into()),
                requires_confirmation: true,
            }
        );
    }

    #[test]
    fn proposal_requires_stamped_session_context() {
        let error = proposal(
            ProposeWorkingDirectoryRequest {
                path: "/tmp/project".into(),
                reason: None,
                session_id: None,
            },
            "/private/tmp/project".into(),
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("missing Harness session context"));
    }
}
