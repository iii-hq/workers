//! `shell::scm::*` — LLM-written git commit messages for the IDE's Commit
//! panel.
//!
//! The UI owns the git side: it runs `git diff`, trims it to a stat plus a
//! unified diff and passes the text in. This module owns the prompt and the
//! model call, so the model, reasoning effort and house rules come from the
//! worker's LIVE configuration (`commit_messages`) instead of being baked into
//! the page. The call goes worker-to-worker to `router::complete`, like
//! `document::ocr` does, so it needs no permission entry of its own.

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::{CommitMessagesConfig, CommitThinking};

pub const CONFIG_FN_ID: &str = "shell::scm::commit-message-config";
pub const COMMIT_MESSAGE_FN_ID: &str = "shell::scm::commit-message";

/// Longest `changes` text sent to the model, in characters.
pub const MAX_CHANGES_CHARS: usize = 60_000;
const TRUNCATED_MARKER: &str = "[diff truncated]";
const MAX_OUTPUT_TOKENS: u64 = 1024;
/// A reasoning model over a 60k-char diff can think for a while.
const COMPLETE_TIMEOUT_MS: u64 = 120_000;

/// No model configured and none passed by the caller.
pub const CODE_NO_MODEL: &str = "NO_MODEL";
/// `changes` was empty or whitespace.
pub const CODE_EMPTY_CHANGES: &str = "EMPTY_CHANGES";
/// `router::complete` failed, or the model's turn ended in an error.
pub const CODE_ROUTER_FAILED: &str = "ROUTER_FAILED";
/// The model answered with no usable text.
pub const CODE_EMPTY_MESSAGE: &str = "EMPTY_MESSAGE";

const SYSTEM_PROMPT: &str = "You write git commit messages for the change the user shows you.
Reply with the commit message only: no preamble, no quotes, no Markdown fences.
First line: an imperative summary of at most 72 characters.
If the change needs explaining, add a blank line and a short body wrapped at 72 characters that says what changed and why.
Match the style of the repository's recent subjects when they are given.";

// Like every request type here (see `ListRequest`), NOT `deny_unknown_fields`:
// the engine injects `_caller_worker_id` into every call's payload, so a
// strict struct would reject every call.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CommitMessageConfigRequest {}

#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct CommitMessageConfigResponse {
    /// Configured `commit_messages.model`, trimmed; null when unset.
    pub model: Option<String>,
    /// Configured reasoning effort: default|minimal|low|medium|high|xhigh.
    pub thinking: CommitThinking,
    /// Configured `commit_messages.instructions`.
    pub instructions: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CommitMessageRequest {
    /// Diff text prepared by the UI (stat + unified diff). Capped at 60000
    /// characters; the rest is cut and marked `[diff truncated]`.
    pub changes: String,
    /// Recent commit subjects of the repository, for style.
    #[serde(default)]
    pub recent_subjects: Option<Vec<String>>,
    /// `provider::model` (or bare model id) used when
    /// `commit_messages.model` is unset: the chat's default model.
    #[serde(default)]
    pub fallback_model: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct CommitMessageResponse {
    /// The commit message: summary line, and a body when the change needs one.
    pub message: String,
    /// The model id actually used, as configured or passed.
    pub model: String,
}

/// `router::complete`, behind a seam so tests run without an engine.
#[async_trait]
pub trait Router: Send + Sync {
    async fn complete(&self, payload: Value) -> Result<Value, Error>;
}

pub struct IiiRouter(pub IIIClient);

#[async_trait]
impl Router for IiiRouter {
    async fn complete(&self, payload: Value) -> Result<Value, Error> {
        self.0
            .trigger(TriggerRequest {
                function_id: "router::complete".to_string(),
                payload,
                action: None,
                timeout_ms: Some(COMPLETE_TIMEOUT_MS),
            })
            .await
    }
}

fn fail(code: &str, message: impl Into<String>) -> Error {
    Error::Remote {
        code: code.to_string(),
        message: message.into(),
        stacktrace: None,
    }
}

pub fn commit_message_config(cfg: &CommitMessagesConfig) -> CommitMessageConfigResponse {
    CommitMessageConfigResponse {
        model: cfg.model_id().map(str::to_string),
        thinking: cfg.thinking,
        instructions: cfg.instructions.clone(),
    }
}

/// The configured model, else the caller's fallback, else `NO_MODEL`.
pub fn resolve_model(cfg: &CommitMessagesConfig, fallback: Option<&str>) -> Result<String, Error> {
    cfg.model_id()
        .or_else(|| fallback.map(str::trim).filter(|m| !m.is_empty()))
        .map(str::to_string)
        .ok_or_else(|| {
            fail(
                CODE_NO_MODEL,
                "no model for commit messages: set commit_messages.model in the IDE settings or \
                 pick a chat model",
            )
        })
}

/// `provider::model` → `(Some(provider), model)`; a bare id → `(None, id)`.
/// Splits on the FIRST `::` (model ids may contain `:` or further `::`); a
/// half-empty pair is not a provider split, so the whole id stays the model.
pub fn split_model(id: &str) -> (Option<&str>, &str) {
    match id.split_once("::") {
        Some((provider, model)) if !provider.is_empty() && !model.is_empty() => {
            (Some(provider), model)
        }
        _ => (None, id),
    }
}

/// `changes` cut to `MAX_CHANGES_CHARS` characters (never mid-character) with a
/// `[diff truncated]` line appended when something was cut.
pub fn cap_changes(changes: &str) -> String {
    match changes.char_indices().nth(MAX_CHANGES_CHARS) {
        None => changes.to_string(),
        Some((cut, _)) => {
            let kept = &changes[..cut];
            let sep = if kept.ends_with('\n') { "" } else { "\n" };
            format!("{kept}{sep}{TRUNCATED_MARKER}")
        }
    }
}

pub fn system_prompt(instructions: &str) -> String {
    let instructions = instructions.trim();
    if instructions.is_empty() {
        return SYSTEM_PROMPT.to_string();
    }
    format!(
        "{SYSTEM_PROMPT}\n\nThe repository's own instructions win over the rules above:\n{instructions}"
    )
}

/// The user turn: recent subjects (blank ones dropped), then the change.
pub fn user_text(recent_subjects: &[String], changes: &str) -> String {
    let mut text = String::new();
    let subjects: Vec<&str> = recent_subjects
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if !subjects.is_empty() {
        text.push_str("Recent commit subjects in this repository:\n");
        for subject in subjects {
            text.push_str(&format!("- {subject}\n"));
        }
        text.push('\n');
    }
    text.push_str("The change to describe:\n");
    text.push_str(changes);
    text
}

/// The `router::complete` request. `thinking_level` is omitted for
/// `CommitThinking::Default`, so the model's own default applies.
pub fn build_payload(
    model_id: &str,
    thinking: CommitThinking,
    system_prompt: &str,
    user_text: &str,
    timestamp_ms: u64,
) -> Value {
    let (provider, model) = split_model(model_id);
    let mut payload = json!({
        "model": model,
        "system_prompt": system_prompt,
        "messages": [{
            "role": "user",
            "content": [{ "type": "text", "text": user_text }],
            "timestamp": timestamp_ms,
        }],
        "max_output_tokens": MAX_OUTPUT_TOKENS,
    });
    if let Some(provider) = provider {
        payload["provider"] = json!(provider);
    }
    if let Some(level) = thinking.level() {
        payload["thinking_level"] = json!(level);
    }
    payload
}

/// The `text` blocks of a `router::complete` answer, concatenated.
pub fn extract_text(answer: &Value) -> String {
    answer
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<String>()
        })
        .unwrap_or_default()
}

/// Models still wrap the message now and then despite the prompt: trim it,
/// unwrap one whole-text ``` fence (with or without a language tag), then one
/// pair of wrapping quotes.
pub fn clean_message(raw: &str) -> String {
    let text = strip_fence(raw.trim()).trim();
    strip_quotes(text).trim().to_string()
}

fn strip_fence(text: &str) -> &str {
    let Some(body) = text
        .strip_prefix("```")
        .and_then(|rest| rest.strip_suffix("```"))
    else {
        return text;
    };
    // Two separate fences ("```a``` and ```b```") are not one wrapper.
    if body.contains("```") {
        return text;
    }
    match body.split_once('\n') {
        // A first line that is one bare word is the language tag; any other
        // first line is the message itself.
        Some((tag, rest)) if !tag.contains(char::is_whitespace) => rest,
        _ => body,
    }
}

fn strip_quotes(text: &str) -> &str {
    for (open, close) in [('"', '"'), ('\'', '\''), ('\u{201c}', '\u{201d}')] {
        if let Some(inner) = text
            .strip_prefix(open)
            .and_then(|rest| rest.strip_suffix(close))
        {
            // `"fix a" and "fix b"` is two quotes, not one wrapper.
            if !inner.contains(open) && !inner.contains(close) {
                return inner;
            }
        }
    }
    text
}

/// A turn the router ended in error answers `Ok` with `stop_reason: "error"`
/// rather than failing the call; surface that as a router failure.
fn router_error_message(answer: &Value) -> Option<String> {
    let message = answer.get("message")?;
    if message.get("stop_reason").and_then(Value::as_str) != Some("error") {
        return None;
    }
    Some(
        message
            .get("error_message")
            .and_then(Value::as_str)
            .unwrap_or("the model ended its turn with an error")
            .to_string(),
    )
}

pub async fn commit_message(
    cfg: &CommitMessagesConfig,
    router: &dyn Router,
    req: CommitMessageRequest,
    now_ms: u64,
) -> Result<CommitMessageResponse, Error> {
    if req.changes.trim().is_empty() {
        return Err(fail(
            CODE_EMPTY_CHANGES,
            "changes is empty: pass the diff text of the files to commit",
        ));
    }
    let model = resolve_model(cfg, req.fallback_model.as_deref())?;
    let payload = build_payload(
        &model,
        cfg.thinking,
        &system_prompt(&cfg.instructions),
        &user_text(
            req.recent_subjects.as_deref().unwrap_or_default(),
            &cap_changes(&req.changes),
        ),
        now_ms,
    );

    let answer = router.complete(payload).await.map_err(|e| {
        let detail = match &e {
            Error::Remote { message, .. } => message.clone(),
            other => other.to_string(),
        };
        fail(
            CODE_ROUTER_FAILED,
            format!("router::complete failed for model {model}: {detail}"),
        )
    })?;
    if let Some(detail) = router_error_message(&answer) {
        return Err(fail(
            CODE_ROUTER_FAILED,
            format!("router::complete failed for model {model}: {detail}"),
        ));
    }

    let message = clean_message(&extract_text(&answer));
    if message.is_empty() {
        return Err(fail(
            CODE_EMPTY_MESSAGE,
            format!(
                "model {model} returned no commit message text; try again or pick another model"
            ),
        ));
    }
    Ok(CommitMessageResponse { message, model })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn cfg(
        model: Option<&str>,
        thinking: CommitThinking,
        instructions: &str,
    ) -> CommitMessagesConfig {
        CommitMessagesConfig {
            model: model.map(str::to_string),
            thinking,
            instructions: instructions.to_string(),
        }
    }

    fn code(err: &Error) -> &str {
        match err {
            Error::Remote { code, .. } => code,
            other => panic!("expected a coded error, got {other:?}"),
        }
    }

    fn message_of(err: &Error) -> &str {
        match err {
            Error::Remote { message, .. } => message,
            other => panic!("expected a coded error, got {other:?}"),
        }
    }

    // ── model resolution ──────────────────────────────────────────────

    #[test]
    fn configured_model_wins_over_fallback_and_is_trimmed() {
        let c = cfg(Some("  openai::gpt-5  "), CommitThinking::Low, "");
        assert_eq!(
            resolve_model(&c, Some("anthropic::sonnet")).unwrap(),
            "openai::gpt-5"
        );
    }

    #[test]
    fn fallback_is_used_when_config_is_unset_or_blank() {
        for model in [None, Some(""), Some("   ")] {
            let c = cfg(model, CommitThinking::Low, "");
            assert_eq!(
                resolve_model(&c, Some(" anthropic::sonnet ")).unwrap(),
                "anthropic::sonnet"
            );
        }
    }

    #[test]
    fn no_model_anywhere_is_a_coded_error() {
        let c = cfg(None, CommitThinking::Low, "");
        for fallback in [None, Some(""), Some("  ")] {
            let err = resolve_model(&c, fallback).unwrap_err();
            assert_eq!(code(&err), CODE_NO_MODEL);
            assert!(message_of(&err).contains("no model for commit messages"));
            assert!(message_of(&err).contains("commit_messages.model"));
        }
    }

    #[test]
    fn config_response_trims_model_and_maps_blank_to_null() {
        let r = commit_message_config(&cfg(
            Some(" a::b "),
            CommitThinking::High,
            "Use Conventional Commits",
        ));
        assert_eq!(
            r,
            CommitMessageConfigResponse {
                model: Some("a::b".into()),
                thinking: CommitThinking::High,
                instructions: "Use Conventional Commits".into(),
            }
        );
        assert_eq!(
            commit_message_config(&cfg(Some("  "), CommitThinking::Low, "")).model,
            None
        );
        let wire =
            serde_json::to_value(commit_message_config(&CommitMessagesConfig::default())).unwrap();
        assert_eq!(
            wire,
            json!({ "model": null, "thinking": "low", "instructions": "" })
        );
    }

    // ── provider split ────────────────────────────────────────────────

    #[test]
    fn split_model_takes_the_first_double_colon() {
        assert_eq!(
            split_model("anthropic::claude-sonnet-4"),
            (Some("anthropic"), "claude-sonnet-4")
        );
        assert_eq!(
            split_model("openrouter::meta::llama:free"),
            (Some("openrouter"), "meta::llama:free")
        );
        assert_eq!(split_model("gpt-5"), (None, "gpt-5"));
        assert_eq!(split_model("ns:model"), (None, "ns:model"));
        assert_eq!(split_model("::model"), (None, "::model"));
        assert_eq!(split_model("provider::"), (None, "provider::"));
    }

    // ── changes cap ───────────────────────────────────────────────────

    #[test]
    fn short_changes_pass_through() {
        let exact = "x".repeat(MAX_CHANGES_CHARS);
        assert_eq!(cap_changes(&exact), exact);
        assert_eq!(cap_changes("diff"), "diff");
    }

    #[test]
    fn long_changes_are_cut_on_a_char_boundary_and_marked() {
        // 3-byte chars: a byte-indexed cut at 60_000 would land mid-character.
        let long = "€".repeat(MAX_CHANGES_CHARS + 5);
        let capped = cap_changes(&long);
        assert_eq!(
            capped,
            format!("{}\n[diff truncated]", "€".repeat(MAX_CHANGES_CHARS))
        );

        let ends_in_newline = format!("{}\nrest", "a".repeat(MAX_CHANGES_CHARS - 1));
        assert_eq!(
            cap_changes(&ends_in_newline),
            format!("{}\n[diff truncated]", "a".repeat(MAX_CHANGES_CHARS - 1))
        );
    }

    // ── prompt ────────────────────────────────────────────────────────

    #[test]
    fn system_prompt_is_verbatim_and_appends_instructions() {
        let base = system_prompt("");
        assert!(
            base.starts_with("You write git commit messages for the change the user shows you.\n")
        );
        assert!(base
            .ends_with("Match the style of the repository's recent subjects when they are given."));
        assert_eq!(system_prompt("  \n "), base);

        let with = system_prompt("Use Conventional Commits");
        assert_eq!(
            with,
            format!(
                "{base}\n\nThe repository's own instructions win over the rules above:\nUse Conventional Commits"
            )
        );
    }

    #[test]
    fn user_text_lists_subjects_then_the_change() {
        assert_eq!(user_text(&[], "DIFF"), "The change to describe:\nDIFF");
        assert_eq!(
            user_text(&["feat: a".into(), " fix: b ".into(), "  ".into()], "DIFF"),
            "Recent commit subjects in this repository:\n- feat: a\n- fix: b\n\nThe change to describe:\nDIFF"
        );
    }

    #[test]
    fn payload_splits_provider_and_sends_thinking_level() {
        let p = build_payload(
            "anthropic::sonnet",
            CommitThinking::Medium,
            "SYS",
            "USER",
            1234,
        );
        assert_eq!(
            p,
            json!({
                "model": "sonnet",
                "provider": "anthropic",
                "system_prompt": "SYS",
                "messages": [{
                    "role": "user",
                    "content": [{ "type": "text", "text": "USER" }],
                    "timestamp": 1234,
                }],
                "max_output_tokens": 1024,
                "thinking_level": "medium",
            })
        );
    }

    #[test]
    fn payload_omits_provider_for_bare_ids_and_thinking_for_default() {
        let p = build_payload("gpt-5", CommitThinking::Default, "SYS", "USER", 1);
        assert_eq!(p["model"], "gpt-5");
        assert!(p.get("provider").is_none());
        assert!(p.get("thinking_level").is_none());
    }

    // ── response text ─────────────────────────────────────────────────

    #[test]
    fn extract_text_joins_text_blocks_and_skips_the_rest() {
        let answer = json!({ "message": { "content": [
            { "type": "thinking", "thinking": "hmm" },
            { "type": "text", "text": "feat: a" },
            { "type": "tool_call", "name": "x" },
            { "type": "text", "text": "\n\nbody" },
        ]}});
        assert_eq!(extract_text(&answer), "feat: a\n\nbody");
        assert_eq!(extract_text(&json!({})), "");
        assert_eq!(
            extract_text(&json!({ "message": { "content": "nope" } })),
            ""
        );
    }

    #[test]
    fn clean_message_strips_fences() {
        assert_eq!(clean_message("  feat: a\n\nbody \n"), "feat: a\n\nbody");
        assert_eq!(clean_message("```\nfeat: a\n```"), "feat: a");
        assert_eq!(
            clean_message("```text\nfeat: a\n\nbody\n```\n"),
            "feat: a\n\nbody"
        );
        assert_eq!(clean_message("```git-commit\nfeat: a\n```"), "feat: a");
        // A first line with spaces is the message, not a tag.
        assert_eq!(
            clean_message("```fix: a thing\n\nbody\n```"),
            "fix: a thing\n\nbody"
        );
        assert_eq!(clean_message("```feat: a```"), "feat: a");
        // Two fences, or a fence that is not the whole text, stay.
        assert_eq!(clean_message("```a``` and ```b```"), "```a``` and ```b```");
        assert_eq!(
            clean_message("feat: a\n\n```\ncode\n```"),
            "feat: a\n\n```\ncode\n```"
        );
    }

    #[test]
    fn clean_message_strips_one_pair_of_wrapping_quotes() {
        assert_eq!(clean_message("\"feat: a\""), "feat: a");
        assert_eq!(clean_message("'feat: a'"), "feat: a");
        assert_eq!(clean_message("\u{201c}feat: a\u{201d}"), "feat: a");
        assert_eq!(clean_message("```\n\"feat: a\"\n```"), "feat: a");
        // Interior quotes mean these were never one wrapper.
        assert_eq!(
            clean_message("\"fix a\" and \"fix b\""),
            "\"fix a\" and \"fix b\""
        );
        assert_eq!(clean_message("fix: \"a\""), "fix: \"a\"");
        assert_eq!(clean_message("\""), "\"");
    }

    // ── handler ───────────────────────────────────────────────────────

    struct StubRouter {
        answer: Result<Value, Error>,
        seen: Mutex<Vec<Value>>,
    }

    impl StubRouter {
        fn new(answer: Result<Value, Error>) -> Self {
            Self {
                answer,
                seen: Mutex::new(Vec::new()),
            }
        }
        fn saying(text: &str) -> Self {
            Self::new(Ok(
                json!({ "message": { "content": [{ "type": "text", "text": text }], "stop_reason": "end" } }),
            ))
        }
    }

    #[async_trait]
    impl Router for StubRouter {
        async fn complete(&self, payload: Value) -> Result<Value, Error> {
            self.seen.lock().unwrap().push(payload);
            self.answer.clone()
        }
    }

    fn request(
        changes: &str,
        subjects: Option<Vec<&str>>,
        fallback: Option<&str>,
    ) -> CommitMessageRequest {
        CommitMessageRequest {
            changes: changes.to_string(),
            recent_subjects: subjects.map(|s| s.into_iter().map(String::from).collect()),
            fallback_model: fallback.map(String::from),
        }
    }

    #[tokio::test]
    async fn generates_a_message_with_the_configured_model_and_prompt() {
        let router = StubRouter::saying("```\nfix: handle empty diff\n```");
        let c = cfg(
            Some("anthropic::sonnet"),
            CommitThinking::High,
            "Use Conventional Commits",
        );
        let out = commit_message(
            &c,
            &router,
            request("DIFF", Some(vec!["feat: x"]), Some("other::model")),
            99,
        )
        .await
        .unwrap();
        assert_eq!(
            out,
            CommitMessageResponse {
                message: "fix: handle empty diff".into(),
                model: "anthropic::sonnet".into()
            }
        );

        let seen = router.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0]["provider"], "anthropic");
        assert_eq!(seen[0]["model"], "sonnet");
        assert_eq!(seen[0]["thinking_level"], "high");
        assert_eq!(seen[0]["max_output_tokens"], 1024);
        assert_eq!(seen[0]["messages"][0]["timestamp"], 99);
        assert!(seen[0]["system_prompt"]
            .as_str()
            .unwrap()
            .ends_with("Use Conventional Commits"));
        assert_eq!(
            seen[0]["messages"][0]["content"][0]["text"],
            "Recent commit subjects in this repository:\n- feat: x\n\nThe change to describe:\nDIFF"
        );
    }

    #[tokio::test]
    async fn falls_back_to_the_callers_model_and_reports_it() {
        let router = StubRouter::saying("feat: a");
        let out = commit_message(
            &CommitMessagesConfig::default(),
            &router,
            request("DIFF", None, Some("gpt-5")),
            1,
        )
        .await
        .unwrap();
        assert_eq!(out.model, "gpt-5");
        let seen = router.seen.lock().unwrap();
        assert_eq!(seen[0]["model"], "gpt-5");
        assert!(seen[0].get("provider").is_none());
        assert_eq!(seen[0]["thinking_level"], "low");
    }

    #[tokio::test]
    async fn rejects_blank_changes_before_calling_the_router() {
        let router = StubRouter::saying("feat: a");
        let c = cfg(Some("a::b"), CommitThinking::Low, "");
        let err = commit_message(&c, &router, request(" \n\t", None, None), 1)
            .await
            .unwrap_err();
        assert_eq!(code(&err), CODE_EMPTY_CHANGES);
        assert!(router.seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn no_model_never_reaches_the_router() {
        let router = StubRouter::saying("feat: a");
        let err = commit_message(
            &CommitMessagesConfig::default(),
            &router,
            request("DIFF", None, None),
            1,
        )
        .await
        .unwrap_err();
        assert_eq!(code(&err), CODE_NO_MODEL);
        assert!(router.seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn router_failures_name_router_complete_and_its_message() {
        let c = cfg(Some("a::b"), CommitThinking::Low, "");

        let rejected = StubRouter::new(Err(Error::Remote {
            code: "R100".into(),
            message: "provider a is not configured".into(),
            stacktrace: None,
        }));
        let err = commit_message(&c, &rejected, request("DIFF", None, None), 1)
            .await
            .unwrap_err();
        assert_eq!(code(&err), CODE_ROUTER_FAILED);
        assert!(message_of(&err).contains("router::complete"));
        assert!(message_of(&err).contains("provider a is not configured"));

        let timed_out = StubRouter::new(Err(Error::Timeout));
        let err = commit_message(&c, &timed_out, request("DIFF", None, None), 1)
            .await
            .unwrap_err();
        assert_eq!(code(&err), CODE_ROUTER_FAILED);
        assert!(message_of(&err).contains("timed out"));

        let errored_turn = StubRouter::new(Ok(json!({ "message": {
            "content": [], "stop_reason": "error", "error_message": "rate limited",
        }})));
        let err = commit_message(&c, &errored_turn, request("DIFF", None, None), 1)
            .await
            .unwrap_err();
        assert_eq!(code(&err), CODE_ROUTER_FAILED);
        assert!(message_of(&err).contains("rate limited"));
    }

    #[tokio::test]
    async fn empty_model_text_is_an_error() {
        let c = cfg(Some("a::b"), CommitThinking::Low, "");
        for text in ["", "  \n", "``` ```", "\"\""] {
            let router = StubRouter::saying(text);
            let err = commit_message(&c, &router, request("DIFF", None, None), 1)
                .await
                .unwrap_err();
            assert_eq!(code(&err), CODE_EMPTY_MESSAGE, "text {text:?}");
        }
    }

    #[test]
    fn request_tolerates_null_optionals_and_the_engine_injected_caller_id() {
        let req: CommitMessageRequest = serde_json::from_value(json!({
            "changes": "DIFF",
            "recent_subjects": null,
            "fallback_model": null,
            "_caller_worker_id": "w-1",
        }))
        .unwrap();
        assert_eq!(req.changes, "DIFF");
        assert!(req.recent_subjects.is_none() && req.fallback_model.is_none());
        serde_json::from_value::<CommitMessageRequest>(json!({})).expect_err("changes is required");
        serde_json::from_value::<CommitMessageConfigRequest>(json!({ "_caller_worker_id": "w-1" }))
            .unwrap();
    }
}
