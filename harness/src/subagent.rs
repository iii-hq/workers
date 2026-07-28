//! Sub-agent spawning (harness.md § Sub-agents). A child is an ordinary
//! harness run in a child session, seeded through the same CAS as
//! `harness::send`. Spawning is fire-and-forget: the caller gets the child's
//! ids immediately and never its result — delegation flows one way, downward,
//! and outcomes come back only through the state the child writes and the
//! `harness::turn-completed` event its finalize emits. The `ParentLink` is
//! kept for event filters and console nesting, not for result delivery.

use serde_json::{json, Value};

use crate::config::WorkerConfig;
use crate::deps::Deps;
use crate::error::HarnessError;
use crate::functions::send::{self as send, normalize_message, TurnLineage};
use crate::functions::spawn::SpawnRequest;
use crate::ids;
use crate::policy;
use crate::prompt;
use crate::prompt::Mode;
use crate::trigger::ResultData;
use crate::types::content::ContentBlock;
use crate::types::turn::{fs_scope_metadata, FunctionPolicy, ParentLink, TurnOptions, TurnRecord};

/// The ids of a freshly-seeded child turn.
pub struct ChildIds {
    pub session_id: String,
    pub turn_id: String,
}

/// Guarded spawn from a live turn (the dispatch path AND the approval-release
/// path): enforce depth/fan-out, subset the policy against the parent, seed
/// the child with full linkage, and return its ids immediately. Guard
/// violations return an `is_error` result (never a throw) so the model can
/// adapt.
pub async fn spawn_from_turn(
    deps: &Deps,
    parent: &TurnRecord,
    call_id: &str,
    arguments: &Value,
) -> Result<ChildIds, ResultData> {
    let cfg = deps.cfg().await;
    let req: SpawnRequest = serde_json::from_value(arguments.clone()).map_err(|e| {
        is_error(
            "harness/invalid_request",
            format!("invalid spawn arguments: {e}"),
        )
    })?;
    // Depth budget.
    if parent.depth + 1 > cfg.max_depth {
        return Err(is_error(
            "harness/spawn_depth_exceeded",
            format!(
                "spawn depth {} exceeds max_depth {}",
                parent.depth + 1,
                cfg.max_depth
            ),
        ));
    }
    // Fan-out budget: children spawned by this turn (spawns settle instantly,
    // so the cap is a per-turn total, not a live count).
    let spawned = parent.spawned_children().len() as u32;
    if spawned >= cfg.max_children {
        return Err(is_error(
            "harness/spawn_fanout_exceeded",
            format!(
                "{spawned} children spawned this turn at or above max_children {} — the cap is \
                 PER TURN. Consolidate the remaining work into the children already running \
                 (one child can cover several parts), or start the remainder from a later turn \
                 (e.g. your next notification-woken one); do not retry this spawn in this turn.",
                cfg.max_children
            ),
        ));
    }

    let parent_link = ParentLink {
        session_id: parent.session_id.clone(),
        turn_id: parent.turn_id.clone(),
        function_call_id: call_id.to_string(),
    };
    seed_child(deps, &cfg, &req, Some(&parent_link), Some(parent))
        .await
        .map_err(|e| is_error(e.code(), e.to_string()))
}

/// The immediate function result for a successful fire-and-forget spawn. The
/// hint line teaches models still carrying the old parked-spawn doctrine that
/// no result will come back through this call.
pub fn spawned_result(child: &ChildIds) -> ResultData {
    let ids = json!({
        "child_session_id": child.session_id,
        "child_turn_id": child.turn_id,
    });
    let text = format!(
        "{ids}\nfire-and-forget: this turn will NOT receive the child's result — or its \
         failure. The child talks to the world only through the destination its task names; \
         read that destination, poll `harness::status` on the child for its health, and put a \
         deadline (a `timer` wake or a binding `lifecycle`) on anything you wait for."
    );
    ResultData {
        content: vec![ContentBlock::text(text)],
        is_error: false,
        details: json!({
            "child_session_id": child.session_id,
            "child_turn_id": child.turn_id,
            "fire_and_forget": true,
        }),
    }
}

/// Direct-call entry (a consumer starting a linked child). No parent linkage
/// or subsetting — the request's policy applies as-is, minus the leaf deny
/// set unless the spawn passed `options.orchestrator: true`.
pub async fn spawn_child(
    deps: &Deps,
    req: &SpawnRequest,
    parent: Option<&ParentLink>,
) -> Result<ChildIds, HarnessError> {
    let cfg = deps.cfg().await;
    seed_child(deps, &cfg, req, parent, None).await
}

/// Resolve a child's dispatch policy. An in-turn child starts from the
/// PARENT'S policy (`subset_policy` forbids escalation and honours an explicit
/// narrower request); a parentless (direct/CLI) spawn starts from explicit
/// options, else the read-only baseline. Then the capability wall: unless the
/// spawn opted into `orchestrator: true`, the child's deny globs gain
/// [`policy::CONTROL_PLANE_DENY`] — a leaf performs its assignment and updates
/// the shared medium; it cannot spawn, message sessions, or touch trigger
/// registrations, whatever its prompt says. Denies union through subsetting,
/// so a leaf's own children stay leaves. Finally an ask-mode child is clamped
/// to the configured default policy (`*` as shipped).
fn child_functions(
    cfg: &WorkerConfig,
    parent_record: Option<&TurnRecord>,
    requested: Option<&FunctionPolicy>,
    mode: Option<Mode>,
    orchestrator: bool,
) -> Option<FunctionPolicy> {
    let mut functions = match parent_record {
        Some(p) => policy::subset_policy(p.options.functions.as_ref(), requested),
        None => requested.cloned().or_else(|| cfg.default_functions.clone()),
    };
    if !orchestrator {
        if let Some(p) = functions.as_mut() {
            p.deny
                .extend(policy::CONTROL_PLANE_DENY.iter().map(|s| s.to_string()));
        }
    }
    crate::functions::send::clamp_for_mode(cfg, mode, functions)
}

/// Seed a child session + turn and enqueue its first step. When
/// `parent_record` is set the policy is subset against it, `max_turns` is
/// capped at the parent's remaining budget, and linkage metadata is recorded.
async fn seed_child(
    deps: &Deps,
    cfg: &WorkerConfig,
    req: &SpawnRequest,
    parent: Option<&ParentLink>,
    parent_record: Option<&TurnRecord>,
) -> Result<ChildIds, HarnessError> {
    let session = deps.session().await;

    let model = req
        .model
        .clone()
        .or_else(|| parent_record.map(|p| p.options.model.clone()))
        .ok_or_else(|| {
            HarnessError::InvalidRequest(
                "spawn requires a model — only an IN-TURN spawn inherits its parent's; a \
                 parentless spawn (console, workflow, CLI) inherits nothing, so name `model` \
                 explicitly in that spawn payload"
                    .into(),
            )
        })?;
    let provider = child_provider(req, parent_record);
    // Children get the embedded minimal sub-agent identity, never the
    // orchestrator prompt the router serves to top-level agents: a child
    // knows its one task, its state destination, and nothing else — by
    // design. Spawn `options.system_prompt` (+ override strategy) is the
    // escape hatch for a child that genuinely needs a different identity.
    let identity = prompt::SUBAGENT;

    let functions = child_functions(
        cfg,
        parent_record,
        req.options.as_ref().and_then(|o| o.functions.as_ref()),
        req.options.as_ref().and_then(|o| o.mode),
        req.options.as_ref().is_some_and(|o| o.orchestrator),
    );

    let depth = parent_record.map(|p| p.depth + 1).unwrap_or(0);
    let requested_turns = req
        .options
        .as_ref()
        .and_then(|o| o.max_turns)
        .unwrap_or(cfg.default_max_turns);
    let max_turns = match parent_record {
        Some(p) => {
            let remaining = p.options.max_turns.saturating_sub(p.turn_count).max(1);
            requested_turns.min(remaining).max(1)
        }
        None => requested_turns,
    };

    // Child session, with sub-agent linkage merged into SessionMeta.metadata.
    // A live parent turn gives the full linkage (resolve + display). A direct
    // parentless spawn has no parent turn, but a caller-supplied
    // `parent_session_id` still writes a display-only link so the console
    // nests the child (no policy inheritance, no parent-call resolution).
    // `spawned_by` is always "agent": every spawn is a direct call now —
    // trigger delivery never creates an agent.
    let linkage = match parent {
        Some(p) => Some(json!({
            "parent_session_id": p.session_id,
            "parent_turn_id": p.turn_id,
            "function_call_id": p.function_call_id,
            "depth": depth,
            "spawned_by": "agent",
        })),
        None => req.parent_session_id.as_ref().map(|psid| {
            json!({
                "parent_session_id": psid,
                "depth": depth,
                "spawned_by": "agent",
            })
        }),
    };
    let child_session_id = match &req.session_id {
        Some(id) => {
            let created = session.ensure(id, None, linkage.as_ref()).await?;
            if !created {
                // Reuse is legitimate (a fork, or delivering a reaction into an
                // existing chat) but silent reuse of a stale id is a classic
                // pipeline bug: the old transcript carries over and the console
                // keeps the session nested under whoever created it first.
                tracing::info!(
                    child_session_id = %id,
                    "harness::spawn reused an existing session — prior transcript and parent linkage retained"
                );
            }
            id.clone()
        }
        None => session.create(None, linkage.as_ref()).await?,
    };

    // The task is the child's opening user message — machine-sent, so every
    // spawn marks the entry (origin + a readable id prefix, the notify
    // pattern) or clients would render it as something the human typed.
    let task = normalize_message(req.task.clone())?;
    let (entry_id, origin) = (Some(ids::spawn_entry_id()), Some(json!({ "spawn": true })));
    let options = TurnOptions {
        model,
        provider,
        system_prompt: prompt::resolve_system_prompt(
            req.options.as_ref().and_then(|o| o.system_prompt.clone()),
            req.options
                .as_ref()
                .map(|o| o.system_prompt_strategy)
                .unwrap_or_default(),
            req.options.as_ref().and_then(|o| o.mode),
            Some(identity),
        ),
        mode: req.options.as_ref().and_then(|o| o.mode),
        max_turns,
        max_output_tokens: req
            .options
            .as_ref()
            .and_then(|o| o.max_output_tokens)
            .or_else(|| parent_record.and_then(|p| p.options.max_output_tokens)),
        // Children cannot widen or replace their root turn's execution
        // budget; every generation charges the same durable ledger.
        max_total_tokens: parent_record.and_then(|p| p.options.max_total_tokens),
        max_cost_usd: parent_record.and_then(|p| p.options.max_cost_usd),
        budget_root_session_id: parent_record
            .and_then(|p| p.options.budget_root_session_id.clone()),
        thinking_level: req.options.as_ref().and_then(|o| o.thinking_level),
        provider_options: None,
        output: req
            .options
            .as_ref()
            .and_then(|o| o.output.clone())
            .unwrap_or_default(),
        functions,
        metadata: child_filesystem_scope(
            req.options
                .as_ref()
                .and_then(|o| o.filesystem_root.as_deref()),
            parent_record,
        )?,
        max_validation_retries: cfg.max_validation_retries,
        max_transient_resumes: cfg.max_transient_resumes,
    };

    let lineage = TurnLineage {
        depth,
        parent: parent.cloned(),
        // Self-parent is dropped: a reaction delivered INTO session X (e.g. a
        // reporter posting into the chat) must not carry X as its display
        // parent, or its own turn-completed would match a
        // `parent_session_id: X` subscription and re-fire the reaction — an
        // infinite loop.
        display_parent_session_id: match parent {
            Some(_) => None,
            None => req
                .parent_session_id
                .clone()
                .filter(|p| p != &child_session_id),
        },
    };

    // The shared seeding tail: mid-stream parking, the CAS seed, and the merge
    // double-check — identical for a user message, a wake, and a child's
    // opening task. Seeding by hand here is what let a spawn into an ALREADY
    // RUNNING session overwrite that session's turn record; the merge path
    // steers it instead.
    let (outcome, _) = send::deliver(
        deps,
        cfg,
        &child_session_id,
        options,
        send::Delivery {
            message: &task,
            entry_id: entry_id.as_deref(),
            origin: origin.as_ref(),
            lineage: &lineage,
        },
    )
    .await?;

    Ok(ChildIds {
        session_id: outcome.session_id,
        turn_id: outcome.turn_id,
    })
}

/// The child's provider. An explicit request wins; otherwise the parent's
/// provider is inherited ONLY when the model is inherited too — model and
/// provider must stay a coherent pair. A spawn that names its own model must
/// not carry the parent's provider onto a foreign model (a zai::glm parent
/// spawning `model=claude-*` would pin the claude model to Z.AI, which
/// rejects it upstream as an unknown model); leaving it unset lets the
/// router route the model by catalog.
fn child_provider(req: &SpawnRequest, parent_record: Option<&TurnRecord>) -> Option<String> {
    req.provider.clone().or_else(|| {
        if req.model.is_some() {
            None
        } else {
            parent_record.and_then(|p| p.options.provider.clone())
        }
    })
}

/// A child inherits only the parent's filesystem scope, so the session's
/// picked directory stays reachable from sub-agent shell/coder calls. The rest
/// of the parent metadata belongs to the parent's turn and must not leak.
fn inherit_filesystem_scope(parent: Option<&TurnRecord>) -> Option<Value> {
    let root = parent.and_then(|p| p.options.filesystem_root())?;
    Some(fs_scope_metadata(root))
}

/// The child's `metadata.fs_scope`: an explicit spawn `filesystem_root`
/// wins; absent, the parent's scope is inherited unchanged. The explicit
/// value is deliberately not validated against any jail — the shell
/// worker's roots and the approval gate on `harness::spawn` remain the
/// security boundary.
fn child_filesystem_scope(
    explicit_root: Option<&str>,
    parent: Option<&TurnRecord>,
) -> Result<Option<Value>, HarnessError> {
    let Some(root) = explicit_root else {
        return Ok(inherit_filesystem_scope(parent));
    };
    if !std::path::Path::new(root).is_absolute() {
        return Err(HarnessError::InvalidRequest(format!(
            "spawn filesystem_root must be an absolute path, got {root:?}"
        )));
    }
    Ok(Some(fs_scope_metadata(root)))
}

fn is_error(code: &str, message: String) -> ResultData {
    ResultData {
        content: vec![ContentBlock::text(message.clone())],
        is_error: true,
        details: json!({ "error": code, "message": message }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::output::OutputContract;
    use crate::types::turn::TurnStatus;

    fn parent_record(metadata: Option<Value>) -> TurnRecord {
        TurnRecord {
            turn_id: "t_parent".into(),
            session_id: "s_parent".into(),
            status: TurnStatus::AwaitingFunctions,
            step: 1,
            turn_count: 1,
            depth: 0,
            message_preview: None,
            abort: false,
            watermark_entry_id: None,
            stream_request_id: None,
            options: TurnOptions {
                model: "m".into(),
                provider: None,
                system_prompt: None,
                mode: None,
                max_turns: 16,
                max_output_tokens: None,
                max_total_tokens: None,
                max_cost_usd: None,
                budget_root_session_id: None,
                thinking_level: None,
                provider_options: None,
                output: OutputContract::Text,
                functions: None,
                metadata,
                max_validation_retries: 2,
                max_transient_resumes: 1,
            },
            calls: Default::default(),
            parent: None,
            display_parent_session_id: None,
            functions_generation: None,
            result: None,
            result_error: None,
            validation_retries: 0,
            transient_resumes: 0,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn spawn_request(model: Option<&str>, provider: Option<&str>) -> SpawnRequest {
        SpawnRequest {
            task: crate::functions::send::MessageInput::Text("t".into()),
            model: model.map(str::to_string),
            provider: provider.map(str::to_string),
            session_id: None,
            parent_session_id: None,
            options: None,
        }
    }

    #[test]
    fn provider_is_inherited_only_together_with_the_model() {
        let mut parent = parent_record(None);
        parent.options.provider = Some("zai".into());

        // Explicit model, no provider: the parent's provider must NOT ride
        // along — the router resolves the model's own provider instead.
        assert_eq!(
            child_provider(
                &spawn_request(Some("claude-sonnet-4-6"), None),
                Some(&parent)
            ),
            None
        );
        // Neither given: the parent's coherent model+provider pair applies.
        assert_eq!(
            child_provider(&spawn_request(None, None), Some(&parent)),
            Some("zai".into())
        );
        // An explicit provider always wins, with or without an explicit model.
        assert_eq!(
            child_provider(
                &spawn_request(Some("glm-5.2"), Some("openai")),
                Some(&parent)
            ),
            Some("openai".into())
        );
        assert_eq!(
            child_provider(&spawn_request(None, Some("openai")), Some(&parent)),
            Some("openai".into())
        );
        // Parentless spawns have nothing to inherit either way.
        assert_eq!(child_provider(&spawn_request(None, None), None), None);
    }

    #[test]
    fn child_inherits_the_parent_filesystem_root() {
        let parent = parent_record(Some(json!({
            "fs_scope": { "root": "/work/project" },
            "message_id": "m_1",
            "session_id": "s_console",
        })));
        assert_eq!(
            inherit_filesystem_scope(Some(&parent)),
            Some(json!({ "fs_scope": { "root": "/work/project" } }))
        );
    }

    #[test]
    fn child_metadata_stays_none_without_a_parent_filesystem_root() {
        // Direct spawns have no parent record; parents without a picked
        // directory must not fabricate one. Other metadata keys are per-turn
        // tracing and never leak onto the child.
        assert_eq!(inherit_filesystem_scope(None), None);
        let unscoped = parent_record(Some(json!({ "message_id": "m_1" })));
        assert_eq!(inherit_filesystem_scope(Some(&unscoped)), None);
        let bare = parent_record(None);
        assert_eq!(inherit_filesystem_scope(Some(&bare)), None);
    }

    #[test]
    fn explicit_filesystem_root_overrides_the_parent_scope() {
        let parent = parent_record(Some(json!({
            "fs_scope": { "root": "/work/project" },
        })));
        assert_eq!(
            child_filesystem_scope(Some("/work/project/.wt/wt_1"), Some(&parent)).unwrap(),
            Some(json!({ "fs_scope": { "root": "/work/project/.wt/wt_1" } }))
        );
        // Without a parent the explicit root still applies.
        assert_eq!(
            child_filesystem_scope(Some("/elsewhere"), None).unwrap(),
            Some(json!({ "fs_scope": { "root": "/elsewhere" } }))
        );
    }

    #[test]
    fn absent_filesystem_root_keeps_the_inheritance_behavior() {
        let parent = parent_record(Some(json!({
            "fs_scope": { "root": "/work/project" },
        })));
        assert_eq!(
            child_filesystem_scope(None, Some(&parent)).unwrap(),
            inherit_filesystem_scope(Some(&parent)),
        );
        // Metadata stays None when neither side scopes, so the record's wire
        // shape is unchanged (skip_serializing_if keeps it absent).
        assert_eq!(child_filesystem_scope(None, None).unwrap(), None);
    }

    #[test]
    fn relative_filesystem_root_is_rejected() {
        let err = child_filesystem_scope(Some("relative/dir"), None).unwrap_err();
        assert_eq!(err.code(), "harness/invalid_request");
        assert!(err.to_string().contains("absolute path"));
    }

    fn broad_policy() -> FunctionPolicy {
        FunctionPolicy {
            allow: vec!["*".into()],
            deny: vec![],
            expose: Default::default(),
        }
    }

    fn narrowed_cfg() -> WorkerConfig {
        WorkerConfig {
            default_functions: Some(FunctionPolicy {
                allow: vec!["state::get".into()],
                deny: vec![],
                expose: Default::default(),
            }),
            ..WorkerConfig::default()
        }
    }

    #[test]
    fn ask_mode_child_is_clamped_to_the_configured_default() {
        // The shipped default is `*`, so ask no longer strips the data plane…
        let cfg = WorkerConfig::default();
        let mut parent = parent_record(None);
        parent.options.functions = Some(broad_policy());
        let asked = child_functions(&cfg, Some(&parent), None, Some(Mode::Ask), false);
        assert!(policy::CompiledPolicy::from(asked.as_ref()).allows("state::set"));

        // …but the chokepoint still bites when the operator narrows the default.
        let asked = child_functions(&narrowed_cfg(), Some(&parent), None, Some(Mode::Ask), false);
        let compiled = policy::CompiledPolicy::from(asked.as_ref());
        assert!(compiled.allows("state::get"));
        assert!(!compiled.allows("state::set"));
    }

    #[test]
    fn ask_mode_keeps_the_narrower_of_parent_and_baseline() {
        let cfg = WorkerConfig::default();
        let mut parent = parent_record(None);
        parent.options.functions = Some(FunctionPolicy {
            allow: vec!["state::get".into()],
            deny: vec![],
            expose: Default::default(),
        });
        let asked = child_functions(&cfg, Some(&parent), None, Some(Mode::Ask), false);
        let compiled = policy::CompiledPolicy::from(asked.as_ref());
        assert!(compiled.allows("state::get"));
        // In the baseline but outside the inherited policy — still denied.
        assert!(!compiled.allows("state::list"));
    }

    #[test]
    fn parentless_ask_child_takes_the_configured_default() {
        let f = child_functions(&narrowed_cfg(), None, None, Some(Mode::Ask), false);
        let compiled = policy::CompiledPolicy::from(f.as_ref());
        assert!(compiled.allows("state::get"));
        assert!(!compiled.allows("state::set"));
    }

    #[test]
    fn ask_mode_child_clamps_an_explicit_broad_request() {
        // The direct escalation vector: a spawn that explicitly asks for `*`
        // under a narrowed operator default. Parented or not, the ask child
        // stays capped.
        let cfg = narrowed_cfg();
        let mut parent = parent_record(None);
        parent.options.functions = Some(broad_policy());
        for parent_rec in [Some(&parent), None] {
            let f = child_functions(
                &cfg,
                parent_rec,
                Some(&broad_policy()),
                Some(Mode::Ask),
                false,
            );
            let compiled = policy::CompiledPolicy::from(f.as_ref());
            assert!(compiled.allows("state::get"));
            assert!(!compiled.allows("state::set"));
            assert!(!compiled.allows("harness::spawn"));
        }
    }

    #[test]
    fn parentless_child_defaults_are_unclamped_outside_ask() {
        // The pre-existing parentless arms child_functions now owns: an explicit
        // request applies as-is, and no request falls back to the configured
        // default (`*` as shipped).
        let cfg = WorkerConfig::default();
        let explicit = child_functions(&cfg, None, Some(&broad_policy()), Some(Mode::Agent), false);
        assert!(policy::CompiledPolicy::from(explicit.as_ref()).allows("state::set"));
        let defaulted = child_functions(&cfg, None, None, None, false);
        assert!(policy::CompiledPolicy::from(defaulted.as_ref()).allows("state::set"));
    }

    #[test]
    fn spawned_children_are_leaves_by_default() {
        // The capability wall: a `*` parent's child loses the orchestration
        // surface unless the spawn says `orchestrator: true`.
        let cfg = WorkerConfig::default();
        let mut parent = parent_record(None);
        parent.options.functions = Some(broad_policy());
        let leaf = child_functions(&cfg, Some(&parent), None, Some(Mode::Agent), false);
        let compiled = policy::CompiledPolicy::from(leaf.as_ref());
        assert!(compiled.allows("state::set"), "data-plane must survive");
        for id in [
            "harness::spawn",
            "harness::send",
            "engine::register_trigger",
            "engine::unregister_trigger",
            "engine::registered-triggers::list",
        ] {
            assert!(!compiled.allows(id), "{id} must be denied to a leaf child");
        }
    }

    #[test]
    fn an_orchestrator_child_keeps_the_control_plane_capped_by_the_parent() {
        let cfg = WorkerConfig::default();
        let mut parent = parent_record(None);
        parent.options.functions = Some(broad_policy());
        let orch = child_functions(&cfg, Some(&parent), None, Some(Mode::Agent), true);
        let compiled = policy::CompiledPolicy::from(orch.as_ref());
        assert!(compiled.allows("harness::spawn"));
        assert!(compiled.allows("engine::register_trigger"));

        // A narrow parent grants nothing extra, orchestrator or not.
        parent.options.functions = Some(FunctionPolicy {
            allow: vec!["state::*".into()],
            deny: vec![],
            expose: Default::default(),
        });
        let capped = child_functions(&cfg, Some(&parent), None, Some(Mode::Agent), true);
        assert!(!policy::CompiledPolicy::from(capped.as_ref()).allows("harness::spawn"));
    }

    #[test]
    fn a_leaf_parents_child_cannot_reclaim_the_control_plane() {
        // Denies union through subsetting: a leaf's denies ride into its own
        // child even when that child is spawned `orchestrator: true`.
        let cfg = WorkerConfig::default();
        let mut leaf_parent = parent_record(None);
        let mut leaf_policy = broad_policy();
        leaf_policy
            .deny
            .extend(policy::CONTROL_PLANE_DENY.iter().map(|s| s.to_string()));
        leaf_parent.options.functions = Some(leaf_policy);
        let child = child_functions(&cfg, Some(&leaf_parent), None, Some(Mode::Agent), true);
        assert!(!policy::CompiledPolicy::from(child.as_ref()).allows("harness::spawn"));
    }

    #[test]
    fn options_functions_still_narrow_a_leaf() {
        let cfg = WorkerConfig::default();
        let mut parent = parent_record(None);
        parent.options.functions = Some(broad_policy());
        let narrow = FunctionPolicy {
            allow: vec!["state::set".into()],
            deny: vec![],
            expose: Default::default(),
        };
        let child = child_functions(&cfg, Some(&parent), Some(&narrow), Some(Mode::Agent), false);
        let compiled = policy::CompiledPolicy::from(child.as_ref());
        assert!(compiled.allows("state::set"));
        assert!(!compiled.allows("state::get"));
        assert!(!compiled.allows("harness::spawn"));
    }

    #[test]
    fn spawned_result_promises_no_failure_backchannel() {
        // A child's death is not injected into the spawner: the result text
        // must steer the caller to the medium + status + deadlines instead of
        // promising a [child-failure] wake-up that no longer exists.
        let ids = ChildIds {
            session_id: "s_child".into(),
            turn_id: "t_child".into(),
        };
        let result = spawned_result(&ids);
        let text = serde_json::to_string(&result.content).unwrap();
        assert!(!text.contains("[child-failure]"));
        assert!(text.contains("harness::status"));
        assert!(text.contains("deadline"));
    }
}
