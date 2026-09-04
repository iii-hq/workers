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
use crate::functions::spawn::{SpawnRequest, SubagentDisplay};
use crate::ids;
use crate::policy;
use crate::prompt;
use crate::trigger::ResultData;
use crate::types::content::ContentBlock;
use crate::types::turn::{fs_scope_metadata, FunctionPolicy, ParentLink, TurnOptions, TurnRecord};

/// The ids of a freshly-seeded child turn.
pub struct ChildIds {
    pub session_id: String,
    pub turn_id: String,
    /// The named session already existed — its transcript and parent linkage
    /// were retained (in-turn reuse is confined to the caller's own tree).
    pub reused: bool,
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
    // Fan-out budget: only child SESSIONS created by this turn consume slots.
    // At capacity, resolve an explicit target before rejecting it: appending a
    // task to an eligible existing session creates no child and must remain
    // available even when every creation slot has been used.
    let created = parent.created_child_session_count() as u32;
    let reuses_existing = if created >= cfg.max_children {
        request_targets_reusable_session(deps, parent, &req)
            .await
            .map_err(|e| is_error(e.code(), e.to_string()))?
    } else {
        false
    };
    enforce_fanout(created, cfg.max_children, reuses_existing)?;

    let parent_link = ParentLink {
        session_id: parent.session_id.clone(),
        turn_id: parent.turn_id.clone(),
        function_call_id: call_id.to_string(),
    };
    seed_child(
        deps,
        &cfg,
        &req,
        Some(&parent_link),
        Some(parent),
        reuses_existing,
    )
    .await
    .map_err(|e| is_error(e.code(), e.to_string()))
}

/// At-capacity preflight for an explicitly named session. A successful result
/// switches `seed_child` to reuse-only admission: it rechecks existence and
/// ownership, but can no longer create the target if it disappears meanwhile.
async fn request_targets_reusable_session(
    deps: &Deps,
    parent: &TurnRecord,
    req: &SpawnRequest,
) -> Result<bool, HarnessError> {
    let Some(target_session_id) = req.session_id.as_deref() else {
        return Ok(false);
    };
    let session = deps.session().await;
    let Some(metadata) = session.metadata_of(target_session_id).await? else {
        return Ok(false);
    };
    validate_turn_reuse(
        &parent.session_id,
        target_session_id,
        metadata.get("parent_session_id").and_then(Value::as_str),
    )?;
    Ok(true)
}

fn enforce_fanout(
    created: u32,
    max_children: u32,
    reuses_existing: bool,
) -> Result<(), ResultData> {
    if reuses_existing || created < max_children {
        return Ok(());
    }
    Err(is_error(
        "harness/spawn_fanout_exceeded",
        format!(
            "{created} child sessions created this turn at or above max_children \
             {max_children} — the cap is PER TURN. Consolidate the remaining work into the \
             children already running (one child can cover several parts), or start the \
             remainder from a later turn (e.g. your next notification-woken one); do not \
             retry this spawn in this turn."
        ),
    ))
}

/// The immediate function result for a successful fire-and-forget spawn.
pub fn spawned_result(child: &ChildIds) -> ResultData {
    let ids = json!({
        "child_session_id": child.session_id,
        "child_turn_id": child.turn_id,
    });
    let reuse_note = if child.reused {
        "\nreused: the named session already existed (inside your own tree) — its prior \
         transcript is retained and this task was appended to it."
    } else {
        ""
    };
    let text = format!("{ids}{reuse_note}");
    ResultData {
        content: vec![ContentBlock::text(text)],
        is_error: false,
        details: json!({
            "child_session_id": child.session_id,
            "child_turn_id": child.turn_id,
            "fire_and_forget": true,
            "reused": child.reused,
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
    seed_child(deps, &cfg, req, parent, None, false).await
}

/// Resolve a child's dispatch policy. An in-turn child starts from the
/// PARENT'S policy (`subset_policy` forbids escalation and honours an explicit
/// narrower request); a parentless (direct/CLI) spawn starts from explicit
/// options, else the read-only baseline. Then the capability wall: unless the
/// spawn opted into `orchestrator: true`, the child's deny globs gain
/// [`policy::CONTROL_PLANE_DENY`] — a leaf performs its assignment and updates
/// the shared medium; it cannot spawn, message sessions, or touch trigger
/// registrations, whatever its prompt says. Denies union through subsetting,
/// so a leaf's own children stay leaves. Any real whitelist then keeps the
/// contract-discovery pair ([`policy::CHILD_DISCOVERY_ALLOW`]): the sub-agent
/// contract makes a `functions::info` round mandatory, so an allow-list of
/// just the work functions quietly starves an obedient child.
fn child_functions(
    cfg: &WorkerConfig,
    parent_record: Option<&TurnRecord>,
    requested: Option<&FunctionPolicy>,
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
    if let Some(p) = functions.as_mut() {
        // An EMPTY allow is deliberate dispatch-disabled — granting a
        // browse-only catalog to a child that can call nothing helps nobody.
        if !p.allow.is_empty() {
            for id in policy::CHILD_DISCOVERY_ALLOW {
                // Skip when already covered, and skip a dead entry when a
                // deny glob claims it — deny wins at dispatch either way.
                if !policy::glob_covered(id, &p.allow) && !policy::glob_covered(id, &p.deny) {
                    p.allow.push(id.to_string());
                }
            }
        }
    }
    functions
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
    reuse_only: bool,
) -> Result<ChildIds, HarnessError> {
    let session = deps.session().await;

    // Resolve the agent profile (if named) before anything else fallible —
    // an unknown id must not leave a session behind.
    let agent = match req.agent.as_deref() {
        Some(id) => {
            if req
                .options
                .as_ref()
                .is_some_and(|o| o.system_prompt.is_some())
            {
                return Err(HarnessError::InvalidRequest(
                    "spawn `agent` supplies the child's system prompt; drop \
                     `options.system_prompt` or drop `agent` (with `agent` set, \
                     `system_prompt_strategy` is ignored: the profile's resolved \
                     prompt is the child's whole identity)"
                        .into(),
                ));
            }
            Some(crate::agents::resolve(deps, cfg, id).await?)
        }
        None => None,
    };
    let display = normalize_display(merged_display(req.display.as_ref(), agent.as_ref()).as_ref())?;

    let agent_route = agent
        .as_ref()
        .and_then(|profile| profile.model_and_provider());
    let model = agent_route
        .as_ref()
        .map(|(model, _)| model.clone())
        .or_else(|| req.model.clone())
        .or_else(|| parent_record.map(|p| p.options.model.clone()))
        .ok_or_else(|| {
            HarnessError::InvalidRequest(
                "spawn requires a model — only an IN-TURN spawn inherits its parent's; a \
                 parentless spawn (console, workflow, CLI) inherits nothing, so name `model` \
                 explicitly in that spawn payload"
                    .into(),
            )
        })?;
    let inherits_parent_model = req.model.is_none() && agent_route.is_none();
    let provider = agent_route
        .and_then(|(_, provider)| provider)
        .or_else(|| child_provider(req, parent_record, inherits_parent_model));
    // Children get the same single identity as every agent (the stored
    // `system-prompts/default` override when present, else the embedded
    // prompt); what makes a child a leaf is its POLICY (the control-plane
    // deny set), not a separate prompt. Spawn `options.system_prompt`
    // (+ override strategy) is the escape hatch for a child that genuinely
    // needs a different identity; an `agent` profile IS a different identity
    // — its resolved prompt replaces the default outright, so the default is
    // only fetched when no profile is set.
    let identity = if agent.is_some() {
        String::new()
    } else {
        prompt::effective_default(&deps.iii).await.identity
    };
    let identity = identity.as_str();

    let orchestrator = req
        .options
        .as_ref()
        .and_then(|o| o.orchestrator)
        .unwrap_or(false);

    let functions = child_functions(
        cfg,
        parent_record,
        req.options.as_ref().and_then(|o| o.functions.as_ref()),
        orchestrator,
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

    // Resolve every locally fallible input before creating a session. An
    // invalid task or filesystem root must not leave behind an uncounted empty
    // session that a later call can reuse around the fan-out budget.
    let task = normalize_message(req.task.clone())?;
    let (entry_id, origin) = (Some(ids::spawn_entry_id()), Some(json!({ "spawn": true })));
    let mut thinking_level = req.options.as_ref().and_then(|o| o.thinking_level);
    let mut provider_options = None;
    if let Some(agent) = agent.as_ref() {
        agent.apply_reasoning(
            provider.as_deref(),
            &mut thinking_level,
            &mut provider_options,
        );
    }
    let mut options = TurnOptions {
        model,
        provider,
        system_prompt: match agent.as_ref() {
            Some(a) => Some(a.prompt.clone()),
            None => prompt::resolve_system_prompt(
                req.options.as_ref().and_then(|o| o.system_prompt.clone()),
                req.options
                    .as_ref()
                    .map(|o| o.system_prompt_strategy)
                    .unwrap_or_default(),
                identity,
            ),
        },
        skills_prompt: None,
        skill_context: None,
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
        thinking_level,
        provider_options,
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
        // The child's OWN identity, never the parent's.
        agent: agent.as_ref().map(|a| a.identity.clone()),
        max_validation_retries: req
            .options
            .as_ref()
            .and_then(|o| o.max_validation_retries)
            .unwrap_or(cfg.max_validation_retries),
        max_transient_resumes: cfg.max_transient_resumes,
    };
    // Child session, with sub-agent linkage and optional display identity
    // merged into SessionMeta.metadata. The display name is also the title on
    // creation; `session::ensure` deliberately keeps an existing session's
    // title and metadata on reuse.
    // A live parent turn gives the full linkage (resolve + display). A direct
    // parentless spawn has no parent turn, but a caller-supplied
    // `parent_session_id` still writes a display-only link so the console
    // nests the child (no policy inheritance, no parent-call resolution).
    // `spawned_by` is always "agent": every spawn is a direct call now —
    // trigger delivery never creates an agent.
    let linkage = send::session_metadata_with_agent(
        child_session_metadata(
            parent,
            req.parent_session_id.as_deref(),
            depth,
            display.as_ref(),
        ),
        agent.as_ref(),
    );
    let title = display.as_ref().map(|value| value.name.as_str());
    let mut reused = false;
    let child_session_id = match &req.session_id {
        Some(id) if reuse_only => {
            let metadata = session.metadata_of(id).await?.ok_or_else(|| {
                HarnessError::InvalidRequest(format!(
                    "spawn session_id `{id}` no longer exists: the turn is already at its \
                     max_children creation cap, so this call may reuse an existing session but \
                     cannot recreate one — retry after the session exists again"
                ))
            })?;
            if let Some(p) = parent {
                validate_turn_reuse(
                    &p.session_id,
                    id,
                    metadata.get("parent_session_id").and_then(Value::as_str),
                )?;
            }
            reused = true;
            id.clone()
        }
        Some(id) => {
            let ensured = session.ensure(id, title, linkage.as_ref()).await?;
            if !ensured.created {
                // Reuse is legitimate for a parentless caller (a fork, or
                // delivering a reaction into an existing chat). From a live
                // turn it is almost always a cross-run id collision — models
                // re-invent the same "random" ids — so it is confined to the
                // caller's own tree and reported back either way (`reused`).
                if let Some(p) = parent {
                    validate_turn_reuse(
                        &p.session_id,
                        id,
                        ensured
                            .metadata
                            .get("parent_session_id")
                            .and_then(Value::as_str),
                    )?;
                }
                reused = true;
                tracing::info!(
                    child_session_id = %id,
                    "harness::spawn reused an existing session — prior transcript and parent linkage retained"
                );
            }
            id.clone()
        }
        None => session.create(title, linkage.as_ref()).await?,
    };

    let previous_child = if reused {
        crate::state::get_turn(&deps.iii, &child_session_id, cfg.session_timeout_ms).await?
    } else {
        None
    };
    // A profile's skill filter counts as an explicit request: it must survive
    // terminal-session reuse (the rebase pass otherwise restores the prior
    // turn's filter), which also means an agent-spawn into an ACTIVE reused
    // session fails the no-mid-turn-skill-change guard — acceptable.
    let requested_skills = req
        .options
        .as_ref()
        .and_then(|options| options.skills.as_deref())
        .or_else(|| agent.as_ref().and_then(|a| a.skills.as_deref()));
    crate::functions::send::validate_active_skill_request(
        previous_child
            .as_ref()
            .is_some_and(|record| !record.status.is_terminal()),
        requested_skills.is_some(),
    )?;
    crate::functions::send::prepare_skill_context(
        deps,
        &mut options,
        child_skill_previous(reused, previous_child.as_ref()),
        requested_skills,
    )
    .await?;

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
            caller_holds_session_lock: caller_holds_child_session_lock(
                parent_record,
                &child_session_id,
            ),
            skills_explicit: requested_skills.is_some(),
        },
    )
    .await?;

    Ok(ChildIds {
        session_id: outcome.session_id,
        turn_id: outcome.turn_id,
        reused,
    })
}

fn child_skill_previous(reused: bool, previous_child: Option<&TurnRecord>) -> Option<&TurnOptions> {
    reused
        .then(|| previous_child.map(|record| &record.options))
        .flatten()
}

fn caller_holds_child_session_lock(
    parent_record: Option<&TurnRecord>,
    child_session_id: &str,
) -> bool {
    parent_record.is_some_and(|parent| parent.session_id == child_session_id)
}

/// Display defaults from the agent profile: no explicit display → the
/// profile's name (truncated to the 48-char cap), icon, and color; an explicit
/// display keeps its fields and borrows profile appearance where fields are
/// absent. Without a profile the request passes through untouched.
fn merged_display(
    display: Option<&SubagentDisplay>,
    agent: Option<&crate::agents::ResolvedAgent>,
) -> Option<SubagentDisplay> {
    let Some(agent) = agent else {
        return display.cloned();
    };
    match display {
        Some(d) => Some(SubagentDisplay {
            icon: d.icon.or(agent.icon),
            color: d.color.or(agent.color),
            ..d.clone()
        }),
        None => Some(SubagentDisplay {
            name: agent.name.chars().take(48).collect(),
            icon: agent.icon,
            color: agent.color,
        }),
    }
}

fn normalize_display(
    display: Option<&SubagentDisplay>,
) -> Result<Option<SubagentDisplay>, HarnessError> {
    let Some(display) = display else {
        return Ok(None);
    };
    let name = display.name.trim();
    if name.is_empty() {
        return Err(HarnessError::InvalidRequest(
            "spawn display.name must not be empty".into(),
        ));
    }
    if name.chars().count() > 48 {
        return Err(HarnessError::InvalidRequest(
            "spawn display.name must be at most 48 characters".into(),
        ));
    }
    Ok(Some(SubagentDisplay {
        name: name.to_string(),
        icon: display.icon,
        color: display.color,
    }))
}

fn child_session_metadata(
    parent: Option<&ParentLink>,
    display_parent_session_id: Option<&str>,
    depth: u32,
    display: Option<&SubagentDisplay>,
) -> Option<Value> {
    let mut metadata = serde_json::Map::new();
    if let Some(parent) = parent {
        metadata.insert(
            "parent_session_id".into(),
            Value::String(parent.session_id.clone()),
        );
        metadata.insert(
            "parent_turn_id".into(),
            Value::String(parent.turn_id.clone()),
        );
        metadata.insert(
            "function_call_id".into(),
            Value::String(parent.function_call_id.clone()),
        );
        metadata.insert("depth".into(), json!(depth));
        metadata.insert("spawned_by".into(), Value::String("agent".into()));
    } else if let Some(parent_session_id) = display_parent_session_id {
        metadata.insert(
            "parent_session_id".into(),
            Value::String(parent_session_id.to_string()),
        );
        metadata.insert("depth".into(), json!(depth));
        metadata.insert("spawned_by".into(), Value::String("agent".into()));
    }
    if let Some(display) = display {
        metadata.insert(
            "subagent_display".into(),
            serde_json::to_value(display).expect("sub-agent display always serializes"),
        );
    }
    (!metadata.is_empty()).then_some(Value::Object(metadata))
}

/// The child's provider. An explicit request wins; otherwise the parent's
/// provider is inherited ONLY when the model is inherited too — model and
/// provider must stay a coherent pair. A spawn that names its own model (or
/// takes one from an agent profile) must not carry the parent's provider onto
/// a foreign model (a zai::glm parent spawning `model=claude-*` would pin the
/// claude model to Z.AI, which rejects it upstream as an unknown model);
/// leaving it unset lets the router route the model by catalog.
fn child_provider(
    req: &SpawnRequest,
    parent_record: Option<&TurnRecord>,
    inherits_parent_model: bool,
) -> Option<String> {
    req.provider.clone().or_else(|| {
        if inherits_parent_model {
            parent_record.and_then(|p| p.options.provider.clone())
        } else {
            None
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

/// Reuse guard for an in-turn spawn that named an EXISTING session. Reuse is
/// confined to the caller's own tree — itself (the merge path) or a child it
/// spawned (re-task/retry). Anything else is a cross-owner collision:
/// agent-invented ids repeat across runs (the same prompt re-invents the same
/// "random" suffix), and silent reuse appends the new task to the old
/// transcript and leaves the console nesting under the original parent.
/// Parentless (direct) spawns are not guarded — forks and reaction delivery
/// legitimately target foreign sessions, and there is no caller to check.
fn validate_turn_reuse(
    caller_session_id: &str,
    target_session_id: &str,
    existing_parent: Option<&str>,
) -> Result<(), HarnessError> {
    if target_session_id == caller_session_id || existing_parent == Some(caller_session_id) {
        return Ok(());
    }
    let owner = match existing_parent {
        Some(parent) => format!("belongs to parent `{parent}`"),
        None => "is a root session with no parent linkage".to_string(),
    };
    Err(HarnessError::InvalidRequest(format!(
        "spawn session_id `{target_session_id}` already exists and {owner}: an in-turn spawn \
         may reuse only its own session (`{caller_session_id}`) or a child it spawned itself. \
         Reuse keeps the existing transcript and parent, so a colliding id hijacks another \
         run's session — pick a fresh id, or omit session_id and the harness will generate one"
    )))
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
    use crate::functions::spawn::{SubagentColor, SubagentIcon};
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
                skills_prompt: None,
                skill_context: None,
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
                agent: None,
                max_validation_retries: 2,
                max_transient_resumes: 1,
            },
            calls: Default::default(),
            parent: None,
            display_parent_session_id: None,
            functions_generation: None,
            function_contract_ledger: Default::default(),
            skill_ack: None,
            skills_started: false,
            context_snapshot: None,
            result: None,
            result_error: None,
            validation_retries: 0,
            transient_resumes: 0,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn only_an_in_turn_self_session_spawn_inherits_the_callers_lock() {
        let parent = parent_record(None);

        assert!(caller_holds_child_session_lock(
            Some(&parent),
            &parent.session_id
        ));
        assert!(!caller_holds_child_session_lock(Some(&parent), "s_child"));
        assert!(!caller_holds_child_session_lock(None, &parent.session_id));
    }

    fn spawn_request(model: Option<&str>, provider: Option<&str>) -> SpawnRequest {
        SpawnRequest {
            task: crate::functions::send::MessageInput::Text("t".into()),
            agent: None,
            display: None,
            model: model.map(str::to_string),
            provider: provider.map(str::to_string),
            session_id: None,
            parent_session_id: None,
            options: None,
        }
    }

    fn display(name: &str) -> SubagentDisplay {
        SubagentDisplay {
            name: name.into(),
            icon: Some(SubagentIcon::Code),
            color: Some(SubagentColor::Blue),
        }
    }

    #[test]
    fn display_name_is_trimmed_and_bounded_by_characters() {
        let normalized = normalize_display(Some(&display("  Frontend  ")))
            .unwrap()
            .unwrap();
        assert_eq!(normalized.name, "Frontend");
        assert_eq!(normalized.icon, Some(SubagentIcon::Code));
        assert_eq!(normalized.color, Some(SubagentColor::Blue));

        assert!(normalize_display(Some(&display(" \n\t "))).is_err());
        assert!(normalize_display(Some(&display(&"é".repeat(48)))).is_ok());
        let error = normalize_display(Some(&display(&"é".repeat(49)))).unwrap_err();
        assert_eq!(error.code(), "harness/invalid_request");
        assert!(error.to_string().contains("at most 48 characters"));
    }

    #[test]
    fn display_metadata_is_merged_with_the_durable_parent_link() {
        let parent = ParentLink {
            session_id: "s_parent".into(),
            turn_id: "t_parent".into(),
            function_call_id: "call_spawn".into(),
        };
        assert_eq!(
            child_session_metadata(Some(&parent), None, 1, Some(&display("Frontend"))),
            Some(json!({
                "parent_session_id": "s_parent",
                "parent_turn_id": "t_parent",
                "function_call_id": "call_spawn",
                "depth": 1,
                "spawned_by": "agent",
                "subagent_display": {
                    "name": "Frontend",
                    "icon": "code",
                    "color": "blue"
                }
            }))
        );
    }

    #[test]
    fn parentless_display_still_creates_session_metadata() {
        assert_eq!(
            child_session_metadata(None, None, 0, Some(&display("Explorer"))),
            Some(json!({
                "subagent_display": {
                    "name": "Explorer",
                    "icon": "code",
                    "color": "blue"
                }
            }))
        );
        assert_eq!(child_session_metadata(None, None, 0, None), None);
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
                Some(&parent),
                false
            ),
            None
        );
        // Neither given: the parent's coherent model+provider pair applies.
        assert_eq!(
            child_provider(&spawn_request(None, None), Some(&parent), true),
            Some("zai".into())
        );
        // A model taken from an agent PROFILE is foreign too: the parent
        // pair must not split even though the request itself named no model.
        assert_eq!(
            child_provider(&spawn_request(None, None), Some(&parent), false),
            None
        );
        // An explicit provider always wins, with or without an explicit model.
        assert_eq!(
            child_provider(
                &spawn_request(Some("glm-5.2"), Some("openai")),
                Some(&parent),
                false
            ),
            Some("openai".into())
        );
        assert_eq!(
            child_provider(&spawn_request(None, Some("openai")), Some(&parent), true),
            Some("openai".into())
        );
        // Parentless spawns have nothing to inherit either way.
        assert_eq!(child_provider(&spawn_request(None, None), None, true), None);
    }

    #[test]
    fn fresh_children_ignore_prior_skill_context_while_reused_children_inherit_their_own() {
        let mut previous_child = parent_record(None);
        previous_child.options.skill_context = Some(crate::types::turn::SkillContext {
            filter: Some(vec!["child-only".into()]),
            baseline: Some("frozen child baseline".into()),
        });

        assert!(child_skill_previous(false, Some(&previous_child)).is_none());
        let reused = child_skill_previous(true, Some(&previous_child)).unwrap();
        assert_eq!(
            reused.skill_context.as_ref().unwrap().filter,
            Some(vec!["child-only".into()])
        );
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
    fn parentless_child_defaults() {
        // The parentless arms child_functions owns: an explicit request
        // applies as-is, and no request falls back to the configured default
        // (`*` as shipped).
        let cfg = WorkerConfig::default();
        let explicit = child_functions(&cfg, None, Some(&broad_policy()), false);
        assert!(policy::CompiledPolicy::from(explicit.as_ref()).allows("state::set"));
        let defaulted = child_functions(&cfg, None, None, false);
        assert!(policy::CompiledPolicy::from(defaulted.as_ref()).allows("state::set"));

        // A narrowed operator default is what a parentless child falls back to.
        let narrowed = child_functions(&narrowed_cfg(), None, None, false);
        let compiled = policy::CompiledPolicy::from(narrowed.as_ref());
        assert!(compiled.allows("state::get"));
        assert!(!compiled.allows("state::set"));
    }

    #[test]
    fn spawned_children_are_leaves_by_default() {
        // The capability wall: a `*` parent's child loses the orchestration
        // surface unless the spawn says `orchestrator: true`.
        let cfg = WorkerConfig::default();
        let mut parent = parent_record(None);
        parent.options.functions = Some(broad_policy());
        let leaf = child_functions(&cfg, Some(&parent), None, false);
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
        let orch = child_functions(&cfg, Some(&parent), None, true);
        let compiled = policy::CompiledPolicy::from(orch.as_ref());
        assert!(compiled.allows("harness::spawn"));
        assert!(compiled.allows("engine::register_trigger"));

        // A narrow parent grants nothing extra, orchestrator or not.
        parent.options.functions = Some(FunctionPolicy {
            allow: vec!["state::*".into()],
            deny: vec![],
            expose: Default::default(),
        });
        let capped = child_functions(&cfg, Some(&parent), None, true);
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
        let child = child_functions(&cfg, Some(&leaf_parent), None, true);
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
        let child = child_functions(&cfg, Some(&parent), Some(&narrow), false);
        let compiled = policy::CompiledPolicy::from(child.as_ref());
        assert!(compiled.allows("state::set"));
        assert!(!compiled.allows("state::get"));
        assert!(!compiled.allows("harness::spawn"));
    }

    /// Prevents: the discovery-starved courier — an allow-list of just the
    /// work functions makes the sub-agent contract's mandatory
    /// `functions::list`/`::info` round impossible, so the obedient child
    /// reports FAILED while siblings that skip discovery succeed.
    #[test]
    fn a_narrowed_child_always_keeps_contract_discovery() {
        let cfg = WorkerConfig::default();
        let mut parent = parent_record(None);
        parent.options.functions = Some(broad_policy());
        let narrow = FunctionPolicy {
            allow: vec!["database::executeBatch".into()],
            deny: vec![],
            expose: Default::default(),
        };
        let child = child_functions(&cfg, Some(&parent), Some(&narrow), false);
        let compiled = policy::CompiledPolicy::from(child.as_ref());
        assert!(compiled.allows("database::executeBatch"));
        assert!(compiled.allows("engine::functions::list"));
        assert!(compiled.allows("engine::functions::info"));
        // The union grants the metadata plane only — the leaf wall and the
        // whitelist still hold.
        assert!(!compiled.allows("engine::register_trigger"));
        assert!(!compiled.allows("database::execute"));
    }

    #[test]
    fn the_discovery_union_adds_nothing_redundant_dead_or_widening() {
        let cfg = WorkerConfig::default();
        let mut parent = parent_record(None);
        parent.options.functions = Some(broad_policy());

        // Already covered by a glob: no duplicate entries.
        let covered = FunctionPolicy {
            allow: vec!["engine::*".into(), "state::set".into()],
            deny: vec![],
            expose: Default::default(),
        };
        let child = child_functions(&cfg, Some(&parent), Some(&covered), false).unwrap();
        assert_eq!(child.allow, vec!["engine::*", "state::set"]);

        // Explicitly denied: deny wins, and no dead allow entry is written.
        let denied = FunctionPolicy {
            allow: vec!["database::executeBatch".into()],
            deny: vec!["engine::functions::*".into()],
            expose: Default::default(),
        };
        let child = child_functions(&cfg, Some(&parent), Some(&denied), false).unwrap();
        assert_eq!(child.allow, vec!["database::executeBatch"]);
        assert!(!policy::CompiledPolicy::from(Some(&child)).allows("engine::functions::info"));

        // An EMPTY allow is deliberate dispatch-disabled — it must stay that
        // way, not become a browse-only two-entry whitelist.
        let disabled = FunctionPolicy {
            allow: vec![],
            deny: vec![],
            expose: Default::default(),
        };
        let child = child_functions(&cfg, None, Some(&disabled), false).unwrap();
        assert!(child.allow.is_empty());
        assert!(!policy::CompiledPolicy::from(Some(&child)).allows("engine::functions::list"));
    }

    #[test]
    fn spawned_result_is_compact_for_fresh_and_reused_children() {
        let fresh = spawned_result(&ChildIds {
            session_id: "s_child".into(),
            turn_id: "t_child".into(),
            reused: false,
        });
        assert_eq!(
            fresh.content,
            vec![ContentBlock::text(
                r#"{"child_session_id":"s_child","child_turn_id":"t_child"}"#,
            )]
        );
        assert_eq!(
            fresh.details,
            json!({
                "child_session_id": "s_child",
                "child_turn_id": "t_child",
                "fire_and_forget": true,
                "reused": false,
            })
        );

        let reused = spawned_result(&ChildIds {
            session_id: "s_child".into(),
            turn_id: "t_child".into(),
            reused: true,
        });
        assert_eq!(
            reused.content,
            vec![ContentBlock::text(
                "{\"child_session_id\":\"s_child\",\"child_turn_id\":\"t_child\"}\nreused: \
                 the named session already existed (inside your own tree) — its prior transcript is \
                 retained and this task was appended to it.",
            )]
        );
        assert_eq!(
            reused.details,
            json!({
                "child_session_id": "s_child",
                "child_turn_id": "t_child",
                "fire_and_forget": true,
                "reused": true,
            })
        );
    }

    /// Prevents: the courier-k3m7 mis-nesting — an in-turn spawn naming an id
    /// that an EARLIER run created must fail loudly (naming the owner and the
    /// remedy), never silently hijack that run's session. Reuse inside the
    /// caller's own tree stays legal for retry/re-task flows.
    #[test]
    fn turn_reuse_is_confined_to_the_callers_tree() {
        assert!(validate_turn_reuse("s_parent", "s_parent", None).is_ok());
        assert!(validate_turn_reuse("s_parent", "courier-1", Some("s_parent")).is_ok());

        let err = validate_turn_reuse("s_parent", "courier-1", Some("console-old"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("courier-1"), "{err}");
        assert!(err.contains("console-old"), "{err}");
        assert!(err.contains("s_parent"), "{err}");
        assert!(err.contains("omit session_id"), "{err}");

        // An existing ROOT session (no linkage) is someone's chat, not a
        // spawn target — a grandchild (linked to a different parent) is
        // equally out of reach.
        let err = validate_turn_reuse("s_parent", "console-x", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("root session"), "{err}");
    }

    #[test]
    fn spawned_result_reports_reuse() {
        let fresh = spawned_result(&ChildIds {
            session_id: "s".into(),
            turn_id: "t".into(),
            reused: false,
        });
        assert_eq!(fresh.details["reused"], false);
        let text = serde_json::to_string(&fresh.content).unwrap();
        assert!(!text.contains("already existed"), "{text}");

        let reused = spawned_result(&ChildIds {
            session_id: "s".into(),
            turn_id: "t".into(),
            reused: true,
        });
        assert_eq!(reused.details["reused"], true);
        let text = serde_json::to_string(&reused.content).unwrap();
        assert!(text.contains("already existed"), "{text}");
        assert!(text.contains("transcript is retained"), "{text}");
    }

    #[test]
    fn existing_session_reuse_is_not_blocked_at_fanout_capacity() {
        assert!(enforce_fanout(8, 8, true).is_ok());

        let error = enforce_fanout(8, 8, false).unwrap_err();
        assert!(error.is_error);
        assert_eq!(error.details["error"], "harness/spawn_fanout_exceeded");
        assert!(error.details["message"]
            .as_str()
            .is_some_and(|message| message.contains("8 child sessions created this turn")));
    }

    fn resolved_agent(name: &str, icon: Option<SubagentIcon>) -> crate::agents::ResolvedAgent {
        crate::agents::ResolvedAgent {
            identity: crate::types::turn::AgentIdentity {
                id: name.to_lowercase(),
                name: Some(name.to_string()),
                icon: None,
                color: None,
            },
            prompt: format!("You are {name}. Work."),
            skills: None,
            model: None,
            reasoning_effort: None,
            name: name.to_string(),
            icon,
            color: None,
        }
    }

    #[test]
    fn display_merges_profile_defaults_under_explicit_fields() {
        let mut coder = resolved_agent("Coder", Some(SubagentIcon::Code));
        coder.color = Some(SubagentColor::Purple);
        // No explicit display → full profile identity.
        let display = merged_display(None, Some(&coder)).unwrap();
        assert_eq!(display.name, "Coder");
        assert_eq!(display.icon, Some(SubagentIcon::Code));
        assert_eq!(display.color, Some(SubagentColor::Purple));
        // Explicit display keeps its fields, borrowing missing profile fields.
        let explicit = SubagentDisplay {
            name: "Fixer".into(),
            icon: None,
            color: Some(SubagentColor::Blue),
        };
        let display = merged_display(Some(&explicit), Some(&coder)).unwrap();
        assert_eq!(display.name, "Fixer");
        assert_eq!(display.icon, Some(SubagentIcon::Code));
        assert_eq!(display.color, Some(SubagentColor::Blue));
        let colorless = SubagentDisplay {
            color: None,
            ..explicit.clone()
        };
        assert_eq!(
            merged_display(Some(&colorless), Some(&coder))
                .unwrap()
                .color,
            Some(SubagentColor::Purple)
        );
        let iconed = SubagentDisplay {
            icon: Some(SubagentIcon::Search),
            ..explicit.clone()
        };
        assert_eq!(
            merged_display(Some(&iconed), Some(&coder)).unwrap().icon,
            Some(SubagentIcon::Search)
        );
        // No profile → request passes through untouched (including None).
        assert_eq!(merged_display(None, None), None);
        assert_eq!(merged_display(Some(&explicit), None), Some(explicit));
        // Long profile names are truncated to the display cap, not rejected.
        let long = resolved_agent(&"x".repeat(60), None);
        let display = merged_display(None, Some(&long)).unwrap();
        assert_eq!(display.name.chars().count(), 48);
        assert!(normalize_display(Some(&display)).is_ok());
    }
}
