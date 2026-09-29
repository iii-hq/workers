//! Mirror a `devin::run` turn into `session-manager`, so a Devin run shows up
//! in the console's session list and transcript like a harness session.
//!
//! `agent::events` / `devin::events` are a live tape: a console window opened
//! after the run has nothing to replay from them, and without the stream
//! worker they carry nothing at all. The session manager is the durable side
//! the console reads (`session::list`, `session::messages`), so a turn writes
//! the same records a harness turn does:
//!
//! - `session::ensure` with a title and `{ agent: "devin" }` metadata. A run
//!   with a `parent_session_id` carries it in the metadata, which nests it
//!   under the session that asked for it (it only applies on creation, so it
//!   is sent before the first turn); a run without one is created with
//!   `kind: "automation"`, session-manager's kind for machine-driven runs;
//! - `session::set-status` `working`;
//! - `session::append` the user prompt, then an empty assistant entry under a
//!   deterministic id, streamed into with `session::update-message` as the CLI
//!   prints and written once more with the final text;
//! - on failure, a `custom` `error` entry (the harness's `finalize_failed`
//!   record, which the console renders as a durable notice);
//! - `session::set-status` `done` / `error`, with `stopped` for an aborted run
//!   (the harness projection).
//!
//! Everything here is best-effort. `session-manager` is a dependency of the
//! console's views, not of running Devin: when it is not installed the link
//! logs that once and the run continues unrecorded.

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

use crate::optional::OptionalDependency;
use crate::wire::{assistant_message, now_ms};

const TIMEOUT_MS: u64 = 15_000;
/// Minimum spacing between streamed `session::update-message` writes. The CLI
/// prints line by line; one write per line would be one bus call per line.
const STREAM_UPDATE_EVERY_MS: u64 = 500;
/// Cap for the status reason / error summary (a short cause, not the output).
const REASON_MAX_CHARS: usize = 200;

static SESSIONS: OptionalDependency = OptionalDependency::new("session-manager (session::*)");

/// One turn's handle into its session-manager session. `None` from [`open`]
/// means the run is not being recorded (session-manager absent or failing).
pub struct TurnLink {
    iii: IIIClient,
    session_id: String,
    turn_id: String,
    model: String,
    last_update_ms: u64,
    last_len: usize,
}

/// A fresh turn id (`t_<uuid>`), the harness's id shape.
fn new_turn_id() -> String {
    format!("t_{}", uuid::Uuid::new_v4().simple())
}

pub fn user_entry_id(turn_id: &str) -> String {
    format!("e_{turn_id}_user")
}

pub fn assistant_entry_id(turn_id: &str) -> String {
    format!("e_{turn_id}_assistant")
}

pub fn error_entry_id(turn_id: &str) -> String {
    format!("e_{turn_id}_error")
}

/// One line of the prompt, so a session list stays readable.
pub fn title(prompt: &str) -> String {
    let line = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return "devin run".to_string();
    }
    if line.chars().count() > 60 {
        let head: String = line.chars().take(59).collect();
        format!("{head}…")
    } else {
        line
    }
}

/// session-manager's `SessionKind` for a machine-driven run.
const AUTOMATION_KIND: &str = "automation";

/// The `session::ensure` request. Title, metadata and kind only apply when the
/// call creates the session; a resumed `session_id` keeps what it was created
/// with. A delegated run (with a parent) nests under its parent and keeps the
/// default kind, as a harness sub-agent does; a top-level run is an
/// `automation` session, not a human chat.
pub fn ensure_payload(session_id: &str, prompt: &str, parent_session_id: Option<&str>) -> Value {
    let mut payload = json!({
        "session_id": session_id,
        "title": title(prompt),
        "metadata": { "agent": "devin" },
    });
    match parent_session_id.filter(|p| !p.is_empty()) {
        Some(parent) => payload["metadata"]["parent_session_id"] = json!(parent),
        None => payload["kind"] = json!(AUTOMATION_KIND),
    }
    payload
}

fn text_content(text: &str) -> Value {
    if text.is_empty() {
        json!([])
    } else {
        json!([{ "type": "text", "text": text }])
    }
}

/// The user entry: what the caller asked (without the iii runtime context the
/// worker prepends for the CLI).
pub fn user_message(prompt: &str) -> Value {
    json!({
        "role": "user",
        "content": text_content(prompt),
        "timestamp": now_ms(),
    })
}

/// A short cause for a failed run: the last non-empty output line (where the
/// CLI and the worker put the failure), capped.
pub fn short_reason(text: &str) -> String {
    let line = text
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("devin CLI run failed");
    line.chars().take(REASON_MAX_CHARS).collect()
}

/// The terminal session status for a finished run — the harness projection:
/// completed -> `done`, stopped -> `done` + `stopped`, failed -> `error` + cause.
pub fn terminal_status(
    stop_reason: &str,
    is_error: bool,
    text: &str,
) -> (&'static str, Option<String>) {
    if is_error {
        ("error", Some(short_reason(text)))
    } else if stop_reason == "aborted" {
        ("done", Some("stopped".to_string()))
    } else {
        ("done", None)
    }
}

/// The `custom` `error` record the console renders as a failure notice.
pub fn error_record(reason: &str, model: &str) -> Value {
    json!({
        "status": "error",
        "summary": reason,
        "reason": reason,
        "message": format!("devin run failed — {reason}"),
        "provider": "devin",
        "model": model,
        "timestamp": now_ms(),
    })
}

async fn call(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
) -> Result<Value, iii_sdk::errors::Error> {
    iii.trigger(TriggerRequest {
        function_id: function_id.to_string(),
        payload,
        action: None,
        timeout_ms: Some(TIMEOUT_MS),
    })
    .await
}

/// Best-effort call: logs a failure (once, when the function is missing) and
/// reports whether it succeeded.
async fn try_call(iii: &IIIClient, session_id: &str, function_id: &str, payload: Value) -> bool {
    let res = call(iii, function_id, payload).await;
    let error = res.err();
    if SESSIONS.observe(error.as_ref(), now_ms()) {
        return false;
    }
    match error {
        Some(e) => {
            tracing::warn!(session_id, function_id, error = %e, "session-manager write failed");
            false
        }
        None => true,
    }
}

/// Correlation echoed on every session-manager event this turn causes.
fn origin(turn_id: &str) -> Value {
    json!({ "turn_id": turn_id, "agent": "devin" })
}

/// `session::set-status` request.
pub fn status_request(session_id: &str, status: &str, reason: Option<&str>) -> Value {
    let mut payload = json!({ "session_id": session_id, "status": status });
    if let Some(r) = reason {
        payload["reason"] = json!(r);
    }
    payload
}

/// `session::append` of the user prompt.
pub fn user_append_request(session_id: &str, turn_id: &str, prompt: &str) -> Value {
    json!({
        "session_id": session_id,
        "entry_id": user_entry_id(turn_id),
        "message": user_message(prompt),
        "origin": origin(turn_id),
    })
}

/// `session::append` of the empty assistant entry the output streams into —
/// the harness's generate step: append empty under a deterministic id, then
/// update it.
pub fn assistant_append_request(session_id: &str, turn_id: &str, model: &str) -> Value {
    json!({
        "session_id": session_id,
        "entry_id": assistant_entry_id(turn_id),
        "message": assistant_message(vec![], model, "end"),
        "origin": origin(turn_id),
    })
}

/// `session::update-message` replacing the assistant entry's content.
pub fn update_request(session_id: &str, turn_id: &str, text: &str) -> Value {
    json!({
        "session_id": session_id,
        "entry_id": assistant_entry_id(turn_id),
        "content": text_content(text),
        "origin": origin(turn_id),
    })
}

/// `session::append` of the failure record. It rides the dedicated `custom`
/// field: only that creates a `kind: "custom"` entry the console renders.
pub fn error_append_request(session_id: &str, turn_id: &str, reason: &str, model: &str) -> Value {
    json!({
        "session_id": session_id,
        "entry_id": error_entry_id(turn_id),
        "custom": { "custom_type": "error", "data": error_record(reason, model) },
        "origin": origin(turn_id),
    })
}

/// Create or adopt the run's session and write the opening records. Returns
/// `None` when the run cannot be recorded; the run proceeds either way.
pub async fn open(
    iii: &IIIClient,
    session_id: &str,
    parent_session_id: Option<&str>,
    prompt: &str,
    model: &str,
) -> Option<TurnLink> {
    if !SESSIONS.should_try(now_ms()) {
        return None;
    }
    let ensure = ensure_payload(session_id, prompt, parent_session_id);
    if !try_call(iii, session_id, "session::ensure", ensure).await {
        return None;
    }
    let turn_id = new_turn_id();
    let status = status_request(session_id, "working", None);
    try_call(iii, session_id, "session::set-status", status).await;
    let user = user_append_request(session_id, &turn_id, prompt);
    if !try_call(iii, session_id, "session::append", user).await {
        return None;
    }
    let assistant = assistant_append_request(session_id, &turn_id, model);
    if !try_call(iii, session_id, "session::append", assistant).await {
        return None;
    }
    Some(TurnLink {
        iii: iii.clone(),
        session_id: session_id.to_string(),
        turn_id,
        model: model.to_string(),
        last_update_ms: 0,
        last_len: 0,
    })
}

/// Best-effort `error` status for a run whose task died before it could
/// finish its own link (a panic in `devin::start`).
pub async fn mark_error(iii: &IIIClient, session_id: &str, reason: &str) {
    if SESSIONS.is_missing() {
        return;
    }
    let payload = status_request(session_id, "error", Some(reason));
    if let Err(e) = call(iii, "session::set-status", payload).await {
        tracing::debug!(session_id, error = %e, "session::set-status error failed");
    }
}

impl TurnLink {
    async fn write_text(&self, text: &str) {
        let payload = update_request(&self.session_id, &self.turn_id, text);
        try_call(
            &self.iii,
            &self.session_id,
            "session::update-message",
            payload,
        )
        .await;
    }

    /// Stream the output so far into the assistant entry, at most once per
    /// [`STREAM_UPDATE_EVERY_MS`].
    pub async fn stream(&mut self, text: &str) {
        let now = now_ms();
        if text.len() == self.last_len
            || now.saturating_sub(self.last_update_ms) < STREAM_UPDATE_EVERY_MS
        {
            return;
        }
        self.last_update_ms = now;
        self.last_len = text.len();
        self.write_text(text).await;
    }

    /// Write the final text, the failure record when the run failed, and the
    /// terminal status.
    pub async fn finish(self, text: &str, stop_reason: &str, is_error: bool) {
        self.write_text(text).await;
        let (status, reason) = terminal_status(stop_reason, is_error, text);
        if is_error {
            let cause = reason.as_deref().unwrap_or_default();
            let record = error_append_request(&self.session_id, &self.turn_id, cause, &self.model);
            try_call(&self.iii, &self.session_id, "session::append", record).await;
        }
        let payload = status_request(&self.session_id, status, reason.as_deref());
        try_call(&self.iii, &self.session_id, "session::set-status", payload).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_is_one_readable_line() {
        assert_eq!(title("  fix\nthe   tests "), "fix the tests");
        assert_eq!(title(""), "devin run");
        let long = "x".repeat(80);
        let t = title(&long);
        assert_eq!(t.chars().count(), 60);
        assert!(t.ends_with('…'));
    }

    // Prevents: a delegated run losing its nesting — `parent_session_id` only
    // applies when `session::ensure` creates the session.
    #[test]
    fn delegated_run_nests_under_its_parent_with_the_default_kind() {
        let with_parent = ensure_payload("s1", "do X", Some("s_parent"));
        assert_eq!(with_parent["session_id"], "s1");
        assert_eq!(with_parent["title"], "do X");
        assert_eq!(with_parent["metadata"]["agent"], "devin");
        assert_eq!(with_parent["metadata"]["parent_session_id"], "s_parent");
        assert!(with_parent.get("kind").is_none());
    }

    // Prevents: a top-level machine-driven run being filed as a human chat.
    #[test]
    fn top_level_run_is_an_automation_session() {
        for parent in [None, Some("")] {
            let bare = ensure_payload("s1", "do X", parent);
            assert_eq!(bare["kind"], "automation");
            assert_eq!(bare["metadata"]["agent"], "devin");
            assert!(bare["metadata"].get("parent_session_id").is_none());
        }
    }

    // The shapes session-manager's `AgentMessage` accepts (role-tagged,
    // content blocks, ms timestamp; assistant with stop_reason/model/provider).
    #[test]
    fn transcript_messages_match_the_session_manager_contract() {
        let user = user_message("hello");
        assert_eq!(user["role"], "user");
        assert_eq!(
            user["content"],
            json!([{ "type": "text", "text": "hello" }])
        );
        assert!(user["timestamp"].as_u64().unwrap() > 0);

        let assistant = assistant_message(vec![], "", "end");
        assert_eq!(assistant["role"], "assistant");
        assert_eq!(assistant["content"], json!([]));
        assert_eq!(assistant["stop_reason"], "end");
        assert_eq!(assistant["agent"], "devin");
        assert_eq!(assistant["provider"], "devin");
        assert_eq!(assistant["model"], "");

        assert_eq!(text_content(""), json!([]));
    }

    // Prevents: a failure record stored as a plain message (invisible to the
    // console's custom-entry renderer), or streamed updates missing the entry.
    #[test]
    fn requests_target_the_turn_entries() {
        let user = user_append_request("s1", "t_1", "do X");
        assert_eq!(user["entry_id"], "e_t_1_user");
        assert_eq!(user["message"]["role"], "user");
        assert_eq!(
            user["origin"],
            json!({ "turn_id": "t_1", "agent": "devin" })
        );

        let assistant = assistant_append_request("s1", "t_1", "");
        assert_eq!(assistant["entry_id"], "e_t_1_assistant");
        assert_eq!(assistant["message"]["role"], "assistant");

        let update = update_request("s1", "t_1", "partial");
        assert_eq!(update["entry_id"], "e_t_1_assistant");
        assert_eq!(
            update["content"],
            json!([{ "type": "text", "text": "partial" }])
        );

        let error = error_append_request("s1", "t_1", "boom", "");
        assert_eq!(error["entry_id"], "e_t_1_error");
        assert_eq!(error["custom"]["custom_type"], "error");
        assert!(error.get("message").is_none());

        assert_eq!(
            status_request("s1", "done", Some("stopped")),
            json!({ "session_id": "s1", "status": "done", "reason": "stopped" })
        );
        assert!(status_request("s1", "working", None)
            .get("reason")
            .is_none());
    }

    #[test]
    fn entry_ids_are_deterministic_per_turn() {
        assert_eq!(user_entry_id("t_1"), "e_t_1_user");
        assert_eq!(assistant_entry_id("t_1"), "e_t_1_assistant");
        assert_eq!(error_entry_id("t_1"), "e_t_1_error");
        assert!(new_turn_id().starts_with("t_"));
        assert_ne!(new_turn_id(), new_turn_id());
    }

    #[test]
    fn terminal_status_follows_the_harness_projection() {
        assert_eq!(terminal_status("end", false, "ok"), ("done", None));
        assert_eq!(
            terminal_status("aborted", false, ""),
            ("done", Some("stopped".to_string()))
        );
        assert_eq!(
            terminal_status("error", true, "working...\nUser rejected tool permission\n"),
            ("error", Some("User rejected tool permission".to_string()))
        );
        assert_eq!(
            terminal_status("error", true, ""),
            ("error", Some("devin CLI run failed".to_string()))
        );
    }

    #[test]
    fn short_reason_is_capped() {
        let long = "e".repeat(500);
        assert_eq!(short_reason(&long).chars().count(), REASON_MAX_CHARS);
    }

    #[test]
    fn error_record_renders_as_a_console_notice() {
        let record = error_record("boom", "swe-1.6");
        assert_eq!(record["status"], "error");
        assert_eq!(record["reason"], "boom");
        assert_eq!(record["message"], "devin run failed — boom");
        assert_eq!(record["provider"], "devin");
        assert_eq!(record["model"], "swe-1.6");
    }
}
