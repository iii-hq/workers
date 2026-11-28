//! `harness::set-description` (chat-projects D1, D13): the agent keeps its
//! conversation's one-line description, and names a new conversation once,
//! inside the turn it is already running.
//!
//! The model sees it as a tool in the cached tools block (in every exposure
//! mode); the turn loop answers the call with the turn's own session id, next
//! to `harness::ask`, so the model cannot write another session. The
//! registered handler only answers direct calls, always with an error. Every
//! write is one `session::patch-meta` (D9). This module holds the request and
//! response shapes and the pure decision; the turn loop does the I/O.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::call::ResultData;
use crate::clients::session::{MetaPatch, SessionSummary};
use crate::error::HarnessError;
use crate::types::content::ContentBlock;
use crate::types::model::AgentFunction;

pub const SET_DESCRIPTION_ID: &str = "harness::set-description";

/// The tool description, exactly as the spec words it (chat-projects §1.2.3).
pub const TOOL_DESC: &str = "Set this conversation's one-line description, shown under its name in every client. Call it when the goal or the state of the work changes in a way the user would want to see at a glance: once you understand the task, when you open a pull request, when you finish or get blocked. Under 100 characters, plain text, no trailing period. Call it together with your other tool calls or at the end of your reply; never as the only thing you do in a step, and never mention it. Pass `title` only for a new conversation, as a short name of 2 to 6 words.";

pub const MAX_DESCRIPTION_CHARS: usize = 140;
pub const MAX_TITLE_CHARS: usize = 60;
/// Descriptions kept in `description_history`, newest first.
pub const HISTORY_KEPT: usize = 5;
/// The refusal when the user wrote the description (`description_manual`).
pub const USER_WROTE_IT: &str = "The user wrote this description; leave it.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SetDescriptionRequest {
    /// 1-140 characters, one line.
    #[schemars(length(min = 1, max = 140))]
    pub description: String,
    /// 1-60 characters; honoured once, for a conversation nobody named yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 60))]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SetDescriptionResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Set when a `title` was passed and not written: the user named the
    /// conversation, or the agent already did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_ignored: Option<bool>,
}

/// Why a direct call fails: only the turn loop knows whose conversation it is.
pub const DIRECT_CALL_MESSAGE: &str = "harness::set-description only works inside an agent turn, for \
     the conversation the turn runs in: call it from a model turn, not directly";

/// The registered handler, reached only by a direct call.
pub async fn direct_handle(
    _req: SetDescriptionRequest,
) -> Result<SetDescriptionResponse, HarnessError> {
    Err(HarnessError::InvalidRequest(DIRECT_CALL_MESSAGE.into()))
}

/// The tool the model is offered, when the dispatch policy allows it and
/// `descriptions.agent_writes` is on.
pub fn tool() -> AgentFunction {
    AgentFunction {
        name: SET_DESCRIPTION_ID.to_string(),
        description: TOOL_DESC.to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_DESCRIPTION_CHARS,
                    "description": "The one-line description: under 100 characters, plain text, no trailing period."
                },
                "title": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_TITLE_CHARS,
                    "description": "Only for a new conversation: a short name of 2 to 6 words."
                }
            },
            "required": ["description"]
        }),
        label: None,
        execution_mode: Some("sequential".to_string()),
    }
}

/// Trim, collapse every run of whitespace (line breaks included) to one
/// space, and cut at `max` characters.
pub fn one_line(raw: &str, max: usize) -> String {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .chars()
        .take(max)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// What the turn loop does with one call.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Answer without writing.
    Answer(SetDescriptionResponse),
    /// Write `patch` with one `session::patch-meta`, then answer.
    Write {
        patch: MetaPatch,
        response: SetDescriptionResponse,
    },
}

fn refused(reason: &str) -> Decision {
    Decision::Answer(SetDescriptionResponse {
        ok: false,
        reason: Some(reason.to_string()),
        title_ignored: None,
    })
}

fn flag(metadata: &Map<String, Value>, key: &str) -> bool {
    metadata.get(key).and_then(Value::as_bool) == Some(true)
}

/// Decide one call against the conversation as stored. `now_ms` stamps
/// `description_at_ms`.
pub fn decide(args: &Value, current: &SessionSummary, now_ms: i64) -> Decision {
    let request: SetDescriptionRequest = match serde_json::from_value(args.clone()) {
        Ok(request) => request,
        Err(error) => return refused(&format!("invalid arguments: {error}")),
    };
    let description = one_line(&request.description, MAX_DESCRIPTION_CHARS);
    if description.is_empty() {
        return refused("empty");
    }
    let empty = Map::new();
    let metadata = current.metadata.as_ref().unwrap_or(&empty);
    if flag(metadata, "description_manual") {
        return refused(USER_WROTE_IT);
    }

    let title = request
        .title
        .as_deref()
        .map(|title| one_line(title, MAX_TITLE_CHARS))
        .filter(|title| !title.is_empty());
    let title_allowed = !flag(metadata, "title_manual") && !metadata.contains_key("title_by");
    let title_ignored = request.title.is_some() && (!title_allowed || title.is_none());
    let write_title = title.filter(|_| title_allowed);

    let response = SetDescriptionResponse {
        ok: true,
        reason: None,
        title_ignored: title_ignored.then_some(true),
    };
    let same_description = description == current.description;
    if same_description && write_title.is_none() {
        return Decision::Answer(response);
    }

    let mut patch = MetaPatch::default();
    if !same_description {
        let mut history: Vec<Value> = Vec::new();
        if !current.description.trim().is_empty() {
            history.push(json!({
                "text": current.description,
                "by": metadata
                    .get("description_by")
                    .and_then(Value::as_str)
                    .unwrap_or("user"),
                "at_ms": metadata
                    .get("description_at_ms")
                    .and_then(Value::as_i64)
                    .unwrap_or(current.updated_at),
            }));
        }
        if let Some(older) = metadata
            .get("description_history")
            .and_then(Value::as_array)
        {
            history.extend(older.iter().cloned());
        }
        history.truncate(HISTORY_KEPT);
        patch.description = Some(description);
        patch.set.insert("description_by".into(), json!("agent"));
        patch.set.insert("description_at_ms".into(), json!(now_ms));
        patch
            .set
            .insert("description_history".into(), Value::Array(history));
    }
    if let Some(title) = write_title {
        patch.title = Some(title);
        patch.set.insert("title_by".into(), json!("agent"));
    }
    Decision::Write { patch, response }
}

/// The function result the model reads.
pub fn result(response: &SetDescriptionResponse) -> ResultData {
    let details = serde_json::to_value(response).unwrap_or(Value::Null);
    ResultData {
        content: vec![ContentBlock::text(details.to_string())],
        is_error: false,
        details,
    }
}

/// The result when the session-manager write failed.
pub fn write_failed(error: &HarnessError) -> ResultData {
    ResultData {
        content: vec![ContentBlock::text(format!(
            "The description could not be saved: {error}"
        ))],
        is_error: true,
        details: json!({ "ok": false, "reason": "write_failed" }),
    }
}

/// Whether a step's only calls are `harness::set-description`, in an
/// assistant message that already has reply text: the turn loop then keeps
/// that text as the turn's answer and answers the description with
/// [`GO_ON_NOTE`]. `call_ids` are the step's dispatched calls' function ids.
pub fn describes_beside_reply<'a>(
    mut call_ids: impl Iterator<Item = &'a str>,
    has_reply_text: bool,
    has_submit: bool,
) -> bool {
    let mut any = false;
    let only_descriptions = call_ids.all(|id| {
        any = true;
        id == SET_DESCRIPTION_ID
    });
    any && only_descriptions && has_reply_text && !has_submit
}

/// What the description answers when its step has reply text beside it, at
/// any point of the turn. That text can be an announcement ("I'll check the
/// tests first"): a model that makes one call per step sends it with this
/// call alone, and ending the turn there stopped such turns before their
/// work, at the turn's start and after other work alike. So the turn goes
/// on, and the model ends it itself when the text was its answer.
pub const GO_ON_NOTE: &str = "If your reply above already answers the user, end your turn now and write nothing more; if work remains, make your next call.";

/// Whether any of these calls is not `harness::set-description`.
pub fn did_other_work<'a>(mut function_ids: impl Iterator<Item = &'a str>) -> bool {
    function_ids.any(|id| id != SET_DESCRIPTION_ID)
}

/// The description's result with [`GO_ON_NOTE`] as `next`, its content kept
/// (a failed write's error included) and the note added after it.
pub fn with_go_on(mut data: ResultData) -> ResultData {
    if let Value::Object(details) = &mut data.details {
        details.insert("next".into(), Value::String(GO_ON_NOTE.into()));
    }
    data.content.push(ContentBlock::text(GO_ON_NOTE));
    data
}

/// Whether an assistant message carries reply text (not only thinking or calls).
pub fn has_reply_text(content: &[ContentBlock]) -> bool {
    content.iter().any(|block| match block {
        ContentBlock::Text { text, .. } => !text.trim().is_empty(),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(description: &str, metadata: Value) -> SessionSummary {
        SessionSummary {
            session_id: "s_1".into(),
            title: "New conversation".into(),
            description: description.into(),
            metadata: metadata.as_object().cloned(),
            updated_at: 50,
            ..SessionSummary::default()
        }
    }

    fn write(decision: Decision) -> (MetaPatch, SetDescriptionResponse) {
        match decision {
            Decision::Write { patch, response } => (patch, response),
            other => panic!("expected a write, got {other:?}"),
        }
    }

    fn answer(decision: Decision) -> SetDescriptionResponse {
        match decision {
            Decision::Answer(response) => response,
            other => panic!("expected an answer, got {other:?}"),
        }
    }

    #[test]
    fn the_tool_carries_the_exact_description() {
        let tool = tool();
        assert_eq!(tool.name, "harness::set-description");
        assert!(
            tool.description
                .starts_with("Set this conversation's one-line description")
        );
        assert!(
            tool.description
                .ends_with("as a short name of 2 to 6 words.")
        );
        assert_eq!(tool.parameters["required"], json!(["description"]));
    }

    #[test]
    fn text_is_trimmed_collapsed_and_cut() {
        assert_eq!(
            one_line("  Fixing   the\nlogin\t form  ", 140),
            "Fixing the login form"
        );
        let long = "é".repeat(200);
        assert_eq!(one_line(&long, 140).chars().count(), 140);
        assert_eq!(
            answer(decide(
                &json!({ "description": " \n " }),
                &session("", json!({})),
                1
            ))
            .reason
            .as_deref(),
            Some("empty")
        );
    }

    /// The lock: a description the user wrote is never overwritten.
    #[test]
    fn a_user_description_is_locked() {
        let current = session(
            "Mine",
            json!({ "description_manual": true, "description_by": "user" }),
        );
        let response = answer(decide(&json!({ "description": "Theirs" }), &current, 1));
        assert!(!response.ok);
        assert_eq!(response.reason.as_deref(), Some(USER_WROTE_IT));
    }

    #[test]
    fn the_same_text_is_a_no_op() {
        let current = session(
            "Fixing the login form",
            json!({ "description_by": "agent" }),
        );
        let response = answer(decide(
            &json!({ "description": "  Fixing  the login form " }),
            &current,
            9,
        ));
        assert_eq!(
            response,
            SetDescriptionResponse {
                ok: true,
                reason: None,
                title_ignored: None
            }
        );
    }

    /// The old description is pushed into the history, newest first, five kept.
    #[test]
    fn the_history_keeps_the_last_five_newest_first() {
        let older: Vec<Value> = (0..5)
            .map(|i| json!({ "text": format!("d{i}"), "by": "agent", "at_ms": 10 - i }))
            .collect();
        let current = session(
            "Current",
            json!({ "description_by": "user", "description_at_ms": 40, "description_history": older }),
        );
        let (patch, response) = write(decide(&json!({ "description": "Next" }), &current, 99));
        assert!(response.ok);
        assert_eq!(patch.description.as_deref(), Some("Next"));
        assert_eq!(patch.set["description_by"], "agent");
        assert_eq!(patch.set["description_at_ms"], 99);
        let history = patch.set["description_history"].as_array().unwrap();
        assert_eq!(history.len(), 5);
        assert_eq!(
            history[0],
            json!({ "text": "Current", "by": "user", "at_ms": 40 })
        );
        assert_eq!(history[4]["text"], "d3");
        assert!(patch.title.is_none());
        assert!(patch.unset.is_empty());

        // A first description has an empty history.
        let (patch, _) = write(decide(
            &json!({ "description": "First" }),
            &session("", json!({})),
            1,
        ));
        assert_eq!(patch.set["description_history"], json!([]));
    }

    /// D13: the title is written once, only while nobody named the conversation.
    #[test]
    fn the_title_is_honoured_once() {
        let (patch, response) = write(decide(
            &json!({ "description": "Fixing login", "title": " Login   form fix " }),
            &session("", json!({})),
            1,
        ));
        assert_eq!(patch.title.as_deref(), Some("Login form fix"));
        assert_eq!(patch.set["title_by"], "agent");
        assert_eq!(response.title_ignored, None);

        // Named by the agent before.
        let (patch, response) = write(decide(
            &json!({ "description": "Next step", "title": "Other" }),
            &session("Fixing login", json!({ "title_by": "agent" })),
            2,
        ));
        assert!(patch.title.is_none());
        assert!(!patch.set.contains_key("title_by"));
        assert_eq!(response.title_ignored, Some(true));

        // Named by the user.
        let response = answer(decide(
            &json!({ "description": "Fixing login", "title": "Other" }),
            &session("Fixing login", json!({ "title_manual": true })),
            3,
        ));
        assert_eq!(response.title_ignored, Some(true));

        // The same description with a title still allowed writes the title only.
        let (patch, _) = write(decide(
            &json!({ "description": "Fixing login", "title": "Login fix" }),
            &session("Fixing login", json!({})),
            4,
        ));
        assert!(patch.description.is_none());
        assert_eq!(patch.title.as_deref(), Some("Login fix"));
    }

    #[test]
    fn only_descriptions_with_reply_text_keep_the_reply() {
        let only = [SET_DESCRIPTION_ID];
        let mixed = [SET_DESCRIPTION_ID, "shell::exec"];
        assert!(describes_beside_reply(only.iter().copied(), true, false));
        assert!(describes_beside_reply(
            [SET_DESCRIPTION_ID, SET_DESCRIPTION_ID].iter().copied(),
            true,
            false
        ));
        assert!(
            !describes_beside_reply(only.iter().copied(), false, false),
            "no reply text"
        );
        assert!(
            !describes_beside_reply(mixed.iter().copied(), true, false),
            "another call drops the kept reply"
        );
        assert!(
            !describes_beside_reply(only.iter().copied(), true, true),
            "submit_result closes the turn its way"
        );
        assert!(!describes_beside_reply(std::iter::empty(), true, false));
        assert!(has_reply_text(&[ContentBlock::text("Done.")]));
        assert!(!has_reply_text(&[ContentBlock::text("  ")]));
    }

    #[test]
    fn other_work_is_any_call_but_the_description() {
        assert!(!did_other_work(std::iter::empty()));
        assert!(!did_other_work([SET_DESCRIPTION_ID].into_iter()));
        assert!(did_other_work(
            [SET_DESCRIPTION_ID, "shell::exec"].into_iter()
        ));
    }

    #[test]
    fn go_on_note_rides_on_the_answer() {
        let data = with_go_on(result(&SetDescriptionResponse {
            ok: true,
            reason: None,
            title_ignored: None,
        }));
        assert_eq!(data.details["ok"], json!(true));
        assert_eq!(data.details["next"], json!(GO_ON_NOTE));
        let texts: Vec<&str> = data
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts.len(), 2, "{texts:?}");
        assert!(texts[0].contains("\"ok\":true"), "{texts:?}");
        assert_eq!(texts[1], GO_ON_NOTE);
    }

    #[test]
    fn go_on_note_keeps_a_failed_writes_error() {
        let failed = write_failed(&HarnessError::Dependency("session store is down".into()));
        let data = with_go_on(failed);
        let ContentBlock::Text { text, .. } = &data.content[0] else {
            panic!("text content")
        };
        assert!(text.contains("session store is down"), "{text}");
        assert_eq!(data.details["next"], json!(GO_ON_NOTE));
    }
}
