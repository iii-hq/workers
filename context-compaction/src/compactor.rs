//! Proactive token-budget compactor.
//!
//! Subscribes to the `agent::events` stream and accumulates token usage per
//! session. When usage / context_window crosses the configured threshold
//! (default 0.85), calls `session::compact` via the iii bus to checkpoint
//! a summary and reset the running window.
//!
//! Per-session state lives in a [`CompactorRegistry`] keyed by the stream
//! item's `group_id` (the session id). Drives a different code path than
//! [`crate::watcher`], which is reactive (post-overflow) rather than
//! proactive (threshold-triggered).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use harness_types::{
    AgentEvent, AgentMessage, AssistantMessageEvent, ContentBlock, ToolResultMessage, Usage,
};
use iii_sdk::{
    FunctionRef, IIIError, RegisterFunctionMessage, RegisterTriggerInput, Trigger, TriggerRequest,
    III,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;

const FN_ID: &str = "context_compaction::compactor";
const STREAM: &str = "agent::events";

const DEFAULT_THRESHOLD_PCT: f32 = 0.85;
const DEFAULT_CONTEXT_WINDOW: u64 = 200_000;
const FILE_OP_TOOLS: &[&str] = &["read", "write", "edit"];

/// Set of file operations a compaction summarises. Inlined from the
/// `session-tree` worker's wire shape; matches the JSON `details` field
/// passed to `session::compact`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactionDetails {
    #[serde(default)]
    pub read_files: Vec<String>,
    #[serde(default)]
    pub modified_files: Vec<String>,
}

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
    /// Build a config with [`DEFAULT_CONTEXT_WINDOW`]. Callers that want a
    /// model-aware window should use [`Self::resolve`] (async, hits the
    /// `models-catalog` worker on the bus) or set `context_window` directly.
    pub fn new(session_id: String, provider: String, model: String) -> Self {
        Self {
            session_id,
            threshold_pct: DEFAULT_THRESHOLD_PCT,
            model,
            provider,
            context_window: DEFAULT_CONTEXT_WINDOW,
        }
    }

    /// Build a config, resolving `context_window` via `models::get` on the
    /// iii bus. Falls back to [`DEFAULT_CONTEXT_WINDOW`] when the lookup
    /// fails (no `models-catalog` worker on the bus, unknown model, etc.).
    pub async fn resolve(iii: &III, session_id: String, provider: String, model: String) -> Self {
        let context_window = lookup_context_window(iii, &provider, &model)
            .await
            .unwrap_or(DEFAULT_CONTEXT_WINDOW);
        Self {
            session_id,
            threshold_pct: DEFAULT_THRESHOLD_PCT,
            model,
            provider,
            context_window,
        }
    }
}

async fn lookup_context_window(iii: &III, provider: &str, model_id: &str) -> Option<u64> {
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "models::get".to_string(),
            payload: json!({ "provider": provider, "id": model_id }),
            action: None,
            timeout_ms: Some(2_000),
        })
        .await
        .ok()?;
    resp.get("context_window").and_then(|v| v.as_u64())
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
    #[error("session-tree error: {0}")]
    Storage(String),
    #[error("summarise error: {0}")]
    Summarise(String),
}

/// Bus surface needed by the compactor — the two `session::*` operations.
///
/// Production callers wrap an `iii_sdk::III` via [`IiiSdkBus`]; tests inject
/// an in-memory implementation. The trait is small on purpose: keep the
/// abstraction scoped to exactly what the compactor needs from `session-tree`.
#[async_trait]
pub trait IiiBus: Send + Sync {
    /// Load the active session's messages. Mirrors `session::messages`.
    async fn load_messages(&self, session_id: &str) -> Result<Vec<AgentMessage>, CompactionError>;

    /// Append a compaction entry. Mirrors `session::compact`. Returns the new
    /// entry's id.
    async fn compact(
        &self,
        session_id: &str,
        summary: &str,
        details: &CompactionDetails,
        tokens_before: u64,
    ) -> Result<String, CompactionError>;
}

/// Production [`IiiBus`] backed by the iii bus. Calls `session::messages`
/// and `session::compact` on whichever worker registered them
/// (`session-tree` in the standard topology).
pub struct IiiSdkBus {
    pub iii: III,
}

impl IiiSdkBus {
    pub fn new(iii: III) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl IiiBus for IiiSdkBus {
    async fn load_messages(&self, session_id: &str) -> Result<Vec<AgentMessage>, CompactionError> {
        let resp = self
            .iii
            .trigger(TriggerRequest {
                function_id: "session::messages".to_string(),
                payload: json!({ "session_id": session_id }),
                action: None,
                timeout_ms: Some(5_000),
            })
            .await
            .map_err(|e| CompactionError::Storage(e.to_string()))?;
        let messages = resp.get("messages").cloned().unwrap_or_else(|| json!([]));
        serde_json::from_value(messages).map_err(|e| CompactionError::Storage(e.to_string()))
    }

    async fn compact(
        &self,
        session_id: &str,
        summary: &str,
        details: &CompactionDetails,
        tokens_before: u64,
    ) -> Result<String, CompactionError> {
        let resp = self
            .iii
            .trigger(TriggerRequest {
                function_id: "session::compact".to_string(),
                payload: json!({
                    "session_id": session_id,
                    "summary": summary,
                    "details": details,
                    "tokens_before": tokens_before,
                }),
                action: None,
                timeout_ms: Some(10_000),
            })
            .await
            .map_err(|e| CompactionError::Storage(e.to_string()))?;
        resp.get("entry_id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| CompactionError::Storage("session::compact returned no entry_id".into()))
    }
}

/// Internal state of an active compactor.
#[derive(Debug, Default)]
struct CompactionState {
    last_usage: Usage,
    rolling_total: u64,
    in_flight: bool,
    last_compaction_id: Option<String>,
}

/// Watches `AgentEvent`s and triggers compaction when token usage exceeds
/// `config.threshold_pct * config.context_window`.
pub struct Compactor<F: SummariseFn + 'static> {
    pub bus: Arc<dyn IiiBus>,
    pub summariser: Arc<F>,
    pub config: CompactionConfig,
    state: Mutex<CompactionState>,
}

impl<F> Compactor<F>
where
    F: SummariseFn + 'static,
{
    pub fn new(bus: Arc<dyn IiiBus>, summariser: Arc<F>, config: CompactionConfig) -> Self {
        Self {
            bus,
            summariser,
            config,
            state: Mutex::new(CompactionState::default()),
        }
    }

    pub fn config_set_threshold(&mut self, pct: f32) {
        self.config.threshold_pct = pct.clamp(0.0, 1.0);
    }

    pub async fn current_usage_pct(&self) -> f32 {
        let state = self.state.lock().await;
        usage_pct(state.rolling_total, self.config.context_window)
    }

    pub async fn last_compaction_id(&self) -> Option<String> {
        self.state.lock().await.last_compaction_id.clone()
    }

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

    pub async fn compact_now(
        &self,
        custom_instructions: Option<String>,
    ) -> Result<String, CompactionError> {
        let session_id = self.config.session_id.clone();
        let messages = self.bus.load_messages(&session_id).await?;
        let file_ops = extract_file_ops(&messages);

        let tokens_before = {
            let state = self.state.lock().await;
            state.rolling_total
        };

        let summary = self
            .summariser
            .summarise(messages, custom_instructions)
            .await?;

        let entry_id = self
            .bus
            .compact(&session_id, &summary, &file_ops, tokens_before)
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

/// Per-session [`Compactor`] registry. Routes incoming `agent::events`
/// stream items to the right Compactor by `group_id` (= session id) and
/// creates one on first sight of a session.
pub struct CompactorRegistry<F: SummariseFn + 'static> {
    bus: Arc<dyn IiiBus>,
    summariser: Arc<F>,
    /// `Some` in production (used to call `models::get` for context_window
    /// resolution). `None` in tests, in which case new sessions get a
    /// default config without any bus call.
    iii: Option<III>,
    compactors: Mutex<HashMap<String, Arc<Compactor<F>>>>,
}

impl<F: SummariseFn + 'static> CompactorRegistry<F> {
    pub fn new(bus: Arc<dyn IiiBus>, summariser: Arc<F>, iii: III) -> Self {
        Self {
            bus,
            summariser,
            iii: Some(iii),
            compactors: Mutex::new(HashMap::new()),
        }
    }

    /// Stream-trigger handler entry point. The `payload` is the engine's
    /// stream-item envelope: `{ stream_name, group_id, item_id, data }`.
    pub async fn handle(self: &Arc<Self>, payload: Value) -> Result<Value, IIIError> {
        let Some(session_id) = payload
            .get("group_id")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return Ok(json!({ "ok": true, "skipped": "no_group_id" }));
        };
        let data = payload.get("data").cloned().unwrap_or(Value::Null);
        let event: AgentEvent = match serde_json::from_value(data) {
            Ok(e) => e,
            Err(_) => {
                return Ok(json!({ "ok": true, "skipped": "not_agent_event" }));
            }
        };
        let compactor = self.get_or_create(&session_id, &event).await;
        compactor.observe(&event).await;
        Ok(json!({ "ok": true }))
    }

    async fn get_or_create(&self, session_id: &str, event: &AgentEvent) -> Arc<Compactor<F>> {
        {
            let map = self.compactors.lock().await;
            if let Some(c) = map.get(session_id) {
                return Arc::clone(c);
            }
        }
        let (provider, model) = first_seen_provider_model(event)
            .unwrap_or_else(|| ("unknown".into(), "unknown".into()));
        let config = match &self.iii {
            Some(iii) => {
                CompactionConfig::resolve(iii, session_id.to_string(), provider, model).await
            }
            None => CompactionConfig::new(session_id.to_string(), provider, model),
        };
        let compactor = Arc::new(Compactor::new(
            Arc::clone(&self.bus),
            Arc::clone(&self.summariser),
            config,
        ));
        let mut map = self.compactors.lock().await;
        Arc::clone(map.entry(session_id.to_string()).or_insert(compactor))
    }

    #[cfg(test)]
    fn for_tests(bus: Arc<dyn IiiBus>, summariser: Arc<F>) -> Self {
        Self {
            bus,
            summariser,
            iii: None,
            compactors: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    async fn seed(&self, session_id: String, compactor: Arc<Compactor<F>>) {
        self.compactors.lock().await.insert(session_id, compactor);
    }
}

fn first_seen_provider_model(event: &AgentEvent) -> Option<(String, String)> {
    match event {
        AgentEvent::TurnEnd {
            message: AgentMessage::Assistant(a),
            ..
        } => Some((a.provider.clone(), a.model.clone())),
        AgentEvent::MessageUpdate {
            message: AgentMessage::Assistant(a),
            ..
        } => Some((a.provider.clone(), a.model.clone())),
        AgentEvent::MessageEnd {
            message: AgentMessage::Assistant(a),
        } => Some((a.provider.clone(), a.model.clone())),
        _ => None,
    }
}

/// Handle for the compactor's registered function + trigger.
pub struct CompactorHandle {
    function: Option<FunctionRef>,
    trigger: Option<Trigger>,
}

impl CompactorHandle {
    pub fn unregister_all(mut self) {
        if let Some(t) = self.trigger.take() {
            t.unregister();
        }
        if let Some(f) = self.function.take() {
            f.unregister();
        }
    }
}

/// Register the compactor's function + stream trigger on `iii`.
pub fn register<F: SummariseFn + 'static>(
    iii: &III,
    summariser: Arc<F>,
) -> Result<CompactorHandle, IIIError> {
    let bus: Arc<dyn IiiBus> = Arc::new(IiiSdkBus::new(iii.clone()));
    let registry = Arc::new(CompactorRegistry::new(bus, summariser, iii.clone()));

    let registry_for_handler = Arc::clone(&registry);
    let function = iii.register_function((
        RegisterFunctionMessage::with_id(FN_ID.into())
            .with_description("Per-session token-budget compactor on agent::events.".into()),
        move |payload: Value| {
            let registry = Arc::clone(&registry_for_handler);
            async move { registry.handle(payload).await }
        },
    ));
    let trigger = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "stream".into(),
        function_id: FN_ID.into(),
        config: json!({ "stream_name": STREAM }),
        metadata: None,
    })?;
    Ok(CompactorHandle {
        function: Some(function),
        trigger: Some(trigger),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_types::{
        AssistantMessage, ContentBlock, StopReason, TextContent, ToolResultMessage, UserMessage,
    };
    use std::sync::atomic::{AtomicU64, Ordering};

    /// One compaction recorded by [`InMemoryBus::compact`]. Some fields are
    /// kept for future assertions even when current tests only check
    /// `entry_id` / `session_id` / `summary`.
    #[derive(Debug, Clone)]
    #[allow(dead_code)]
    struct CompactionRecord {
        entry_id: String,
        session_id: String,
        summary: String,
        details: CompactionDetails,
        tokens_before: u64,
    }

    /// Fully in-process [`IiiBus`] for tests.
    struct InMemoryBus {
        messages: Mutex<HashMap<String, Vec<AgentMessage>>>,
        compactions: Mutex<Vec<CompactionRecord>>,
        seq: AtomicU64,
    }

    impl InMemoryBus {
        fn new() -> Self {
            Self {
                messages: Mutex::new(HashMap::new()),
                compactions: Mutex::new(Vec::new()),
                seq: AtomicU64::new(0),
            }
        }

        async fn seed_messages(&self, session_id: &str, msgs: Vec<AgentMessage>) {
            self.messages
                .lock()
                .await
                .insert(session_id.to_string(), msgs);
        }

        async fn recorded_compactions(&self) -> Vec<CompactionRecord> {
            self.compactions.lock().await.clone()
        }
    }

    #[async_trait]
    impl IiiBus for InMemoryBus {
        async fn load_messages(
            &self,
            session_id: &str,
        ) -> Result<Vec<AgentMessage>, CompactionError> {
            Ok(self
                .messages
                .lock()
                .await
                .get(session_id)
                .cloned()
                .unwrap_or_default())
        }

        async fn compact(
            &self,
            session_id: &str,
            summary: &str,
            details: &CompactionDetails,
            tokens_before: u64,
        ) -> Result<String, CompactionError> {
            let n = self.seq.fetch_add(1, Ordering::SeqCst);
            let entry_id = format!("compact-{n}");
            self.compactions.lock().await.push(CompactionRecord {
                entry_id: entry_id.clone(),
                session_id: session_id.to_string(),
                summary: summary.to_string(),
                details: details.clone(),
                tokens_before,
            });
            Ok(entry_id)
        }
    }

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

    fn compactor_for(
        bus: Arc<InMemoryBus>,
        session_id: String,
        threshold: f32,
        ctx_window: u64,
    ) -> Arc<Compactor<MockSummariser>> {
        let summariser = Arc::new(MockSummariser {
            canned: "summary".into(),
        });
        let mut config =
            CompactionConfig::new(session_id, "anthropic".into(), "claude-opus-4-7".into());
        config.threshold_pct = threshold;
        config.context_window = ctx_window;
        Arc::new(Compactor::new(bus, summariser, config))
    }

    #[tokio::test]
    async fn default_config_uses_85_percent_threshold() {
        let cfg = CompactionConfig::new("s".into(), "anthropic".into(), "claude-opus-4-7".into());
        assert!((cfg.threshold_pct - 0.85).abs() < f32::EPSILON);
        assert_eq!(cfg.context_window, DEFAULT_CONTEXT_WINDOW);
    }

    #[tokio::test]
    async fn observe_folds_usage_into_rolling_total() {
        let bus = Arc::new(InMemoryBus::new());
        let comp = compactor_for(Arc::clone(&bus), "s1".into(), 0.99, 100_000);
        comp.observe(&turn_end_with_usage(100, 50)).await;
        comp.observe(&turn_end_with_usage(200, 25)).await;
        let pct = comp.current_usage_pct().await;
        assert!((pct - 0.00375).abs() < 1e-6);
    }

    #[tokio::test]
    async fn current_usage_pct_math() {
        let bus = Arc::new(InMemoryBus::new());
        let comp = compactor_for(Arc::clone(&bus), "s1".into(), 0.99, 1_000);
        comp.observe(&turn_end_with_usage(300, 200)).await;
        let pct = comp.current_usage_pct().await;
        assert!((pct - 0.5).abs() < 1e-6);
    }

    #[tokio::test]
    async fn current_usage_pct_zero_window_safe() {
        let bus = Arc::new(InMemoryBus::new());
        let comp = compactor_for(Arc::clone(&bus), "s1".into(), 0.5, 0);
        comp.observe(&turn_end_with_usage(10, 10)).await;
        assert!((comp.current_usage_pct().await - 0.0).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn observe_streamed_usage_event_updates_state() {
        let bus = Arc::new(InMemoryBus::new());
        let comp = compactor_for(Arc::clone(&bus), "s1".into(), 0.99, 100_000);
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
        let bus = Arc::new(InMemoryBus::new());
        let session_id = "s1".to_string();
        bus.seed_messages(
            &session_id,
            vec![AgentMessage::User(UserMessage {
                content: vec![ContentBlock::Text(TextContent { text: "hi".into() })],
                timestamp: 0,
            })],
        )
        .await;
        let comp = compactor_for(Arc::clone(&bus), session_id.clone(), 0.5, 1_000);

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

        let recorded = bus.recorded_compactions().await;
        assert_eq!(
            recorded.len(),
            1,
            "exactly one compaction should be recorded"
        );
        assert_eq!(recorded[0].session_id, session_id);
    }

    #[tokio::test]
    async fn compact_now_appends_compaction_entry_and_resets_state() {
        let bus = Arc::new(InMemoryBus::new());
        let session_id = "s1".to_string();
        bus.seed_messages(
            &session_id,
            vec![AgentMessage::User(UserMessage {
                content: vec![ContentBlock::Text(TextContent { text: "x".into() })],
                timestamp: 0,
            })],
        )
        .await;
        let comp = compactor_for(Arc::clone(&bus), session_id.clone(), 0.99, 1_000);
        comp.observe(&turn_end_with_usage(100, 50)).await;
        let id = comp.compact_now(None).await.unwrap();

        let recorded = bus.recorded_compactions().await;
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].entry_id, id);
        assert_eq!(recorded[0].summary, "summary");

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
        let bus: Arc<dyn IiiBus> = Arc::new(InMemoryBus::new());
        let mut config =
            CompactionConfig::new("s1".into(), "anthropic".into(), "claude-opus-4-7".into());
        config.threshold_pct = 0.5;
        let summariser = Arc::new(MockSummariser { canned: "s".into() });
        let mut comp = Compactor::new(bus, summariser, config);
        comp.config_set_threshold(2.0);
        assert!((comp.config.threshold_pct - 1.0).abs() < f32::EPSILON);
        comp.config_set_threshold(-1.0);
        assert!((comp.config.threshold_pct - 0.0).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn registry_routes_event_by_group_id() {
        let bus = Arc::new(InMemoryBus::new());
        let summariser = Arc::new(MockSummariser { canned: "s".into() });
        let registry: Arc<CompactorRegistry<MockSummariser>> = Arc::new(
            CompactorRegistry::for_tests(Arc::clone(&bus) as Arc<dyn IiiBus>, summariser),
        );

        let comp = compactor_for(Arc::clone(&bus), "s1".into(), 0.99, 1_000);
        registry.seed("s1".into(), Arc::clone(&comp)).await;

        let envelope = json!({
            "stream_name": "agent::events",
            "group_id": "s1",
            "item_id": "s1:0",
            "data": serde_json::to_value(turn_end_with_usage(300, 200)).unwrap(),
        });
        let resp = registry.handle(envelope).await.unwrap();
        assert_eq!(resp["ok"], true);

        let pct = comp.current_usage_pct().await;
        assert!((pct - 0.5).abs() < 1e-6);
    }

    #[tokio::test]
    async fn registry_skips_envelope_without_group_id() {
        let bus = Arc::new(InMemoryBus::new());
        let summariser = Arc::new(MockSummariser { canned: "s".into() });
        let registry: Arc<CompactorRegistry<MockSummariser>> = Arc::new(
            CompactorRegistry::for_tests(Arc::clone(&bus) as Arc<dyn IiiBus>, summariser),
        );

        let envelope = json!({ "stream_name": "agent::events", "data": {} });
        let resp = registry.handle(envelope).await.unwrap();
        assert_eq!(resp["skipped"], "no_group_id");
    }

    #[tokio::test]
    async fn registry_skips_envelope_with_non_event_data() {
        let bus = Arc::new(InMemoryBus::new());
        let summariser = Arc::new(MockSummariser { canned: "s".into() });
        let registry: Arc<CompactorRegistry<MockSummariser>> = Arc::new(
            CompactorRegistry::for_tests(Arc::clone(&bus) as Arc<dyn IiiBus>, summariser),
        );

        let envelope = json!({
            "stream_name": "agent::events",
            "group_id": "s1",
            "item_id": "s1:0",
            "data": { "garbage": true },
        });
        let resp = registry.handle(envelope).await.unwrap();
        assert_eq!(resp["skipped"], "not_agent_event");
    }
}
