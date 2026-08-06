//! The five synchronous hook points (harness.md § Hooks). Siblings bind iii
//! triggers to `harness::hook::*` types; the harness invokes them in-path, in
//! priority order, and acts on their return value (veto / hold / mutate).
//!
//! P1 registers the trigger types and collects bindings; the in-path chain
//! invocation is wired into the loop in a later phase.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PRE_TURN: &str = "harness::hook::pre-turn";
pub const PRE_GENERATE: &str = "harness::hook::pre-generate";
pub const POST_GENERATE: &str = "harness::hook::post-generate";
pub const PRE_TRIGGER: &str = "harness::hook::pre-trigger";
pub const POST_TRIGGER: &str = "harness::hook::post-trigger";
pub const POST_TURN: &str = "harness::hook::post-turn";

/// The six hook points, in spec order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookPoint {
    PreTurn,
    PreGenerate,
    PostGenerate,
    PreTrigger,
    PostTrigger,
    PostTurn,
}

impl HookPoint {
    pub fn trigger_type(self) -> &'static str {
        match self {
            HookPoint::PreTurn => PRE_TURN,
            HookPoint::PreGenerate => PRE_GENERATE,
            HookPoint::PostGenerate => POST_GENERATE,
            HookPoint::PreTrigger => PRE_TRIGGER,
            HookPoint::PostTrigger => POST_TRIGGER,
            HookPoint::PostTurn => POST_TURN,
        }
    }

    pub fn all() -> [HookPoint; 6] {
        [
            HookPoint::PreTurn,
            HookPoint::PreGenerate,
            HookPoint::PostGenerate,
            HookPoint::PreTrigger,
            HookPoint::PostTrigger,
            HookPoint::PostTurn,
        ]
    }

    /// The `HookInput.point` field value (snake_case, per the spec contract).
    pub fn input_name(self) -> &'static str {
        match self {
            HookPoint::PreTurn => "pre_turn",
            HookPoint::PreGenerate => "pre_generate",
            HookPoint::PostGenerate => "post_generate",
            HookPoint::PreTrigger => "pre_trigger",
            HookPoint::PostTrigger => "post_trigger",
            HookPoint::PostTurn => "post_turn",
        }
    }

    /// Default `on_error` policy: fail-closed for `pre_*`, fail-open for
    /// `post_*` (harness.md § Chain, hold, and failure semantics).
    /// `post_turn` is fail-closed despite the name: it GATES completion the
    /// way `pre_*` hooks gate spend — an erroring validator must nudge the
    /// bounded retry budget, never silently pass a result.
    pub fn default_fail_closed(self) -> bool {
        matches!(
            self,
            HookPoint::PreTurn
                | HookPoint::PreGenerate
                | HookPoint::PreTrigger
                | HookPoint::PostTurn
        )
    }
}

/// The `config` of a `harness::hook::<point>` trigger binding.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HookTriggerConfig {
    /// pre/post_trigger only: target function_id globs to consult on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub functions: Option<Vec<String>>,
    /// post_turn only: session_id globs this validator gates (omit = all).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sessions: Option<Vec<String>>,
    /// post_turn only: template mode — send THIS argument object to the
    /// bound function instead of the hook envelope, with the turn's parsed
    /// result injected at `result_into`. Lets a plain composition function
    /// (`fp::pipe`) validate turns without speaking the hook contract; its
    /// receipt is read as the verdict (`valid`, or `short_circuited`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    /// post_turn template mode: JSON pointer where the result lands in
    /// `payload` (default `/value`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_into: Option<String>,
    /// post_turn only: custom corrective prompt sent VERBATIM when this
    /// validator denies (replaces the generic "result was not accepted"
    /// wrapper). Placeholders: `{value}` = the validator's measured value
    /// (fp::pipe receipt `value_preview`), `{reason}` = the deny reason.
    /// Validator ERRORS keep the generic text — a task-shaped prompt must
    /// not mask a broken validator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_prompt: Option<String>,
    /// Chain order: ascending, ties broken by function_id (default 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    /// Per-invocation timeout (default 5000ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Failure policy (default fail_closed for pre_* and post_turn,
    /// fail_open for the other post_*).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_error: Option<String>,
}

/// One parsed hook binding.
#[derive(Debug, Clone)]
pub struct HookBinding {
    pub function_id: String,
    pub functions: Option<Vec<String>>,
    pub sessions: Option<Vec<String>>,
    pub payload: Option<Value>,
    pub result_into: Option<String>,
    pub retry_prompt: Option<String>,
    pub priority: i64,
    pub timeout_ms: u64,
    pub fail_closed: bool,
}

impl HookBinding {
    fn parse(point: HookPoint, config: TriggerConfig) -> Result<Self, String> {
        let raw = if config.config.is_null() {
            Value::Object(Default::default())
        } else {
            config.config.clone()
        };
        let cfg: HookTriggerConfig =
            serde_json::from_value(raw).map_err(|e| format!("invalid hook config: {e}"))?;
        let fail_closed = match cfg.on_error.as_deref() {
            Some("fail_closed") => true,
            Some("fail_open") => false,
            Some(other) => return Err(format!("invalid on_error `{other}`")),
            None => point.default_fail_closed(),
        };
        Ok(HookBinding {
            function_id: config.function_id,
            functions: cfg.functions,
            sessions: cfg.sessions,
            payload: cfg.payload,
            result_into: cfg.result_into,
            retry_prompt: cfg.retry_prompt,
            priority: cfg.priority.unwrap_or(0),
            timeout_ms: cfg.timeout_ms.unwrap_or(5_000),
            fail_closed,
        })
    }
}

#[derive(Clone, Default)]
pub struct HookSet {
    inner: Arc<Mutex<HashMap<String, HookBinding>>>,
}

impl HookSet {
    fn add(&self, point: HookPoint, config: TriggerConfig) -> Result<(), String> {
        let binding = HookBinding::parse(point, config.clone())?;
        self.lock().insert(config.id, binding);
        Ok(())
    }

    fn remove(&self, id: &str) {
        self.lock().remove(id);
    }

    /// Bindings sorted into chain order: ascending priority, ties by
    /// function_id.
    pub fn ordered(&self) -> Vec<HookBinding> {
        let mut v: Vec<HookBinding> = self.lock().values().cloned().collect();
        v.sort_by(|a, b| {
            a.priority
                .cmp(&b.priority)
                .then_with(|| a.function_id.cmp(&b.function_id))
        });
        v
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    pub fn has_function_binding(&self, function_id: &str, target_function_id: &str) -> bool {
        self.lock().values().any(|binding| {
            binding.function_id == function_id
                && runner::functions_match(binding, target_function_id)
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, HookBinding>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }
}

struct HookTriggerHandler {
    point: HookPoint,
    set: HookSet,
}

#[async_trait]
impl TriggerHandler for HookTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let id = config.id.clone();
        let function_id = config.function_id.clone();
        self.set.add(self.point, config).map_err(Error::Handler)?;
        tracing::info!(trigger_type = self.point.trigger_type(), %id, %function_id, "hook binding registered");
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.set.remove(&config.id);
        Ok(())
    }
}

/// All five hook subscriber sets plus the engine handle for in-path
/// invocation. Cloned into [`crate::deps::Deps`].
#[derive(Clone)]
pub struct HookRegistry {
    pub iii: Arc<IIIClient>,
    pub pre_turn: HookSet,
    pub pre_generate: HookSet,
    pub post_generate: HookSet,
    pub pre_trigger: HookSet,
    pub post_trigger: HookSet,
    pub post_turn: HookSet,
    /// Agent-registered post-turn validators: subscription id → (owner
    /// session, engine unregister handle). The subscribe intercept records
    /// here so teardown stays owner-checked.
    /// ponytail: in-memory only — a harness restart orphans the engine-side
    /// binding (it re-arms, but ownership is forgotten; operator cleanup).
    /// Persist to state if that bites.
    owned: Arc<Mutex<HashMap<String, (String, iii_sdk::trigger::Trigger)>>>,
}

impl HookRegistry {
    /// Register the five hook trigger types and return the registry. Must run
    /// before function registration so handlers capture the sets.
    pub fn register(iii: &Arc<IIIClient>) -> Self {
        let registry = HookRegistry {
            iii: iii.clone(),
            pre_turn: HookSet::default(),
            pre_generate: HookSet::default(),
            post_generate: HookSet::default(),
            pre_trigger: HookSet::default(),
            post_trigger: HookSet::default(),
            post_turn: HookSet::default(),
            owned: Arc::new(Mutex::new(HashMap::new())),
        };
        registry.register_type(iii, HookPoint::PreTurn, registry.pre_turn.clone());
        registry.register_type(iii, HookPoint::PreGenerate, registry.pre_generate.clone());
        registry.register_type(iii, HookPoint::PostGenerate, registry.post_generate.clone());
        registry.register_type(iii, HookPoint::PreTrigger, registry.pre_trigger.clone());
        registry.register_type(iii, HookPoint::PostTrigger, registry.post_trigger.clone());
        registry.register_type(iii, HookPoint::PostTurn, registry.post_turn.clone());
        tracing::info!("registered the six harness::hook::* trigger types");
        registry
    }

    fn register_type(&self, iii: &Arc<IIIClient>, point: HookPoint, set: HookSet) {
        let description = match point {
            HookPoint::PreTurn => "Synchronous hook: first step of a turn, before any model spend. May veto.",
            HookPoint::PreGenerate => "Synchronous hook: after context assembly, before generation. May extend the system prompt, append messages, or veto.",
            HookPoint::PostGenerate => "Synchronous hook: after the final assistant message update. Observe only.",
            HookPoint::PreTrigger => "Synchronous hook: after the allow/deny policy passes, before the target is invoked. May deny, hold, or rewrite arguments.",
            HookPoint::PostTrigger => "Synchronous hook: after the target returns, before the result is appended. May rewrite content/details/is_error.",
            HookPoint::PostTurn => "Synchronous hook: at finalize, after the output contract validated the result, before the turn completes. Deny re-prompts the turn (bounded by max_validation_retries). Config `sessions` globs scope it; config `payload`+`result_into` bind a plain composition function (fp::pipe) as the validator.",
        };
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                point.trigger_type(),
                description,
                HookTriggerHandler { point, set },
            )
            .trigger_request_format::<HookTriggerConfig>(),
        );
    }

    /// Record an agent-registered post-turn validator; returns the
    /// subscription id handed back to the agent.
    pub fn record_owned(&self, session_id: &str, handle: iii_sdk::trigger::Trigger) -> String {
        let id = format!("posthook_{}", uuid::Uuid::new_v4().simple());
        self.owned
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.clone(), (session_id.to_string(), handle));
        id
    }

    /// Owner-checked teardown for an agent-registered post-turn validator.
    /// `None` = not one of ours (or not this caller's) — fall through to the
    /// normal subscription path.
    pub fn unregister_owned(&self, id: &str, session_id: &str) -> Option<bool> {
        let mut owned = self.owned.lock().unwrap_or_else(|p| p.into_inner());
        match owned.get(id) {
            Some((owner, _)) if owner == session_id => {
                let (_, handle) = owned.remove(id).expect("checked above");
                handle.unregister();
                Some(true)
            }
            _ => None,
        }
    }

    pub fn set_for(&self, point: HookPoint) -> &HookSet {
        match point {
            HookPoint::PreTurn => &self.pre_turn,
            HookPoint::PreGenerate => &self.pre_generate,
            HookPoint::PostGenerate => &self.post_generate,
            HookPoint::PreTrigger => &self.pre_trigger,
            HookPoint::PostTrigger => &self.post_trigger,
            HookPoint::PostTurn => &self.post_turn,
        }
    }

    pub fn filesystem_boundary(
        &self,
        target_function_id: &str,
    ) -> crate::filesystem_scope::FilesystemBoundary {
        // fp::pipe is stamped on behalf of its scoped steps: give it the
        // boundary a direct scoped call would get, or a pipe step would run
        // under a WIDER jail than the same call made directly. shell::exec
        // represents the watch's fixed shell::*/coder::* binding set.
        let target_function_id = if target_function_id == crate::filesystem_scope::PIPE_FUNCTION_ID
        {
            "shell::exec"
        } else {
            target_function_id
        };
        if self.post_trigger.has_function_binding(
            crate::filesystem_scope::FILESYSTEM_ACCESS_WATCH_ID,
            target_function_id,
        ) {
            crate::filesystem_scope::FilesystemBoundary::Workspace
        } else {
            crate::filesystem_scope::FilesystemBoundary::ConfiguredRoots
        }
    }
}

pub mod runner;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cfg(id: &str, function_id: &str, body: Value) -> TriggerConfig {
        TriggerConfig {
            id: id.into(),
            function_id: function_id.into(),
            config: body,
            metadata: None,
            namespace: None,
        }
    }

    #[test]
    fn bindings_order_by_priority_then_function_id() {
        let set = HookSet::default();
        set.add(
            HookPoint::PreTrigger,
            cfg("a", "z::hook", json!({ "priority": 5 })),
        )
        .unwrap();
        set.add(
            HookPoint::PreTrigger,
            cfg("b", "a::hook", json!({ "priority": 5 })),
        )
        .unwrap();
        set.add(
            HookPoint::PreTrigger,
            cfg("c", "m::hook", json!({ "priority": 1 })),
        )
        .unwrap();
        let order: Vec<String> = set.ordered().into_iter().map(|b| b.function_id).collect();
        assert_eq!(order, vec!["m::hook", "a::hook", "z::hook"]);
    }

    #[test]
    fn filesystem_boundary_tracks_the_live_access_watch_binding() {
        let set = HookSet::default();
        assert!(!set.has_function_binding(
            crate::filesystem_scope::FILESYSTEM_ACCESS_WATCH_ID,
            "shell::fs::ls"
        ));
        set.add(
            HookPoint::PostTrigger,
            cfg(
                "access-watch",
                crate::filesystem_scope::FILESYSTEM_ACCESS_WATCH_ID,
                json!({ "functions": ["shell::*", "coder::*"] }),
            ),
        )
        .unwrap();
        assert!(set.has_function_binding(
            crate::filesystem_scope::FILESYSTEM_ACCESS_WATCH_ID,
            "shell::fs::ls"
        ));
        assert!(!set.has_function_binding(
            crate::filesystem_scope::FILESYSTEM_ACCESS_WATCH_ID,
            "web::fetch"
        ));
        set.remove("access-watch");
        assert!(!set.has_function_binding(
            crate::filesystem_scope::FILESYSTEM_ACCESS_WATCH_ID,
            "shell::fs::ls"
        ));
    }

    #[test]
    fn pipe_boundary_matches_a_direct_scoped_call() {
        let registry = HookRegistry {
            iii: Arc::new(IIIClient::new("ws://127.0.0.1:0")),
            pre_turn: HookSet::default(),
            pre_generate: HookSet::default(),
            post_generate: HookSet::default(),
            pre_trigger: HookSet::default(),
            post_trigger: HookSet::default(),
            post_turn: HookSet::default(),
            owned: Arc::new(Mutex::new(HashMap::new())),
        };
        use crate::filesystem_scope::FilesystemBoundary;
        // no access watch bound: both direct scoped calls and the pipe run at
        // the default boundary
        assert_eq!(
            registry.filesystem_boundary("fp::pipe"),
            FilesystemBoundary::ConfiguredRoots
        );
        registry
            .post_trigger
            .add(
                HookPoint::PostTrigger,
                cfg(
                    "access-watch",
                    crate::filesystem_scope::FILESYSTEM_ACCESS_WATCH_ID,
                    json!({ "functions": ["shell::*", "coder::*"] }),
                ),
            )
            .unwrap();
        // watch bound to shell::*/coder::* only — the pipe must still pick up
        // Workspace, or its steps would run under a wider jail than direct calls
        assert_eq!(
            registry.filesystem_boundary("shell::exec"),
            FilesystemBoundary::Workspace
        );
        assert_eq!(
            registry.filesystem_boundary("fp::pipe"),
            FilesystemBoundary::Workspace
        );
        assert_eq!(
            registry.filesystem_boundary("web::fetch"),
            FilesystemBoundary::ConfiguredRoots
        );
    }

    #[test]
    fn on_error_default_depends_on_point() {
        let set = HookSet::default();
        set.add(HookPoint::PreTrigger, cfg("a", "h", json!({})))
            .unwrap();
        assert!(set.ordered()[0].fail_closed);

        let post = HookSet::default();
        post.add(HookPoint::PostTrigger, cfg("a", "h", json!({})))
            .unwrap();
        assert!(!post.ordered()[0].fail_closed);
    }

    #[test]
    fn invalid_on_error_rejected() {
        let set = HookSet::default();
        assert!(set
            .add(
                HookPoint::PreTurn,
                cfg("a", "h", json!({ "on_error": "maybe" }))
            )
            .is_err());
    }
}
