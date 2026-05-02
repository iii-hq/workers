//! Context compaction worker.
//!
//! Subscribes to harness `agent::events/<session_id>` streams, accumulates
//! token usage from `AgentEvent::TurnEnd` and `AssistantMessageEvent::Usage`,
//! and triggers compaction when usage / context_window crosses the configured
//! threshold (default 0.85). Persists the resulting summary via
//! [`session_tree::compact`].

use std::sync::Arc;

use async_trait::async_trait;
use harness_types::{
    AgentEvent, AgentMessage, AssistantMessageEvent, ContentBlock, ToolResultMessage, Usage,
};
use models_catalog::get as get_model;
use session_tree::{
    compact as session_tree_compact, load_messages, CompactionDetails, SessionError, SessionStore,
};
use tokio::sync::Mutex;

const DEFAULT_THRESHOLD_PCT: f32 = 0.85;
const DEFAULT_CONTEXT_WINDOW: u64 = 200_000;
const FILE_OP_TOOLS: &[&str] = &["read", "write", "edit"];

/// Configuration for a single-session [`Compactor`].
#[derive(Debug, Clone)]
pub struct CompactionConfig {
    pub session_id: String,
    pub threshold_pct: f32,
    pub model: String,
    pub provider: String,
    pub context_window: u64,
}

impl CompactionConfig {
    /// Build a config, resolving `context_window` from the embedded
    /// `models-catalog`. Falls back to [`DEFAULT_CONTEXT_WINDOW`] for unknown
    /// model+provider pairs so callers always get a usable threshold.
    pub fn new(session_id: String, provider: String, model: String) -> Self {
        let context_window =
            get_model(&provider, &model).map_or(DEFAULT_CONTEXT_WINDOW, |m| m.context_window);
        Self {
            session_id,
            threshold_pct: DEFAULT_THRESHOLD_PCT,
            model,
            provider,
            context_window,
        }
    }
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            threshold_pct: DEFAULT_THRESHOLD_PCT,
            model: String::new(),
            provider: String::new(),
            context_window: DEFAULT_CONTEXT_WINDOW,
        }
    }
}

/// Pluggable summarisation backend.
#[async_trait]
pub trait SummariseFn: Send + Sync {
    async fn summarise(
        &self,
        messages: Vec<AgentMessage>,
        instructions: Option<String>,
    ) -> Result<String, CompactionError>;
}

/// Errors produced by the compactor.
#[derive(Debug, thiserror::Error)]
pub enum CompactionError {
    #[error("storage error: {0}")]
    Storage(#[from] SessionError),
    #[error("summarise error: {0}")]
    Summarise(String),
}

/// Internal state of an active compactor.
#[derive(Debug, Default)]
struct CompactionState {
    /// Most recent observed token usage from the in-flight stream.
    last_usage: Usage,
    /// Rolling input + output total. Folded across `TurnEnd` boundaries to
    /// survive between turns.
    rolling_total: u64,
    /// True while a compaction task is in flight; prevents `observe` from
    /// double-firing.
    in_flight: bool,
    /// Last compaction entry id appended to the session, if any.
    last_compaction_id: Option<String>,
}

/// Watches `AgentEvent`s and triggers compaction when token usage exceeds
/// `config.threshold_pct * config.context_window`.
pub struct Compactor<S: SessionStore + Send + Sync + 'static, F: SummariseFn + 'static> {
    pub store: Arc<S>,
    pub summariser: Arc<F>,
    pub config: CompactionConfig,
    state: Mutex<CompactionState>,
}

impl<S, F> Compactor<S, F>
where
    S: SessionStore + Send + Sync + 'static,
    F: SummariseFn + 'static,
{
    pub fn new(store: Arc<S>, summariser: Arc<F>, config: CompactionConfig) -> Self {
        Self {
            store,
            summariser,
            config,
            state: Mutex::new(CompactionState::default()),
        }
    }

    /// Update the threshold for the live config. Useful for runtime tuning.
    pub fn config_set_threshold(&mut self, pct: f32) {
        self.config.threshold_pct = pct.clamp(0.0, 1.0);
    }

    /// Current usage as a fraction of the configured context window.
    /// Returns 0.0 when the context window is zero (defensive divide-by-zero).
    pub async fn current_usage_pct(&self) -> f32 {
        let state = self.state.lock().await;
        usage_pct(state.rolling_total, self.config.context_window)
    }

    /// Returns the id of the most recently appended compaction entry, if any.
    pub async fn last_compaction_id(&self) -> Option<String> {
        self.state.lock().await.last_compaction_id.clone()
    }

    /// Fold an `AgentEvent`. When the threshold is crossed and no compaction
    /// is in flight, schedules `compact_now` on a background task.
    pub async fn observe(self: &Arc<Self>, event: &AgentEvent) {
        let triggered = {
            let mut state = self.state.lock().await;
            match event {
                AgentEvent::MessageUpdate {
                    llm_event: AssistantMessageEvent::Usage(u),
                    ..
                } => {
                    state.last_usage = *u;
                }
                AgentEvent::TurnEnd { message, .. } => {
                    if let AgentMessage::Assistant(a) = message {
                        if let Some(u) = a.usage {
                            state.last_usage = u;
                        }
                    }
                    state.rolling_total = state
                        .rolling_total
                        .saturating_add(state.last_usage.input)
                        .saturating_add(state.last_usage.output);
                    state.last_usage = Usage::default();
                }
                _ => {}
            }
            let pct = usage_pct(state.rolling_total, self.config.context_window);
            if pct >= self.config.threshold_pct && !state.in_flight {
                state.in_flight = true;
                true
            } else {
                false
            }
        };

        if triggered {
            let this = Arc::clone(self);
            tokio::spawn(async move {
                if let Err(err) = this.compact_now(None).await {
                    tracing::warn!(error = %err, session = %this.config.session_id, "compaction failed");
                }
            });
        }
    }

    /// Run compaction synchronously. Loads the active path, summarises it,
    /// extracts file ops, and appends a `Compaction` entry.
    pub async fn compact_now(
        &self,
        custom_instructions: Option<String>,
    ) -> Result<String, CompactionError> {
        let session_id = self.config.session_id.clone();
        let messages = load_messages(self.store.as_ref(), &session_id, None).await?;
        let file_ops = extract_file_ops(&messages);

        let tokens_before = {
            let state = self.state.lock().await;
            state.rolling_total
        };

        let summary = self
            .summariser
            .summarise(messages, custom_instructions)
            .await?;

        let entry_id = session_tree_compact(
            self.store.as_ref(),
            &session_id,
            summary,
            file_ops,
            None,
            tokens_before,
        )
        .await?;

        let mut state = self.state.lock().await;
        state.in_flight = false;
        state.rolling_total = 0;
        state.last_usage = Usage::default();
        state.last_compaction_id = Some(entry_id.clone());
        Ok(entry_id)
    }
}

fn usage_pct(rolling_total: u64, context_window: u64) -> f32 {
    if context_window == 0 {
        return 0.0;
    }
    (rolling_total as f32) / (context_window as f32)
}

/// Walks tool-call and tool-result content blocks across `messages` and pulls
/// out paths from `read`, `write`, and `edit` tool calls.
pub fn extract_file_ops(messages: &[AgentMessage]) -> CompactionDetails {
    let mut read_files: Vec<String> = Vec::new();
    let mut modified_files: Vec<String> = Vec::new();
    for message in messages {
        match message {
            AgentMessage::Assistant(a) => {
                gather_from_blocks(&a.content, &mut read_files, &mut modified_files);
            }
            AgentMessage::User(u) => {
                gather_from_blocks(&u.content, &mut read_files, &mut modified_files);
            }
            AgentMessage::ToolResult(tr) => {
                if let Some(path) = path_for_tool_result(tr) {
                    push_unique_for_tool(&tr.tool_name, path, &mut read_files, &mut modified_files);
                }
            }
            AgentMessage::Custom(_) => {}
        }
    }
    CompactionDetails {
        read_files,
        modified_files,
    }
}

fn gather_from_blocks(
    blocks: &[ContentBlock],
    read_files: &mut Vec<String>,
    modified_files: &mut Vec<String>,
) {
    for block in blocks {
        if let ContentBlock::ToolCall {
            name, arguments, ..
        } = block
        {
            let lname = name.to_ascii_lowercase();
            if !FILE_OP_TOOLS.iter().any(|t| lname.ends_with(t)) {
                continue;
            }
            let Some(path) = path_from_args(arguments) else {
                continue;
            };
            push_unique_for_tool(name, path, read_files, modified_files);
        } else if let ContentBlock::ToolResult { content, .. } = block {
            gather_from_blocks(content, read_files, modified_files);
        }
    }
}

fn path_for_tool_result(tr: &ToolResultMessage) -> Option<String> {
    let from_details = tr
        .details
        .get("path")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            tr.details
                .get("file_path")
                .and_then(serde_json::Value::as_str)
        })
        .map(str::to_string);
    if from_details.is_some() {
        return from_details;
    }
    None
}

fn path_from_args(args: &serde_json::Value) -> Option<String> {
    args.get("path")
        .and_then(serde_json::Value::as_str)
        .or_else(|| args.get("file_path").and_then(serde_json::Value::as_str))
        .map(str::to_string)
}

fn push_unique_for_tool(
    tool_name: &str,
    path: String,
    read_files: &mut Vec<String>,
    modified_files: &mut Vec<String>,
) {
    let lname = tool_name.to_ascii_lowercase();
    if lname.ends_with("read") {
        if !read_files.contains(&path) {
            read_files.push(path);
        }
    } else if lname.ends_with("write") || lname.ends_with("edit") {
        if !modified_files.contains(&path) {
            modified_files.push(path);
        }
    }
}

/// Topic and function id constants exposed by [`register_with_iii`].
pub mod topics {
    /// Source topic the compactor subscribes to. The agent loop publishes
    /// `AgentEvent` payloads here.
    pub const AGENT_EVENTS: &str = "agent::events";
    /// Sink topic the compactor publishes to when overflow is detected.
    /// `agent::transform_context` subscribers run before the next turn.
    pub const TRANSFORM_CONTEXT: &str = "agent::transform_context";
    /// Function id under which the subscriber handler registers.
    pub const SUBSCRIBER_FN: &str = "context_compaction::on_event";
}

/// Decide whether an `AgentEvent` payload (the wire form of [`AgentEvent`])
/// signals a context-overflow condition that warrants a `transform_context`
/// publication.
///
/// Returns `true` when the message in `MessageEnd`, `TurnEnd`, or
/// `MessageUpdate` carries `error_kind == "context_overflow"`. Other shapes
/// (including unrelated event types) return `false` so the subscriber stays a
/// pure pass-through except on the trigger condition.
pub fn payload_signals_overflow(payload: &serde_json::Value) -> bool {
    let kind = payload.get("type").and_then(serde_json::Value::as_str);
    let Some(kind) = kind else { return false };
    let message = match kind {
        "message_end" | "message_start" | "message_update" => payload.get("message"),
        "turn_end" => payload.get("message"),
        _ => None,
    };
    let Some(message) = message else { return false };
    // AgentMessage is tagged: {"role": "assistant", "error_kind": "context_overflow", ...}
    let error_kind = message
        .get("error_kind")
        .and_then(serde_json::Value::as_str);
    matches!(error_kind, Some("context_overflow"))
}

/// Register the context-compaction subscriber on `iii`.
///
/// On startup this:
/// 1. Registers the function [`topics::SUBSCRIBER_FN`].
/// 2. Binds it to the [`topics::AGENT_EVENTS`] pubsub topic via a
///    `subscribe` trigger.
///
/// When an event with `error_kind == "context_overflow"` arrives the handler
/// publishes a payload to [`topics::TRANSFORM_CONTEXT`] so downstream
/// `transform_context` subscribers can produce a compacted message tail.
///
/// Returns a handle that can deregister both refs in one shot.
pub fn register_with_iii(iii: &iii_sdk::III) -> Result<CompactionSubscriber, iii_sdk::IIIError> {
    use iii_sdk::{IIIError, RegisterFunctionMessage, RegisterTriggerInput, TriggerAction};
    use serde_json::json;

    let iii_for_handler = iii.clone();
    let function_ref = iii.register_function((
        RegisterFunctionMessage::with_id(topics::SUBSCRIBER_FN.into()).with_description(
            "Subscriber that watches agent::events for context_overflow signals and \
                 republishes them as agent::transform_context"
                .into(),
        ),
        move |payload: serde_json::Value| {
            let iii = iii_for_handler.clone();
            async move {
                if !payload_signals_overflow(&payload) {
                    return Ok(json!({ "ok": true, "compacted": false }));
                }
                let request = json!({
                    "topic": topics::TRANSFORM_CONTEXT,
                    "data": {
                        "reason": "context_overflow",
                        "source_event": payload,
                    },
                });
                iii.trigger(iii_sdk::TriggerRequest {
                    function_id: "publish".to_string(),
                    payload: request,
                    action: Some(TriggerAction::Void),
                    timeout_ms: None,
                })
                .await
                .map_err(|e| IIIError::Handler(e.to_string()))?;
                Ok(json!({ "ok": true, "compacted": true }))
            }
        },
    ));

    let trigger = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "subscribe".to_string(),
        function_id: topics::SUBSCRIBER_FN.to_string(),
        config: json!({ "topic": topics::AGENT_EVENTS }),
        metadata: None,
    })?;

    Ok(CompactionSubscriber {
        function_ref: Some(function_ref),
        trigger: Some(trigger),
    })
}

/// Handle returned by [`register_with_iii`].
pub struct CompactionSubscriber {
    function_ref: Option<iii_sdk::FunctionRef>,
    trigger: Option<iii_sdk::Trigger>,
}

impl CompactionSubscriber {
    /// Unregister the subscriber and the underlying function.
    pub fn unregister_all(mut self) {
        if let Some(t) = self.trigger.take() {
            t.unregister();
        }
        if let Some(f) = self.function_ref.take() {
            f.unregister();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_types::{
        AssistantMessage, ContentBlock, ErrorKind, StopReason, TextContent, ToolResultMessage,
        UserMessage,
    };
    use session_tree::{append_message, create_session, InMemoryStore};

    struct MockSummariser {
        canned: String,
    }

    #[async_trait]
    impl SummariseFn for MockSummariser {
        async fn summarise(
            &self,
            _messages: Vec<AgentMessage>,
            _instructions: Option<String>,
        ) -> Result<String, CompactionError> {
            Ok(self.canned.clone())
        }
    }

    fn assistant_with_usage(input: u64, output: u64) -> AgentMessage {
        AgentMessage::Assistant(AssistantMessage {
            content: vec![ContentBlock::Text(TextContent { text: "ok".into() })],
            stop_reason: StopReason::End,
            error_message: None,
            error_kind: None,
            usage: Some(Usage {
                input,
                output,
                cache_read: 0,
                cache_write: 0,
                cost_usd: None,
            }),
            model: "claude-opus-4-7".into(),
            provider: "anthropic".into(),
            timestamp: 0,
        })
    }

    fn turn_end_with_usage(input: u64, output: u64) -> AgentEvent {
        AgentEvent::TurnEnd {
            message: assistant_with_usage(input, output),
            tool_results: Vec::new(),
        }
    }

    async fn compactor_for(
        store: Arc<InMemoryStore>,
        session_id: String,
        threshold: f32,
        ctx_window: u64,
    ) -> Arc<Compactor<InMemoryStore, MockSummariser>> {
        let summariser = Arc::new(MockSummariser {
            canned: "summary".into(),
        });
        let mut config =
            CompactionConfig::new(session_id, "anthropic".into(), "claude-opus-4-7".into());
        config.threshold_pct = threshold;
        config.context_window = ctx_window;
        Arc::new(Compactor::new(store, summariser, config))
    }

    #[tokio::test]
    async fn default_config_uses_85_percent_threshold() {
        let cfg = CompactionConfig::new("s".into(), "anthropic".into(), "claude-opus-4-7".into());
        assert!((cfg.threshold_pct - 0.85).abs() < f32::EPSILON);
        assert_eq!(cfg.context_window, 1_000_000);
    }

    #[tokio::test]
    async fn config_unknown_model_falls_back_to_default_window() {
        let cfg = CompactionConfig::new("s".into(), "made-up".into(), "made-up-model".into());
        assert_eq!(cfg.context_window, DEFAULT_CONTEXT_WINDOW);
    }

    #[tokio::test]
    async fn observe_folds_usage_into_rolling_total() {
        let store = Arc::new(InMemoryStore::new());
        let session_id = create_session(store.as_ref(), None, None).await.unwrap();
        let comp = compactor_for(Arc::clone(&store), session_id, 0.99, 100_000).await;

        comp.observe(&turn_end_with_usage(100, 50)).await;
        comp.observe(&turn_end_with_usage(200, 25)).await;

        let pct = comp.current_usage_pct().await;
        assert!((pct - 0.00375).abs() < 1e-6);
    }

    #[tokio::test]
    async fn current_usage_pct_math() {
        let store = Arc::new(InMemoryStore::new());
        let session_id = create_session(store.as_ref(), None, None).await.unwrap();
        let comp = compactor_for(Arc::clone(&store), session_id, 0.99, 1_000).await;
        comp.observe(&turn_end_with_usage(300, 200)).await;
        let pct = comp.current_usage_pct().await;
        assert!((pct - 0.5).abs() < 1e-6);
    }

    #[tokio::test]
    async fn current_usage_pct_zero_window_safe() {
        let store = Arc::new(InMemoryStore::new());
        let session_id = create_session(store.as_ref(), None, None).await.unwrap();
        let comp = compactor_for(Arc::clone(&store), session_id, 0.5, 0).await;
        comp.observe(&turn_end_with_usage(10, 10)).await;
        assert!((comp.current_usage_pct().await - 0.0).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn observe_streamed_usage_event_updates_state() {
        let store = Arc::new(InMemoryStore::new());
        let session_id = create_session(store.as_ref(), None, None).await.unwrap();
        let comp = compactor_for(Arc::clone(&store), session_id, 0.99, 100_000).await;

        comp.observe(&AgentEvent::MessageUpdate {
            message: assistant_with_usage(0, 0),
            llm_event: AssistantMessageEvent::Usage(Usage {
                input: 400,
                output: 100,
                cache_read: 0,
                cache_write: 0,
                cost_usd: None,
            }),
        })
        .await;
        comp.observe(&AgentEvent::TurnEnd {
            message: AgentMessage::Assistant(AssistantMessage {
                content: Vec::new(),
                stop_reason: StopReason::End,
                error_message: None,
                error_kind: None,
                usage: None,
                model: "claude-opus-4-7".into(),
                provider: "anthropic".into(),
                timestamp: 0,
            }),
            tool_results: Vec::new(),
        })
        .await;

        let pct = comp.current_usage_pct().await;
        assert!((pct - 0.005).abs() < 1e-6);
    }

    #[tokio::test]
    async fn threshold_trigger_spawns_compaction_only_once() {
        let store = Arc::new(InMemoryStore::new());
        let session_id = create_session(store.as_ref(), None, None).await.unwrap();
        append_message(
            store.as_ref(),
            &session_id,
            None,
            AgentMessage::User(UserMessage {
                content: vec![ContentBlock::Text(TextContent { text: "hi".into() })],
                timestamp: 0,
            }),
        )
        .await
        .unwrap();
        let comp = compactor_for(Arc::clone(&store), session_id.clone(), 0.5, 1_000).await;

        for _ in 0..5 {
            comp.observe(&turn_end_with_usage(200, 100)).await;
        }

        for _ in 0..30 {
            tokio::task::yield_now().await;
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            if comp.last_compaction_id().await.is_some() {
                break;
            }
        }

        let entries = store.load_entries(&session_id).await.unwrap();
        let compactions: Vec<_> = entries
            .iter()
            .filter(|e| matches!(e, session_tree::SessionEntry::Compaction { .. }))
            .collect();
        assert_eq!(
            compactions.len(),
            1,
            "exactly one compaction should be appended"
        );
    }

    #[tokio::test]
    async fn compact_now_appends_compaction_entry_and_resets_state() {
        let store = Arc::new(InMemoryStore::new());
        let session_id = create_session(store.as_ref(), None, None).await.unwrap();
        append_message(
            store.as_ref(),
            &session_id,
            None,
            AgentMessage::User(UserMessage {
                content: vec![ContentBlock::Text(TextContent { text: "x".into() })],
                timestamp: 0,
            }),
        )
        .await
        .unwrap();
        let comp = compactor_for(Arc::clone(&store), session_id.clone(), 0.99, 1_000).await;
        comp.observe(&turn_end_with_usage(100, 50)).await;
        let id = comp.compact_now(None).await.unwrap();
        let entries = store.load_entries(&session_id).await.unwrap();
        let last = entries.last().unwrap();
        assert_eq!(last.id(), id);
        assert!(matches!(
            last,
            session_tree::SessionEntry::Compaction { .. }
        ));
        let pct = comp.current_usage_pct().await;
        assert!((pct - 0.0).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn extract_file_ops_gathers_paths_from_tool_calls() {
        let messages = vec![
            AgentMessage::Assistant(AssistantMessage {
                content: vec![
                    ContentBlock::ToolCall {
                        id: "1".into(),
                        name: "read".into(),
                        arguments: serde_json::json!({ "path": "src/a.rs" }),
                    },
                    ContentBlock::ToolCall {
                        id: "2".into(),
                        name: "write".into(),
                        arguments: serde_json::json!({ "file_path": "src/b.rs" }),
                    },
                    ContentBlock::ToolCall {
                        id: "3".into(),
                        name: "edit".into(),
                        arguments: serde_json::json!({ "path": "src/c.rs" }),
                    },
                ],
                stop_reason: StopReason::Tool,
                error_message: None,
                error_kind: None,
                usage: None,
                model: "claude-opus-4-7".into(),
                provider: "anthropic".into(),
                timestamp: 0,
            }),
            AgentMessage::ToolResult(ToolResultMessage {
                tool_call_id: "4".into(),
                tool_name: "read".into(),
                content: Vec::new(),
                details: serde_json::json!({ "path": "docs/x.md" }),
                is_error: false,
                timestamp: 0,
            }),
        ];

        let ops = extract_file_ops(&messages);
        assert!(ops.read_files.contains(&"src/a.rs".to_string()));
        assert!(ops.read_files.contains(&"docs/x.md".to_string()));
        assert!(ops.modified_files.contains(&"src/b.rs".to_string()));
        assert!(ops.modified_files.contains(&"src/c.rs".to_string()));
    }

    #[tokio::test]
    async fn extract_file_ops_dedupes() {
        let block = ContentBlock::ToolCall {
            id: "1".into(),
            name: "read".into(),
            arguments: serde_json::json!({ "path": "x" }),
        };
        let messages = vec![
            AgentMessage::Assistant(AssistantMessage {
                content: vec![block.clone(), block.clone()],
                stop_reason: StopReason::Tool,
                error_message: None,
                error_kind: None,
                usage: None,
                model: "claude-opus-4-7".into(),
                provider: "anthropic".into(),
                timestamp: 0,
            }),
            AgentMessage::Assistant(AssistantMessage {
                content: vec![block],
                stop_reason: StopReason::Tool,
                error_message: None,
                error_kind: None,
                usage: None,
                model: "claude-opus-4-7".into(),
                provider: "anthropic".into(),
                timestamp: 0,
            }),
        ];
        let ops = extract_file_ops(&messages);
        assert_eq!(ops.read_files, vec!["x".to_string()]);
    }

    #[tokio::test]
    async fn config_set_threshold_clamps() {
        let store = Arc::new(InMemoryStore::new());
        let session_id = create_session(store.as_ref(), None, None).await.unwrap();
        let mut config =
            CompactionConfig::new(session_id, "anthropic".into(), "claude-opus-4-7".into());
        config.threshold_pct = 0.5;
        let summariser = Arc::new(MockSummariser { canned: "s".into() });
        let mut comp = Compactor::new(store, summariser, config);
        comp.config_set_threshold(2.0);
        assert!((comp.config.threshold_pct - 1.0).abs() < f32::EPSILON);
        comp.config_set_threshold(-1.0);
        assert!((comp.config.threshold_pct - 0.0).abs() < f32::EPSILON);
    }

    fn assistant_with_overflow() -> AgentMessage {
        AgentMessage::Assistant(AssistantMessage {
            content: vec![ContentBlock::Text(TextContent {
                text: "ran out".into(),
            })],
            stop_reason: StopReason::Error,
            error_message: Some("context window exceeded".into()),
            error_kind: Some(ErrorKind::ContextOverflow),
            usage: None,
            model: "claude-opus-4-7".into(),
            provider: "anthropic".into(),
            timestamp: 0,
        })
    }

    #[test]
    fn payload_signals_overflow_message_end() {
        let event = AgentEvent::MessageEnd {
            message: assistant_with_overflow(),
        };
        let payload = serde_json::to_value(&event).unwrap();
        assert!(payload_signals_overflow(&payload));
    }

    #[test]
    fn payload_signals_overflow_turn_end() {
        let event = AgentEvent::TurnEnd {
            message: assistant_with_overflow(),
            tool_results: Vec::new(),
        };
        let payload = serde_json::to_value(&event).unwrap();
        assert!(payload_signals_overflow(&payload));
    }

    #[test]
    fn payload_does_not_signal_when_no_error_kind() {
        let event = AgentEvent::MessageEnd {
            message: AgentMessage::User(UserMessage {
                content: vec![ContentBlock::Text(TextContent { text: "hi".into() })],
                timestamp: 0,
            }),
        };
        let payload = serde_json::to_value(&event).unwrap();
        assert!(!payload_signals_overflow(&payload));
    }

    #[test]
    fn payload_does_not_signal_for_unrelated_events() {
        let payload = serde_json::to_value(&AgentEvent::AgentStart).unwrap();
        assert!(!payload_signals_overflow(&payload));
        let payload = serde_json::json!({ "type": "tool_execution_start" });
        assert!(!payload_signals_overflow(&payload));
    }
}
